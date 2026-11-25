//! Who may do what in a store. The owner is `stores.owner_id`; managers and staff are
//! live rows of `store_members`. Everything is read from the database on each call,
//! so removing someone takes effect on their next request.

use anyhow::Context;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{auth::AuthUser, error::AppError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner,
    Manager,
    Staff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    ManageTeam,
    ManageBranches,
    ManagePayout,
    DeleteStore,
    EditOwnerFields,
    EditStoreDetails,
    PublishProducts,
    EditLivePrices,
    DeleteProducts,
    AnswerDisputes,
    ReplyReviews,
    EditProducts,
    /// Stock at the given branch.
    EditStock(Uuid),
    /// Orders fulfilled from the given branch.
    ActOnOrder(Uuid),
    ViewStore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Membership {
    pub store_id: Uuid,
    pub user_id: Uuid,
    pub role: Role,
    /// Only ever set for staff tied to one branch.
    pub branch_id: Option<Uuid>,
}

impl Membership {
    pub fn can(&self, action: Action) -> bool {
        match self.role {
            Role::Owner => true,
            Role::Manager => !matches!(
                action,
                Action::ManageTeam
                    | Action::ManageBranches
                    | Action::ManagePayout
                    | Action::DeleteStore
                    | Action::EditOwnerFields
            ),
            Role::Staff => match action {
                Action::EditProducts | Action::ViewStore => true,
                Action::EditStock(b) | Action::ActOnOrder(b) => self.covers_branch(b),
                _ => false,
            },
        }
    }

    /// True unless this is staff tied to a different branch.
    pub fn covers_branch(&self, branch: Uuid) -> bool {
        match (self.role, self.branch_id) {
            (Role::Staff, Some(own)) => own == branch,
            _ => true,
        }
    }
}

/// The user's standing in a live store, or `None` for a stranger, a removed member or a
/// deleted store.
pub async fn membership(
    conn: &mut PgConnection,
    store_id: Uuid,
    user: Uuid,
) -> Result<Option<Membership>, AppError> {
    let row: Option<(Uuid, Option<String>, Option<Uuid>)> = sqlx::query_as(
        "SELECT s.owner_id, m.role::text, m.branch_id \
         FROM stores s \
         LEFT JOIN store_members m \
           ON m.store_id = s.id AND m.user_id = $2 AND m.removed_at IS NULL \
         WHERE s.id = $1 AND s.deleted_at IS NULL",
    )
    .bind(store_id)
    .bind(user)
    .fetch_optional(&mut *conn)
    .await
    .context("load store membership")?;
    let Some((owner, role, branch_id)) = row else {
        return Ok(None);
    };
    let (role, branch_id) = if owner == user {
        (Role::Owner, None)
    } else {
        match role.as_deref() {
            Some("manager") => (Role::Manager, None),
            Some("staff") => (Role::Staff, branch_id),
            _ => return Ok(None),
        }
    };
    Ok(Some(Membership {
        store_id,
        user_id: user,
        role,
        branch_id,
    }))
}

/// 404 for non-members and deleted stores (the same answer a stranger gets), 403
/// `not_allowed` when the member's role does not allow `action`.
pub async fn require(
    conn: &mut PgConnection,
    store_id: Uuid,
    user: Uuid,
    action: Action,
) -> Result<Membership, AppError> {
    let member = membership(conn, store_id, user)
        .await?
        .ok_or(AppError::NotFound)?;
    if member.can(action) {
        Ok(member)
    } else {
        Err(AppError::Forbidden("not_allowed"))
    }
}

/// The gate in front of the seller routes: a seller, or a live member of a live store
/// (accepting an invite does not make someone a seller). Anyone else keeps today's
/// 403 `not_a_seller`; which store they may touch is still [`require`]'s answer.
pub async fn require_seller_or_member(
    conn: &mut PgConnection,
    user: &AuthUser,
) -> Result<(), AppError> {
    if user.is_seller {
        return Ok(());
    }
    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM store_members m JOIN stores s ON s.id = m.store_id \
         WHERE m.user_id = $1 AND m.removed_at IS NULL AND s.deleted_at IS NULL)",
    )
    .bind(user.id)
    .fetch_one(&mut *conn)
    .await
    .context("check store membership")?;
    if member {
        Ok(())
    } else {
        Err(AppError::Forbidden("not_a_seller"))
    }
}

/// SQL boolean: the user bound at `user_param` (e.g. `$2`) is the owner or a live
/// member of the store aliased `s`.
pub fn is_member_sql(user_param: &str) -> String {
    format!(
        "(s.owner_id = {user_param} OR EXISTS (SELECT 1 FROM store_members sm \
         WHERE sm.store_id = s.id AND sm.user_id = {user_param} AND sm.removed_at IS NULL))"
    )
}

/// As [`is_member_sql`], and the user covers the branch in `branch_col`: the owner, a
/// manager, store-wide staff, or staff tied to that branch.
pub fn covers_branch_sql(user_param: &str, branch_col: &str) -> String {
    format!(
        "(s.owner_id = {user_param} OR EXISTS (SELECT 1 FROM store_members sm \
         WHERE sm.store_id = s.id AND sm.user_id = {user_param} AND sm.removed_at IS NULL \
         AND (sm.role = 'manager' OR sm.branch_id IS NULL OR sm.branch_id = {branch_col})))"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(role: Role, branch_id: Option<Uuid>) -> Membership {
        Membership {
            store_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            role,
            branch_id,
        }
    }

    #[test]
    fn can_matrix() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let owner_only = |x: Action| {
            matches!(
                x,
                Action::ManageTeam
                    | Action::ManageBranches
                    | Action::ManagePayout
                    | Action::DeleteStore
                    | Action::EditOwnerFields
            )
        };
        let manager_up = |x: Action| {
            matches!(
                x,
                Action::EditStoreDetails
                    | Action::PublishProducts
                    | Action::EditLivePrices
                    | Action::DeleteProducts
                    | Action::AnswerDisputes
                    | Action::ReplyReviews
            )
        };
        let all = [
            Action::ManageTeam,
            Action::ManageBranches,
            Action::ManagePayout,
            Action::DeleteStore,
            Action::EditOwnerFields,
            Action::EditStoreDetails,
            Action::PublishProducts,
            Action::EditLivePrices,
            Action::DeleteProducts,
            Action::AnswerDisputes,
            Action::ReplyReviews,
            Action::EditProducts,
            Action::EditStock(a),
            Action::ActOnOrder(a),
            Action::EditStock(b),
            Action::ActOnOrder(b),
            Action::ViewStore,
        ];
        let owner = m(Role::Owner, None);
        let manager = m(Role::Manager, None);
        let wide = m(Role::Staff, None);
        let tied = m(Role::Staff, Some(a));
        for x in all {
            let branch_action = matches!(x, Action::EditStock(_) | Action::ActOnOrder(_));
            let staff_base = !owner_only(x) && !manager_up(x) && !branch_action;
            assert!(owner.can(x), "owner {x:?}");
            assert_eq!(manager.can(x), !owner_only(x), "manager {x:?}");
            assert_eq!(wide.can(x), staff_base || branch_action, "wide {x:?}");
            let on_a = matches!(x, Action::EditStock(br) | Action::ActOnOrder(br) if br == a);
            assert_eq!(tied.can(x), staff_base || on_a, "tied {x:?}");
        }
        assert!(tied.can(Action::EditStock(a)));
        assert!(!tied.can(Action::EditStock(b)));
        assert!(!tied.covers_branch(b));
        assert!(manager.covers_branch(b));
    }
}

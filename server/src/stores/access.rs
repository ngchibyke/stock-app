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
    /// False once that branch has closed: the staff member keeps the store's staff
    /// rights but no branch's orders or stock until the owner moves them.
    pub branch_open: bool,
    /// The member has passed the identity check. Owners are held back at the store
    /// instead; managers and staff need it to act on orders or edit stock.
    pub identity_passed: bool,
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

    /// True unless this is staff tied to a different branch, or to a closed one.
    pub fn covers_branch(&self, branch: Uuid) -> bool {
        match (self.role, self.branch_id) {
            (Role::Staff, Some(own)) => self.branch_open && own == branch,
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
    load_membership(conn, store_id, user, false).await
}

/// [`membership`]; `deleted_too` also answers for a deleted store.
async fn load_membership(
    conn: &mut PgConnection,
    store_id: Uuid,
    user: Uuid,
    deleted_too: bool,
) -> Result<Option<Membership>, AppError> {
    #[allow(clippy::type_complexity)]
    let row: Option<(Uuid, Option<String>, Option<Uuid>, bool, bool)> = sqlx::query_as(
        "SELECT s.owner_id, m.role::text, m.branch_id, b.closed_at IS NULL, \
                EXISTS (SELECT 1 FROM identity_checks ic \
                        WHERE ic.user_id = $2 AND ic.status = 'passed') \
         FROM stores s \
         LEFT JOIN store_members m \
           ON m.store_id = s.id AND m.user_id = $2 AND m.removed_at IS NULL \
         LEFT JOIN store_branches b ON b.id = m.branch_id \
         WHERE s.id = $1 AND ($3 OR s.deleted_at IS NULL)",
    )
    .bind(store_id)
    .bind(user)
    .bind(deleted_too)
    .fetch_optional(&mut *conn)
    .await
    .context("load store membership")?;
    let Some((owner, role, branch_id, branch_open, identity_passed)) = row else {
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
        branch_open: branch_id.is_none() || branch_open,
        identity_passed,
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
    check(load_membership(conn, store_id, user, false).await?, action)
}

/// [`require`] that still answers once the store is deleted, for the seller's side of
/// what outlives it: an order's dispute and its review (a deleted store's orders keep
/// their steps, tracking and return too). Non-members still get 404, other members 403.
pub async fn require_even_if_deleted(
    conn: &mut PgConnection,
    store_id: Uuid,
    user: Uuid,
    action: Action,
) -> Result<Membership, AppError> {
    check(load_membership(conn, store_id, user, true).await?, action)
}

fn check(member: Option<Membership>, action: Action) -> Result<Membership, AppError> {
    let member = member.ok_or(AppError::NotFound)?;
    if member.can(action) {
        // Order actions and stock edits wait for a member's identity check.
        let acts = matches!(action, Action::EditStock(_) | Action::ActOnOrder(_));
        if acts && member.role != Role::Owner && !member.identity_passed {
            return Err(AppError::Forbidden("identity_required"));
        }
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
/// manager, store-wide staff, or staff tied to that branch while it is open.
pub fn covers_branch_sql(user_param: &str, branch_col: &str) -> String {
    format!(
        "(s.owner_id = {user_param} OR EXISTS (SELECT 1 FROM store_members sm \
         WHERE sm.store_id = s.id AND sm.user_id = {user_param} AND sm.removed_at IS NULL \
         AND (sm.role = 'manager' OR sm.branch_id IS NULL \
              OR (sm.branch_id = {branch_col} AND EXISTS (SELECT 1 FROM store_branches sb \
                  WHERE sb.id = sm.branch_id AND sb.closed_at IS NULL)))))"
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
            branch_open: true,
            identity_passed: true,
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
        // Tied to a branch that has closed: no branch's orders or stock.
        let closed = Membership {
            branch_open: false,
            ..tied
        };
        assert!(!closed.can(Action::EditStock(a)) && !closed.can(Action::ActOnOrder(a)));
        assert!(closed.can(Action::ViewStore) && closed.can(Action::EditProducts));
    }
}

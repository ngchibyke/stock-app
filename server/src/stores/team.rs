//! A store's team: the owner invites a phone as manager or staff, the phone's owner
//! accepts or declines in the app, and the owner changes or ends memberships.
//!
//! **Invite probing.** `POST /stores/{key}/invites` answers the same `201 {id}` and
//! does the same work whether or not the phone has an account; the only difference
//! is the in-app notice inserted for an account (never an SMS). An invite is matched
//! to a user only by their own verified phone's blind index, so only the phone's
//! owner sees or answers it, whenever they sign up. The phone itself is never stored,
//! logged or audited: an invite keeps its blind index and last four digits.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{delete, get, patch, post},
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use sqlx::PgConnection;
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    access::{self, Action, Role},
    routes::require_seller_or_team,
};
use crate::{
    AppState, audit,
    auth::AuthUser,
    crypto::blind_index,
    error::AppError,
    escrow::jobs::{BATCH, Stats},
    extract::AppJson,
    notify::{
        kinds::Kind,
        service::{Notice, enqueue},
    },
    phone::normalize_ng_phone,
};

/// Live members a store may have (the owner is not counted).
pub const MAX_TEAM_MEMBERS: i64 = 50;
/// Invites one store may send in a rolling 24 hours.
pub const INVITES_PER_DAY: i64 = 20;
/// How long an invite stays open.
pub const INVITE_TTL_DAYS: i32 = 7;
/// An invite that expired, was declined or was revoked this long ago is deleted
/// (spec section 13; the DPO confirms the period before launch).
pub const INVITE_RETENTION_DAYS: i32 = 30;
/// Job name (`job_runs.job`).
pub const PURGE_INVITES: &str = "purge_invites";

/// An invite still open: not answered, not revoked, not expired (alias `i`).
pub(crate) const LIVE: &str = "i.accepted_at IS NULL AND i.declined_at IS NULL \
                    AND i.revoked_at IS NULL AND i.expires_at > now()";

pub fn router() -> axum::Router<AppState> {
    Router::new()
        .route("/stores/{key}/invites", post(send_invite))
        .route("/stores/{key}/invites/{id}", delete(revoke_invite))
        .route("/stores/{key}/team", get(team))
        .route(
            "/stores/{key}/members/{user_id}",
            patch(change_member).delete(remove_member),
        )
        .route("/me/invites", get(my_invites))
        .route("/me/invites/{id}/accept", post(accept))
        .route("/me/invites/{id}/decline", post(decline))
}

fn parse_id(key: &str) -> Result<Uuid, AppError> {
    key.parse().map_err(|_| AppError::NotFound)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "store_member_role", rename_all = "lowercase")]
enum TeamRole {
    Manager,
    Staff,
}

impl TeamRole {
    fn as_str(self) -> &'static str {
        match self {
            TeamRole::Manager => "manager",
            TeamRole::Staff => "staff",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewInvite {
    phone: String,
    role: TeamRole,
    branch_id: Option<Uuid>,
}

// Hand-written so the invited phone never reaches logs or panics.
impl std::fmt::Debug for NewInvite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewInvite")
            .field("phone", &"[redacted]")
            .field("role", &self.role)
            .field("branch_id", &self.branch_id)
            .finish()
    }
}

/// `branch_id` absent keeps the member's branch; `null` makes them store-wide.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberPatch {
    role: Option<TeamRole>,
    #[serde(default, deserialize_with = "present")]
    branch_id: Option<Option<Uuid>>,
}

fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<Uuid>>, D::Error> {
    Option::<Uuid>::deserialize(d).map(Some)
}

#[derive(Debug, Serialize)]
struct BranchRef {
    id: Uuid,
    name: String,
}

fn branch_ref(id: Option<Uuid>, name: Option<String>) -> Option<BranchRef> {
    Some(BranchRef {
        id: id?,
        name: name?,
    })
}

#[derive(Debug, Serialize)]
struct MemberView {
    user_id: Uuid,
    display_name: String,
    role: TeamRole,
    branch: Option<BranchRef>,
}

#[derive(Debug, Serialize)]
struct PendingInvite {
    id: Uuid,
    phone_last4: String,
    role: TeamRole,
    branch: Option<BranchRef>,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
}

#[derive(Debug, Serialize)]
struct TeamView {
    members: Vec<MemberView>,
    invites: Vec<PendingInvite>,
}

#[derive(Debug, Serialize)]
struct InviteStore {
    id: Uuid,
    name: String,
    slug: String,
    logo_url: Option<String>,
}

#[derive(Debug, Serialize)]
struct MyInvite {
    id: Uuid,
    store: InviteStore,
    role: TeamRole,
    branch: Option<BranchRef>,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
}

/// 422 unless `branch` is an open branch of `store`.
async fn check_branch(conn: &mut PgConnection, store: Uuid, branch: Uuid) -> Result<(), AppError> {
    let closed: Option<bool> = sqlx::query_scalar(
        "SELECT closed_at IS NOT NULL FROM store_branches WHERE id = $1 AND store_id = $2",
    )
    .bind(branch)
    .bind(store)
    .fetch_optional(&mut *conn)
    .await
    .context("load branch")?;
    match closed {
        None => Err(AppError::Unprocessable(
            "unknown_branch",
            "That branch is not one of this store's".into(),
        )),
        Some(true) => Err(AppError::Unprocessable(
            "branch_closed",
            "That branch is closed".into(),
        )),
        Some(false) => Ok(()),
    }
}

fn staff_only_branch() -> AppError {
    AppError::Unprocessable(
        "staff_only_branch",
        "Only staff can be tied to one branch; managers work across the store".into(),
    )
}

/// Locks a live store row (after `require` has said who may act on it).
async fn lock_store(conn: &mut PgConnection, store: Uuid) -> Result<Uuid, AppError> {
    sqlx::query_scalar(
        "SELECT owner_id FROM stores WHERE id = $1 AND deleted_at IS NULL FOR NO KEY UPDATE",
    )
    .bind(store)
    .fetch_optional(&mut *conn)
    .await
    .context("lock store")?
    .ok_or(AppError::NotFound)
}

async fn live_member_count(conn: &mut PgConnection, store: Uuid) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM store_members WHERE store_id = $1 AND removed_at IS NULL",
    )
    .bind(store)
    .fetch_one(&mut *conn)
    .await
    .context("count team")?)
}

fn team_full() -> AppError {
    AppError::ConflictDetail("team_full", "This store's team is full".into())
}

async fn send_invite(
    State(state): State<AppState>,
    user: AuthUser,
    Path(key): Path<String>,
    AppJson(body): AppJson<NewInvite>,
) -> Result<impl IntoResponse, AppError> {
    require_seller_or_team(&state, &user).await?;
    let store = parse_id(&key)?;
    let phone = normalize_ng_phone(&body.phone)?;
    let phone_idx = blind_index(&state.index_key, &phone);
    let last4 = phone[phone.len() - 4..].to_string();

    let mut tx = state.db.begin().await.context("begin")?;
    access::require(&mut tx, store, user.id, Action::ManageTeam).await?;
    lock_store(&mut tx, store).await?;
    if body.role == TeamRole::Manager && body.branch_id.is_some() {
        return Err(staff_only_branch());
    }
    if let Some(branch) = body.branch_id {
        check_branch(&mut tx, store, branch).await?;
    }
    // Only the caller's own phone and this store's members are compared; nothing
    // about anyone else's account is read before the invite is stored.
    let own: bool = sqlx::query_scalar("SELECT phone_idx = $2 FROM users WHERE id = $1")
        .bind(user.id)
        .bind(&phone_idx)
        .fetch_one(&mut *tx)
        .await
        .context("compare own phone")?;
    if own {
        return Err(AppError::Unprocessable(
            "cannot_invite_self",
            "You cannot invite yourself".into(),
        ));
    }
    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM store_members m JOIN users u ON u.id = m.user_id \
         WHERE m.store_id = $1 AND m.removed_at IS NULL AND u.phone_idx = $2)",
    )
    .bind(store)
    .bind(&phone_idx)
    .fetch_one(&mut *tx)
    .await
    .context("check existing member")?;
    if member {
        return Err(AppError::ConflictDetail(
            "already_member",
            "That phone already belongs to a member of this store".into(),
        ));
    }
    if live_member_count(&mut tx, store).await? >= MAX_TEAM_MEMBERS {
        return Err(team_full());
    }
    let recent: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM store_invites \
         WHERE store_id = $1 AND created_at > now() - interval '24 hours'",
    )
    .bind(store)
    .fetch_one(&mut *tx)
    .await
    .context("count recent invites")?;
    if recent >= INVITES_PER_DAY {
        return Err(AppError::TooManyRequests("too_many_invites"));
    }

    // A new invite to the same phone replaces the open one.
    let replaced: Vec<Uuid> = sqlx::query_scalar(&format!(
        "UPDATE store_invites i SET revoked_at = now() \
         WHERE i.store_id = $1 AND i.phone_idx = $2 AND {LIVE} RETURNING i.id"
    ))
    .bind(store)
    .bind(&phone_idx)
    .fetch_all(&mut *tx)
    .await
    .context("replace open invite")?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO store_invites \
             (store_id, phone_idx, phone_last4, role, branch_id, invited_by, expires_at) \
         VALUES ($1, $2, $3, $4::store_member_role, $5, $6, now() + make_interval(days => $7)) \
         RETURNING id",
    )
    .bind(store)
    .bind(&phone_idx)
    .bind(&last4)
    .bind(body.role.as_str())
    .bind(body.branch_id)
    .bind(user.id)
    .bind(INVITE_TTL_DAYS)
    .fetch_one(&mut *tx)
    .await
    .context("insert invite")?;
    for old in replaced {
        audit::record(
            &mut tx,
            Some(user.id),
            "store.invite_revoked",
            "store",
            Some(store),
            json!({ "invite_id": old, "replaced_by": id }),
        )
        .await?;
    }
    audit::record(
        &mut tx,
        Some(user.id),
        "store.invite_sent",
        "store",
        Some(store),
        json!({ "invite_id": id, "phone_last4": last4, "role": body.role.as_str() }),
    )
    .await?;
    // The same lookup runs for every phone; only an account gets the notice.
    let invitee: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users \
         WHERE phone_idx = $1 AND phone_verified_at IS NOT NULL AND deleted_at IS NULL",
    )
    .bind(&phone_idx)
    .fetch_optional(&mut *tx)
    .await
    .context("find invitee")?;
    if let Some(invitee) = invitee {
        enqueue(
            &mut tx,
            Notice::user(invitee, Kind::StoreInvite, format!("store_invite:{id}")),
        )
        .await?;
    }
    tx.commit().await.context("commit")?;
    Ok((
        StatusCode::CREATED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "id": id })),
    ))
}

async fn revoke_invite(
    State(state): State<AppState>,
    user: AuthUser,
    Path((key, id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    require_seller_or_team(&state, &user).await?;
    let (store, id) = (parse_id(&key)?, parse_id(&id)?);
    let mut tx = state.db.begin().await.context("begin")?;
    access::require(&mut tx, store, user.id, Action::ManageTeam).await?;
    let revoked: Option<Uuid> = sqlx::query_scalar(&format!(
        "UPDATE store_invites i SET revoked_at = now() \
         WHERE i.id = $1 AND i.store_id = $2 AND {LIVE} RETURNING i.id"
    ))
    .bind(id)
    .bind(store)
    .fetch_optional(&mut *tx)
    .await
    .context("revoke invite")?;
    revoked.ok_or(AppError::NotFound)?;
    audit::record(
        &mut tx,
        Some(user.id),
        "store.invite_revoked",
        "store",
        Some(store),
        json!({ "invite_id": id }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(StatusCode::NO_CONTENT)
}

/// The owner and managers read the team; staff get 403.
async fn team(
    State(state): State<AppState>,
    user: AuthUser,
    Path(key): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    require_seller_or_team(&state, &user).await?;
    let store = parse_id(&key)?;
    let mut conn = state.db.acquire().await.context("acquire")?;
    let me = access::require(&mut conn, store, user.id, Action::ViewStore).await?;
    if me.role == Role::Staff {
        return Err(AppError::Forbidden("not_allowed"));
    }
    let members = members_of(&mut conn, store, None).await?;
    type Row = (
        Uuid,
        String,
        TeamRole,
        Option<Uuid>,
        Option<String>,
        OffsetDateTime,
    );
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT i.id, i.phone_last4, i.role, b.id, b.name, i.expires_at \
             FROM store_invites i LEFT JOIN store_branches b ON b.id = i.branch_id \
             WHERE i.store_id = $1 AND {LIVE} ORDER BY i.created_at DESC, i.id"
    ))
    .bind(store)
    .fetch_all(&mut *conn)
    .await
    .context("list invites")?;
    let invites = rows
        .into_iter()
        .map(
            |(id, phone_last4, role, b_id, b_name, expires_at)| PendingInvite {
                id,
                phone_last4,
                role,
                branch: branch_ref(b_id, b_name),
                expires_at,
            },
        )
        .collect();
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(TeamView { members, invites }),
    ))
}

/// The store's live members, oldest first, or only `only`.
async fn members_of(
    conn: &mut PgConnection,
    store: Uuid,
    only: Option<Uuid>,
) -> Result<Vec<MemberView>, AppError> {
    type Row = (Uuid, String, TeamRole, Option<Uuid>, Option<String>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT m.user_id, u.display_name, m.role, b.id, b.name \
         FROM store_members m JOIN users u ON u.id = m.user_id \
         LEFT JOIN store_branches b ON b.id = m.branch_id \
         WHERE m.store_id = $1 AND m.removed_at IS NULL AND ($2::uuid IS NULL OR m.user_id = $2) \
         ORDER BY m.created_at, m.id",
    )
    .bind(store)
    .bind(only)
    .fetch_all(&mut *conn)
    .await
    .context("list members")?;
    Ok(rows
        .into_iter()
        .map(|(user_id, display_name, role, b_id, b_name)| MemberView {
            user_id,
            display_name,
            role,
            branch: branch_ref(b_id, b_name),
        })
        .collect())
}

async fn change_member(
    State(state): State<AppState>,
    user: AuthUser,
    Path((key, member)): Path<(String, String)>,
    AppJson(body): AppJson<MemberPatch>,
) -> Result<impl IntoResponse, AppError> {
    require_seller_or_team(&state, &user).await?;
    let (store, member) = (parse_id(&key)?, parse_id(&member)?);
    let mut tx = state.db.begin().await.context("begin")?;
    access::require(&mut tx, store, user.id, Action::ManageTeam).await?;
    lock_store(&mut tx, store).await?;
    let current: Option<(TeamRole, Option<Uuid>)> = sqlx::query_as(
        "SELECT role, branch_id FROM store_members \
         WHERE store_id = $1 AND user_id = $2 AND removed_at IS NULL FOR UPDATE",
    )
    .bind(store)
    .bind(member)
    .fetch_optional(&mut *tx)
    .await
    .context("lock member")?;
    let (role, branch) = current.ok_or(AppError::NotFound)?;
    let new_role = body.role.unwrap_or(role);
    let new_branch = match body.branch_id {
        Some(b) => b,
        // Managers are store-wide, so a promotion drops the branch.
        None if new_role == TeamRole::Manager => None,
        None => branch,
    };
    if new_role == TeamRole::Manager && new_branch.is_some() {
        return Err(staff_only_branch());
    }
    if let Some(b) = new_branch.filter(|b| Some(*b) != branch) {
        check_branch(&mut tx, store, b).await?;
    }
    if (new_role, new_branch) != (role, branch) {
        sqlx::query(
            "UPDATE store_members SET role = $3::store_member_role, branch_id = $4 \
             WHERE store_id = $1 AND user_id = $2 AND removed_at IS NULL",
        )
        .bind(store)
        .bind(member)
        .bind(new_role.as_str())
        .bind(new_branch)
        .execute(&mut *tx)
        .await
        .context("update member")?;
        audit::record(
            &mut tx,
            Some(user.id),
            "store.member_role_changed",
            "store",
            Some(store),
            json!({
                "user_id": member,
                "role": new_role.as_str(),
                "branch_id": new_branch,
                "previous_role": role.as_str(),
                "previous_branch_id": branch,
            }),
        )
        .await?;
    }
    let view = members_of(&mut tx, store, Some(member))
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("member vanished"))?;
    tx.commit().await.context("commit")?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(view)))
}

async fn remove_member(
    State(state): State<AppState>,
    user: AuthUser,
    Path((key, member)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    require_seller_or_team(&state, &user).await?;
    let (store, member) = (parse_id(&key)?, parse_id(&member)?);
    let mut tx = state.db.begin().await.context("begin")?;
    access::require(&mut tx, store, user.id, Action::ManageTeam).await?;
    lock_store(&mut tx, store).await?;
    let ended: Option<Uuid> = sqlx::query_scalar(
        "UPDATE store_members SET removed_at = now(), removed_by = $3 \
         WHERE store_id = $1 AND user_id = $2 AND removed_at IS NULL RETURNING id",
    )
    .bind(store)
    .bind(member)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await
    .context("remove member")?;
    ended.ok_or(AppError::NotFound)?;
    audit::record(
        &mut tx,
        Some(user.id),
        "store.member_removed",
        "store",
        Some(store),
        json!({ "user_id": member }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(StatusCode::NO_CONTENT)
}

/// SQL: the open invite `$1` is to the verified phone of user `$2`.
const MINE: &str = "i.id = $1 AND i.phone_idx = (SELECT u.phone_idx FROM users u \
                    WHERE u.id = $2 AND u.phone_verified_at IS NOT NULL)";

async fn my_invites(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    type Row = (
        Uuid,
        Uuid,
        String,
        String,
        Option<String>,
        TeamRole,
        Option<Uuid>,
        Option<String>,
        OffsetDateTime,
    );
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT i.id, s.id, s.name, s.slug, s.logo_key, i.role, b.id, b.name, \
                i.expires_at \
         FROM store_invites i \
         JOIN users u ON u.id = $1 AND u.phone_idx = i.phone_idx \
                     AND u.phone_verified_at IS NOT NULL \
         JOIN stores s ON s.id = i.store_id AND s.deleted_at IS NULL \
         LEFT JOIN store_branches b ON b.id = i.branch_id \
         WHERE {LIVE} ORDER BY i.created_at DESC, i.id"
    ))
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    .context("list my invites")?;
    let invites = rows
        .into_iter()
        .map(
            |(id, store_id, name, slug, logo_key, role, b_id, b_name, expires_at)| MyInvite {
                id,
                store: InviteStore {
                    id: store_id,
                    name,
                    slug,
                    logo_url: logo_key
                        .as_deref()
                        .and_then(|k| state.objects.public_url(k)),
                },
                role,
                branch: branch_ref(b_id, b_name),
                expires_at,
            },
        )
        .collect::<Vec<_>>();
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(invites)))
}

/// 404 unless the invite is open and to the caller's verified phone.
async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let id = parse_id(&id)?;
    let mut tx = state.db.begin().await.context("begin")?;
    let store: Uuid = sqlx::query_scalar(&format!(
        "SELECT i.store_id FROM store_invites i WHERE {MINE} AND {LIVE}"
    ))
    .bind(id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await
    .context("find invite")?
    .ok_or(AppError::NotFound)?;
    // The store first (the lock order), then the invite; decline and revoke lock
    // only the invite, so they cannot deadlock with this.
    let owner = lock_store(&mut tx, store).await?;
    let invite: Option<(TeamRole, Option<Uuid>, Uuid)> = sqlx::query_as(&format!(
        "SELECT i.role, i.branch_id, i.invited_by FROM store_invites i \
         WHERE {MINE} AND {LIVE} FOR UPDATE"
    ))
    .bind(id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await
    .context("lock invite")?;
    let (role, branch, invited_by) = invite.ok_or(AppError::NotFound)?;
    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM store_members \
         WHERE store_id = $1 AND user_id = $2 AND removed_at IS NULL)",
    )
    .bind(store)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await
    .context("check membership")?;
    if member || owner == user.id {
        return Err(AppError::ConflictDetail(
            "already_member",
            "You are already on this store's team".into(),
        ));
    }
    if live_member_count(&mut tx, store).await? >= MAX_TEAM_MEMBERS {
        return Err(team_full());
    }
    if let Some(b) = branch {
        check_branch(&mut tx, store, b).await?;
    }
    sqlx::query(
        "INSERT INTO store_members (store_id, user_id, role, branch_id, added_by) \
         VALUES ($1, $2, $3::store_member_role, $4, $5)",
    )
    .bind(store)
    .bind(user.id)
    .bind(role.as_str())
    .bind(branch)
    .bind(invited_by)
    .execute(&mut *tx)
    .await
    .context("add member")?;
    sqlx::query("UPDATE store_invites SET accepted_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .context("accept invite")?;
    audit::record(
        &mut tx,
        Some(user.id),
        "store.member_added",
        "store",
        Some(store),
        json!({
            "user_id": user.id,
            "invite_id": id,
            "role": role.as_str(),
            "branch_id": branch,
        }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "store_id": store, "role": role, "branch_id": branch })),
    ))
}

async fn decline(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let id = parse_id(&id)?;
    let mut tx = state.db.begin().await.context("begin")?;
    let store: Uuid = sqlx::query_scalar(&format!(
        "UPDATE store_invites i SET declined_at = now() WHERE {MINE} AND {LIVE} \
         RETURNING i.store_id"
    ))
    .bind(id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await
    .context("decline invite")?
    .ok_or(AppError::NotFound)?;
    audit::record(
        &mut tx,
        Some(user.id),
        "store.invite_declined",
        "store",
        Some(store),
        json!({ "invite_id": id }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(StatusCode::NO_CONTENT)
}

/// Job 17, `purge_invites`: deletes invites that expired unanswered, or were
/// declined or revoked, more than [`INVITE_RETENTION_DAYS`] ago. An accepted invite
/// stays with the membership it explains.
pub async fn purge_invites(
    state: &AppState,
    _cancel: &CancellationToken,
) -> Result<Stats, AppError> {
    let purged = sqlx::query(
        "DELETE FROM store_invites WHERE id IN ( \
             SELECT id FROM store_invites \
             WHERE accepted_at IS NULL \
               AND COALESCE(declined_at, revoked_at, expires_at) \
                   < now() - make_interval(days => $1) \
             ORDER BY created_at LIMIT $2)",
    )
    .bind(INVITE_RETENTION_DAYS)
    .bind(BATCH)
    .execute(&state.db)
    .await
    .context("purge old invites")?
    .rows_affected();
    Ok(Stats {
        done: u32::try_from(purged).unwrap_or(u32::MAX),
        failed: 0,
    })
}

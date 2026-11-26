//! A store's branches: each has its own pickup address, phone, self-delivery fee and
//! stock. The store's first branch (position 0) is made with the store; the owner adds,
//! edits and closes the others. Any member reads the list, but only the owner and
//! managers see a branch's address and phone (staff get its name and area council).
//!
//! **Closing** is refused while the branch has an order not yet in a final state
//! (409 `branch_has_open_orders`) or when it is the last open branch (409
//! `last_branch`); the first branch cannot close yet (409 `first_branch`), since
//! checkout and deliveries still send from it. It takes the branch's stock out of the variant totals and revokes
//! open invites tied to it. Staff tied to it stay in the team but lose its orders and
//! stock ([`access`] treats a closed branch as nobody's) until the owner moves them to
//! another branch. Positions are never reused, so a closed branch keeps its own.
//!
//! Until checkout quotes per branch (tasks 7 and 8), deliveries read the store's own
//! pickup columns, so an edit of the first branch here is copied onto them.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    access::{self, Action, Role},
    routes::require_seller_or_team,
    service::{parse_postcode, validate_address, validate_delivery_details, validate_name},
    team::LIVE,
};
use crate::{
    AppState, audit, auth::AuthUser, crypto::FieldCipher, delivery::area::AreaCouncil,
    error::AppError, extract::AppJson, limits::MAX_OPEN_BRANCHES, phone::normalize_ng_phone, stock,
};

/// Order states after which a branch owes nothing more on the order.
const FINAL_ORDER_STATES: &str = "('completed', 'refunded', 'cancelled')";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/stores/{key}/branches", get(list).post(create))
        .route("/stores/{key}/branches/{id}", patch(update))
        .route("/stores/{key}/branches/{id}/close", post(close))
}

fn parse_id(key: &str) -> Result<Uuid, AppError> {
    key.parse().map_err(|_| AppError::NotFound)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewBranch {
    name: String,
    pickup_address: String,
    pickup_area_council: String,
    pickup_postcode: Option<String>,
    business_phone: String,
    self_delivery_fee_kobo: Option<i64>,
    #[serde(default)]
    collect_enabled: bool,
}

/// Absent keeps a field; a blank postcode clears it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BranchPatch {
    name: Option<String>,
    pickup_address: Option<String>,
    pickup_area_council: Option<String>,
    pickup_postcode: Option<String>,
    business_phone: Option<String>,
    self_delivery_fee_kobo: Option<i64>,
    collect_enabled: Option<bool>,
}

/// A branch as stored, address and phone still encrypted. No `Debug`: nothing here
/// is ever logged.
#[derive(sqlx::FromRow)]
struct Branch {
    id: Uuid,
    name: String,
    pickup_address_enc: Vec<u8>,
    pickup_area_council: Option<AreaCouncil>,
    pickup_postcode: Option<String>,
    business_phone_enc: Option<Vec<u8>>,
    self_delivery_fee_kobo: Option<i64>,
    pickup_courier_address_code: Option<i64>,
    collect_enabled: bool,
    position: i32,
    closed_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
}

const COLUMNS: &str = "id, name, pickup_address_enc, pickup_area_council, pickup_postcode, \
                       business_phone_enc, self_delivery_fee_kobo, pickup_courier_address_code, \
                       collect_enabled, position, closed_at, created_at";

/// What every member sees of a branch; the owner and managers also get `details`.
#[derive(Serialize)]
struct BranchView {
    id: Uuid,
    name: String,
    pickup_area_council: Option<AreaCouncil>,
    collect_enabled: bool,
    position: i32,
    #[serde(with = "time::serde::rfc3339::option")]
    closed_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(flatten)]
    details: Option<BranchDetails>,
}

#[derive(Serialize)]
struct BranchDetails {
    pickup_address: String,
    pickup_postcode: Option<String>,
    business_phone: Option<String>,
    self_delivery_fee_kobo: Option<i64>,
}

impl Branch {
    fn view(&self, cipher: &FieldCipher, full: bool) -> Result<BranchView, AppError> {
        let details = if full {
            Some(BranchDetails {
                pickup_address: cipher.decrypt(&self.pickup_address_enc)?,
                pickup_postcode: self.pickup_postcode.clone(),
                business_phone: self.phone(cipher)?,
                self_delivery_fee_kobo: self.self_delivery_fee_kobo,
            })
        } else {
            None
        };
        Ok(BranchView {
            id: self.id,
            name: self.name.clone(),
            pickup_area_council: self.pickup_area_council,
            collect_enabled: self.collect_enabled,
            position: self.position,
            closed_at: self.closed_at,
            created_at: self.created_at,
            details,
        })
    }

    fn phone(&self, cipher: &FieldCipher) -> Result<Option<String>, AppError> {
        self.business_phone_enc
            .as_deref()
            .map(|b| cipher.decrypt(b))
            .transpose()
    }
}

/// The store's delivery flags (store-wide), with the store row locked.
struct StoreFlags {
    courier: bool,
    own: bool,
}

/// Locks a live store row (after `require` has said who may act on it).
async fn lock_store(conn: &mut PgConnection, store: Uuid) -> Result<StoreFlags, AppError> {
    let row: (bool, bool) = sqlx::query_as(
        "SELECT delivery_courier, delivery_self FROM stores \
         WHERE id = $1 AND deleted_at IS NULL FOR NO KEY UPDATE",
    )
    .bind(store)
    .fetch_optional(&mut *conn)
    .await
    .context("lock store")?
    .ok_or(AppError::NotFound)?;
    Ok(StoreFlags {
        courier: row.0,
        own: row.1,
    })
}

/// The store's branch `id` (404 if it is another store's), locked when `lock`.
async fn load_branch(
    conn: &mut PgConnection,
    store: Uuid,
    id: Uuid,
    lock: bool,
) -> Result<Branch, AppError> {
    let lock = if lock { "FOR NO KEY UPDATE" } else { "" };
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM store_branches WHERE id = $1 AND store_id = $2 {lock}"
    ))
    .bind(id)
    .bind(store)
    .fetch_optional(&mut *conn)
    .await
    .context("load branch")?
    .ok_or(AppError::NotFound)
}

fn branch_closed() -> AppError {
    AppError::ConflictDetail("branch_closed", "That branch is closed".into())
}

async fn open_count(conn: &mut PgConnection, store: Uuid) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM store_branches WHERE store_id = $1 AND closed_at IS NULL",
    )
    .bind(store)
    .fetch_one(&mut *conn)
    .await
    .context("count open branches")?)
}

fn parse_area(raw: &str) -> Result<Option<AreaCouncil>, AppError> {
    match raw.trim() {
        "" => Ok(None),
        a => Ok(Some(a.parse()?)),
    }
}

fn no_store() -> [(header::HeaderName, &'static str); 1] {
    [(header::CACHE_CONTROL, "no-store")]
}

/// `GET /stores/{key}/branches`: every branch, first branch first, for any member.
async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(key): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    require_seller_or_team(&state, &user).await?;
    let store = parse_id(&key)?;
    let mut conn = state.db.acquire().await.context("acquire")?;
    let me = access::require(&mut conn, store, user.id, Action::ViewStore).await?;
    let full = me.role != Role::Staff;
    let branches: Vec<Branch> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM store_branches WHERE store_id = $1 ORDER BY position"
    ))
    .bind(store)
    .fetch_all(&mut *conn)
    .await
    .context("list branches")?;
    let views = branches
        .iter()
        .map(|b| b.view(&state.cipher, full))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((no_store(), Json(views)))
}

/// `POST /stores/{key}/branches` (owner).
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(key): Path<String>,
    AppJson(body): AppJson<NewBranch>,
) -> Result<impl IntoResponse, AppError> {
    require_seller_or_team(&state, &user).await?;
    let store = parse_id(&key)?;
    let name = validate_name(&body.name)?;
    let address = validate_address(&body.pickup_address)?;
    let area = parse_area(&body.pickup_area_council)?;
    let postcode = body
        .pickup_postcode
        .as_deref()
        .map(parse_postcode)
        .transpose()?
        .flatten();
    let phone = match body.business_phone.trim() {
        "" => None,
        p => Some(normalize_ng_phone(p)?),
    };
    let address_enc = state.cipher.encrypt(&address);
    let phone_enc = phone.as_deref().map(|p| state.cipher.encrypt(p));

    let mut tx = state.db.begin().await.context("begin")?;
    access::require(&mut tx, store, user.id, Action::ManageBranches).await?;
    let flags = lock_store(&mut tx, store).await?;
    validate_delivery_details(
        flags.courier,
        flags.own,
        body.self_delivery_fee_kobo,
        phone.is_some(),
        area,
    )?;
    if open_count(&mut tx, store).await? >= MAX_OPEN_BRANCHES {
        return Err(AppError::ConflictDetail(
            "branch_limit",
            format!("A store can have at most {MAX_OPEN_BRANCHES} open branches"),
        ));
    }
    // The next free position; a closed branch keeps its own (the store lock serialises this).
    let branch: Branch = sqlx::query_as(&format!(
        "INSERT INTO store_branches (store_id, name, pickup_address_enc, pickup_area_council, \
                                     pickup_postcode, business_phone_enc, self_delivery_fee_kobo, \
                                     collect_enabled, position) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, \
                 (SELECT COALESCE(max(position) + 1, 0) FROM store_branches WHERE store_id = $1)) \
         RETURNING {COLUMNS}"
    ))
    .bind(store)
    .bind(&name)
    .bind(&address_enc)
    .bind(area)
    .bind(&postcode)
    .bind(&phone_enc)
    .bind(body.self_delivery_fee_kobo)
    .bind(body.collect_enabled)
    .fetch_one(&mut *tx)
    .await
    .context("insert branch")?;
    audit::record(
        &mut tx,
        Some(user.id),
        "store.branch_added",
        "store",
        Some(store),
        json!({ "branch_id": branch.id }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok((
        StatusCode::CREATED,
        no_store(),
        Json(branch.view(&state.cipher, true)?),
    ))
}

/// `PATCH /stores/{key}/branches/{id}` (owner). A real move of the address, or a new
/// phone, drops the courier's saved address code (checkout's delivery options refill it).
async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path((key, id)): Path<(String, String)>,
    AppJson(patch): AppJson<BranchPatch>,
) -> Result<impl IntoResponse, AppError> {
    require_seller_or_team(&state, &user).await?;
    let (store, id) = (parse_id(&key)?, parse_id(&id)?);
    let cipher = &state.cipher;
    let mut tx = state.db.begin().await.context("begin")?;
    access::require(&mut tx, store, user.id, Action::ManageBranches).await?;
    let flags = lock_store(&mut tx, store).await?;
    let current = load_branch(&mut tx, store, id, true).await?;
    if current.closed_at.is_some() {
        return Err(branch_closed());
    }

    let name = match &patch.name {
        Some(n) => validate_name(n)?,
        None => current.name.clone(),
    };
    let new_address = patch
        .pickup_address
        .as_deref()
        .map(validate_address)
        .transpose()?;
    let area = match patch.pickup_area_council.as_deref().map(parse_area) {
        Some(a) => a?.or(current.pickup_area_council),
        None => current.pickup_area_council,
    };
    let postcode = match patch.pickup_postcode.as_deref() {
        Some(p) => parse_postcode(p)?,
        None => current.pickup_postcode.clone(),
    };
    let new_phone = patch
        .business_phone
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .map(normalize_ng_phone)
        .transpose()?;
    let fee = patch
        .self_delivery_fee_kobo
        .or(current.self_delivery_fee_kobo);
    let collect = patch.collect_enabled.unwrap_or(current.collect_enabled);

    // Field names only, never values.
    let mut changed: Vec<&str> = Vec::new();
    if name != current.name {
        changed.push("name");
    }
    let address_enc = match &new_address {
        Some(a) if cipher.decrypt(&current.pickup_address_enc)? != *a => {
            changed.push("pickup_address");
            cipher.encrypt(a)
        }
        _ => current.pickup_address_enc.clone(),
    };
    if area != current.pickup_area_council {
        changed.push("pickup_area_council");
    }
    if postcode != current.pickup_postcode {
        changed.push("pickup_postcode");
    }
    let phone_enc = match &new_phone {
        Some(p) if current.phone(cipher)?.as_ref() != Some(p) => {
            changed.push("business_phone");
            Some(cipher.encrypt(p))
        }
        _ => current.business_phone_enc.clone(),
    };
    if fee != current.self_delivery_fee_kobo {
        changed.push("self_delivery_fee_kobo");
    }
    if collect != current.collect_enabled {
        changed.push("collect_enabled");
    }
    let touched = patch.pickup_area_council.is_some()
        || patch.pickup_postcode.is_some()
        || patch.business_phone.is_some()
        || patch.self_delivery_fee_kobo.is_some();
    // A legacy first branch with NULL delivery fields stays editable until one is touched.
    if touched {
        validate_delivery_details(flags.courier, flags.own, fee, phone_enc.is_some(), area)?;
    }
    let moved = ["pickup_address", "pickup_area_council", "pickup_postcode"]
        .iter()
        .any(|f| changed.contains(f));
    let code_stale = moved || changed.contains(&"business_phone");
    let courier_code = if code_stale {
        None
    } else {
        current.pickup_courier_address_code
    };

    let branch: Branch = sqlx::query_as(&format!(
        "UPDATE store_branches SET name = $2, pickup_address_enc = $3, pickup_area_council = $4, \
                                   pickup_postcode = $5, business_phone_enc = $6, \
                                   self_delivery_fee_kobo = $7, collect_enabled = $8, \
                                   pickup_courier_address_code = $9 \
         WHERE id = $1 RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(&name)
    .bind(&address_enc)
    .bind(area)
    .bind(&postcode)
    .bind(&phone_enc)
    .bind(fee)
    .bind(collect)
    .bind(courier_code)
    .fetch_one(&mut *tx)
    .await
    .context("update branch")?;
    if branch.position == 0 {
        // Deliveries still read the store's pickup columns (until tasks 7 and 8).
        sqlx::query(
            "UPDATE stores SET pickup_address_enc = $2, pickup_area_council = $3, \
                               pickup_postcode = $4, business_phone_enc = $5, \
                               self_delivery_fee_kobo = $6, \
                               pickup_courier_address_code = \
                                   CASE WHEN $7 THEN NULL ELSE pickup_courier_address_code END, \
                               pickup_moved_at = CASE WHEN $8 THEN now() ELSE pickup_moved_at END \
             WHERE id = $1",
        )
        .bind(store)
        .bind(&address_enc)
        .bind(area)
        .bind(&postcode)
        .bind(&phone_enc)
        .bind(fee)
        .bind(code_stale)
        .bind(moved)
        .execute(&mut *tx)
        .await
        .context("copy first branch to store")?;
    }
    audit::record(
        &mut tx,
        Some(user.id),
        "store.branch_updated",
        "store",
        Some(store),
        json!({ "branch_id": id, "fields_changed": changed }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok((no_store(), Json(branch.view(cipher, true)?)))
}

/// `POST /stores/{key}/branches/{id}/close` (owner). Locks the store, the branch, then
/// (in [`stock::close_branch`]) the store's variants and the branch's stock rows; the
/// open-order check runs after those locks, so a checkout still reserving at the
/// branch is seen.
async fn close(
    State(state): State<AppState>,
    user: AuthUser,
    Path((key, id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    require_seller_or_team(&state, &user).await?;
    let (store, id) = (parse_id(&key)?, parse_id(&id)?);
    let mut tx = state.db.begin().await.context("begin")?;
    access::require(&mut tx, store, user.id, Action::ManageBranches).await?;
    lock_store(&mut tx, store).await?;
    let current = load_branch(&mut tx, store, id, true).await?;
    if current.closed_at.is_some() {
        return Err(branch_closed());
    }
    if open_count(&mut tx, store).await? <= 1 {
        return Err(AppError::ConflictDetail(
            "last_branch",
            "A store needs at least one open branch".into(),
        ));
    }
    // Task 8 lifts this once checkout, options and PATCH /stores stop using position 0.
    if current.position == 0 {
        return Err(AppError::ConflictDetail(
            "first_branch",
            "The store's first branch cannot be closed yet".into(),
        ));
    }
    let branch: Branch = sqlx::query_as(&format!(
        "UPDATE store_branches SET closed_at = now() WHERE id = $1 RETURNING {COLUMNS}"
    ))
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .context("close branch")?;
    stock::close_branch(&mut tx, id).await?;
    let open_orders: bool = sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM orders \
         WHERE branch_id = $1 AND status NOT IN {FINAL_ORDER_STATES})"
    ))
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .context("check open orders")?;
    if open_orders {
        return Err(AppError::ConflictDetail(
            "branch_has_open_orders",
            "This branch still has orders to finish".into(),
        ));
    }
    let revoked: Vec<Uuid> = sqlx::query_scalar(&format!(
        "UPDATE store_invites i SET revoked_at = now() \
         WHERE i.store_id = $1 AND i.branch_id = $2 AND {LIVE} RETURNING i.id"
    ))
    .bind(store)
    .bind(id)
    .fetch_all(&mut *tx)
    .await
    .context("revoke tied invites")?;
    for invite in revoked {
        audit::record(
            &mut tx,
            Some(user.id),
            "store.invite_revoked",
            "store",
            Some(store),
            json!({ "invite_id": invite, "reason": "branch_closed" }),
        )
        .await?;
    }
    audit::record(
        &mut tx,
        Some(user.id),
        "store.branch_closed",
        "store",
        Some(store),
        json!({ "branch_id": id }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok((no_store(), Json(branch.view(&state.cipher, true)?)))
}

//! Stock per branch. `branch_stock` is the truth; a variant's `stock` and
//! `reserved_quantity` are its totals over the store's **open** branches, kept equal
//! to that sum in the same transaction by [`apply`], the only writer of either
//! ([`set_stock`] goes through it).
//!
//! Lock order (extends the documented one, never reorders it): ... products, the
//! variant row (`FOR NO KEY UPDATE`), then its `branch_stock` row (`FOR UPDATE`).
//! Callers touching several variants call in variant id order.
//!
//! A buyer never gets a line split across branches, so what a buyer sees or is
//! capped by is [`MAX_AVAILABLE_SQL`]: the most one open branch can send.

use std::collections::BTreeMap;

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::put,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{
    AppState, audit,
    auth::AuthUser,
    error::AppError,
    extract::AppJson,
    limits::MAX_STOCK,
    stores::{
        access::{self, Action},
        routes::require_seller_or_team,
    },
};

/// Buyer-facing units of the variant aliased `v`: the largest `stock - reserved`
/// at one open branch (0 when none holds any). A SQL expression.
pub const MAX_AVAILABLE_SQL: &str = "COALESCE((SELECT va.max_available FROM variant_availability va \
     WHERE va.variant_id = v.id), 0)";

/// Units one branch can still sell of a variant: a SQL expression over a
/// `branch_stock` row aliased `bs`. The same difference `variant_availability` sums.
pub const BRANCH_AVAILABLE_SQL: &str = "(bs.stock - bs.reserved_quantity)";

/// Most items one `PUT /branches/{id}/stock` may set.
const MAX_ITEMS: usize = 200;

pub(crate) fn out_of_stock(variant: Uuid) -> AppError {
    AppError::ConflictDetail(
        "out_of_stock",
        format!("Item {variant} does not have enough stock"),
    )
}

pub(crate) fn below_reserved() -> AppError {
    AppError::ConflictDetail(
        "stock_below_reserved",
        "Stock cannot be set below the units reserved by pending checkouts".into(),
    )
}

/// Lock timeouts and deadlocks are 409 `stock_busy`; anything else is internal.
fn db_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> AppError {
    crate::checkout::service::busy_err("stock_busy", ctx)
}

/// One branch's counts for a variant, read under both locks.
pub struct Counts {
    pub stock: i32,
    pub reserved: i32,
    pub branch_open: bool,
}

/// Locks the variant row, then its `branch_stock` row at `branch` (creating an
/// empty one if missing). A branch that is not in the variant's store is an error,
/// never a silent skip.
pub async fn lock_counts(
    conn: &mut PgConnection,
    branch: Uuid,
    variant: Uuid,
) -> Result<Counts, AppError> {
    let store: Option<Uuid> = sqlx::query_scalar(
        "SELECT p.store_id FROM product_variants v JOIN products p ON p.id = v.product_id \
         WHERE v.id = $1 FOR NO KEY UPDATE OF v",
    )
    .bind(variant)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db_err("lock variant"))?;
    // A statement of its own: a statement's snapshot is taken before it waits for a
    // lock, so reading the branch in the locking one could miss a close
    // ([`close_branch`]) that committed while this waited.
    let open: Option<bool> = match store {
        Some(store) => sqlx::query_scalar(
            "SELECT closed_at IS NULL FROM store_branches WHERE id = $1 AND store_id = $2",
        )
        .bind(branch)
        .bind(store)
        .fetch_optional(&mut *conn)
        .await
        .context("load branch")?,
        None => None,
    };
    let branch_open = open.ok_or_else(|| {
        anyhow::anyhow!("branch {branch} is not a branch of variant {variant}'s store")
    })?;
    let row: Option<(i32, i32)> = sqlx::query_as(
        "SELECT stock, reserved_quantity FROM branch_stock \
         WHERE branch_id = $1 AND variant_id = $2 FOR UPDATE",
    )
    .bind(branch)
    .bind(variant)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db_err("lock branch stock"))?;
    let (stock, reserved) = match row {
        Some(r) => r,
        None => {
            // The variant lock is held, so no other writer can insert this row now.
            sqlx::query(
                "INSERT INTO branch_stock (branch_id, variant_id, stock, reserved_quantity) \
                 VALUES ($1, $2, 0, 0)",
            )
            .bind(branch)
            .bind(variant)
            .execute(&mut *conn)
            .await
            .context("insert branch stock")?;
            (0, 0)
        }
    };
    Ok(Counts {
        stock,
        reserved,
        branch_open,
    })
}

/// Units a buyer can take from `branch` now (0 at a closed branch), with the
/// variant and branch rows locked for the rest of the caller's transaction.
pub async fn lock_available(
    conn: &mut PgConnection,
    branch: Uuid,
    variant: Uuid,
) -> Result<i32, AppError> {
    let row = lock_counts(conn, branch, variant).await?;
    Ok(if row.branch_open {
        row.stock - row.reserved
    } else {
        0
    })
}

/// The only writer. Locks the variant row then the branch_stock row (lock order), applies
/// the deltas to both, and errors 409 `out_of_stock` if a check would fail.
///
/// The variant's totals move only while the branch is open (they are sums over open
/// branches). A closed branch takes no new reservation.
pub async fn apply(
    conn: &mut PgConnection,
    branch: Uuid,
    variant: Uuid,
    d_stock: i32,
    d_reserved: i32,
) -> Result<(), AppError> {
    let row = lock_counts(conn, branch, variant).await?;
    let stock = i64::from(row.stock) + i64::from(d_stock);
    let reserved = i64::from(row.reserved) + i64::from(d_reserved);
    if reserved < 0
        || reserved > stock
        || stock > i64::from(MAX_STOCK)
        || (!row.branch_open && d_reserved > 0)
    {
        return Err(out_of_stock(variant));
    }
    if d_stock == 0 && d_reserved == 0 {
        return Ok(());
    }
    sqlx::query(
        "UPDATE branch_stock SET stock = stock + $3, reserved_quantity = reserved_quantity + $4 \
         WHERE branch_id = $1 AND variant_id = $2",
    )
    .bind(branch)
    .bind(variant)
    .bind(d_stock)
    .bind(d_reserved)
    .execute(&mut *conn)
    .await
    .context("update branch stock")?;
    if row.branch_open {
        sqlx::query(
            "UPDATE product_variants \
             SET stock = stock + $2, reserved_quantity = reserved_quantity + $3 WHERE id = $1",
        )
        .bind(variant)
        .bind(d_stock)
        .bind(d_reserved)
        .execute(&mut *conn)
        .await
        .context("update variant totals")?;
    }
    Ok(())
}

/// Sets the on-hand count at one branch. Below what pending checkouts reserve
/// there is 409 `stock_below_reserved` (never clamped); above the cap, 409
/// `out_of_stock` (callers validate the range first).
pub async fn set_stock(
    conn: &mut PgConnection,
    branch: Uuid,
    variant: Uuid,
    stock: i32,
) -> Result<(), AppError> {
    let row = lock_counts(conn, branch, variant).await?;
    if stock < row.reserved {
        return Err(below_reserved());
    }
    apply(conn, branch, variant, stock - row.stock, 0).await
}

/// Takes a closing branch's counts out of its variants' totals, so they stay the sum
/// over open branches. The caller has already marked the branch closed in this
/// transaction (and locked the store, then the branch row).
///
/// Locks every variant of the store in id order, then the branch's rows (the lock
/// order). A checkout reserving at the branch holds a variant lock, so it finishes
/// first and the caller sees its order afterwards; one that comes after waits for this
/// transaction and then finds the branch closed. The branch's own rows keep their
/// counts: a commit or refund of one of its orders still moves only that row.
pub async fn close_branch(conn: &mut PgConnection, branch: Uuid) -> Result<(), AppError> {
    let closed: bool =
        sqlx::query_scalar("SELECT closed_at IS NOT NULL FROM store_branches WHERE id = $1")
            .bind(branch)
            .fetch_one(&mut *conn)
            .await
            .context("load branch")?;
    if !closed {
        return Err(anyhow::anyhow!("close_branch called on open branch {branch}").into());
    }
    sqlx::query(
        "SELECT v.id FROM product_variants v JOIN products p ON p.id = v.product_id \
         JOIN store_branches b ON b.store_id = p.store_id \
         WHERE b.id = $1 ORDER BY v.id FOR NO KEY UPDATE OF v",
    )
    .bind(branch)
    .execute(&mut *conn)
    .await
    .map_err(db_err("lock variants"))?;
    let rows: Vec<(Uuid, i32, i32)> = sqlx::query_as(
        "SELECT variant_id, stock, reserved_quantity FROM branch_stock \
         WHERE branch_id = $1 ORDER BY variant_id FOR UPDATE",
    )
    .bind(branch)
    .fetch_all(&mut *conn)
    .await
    .map_err(db_err("lock branch stock"))?;
    for (variant, stock, reserved) in rows {
        if stock == 0 && reserved == 0 {
            continue;
        }
        sqlx::query(
            "UPDATE product_variants \
             SET stock = stock - $2, reserved_quantity = reserved_quantity - $3 WHERE id = $1",
        )
        .bind(variant)
        .bind(stock)
        .bind(reserved)
        .execute(&mut *conn)
        .await
        .context("update variant totals")?;
    }
    Ok(())
}

/// The count a seller asked for: one number (a store with one open branch), or a
/// count per branch.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum StockInput {
    One(i32),
    PerBranch(BTreeMap<Uuid, i32>),
}

fn check_count(n: i32) -> Result<(), AppError> {
    if n < 0 {
        return Err(AppError::Validation(
            "variant stock must not be negative".into(),
        ));
    }
    if n > MAX_STOCK {
        return Err(AppError::Validation(format!(
            "variant stock must be at most {MAX_STOCK}"
        )));
    }
    Ok(())
}

impl StockInput {
    /// The range check, before anything is read.
    pub fn validate(&self) -> Result<(), AppError> {
        match self {
            StockInput::One(n) => check_count(*n),
            StockInput::PerBranch(m) => m.values().try_for_each(|n| check_count(*n)),
        }
    }

    /// Count per branch, given the store's open branches. A plain number with two
    /// or more open branches is 422 `branch_required`; a branch that is not open in
    /// this store is 422 `unknown_branch`.
    pub fn resolve(&self, open_branches: &[Uuid]) -> Result<BTreeMap<Uuid, i32>, AppError> {
        match self {
            StockInput::One(n) => match open_branches {
                [only] => Ok(BTreeMap::from([(*only, *n)])),
                _ => Err(AppError::Unprocessable(
                    "branch_required",
                    "This store has more than one branch: give the stock per branch".into(),
                )),
            },
            StockInput::PerBranch(m) => {
                if m.keys().any(|b| !open_branches.contains(b)) {
                    return Err(AppError::Unprocessable(
                        "unknown_branch",
                        "Stock names a branch that is not open in this store".into(),
                    ));
                }
                Ok(m.clone())
            }
        }
    }
}

/// The store's open branches, first branch first.
pub async fn open_branches(conn: &mut PgConnection, store_id: Uuid) -> Result<Vec<Uuid>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM store_branches WHERE store_id = $1 AND closed_at IS NULL ORDER BY position",
    )
    .bind(store_id)
    .fetch_all(&mut *conn)
    .await
    .context("load open branches")?)
}

/// On-hand counts per (variant, branch) for these variants.
pub async fn counts(
    conn: &mut PgConnection,
    variants: &[Uuid],
) -> Result<BTreeMap<(Uuid, Uuid), i32>, AppError> {
    let rows: Vec<(Uuid, Uuid, i32)> = sqlx::query_as(
        "SELECT variant_id, branch_id, stock FROM branch_stock WHERE variant_id = ANY($1)",
    )
    .bind(variants)
    .fetch_all(&mut *conn)
    .await
    .context("load branch stock")?;
    Ok(rows.into_iter().map(|(v, b, s)| ((v, b), s)).collect())
}

/// One branch's counts for a variant, as the store's team sees them.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct BranchCount {
    pub branch_id: Uuid,
    pub stock: i32,
    pub reserved_quantity: i32,
}

#[derive(Debug, Deserialize)]
pub struct BranchStockItem {
    pub variant_id: Uuid,
    pub stock: i32,
}

#[derive(Debug, Deserialize)]
pub struct BranchStockRequest {
    pub items: Vec<BranchStockItem>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct BranchStockLine {
    pub variant_id: Uuid,
    pub stock: i32,
    pub reserved_quantity: i32,
}

#[derive(Debug, Serialize)]
pub struct BranchStockView {
    pub branch_id: Uuid,
    pub items: Vec<BranchStockLine>,
}

/// `PUT /branches/{id}/stock`: sets counts for many variants at one branch. Needs
/// [`Action::EditStock`] for that branch; a stranger (or a branch that does not
/// exist or is closed) gets 404. A variant that is not a live variant of this store
/// is 422 `unknown_variant`.
pub async fn set_branch_stock(
    db: &sqlx::PgPool,
    actor: Uuid,
    branch: Uuid,
    request: BranchStockRequest,
) -> Result<BranchStockView, AppError> {
    let mut items = request.items;
    if items.is_empty() || items.len() > MAX_ITEMS {
        return Err(AppError::Validation(format!(
            "items must contain 1 to {MAX_ITEMS} entries"
        )));
    }
    items.iter().try_for_each(|i| check_count(i.stock))?;
    // Variant id order: the lock order for several variants.
    items.sort_by_key(|i| i.variant_id);
    if items.windows(2).any(|w| w[0].variant_id == w[1].variant_id) {
        return Err(AppError::Validation("a variant is listed twice".into()));
    }

    let mut tx = db.begin().await.context("begin")?;
    let store_id: Uuid = sqlx::query_scalar(
        "SELECT store_id FROM store_branches WHERE id = $1 AND closed_at IS NULL",
    )
    .bind(branch)
    .fetch_optional(&mut *tx)
    .await
    .context("load branch")?
    .ok_or(AppError::NotFound)?;
    access::require(&mut tx, store_id, actor, Action::EditStock(branch)).await?;

    let ids: Vec<Uuid> = items.iter().map(|i| i.variant_id).collect();
    let known: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM product_variants v JOIN products p ON p.id = v.product_id \
         WHERE v.id = ANY($1) AND p.store_id = $2 AND p.deleted_at IS NULL",
    )
    .bind(&ids)
    .bind(store_id)
    .fetch_one(&mut *tx)
    .await
    .context("check variants")?;
    if usize::try_from(known).ok() != Some(ids.len()) {
        return Err(AppError::Unprocessable(
            "unknown_variant",
            "An item is not a product of this store".into(),
        ));
    }
    for item in &items {
        set_stock(&mut tx, branch, item.variant_id, item.stock).await?;
    }
    audit::record(
        &mut tx,
        Some(actor),
        "branch.stock_set",
        "branch",
        Some(branch),
        json!({ "variant_ids": ids }),
    )
    .await?;
    let lines: Vec<BranchStockLine> = sqlx::query_as(
        "SELECT variant_id, stock, reserved_quantity FROM branch_stock \
         WHERE branch_id = $1 AND variant_id = ANY($2) ORDER BY variant_id",
    )
    .bind(branch)
    .bind(&ids)
    .fetch_all(&mut *tx)
    .await
    .context("load branch stock")?;
    tx.commit().await.context("commit")?;
    Ok(BranchStockView {
        branch_id: branch,
        items: lines,
    })
}

fn parse_id(key: &str) -> Result<Uuid, AppError> {
    key.parse().map_err(|_| AppError::NotFound)
}

pub fn router() -> Router<AppState> {
    Router::new().route("/branches/{id}/stock", put(put_stock))
}

async fn put_stock(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    AppJson(body): AppJson<BranchStockRequest>,
) -> Result<Json<BranchStockView>, AppError> {
    require_seller_or_team(&state, &user).await?;
    Ok(Json(
        set_branch_stock(&state.db, user.id, parse_id(&id)?, body).await?,
    ))
}

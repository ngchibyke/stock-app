//! Alerts and their acknowledgement (plan 6, task 5). Owner only.
//!
//! An alert is open until a row in `alert_acks` carries its key (append-only,
//! with a note). The list covers the alerts that come from the audit log (late
//! provider outcomes, payout amount mismatches, courier wallet warnings), paid
//! orders that need a person, and orders whose handover code has locked out at
//! least [`LOCKOUTS_BEFORE_ALERT`] times. Shipment attention rows are acknowledged with
//! the `alert_key` the reconciliation view gives them. Keys:
//!
//! - `audit:<audit_log.id>`
//! - `order:<order id>:no_delivery_method` or `:quote_reused`
//! - `handover:<order id>:<id of the newest lockout>`: the next lockout makes a new key
//! - `shipment:<shipment id or courier label>:<status>:<attention>`: a change of state
//!   makes a new key

use anyhow::Context;
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{Owner, note, views};
use crate::{
    AppState, audit,
    delivery::{options::QUOTE_REUSED, shipments::WALLET_ALERT},
    error::AppError,
    extract::{AppJson, AppQuery},
};

const NOTE_MAX: usize = 500;
/// A handover code that has locked this many times raises an alert.
pub const LOCKOUTS_BEFORE_ALERT: i64 = 3;
const STATES: [&str; 3] = ["open", "acked", "all"];
const AUDIT_ALERTS: [&str; 6] = [
    "identity.budget_exhausted",
    crate::statement::MISMATCH,
    "payout.provider_outcome_after_close",
    "payout.reversed_after_success",
    "payout.amount_mismatch",
    WALLET_ALERT,
];

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/alerts", get(list))
        .route("/admin/alerts/ack", post(acknowledge))
}

#[derive(Deserialize)]
struct ListQuery {
    state: Option<String>,
    limit: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
struct AlertRow {
    alert_key: String,
    kind: String,
    subject_id: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    at: OffsetDateTime,
    detail: Value,
    acked_by: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339::option")]
    acked_at: Option<OffsetDateTime>,
    note: Option<String>,
}

#[derive(Serialize)]
struct AlertList {
    items: Vec<AlertRow>,
    limit: i64,
}

async fn list(
    State(state): State<AppState>,
    Owner(owner): Owner,
    AppQuery(q): AppQuery<ListQuery>,
) -> Result<Json<AlertList>, AppError> {
    let which = q.state.as_deref().unwrap_or("open");
    if !STATES.contains(&which) {
        return Err(AppError::Validation(format!(
            "state must be one of {}",
            STATES.join(", ")
        )));
    }
    let limit = views::limit(q.limit)?;
    let items: Vec<AlertRow> = sqlx::query_as(
        "WITH a AS ( \
            SELECT 'audit:' || id AS alert_key, action AS kind, subject_id, at, detail \
              FROM audit_log WHERE action = ANY($1) \
            UNION ALL \
            SELECT 'order:' || o.id || ':no_delivery_method', 'order.no_delivery_method', \
                   o.id, o.paid_at, '{}'::jsonb \
              FROM orders o WHERE o.status = 'paid' AND o.delivery_method IS NULL \
            UNION ALL \
            SELECT 'order:' || o.id || ':quote_reused', 'order.quote_reused', \
                   o.id, o.paid_at, '{}'::jsonb \
              FROM orders o WHERE o.status = 'paid' AND EXISTS ( \
                  SELECT 1 FROM audit_log x WHERE x.action = $2 \
                     AND x.subject_type = 'order' AND x.subject_id = o.id) \
            UNION ALL \
            SELECT 'handover:' || subject_id || ':' || max(id), \
                   'order.repeated_handover_lockout', subject_id, max(at), \
                   jsonb_build_object('lockouts', count(*)) \
              FROM audit_log \
              WHERE action = 'order.handover_attempt' AND detail->>'locked' = 'true' \
                AND subject_id IS NOT NULL \
              GROUP BY subject_id HAVING count(*) >= $3) \
         SELECT a.alert_key, a.kind, a.subject_id, a.at, a.detail, \
                k.acked_by, k.acked_at, k.note \
         FROM a LEFT JOIN alert_acks k ON k.alert_key = a.alert_key \
         WHERE $4 = 'all' OR ($4 = 'open' AND k.alert_key IS NULL) \
                          OR ($4 = 'acked' AND k.alert_key IS NOT NULL) \
         ORDER BY a.at DESC, a.alert_key LIMIT $5",
    )
    .bind(AUDIT_ALERTS.to_vec())
    .bind(QUOTE_REUSED)
    .bind(LOCKOUTS_BEFORE_ALERT)
    .bind(which)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .context("list alerts")?;
    let mut conn = state.db.acquire().await.context("acquire")?;
    views::record_view(&mut conn, owner.id, "alerts", items.len()).await?;
    Ok(Json(AlertList { items, limit }))
}

#[derive(Deserialize)]
struct AckBody {
    alert_key: String,
    note: Option<String>,
}

fn bad_key() -> AppError {
    AppError::Validation("alert_key is not a valid alert key".into())
}

/// Checks that `key` names an alert that exists (404 otherwise) or is shaped
/// like one (422 otherwise).
async fn check_key(db: &sqlx::PgPool, key: &str) -> Result<(), AppError> {
    if key.len() > 200 || !key.chars().all(|c| c.is_ascii_graphic()) {
        return Err(bad_key());
    }
    let (kind, rest) = key.split_once(':').ok_or_else(bad_key)?;
    let found = match kind {
        "audit" => {
            let id: i64 = rest.parse().map_err(|_| bad_key())?;
            sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM audit_log WHERE id = $1 AND action = ANY($2))",
            )
            .bind(id)
            .bind(AUDIT_ALERTS.to_vec())
            .fetch_one(db)
            .await
            .context("check audit alert")?
        }
        "order" => {
            let (id, reason) = rest.split_once(':').ok_or_else(bad_key)?;
            let id: Uuid = id.parse().map_err(|_| bad_key())?;
            if !["no_delivery_method", "quote_reused"].contains(&reason) {
                return Err(bad_key());
            }
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM orders WHERE id = $1)")
                .bind(id)
                .fetch_one(db)
                .await
                .context("check order alert")?
        }
        "handover" => {
            let (id, newest) = rest.split_once(':').ok_or_else(bad_key)?;
            let id: Uuid = id.parse().map_err(|_| bad_key())?;
            let newest: i64 = newest.parse().map_err(|_| bad_key())?;
            sqlx::query_scalar(
                "SELECT count(*) >= $3 AND max(id) = $2 FROM audit_log \
                 WHERE action = 'order.handover_attempt' AND detail->>'locked' = 'true' \
                   AND subject_id = $1",
            )
            .bind(id)
            .bind(newest)
            .bind(LOCKOUTS_BEFORE_ALERT)
            .fetch_one(db)
            .await
            .context("check handover alert")?
        }
        // Shipment rows are derived from several tables and carry their state in
        // the key; the shape is all that is checked.
        "shipment" => {
            if rest.split(':').count() != 3 {
                return Err(bad_key());
            }
            true
        }
        _ => return Err(bad_key()),
    };
    if found {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}

async fn acknowledge(
    State(state): State<AppState>,
    Owner(owner): Owner,
    AppJson(body): AppJson<AckBody>,
) -> Result<Json<Value>, AppError> {
    let note = note("note", body.note.as_deref(), NOTE_MAX)?;
    check_key(&state.db, &body.alert_key).await?;
    let mut tx = state.db.begin().await.context("begin")?;
    let acked_at: Option<OffsetDateTime> = sqlx::query_scalar(
        "INSERT INTO alert_acks (alert_key, acked_by, note) VALUES ($1, $2, $3) \
         ON CONFLICT (alert_key) DO NOTHING RETURNING acked_at",
    )
    .bind(&body.alert_key)
    .bind(owner.id)
    .bind(&note)
    .fetch_optional(&mut *tx)
    .await
    .context("insert ack")?;
    let acked_at = acked_at.ok_or(AppError::Conflict("already_acked"))?;
    audit::record(
        &mut tx,
        Some(owner.id),
        "admin.alert_acked",
        "alert",
        None,
        json!({ "alert_key": body.alert_key }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(Json(json!({
        "alert_key": body.alert_key,
        "acked_at": acked_at.format(&time::format_description::well_known::Rfc3339).context("format")?,
        "note": note,
    })))
}

//! Buyer delivery addresses inside the FCT. The recipient name, phone, address line
//! and landmark are encrypted at rest; the area council and postcode are plaintext.
//! Nothing here logs or audits personal data: audit rows carry the address id only.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    area::{AreaCouncil, normalize_postcode},
    postcode::PostcodeCheck,
};
use crate::{
    AppState, audit, auth::AuthUser, crypto::FieldCipher, error::AppError, extract::AppJson,
    phone::normalize_ng_phone,
};

const MAX_NAME: usize = 100;
const MAX_LINE: usize = 300;
const MAX_LANDMARK: usize = 200;
const MAX_LIVE_ADDRESSES: i64 = 20;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/addresses", get(list).post(create))
        .route("/me/addresses/{id}", delete(remove))
}

fn parse_id(key: &str) -> Result<Uuid, AppError> {
    key.parse().map_err(|_| AppError::NotFound)
}

/// A decrypted delivery address. Task 6 stores it as an encrypted JSON snapshot on
/// the order. `Debug` redacts every personal field.
#[derive(Clone, Serialize, Deserialize)]
pub struct DeliveryAddress {
    pub id: Uuid,
    pub area_council: AreaCouncil,
    pub postcode: Option<String>,
    pub courier_address_code: Option<i64>,
    pub recipient_name: String,
    pub phone: String,
    pub address_line: String,
    pub landmark: Option<String>,
}

impl std::fmt::Debug for DeliveryAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeliveryAddress")
            .field("id", &"[redacted]")
            .field("area_council", &"[redacted]")
            .field("postcode", &"[redacted]")
            .field("courier_address_code", &"[redacted]")
            .field("recipient_name", &"[redacted]")
            .field("phone", &"[redacted]")
            .field("address_line", &"[redacted]")
            .field("landmark", &"[redacted]")
            .finish()
    }
}

#[derive(Deserialize)]
pub struct NewAddress {
    recipient_name: String,
    phone: String,
    address_line: String,
    landmark: Option<String>,
    area_council: String,
    postcode: Option<String>,
}

impl std::fmt::Debug for NewAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NewAddress(<redacted>)")
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    recipient_name_enc: Vec<u8>,
    phone_enc: Vec<u8>,
    address_line_enc: Vec<u8>,
    landmark_enc: Option<Vec<u8>>,
    area_council: AreaCouncil,
    postcode: Option<String>,
    courier_address_code: Option<i64>,
}

const COLUMNS: &str = "id, recipient_name_enc, phone_enc, address_line_enc, landmark_enc, \
                       area_council, postcode, courier_address_code";

impl Row {
    fn open(self, cipher: &FieldCipher) -> Result<DeliveryAddress, AppError> {
        Ok(DeliveryAddress {
            id: self.id,
            area_council: self.area_council,
            postcode: self.postcode,
            courier_address_code: self.courier_address_code,
            recipient_name: cipher.decrypt(&self.recipient_name_enc)?,
            phone: cipher.decrypt(&self.phone_enc)?,
            address_line: cipher.decrypt(&self.address_line_enc)?,
            landmark: self
                .landmark_enc
                .as_deref()
                .map(|b| cipher.decrypt(b))
                .transpose()?,
        })
    }
}

fn required(value: &str, max: usize, field: &str) -> Result<String, AppError> {
    let v = value.trim();
    let n = v.chars().count();
    if n == 0 || n > max {
        return Err(AppError::Validation(format!(
            "{field} must be 1 to {max} characters"
        )));
    }
    Ok(v.to_string())
}

async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(body): AppJson<NewAddress>,
) -> Result<(StatusCode, Json<DeliveryAddress>), AppError> {
    let recipient_name = required(&body.recipient_name, MAX_NAME, "recipient_name")?;
    let address_line = required(&body.address_line, MAX_LINE, "address_line")?;
    let phone = normalize_ng_phone(&body.phone)?;
    let landmark = match body.landmark.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(l) => Some(required(l, MAX_LANDMARK, "landmark")?),
    };
    let area_council: AreaCouncil = body.area_council.parse()?;
    // A blank postcode means "not given"; the normaliser rejects blank.
    let postcode = match body.postcode.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(p) => Some(normalize_postcode(p)?),
    };
    if let Some(code) = &postcode {
        // Only a definite "no such code" blocks; an unreachable lookup never does.
        if let Ok(PostcodeCheck::Invalid) = state.postcodes.check(code).await {
            return Err(AppError::Unprocessable(
                "invalid_postcode",
                "That postcode does not exist".into(),
            ));
        }
    }

    let mut tx = state.db.begin().await.context("begin")?;
    // Serialise per user so the cap cannot be raced past (users lock comes first).
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(user.id)
        .fetch_optional(&mut *tx)
        .await
        .context("lock user")?;
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM buyer_addresses WHERE user_id = $1 AND deleted_at IS NULL",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await
    .context("count addresses")?;
    if live >= MAX_LIVE_ADDRESSES {
        return Err(AppError::Unprocessable(
            "too_many_addresses",
            format!("You can save at most {MAX_LIVE_ADDRESSES} addresses"),
        ));
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO buyer_addresses \
         (user_id, recipient_name_enc, phone_enc, address_line_enc, landmark_enc, \
          area_council, postcode) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(user.id)
    .bind(state.cipher.encrypt(&recipient_name))
    .bind(state.cipher.encrypt(&phone))
    .bind(state.cipher.encrypt(&address_line))
    .bind(landmark.as_deref().map(|l| state.cipher.encrypt(l)))
    .bind(area_council)
    .bind(postcode.as_deref())
    .fetch_one(&mut *tx)
    .await
    .context("insert address")?;
    audit::record(
        &mut tx,
        Some(user.id),
        "address.created",
        "buyer_address",
        Some(id),
        json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;

    Ok((
        StatusCode::CREATED,
        Json(DeliveryAddress {
            id,
            area_council,
            postcode,
            courier_address_code: None,
            recipient_name,
            phone,
            address_line,
            landmark,
        }),
    ))
}

async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<DeliveryAddress>>, AppError> {
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM buyer_addresses \
         WHERE user_id = $1 AND deleted_at IS NULL ORDER BY created_at DESC, id DESC"
    ))
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    .context("list addresses")?;
    let out = rows
        .into_iter()
        .map(|r| r.open(&state.cipher))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(out))
}

async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let id = parse_id(&id)?;
    let mut tx = state.db.begin().await.context("begin")?;
    let deleted: Option<Uuid> = sqlx::query_scalar(
        "UPDATE buyer_addresses SET deleted_at = now() \
         WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL RETURNING id",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await
    .context("delete address")?;
    if deleted.is_none() {
        return Err(AppError::NotFound);
    }
    audit::record(
        &mut tx,
        Some(user.id),
        "address.deleted",
        "buyer_address",
        Some(id),
        json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(StatusCode::NO_CONTENT)
}

/// Loads one of the user's live addresses, decrypted, for checkout. Another user's
/// address and a deleted one are both `NotFound`.
pub async fn load_for_checkout(
    db: &PgPool,
    cipher: &FieldCipher,
    user: Uuid,
    id: Uuid,
) -> Result<DeliveryAddress, AppError> {
    let row: Option<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM buyer_addresses \
         WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL"
    ))
    .bind(id)
    .bind(user)
    .fetch_optional(db)
    .await
    .context("load address")?;
    row.ok_or(AppError::NotFound)?.open(cipher)
}

use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, header, request::Parts},
};
use serde::Serialize;
use uuid::Uuid;

use super::sessions::hash_token;
use crate::{AppState, error::AppError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, sqlx::Type)]
#[sqlx(type_name = "user_role", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Support,
    Owner,
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: Uuid,
    /// The session this request's bearer token belongs to.
    pub session_id: Uuid,
    pub role: Role,
    pub is_seller: bool,
    pub phone_verified: bool,
}

impl AuthUser {
    /// Owner satisfies every role check.
    pub fn require_role(&self, role: Role) -> Result<(), AppError> {
        if self.role == Role::Owner || self.role == role {
            Ok(())
        } else {
            Err(AppError::Forbidden("forbidden"))
        }
    }
}

pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?.trim();
    (!token.is_empty()).then_some(token)
}

/// The routes a person whose deletion is pending may still use.
fn allowed_while_deleting(path: &str) -> bool {
    path == "/me/deletion"
        || path == "/me/export"
        || path.starts_with("/me/export/")
        || path == "/auth/sign-out"
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let token = bearer_token(&parts.headers).ok_or(AppError::Unauthorized)?;
        let row: Option<(Uuid, Uuid, Role, bool, bool, bool)> = sqlx::query_as(
            "SELECT u.id, s.id, u.role, u.is_seller, u.phone_verified_at IS NOT NULL, \
                    EXISTS (SELECT 1 FROM deletion_requests d \
                            WHERE d.user_id = u.id AND d.status = 'pending') \
             FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > now() \
               AND u.deleted_at IS NULL",
        )
        .bind(hash_token(token))
        .fetch_optional(&state.db)
        .await
        .map_err(anyhow::Error::from)?;
        let (id, session_id, role, is_seller, phone_verified, deletion_pending) =
            row.ok_or(AppError::Unauthorized)?;
        // While a deletion is waiting out its cooling-off, the account can only look at
        // or cancel the request, take its data, and sign out.
        if deletion_pending && !allowed_while_deleting(parts.uri.path()) {
            return Err(AppError::Forbidden("deletion_pending"));
        }
        Ok(AuthUser {
            id,
            session_id,
            role,
            is_seller,
            phone_verified,
        })
    }
}

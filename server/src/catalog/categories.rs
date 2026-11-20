//! The fixed list of product categories.

use anyhow::Context;
use axum::{Json, Router, extract::State, routing::get};
use serde::Serialize;
use sqlx::PgExecutor;

use crate::{AppState, error::AppError};

#[derive(Serialize, sqlx::FromRow)]
pub struct Category {
    pub slug: String,
    pub name: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/categories", get(list))
}

/// Whether `slug` is a category. Exact match: slugs are lower-case.
pub async fn exists<'e>(db: impl PgExecutor<'e>, slug: &str) -> Result<bool, AppError> {
    let found = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE slug = $1)")
        .bind(slug)
        .fetch_one(db)
        .await
        .context("check category")?;
    Ok(found)
}

/// Public, no sign-in: the app shows categories before anyone has an account.
async fn list(State(state): State<AppState>) -> Result<Json<Vec<Category>>, AppError> {
    let rows = sqlx::query_as("SELECT slug, name FROM categories ORDER BY position, slug")
        .fetch_all(&state.db)
        .await
        .context("list categories")?;
    Ok(Json(rows))
}

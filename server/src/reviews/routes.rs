//! Review routes.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::Deserialize;
use uuid::Uuid;

use super::service::{self, NewReview, PostOutcome, ReviewPage};
use crate::{
    AppState,
    admin::parse_id,
    auth::AuthUser,
    error::AppError,
    extract::{AppJson, AppQuery},
    stores::{self, routes::MaybeAuth},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/orders/{id}/review", post(review))
        .route("/reviews/{id}/reply", post(reply))
        .route("/stores/{key}/reviews", get(list))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewBody {
    stars: i16,
    body: Option<String>,
    #[serde(default)]
    photo_keys: Vec<String>,
    use_suggestion: Option<Uuid>,
}

async fn review(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    AppJson(body): AppJson<ReviewBody>,
) -> Result<Json<PostOutcome>, AppError> {
    let new = NewReview {
        stars: body.stars,
        body: body.body,
        photo_keys: body.photo_keys,
        use_suggestion: body.use_suggestion,
    };
    Ok(Json(
        service::post_review(&state, user.id, parse_id(&id)?, new).await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyBody {
    body: Option<String>,
    use_suggestion: Option<Uuid>,
}

async fn reply(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    AppJson(body): AppJson<ReplyBody>,
) -> Result<Json<PostOutcome>, AppError> {
    Ok(Json(
        service::post_reply(
            &state,
            user.id,
            parse_id(&id)?,
            body.body.as_deref(),
            body.use_suggestion,
        )
        .await?,
    ))
}

#[derive(Deserialize)]
struct ListQuery {
    before: Option<String>,
    limit: Option<i64>,
}

/// Public. Follows the store's own visibility: a store nobody can see has no
/// reviews to show.
async fn list(
    State(state): State<AppState>,
    MaybeAuth(viewer): MaybeAuth,
    Path(key): Path<String>,
    AppQuery(q): AppQuery<ListQuery>,
) -> Result<Json<ReviewPage>, AppError> {
    let store = stores::service::get_by_slug(&state.db, &key, viewer.map(|u| u.id)).await?;
    Ok(Json(
        service::list_for_store(&state.db, store.id, q.before.as_deref(), q.limit).await?,
    ))
}

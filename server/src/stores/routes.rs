use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, State},
    http::{StatusCode, header, request::Parts},
    response::IntoResponse,
    routing::get,
};
use uuid::Uuid;

use anyhow::Context;
use serde::Serialize;

use super::{
    access::require_seller_or_member,
    service::{self, NewStore, OwnerStore, PublicStore, StorePatch},
};
use crate::{
    AppState,
    auth::{AuthUser, extractor::bearer_token},
    delivery::area::AreaCouncil,
    error::AppError,
    extract::AppJson,
};

// Path parameters share one name: `/stores/{key}` is a slug for GET and an id for PATCH/DELETE.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/stores", axum::routing::post(create))
        .route("/stores/mine", get(mine))
        .route("/stores/{key}", get(show).patch(update).delete(remove))
        .merge(super::team::router())
        .merge(super::branches::router())
}

/// Optional authentication: no credentials means anonymous; bad credentials are still rejected.
pub(crate) struct MaybeAuth(pub Option<AuthUser>);

impl FromRequestParts<AppState> for MaybeAuth {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        if bearer_token(&parts.headers).is_none() {
            return Ok(MaybeAuth(None));
        }
        Ok(MaybeAuth(Some(
            AuthUser::from_request_parts(parts, state).await?,
        )))
    }
}

pub(crate) fn require_seller(user: &AuthUser) -> Result<(), AppError> {
    if user.is_seller {
        Ok(())
    } else {
        Err(AppError::Forbidden("not_a_seller"))
    }
}

/// [`require_seller_or_member`] on a pooled connection, for the seller routes.
pub(crate) async fn require_seller_or_team(
    state: &AppState,
    user: &AuthUser,
) -> Result<(), AppError> {
    if user.is_seller {
        return Ok(());
    }
    let mut conn = state.db.acquire().await.context("acquire")?;
    require_seller_or_member(&mut conn, user).await
}

/// One of `GET /stores/mine`: the store as its team sees it, and the caller's place.
#[derive(Debug, Serialize)]
struct MineView {
    #[serde(flatten)]
    store: MineStore,
    my_role: String,
    my_branch_id: Option<Uuid>,
}

/// The owner and managers see the pickup address and delivery details; staff see
/// what they see of a branch: no address, postcode or phone.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum MineStore {
    Full(OwnerStore),
    Staff {
        #[serde(flatten)]
        store: PublicStore,
        district: String,
        plot: Option<String>,
        pickup_area_council: Option<AreaCouncil>,
    },
}

fn parse_id(key: &str) -> Result<Uuid, AppError> {
    key.parse().map_err(|_| AppError::NotFound)
}

async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(body): AppJson<NewStore>,
) -> Result<impl IntoResponse, AppError> {
    require_seller(&user)?;
    let store =
        service::create_store(&state.db, &state.cipher, &state.market, user.id, body).await?;
    let plot = service::plots_of(&state.db, &[store.id])
        .await?
        .remove(&store.id);
    Ok((
        StatusCode::CREATED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(store.owner_view(&state.cipher, &*state.objects, plot)?),
    ))
}

async fn mine(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    require_seller_or_team(&state, &user).await?;
    let stores = service::list_mine(&state.db, user.id).await?;
    let ids: Vec<Uuid> = stores.iter().map(|s| s.store.id).collect();
    let mut plots = service::plots_of(&state.db, &ids).await?;
    let views = stores
        .into_iter()
        .map(|m| {
            let plot = plots.remove(&m.store.id);
            let store = if m.my_role == "staff" {
                MineStore::Staff {
                    store: m.store.public(&*state.objects),
                    district: m.store.district.clone(),
                    plot,
                    pickup_area_council: m.store.pickup_area_council,
                }
            } else {
                MineStore::Full(m.store.owner_view(&state.cipher, &*state.objects, plot)?)
            };
            Ok(MineView {
                store,
                my_role: m.my_role,
                my_branch_id: m.my_branch_id,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(views)))
}

async fn show(
    State(state): State<AppState>,
    MaybeAuth(viewer): MaybeAuth,
    Path(slug): Path<String>,
) -> Result<Json<PublicStore>, AppError> {
    let store = service::get_by_slug(&state.db, &slug, viewer.map(|u| u.id)).await?;
    Ok(Json(store.public(&*state.objects)))
}

async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(key): Path<String>,
    AppJson(patch): AppJson<StorePatch>,
) -> Result<impl IntoResponse, AppError> {
    require_seller_or_team(&state, &user).await?;
    let store = service::update_store(
        &state.db,
        &state.cipher,
        &state.market,
        user.id,
        parse_id(&key)?,
        patch,
    )
    .await?;
    let plot = service::plots_of(&state.db, &[store.id])
        .await?
        .remove(&store.id);
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(store.owner_view(&state.cipher, &*state.objects, plot)?),
    ))
}

async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path(key): Path<String>,
) -> Result<StatusCode, AppError> {
    require_seller_or_team(&state, &user).await?;
    service::delete_store(&state.db, &state.market, user.id, parse_id(&key)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

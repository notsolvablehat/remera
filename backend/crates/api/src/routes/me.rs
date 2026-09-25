use axum::{Json, response::IntoResponse};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{extractors::auth_user::AuthUser, state::AppState};

#[derive(Serialize, ToSchema)]
struct MeResponse {
    id: String,
    email: Option<String>,
    name: Option<String>,
}

#[utoipa::path(
    get,
    path="/me",
    tag="Auth",
    responses((status=200, description="Current user", body=MeResponse))
)]
async fn get_me(user: AuthUser) -> impl IntoResponse {
    Json(MeResponse {
        id: user.id,
        email: user.email,
        name: user.name,
    })
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_me))
}

use axum::{Json, response::IntoResponse};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{extractors::auth_user::AuthUser, state::AppState};

#[derive(Serialize, ToSchema)]
struct MeResponse {
    id: String,
    name: Option<String>,
    email: Option<String>,
    email_verified: bool,
    image: Option<String>,
    username: Option<String>,
    banned: bool,
    ban_reason: Option<String>,
    ban_expires: Option<String>,
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
        name: user.name,
        email: user.email,
        email_verified: user.email_verified,
        image: user.image,
        username: user.username,
        banned: user.banned,
        ban_reason: user.ban_reason,
        ban_expires: user.ban_expires,
    })
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_me))
}

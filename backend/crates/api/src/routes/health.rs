use axum::Json;
use serde::Serialize;
use serde_json::{Value, json};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::state::AppState;

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
}

#[utoipa::path(
    get,
    path = "/healthz",
    tag="Meta",
    responses(
        (status = 200, description = "Service is alive", body = HealthResponse)
    )
)]
async fn healthz() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

#[utoipa::path(
    get,
    path = "/",
    tag="Meta",
    responses(
        (status = 200, description = "Application is running")
    )
)]
async fn get_slash() -> Json<Value> {
    Json(json!({
        "application": "running",
        "name": "remera"
    }))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(healthz))
        .routes(routes!(get_slash))
}

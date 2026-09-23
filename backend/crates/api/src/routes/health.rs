use axum::{Json, Router, routing::get};
use serde_json::{Value, json};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/", get(get_slash))
}

async fn get_slash() -> Json<Value> {
    Json(json!({
        "application": "running",
        "name": "remera"
    }))
}
async fn healthz() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

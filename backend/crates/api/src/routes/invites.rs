use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use storage::allowlist_repo;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;
use validator::Validate;

use crate::{
    extractors::container_access::{ContainerAccess, Owner},
    state::AppState,
};

#[derive(Deserialize, Validate, ToSchema)]
pub struct AddAllowlistEntryRequest {
    #[validate(email)]
    pub email: String,
}

#[derive(Serialize, ToSchema)]
pub struct AllowlistEntryDto {
    pub email: String,
    pub claimed: bool,
}

#[derive(Serialize, ToSchema)]
pub struct AllowlistResponse {
    pub entries: Vec<AllowlistEntryDto>,
}

fn db_error() -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "db_error"})),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/containers/{container_id}/edit-allowlist",
    tag = "Invites",
    params(("container_id" = Uuid, Path, description = "Container id")),
    request_body = AddAllowlistEntryRequest,
    responses(
        (status = 201, description = "Email added to the edit allow-list"),
        (status = 400, description = "Invalid email"),
        (status = 403, description = "Not the container owner"),
    )
)]
async fn add_allowlist_entry(
    _access: ContainerAccess<Owner>,
    State(state): State<AppState>,
    Path(container_id): Path<Uuid>,
    Json(body): Json<AddAllowlistEntryRequest>,
) -> axum::response::Response {
    if body.validate().is_err() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid_email"})),
        )
            .into_response();
    }

    match allowlist_repo::insert(&state.db, container_id, &body.email).await {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(_) => db_error(),
    }
}

#[utoipa::path(
    get,
    path = "/containers/{container_id}/edit-allowlist",
    tag = "Invites",
    params(("container_id" = Uuid, Path, description = "Container id")),
    responses(
        (status = 200, description = "Allow-listed emails and claim status", body = AllowlistResponse),
        (status = 403, description = "Not the container owner"),
    )
)]
async fn list_allowlist(
    _access: ContainerAccess<Owner>,
    State(state): State<AppState>,
    Path(container_id): Path<Uuid>,
) -> axum::response::Response {
    match allowlist_repo::list_for_container(&state.db, container_id).await {
        Ok(entries) => Json(AllowlistResponse {
            entries: entries
                .into_iter()
                .map(|e| AllowlistEntryDto {
                    email: e.email,
                    claimed: e.claimed_at.is_some(),
                })
                .collect(),
        })
        .into_response(),
        Err(_) => db_error(),
    }
}

#[utoipa::path(
    delete,
    path = "/containers/{container_id}/edit-allowlist/{email}",
    tag = "Invites",
    params(
        ("container_id" = Uuid, Path, description = "Container id"),
        ("email" = String, Path, description = "Allow-listed email to remove"),
    ),
    responses(
        (status = 204, description = "Entry removed (and Editor access revoked, if claimed)"),
        (status = 403, description = "Not the container owner"),
    )
)]
async fn delete_allowlist_entry(
    _access: ContainerAccess<Owner>,
    State(state): State<AppState>,
    Path((container_id, email)): Path<(Uuid, String)>,
) -> axum::response::Response {
    // Revoke Editor membership granted via this allow-list entry before
    // deleting the entry itself — otherwise the FK cascade on
    // `container_edit_allowlist` (container_id) has nothing to key off of,
    // since there's no reverse FK from container_member back here.
    let member = match sqlx::query!(
        "SELECT cm.user_id FROM container_member cm
         JOIN users u ON u.id = cm.user_id
         WHERE cm.container_id = $1 AND u.email = $2 AND cm.role = 'editor'",
        container_id,
        email
    )
    .fetch_optional(&state.db)
    .await
    {
        Ok(member) => member,
        Err(_) => return db_error(),
    };

    if let Some(row) = member {
        let deleted = sqlx::query!(
            "DELETE FROM container_member WHERE container_id = $1 AND user_id = $2",
            container_id,
            row.user_id
        )
        .execute(&state.db)
        .await;

        if deleted.is_err() {
            return db_error();
        }

        state.cache.invalidate(&(container_id, row.user_id)).await;
    }

    match allowlist_repo::delete(&state.db, container_id, &email).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => db_error(),
    }
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(add_allowlist_entry))
        .routes(routes!(list_allowlist))
        .routes(routes!(delete_allowlist_entry))
}

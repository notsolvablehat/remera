use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use storage::{allowlist_repo, containers_repo};
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

#[derive(Serialize, ToSchema)]
pub struct SharePreviewResponse {
    pub container_id: Uuid,
    pub name: String,
    pub media_count: i64,
}

/// Public, no auth, no membership required — this is the landing-page
/// preview a share link resolves to, not the media grid itself (that's
/// `GET /containers/{cid}/media?share_token=...`, gated by
/// `ContainerViewAccess`). Rate-limited the same as every other route,
/// via the global governor layer in router.rs.
#[utoipa::path(
    get,
    path = "/invites/{token}",
    tag = "Invites",
    params(("token" = String, Path, description = "Share-link token")),
    responses(
        (status = 200, description = "Container preview", body = SharePreviewResponse),
        (status = 404, description = "Invalid, expired, or rotated-away token"),
        (status = 423, description = "Container is locked"),
    )
)]
async fn resolve_share_token(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> axum::response::Response {
    let Ok((container_id, share_link_id)) = state.share_link_codec.decode(&token) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "invalid_token"})),
        )
            .into_response();
    };

    match containers_repo::get_share_preview(&state.db, container_id, share_link_id).await {
        Ok(Some(preview)) if preview.is_locked => (
            StatusCode::LOCKED,
            Json(serde_json::json!({"error": "container_locked"})),
        )
            .into_response(),
        Ok(Some(preview)) => Json(SharePreviewResponse {
            container_id,
            name: preview.name,
            media_count: preview.media_count,
        })
        .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "invalid_token"})),
        )
            .into_response(),
        Err(_) => db_error(),
    }
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(add_allowlist_entry))
        .routes(routes!(list_allowlist))
        .routes(routes!(delete_allowlist_entry))
        .routes(routes!(resolve_share_token))
}

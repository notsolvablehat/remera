use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use domain::Role;
use serde::{Deserialize, Serialize};
use storage::containers_repo::{self, ContainersRepoError};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;
use validator::Validate;

use crate::{
    extractors::{
        auth_user::AuthUser,
        container_access::{ContainerAccess, ContainerStatus, Owner, Viewer},
    },
    state::AppState,
};

#[derive(Deserialize, Validate, ToSchema)]
pub struct CreateContainerRequest {
    #[validate(length(min = 1, max = 200))]
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct ContainerDto {
    pub id: Uuid,
    pub name: String,
    pub is_public: bool,
    pub is_locked: bool,
    pub role: String,
}

#[derive(Serialize, ToSchema)]
pub struct ContainerListResponse {
    pub containers: Vec<ContainerDto>,
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
    path = "/containers",
    tag = "Containers",
    request_body = CreateContainerRequest,
    responses(
        (status = 201, description = "Container created", body = ContainerDto),
        (status = 400, description = "Invalid name"),
        (status = 409, description = "Owned-container quota exceeded"),
    )
)]
async fn create_container(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateContainerRequest>,
) -> axum::response::Response {
    if body.validate().is_err() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid_name"})),
        )
            .into_response();
    }

    match containers_repo::create_with_owner(&state.db, &user.id, &body.name).await {
        Ok(container) => {
            state
                .cache
                .insert((container.id, user.id.clone()), Role::Owner)
                .await;
            state
                .container_status_cache
                .insert(
                    container.id,
                    ContainerStatus {
                        is_locked: false,
                        is_deleted: false,
                        share_link_id: None,
                    },
                )
                .await;

            (
                StatusCode::CREATED,
                Json(ContainerDto {
                    id: container.id,
                    name: container.name,
                    is_public: container.is_public,
                    is_locked: container.is_locked,
                    role: Role::Owner.as_str().to_string(),
                }),
            )
                .into_response()
        }
        Err(ContainersRepoError::Domain(domain::DomainError::QuotaExceeded)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "owned_container_quota_exceeded"})),
        )
            .into_response(),
        Err(_) => db_error(),
    }
}

#[utoipa::path(
    get,
    path = "/containers",
    tag = "Containers",
    responses((status = 200, description = "Containers the caller belongs to", body = ContainerListResponse))
)]
async fn list_containers(
    user: AuthUser,
    State(state): State<AppState>,
) -> axum::response::Response {
    match containers_repo::list_for_user(&state.db, &user.id).await {
        Ok(rows) => Json(ContainerListResponse {
            containers: rows
                .into_iter()
                .map(|r| ContainerDto {
                    id: r.container.id,
                    name: r.container.name,
                    is_public: r.container.is_public,
                    is_locked: r.container.is_locked,
                    role: r.role.as_str().to_string(),
                })
                .collect(),
        })
        .into_response(),
        Err(_) => db_error(),
    }
}

#[utoipa::path(
    get,
    path = "/containers/{container_id}",
    tag = "Containers",
    params(("container_id" = Uuid, Path, description = "Container id")),
    responses(
        (status = 200, description = "Container metadata", body = ContainerDto),
        (status = 403, description = "Not a member"),
        (status = 404, description = "Container not found"),
        (status = 423, description = "Container is locked (non-owner)"),
    )
)]
async fn get_container(
    access: ContainerAccess<Viewer>,
    State(state): State<AppState>,
    Path(container_id): Path<Uuid>,
) -> axum::response::Response {
    match containers_repo::get_by_id(&state.db, container_id).await {
        Ok(Some(container)) => Json(ContainerDto {
            id: container.id,
            name: container.name,
            is_public: container.is_public,
            is_locked: container.is_locked,
            role: access.role.as_str().to_string(),
        })
        .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not_found"})),
        )
            .into_response(),
        Err(_) => db_error(),
    }
}

#[derive(Deserialize, Validate, ToSchema)]
pub struct UpdateContainerRequest {
    #[validate(length(min = 1, max = 200))]
    pub name: Option<String>,
    #[validate(range(min = 1))]
    pub storage_limit_bytes: Option<i64>,
    #[validate(range(min = 1))]
    pub media_limit: Option<i32>,
}

#[utoipa::path(
    patch,
    path = "/containers/{container_id}",
    tag = "Containers",
    params(("container_id" = Uuid, Path, description = "Container id")),
    request_body = UpdateContainerRequest,
    responses(
        (status = 200, description = "Updated", body = ContainerDto),
        (status = 400, description = "Invalid request, or no fields to update"),
        (status = 403, description = "Not the owner"),
        (status = 404, description = "Container not found"),
    )
)]
async fn update_container(
    access: ContainerAccess<Owner>,
    State(state): State<AppState>,
    Path(container_id): Path<Uuid>,
    Json(body): Json<UpdateContainerRequest>,
) -> axum::response::Response {
    let no_fields =
        body.name.is_none() && body.storage_limit_bytes.is_none() && body.media_limit.is_none();
    if body.validate().is_err() || no_fields {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid_request"})),
        )
            .into_response();
    }

    match containers_repo::update_metadata(
        &state.db,
        container_id,
        body.name.as_deref(),
        body.storage_limit_bytes,
        body.media_limit,
    )
    .await
    {
        Ok(Some(container)) => Json(ContainerDto {
            id: container.id,
            name: container.name,
            is_public: container.is_public,
            is_locked: container.is_locked,
            role: access.role.as_str().to_string(),
        })
        .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not_found"})),
        )
            .into_response(),
        Err(_) => db_error(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct SetLockRequest {
    pub locked: bool,
}

#[utoipa::path(
    put,
    path = "/containers/{container_id}/lock",
    tag = "Containers",
    params(("container_id" = Uuid, Path, description = "Container id")),
    request_body = SetLockRequest,
    responses(
        (status = 200, description = "Lock state updated"),
        (status = 403, description = "Not the owner"),
        (status = 404, description = "Container not found"),
    )
)]
async fn set_container_lock(
    _access: ContainerAccess<Owner>,
    State(state): State<AppState>,
    Path(container_id): Path<Uuid>,
    Json(body): Json<SetLockRequest>,
) -> axum::response::Response {
    match containers_repo::set_locked(&state.db, container_id, body.locked).await {
        Ok(true) => {
            // Invalidate immediately — otherwise anyone with a live cache
            // entry could keep acting on the stale lock state for up to
            // the cache's TTL (see extractors/container_access.rs).
            state.container_status_cache.invalidate(&container_id).await;
            StatusCode::OK.into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not_found"})),
        )
            .into_response(),
        Err(_) => db_error(),
    }
}

#[utoipa::path(
    delete,
    path = "/containers/{container_id}",
    tag = "Containers",
    params(("container_id" = Uuid, Path, description = "Container id")),
    responses(
        (status = 204, description = "Soft-deleted"),
        (status = 403, description = "Not the owner"),
        (status = 404, description = "Container not found"),
    )
)]
async fn delete_container(
    _access: ContainerAccess<Owner>,
    State(state): State<AppState>,
    Path(container_id): Path<Uuid>,
) -> axum::response::Response {
    match containers_repo::soft_delete(&state.db, container_id).await {
        Ok(true) => {
            state.container_status_cache.invalidate(&container_id).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not_found"})),
        )
            .into_response(),
        Err(_) => db_error(),
    }
}

#[derive(Serialize, ToSchema)]
pub struct ContainerUsageResponse {
    pub storage_bytes: i64,
    pub storage_limit: i64,
    pub media_count: i32,
    pub media_limit: i32,
    pub member_count: i64,
}

#[utoipa::path(
    get,
    path = "/containers/{container_id}/usage",
    tag = "Containers",
    params(("container_id" = Uuid, Path, description = "Container id")),
    responses(
        (status = 200, description = "Storage/media/member usage", body = ContainerUsageResponse),
        (status = 403, description = "Not a member"),
        (status = 404, description = "Container not found"),
        (status = 423, description = "Container is locked (non-owner)"),
    )
)]
async fn get_container_usage(
    _access: ContainerAccess<Viewer>,
    State(state): State<AppState>,
    Path(container_id): Path<Uuid>,
) -> axum::response::Response {
    match containers_repo::get_usage(&state.db, container_id).await {
        Ok(Some(usage)) => Json(ContainerUsageResponse {
            storage_bytes: usage.storage_bytes,
            storage_limit: usage.storage_limit,
            media_count: usage.media_count,
            media_limit: usage.media_limit,
            member_count: usage.member_count,
        })
        .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not_found"})),
        )
            .into_response(),
        Err(_) => db_error(),
    }
}

#[derive(Serialize, ToSchema)]
pub struct ShareLinkResponse {
    pub token: String,
}

#[utoipa::path(
    get,
    path = "/containers/{container_id}/share-link",
    tag = "Containers",
    params(("container_id" = Uuid, Path, description = "Container id")),
    responses(
        (status = 200, description = "The container's public View share-link token (created if one doesn't exist yet)", body = ShareLinkResponse),
        (status = 403, description = "Not the owner"),
        (status = 404, description = "Container not found"),
    )
)]
async fn get_share_link(
    _access: ContainerAccess<Owner>,
    State(state): State<AppState>,
    Path(container_id): Path<Uuid>,
) -> axum::response::Response {
    match containers_repo::get_or_create_share_link_id(&state.db, container_id).await {
        Ok(Some(share_link_id)) => {
            // The cached ContainerStatus (read by ContainerViewAccess) may
            // still hold `share_link_id: None` from before this call — a
            // freshly created id has to be visible immediately, not after
            // up to 300s of cache TTL.
            state.container_status_cache.invalidate(&container_id).await;
            let token = state.share_link_codec.encode(container_id, share_link_id);
            Json(ShareLinkResponse { token }).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not_found"})),
        )
            .into_response(),
        Err(_) => db_error(),
    }
}

#[utoipa::path(
    post,
    path = "/containers/{container_id}/share-link/rotate",
    tag = "Containers",
    params(("container_id" = Uuid, Path, description = "Container id")),
    responses(
        (status = 200, description = "A new token — every previously issued share link stops working", body = ShareLinkResponse),
        (status = 403, description = "Not the owner"),
        (status = 404, description = "Container not found"),
    )
)]
async fn rotate_share_link(
    _access: ContainerAccess<Owner>,
    State(state): State<AppState>,
    Path(container_id): Path<Uuid>,
) -> axum::response::Response {
    match containers_repo::rotate_share_link_id(&state.db, container_id).await {
        Ok(Some(share_link_id)) => {
            // Old tokens embed the previous share_link_id, which no
            // longer matches — but only once the cached ContainerStatus
            // (read by ContainerViewAccess) is refreshed with the new one.
            state.container_status_cache.invalidate(&container_id).await;
            let token = state.share_link_codec.encode(container_id, share_link_id);
            Json(ShareLinkResponse { token }).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not_found"})),
        )
            .into_response(),
        Err(_) => db_error(),
    }
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(create_container))
        .routes(routes!(list_containers))
        .routes(routes!(get_container))
        .routes(routes!(update_container))
        .routes(routes!(delete_container))
        .routes(routes!(set_container_lock))
        .routes(routes!(get_container_usage))
        .routes(routes!(get_share_link))
        .routes(routes!(rotate_share_link))
}

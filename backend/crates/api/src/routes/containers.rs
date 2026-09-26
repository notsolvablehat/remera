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
        container_access::{ContainerAccess, Viewer},
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

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(create_container))
        .routes(routes!(list_containers))
        .routes(routes!(get_container))
}

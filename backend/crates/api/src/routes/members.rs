use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use domain::{DomainError, Role};
use serde::{Deserialize, Serialize};
use storage::{
    containers_repo,
    members_repo::{self, MembersRepoError},
};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    extractors::container_access::{ContainerAccess, Owner, Viewer},
    state::AppState,
};

#[derive(Serialize, ToSchema)]
pub struct MemberDto {
    pub user_id: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub role: String,
    pub joined_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct MemberListResponse {
    pub members: Vec<MemberDto>,
}

fn db_error() -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "db_error"})),
    )
        .into_response()
}

fn error_response(status: StatusCode, error: &str) -> axum::response::Response {
    (status, Json(serde_json::json!({"error": error}))).into_response()
}

#[utoipa::path(
    get,
    path = "/containers/{container_id}/members",
    tag = "Members",
    params(("container_id" = Uuid, Path, description = "Container id")),
    responses(
        (status = 200, description = "Members of the container", body = MemberListResponse),
        (status = 403, description = "Not a member"),
        (status = 404, description = "Container not found"),
        (status = 423, description = "Container is locked (non-owner)"),
    )
)]
async fn list_members(
    _access: ContainerAccess<Viewer>,
    State(state): State<AppState>,
    Path(container_id): Path<Uuid>,
) -> axum::response::Response {
    match members_repo::list_for_container(&state.db, container_id).await {
        Ok(members) => Json(MemberListResponse {
            members: members
                .into_iter()
                .map(|m| MemberDto {
                    user_id: m.user_id,
                    name: m.name,
                    email: m.email,
                    role: m.role.as_str().to_string(),
                    joined_at: m.joined_at.to_rfc3339(),
                })
                .collect(),
        })
        .into_response(),
        Err(_) => db_error(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateMemberRoleRequest {
    /// "editor" or "viewer" — use POST .../transfer-ownership to change
    /// who the owner is, this endpoint rejects "owner".
    pub role: String,
}

#[derive(Serialize, ToSchema)]
pub struct MemberRoleResponse {
    pub user_id: String,
    pub role: String,
}

#[utoipa::path(
    patch,
    path = "/containers/{container_id}/members/{user_id}",
    tag = "Members",
    params(
        ("container_id" = Uuid, Path, description = "Container id"),
        ("user_id" = String, Path, description = "Member's user id"),
    ),
    request_body = UpdateMemberRoleRequest,
    responses(
        (status = 200, description = "Role updated", body = MemberRoleResponse),
        (status = 400, description = "Invalid role, or target is the owner"),
        (status = 403, description = "Not the owner"),
        (status = 404, description = "Member not found"),
        (status = 423, description = "Container is locked"),
    )
)]
async fn update_member_role(
    _access: ContainerAccess<Owner>,
    State(state): State<AppState>,
    Path((container_id, user_id)): Path<(Uuid, String)>,
    Json(body): Json<UpdateMemberRoleRequest>,
) -> axum::response::Response {
    let Ok(role @ (Role::Editor | Role::Viewer)) = body.role.parse::<Role>() else {
        return error_response(StatusCode::BAD_REQUEST, "invalid_role");
    };

    let container = match containers_repo::get_by_id(&state.db, container_id).await {
        Ok(Some(container)) => container,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => return db_error(),
    };

    if container.owner_id == user_id {
        return error_response(StatusCode::BAD_REQUEST, "use_transfer_ownership");
    }

    match members_repo::update_role(&state.db, container_id, &user_id, role).await {
        Ok(true) => {
            state
                .cache
                .invalidate(&(container_id, user_id.clone()))
                .await;
            Json(MemberRoleResponse {
                user_id,
                role: role.as_str().to_string(),
            })
            .into_response()
        }
        Ok(false) => error_response(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => db_error(),
    }
}

#[utoipa::path(
    delete,
    path = "/containers/{container_id}/members/{user_id}",
    tag = "Members",
    params(
        ("container_id" = Uuid, Path, description = "Container id"),
        ("user_id" = String, Path, description = "Member's user id"),
    ),
    responses(
        (status = 204, description = "Member removed (or self-left)"),
        (status = 403, description = "Not the owner, and not removing yourself"),
        (status = 404, description = "Member not found"),
        (status = 409, description = "Target is the owner — transfer ownership first"),
        (status = 423, description = "Container is locked (non-owner)"),
    )
)]
async fn remove_member(
    access: ContainerAccess<Viewer>,
    State(state): State<AppState>,
    Path((container_id, user_id)): Path<(Uuid, String)>,
) -> axum::response::Response {
    if user_id != access.user.id && access.role != Role::Owner {
        return error_response(StatusCode::FORBIDDEN, "forbidden");
    }

    match members_repo::remove_member(&state.db, container_id, &user_id).await {
        Ok(true) => {
            state.cache.invalidate(&(container_id, user_id)).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => error_response(StatusCode::NOT_FOUND, "not_found"),
        Err(MembersRepoError::Domain(DomainError::OwnerTransferRequired)) => {
            error_response(StatusCode::CONFLICT, "owner_must_transfer_first")
        }
        Err(_) => db_error(),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct TransferOwnershipRequest {
    pub new_owner_user_id: String,
}

#[utoipa::path(
    post,
    path = "/containers/{container_id}/transfer-ownership",
    tag = "Members",
    params(("container_id" = Uuid, Path, description = "Container id")),
    request_body = TransferOwnershipRequest,
    responses(
        (status = 200, description = "Ownership transferred"),
        (status = 400, description = "Target is already the owner"),
        (status = 403, description = "Not the owner"),
        (status = 404, description = "Target is not a member of this container"),
        (status = 423, description = "Container is locked"),
    )
)]
async fn transfer_ownership(
    access: ContainerAccess<Owner>,
    State(state): State<AppState>,
    Path(container_id): Path<Uuid>,
    Json(body): Json<TransferOwnershipRequest>,
) -> axum::response::Response {
    if body.new_owner_user_id == access.user.id {
        return error_response(StatusCode::BAD_REQUEST, "already_the_owner");
    }

    match members_repo::transfer_ownership(
        &state.db,
        container_id,
        &access.user.id,
        &body.new_owner_user_id,
    )
    .await
    {
        Ok(()) => {
            state
                .cache
                .invalidate(&(container_id, access.user.id.clone()))
                .await;
            state
                .cache
                .invalidate(&(container_id, body.new_owner_user_id))
                .await;
            StatusCode::OK.into_response()
        }
        Err(MembersRepoError::Domain(DomainError::NotFound)) => {
            error_response(StatusCode::NOT_FOUND, "target_not_a_member")
        }
        Err(_) => db_error(),
    }
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_members))
        .routes(routes!(update_member_role))
        .routes(routes!(remove_member))
        .routes(routes!(transfer_ownership))
}

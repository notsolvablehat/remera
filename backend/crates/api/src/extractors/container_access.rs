use axum::{
    extract::{FromRequestParts, Path},
    http::request::Parts,
    response::Response,
};
use serde::Deserialize;

use domain::{Role, role_meets_minimum};
use uuid::Uuid;

use crate::{extractors::auth_user::AuthUser, state::AppState};

pub struct Viewer;
pub struct Editor;
pub struct Owner;

pub struct ContainerAccess<R> {
    pub user: AuthUser,
    pub role: Role,
    // PhantomData because R (Viewer/Editor/Owner) only exists to make
    // ContainerAccess<Viewer> and ContainerAccess<Editor> distinct types
    // at the handler signature level — it's never stored as data.
    _marker: std::marker::PhantomData<R>,
}

#[derive(Deserialize)]
struct ContainerPath {
    container_id: Uuid,
}

/// Cached alongside (but separately from) the per-user role cache —
/// locked/deleted are properties of the *container*, not of a
/// particular membership, so one cache entry per container (not per
/// (container, user) pair) covers every caller.
#[derive(Debug, Clone, Copy)]
pub struct ContainerStatus {
    pub is_locked: bool,
    pub is_deleted: bool,
}

async fn get_container_status(
    state: &AppState,
    container_id: Uuid,
) -> Result<Option<ContainerStatus>, sqlx::Error> {
    if let Some(status) = state.container_status_cache.get(&container_id).await {
        return Ok(Some(status));
    }

    let row = sqlx::query!(
        "select is_locked, deleted_at from container where id = $1",
        container_id
    )
    .fetch_optional(&state.db)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let status = ContainerStatus {
        is_locked: row.is_locked,
        is_deleted: row.deleted_at.is_some(),
    };

    state
        .container_status_cache
        .insert(container_id, status)
        .await;

    Ok(Some(status))
}

// Stamps out one FromRequestParts impl per role marker instead of
// hand-writing the same lookup three times.
macro_rules! impl_container_access {
    ($marker:ty, $min_role:expr) => {
        impl FromRequestParts<AppState> for ContainerAccess<$marker> {
            type Rejection = Response;

            async fn from_request_parts(
                parts: &mut Parts,
                state: &AppState,
            ) -> Result<Self, Self::Rejection> {
                use axum::{Json, http::StatusCode, response::IntoResponse};

                let user = AuthUser::from_request_parts(parts, state)
                    .await
                    .map_err(IntoResponse::into_response)?;

                let Path(ContainerPath { container_id }) =
                    Path::from_request_parts(parts, state).await.map_err(|_| {
                        (StatusCode::BAD_REQUEST, "missing_container_id").into_response()
                    })?;

                // Check the cache before hitting Postgres — moka::future::Cache
                // already lives on AppState (see state.rs), so this is free
                // once "revisit: moka role cache" from the guide is wired in.
                let cached = state
                    .cache
                    .get(&(container_id, user.id.clone()))
                    .await;

                let member_role = if let Some(role) = cached {
                    Some(role)
                } else {
                    let row = sqlx::query!(
                        "select role from container_member where container_id = $1 and user_id = $2",
                        container_id,
                        user.id
                    )
                    .fetch_optional(&state.db)
                    .await
                    .map_err(|_| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({"error": "db error"})),
                        )
                            .into_response()
                    })?;

                    let role = row.and_then(|r| r.role.parse::<Role>().ok());

                    if let Some(role) = role {
                        state
                            .cache
                            .insert((container_id, user.id.clone()), role)
                            .await;
                    }

                    role
                };

                // Locked/deleted checks come before the "are you even a
                // member" check, matching the order in
                // docs/architecture/howisthebackendstructured-1.md's
                // request-flow traces — a locked container blocks
                // everyone (including non-members) except its owner.
                let status = get_container_status(state, container_id)
                    .await
                    .map_err(|_| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({"error": "db error"})),
                        )
                            .into_response()
                    })?
                    .ok_or_else(|| {
                        (
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({"error": "not_found"})),
                        )
                            .into_response()
                    })?;

                if status.is_deleted {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": "not_found"})),
                    )
                        .into_response());
                }

                if status.is_locked && member_role != Some(Role::Owner) {
                    return Err((
                        StatusCode::LOCKED,
                        Json(serde_json::json!({"error": "container_locked"})),
                    )
                        .into_response());
                }

                let Some(member_role) = member_role else {
                    return Err((
                        StatusCode::FORBIDDEN,
                        Json(serde_json::json!({"error": "forbidden"})),
                    )
                        .into_response());
                };

                if !role_meets_minimum(member_role, $min_role) {
                    return Err((
                        StatusCode::FORBIDDEN,
                        Json(serde_json::json!({"error": "forbidden"})),
                    )
                        .into_response());
                }

                Ok(ContainerAccess {
                    user,
                    role: member_role,
                    _marker: std::marker::PhantomData,
                })
            }
        }
    };
}

impl_container_access!(Viewer, Role::Viewer);
impl_container_access!(Editor, Role::Editor);
impl_container_access!(Owner, Role::Owner);

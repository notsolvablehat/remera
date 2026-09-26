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
                    role
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

                    let role = row
                        .and_then(|r| r.role.parse::<Role>().ok())
                        .ok_or_else(|| {
                            (
                                StatusCode::FORBIDDEN,
                                Json(serde_json::json!({"error": "forbidden"})),
                            )
                                .into_response()
                        })?;

                    state
                        .cache
                        .insert((container_id, user.id.clone()), role)
                        .await;

                    role
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

use axum::{
    extract::{FromRequestParts, Path, Query},
    http::request::Parts,
    response::Response,
};
use serde::Deserialize;

use domain::{Role, role_meets_minimum};
use uuid::Uuid;

use crate::{
    extractors::auth_user::{AuthUser, MaybeUser},
    state::AppState,
};

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

fn error_response(status: axum::http::StatusCode, error: &str) -> Response {
    use axum::{Json, response::IntoResponse};
    (status, Json(serde_json::json!({"error": error}))).into_response()
}

fn db_error() -> Response {
    error_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "db_error")
}

/// Cached alongside (but separately from) the per-user role cache —
/// locked/deleted/the share-link id are properties of the *container*,
/// not of a particular membership, so one cache entry per container
/// (not per (container, user) pair) covers every caller.
#[derive(Debug, Clone, Copy)]
pub struct ContainerStatus {
    pub is_locked: bool,
    pub is_deleted: bool,
    pub share_link_id: Option<Uuid>,
}

async fn get_container_status(
    state: &AppState,
    container_id: Uuid,
) -> Result<Option<ContainerStatus>, sqlx::Error> {
    if let Some(status) = state.container_status_cache.get(&container_id).await {
        return Ok(Some(status));
    }

    let row = sqlx::query!(
        "select is_locked, deleted_at, share_link_id from container where id = $1",
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
        share_link_id: row.share_link_id,
    };

    state
        .container_status_cache
        .insert(container_id, status)
        .await;

    Ok(Some(status))
}

/// Cache-then-DB role lookup, shared by `ContainerAccess<R>` and
/// `ContainerViewAccess`.
async fn get_member_role(
    state: &AppState,
    container_id: Uuid,
    user_id: &str,
) -> Result<Option<Role>, sqlx::Error> {
    if let Some(role) = state.cache.get(&(container_id, user_id.to_string())).await {
        return Ok(Some(role));
    }

    let row = sqlx::query!(
        "select role from container_member where container_id = $1 and user_id = $2",
        container_id,
        user_id
    )
    .fetch_optional(&state.db)
    .await?;

    let role = row.and_then(|r| r.role.parse::<Role>().ok());

    if let Some(role) = role {
        state
            .cache
            .insert((container_id, user_id.to_string()), role)
            .await;
    }

    Ok(role)
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
                use axum::{http::StatusCode, response::IntoResponse};

                let user = AuthUser::from_request_parts(parts, state)
                    .await
                    .map_err(IntoResponse::into_response)?;

                let Path(ContainerPath { container_id }) = Path::from_request_parts(parts, state)
                    .await
                    .map_err(|_| error_response(StatusCode::BAD_REQUEST, "missing_container_id"))?;

                let member_role = get_member_role(state, container_id, &user.id)
                    .await
                    .map_err(|_| db_error())?;

                // Locked/deleted checks come before the "are you even a
                // member" check, matching the order in
                // docs/architecture/howisthebackendstructured-1.md's
                // request-flow traces — a locked container blocks
                // everyone (including non-members) except its owner.
                let status = get_container_status(state, container_id)
                    .await
                    .map_err(|_| db_error())?
                    .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "not_found"))?;

                if status.is_deleted {
                    return Err(error_response(StatusCode::NOT_FOUND, "not_found"));
                }

                if status.is_locked && member_role != Some(Role::Owner) {
                    return Err(error_response(StatusCode::LOCKED, "container_locked"));
                }

                let Some(member_role) = member_role else {
                    return Err(error_response(StatusCode::FORBIDDEN, "forbidden"));
                };

                if !role_meets_minimum(member_role, $min_role) {
                    return Err(error_response(StatusCode::FORBIDDEN, "forbidden"));
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

#[derive(Deserialize)]
struct ShareTokenQuery {
    share_token: Option<String>,
}

/// View access for routes anonymous share-link viewers should reach —
/// the media read routes, not membership-sensitive ones like
/// `GET /containers/{cid}` or the members list. Tries, in order:
/// 1. An authenticated member with Viewer+ role (same check as
///    `ContainerAccess<Viewer>`).
/// 2. A `?share_token=` query param that decrypts to this container's
///    id and matches its *current* `share_link_id` (rotation
///    invalidates old tokens — see `containers_repo::rotate_share_link_id`).
///
/// A locked container rejects both paths — "locking overrides even
/// public view access" per backend/AGENTS.md's design decisions —
/// except an authenticated Owner, same as `ContainerAccess<R>`.
pub struct ContainerViewAccess {
    // Not read by any handler yet (they don't need to distinguish an
    // authenticated member from an anonymous share-link viewer today) —
    // kept for whichever route needs that distinction next (e.g.
    // attributing a view, or a per-user rate limit tighter than the
    // global one).
    #[allow(dead_code)]
    pub user: Option<AuthUser>,
}

impl FromRequestParts<AppState> for ContainerViewAccess {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        use axum::http::StatusCode;

        let Path(ContainerPath { container_id }) = Path::from_request_parts(parts, state)
            .await
            .map_err(|_| error_response(StatusCode::BAD_REQUEST, "missing_container_id"))?;

        let status = get_container_status(state, container_id)
            .await
            .map_err(|_| db_error())?
            .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "not_found"))?;

        if status.is_deleted {
            return Err(error_response(StatusCode::NOT_FOUND, "not_found"));
        }

        let MaybeUser(maybe_user) = MaybeUser::from_request_parts(parts, state)
            .await
            .expect("MaybeUser is infallible");

        if let Some(user) = maybe_user {
            let member_role = get_member_role(state, container_id, &user.id)
                .await
                .map_err(|_| db_error())?;

            if status.is_locked && member_role != Some(Role::Owner) {
                return Err(error_response(StatusCode::LOCKED, "container_locked"));
            }

            if let Some(role) = member_role
                && role_meets_minimum(role, Role::Viewer)
            {
                return Ok(ContainerViewAccess { user: Some(user) });
            }

            // Authenticated but not a sufficient member — fall through
            // to the share-token check below rather than rejecting
            // outright, since a non-member may still hold a valid link.
        } else if status.is_locked {
            return Err(error_response(StatusCode::LOCKED, "container_locked"));
        }

        let Query(ShareTokenQuery { share_token }) = Query::from_request_parts(parts, state)
            .await
            .unwrap_or(Query(ShareTokenQuery { share_token: None }));

        if let Some(token) = share_token
            && let Ok((token_container_id, token_share_link_id)) =
                state.share_link_codec.decode(&token)
            && token_container_id == container_id
            && status.share_link_id == Some(token_share_link_id)
        {
            return Ok(ContainerViewAccess { user: None });
        }

        Err(error_response(StatusCode::FORBIDDEN, "forbidden"))
    }
}

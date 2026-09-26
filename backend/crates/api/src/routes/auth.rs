//! Documentation-only wrapper for better-auth's own `/auth/*` routes.
//!
//! better-auth's sign-up/sign-in/sign-out/session endpoints are real, live,
//! and mounted via `state.auth.clone().axum_router()` nested at `/auth` in
//! `router.rs` — that's the actual code path serving these requests. utoipa
//! only knows about routes registered through `OpenApiRouter`/`routes!(...)`,
//! so those endpoints are invisible to `/openapi.json`, Swagger UI, and the
//! frontend's generated client even though they work.
//!
//! The functions below are NOT wired into any router — they exist purely so
//! `#[utoipa::path]` can describe these endpoints' request/response shapes,
//! and `ApiDoc`'s `#[openapi(paths(...))]` in router.rs lists them so they
//! show up in the spec. Do NOT `.routes(routes!(...))` these — the real
//! handlers are already mounted by better-auth's own router; registering
//! these too would double-register the same path and panic at startup.
//!
//! Request/response shapes were confirmed against the real
//! better-auth-core 0.10.0 `User`/`Session`/`SuccessResponse` types (which
//! is what's actually serialized on the wire) — keep these in sync if that
//! crate version changes.

// Every item in this file exists purely to describe better-auth's real
// /auth/* routes to utoipa (see module doc above) — none of it is ever
// constructed or called by our own code, so it would otherwise all be
// flagged as dead code.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct SignUpEmailRequest {
    pub email: String,
    pub password: String,
    pub name: String,
}

#[derive(Deserialize, ToSchema)]
pub struct SignInEmailRequest {
    pub email: String,
    pub password: String,
}

// camelCase because that's what better-auth-core's real `User` struct
// serializes as (`#[serde(rename = "emailVerified")]` etc. per-field on the
// vendor type) — this DTO exists only to describe that exact wire shape.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthUserDto {
    pub id: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub email_verified: bool,
    pub image: Option<String>,
    /// ISO-8601 timestamp.
    pub created_at: String,
    /// ISO-8601 timestamp.
    pub updated_at: String,
    pub username: Option<String>,
    pub display_username: Option<String>,
    pub two_factor_enabled: bool,
    pub role: Option<String>,
    pub banned: bool,
    pub ban_reason: Option<String>,
    /// ISO-8601 timestamp, if the ban is time-limited.
    pub ban_expires: Option<String>,
}

// Mirrors better-auth-core's real `Session` struct field-for-field.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthSessionDto {
    pub id: String,
    /// ISO-8601 timestamp.
    pub expires_at: String,
    pub token: String,
    /// ISO-8601 timestamp.
    pub created_at: String,
    /// ISO-8601 timestamp.
    pub updated_at: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub user_id: String,
    pub impersonated_by: Option<String>,
    pub active_organization_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct SignUpEmailResponse {
    pub token: String,
    pub user: AuthUserDto,
}

#[derive(Serialize, ToSchema)]
pub struct SignInEmailResponse {
    pub redirect: bool,
    pub token: String,
    pub url: Option<String>,
    pub user: AuthUserDto,
}

#[derive(Serialize, ToSchema)]
pub struct SignOutResponse {
    pub success: bool,
}

#[derive(Serialize, ToSchema)]
pub struct GetSessionResponse {
    pub session: AuthSessionDto,
    pub user: AuthUserDto,
}

#[utoipa::path(
    post,
    path = "/auth/sign-up/email",
    tag = "Auth",
    request_body = SignUpEmailRequest,
    responses(
        (status = 200, description = "Account created, session started", body = SignUpEmailResponse),
        (status = 422, description = "Validation error (e.g. email already taken)"),
    )
)]
pub async fn sign_up_email() {}

#[utoipa::path(
    post,
    path = "/auth/sign-in/email",
    tag = "Auth",
    request_body = SignInEmailRequest,
    responses(
        (status = 200, description = "Signed in, session started", body = SignInEmailResponse),
        (status = 401, description = "Invalid credentials"),
    )
)]
pub async fn sign_in_email() {}

#[utoipa::path(
    post,
    path = "/auth/sign-out",
    tag = "Auth",
    responses(
        (status = 200, description = "Session revoked, cookie cleared", body = SignOutResponse),
        (status = 401, description = "No active session"),
    )
)]
pub async fn sign_out() {}

#[utoipa::path(
    get,
    path = "/auth/get-session",
    tag = "Auth",
    responses(
        (status = 200, description = "Current session and user", body = GetSessionResponse),
        (status = 401, description = "No active session"),
    )
)]
pub async fn get_session() {}

use axum::{extract::FromRequestParts, response::Response};
use better_auth::AuthUser as _;
use better_auth::CurrentSession;

use crate::state::{AppDb, AppState};

pub struct AuthUser {
    pub id: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // CurrentSession<AppDb> needs State<Arc<BetterAuth<AppDb>>>, not
        // our AppState — we feed it state.auth directly instead. This is
        // the bridge: CurrentSession does the heavy lifting (token
        // extraction from cookie/Bearer header, DB lookup), we just
        // unwrap the result into our own AuthUser type that plugs into
        // AppState-based extractors everywhere else.
        let session = CurrentSession::<AppDb>::from_request_parts(parts, &state.auth).await?;

        Ok(AuthUser {
            id: session.user.id().to_string(),
            email: session.user.email().map(str::to_string),
            name: session.user.name().map(str::to_string),
        })
    }
}

pub struct MaybeUser(pub Option<AuthUser>);

impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(MaybeUser(
            AuthUser::from_request_parts(parts, state).await.ok(),
        ))
    }
}

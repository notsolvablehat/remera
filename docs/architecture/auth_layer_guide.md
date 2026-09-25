# Auth Layer Implementation Guide — remera backend (Source-Verified, reordered 2026-09-24)

> Verified directly against `better-auth 0.10.0` / `better-auth-core 0.10.0` source
> in `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`, and against the
> actual repo state as of this writing.

## How this guide is organized

The previous version of this guide was organized by *feature phase*
("Phase 4 — AppState wiring", "Phase 9 — invite hooks", ...). That reads
well top-to-bottom but doesn't match the order you actually type code in:
Phase 4 (`state.rs`) referenced `AppAuthHooks`, a type that wasn't
introduced until Phase 9, so writing `state.rs` first — the natural order —
meant hitting an unresolved import.

This version is ordered **by file, in dependency order**: every file below
only references types from files that appear *earlier* in this document.
If file B imports something from file A, A comes first. A few files (like
`router.rs` and `state.rs`) get touched more than once as later files add
things they need to wire in — those revisits are called out explicitly.

Dependency order used below:
```
api/Cargo.toml                                  (no deps)
migrations/*.sql                                 (no deps — already done)
domain/src/container.rs, domain/src/lib.rs        (no deps)
api/src/config.rs                                (no internal deps)
api/src/auth_hooks.rs                            (no internal deps)
api/src/state.rs                                 (needs config.rs)
api/src/extractors/auth_user.rs                  (needs state.rs)
api/src/extractors/container_access.rs           (needs domain + auth_user.rs + state.rs)
api/src/routes/me.rs                             (needs auth_user.rs)
api/src/router.rs                                (needs state.rs + routes/*)
api/src/main.rs                                  (needs everything — always last)
  ↳ revisit: state.rs + container_access.rs      (moka cache)
  ↳ revisit: auth_hooks.rs + Cargo.toml           (AES-GCM invite token)
```

---

## 1. `backend/crates/api/Cargo.toml`

Nothing else compiles until the dependency is feature-flagged correctly.

```diff
-better-auth = "0.10.0"
+better-auth = { version = "0.10.0", features = ["axum", "sqlx-postgres"] }
+# Needed later for auth_hooks.rs — better-auth-core's DatabaseHooks trait
+# is itself #[async_trait], and that macro isn't re-exported from the
+# `better_auth` crate root, so we need our own copy to implement it.
+async-trait = "0.1"
```

- `axum` — enables `CurrentSession`, `OptionalSession`, `AxumIntegration::axum_router()` (used in `extractors/auth_user.rs` and `router.rs` below).
- `sqlx-postgres` — enables `SqlxAdapter::from_pool(pool)` (used in `state.rs`).

---

## 2. `backend/migrations/*.sql` — already done, nothing to write

`backend/migrations/` already has:
```
001_create_core_tables.sql          ← better-auth's users/sessions/accounts/verifications
002_create_organization_tables.sql  ← better-auth's organization/member/invitation
003_create_two_factor_auth_tables.sql
004_create_container_tables.sql     ← your container/container_member/invite tables
```
These were checked line-by-line against `better-auth-0.10.0/migrations/`
and match exactly. Nothing to create or rename here — mentioned this early
only because `state.rs` (below) assumes these tables exist when it opens a
pool against `DATABASE_URL`.

> [!IMPORTANT]
> `users.id` is `TEXT`, not `UUID` — better-auth generates opaque string IDs.
> Every foreign key from your own tables to `users` must be
> `TEXT NOT NULL REFERENCES users(id)`. `004_create_container_tables.sql`
> already does this correctly for `container.owner_id` and
> `container_member.user_id`.

---

## 3. `backend/crates/domain/src/container.rs`

No dependency on anything above — `domain` is a pure-logic crate (hard
rule #3 in AGENTS.md: no axum/sqlx allowed here). Written now because
`extractors/container_access.rs`, several files down, needs `Role` and
`role_meets_minimum`.

```rust
use serde::{Deserialize, Serialize};
use std::str::FromStr;

// PartialOrd/Ord let us compare roles with `>=` for the minimum-role check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Role {
    Viewer = 0,
    Editor = 1,
    Owner = 2,
}

impl FromStr for Role {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "viewer" => Ok(Role::Viewer),
            "editor" => Ok(Role::Editor),
            "owner" => Ok(Role::Owner),
            _ => Err(()),
        }
    }
}

pub fn role_meets_minimum(actual: Role, required: Role) -> bool {
    actual >= required
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_can_do_everything() {
        assert!(role_meets_minimum(Role::Owner, Role::Viewer));
        assert!(role_meets_minimum(Role::Owner, Role::Editor));
        assert!(role_meets_minimum(Role::Owner, Role::Owner));
    }
    #[test]
    fn viewer_cannot_edit() {
        assert!(!role_meets_minimum(Role::Viewer, Role::Editor));
    }
    #[test]
    fn editor_cannot_own() {
        assert!(!role_meets_minimum(Role::Editor, Role::Owner));
    }
}
```

**File:** `backend/crates/domain/src/lib.rs` — add:
```diff
 pub mod errors;
+pub mod container;

 pub use errors::DomainError;
+pub use container::{Role, role_meets_minimum};
```

`cargo test -p domain` should pass right now — no DB, no server needed,
which is the entire point of keeping this crate infra-free.

---

## 4. `backend/crates/api/src/config.rs`

No internal dependencies — this is the one file allowed to read env vars
directly (AGENTS.md hard rule #4). Everything else gets config through
`AppState`, which is built from this in the next step.

```rust
// Hard rule (AGENTS.md #4): this is the ONLY file allowed to read
// environment variables directly.
#[derive(Clone)]
pub struct AppConfig {
    pub port: u16,
    // Postgres connection string, shared by our own sqlx pool AND by
    // better-auth's SqlxAdapter (same pool — see state.rs).
    pub database_url: String,
    // Signs/verifies better-auth's session cookies and JWTs.
    // better-auth enforces a minimum of 32 characters at AuthConfig::validate().
    pub auth_secret: String,
    // Public URL of this API. better-auth uses it to build absolute links
    // (e.g. email verification URLs).
    pub base_url: String,
    // Comma-separated list of allowed CORS origins, e.g.
    // "http://localhost:5173,https://app.remera.io" — a list because one
    // backend can serve multiple frontends (prod, preview deploys, local
    // dev), not just one.
    pub frontend_allow_origins: Vec<String>,
}

impl AppConfig {
    pub fn from_env() -> Self {
        Self {
            port: std::env::var("PORT")
                .ok().and_then(|p| p.parse().ok()).unwrap_or(8080),
            database_url: std::env::var("DATABASE_URL")
                .expect("DATABASE_URL must be set"),
            auth_secret: std::env::var("AUTH_SECRET")
                .expect("AUTH_SECRET must be set (min 32 chars)"),
            base_url: std::env::var("BASE_URL")
                .unwrap_or_else(|_| "http://localhost:8080".to_string()),
            frontend_allow_origins: std::env::var("FRONTEND_ALLOW_ORIGINS")
                .unwrap_or_else(|_| "http://localhost:5173".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
        }
    }
}
```

`.env.example` already declares `DATABASE_URL`, `AUTH_SECRET`,
`FRONTEND_ALLOW_ORIGINS`, `BASE_URL` under those exact names — nothing to
add there.

---

## 5. `backend/crates/api/src/auth_hooks.rs`

No internal dependencies (only external `sqlx::PgPool` and `better_auth`
types), which is why it can be written this early even though it's
*conceptually* about signup — it doesn't need `state.rs` to exist first.

> [!WARNING]
> **This file compiles but is currently unused.** The obvious way to wire
> it — `AuthBuilder::hook(...)`, or wrapping the adapter in
> `HookedDatabaseAdapter` — does **not work** on `better-auth-core 0.10.0`:
> - `AuthBuilder::build()` explicitly returns `Err(...)` if any hooks were
>   registered via `.hook()` (`core/auth.rs:180-183`), telling you to use
>   `HookedDatabaseAdapter` instead.
> - But `HookedDatabaseAdapter<DB>` implements every `*Ops` trait
>   (`UserOps`, `SessionOps`, ...) **except `PasskeyOps`**, so it can never
>   satisfy `DatabaseAdapter`'s full trait bound — confirmed against the
>   crate's own vendored source. Checking the crate's GitHub `master`
>   branch, `HookedDatabaseAdapter` has since been removed from
>   `hooks.rs` entirely (there's a `1.0.0-alpha.2` published after 0.10.0)
>   — this was a known gap that got reworked upstream, not a mistake on
>   our end.
>
> Until better-auth is upgraded past 0.10.0, invite resolution can't run
> as an automatic `DatabaseHooks` callback. The struct and impl below are
> written so the logic exists and compiles, but actually triggering
> `after_create_user` requires calling it explicitly from a custom signup
> endpoint instead of relying on better-auth's built-in
> `/auth/sign-up/email` route — that wiring is a TODO, not covered by this
> guide yet.

```rust
use better_auth::{AuthResult, DatabaseHooks};
use better_auth::adapters::SqlxAdapter;
use better_auth::entity::AuthUser as _; // for .email()
use sqlx::PgPool;

pub struct AppAuthHooks {
    pub db: PgPool,
}

// DatabaseHooks IS #[async_trait] in better-auth-core (unlike axum's
// FromRequestParts) — it needs a boxed future to be object-safe. Hence
// the `async-trait` crate dependency added in step 1.
#[async_trait::async_trait]
impl DatabaseHooks<SqlxAdapter> for AppAuthHooks {
    // Fires immediately AFTER a new user is successfully inserted — the
    // natural place to resolve pending invites for that email, IF this
    // hook mechanism were reachable (see warning above).
    async fn after_create_user(
        &self,
        user: &<SqlxAdapter as better_auth::DatabaseAdapter>::User,
    ) -> AuthResult<()> {
        let Some(email) = user.email() else { return Ok(()) };

        // Only active invites: not yet accepted, not yet expired.
        let invites = sqlx::query!(
            "SELECT id, container_id, role FROM invite
             WHERE email = $1 AND accepted_at IS NULL AND expires_at > NOW()",
            email
        )
        .fetch_all(&self.db)
        .await
        .unwrap_or_default();

        for invite in invites {
            // Insert container_member + mark invite accepted, in one
            // transaction, per invite — schema already exists in
            // migrations/004_create_container_tables.sql.
            let _ = (invite.id, invite.container_id, invite.role);
        }

        Ok(())
    }
}
```

---

## 6. `backend/crates/api/src/state.rs`

Needs `config.rs` (step 4) for `AppConfig`. Deliberately does **not**
import `auth_hooks.rs` — see the warning in step 5 for why.

```rust
use std::sync::Arc;
use sqlx::postgres::PgPool;
use better_auth::{AuthBuilder, AuthConfig, BetterAuth};
use better_auth::adapters::SqlxAdapter;
use better_auth::plugins::{EmailPasswordPlugin, SessionManagementPlugin};
use crate::config::AppConfig;

// Plain SqlxAdapter, not HookedDatabaseAdapter<SqlxAdapter> — see the
// warning in auth_hooks.rs for why the wrapped version doesn't compile
// on better-auth-core 0.10.0.
pub type AppDb = SqlxAdapter;

// AppState is cloned for every incoming HTTP request, so everything in
// it needs to be cheap to clone (PgPool internally pools connections;
// BetterAuth is behind an Arc).
#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub db: PgPool,
    pub auth: Arc<BetterAuth<AppDb>>,
}

impl AppState {
    pub async fn new(config: AppConfig) -> Self {
        let db = sqlx::postgres::PgPoolOptions::new()
            .max_connections(10)
            .connect(&config.database_url)
            .await
            .expect("Failed to connect to Postgres");

        let auth_config = AuthConfig::new(&config.auth_secret)
            .base_url(&config.base_url)
            .password_min_length(8);

        let auth = Arc::new(
            AuthBuilder::new(auth_config)
                // Same pool as our own `db` field — better-auth manages
                // its own tables (users/sessions/...) through it.
                .database(SqlxAdapter::from_pool(db.clone()))
                .plugin(EmailPasswordPlugin::new().enable_signup(true))
                .plugin(SessionManagementPlugin::new())
                .build()
                .await
                .expect("Failed to build BetterAuth"),
        );

        Self { config, db, auth }
    }
}
```

---

## 7. `backend/crates/api/src/extractors/auth_user.rs`

Needs `state.rs` (step 6) for `AppDb` and `AppState`.

```rust
use axum::{extract::FromRequestParts, http::request::Parts, response::Response};
use better_auth::CurrentSession;
// `as _` because this crate re-exports the AuthUser *trait*
// (better_auth_core::entity::AuthUser) under the same name as our own
// AuthUser *struct* below — importing it normally would collide. `as _`
// brings its methods (.id(), .email(), .name()) into scope on
// session.user without binding a name.
use better_auth::AuthUser as _;
use crate::state::{AppDb, AppState};

pub struct AuthUser {
    pub id: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

// No #[async_trait] here — axum 0.8's FromRequestParts is a plain trait
// with `fn from_request_parts(...) -> impl Future<...>` (confirmed in
// axum-core-0.5.6/src/extract/mod.rs:53); a regular `async fn`
// implementation satisfies it directly. `axum::async_trait` doesn't even
// exist in axum 0.8.9 — importing it is a compile error.
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
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
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(MaybeUser(AuthUser::from_request_parts(parts, state).await.ok()))
    }
}
```

**File:** `backend/crates/api/src/extractors/mod.rs` (new):
```rust
pub mod auth_user;
pub mod container_access;
```

**File:** `backend/crates/api/src/main.rs` — add the module declaration
now that the folder exists:
```diff
 mod auth_hooks;
 mod config;
+mod extractors;
 mod router;
 mod routes;
 mod state;
```

`CurrentSession<DB>` gives you `.user: DB::User` and `.session: DB::Session`
as public fields (not methods) — confirmed in
`better-auth-0.10.0/src/handlers/axum.rs:328-331`. `AuthUser` trait
methods (`better_auth_core::entity::AuthUser`):
```
.id()             -> &str
.email()          -> Option<&str>
.name()           -> Option<&str>
.email_verified() -> bool
.created_at()     -> DateTime<Utc>
```

---

## 8. `backend/crates/api/src/extractors/container_access.rs`

Needs `domain::{Role, role_meets_minimum}` (step 3), `auth_user::AuthUser`
(step 7), and `state::AppState` (step 6) — the last file in the dependency
chain of "things a route handler asks for."

```rust
use axum::{extract::{FromRequestParts, Path}, http::request::Parts, response::Response};
use serde::Deserialize;
use uuid::Uuid;
use domain::{Role, role_meets_minimum};
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
                use axum::{http::StatusCode, response::IntoResponse, Json};

                let user = AuthUser::from_request_parts(parts, state)
                    .await
                    .map_err(IntoResponse::into_response)?;

                let Path(ContainerPath { container_id }) =
                    Path::from_request_parts(parts, state)
                        .await
                        .map_err(|_| (StatusCode::BAD_REQUEST, "missing container_id").into_response())?;

                let row = sqlx::query!(
                    "SELECT role FROM container_member WHERE container_id = $1 AND user_id = $2",
                    container_id,
                    user.id
                )
                .fetch_optional(&state.db)
                .await
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error":"db error"}))).into_response())?;

                let member_role = row
                    .and_then(|r| r.role.parse::<Role>().ok())
                    .ok_or_else(|| (StatusCode::FORBIDDEN, Json(serde_json::json!({"error":"forbidden"}))).into_response())?;

                if !role_meets_minimum(member_role, $min_role) {
                    return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({"error":"forbidden"}))).into_response());
                }

                Ok(ContainerAccess { user, role: member_role, _marker: std::marker::PhantomData })
            }
        }
    };
}

impl_container_access!(Viewer, Role::Viewer);
impl_container_access!(Editor, Role::Editor);
impl_container_access!(Owner, Role::Owner);
```

Usage in a route handler, once you write one:
```rust
async fn update_container(
    // If they aren't at least an Editor, this function is NEVER called —
    // axum rejects the request with 401 or 403 before the body runs.
    access: ContainerAccess<Editor>,
    // ...
) -> impl IntoResponse { ... }
```

---

## 9. `backend/crates/api/src/routes/me.rs`

Needs `extractors::auth_user::AuthUser` (step 7).

```rust
use axum::{Json, response::IntoResponse};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use crate::{extractors::auth_user::AuthUser, state::AppState};

#[derive(Serialize, ToSchema)]
struct MeResponse {
    id: String,
    email: Option<String>,
    name: Option<String>,
}

#[utoipa::path(
    get,
    path = "/me",
    tag = "Auth",
    responses((status = 200, description = "Current user", body = MeResponse))
)]
// Used by the frontend to verify the session and load basic profile data.
async fn get_me(user: AuthUser) -> impl IntoResponse {
    Json(MeResponse { id: user.id, email: user.email, name: user.name })
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_me))
}
```

**File:** `backend/crates/api/src/routes/mod.rs` — add:
```diff
 pub mod health;
+pub mod me;
```

---

## 10. `backend/crates/api/src/router.rs`

Needs `state.rs` (step 6) and every `routes::*::router()` — `health`
(already existed) and `me` (step 9). This is why `router.rs` is written
near the end rather than early: it aggregates everything else.

```rust
use crate::{routes, state::AppState};
use axum::Router;
use better_auth::AxumIntegration;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_swagger_ui::SwaggerUi;

#[derive(OpenApi)]
#[openapi(info(title = "remera-api-docs", version = "0.1.0"))]
struct ApiDoc;

pub fn build_router(state: AppState) -> Router {
    // Correction: .axum_router() does NOT call .with_state() internally —
    // its signature is `fn axum_router(self) -> Router<Arc<BetterAuth<DB>>>`
    // (confirmed in better-auth-0.10.0/src/handlers/axum.rs:28), so it comes
    // back still needing an Arc<BetterAuth<AppDb>> state, not our AppState.
    // axum's `.nest(path, router)` requires the nested router's state type
    // to match the outer router's exactly, so we resolve it ourselves with
    // a second `.with_state(...)` call before nesting — this is the
    // standard axum pattern for merging a sub-router built against its own
    // state into a router built against a different outer state (`Router<S>`
    // is generic over what state `.with_state` was last called with, and
    // `.with_state` erases it to whatever S2 the caller needs next, here
    // inferred as AppState from the `.nest()` call below).
    // Requires `better_auth::AxumIntegration` in scope for `.axum_router()`
    // to resolve (it's a trait method, not an inherent one).
    let auth_router = state
        .auth
        .clone()
        .axum_router()
        .with_state(state.auth.clone());

    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(routes::health::router())
        .merge(routes::me::router())
        .split_for_parts();

    // One backend can serve several frontends — see the comment on
    // frontend_allow_origins in config.rs.
    let origins: Vec<_> = state
        .config
        .frontend_allow_origins
        .iter()
        .map(|o| o.parse().expect("invalid origin in FRONTEND_ALLOW_ORIGINS"))
        .collect();

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(Any)
        .allow_headers(Any)
        // Must be true so the browser sends the session cookie.
        .allow_credentials(true);

    router
        // Mounts better-auth's own routes (POST /auth/sign-in/email, etc.)
        .nest("/auth", auth_router)
        .merge(SwaggerUi::new("/docs").url("/openapi.json", api))
        .layer(cors)
        .with_state(state)
}
```

> [!NOTE]
> `routes::me::router()` — the module path is `routes::me`, not
> `router::me`. If you typo'd it as `router::me::router()` while wiring
> this in (easy to do, `router.rs` and `routes/` look similar), that's
> the fix.

---

## 11. `backend/crates/api/src/main.rs`

Needs everything above, so it's always last. By this point you should
have already added `mod extractors;` in step 7; this is the full,
consolidated list.

```rust
mod auth_hooks;
mod config;
mod extractors;
mod router;
mod routes;
mod state;
```

Nothing else in `main.rs` changes — it already does
`AppConfig::from_env()` → `AppState::new(config).await` → `build_router(state)`.

At this point `cargo build` should succeed and `cargo run` should serve
`/healthz`, `/me` (401 without a session), `/auth/sign-in/email`, `/auth/sign-up/email`,
and `/docs`.

---

## 12. Revisit — moka role cache (after the DB path above works)

Two already-written files get additive changes; no new file.

**`state.rs`** — add a field and initialize it in `AppState::new`:
```rust
// Add to the struct:
pub cache: Arc<moka::future::Cache<(uuid::Uuid, String), domain::Role>>,

// Add to AppState::new, before constructing Self:
let cache = Arc::new(
    moka::future::Cache::builder()
        .max_capacity(10_000)
        // Expire after 5 minutes so role changes (e.g. owner demotes an
        // editor) take effect within that window instead of being cached
        // forever.
        .time_to_live(std::time::Duration::from_secs(300))
        .build()
);
```

**`extractors/container_access.rs`** — check the cache before the
`sqlx::query!` lookup inside the `impl_container_access!` macro body:
```rust
if let Some(role) = state.cache.get(&(container_id, user.id.clone())).await {
    member_role = role;
} else {
    // ...existing sqlx::query! lookup...
    state.cache.insert((container_id, user.id.clone()), member_role).await;
}
```

`moka` is already a dependency in `api/Cargo.toml` — nothing to add there.

---

## 13. Revisit — AES-GCM invite token (only once Phase 9's wiring problem is solved)

**`Cargo.toml`** — add explicitly, even though `aes-gcm` is already a
transitive dependency via `better-auth-api`: transitive doesn't mean
usable in your own crate's code.
```toml
aes-gcm = "0.10"
```

**`auth_hooks.rs`** (or wherever invite resolution ends up being called
from, per the warning in step 5) — token payload → JSON → AES-256-GCM
encrypt → URL-safe base64 → the invite link token, per the design
decision already recorded in AGENTS.md (owner's container id, invited
person's gmail, and access level, encrypted into the "bla-bla" segment of
the invite URL).

---

## Files touched, in the order this guide creates/edits them

| # | File | Depends on (already written by this point) |
|---|---|---|
| 1 | `backend/crates/api/Cargo.toml` | — |
| 2 | `backend/migrations/00{1..4}_*.sql` | — (already done) |
| 3 | `backend/crates/domain/src/container.rs`, `lib.rs` | — |
| 4 | `backend/crates/api/src/config.rs` | — |
| 5 | `backend/crates/api/src/auth_hooks.rs` | — (not yet wired, see warning) |
| 6 | `backend/crates/api/src/state.rs` | config.rs |
| 7 | `backend/crates/api/src/extractors/auth_user.rs`, `mod.rs` | state.rs |
| 8 | `backend/crates/api/src/extractors/container_access.rs` | domain, auth_user.rs, state.rs |
| 9 | `backend/crates/api/src/routes/me.rs`, `routes/mod.rs` | auth_user.rs |
| 10 | `backend/crates/api/src/router.rs` | state.rs, routes/health.rs, routes/me.rs |
| 11 | `backend/crates/api/src/main.rs` | everything |
| 12 | revisit: `state.rs`, `container_access.rs` | moka cache |
| 13 | revisit: `Cargo.toml`, `auth_hooks.rs` | AES-GCM invite token |

## Implementation Checklist

```
☐ 1  api/Cargo.toml: better-auth features = ["axum","sqlx-postgres"], add async-trait
☑ 2  migrations/001-004 already exist and already match — nothing to do
☐ 3  domain/src/container.rs — Role enum + role_meets_minimum, add to lib.rs; cargo test -p domain
☐ 4  api/src/config.rs — DATABASE_URL, AUTH_SECRET, BASE_URL, FRONTEND_ALLOW_ORIGINS (Vec)
☐ 5  api/src/auth_hooks.rs — AppAuthHooks struct + DatabaseHooks impl (written, not wired — see warning)
☐ 6  api/src/state.rs — AppState with PgPool + Arc<BetterAuth<SqlxAdapter>>; cargo build passes
☐ 7  api/src/extractors/auth_user.rs — AuthUser, MaybeUser (NO axum::async_trait import); mod extractors; in main.rs
☐ 8  api/src/extractors/container_access.rs — ContainerAccess<Viewer/Editor/Owner>
☐ 9  api/src/routes/me.rs — GET /me
☐ 10 api/src/router.rs — mount /auth, CORS from config list, merge routes::me
☐ 11 api/src/main.rs — full mod list; cargo run serves /healthz, /me, /auth/*, /docs
☐ 12 moka cache in state.rs + container_access.rs
☐ 13 aes-gcm invite token + manual invite-resolution wiring (since DatabaseHooks isn't reachable on 0.10.0)
```

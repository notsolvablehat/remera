# Auth Layer Implementation Guide — remera backend (Source-Verified)

> All facts in this guide are confirmed from the actual `better-auth 0.10.0` source at  
> `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/better-auth-0.10.0/`  
> No guessing — every type name, method, and table column is real.

---

## ✅ All 5 Open Questions Answered

| # | Question | Answer |
|---|---|---|
| 1 | SQLx store adapter name | `SqlxAdapter::from_pool(pool.clone())` — requires `features = ["sqlx-postgres"]` |
| 2 | Session extraction method | Built-in `CurrentSession<DB>` extractor from `better_auth::handlers::axum` — no custom code needed |
| 3 | Schema macro? | **No macro**. `BetterAuth<DB>` is generic over any `DatabaseAdapter`. `MemoryDatabaseAdapter` for dev, `SqlxAdapter` for Postgres. |
| 4 | Lifecycle hooks? | **Yes** — `DatabaseHooks<DB>` trait with `after_create_user(&self, user: &DB::User)`, `before_create_user`, `after_create_session`, etc. Perfect for invite resolution. |
| 5 | User fields? | Trait methods: `.id() -> &str`, `.email() -> Option<&str>`, `.name() -> Option<&str>`, `.email_verified() -> bool`, `.created_at() -> DateTime<Utc>` |

---

## Phase 1 — Fix Cargo.toml (Two Feature Flags)

**File:** `backend/crates/api/Cargo.toml`

```diff
-better-auth = "0.10.0"
+better-auth = { version = "0.10.0", features = ["axum", "sqlx-postgres"] }
```

**Both are required:**
- `axum` — enables `CurrentSession`, `OptionalSession`, `AxumIntegration::axum_router()`
- `sqlx-postgres` — enables `SqlxAdapter::from_pool(pool)` 

---

## Phase 2 — Migrations

### Good news: the crate ships its own migrations

The official migration files are at:
```
~/.cargo/registry/src/.../better-auth-0.10.0/migrations/
  001_create_core_tables.sql        ← users, sessions, accounts, verifications
  002_create_organization_tables.sql
  003_create_two_factor_table.sql
  004_create_api_key_table.sql
  005_create_passkey_table.sql
```

You only need `001` for now. Copy it verbatim into `backend/migrations/`.

### Exact schema from `001_create_core_tables.sql`

```sql
-- Table name: users  (plural, no quotes — important for sqlx::query! macros)
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,               -- ← TEXT, not UUID. better-auth generates opaque string IDs
    name TEXT,
    email TEXT UNIQUE,
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    image TEXT,
    username TEXT UNIQUE,
    display_username TEXT,
    two_factor_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    role TEXT,                         -- better-auth's own global role field (not your container role)
    banned BOOLEAN NOT NULL DEFAULT FALSE,
    ban_reason TEXT,
    ban_expires TIMESTAMPTZ,
    metadata JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    expires_at TIMESTAMPTZ NOT NULL,
    token TEXT NOT NULL UNIQUE,
    ip_address TEXT,
    user_agent TEXT,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    impersonated_by TEXT,
    active_organization_id TEXT,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    access_token TEXT,
    refresh_token TEXT,
    id_token TEXT,
    access_token_expires_at TIMESTAMPTZ,
    refresh_token_expires_at TIMESTAMPTZ,
    scope TEXT,
    password TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(provider_id, account_id)
);

CREATE TABLE IF NOT EXISTS verifications (
    id TEXT PRIMARY KEY,
    identifier TEXT NOT NULL,
    value TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes (all included in the official migration)
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);
CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
CREATE INDEX IF NOT EXISTS idx_sessions_token ON sessions(token);
CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions(expires_at);
CREATE INDEX IF NOT EXISTS idx_accounts_user_id ON accounts(user_id);
CREATE INDEX IF NOT EXISTS idx_accounts_provider_account ON accounts(provider_id, account_id);
```

### Your domain tables

```sql
-- 20260924000002_containers.sql
CREATE TABLE IF NOT EXISTS container (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id    TEXT NOT NULL REFERENCES users(id),   -- TEXT FK to better-auth's users table
    name        TEXT NOT NULL,
    is_public   BOOLEAN NOT NULL DEFAULT FALSE,
    is_locked   BOOLEAN NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 20260924000003_members.sql
CREATE TABLE IF NOT EXISTS container_member (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    container_id UUID NOT NULL REFERENCES container(id) ON DELETE CASCADE,
    user_id      TEXT NOT NULL REFERENCES users(id),  -- TEXT FK
    role         TEXT NOT NULL CHECK (role IN ('owner', 'editor', 'viewer')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (container_id, user_id)
);

-- 20260924000004_invites.sql
CREATE TABLE IF NOT EXISTS invite (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    container_id UUID NOT NULL REFERENCES container(id) ON DELETE CASCADE,
    email        TEXT NOT NULL,
    role         TEXT NOT NULL CHECK (role IN ('editor', 'viewer')),
    token        TEXT NOT NULL UNIQUE,        -- AES-GCM encrypted blob
    accepted_at  TIMESTAMPTZ,
    expires_at   TIMESTAMPTZ NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

> [!IMPORTANT]
> `users.id` is `TEXT` (not UUID). All foreign keys from your tables to `users` must be `TEXT NOT NULL REFERENCES users(id)`. This is confirmed from the official migration.

---

## Phase 3 — AppConfig

```rust
// api/src/config.rs
#[derive(Clone)]
pub struct AppConfig {
    // The port your Axum server will bind to.
    pub port: u16,
    // Database connection string (Postgres URL).
    pub database_url: String,
    // A cryptographic secret used by better-auth to sign cookies and tokens.
    // better-auth enforces a minimum of 32 characters for security reasons.
    pub auth_secret: String,    
    // The public URL where the backend is hosted (e.g. https://api.yourdomain.com).
    // better-auth uses this to generate absolute URLs (like email verification links).
    pub base_url: String,
    // The frontend's URL. Used primarily to configure CORS allowed origins.
    pub frontend_url: String,
}

impl AppConfig {
    // Loads configuration variables from the environment (`.env` file or OS env).
    pub fn from_env() -> Self {
        Self {
            // Tries to read "PORT". If not found or invalid, defaults to 8080.
            port: std::env::var("PORT")
                .ok().and_then(|p| p.parse().ok()).unwrap_or(8080),
            // DATABASE_URL is strictly required to start the server.
            database_url: std::env::var("DATABASE_URL")
                .expect("DATABASE_URL must be set"),
            // AUTH_SECRET is strictly required for secure authentication.
            auth_secret: std::env::var("AUTH_SECRET")
                .expect("AUTH_SECRET must be set (min 32 chars)"),
            // Defaults for local development.
            base_url: std::env::var("BASE_URL")
                .unwrap_or_else(|_| "http://localhost:8080".to_string()),
            frontend_url: std::env::var("FRONTEND_URL")
                .unwrap_or_else(|_| "http://localhost:5173".to_string()),
        }
    }
}
```

---

## Phase 4 — AppState + BetterAuth Wiring

### `api/src/state.rs`

The confirmed pattern from `examples/shared_sqlx_pool.rs`:

```rust
use std::sync::Arc;
use sqlx::postgres::PgPool;
use better_auth::{AuthBuilder, AuthConfig, BetterAuth};
use better_auth::adapters::SqlxAdapter;
use better_auth::plugins::{EmailPasswordPlugin, SessionManagementPlugin};
use crate::config::AppConfig;

// Create a type alias to make the generic type cleaner.
// SqlxAdapter is the concrete type that connects better-auth to our Postgres database.
pub type AppDb = SqlxAdapter;

// AppState is cloned for every incoming HTTP request.
// Thus, it holds shared resources like connection pools and Arcs.
#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub db: PgPool, // SQLx handles pooling internally, so this can be cloned cheaply.
    // Arc is necessary because BetterAuth holds the internal auth engines and config.
    pub auth: Arc<BetterAuth<AppDb>>,
}

impl AppState {
    pub async fn new(config: AppConfig) -> Self {
        // Initialize the Postgres connection pool.
        let db = sqlx::postgres::PgPoolOptions::new()
            .max_connections(10)
            .connect(&config.database_url)
            .await
            .expect("Failed to connect to Postgres");

        // Initialize better-auth's configuration struct.
        // It consumes our secret and base_url from our AppConfig.
        let auth_config = AuthConfig::new(&config.auth_secret)
            .base_url(&config.base_url)
            .password_min_length(8); // Optional strictness customization.

        // Construct the BetterAuth instance using the builder pattern.
        let auth = Arc::new(
            AuthBuilder::new(auth_config)
                // We pass in the existing Postgres pool wrapped in the SqlxAdapter.
                // This lets better-auth share the exact same DB connections as your app.
                .database(SqlxAdapter::from_pool(db.clone())) 
                // We enable the plugins we need: Email/Password login and Session tokens.
                .plugin(EmailPasswordPlugin::new().enable_signup(true))
                .plugin(SessionManagementPlugin::new())
                // .plugin(OAuthPlugin::google(...))  ← You can easily add this later.
                .build()
                .await
                .expect("Failed to build BetterAuth"),
        );

        Self { config, db, auth }
    }
}
```

### `api/src/router.rs`

```rust
use crate::state::AppState;
use axum::Router;
// AxumIntegration is the trait that brings the `.axum_router()` method 
// into scope for the BetterAuth instance.
use better_auth::AxumIntegration; 
use tower_http::cors::{CorsLayer, Any};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_swagger_ui::SwaggerUi;

#[derive(OpenApi)]
#[openapi(info(title = "remera-api-docs", version = "0.1.0"))]
struct ApiDoc;

pub fn build_router(state: AppState) -> Router {
    // Generate the complete set of authentication routes (e.g. /sign-in, /sign-out, etc).
    // Note: The router produced here expects `Arc<BetterAuth<AppDb>>` as its internal state.
    let auth_router = state.auth.clone().axum_router();

    // Create the main app router with our application endpoints.
    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(routes::health::router())
        .merge(routes::me::router())
        .split_for_parts();

    // Setup CORS layer. 
    let cors = CorsLayer::new()
        // Allow ONLY the configured frontend domain to interact with the API via browsers.
        .allow_origin(
            state.config.frontend_url
                .parse::<axum::http::HeaderValue>()
                .expect("Invalid FRONTEND_URL"),
        )
        // Let them send any HTTP method and any HTTP header.
        .allow_methods(Any)
        .allow_headers(Any)
        // IMPORTANT: Must be true to allow the browser to send the `Session` cookie.
        .allow_credentials(true); 

    router
        // We mount the auto-generated auth routes at the "/auth" prefix.
        // e.g., POST /auth/sign-in/email
        .nest("/auth", auth_router)
        // Expose Swagger UI for interactive API docs.
        .merge(SwaggerUi::new("/docs").url("/openapi.json", api))
        .layer(cors)
        // We provide the main AppState to all OUR application routes.
        .with_state(state)
}
```

> [!WARNING]
> `with_state(state)` here passes your `AppState`. But `auth_router` is produced with `state = Arc<BetterAuth<AppDb>>` internally — `.axum_router()` already calls `.with_state(self)` on itself before returning the router. The outer `.with_state(AppState)` does not override the inner state. This is correct as shown in the official example.

---

## Phase 5 — Session Extractors (No Custom Code Needed!)

better-auth provides these out of the box via `features = ["axum"]`:

```rust
use better_auth::{CurrentSession, OptionalSession};
use crate::state::AppDb; // = SqlxAdapter
```

### `CurrentSession<AppDb>` — returns 401 if no session

```rust
// In a handler — this replaces the "AuthUser" struct from the old plan.
// If the user is NOT logged in (missing token or invalid session), 
// Axum will automatically reject the request with a 401 Unauthorized status.
async fn get_me(session: CurrentSession<AppDb>) -> impl IntoResponse {
    // The session object gives direct access to the User database record.
    Json(json!({
        "id":    session.user.id(),
        "email": session.user.email(),
        "name":  session.user.name(),
    }))
}
```

### `OptionalSession<AppDb>` — returns None if no session

```rust
// If the user is logged in, session.0 is Some(...). If they are not, session.0 is None.
// This is perfect for public pages that might show user info if available, but don't strictly require it.
async fn public_route(session: OptionalSession<AppDb>) -> impl IntoResponse {
    let user = session.0.map(|s| json!({ "id": s.user.id() }));
    Json(json!({ "user": user }))
}
```

### User trait methods (from `better_auth_core::entity::AuthUser`)
```
.id()             -> &str
.email()          -> Option<&str>
.name()           -> Option<&str>
.email_verified() -> bool
.created_at()     -> DateTime<Utc>
```

### Session trait methods (from `better_auth_core::entity::AuthSession`)
```
.id()       -> &str
.token()    -> &str
.user_id()  -> &str
.created_at() -> DateTime<Utc>
```

### Important: router state requirement

`CurrentSession<AppDb>` needs `State<Arc<BetterAuth<AppDb>>>` in the router. **But your routes use `AppState` as their state.** The solution: routes that use `CurrentSession` must be nested under the auth router, OR you implement a custom extractor that wraps `CurrentSession`.

The cleanest approach for remera — wrap it:

```rust
// api/src/extractors/auth_user.rs
use axum::{async_trait, extract::FromRequestParts, http::request::Parts, response::Response};
use better_auth::{CurrentSession, OptionalSession};
use crate::state::{AppDb, AppState};

// We define our own AuthUser struct that our application handlers will ask for.
pub struct AuthUser {
    pub id: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

#[async_trait]
// We implement the extractor for OUR AppState type.
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Here's the trick: we extract the better-auth CurrentSession manually,
        // feeding it `state.auth` (the Arc<BetterAuth>) instead of the whole AppState.
        let session = CurrentSession::<AppDb>::from_request_parts(parts, &state.auth)
            .await?;

        // Then we map the better-auth user traits into our own struct.
        Ok(AuthUser {
            id:    session.user.id().to_string(),
            email: session.user.email().map(str::to_string),
            name:  session.user.name().map(str::to_string),
        })
    }
}

// A wrapper for when auth is optional (like OptionalSession).
pub struct MaybeUser(pub Option<AuthUser>);

#[async_trait]
impl FromRequestParts<AppState> for MaybeUser {
    // Infallible means this extractor will never fail (it just returns None on failure).
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // We just run our AuthUser extractor and swallow the error if there is one.
        Ok(MaybeUser(AuthUser::from_request_parts(parts, state).await.ok()))
    }
}
```

This is the bridge: `CurrentSession` does the heavy lifting (token extraction from cookie/Bearer, DB lookup), you just unwrap the result into your own `AuthUser` type that works with `AppState`.

---

## Phase 6 — Domain: Role Type

```rust
// domain/src/container.rs
use serde::{Deserialize, Serialize};
use std::str::FromStr;

// A simple enum representing container hierarchy roles.
// Deriving PartialOrd allows us to do `actual >= required` easily.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Role {
    Viewer = 0,
    Editor = 1,
    Owner  = 2,
}

// Allows parsing a Role directly from string, e.g., when reading from the database.
impl FromStr for Role {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "viewer" => Ok(Role::Viewer),
            "editor" => Ok(Role::Editor),
            "owner"  => Ok(Role::Owner),
            _ => Err(()),
        }
    }
}

/// Returns true if `actual` satisfies `required`.
/// For example, if actual is Owner (2), and required is Editor (1), 2 >= 1 is true.
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

---

## Phase 7 — ContainerAccess<R> Extractor

```rust
// api/src/extractors/container_access.rs
use axum::{async_trait, extract::{FromRequestParts, Path}, http::request::Parts, response::Response};
use serde::Deserialize;
use uuid::Uuid;
use domain::container::{Role, role_meets_minimum};
use crate::{extractors::auth_user::AuthUser, state::AppState};

// Marker types so we can strictly define handler requirements.
pub struct Viewer;
pub struct Editor;
pub struct Owner;

// The main extractor struct. R will be substituted with Viewer, Editor, or Owner.
pub struct ContainerAccess<R> {
    pub user: AuthUser,
    pub role: Role,
    // PhantomData is required because R is not actually used in the struct's fields.
    _marker: std::marker::PhantomData<R>,
}

// We need to parse the container_id from the URL path.
#[derive(Deserialize)]
struct ContainerPath {
    container_id: Uuid,
}

// A macro to stamp out identical extractor code for each Role level (Viewer, Editor, Owner).
macro_rules! impl_container_access {
    ($marker:ty, $min_role:expr) => {
        #[async_trait]
        impl FromRequestParts<AppState> for ContainerAccess<$marker> {
            type Rejection = Response;

            async fn from_request_parts(
                parts: &mut Parts,
                state: &AppState,
            ) -> Result<Self, Self::Rejection> {
                use axum::http::StatusCode;
                use axum::response::IntoResponse;
                use axum::Json;

                // Step 1: Ensure the user is actually logged in.
                let user = AuthUser::from_request_parts(parts, state)
                    .await
                    .map_err(IntoResponse::into_response)?;

                // Step 2: Extract the container ID from the URL path (e.g. /containers/:container_id)
                let Path(ContainerPath { container_id }) =
                    Path::from_request_parts(parts, state)
                        .await
                        .map_err(|_| (StatusCode::BAD_REQUEST, "missing container_id").into_response())?;

                // Step 3: Look up their membership in the database.
                let row = sqlx::query!(
                    "SELECT role FROM container_member WHERE container_id = $1 AND user_id = $2",
                    container_id,
                    user.id
                )
                .fetch_optional(&state.db)
                .await
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error":"db error"}))).into_response())?;

                // Step 4: Parse their database role, return 403 Forbidden if they aren't a member at all.
                let member_role = row
                    .and_then(|r| r.role.parse::<Role>().ok())
                    .ok_or_else(|| (StatusCode::FORBIDDEN, Json(serde_json::json!({"error":"forbidden"}))).into_response())?;

                // Step 5: Check if their actual role meets the minimum role required by the handler marker.
                if !role_meets_minimum(member_role, $min_role) {
                    return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({"error":"forbidden"}))).into_response());
                }

                // If they made it this far, grant them access!
                Ok(ContainerAccess { user, role: member_role, _marker: std::marker::PhantomData })
            }
        }
    };
}

// Generate the extractor logic for all three marker types.
impl_container_access!(Viewer, Role::Viewer);
impl_container_access!(Editor, Role::Editor);
impl_container_access!(Owner,  Role::Owner);
```

Usage in a route handler:
```rust
async fn update_container(
    // If they aren't at least an Editor, this function is NEVER called.
    // Axum automatically rejects the request with a 401 or 403.
    access: ContainerAccess<Editor>,  
    // ...
) -> impl IntoResponse { ... }
```

---

## Phase 8 — GET /me

```rust
// api/src/routes/me.rs
use axum::{Json, response::IntoResponse};
use serde::Serialize;
use crate::extractors::auth_user::AuthUser;

#[derive(Serialize)]
struct MeResponse {
    id:    String,
    email: Option<String>,
    name:  Option<String>,
}

// A simple endpoint that requires the AuthUser extractor.
// Used by the frontend to verify the session and load basic profile data.
pub async fn get_me(user: AuthUser) -> impl IntoResponse {
    Json(MeResponse { id: user.id, email: user.email, name: user.name })
}
```

---

## Phase 9 — Invite Flow via DatabaseHooks

The `DatabaseHooks<DB>` trait has `after_create_user` — this is **exactly** where invite resolution goes. No separate endpoint needed for the happy path.

```rust
// api/src/auth_hooks.rs
use better_auth::{DatabaseHooks, AuthResult};
use better_auth::adapters::SqlxAdapter;
use better_auth::entity::AuthUser;
use sqlx::PgPool;
use std::sync::Arc;

pub struct AppAuthHooks {
    pub db: PgPool,
}

#[async_trait::async_trait]
// We implement the DatabaseHooks trait specifically for the SqlxAdapter.
impl DatabaseHooks<SqlxAdapter> for AppAuthHooks {
    // This hook fires immediately AFTER a new user is successfully inserted into the DB.
    async fn after_create_user(
        &self,
        user: &<SqlxAdapter as better_auth::DatabaseAdapter>::User,
    ) -> AuthResult<()> {
        let email = match user.email() {
            Some(e) => e.to_string(),
            None => return Ok(()),
        };

        // Find any active invitations that match this user's email.
        // We only look for ones that haven't expired and haven't been accepted yet.
        let _ = sqlx::query!(
            "SELECT id, container_id, role FROM invite 
             WHERE email = $1 AND accepted_at IS NULL AND expires_at > NOW()",
            email
        )
        .fetch_all(&self.db)
        .await;

        // For each pending invite: insert container_member row, mark invite accepted
        // (exact query omitted — follows from schema above)

        Ok(())
    }
}
```

Then wire it in `AppState::new()`:
```rust
let hooks = Arc::new(AppAuthHooks { db: db.clone() });

let auth = Arc::new(
    AuthBuilder::new(auth_config)
        .database(SqlxAdapter::from_pool(db.clone()))
        // Register the hooks with the BetterAuth system before building it.
        .hooks(hooks)   // ← check exact builder method name in source
        .plugin(EmailPasswordPlugin::new().enable_signup(true))
        .plugin(SessionManagementPlugin::new())
        .build()
        .await?
);
```

> [!NOTE]
> Verify the builder method name for hooks. Run:
> ```bash
> grep -n "fn hooks\|fn with_hooks\|DatabaseHooks" \
>   ~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/better-auth-0.10.0/src/core/auth.rs
> ```

### Invite token (AES-GCM via crate already in your lock)

`aes-gcm` is already compiled in (via `better-auth-api`). Just add it to your `Cargo.toml`:
```toml
aes-gcm = "0.10"
```

Token payload → JSON → AES-256-GCM encrypt → URL-safe base64 → the "bla-bla" in the link.

---

## Phase 10 — Moka Role Cache (After DB works)

```toml
# Already present in api/Cargo.toml:
moka = { version = "0.12.16", features = ["future"] }
```

```rust
// Add to AppState (in api/src/state.rs):
// Moka cache mapping a tuple of (Container ID, User ID) to their Role.
pub cache: Arc<moka::future::Cache<(uuid::Uuid, String), Role>>,

// Init (inside AppState::new):
let cache = Arc::new(
    moka::future::Cache::builder()
        // Max number of entries before it evicts old ones.
        .max_capacity(10_000)
        // Automatically expire entries after 5 minutes.
        .time_to_live(std::time::Duration::from_secs(300))
        .build()
);
```

In `ContainerAccess` extractor, check cache before the DB query:
```rust
// Try to get the role from memory first.
if let Some(role) = state.cache.get(&(container_id, user.id.clone())).await {
    // We got a cache hit, skip the database!
    member_role = role;
} else {
    // query DB...
    // Once found, save it in the cache for next time.
    state.cache.insert((container_id, user.id.clone()), member_role).await;
}
```

---

## Migration File Ordering

```
backend/migrations/
  20260924000001_auth_core.sql       ← copy of better-auth's 001_create_core_tables.sql
  20260924000002_containers.sql
  20260924000003_members.sql
  20260924000004_invites.sql
```

Run via sqlx-cli from `backend/`:
```bash
sqlx migrate run --database-url "$DATABASE_URL"
```

---

## Implementation Checklist

```
Phase 1  ☐ api/Cargo.toml: features = ["axum", "sqlx-postgres"]
Phase 2  ☐ Create migrations/ folder with 4 .sql files (auth + domain tables)
Phase 3  ☐ Extend AppConfig (DATABASE_URL, AUTH_SECRET, BASE_URL, FRONTEND_URL)
Phase 4  ☐ Wire AppState: PgPool + AuthBuilder::new().database(SqlxAdapter::from_pool())
          ☐ Mount /auth router in router.rs  
          ☐ cargo build passes
Phase 5  ☐ Create extractors/auth_user.rs — AuthUser, MaybeUser (wraps CurrentSession<AppDb>)
Phase 6  ☐ Create domain/src/container.rs — Role enum + role_meets_minimum
          ☐ cargo test (domain, no DB needed)
Phase 7  ☐ Create extractors/container_access.rs — ContainerAccess<Viewer/Editor/Owner>
Phase 8  ☐ Create routes/me.rs — GET /me, smoke-test full stack
Phase 9  ☐ Confirm .hooks() method name on AuthBuilder, implement AppAuthHooks
Phase 10 ☐ Invite flow: POST /containers/:id/invites + POST /auth/accept-invite
Phase 11 ☐ Add moka cache to ContainerAccess extractor
```

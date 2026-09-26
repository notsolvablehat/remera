# AGENTS.md — remera backend

Read this before touching the codebase. It says what this service is,
how the crates fit together, what's actually built vs. only decided,
and the conventions to keep so future changes don't fight past ones.

## What this is

A Rust backend for a private, invite-based app where a friend group
shares college photos/videos in a "container" (one container = one
group's archive). Full design context lives in the repo's `docs/`
folder — this file is the fast-orientation version. In particular,
`docs/architecture/auth_layer_guide.md` is the file-by-file
implementation guide for the auth layer described in "Current state"
below; it's kept in sync with real `better-auth` crate behavior
(including version-specific gotchas) as that work progresses.

This file covers `backend/` only. For the frontend (React/Vite client,
API client generation, UI conventions), see `frontend/AGENTS.md`.

## Workspace layout (expected)

```
backend/
├── Cargo.toml                  # workspace root
├── Cargo.lock
├── Dockerfile
├── docker-compose.yml          # local Postgres + app, for dev only
├── .env.example
├── .github/
│   └── workflows/
│       ├── ci.yml              # fmt, clippy, test
│       └── deploy.yml          # build, push image, deploy
├── migrations/                 # sqlx migrations, numbered (not timestamped —
│   ├── 001_create_core_tables.sql        #   see "Current state" below)
│   ├── 002_create_organization_tables.sql
│   ├── 003_create_two_factor_auth_tables.sql
│   ├── 004_create_container_tables.sql
│   └── ...
├── crates/
│   ├── api/                    # the axum binary — thin, wires everything together
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── config.rs       # env var loading, typed AppConfig
│   │       ├── state.rs        # AppState { db, r2_client, cache, auth }
│   │       ├── router.rs       # top-level Router, layer stack
│   │       ├── middleware/
│   │       │   ├── mod.rs
│   │       │   ├── rate_limit.rs
│   │       │   └── request_id.rs
│   │       ├── extractors/
│   │       │   ├── mod.rs
│   │       │   ├── auth_user.rs      # AuthUser, MaybeUser
│   │       │   └── container_access.rs # ContainerAccess<Role>
│   │       └── routes/
│   │           ├── mod.rs
│   │           ├── health.rs
│   │           ├── auth.rs      # docs-only utoipa wrapper for better-auth's /auth/*
│   │           ├── me.rs
│   │           ├── containers.rs
│   │           ├── invites.rs
│   │           ├── members.rs
│   │           └── media.rs
│   │
│   ├── domain/                 # pure business logic, no axum/sqlx types leak in
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── container.rs    # Container, Role, quota rules
│   │       ├── invite.rs
│   │       ├── media.rs
│   │       └── errors.rs       # domain-level errors, mapped to HTTP in api/
│   │
│   ├── storage/                 # sqlx repository layer
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── db.rs            # pool setup
│   │       ├── containers_repo.rs
│   │       ├── members_repo.rs
│   │       ├── invites_repo.rs
│   │       └── media_repo.rs
│   │
│   └── r2/                      # object storage client wrapper
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── presign.rs       # PUT/GET URL generation
│           └── keys.rs          # object key naming scheme
│
└── tests/
    ├── common/
    │   └── mod.rs                # spins up test DB, test client
    ├── containers_test.rs
    ├── media_test.rs
    └── invites_test.rs
```

## Hard rules — don't violate these

1. **`domain` has no `main.rs` and no `[[bin]]`.** It's a library only.
   If `cargo run` ever again reports "available binaries: api, domain",
   something reintroduced a binary target in `domain` — check for a
   stray `main.rs` or a `fn main()` left in `lib.rs`.
2. **Only one `Cargo.lock` exists, at `backend/Cargo.lock`.** Never run
   `cargo build`/`cargo run` from inside a `crates/*` subfolder — always
   from `backend/`. A `Cargo.lock` appearing inside `crates/api/` or
   `crates/domain/` means that rule got broken; delete it.
3. **`domain` must not depend on `axum`, `sqlx`, `tokio` (networking),
   `aws-sdk-s3`, or any other infra crate.** The entire reason for the
   crate split is that domain logic — quota rules, role rules, the
   "last owner can't leave" rule — gets to be unit-tested with plain
   `cargo test`, no database, no HTTP server. If a domain type needs to
   serialize, `serde` is fine; if it needs to be fallible, `thiserror`
   is fine. Nothing beyond that.
4. **`api/src/config.rs` is the only place that reads environment
   variables.** Every other file gets config through `AppState`, not
   `std::env::var` directly. This keeps every setting discoverable in
   one file and testable without real env vars.
5. **Handlers stay thin.** A route handler in `api/src/routes/` should
   read: extract → call into `domain` (and later `storage`) → map the
   result to an HTTP response. Business logic (quota math, role
   comparisons) belongs in `domain`, not inline in a handler closure.

## Current state (update this section as you go)

**Built:**
- Workspace skeleton (`api` binary, `domain` library)
- `GET /healthz` and `GET /` — liveness/root, no DB dependency, tagged
  `Meta` in the OpenAPI spec
- `tracing` initialized with `EnvFilter`, dev (stdout) vs. prod
  (`app.log` file) split based on `APP_ENV`
- OpenAPI spec generation via `utoipa` + `utoipa-axum`'s `OpenApiRouter`
  (`api/src/router.rs`, `ApiDoc`) — routes and spec are registered
  together via `routes!(...)`, no separate hand-maintained path list.
  **Note:** `utoipa-axum` is pinned to `0.2` (not the latest `0.3`)
  because `utoipa-swagger-ui` hasn't caught up to `utoipa 6.0` yet —
  `0.3` pulls in `utoipa 6.0` and produces duplicate-crate-version
  compile errors. Don't bump without checking `utoipa-swagger-ui`
  compatibility first.
- Swagger UI served at `/docs`, raw spec at `/openapi.json`
- A pre-commit hook (`.husky/pre-commit`, repo root) regenerates the
  frontend's API client whenever `backend/crates/api` changes — see
  `frontend/AGENTS.md` for what it generates and where the output
  lands; this file only needs to know the hook exists and boots this
  service temporarily to read `/openapi.json` from it
- Migrations: `backend/migrations/001-004` already exist and cover both
  better-auth's own tables (`users`, `sessions`, `accounts`,
  `verifications`, `organization`, `member`, `invitation`,
  `two_factor`) and the domain tables (`container`, `container_member`,
  `invite`). They use plain numbered filenames (`001_...`, `002_...`),
  not sqlx's timestamp convention — keep any new migration in that same
  numbered style; sqlx only needs the prefix to sort and be stable, but
  mixing schemes on already-applied files would break the per-migration
  checksum sqlx stores.

**Built — auth layer, working end-to-end** (`better-auth` crate
integration; see `docs/architecture/auth_layer_guide.md` for the
file-by-file build order and rationale behind each piece):
- `AppConfig` carries `database_url`, `auth_secret`, `base_url`,
  `frontend_allow_origins: Vec<String>` (comma-separated
  `FRONTEND_ALLOW_ORIGINS` env var — a list because one backend can
  serve multiple frontends, not a single hardcoded origin).
- `AppState` opens a `PgPool`, builds `Arc<BetterAuth<SqlxAdapter>>` via
  `AuthBuilder` with `EmailPasswordPlugin` + `SessionManagementPlugin`,
  and holds a `moka::future::Cache<(container_id, user_id), Role>` for
  `ContainerAccess` to check before hitting Postgres.
- `router.rs` mounts better-auth's real routes at `/auth` (session
  state resolved via a second `.with_state(...)` call before
  `.nest()` — see the code comment for why that's required, not
  optional), CORS built from `frontend_allow_origins`.
- `extractors/auth_user.rs` — `AuthUser` bridges better-auth's
  `CurrentSession<AppDb>` (which needs `State<Arc<BetterAuth<AppDb>>>`)
  into something usable as a normal `AppState`-based extractor.
  Carries `id, name, email, email_verified, image, username, banned,
  ban_reason, ban_expires` — deliberately not the full 15-column
  `users` table shape (dropped `display_username`, better-auth's own
  `role` field, `two_factor_enabled` — none of those are used).
  `username` is update-only (via better-auth's built-in
  `POST /auth/update-user`), never asked at signup. `MaybeUser` exists
  for future optional-auth routes, not used yet.
- `extractors/container_access.rs` — `ContainerAccess<Viewer/Editor/Owner>`
  is fully implemented (role lookup + moka cache + `role_meets_minimum`
  check), but **no route uses it yet** — there are no container routes
  to protect yet. Wire it in when `routes/containers.rs` etc. get built.
- `domain/container.rs` — `Role` enum + `role_meets_minimum`, unit
  tested (`cargo test -p domain`).
- `routes/me.rs` — `GET /me`, the one route that actually uses
  `AuthUser` today. Verified live: sign-up → sign-in → `/me` with the
  returned token all work against a real local Postgres.
- `routes/auth.rs` — **documentation-only** utoipa wrapper for
  better-auth's real `/auth/sign-up/email`, `/auth/sign-in/email`,
  `/auth/sign-out`, `/auth/get-session`. These functions are never
  mounted as routes (that'd double-register the same path and panic at
  startup) — they exist only so `ApiDoc`'s
  `#[openapi(paths(...), components(schemas(...)))]` in `router.rs`
  picks them up, so Swagger/orval know these endpoints exist. The DTOs
  in this file (`AuthUserDto`, etc.) mirror better-auth's *real* wire
  format field-for-field — don't trim them to match `/me`'s shape,
  they document a different, fixed-by-the-crate response.
  If you wrap more better-auth endpoints later, follow this exact
  pattern (stub fn + `#[utoipa::path]` + list in `ApiDoc`, never a real
  route).
- `auth_hooks.rs` — `AppAuthHooks` + a `DatabaseHooks<SqlxAdapter>` impl
  for invite resolution on signup, written but **not wired to
  anything** — see trap #1 below for why, and "Next up" for the actual
  plan.

**Two version-specific traps in `better-auth 0.10.0` worth knowing
before touching this again:**
1. `HookedDatabaseAdapter<DB>` (the documented way to attach
   `DatabaseHooks`) does not compile in this version — it implements
   every `*Ops` trait except `PasskeyOps`, so it can never satisfy
   `DatabaseAdapter`'s full bound. `AuthBuilder::hook(...)` also
   hard-errors at `.build()` if used directly. Confirmed against the
   crate's own vendored source; upstream's `master` branch has since
   removed `HookedDatabaseAdapter` from `hooks.rs` entirely (there's a
   `1.0.0-alpha.2` published after 0.10.0), so this was a known gap
   that got reworked, not something to retry differently.
   `AuthConfig::disabled_path(...)` doesn't help either — it's checked
   inside `handle_request_inner` too, so disabling a path blocks it
   everywhere, not just axum's route registration; there's no way to
   selectively "unmount but still dispatch" one better-auth route in
   this version. Until the crate is upgraded, `AppDb = SqlxAdapter`
   (unwrapped), and any signup-time logic has to be called explicitly
   by us instead of via a `DatabaseHooks` callback.
2. Custom `impl FromRequestParts<AppState>` extractors must **not**
   import `axum::async_trait` or use `#[async_trait]` — that re-export
   doesn't exist in `axum 0.8.9`; `FromRequestParts` is a native
   `async fn`-returning trait now. (Unrelated: better-auth-core's own
   `DatabaseHooks` trait *does* need the separate `async-trait` crate —
   add it to `api/Cargo.toml` if you implement `DatabaseHooks`.)
3. `BetterAuth<DB>::handle_request(&self, req: AuthRequest) -> AuthResult<AuthResponse>`
   is a public, framework-agnostic dispatch entry point — it runs a
   plugin's real logic (password hashing, session creation, etc.)
   without going through the axum router at all. This is the tool for
   "call better-auth's real signup logic from our own custom endpoint"
   (needed for invite signup — see "Next up"), confirmed to exist and
   work; not yet used anywhere in this codebase.

**Next up — invite-only signup** (design confirmed, not yet
implemented):
- Two signup shapes: **direct** (`email, password` — already works,
  unchanged) and **invite** (`invite_token, email, password, name`).
  The invite token is the only thing that determines `container_id` +
  `role` — deliberately **not** accepted as plain client-supplied
  fields, since that would let any client self-assign to any container
  with any role. The server decrypts the AES-GCM token (see "Design
  decisions" below) to get the real values.
- Plan: a new endpoint (not better-auth's own `/auth/sign-up/email`)
  that decrypts the token, checks the request's `email` matches the
  token's invited email, calls `state.auth.handle_request(...)` (trap
  #3 above) to actually create the account via better-auth's real
  logic, then in the same transaction inserts `container_member` and
  marks the `invite` row `accepted_at`.
- Separately: an already-registered user who gets invited later should
  have it "resolve on next login" per the design doc — planned as an
  opportunistic check inside `extractors/auth_user.rs` (every
  authenticated request already has the verified email in hand), not
  as a hook into better-auth's sign-in route.
- The logic already written in `auth_hooks.rs`'s `after_create_user`
  is the right shape for "find pending invites for this email, resolve
  them" — reuse it as a plain function called from both the new signup
  endpoint and the login-time check, rather than duplicating it.

**Built — containers + edit allow-list ("invite"):**
- `storage` crate exists now (`containers_repo.rs`, `members_repo.rs`,
  `allowlist_repo.rs`) — sqlx lives here, not inline in handlers (except
  `container_access.rs`'s pre-existing inline query, left as-is).
- `domain::Container` struct + `MAX_OWNED_CONTAINERS = 5` added next to
  `Role` in `container.rs`.
- `routes/containers.rs`: `POST /containers` (owner-quota-checked,
  transactional owner-membership insert, seeds the moka cache),
  `GET /containers`, `GET /containers/{container_id}` — the last one is
  the first real usage of `ContainerAccess<Viewer>`.
- Migration `005_create_container_edit_allowlist.sql` — plain
  email-based allow-list table, distinct from the existing `invite`
  table (token/accepted_at/expires_at), which stays unused for now.
- `routes/invites.rs`: `POST/GET/DELETE .../edit-allowlist[/{email}]`,
  all `ContainerAccess<Owner>`-gated. Delete also revokes the granted
  `editor` membership row and invalidates the moka cache entry.
- Passive resolution wired into `extractors/auth_user.rs`: every
  authenticated request calls
  `storage::allowlist_repo::resolve_pending_edit_invites` for the
  session's verified email — covers both "signs up" and "logs in" in
  one place, since better-auth's `DatabaseHooks` can't be used (trap #1
  still applies). `auth_hooks.rs`'s `AppAuthHooks` is now fully
  superseded by this — still present but dead; ask before deleting it.
- Verified live end-to-end: signup → create container → add friend's
  email to allow-list → friend signs up → friend's `GET /containers`
  shows `editor` automatically → delete allow-list entry revokes it →
  6th owned container 409s.
- Frontend client regenerated (`bun run gen:api`) — `Containers` and
  `Invites` tags now have their own folders under `src/lib/api/`.

**Not yet built** (all decided in design discussion, none implemented):
- `r2` crate (presigned URLs) — folder doesn't exist yet;
  `Cargo.toml`'s `members = ["crates/*"]` will pick it up automatically
  once added, no workspace file change needed
- Database connection / `GET /readyz`
- The View share-link mechanism (AES-GCM token, `GET/POST
  .../share-link`, `GET /invites/{token}`) — deliberately out of scope
  for the allow-list work above; the existing `invite` DB table is
  reserved for this, not the allow-list.
- `PATCH/DELETE /containers/{cid}`, lock/usage endpoints, members
  list/transfer-ownership, media routes — see the route list doc.
## Design decisions already made (don't re-litigate these)

- **Single owner per container, with transfer** — not multiple owners.
- **View access** is via 2 things. If the owner's container is private and adding new members with view access requires them to be logged in. Else, if the container is public, no auth required.
- **Edit access** is an allow-list model: an owner adds a Gmail address
  to a container's allow-list; the grant resolves passively, whenever
  that email next signs up *or logs in* (not just signup — check both
  paths when this gets built). Invitation link will have some details regarding the owner and the person invited. Like, the owner's container id, the requested person's gmail, and their accessibility settiongs, like view only, edit etc. Once the user signs up, the frontend will send these details along with the other creds. Then the backend will resolve these extra headers to decide if the user is legit and is accessing only what is provided. 
- **Owner-only lock** overrides every other role's access, including
  Edit, until the owner unlocks it.
- Free-tier constraints shape several choices: R2 for storage (10 GB
  free, zero egress), per-container storage/media caps to avoid
  exhausting that budget, presigned URLs so the server never proxies
  file bytes.
- Crate split follows a hexagonal shape: `domain` (pure logic) ←
  `storage`/`r2` (infra adapters) ← `api` (HTTP wiring). Dependencies
  only point inward.
- Sharing link: This link will be a pre-signed crypto token with all necessary details. Suppose, the container is `x`, owner is `y`, invited person's gmail is `test@gmail.com` and the auth access to them is `view-only`. This data, as json, will be encrypted into a string, maybe using AES encryption. Then the link will be built like: `https://<domain>/invite/<bla-bla>. Frontend will take a note of this "bla-bla" and will redirect the user to signup page. After the user enters their creds, along with this "bla-bla" the creds will be passed to backend. This will be decrypted by backend and will map and match the creds + invite details and either allow or reject the user signup. 
- From above, there will be two ways of signup. One only a normal signup, the other, invite only signup.
## Commands

Run everything from `backend/`, never from inside a `crates/*` folder:

```bash
cargo run                # runs the api binary
cargo build               # builds the whole workspace
cargo test                 # runs tests across all member crates
cargo fmt && cargo clippy -D warnings   # before committing
```

Once the DB layer exists: bring up Postgres (`docker-compose up -d
postgres` from `infra/`), copy `.env.example` to `.env` and fill in
`DATABASE_URL`, then `sqlx migrate run` before `cargo run`.

## When you add a new route

1. Does it need auth? Use the `AuthUser` or `MaybeUser` extractor
   (`api/src/extractors/auth_user.rs`) — don't hand-roll header parsing
   in the handler.
2. Does it touch a container? Use `ContainerAccess<Viewer/Editor/Owner>`
   (`api/src/extractors/container_access.rs`) — don't write a bespoke
   role check. This will be the first route that actually uses it.
3. Business rule (quota, role comparison, lock override)? It goes in
   `domain`, called from the handler — not written inline in
   `api/src/routes/`.
4. Update the "Current state" section above so the next agent (or
   session) doesn't have to re-derive what exists by reading every file.

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
- Migrations: `backend/migrations/001-005` already exist and cover both
  better-auth's own tables (`users`, `sessions`, `accounts`,
  `verifications`, `organization`, `member`, `invitation`,
  `two_factor`) and the domain tables (`container`, `container_member`,
  `invite`, `container_edit_allowlist`). They use plain numbered
  filenames (`001_...`, `002_...`), not sqlx's timestamp convention —
  keep any new migration in that same numbered style; sqlx only needs
  the prefix to sort and be stable, but mixing schemes on
  already-applied files would break the per-migration checksum sqlx
  stores. There's no `_sqlx_migrations` tracking table in the dev DB —
  migrations so far have been applied by hand with `psql -f`, not
  `sqlx migrate run`; keep doing that until `sqlx-cli` is actually
  installed, and don't assume the tracking table exists.

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
  anything** — see trap #1 below for why. Now superseded by the
  allow-list resolution in `extractors/auth_user.rs` (see "Built —
  containers + edit allow-list" below) — dead code, not a pending TODO.

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
   "call better-auth's real signup logic from our own custom endpoint" —
   confirmed to exist and work, but not currently needed: the original
   plan to use it for a dedicated invite-signup endpoint was superseded
   by the allow-list model (see "Superseded" below), which resolves
   grants passively through the *existing* signup/login routes instead.
   Still not used anywhere in this codebase; keep in mind if a future
   flow needs to drive better-auth logic from a non-`/auth/*` route.

**Superseded — invite-only signup via AES-GCM token** (this was the
original plan for Edit access before the allow-list model below was
built; kept here only so nobody re-reads it as current):
- The idea was a token (`invite_token, email, password, name`) that
  the server decrypts to get `container_id` + `role`, using
  `state.auth.handle_request(...)` (trap #3 above) to create the
  account via better-auth's real logic, then inserting
  `container_member` in the same transaction.
- This is **not** what got built for Edit access — see "Built —
  containers + edit allow-list" below, which resolves Edit grants
  passively instead (no token, no dedicated signup endpoint). The
  AES-GCM token idea is still the plan for the separate View
  **share-link** mechanism (see "Not yet built"), which hasn't been
  implemented — don't confuse the two when picking this back up.

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

**Built — media (upload/get/patch/delete), presigned via R2:**
- `r2` crate exists now (`client.rs`, `keys.rs`, `presign.rs`) — thin
  wrapper over `aws-sdk-s3` pointed at Cloudflare R2. `build_client`
  reads endpoint/credentials from `AppConfig` (never `std::env::var`
  directly, per hard rule #4); `force_path_style(true)` is set since
  path-style is what reliably works against non-AWS S3-compatible
  endpoints. `presigned_put_url`/`presigned_get_url`/
  `verify_uploaded_object` (HEAD)/`delete_object` are the whole public
  surface — the server calls these to hand out URLs and verify/clean up
  objects, but **never reads or writes object bytes itself**.
- Config: `AppConfig` now also carries `s3_endpoint`, `r2_access_key_id`,
  `r2_secret_access_key`, `r2_bucket_name` — all required (`.expect()`
  panics at startup if missing, same as `DATABASE_URL`/`AUTH_SECRET`).
  `R2_API_KEY` (a Cloudflare account-level API token, not an S3
  credential) and `R2_ACCOUNT_ID` (superseded by reading `S3_ENDPOINT`
  directly) are documented in `.env.example` but deliberately not read
  anywhere.
- Migration `006_add_container_quota_columns.sql` — adds
  `storage_bytes/storage_limit/media_count/media_limit` to `container`
  (defaults: 2 GiB / 2000 items — `domain::DEFAULT_STORAGE_LIMIT_BYTES`
  / `DEFAULT_MEDIA_LIMIT`). Migration `007_create_media_table.sql` —
  the `media` table itself; `id` has no `DEFAULT`, it's always
  app-generated (`Uuid::now_v7()`) since the object key embeds it.
- `domain::Media`/`MediaStatus` added (`media.rs`) — deliberately no
  `created_at` field (chrono isn't a domain-crate dependency, per hard
  rule #3); timestamps live only in `storage::media_repo::MediaRecord`.
- `storage::media_repo` — two-phase upload as one atomic unit:
  `reserve_and_create_pending` does the quota-check `UPDATE ... WHERE
  storage_bytes + $size <= storage_limit ... RETURNING id` (409 via
  `DomainError::QuotaExceeded` if no row) and the `INSERT` in one
  transaction, so concurrent uploads can't both pass a stale
  count-then-act check. `abort_pending`/`delete_ready` both release the
  reserved quota atomically in the same transaction as the row delete.
- `routes/media.rs`: `POST .../uploads` (Edit+, issues the presigned PUT
  + pending row), `POST .../uploads/{mid}/complete` (uploader-only,
  HEADs R2 and rejects on `size_mismatch`/`not_uploaded_yet`, else
  flips to `ready`), `DELETE .../uploads/{mid}` (uploader-only, aborts a
  still-pending upload), `GET .../media` (View+, keyset-paginated by
  UUIDv7, `ready` only), `GET .../media/{mid}`, `GET
  .../media/{mid}/download` (presigned GET, `Content-Disposition:
  attachment`), `PATCH .../media/{mid}` (uploader-or-owner, renames/sets
  caption only — no way to explicitly clear caption back to null, an
  accepted MVP gap), `DELETE .../media/{mid}` (uploader-or-owner).
- **Not** built here (explicitly deferred, still real gaps): multipart
  upload for large files (`.../uploads/{mid}/parts`), bulk-delete,
  media dimensions (width/height — would need parsing the file), and
  `container.is_locked` enforcement (uploads/deletes don't check it —
  same known gap as "Next up" item 1 below).
- Verified live end-to-end against a real R2 bucket (not mocked):
  create container → request upload URL → `PUT` bytes straight to R2
  (server never in the data path) → `complete` HEAD-verifies size →
  `list`/`get`/`download` → downloaded bytes diffed byte-identical
  against the original file → `PATCH` rename+caption → `DELETE` →
  quota (`storage_bytes`/`media_count`) correctly incremented on
  upload-creation and released on both delete and abort; non-member
  gets 403; declaring one size then uploading a different one gets
  `409 size_mismatch` on complete; abandoning a pending upload and
  calling `complete` gets `409 not_uploaded_yet`, and the abort endpoint
  cleans it up (quota released, R2 object deleted).
- Frontend client regenerated — `Media` tag now has its own folder
  under `src/lib/api/`.

**Built — rate limiting:**
- `middleware/rate_limit.rs` (new — `middleware/` didn't exist before)
  builds a `tower_governor` `GovernorLayer` using the default
  `PeerIpKeyExtractor`: one flat global-per-IP limit for every route
  (30-request burst, replenishing 1/second), plus a spawned background
  task calling the limiter's `retain_recent()` every 60s — without it,
  the in-memory per-IP state map grows forever, since it never forgets
  an IP on its own.
- Applied in `router.rs` as the **outermost** layer (added last, so it
  runs first on every inbound request) — before CORS, before auth
  extraction, before it can touch the DB. This matches the layering
  order described in
  `docs/architecture/howisthebackendstructured-1.md`'s "Summary of the
  access-control shape" (`RequestId → rate limit → auth → ...`).
  Wraps the nested `/auth/*` router too, not just this crate's own
  routes — signup/login are exactly the endpoints most worth rate
  limiting.
  `.use_headers()` is on, so responses (including the `429` itself)
  carry `x-ratelimit-limit`/`-remaining` and `retry-after` — required
  the `governor` crate as a direct dependency (not just `tower_governor`
  re-exporting it) to name `StateInformationMiddleware` in
  `layer()`'s return type.
- `main.rs` now serves via
  `app.into_make_service_with_connect_info::<SocketAddr>()` instead of
  plain `into_make_service()` — required for `PeerIpKeyExtractor` to
  have a `SocketAddr` to key on at all; without this change every
  request would hit `GovernorError::UnableToExtractKey`.
- Deliberately one flat limit, not per-route: fine-tuning (e.g. a
  stricter limit specifically on `/auth/sign-up/email` to slow
  credential-stuffing, a looser one for authenticated `GET` traffic)
  is real future work, not done here — see "Next up".
- Verified live: fired 40 rapid requests at `/healthz` from one IP —
  first 30 succeeded, the rest got `429` with the expected headers;
  waited for one token to replenish and a request succeeded again;
  confirmed normal single-request traffic (signup, etc.) is unaffected.

**Built — container locking, PATCH/DELETE, usage:**
- Migration `008_add_container_deleted_at.sql` — adds `deleted_at` for
  soft-delete. `containers_repo`'s `get_by_id`/`list_for_user`/the
  owned-container quota count all now filter `deleted_at IS NULL`, so a
  soft-deleted container disappears from every read path without an
  actual `DELETE` statement anywhere yet (purging R2 objects + hard
  delete is still a background-job concern, not implemented).
- `extractors/container_access.rs` gained a **second** cache,
  `AppState.container_status_cache: Cache<Uuid, ContainerStatus>`
  (`{is_locked, is_deleted}`) — deliberately separate from the existing
  per-`(container, user)` role cache, since locked/deleted are
  properties of the container, not of a membership. `ContainerAccess`
  now checks this on every request, in the order the flow docs specify:
  deleted → `404` (unconditional, even for the owner) before locked →
  `423` (skipped only if the caller's role is `Owner`) before the
  existing "not a member" → `403` / role-too-low → `403` checks. This
  is a **shared extractor**, so lock enforcement is automatically live
  on every route that already used `ContainerAccess<Role>` — including
  the media routes from the previous session, which previously had a
  documented gap here ("uploads/deletes don't check `is_locked`" — that
  gap is now closed as a side effect, not a separate media-specific fix).
- `storage::containers_repo` gained `update_metadata` (`COALESCE`-based
  patch, same pattern as `media_repo::update_metadata`), `set_locked`,
  `soft_delete`, and `get_usage` (storage/media counts + a
  `container_member` count subquery).
- `routes/containers.rs`: `PATCH /containers/{cid}` (name/
  storage_limit_bytes/media_limit, at least one required),
  `PUT /containers/{cid}/lock` (`{ locked: bool }`), `DELETE
  /containers/{cid}` (soft-delete), `GET /containers/{cid}/usage`. Lock
  and delete both explicitly invalidate `container_status_cache` after
  writing, same reasoning as the role-cache invalidation in
  `routes/invites.rs`'s allow-list revoke — otherwise a cached entry
  could serve stale access for up to the cache's TTL.
- Verified live end-to-end: locked a container as owner, confirmed an
  editor got `423` on both `GET /containers/{cid}` *and*
  `POST .../uploads` (proving the shared-extractor claim above), owner
  retained full access throughout including `PATCH` while locked,
  unlock restored editor access immediately (no stale window),
  `GET .../usage` reflected `PATCH`-updated limits and correct
  `member_count`; soft-deleted a container and confirmed every role
  (owner *and* editor) gets a uniform `404` — not `403`/`423` — and a
  repeated `DELETE` is idempotent (`404`, not `500`); confirmed a
  soft-deleted container no longer counts against the 5-owned-container
  quota.
- Frontend client regenerated — `containers.ts` picked up the three new
  endpoints and their models.

**Built — members (list, role-change, leave/remove, transfer-ownership):**
- `domain::DomainError::OwnerTransferRequired` (new variant) — the
  "last owner can't leave" rule now has real code: `storage::
  members_repo::remove_member` looks up `container.owner_id` first and
  refuses (this error) if the target of a remove/leave is the current
  owner, regardless of who's asking.
- `storage::members_repo` gained `list_for_container` (joins `users`
  for name/email — both nullable in the `users` table, so the DTO
  fields are `Option<String>`), `update_role` (Editor/Viewer only —
  the CHECK constraint on `container_member.role` would reject
  `'owner'` anyway, but validation happens in the route handler first),
  `remove_member`, and `transfer_ownership`.
- `transfer_ownership` is a **real swap**, not a flat demotion: the
  target becomes `owner`, and the *previous* owner takes whatever role
  the target held before the swap (all in one transaction, alongside
  updating `container.owner_id` itself, which is what the owned-container
  quota check in `containers_repo::create_with_owner` reads).
- `routes/members.rs` (new — `routes/mod.rs`/`router.rs` updated):
  `GET .../members` (View+), `PATCH .../members/{uid}` (Owner only;
  rejects `role: "owner"` and rejects targeting the current owner's own
  row with `use_transfer_ownership` — that's what the transfer endpoint
  is for), `DELETE .../members/{uid}` (Owner removing anyone, **or**
  any member removing themselves — "leave" and "remove" are the same
  endpoint, gated by `user_id == caller.id || role == Owner`),
  `POST .../transfer-ownership` (Owner only, body
  `{ new_owner_user_id }`, rejects transferring to yourself).
- Every mutation here (`update_role`, `remove_member`,
  `transfer_ownership`) explicitly invalidates the relevant
  `(container_id, user_id)` entries in `AppState.cache` — same
  immediate-effect reasoning as the lock/allow-list-revoke invalidation
  elsewhere, confirmed live (a demoted/removed user loses access on
  their very next request, not after the cache's TTL).
- **Discovered gap, not fixed here**: `allowlist_repo::insert` uses
  `ON CONFLICT (container_id, email) DO NOTHING`, so once an
  allow-list entry is claimed, re-adding the same email after that
  person leaves/is removed is a silent no-op — they can't be
  re-invited via the allow-list without a manual DB fix. Worth a
  follow-up (e.g. resetting `claimed_at` on conflict instead of
  no-op'ing) but out of scope for the members work itself.
- Verified live end-to-end: listed members (owner + allow-listed
  editor); owner demoted editor→viewer, confirmed the demoted user's
  very next request (an upload attempt) got `403` immediately;
  rejected `role: "owner"` and rejected patching the owner's own row;
  owner blocked from leaving (`409 owner_must_transfer_first`) while a
  regular member's self-leave succeeded (`204`) and immediately lost
  access; non-owner blocked from transferring ownership (`403`),
  transferring to a non-member rejected (`404`), transferring to a
  real member swapped roles correctly (verified via `GET .../members`
  showing the flip, the old owner immediately losing lock/patch
  access, the new owner immediately gaining it, and `container.owner_id`
  in Postgres reflecting the change).
- Frontend client regenerated — `Members` tag now has its own folder
  under `src/lib/api/`.

**Next up — in priority order** (see
`docs/architecture/howisthebackendstructured-1.md` for full route
specs and request-flow traces for all of these):

1. **View share-link** ("the other half of invite") — AES-GCM token,
   public `GET /invites/{token}` landing page, `GET/POST
   .../share-link[/rotate]`. Deliberately deferred when this session
   scoped the invite work down to allow-list-only. The existing
   `invite` DB table (token/accepted_at/expires_at) is reserved for
   this, not the allow-list — don't repurpose it.
2. **Media follow-ups** — multipart upload for large files, bulk-delete,
   media dimensions. See "Built — media" above for exactly what's
   missing.
3. **Operational readiness**:
   - `GET /readyz` (DB-ping readiness probe) — doesn't exist; nothing
     currently tells an orchestrator when it's safe to route traffic.
   - Per-route rate-limit tuning — one flat global limit exists now
     (see "Built — rate limiting" above); splitting it into stricter
     limits for specific routes (signup/login especially) is still
     open.
   - `middleware/request_id.rs` (planned in the workspace layout above)
     doesn't exist — no request-id propagation in logs yet.
   - Integration tests (`backend/tests/`) don't exist at all — every
     route built so far has only been verified by hand with curl
     against a live local Postgres. `tests/common/mod.rs` (test DB +
     client bootstrap) needs to exist before `containers_test.rs` /
     `invites_test.rs` / `media_test.rs` are worth writing.
   - `auth_hooks.rs`'s `AppAuthHooks`/`DatabaseHooks` impl is dead code
     now that allow-list resolution lives in `auth_user.rs` instead —
     worth deleting so it doesn't look load-bearing to a future reader,
     but confirm with whoever's driving before removing it.
   - `allowlist_repo::insert`'s `ON CONFLICT DO NOTHING` means a
     claimed allow-list entry silently blocks re-inviting that email
     after they leave/are removed (see "Built — members" above) —
     small fix, not yet done.
4. **Deployment** — `Dockerfile`, `docker-compose.yml`,
   `.github/workflows/ci.yml`/`deploy.yml` are all in the planned
   workspace layout above but don't exist. Right now there's no way to
   build/ship this except `cargo run` against a manually-provisioned
   local Postgres, and `AUTH_SECRET` in `.env` is a throwaway dev value
   with no real secrets story yet. (R2 credentials in `.env` *are* real
   — a live test bucket — unlike `AUTH_SECRET`; still needs a proper
   secrets story before any real deploy, just flagging it's not a
   placeholder like the others.)

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

# AGENTS.md — remera backend

Read this before touching the codebase. It says what this service is,
how the crates fit together, what's actually built vs. only decided,
and the conventions to keep so future changes don't fight past ones.

## What this is

A Rust backend for a private, invite-based app where a friend group
shares college photos/videos in a "container" (one container = one
group's archive). Full design context lives in the repo's `docs/`
folder once that exists — this file is the fast-orientation version.

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
├── migrations/                 # sqlx migrations, timestamped
│   ├── 20260101000000_init.sql
│   ├── 20260105000000_containers.sql
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
- `GET /healthz` — liveness check, no DB dependency
- Basic `AppConfig` (currently just `PORT`) and `AppState` scaffolding
- `tracing` initialized with `EnvFilter`

**Not yet built** (all decided in design discussion, none implemented):
- `storage` crate (sqlx repos) and `r2` crate (presigned URLs) — folders
  don't exist yet; `Cargo.toml`'s `members = ["crates/*"]` will pick
  them up automatically once added, no workspace file change needed
- Database connection / `GET /readyz`
- Auth (better-auth.rs integration, session/JWT verification extractors)
- Containers, invites/allow-list, members, media routes — see the
  route list and request-flow docs (if present in `docs/`) for the
  full planned surface
- `ContainerAccess<Role>` extractor — the shared permission-check layer
  every private route will use
- moka role cache
- Migrations (no `migrations/` folder yet)

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
   (once built) — don't hand-roll header parsing in the handler.
2. Does it touch a container? Use `ContainerAccess<Role>` (once
   built) — don't write a bespoke role check.
3. Business rule (quota, role comparison, lock override)? It goes in
   `domain`, called from the handler — not written inline in
   `api/src/routes/`.
4. Update the "Current state" section above so the next agent (or
   session) doesn't have to re-derive what exists by reading every file.

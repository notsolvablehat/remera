# Remera — Backend Reference

Companion reference for the Rust backend: crate choices, the full route list, and request-flow traces for every access scenario the app needs to handle.

---

## 1. Cargo crates and what they're for

### Web & async runtime
| Crate | Purpose |
|---|---|
| `tokio` | Async runtime. Everything (axum, sqlx, the R2 client) runs on top of it. |
| `axum` | HTTP framework. Type-safe extractors, routing, and Tower middleware compatibility. |
| `tower` | The middleware abstraction axum is built on — lets you compose request-processing layers. |
| `tower-http` | Ready-made Tower layers: request tracing, compression, timeouts, body-size limits, CORS. |
| `tower_governor` | Rate limiting middleware (per-IP or per-key), used before auth so unauthenticated abuse can't reach your DB. |

### Database
| Crate | Purpose |
|---|---|
| `sqlx` (postgres, runtime-tokio-rustls, uuid, chrono features) | Async Postgres driver with compile-time checked queries — a typo in SQL fails at `cargo build`, not in production. |
| `uuid` (v7 feature) | UUIDv7 ids — time-sortable, so you get free keyset pagination without a separate `created_at` index. |

### Auth
| Crate | Purpose |
|---|---|
| `better-auth` (better-auth.rs) | Session/JWT auth with axum integration. Handles sign-up, login, session verification. |
| `jsonwebtoken` | Only needed if you end up verifying JWTs manually outside what better-auth.rs's extractors give you. |

### Object storage (R2)
| Crate | Purpose |
|---|---|
| `aws-sdk-s3` | S3-compatible client, pointed at R2's endpoint. Generates presigned PUT/GET URLs so the client uploads/downloads directly, never through your server. |
| `aws-config` | Credential/region config loader that `aws-sdk-s3` needs. |

### Caching
| Crate | Purpose |
|---|---|
| `moka` (future feature) | In-memory async cache with TTL — used to cache a user's role-in-container for ~30s so every request in a browsing session doesn't re-query the membership table. |

### Errors & validation
| Crate | Purpose |
|---|---|
| `thiserror` | Typed error enums per layer (`DomainError`, `RepoError`) with clean `Display` messages, mapped to HTTP status codes at the edge. |
| `validator` | Declarative request-payload validation (string length, email format, etc.) via derive macros. |

### Observability
| Crate | Purpose |
|---|---|
| `tracing` | Structured logging/spans throughout the request lifecycle. |
| `tracing-subscriber` (env-filter, json features) | Wires up where those spans/logs actually go (stdout, JSON for log aggregators). |

### Docs & serialization
| Crate | Purpose |
|---|---|
| `utoipa` | Generates an OpenAPI spec from your route/handler annotations — this becomes the contract your frontend's TypeScript types are generated from. |
| `serde` (derive feature) | Serialize/deserialize request and response JSON. |
| `serde_json` | JSON value handling where you need it outside typed structs. |

### Config
| Crate | Purpose |
|---|---|
| `dotenvy` | Loads `.env` in local dev only (never used in the actual container). |

---

## 2. `cargo add` commands

Run from `backend/crates/api/`:

```bash
cargo add tokio --features full
cargo add axum
cargo add tower
cargo add tower-http --features "trace,compression-full,timeout,limit,cors"
cargo add tower_governor

cargo add sqlx --features "runtime-tokio-rustls,postgres,uuid,chrono"
cargo add uuid --features "v7,serde"

cargo add better-auth
cargo add jsonwebtoken

cargo add aws-sdk-s3
cargo add aws-config

cargo add moka --features future

cargo add thiserror
cargo add validator --features derive

cargo add tracing
cargo add tracing-subscriber --features "env-filter,json"

cargo add utoipa --features axum_extras
cargo add serde --features derive
cargo add serde_json

cargo add dotenvy --dev
```

For the `domain` crate (no axum/sqlx dependency, per the hexagonal split):

```bash
cargo add thiserror
cargo add serde --features derive
cargo add uuid --features "v7,serde"
```

---

## 3. API endpoints, in detail

Base path: `/v1`. Auth routes (`/api/auth/*`) are handled by better-auth.rs, not listed here.

### Meta

**`GET /healthz`** — Public. Liveness check; returns 200 if the process is up. No DB call.
**`GET /readyz`** — Public. Readiness check; pings the DB pool. Used by your deploy/orchestration layer to know when to start routing traffic.
**`GET /me`** — Authenticated. Returns the current user's profile (id, name, email, containers they belong to).
**`PATCH /me`** — Authenticated. Update display name/avatar.

### Containers

**`POST /containers`** — Authenticated. Creates a new container with the caller as owner. Enforces the 5-owned-containers cap inside a DB transaction alongside the membership-row insert (see flow in §4).

**`GET /containers`** — Authenticated. Lists containers the caller belongs to, with their role and basic usage stats in each.

**`GET /containers/{cid}`** — Member only (View+). Returns container metadata: name, lock status, storage/media usage against limits, member count.

**`PATCH /containers/{cid}`** — Owner only. Update name, storage limit, media limit.

**`DELETE /containers/{cid}`** — Owner only. Soft-deletes the container (sets `deleted_at`); a background job later purges R2 objects and hard-deletes rows.

**`PUT /containers/{cid}/lock`** — Owner only. Body `{ "locked": true|false }`. When locked, every non-owner request against this container returns `423 Locked`, regardless of role.

**`GET /containers/{cid}/usage`** — Member (View+). Storage bytes used/limit, media count used/limit, member count.

### Invites & allow-list (Edit access)

**`POST /containers/{cid}/edit-allowlist`** — Owner only. Body `{ "email": "friend@gmail.com" }`. Adds a Gmail address to the container's edit allow-list. No invite/accept step — the moment that email signs up or logs in, matching rows auto-grant Edit.

**`GET /containers/{cid}/edit-allowlist`** — Owner only. Lists allow-listed emails and whether each has claimed (signed up and been granted Edit) yet.

**`DELETE /containers/{cid}/edit-allowlist/{email}`** — Owner only. Removes an email from the allow-list. If that person already holds Edit via this allow-list entry, their role is revoked (demoted to no access, since view is via public link, not membership).

**`GET /containers/{cid}/share-link`** — Owner only. Returns (or lazily creates) the container's public View share link/token.

**`POST /containers/{cid}/share-link/rotate`** — Owner only. Invalidates the old view token and issues a new one — use if a link leaks somewhere you don't want.

**`GET /invites/{token}`** — Public, rate-limited. Resolves a share token to container name + thumbnail count, for a landing/preview page before the actual media grid loads.

### Members

**`GET /containers/{cid}/members`** — Member (View+, since it's useful context, but you may choose to restrict to Edit+ if member lists feel sensitive).

**`PATCH /containers/{cid}/members/{uid}`** — Owner only. Change a member's role directly (e.g. demote an editor).

**`DELETE /containers/{cid}/members/{uid}`** — Owner (remove anyone) or self (leave). If the caller is the owner and has no other route to ownership, this fails with `409` until ownership is transferred first.

**`POST /containers/{cid}/transfer-ownership`** — Owner only. Body `{ "new_owner_user_id": ... }`. Atomically swaps the owner role between caller and target; target must already be a member.

### Media

**`POST /containers/{cid}/uploads`** — Edit+. Body: filename, content-type, size. Validates remaining quota, creates a `pending` media row, returns a presigned R2 PUT URL.

**`POST /containers/{cid}/uploads/{mid}/parts`** — Edit+. For files large enough to need multipart upload (videos); returns presigned URLs per part.

**`POST /containers/{cid}/uploads/{mid}/complete`** — Uploader only. Confirms the upload finished; server does a `HEAD` against R2 to verify actual size, then marks the row `ready` and commits the quota usage.

**`DELETE /containers/{cid}/uploads/{mid}`** — Uploader only. Aborts an in-progress upload; cleans up the pending row and any partial R2 object.

**`GET /containers/{cid}/media`** — View+ (public link or member). Query params: `cursor`, `limit`, `type` (image/video), `uploader`. Keyset-paginated by UUIDv7.

**`GET /containers/{cid}/media/{mid}`** — View+. Single media item's metadata (uploader, timestamp, size, dimensions).

**`GET /containers/{cid}/media/{mid}/download`** — View+. Returns a short-TTL presigned R2 GET URL with `Content-Disposition: attachment`.

**`DELETE /containers/{cid}/media/{mid}`** — Uploader (their own) or Owner (anyone's).

**`POST /containers/{cid}/media/bulk-delete`** — Same permission rule as single delete, applied per id. Body: array of media ids, capped at ~100.

---

## 4. Request flows

Each trace shows the layer stack in order: **RequestId → rate limit → auth extraction → access-control extraction → handler → response**. The differences between scenarios live almost entirely in the access-control step.

### 4.1 Unauthenticated user viewing a public container's photos

```
Client → GET /containers/{cid}/media?cursor=...
  1. RequestId assigned, tracing span opened
  2. tower_governor: per-IP rate limit                → 429 if exceeded
  3. Auth extractor: MaybeUser — no session required,
     request proceeds even with no Authorization header
  4. ContainerAccess<View> extractor:
     a. Check container.locked                         → 423 if locked
        (locking overrides even public view access)
     b. No session → check for a valid share-link token
        in query param or header
     c. Token valid + container matches                → grants View
     d. Token invalid/missing/expired                   → 403
  5. Handler queries media rows (keyset page), builds
     presigned GET URLs for thumbnails
  6. Response: 200 with media list + pagination cursor
```

Nothing here touches user identity — this is the "basic / unauthenticated" tier from your original diagram, gated purely by the share token, not by who's asking.

### 4.2 Authenticated member viewing a private container

```
Client → GET /containers/{cid}/media (Authorization: Bearer <session>)
  1. RequestId + rate limit
  2. Auth extractor: AuthUser — verifies session via
     better-auth.rs                                     → 401 if invalid/expired
  3. ContainerAccess<View> extractor:
     a. moka cache lookup for (user_id, cid) role
        - cache hit → use cached role
        - cache miss → query container_members
                       WHERE container_id=? AND user_id=?
                       → cache the result (~30s TTL)
     b. Check container.locked                           → 423 if locked (non-owner)
     c. role is NULL (not a member, no share token used)  → 403
     d. role >= View                                      → proceed
  4. Handler: same media query as 4.1
  5. Response: 200
```

Same handler code as the public case — only the access-control extractor differs, which is exactly why nesting everything under one `ContainerAccess<Role>` extractor (rather than per-route checks) keeps this consistent.

### 4.3 Authenticated owner creating a container

```
Client → POST /containers  { "name": "CS Batch 2022" }
  1. RequestId + rate limit
  2. Auth extractor: AuthUser                            → 401 if invalid
  3. Payload validation via `validator`                  → 400 on bad input
  4. Handler opens a DB transaction:
     a. SELECT COUNT(*) FROM containers
        WHERE owner_id = user AND deleted_at IS NULL
     b. IF count >= 5                                    → 409 Conflict, rollback
     c. INSERT INTO containers (id=uuidv7, owner_id, name,
        storage_limit, media_limit)
     d. INSERT INTO container_members (container_id,
        user_id, role='owner')
     e. COMMIT
  5. moka cache seeded with (user, new_cid) → owner
  6. Response: 201 with the new container row
```

The transaction is what prevents a crash between steps (c) and (d) from producing a container with zero members — either both inserts land, or neither does.

### 4.4 Authenticated owner adding someone to the Edit allow-list

```
Client → POST /containers/{cid}/edit-allowlist  { "email": "friend@gmail.com" }
  1. RequestId + rate limit
  2. Auth extractor: AuthUser                            → 401 if invalid
  3. ContainerAccess<Owner> extractor (role check as in 4.2,
     but requiring role == Owner specifically)            → 403 if not owner
  4. Handler:
     a. Validate email format
     b. INSERT INTO container_edit_allowlist
        (container_id, email) — unique constraint on
        (container_id, email) prevents duplicates
  5. Response: 201, allow-list entry created
     — note: nothing is granted yet. This is the
       allow-list model you chose: access resolves
       later, at sign-up/login time, not here.
```

### 4.5 The allow-listed friend signs up and is auto-granted Edit

```
Client → POST /api/auth/sign-up  (handled by better-auth.rs)
  1. better-auth.rs creates the user record, session
  2. Post-signup hook (or a first-login check in your
     /me handler) runs:
     a. SELECT * FROM container_edit_allowlist
        WHERE email = new_user.email
     b. FOR EACH matching row:
        - INSERT INTO container_members
          (container_id, user_id, role='edit')
          ON CONFLICT DO NOTHING
        - mark allowlist row as "claimed"
  3. moka cache: no entries to invalidate yet, since
     this user had no prior cached role for these containers
  4. Response: signup completes as normal; user now has
     Edit on every container that allow-listed their email
```

This is the step that makes the allow-list model different from an invite-link model: there's no separate "accept" click. The matching happens passively, triggered by the person simply existing in your auth system with that email — whether they signed up specifically because of this container or already had an account and are logging in for the first time since being allow-listed.

### 4.6 Authenticated editor uploading a photo

```
Client → POST /containers/{cid}/uploads
         { "filename": "img.jpg", "content_type": "image/jpeg", "size": 4200000 }
  1. RequestId + rate limit
  2. Auth extractor: AuthUser                            → 401 if invalid
  3. ContainerAccess<Edit> extractor                     → 403 if role < Edit,
                                                             423 if locked
  4. Handler, in one atomic UPDATE:
     UPDATE containers
     SET storage_bytes = storage_bytes + $size,
         media_count = media_count + 1
     WHERE id = $cid
       AND storage_bytes + $size <= storage_limit
       AND media_count + 1 <= media_limit
     RETURNING id
                                                          → 409 if no row returned
                                                             (quota would be exceeded)
  5. INSERT INTO media (id=uuidv7, container_id, uploader_id,
     status='pending', ...)
  6. Sign an R2 PUT URL scoped to key
     c/{cid}/{mid}/orig, with Content-Length and
     Content-Type baked in
  7. Response: 201 with { media_id, upload_url }
  8. (Client PUTs bytes directly to R2 — server not involved)
  9. Client → POST /containers/{cid}/uploads/{mid}/complete
     a. HEAD the R2 object, verify size matches
     b. UPDATE media SET status='ready'
     c. Response: 200
```

The atomic `UPDATE ... WHERE ... RETURNING` is what prevents two simultaneous uploads from both passing a "count then insert" check and blowing past the quota — the DB row lock does the work a separate check-then-act can't.

### 4.7 Owner locking a container mid-session

```
Client → PUT /containers/{cid}/lock  { "locked": true }
  1–3. Same as 4.4 (Owner-only access check)
  4. Handler: UPDATE containers SET locked = true WHERE id = $cid
  5. moka cache: invalidate every cached (user, cid) role
     entry for this container, OR shorten effective TTL —
     otherwise a member with a live cache entry could keep
     acting on a stale "unlocked" role for up to ~30s
  6. Response: 200
```

Any request from a non-owner against this container — including ones already mid-flow, like an upload's `complete` step — now hits the `423 Locked` branch in the access-control extractor on its next check, until the owner flips it back.

### 4.8 Authenticated non-member requesting an Owner-only action

```
Client → PATCH /containers/{cid}  (a member, but not owner)
  1–2. RequestId, rate limit, AuthUser auth              → 401 if invalid
  3. ContainerAccess<Owner> extractor:
     a. role lookup returns 'edit' (or 'view')
     b. role != Owner                                     → 403 Forbidden
  4. Request never reaches the handler
```

This is the "wrong role, right identity" case — distinct from 4.1/4.2's "no access at all," since the person is a real member, just under-privileged for this specific action. Same extractor, different branch.

---

## Summary of the access-control shape

Every scenario above reduces to the same extractor pipeline, varying only in which branch fires:

1. **Rate limit** — always first, unauthenticated or not
2. **Identity** — `MaybeUser` (allows anonymous through) or `AuthUser` (requires a valid session)
3. **Container access** — resolves to a role via: share token (anonymous View), cached membership (authenticated), or a hard `403`/`423` — then compares that role against what the route requires

Keeping this as one reusable extractor per route, rather than bespoke checks per handler, is what makes it tractable to reason about — every route's actual permission logic is a one-line type annotation (`ContainerAccess<Role>`), and the flow traces above are what that type expands to at runtime.

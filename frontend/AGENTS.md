# AGENTS.md — remera frontend

Read this before touching the codebase. It says what this client is,
how it talks to the backend, what's actually built vs. only planned,
and the conventions to keep so future changes don't fight past ones.

This file covers `frontend/` only. For the backend (Rust/axum API,
OpenAPI spec generation, crate layout), see `backend/AGENTS.md`.

## What this is

A React + Vite client for Remera — a private, invite-based app where a
friend group shares college photos/videos in a "container." Full
design context lives in the repo's `docs/` folder once that exists —
this file is the fast-orientation version.

## Stack

- **Vite** — dev server and build tool.
- **React 19** — UI.
- **Tailwind CSS 4** (`@tailwindcss/vite`) — styling, plus
  `tw-animate-css` for animation utilities.
- **`@base-ui/react`** + `shadcn` — headless component primitives and
  the CLI used to pull generated components into `src/components/ui/`.
- **`class-variance-authority`** + `cn` — variant-driven component
  className composition (the `cva`/`cn` pattern shadcn components use).
- **`lucide-react`** — icons.
- **Biome** — lint + format (not ESLint/Prettier). `bun run check` runs
  both lint and format together.
- **TypeScript**, project-referenced (`tsconfig.json` → `tsconfig.app.json`
  / `tsconfig.node.json`), with a `@/*` → `src/*` path alias (mirrored
  in `vite.config.ts`'s `resolve.alias`, so both the type-checker and
  the bundler agree on it).

## How this talks to the backend

The backend (`backend/`) exposes an OpenAPI spec at `/openapi.json`
(via `utoipa`, see `backend/AGENTS.md`). This project does **not**
hand-write API request code — it's generated from that spec by
`orval`.

- `orval.config.ts` (repo root of `frontend/`) — orval's config. Points
  at the backend's live `/openapi.json`, outputs an axios-based client.
- `src/lib/api/` — **generated, do not hand-edit.** One subfolder per
  OpenAPI tag (e.g. `meta/` for routes tagged `"Meta"` on the backend),
  plus `models/` for the generated TypeScript types. Regenerate with
  `bun run gen:api` (backend must be running — see below).
- `src/lib/axios-instance.ts` — **hand-written, not generated.** The
  actual `axios.create(...)` instance plus interceptors (auth headers,
  401 handling, etc. go here). Orval-generated functions call through
  this via a "mutator" (`customInstance`), configured in
  `orval.config.ts`'s `output.override.mutator`. This is the one file
  you edit when you need to change how *every* request behaves — don't
  add per-call interceptor logic elsewhere.
- `.env` — `VITE_API_BASE_URL` must match wherever the backend is
  actually running (check `backend/.env`'s `PORT`) and
  `orval.config.ts`'s `input` URL. These are two separate hardcoded
  values that both need to change together if the backend's port ever
  changes — nothing keeps them in sync automatically.

### Regenerating the client

```bash
# 1. backend running, from backend/:
cargo run

# 2. from frontend/, backend must be up:
bun run gen:api
```

A pre-commit hook (`.husky/pre-commit`, repo root) does this
automatically whenever `backend/crates/api` changes — it boots the
backend temporarily, waits for `/openapi.json` to respond, runs
`gen:api`, stages the regenerated `src/lib/api/`, then kills the
temporary backend process. If a commit touching backend routes seems
slow, that's why.

## Hard rules — don't violate these

1. **Never hand-edit anything under `src/lib/api/`.** It's regenerated
   wholesale by `bun run gen:api` — hand edits get silently overwritten
   next regeneration. If generated output is wrong, fix it at the
   source (the backend's `#[utoipa::path]`/`ToSchema` annotations) or
   in `orval.config.ts`, not in the generated file.
2. **Interceptor/auth logic lives in `axios-instance.ts` only.** Don't
   reach into `AXIOS_INSTANCE` or add ad-hoc headers at individual
   call sites — every request should be able to assume the same
   baseline behavior.
3. **Use the `@/` alias, not relative `../../..` paths**, for anything
   outside the current file's immediate folder.
4. **Update the "Current state" section below** when you add something
   non-obvious, so the next agent/session doesn't have to re-derive
   what exists by reading every file.

## Current state (update this section as you go)

**Built:**
- Vite + React 19 + Tailwind 4 scaffold, Biome for lint/format
- `shadcn`/`@base-ui/react` wired up (`components/ui/button.tsx` as
  the first generated component)
- `orval`-generated axios API client (`src/lib/api/`), currently only
  covering the backend's `Meta` tag (`/`, `/healthz`)
- Custom axios instance with interceptor hooks in place, not yet doing
  anything (`src/lib/axios-instance.ts`)
- Pre-commit automation keeping the generated client in sync with
  backend changes

**Not yet built** (planned, not implemented):
- TanStack Query — once added, `orval.config.ts`'s `output` gains a
  `mode`/`query` option to generate query hooks alongside the plain
  axios functions, so call sites use `useHealthz()`-style hooks
  instead of calling generated functions directly inside effects
- Zustand for client-only state (selected container, upload progress,
  UI state) — server state stays in Query's cache, not Zustand; see
  the design discussion in `backend/AGENTS.md` for the broader app
  shape this is built against
- Auth UI (sign-up/login, invite-link landing page, allow-list flow)
- Container views, media grid/upload UI — depend on the corresponding
  backend routes existing first (see `backend/AGENTS.md`'s "Not yet
  built")

## Commands

From `frontend/`:

```bash
bun install
bun run dev          # start dev server (localhost:5173)
bun run build         # tsc -b && vite build
bun run check          # biome lint + format, writes fixes
bun run typecheck       # tsc --noEmit
bun run gen:api          # regenerate src/lib/api/ from the running backend
```

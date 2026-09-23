# Remera

Monorepo for Remera — a private, invite-based app where a friend group shares
college photos/videos in a "container" (one container = one group's archive).

- `frontend/` — React + Vite + Tailwind client
- `backend/` — Rust (axum) API. See `backend/AGENTS.md` for the full backend
  architecture, conventions, and what's actually built vs. only decided.

## Prerequisites

- [Bun](https://bun.sh) — package manager/runtime for both the repo root
  (git hooks) and `frontend/`.
  ```bash
  curl -fsSL https://bun.sh/install | bash
  ```
- [Rust](https://rustup.rs) — for `backend/`.
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

## Setup

Clone the repo, then from the repo root:

```bash
bun install
```

This installs root-level tooling (Husky) and sets up git hooks via the
`prepare` script — run this once, before working in either `frontend/` or
`backend/`.

### Frontend

```bash
cd frontend
bun install
bun run dev
```

Other useful scripts (see `frontend/package.json`): `bun run build`,
`bun run lint`, `bun run typecheck`.

### Backend

The backend is a Cargo workspace. Always run Cargo commands from `backend/`
— never from inside `backend/crates/*` (see `backend/AGENTS.md`, rule 2).

```bash
cd backend
cp .env.example .env   # fill in values as needed; PORT works out of the box
cargo run
```

This starts the `api` binary. Right now that's enough to hit `GET
/healthz` — the database, auth, and object storage layers aren't wired up
yet, so `DATABASE_URL`/R2/auth values in `.env` aren't required until those
land. Check `backend/AGENTS.md`'s "Current state" section for what's
actually built.

```bash
cargo build                          # build the whole workspace
cargo test                           # run tests across all member crates
cargo fmt && cargo clippy -D warnings   # before committing
```

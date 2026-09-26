-- edit allowlist: owner-controlled, email-based Edit access grants.
-- Grants resolve passively (no accept step) — see backend/AGENTS.md's
-- "Design decisions already made" and routes/invites.rs.
CREATE TABLE IF NOT EXISTS container_edit_allowlist (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    container_id UUID NOT NULL REFERENCES container(id) ON DELETE CASCADE,
    email        TEXT NOT NULL,
    claimed_at   TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (container_id, email)
);

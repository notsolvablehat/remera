-- Soft-delete support for containers (see backend/AGENTS.md — DELETE
-- /containers/{cid} sets this instead of removing the row; a background
-- job to purge R2 objects and hard-delete is future work, not this
-- migration's concern).
ALTER TABLE container
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;

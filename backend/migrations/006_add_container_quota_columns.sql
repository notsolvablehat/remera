-- Free-tier quota tracking, per container (see backend/AGENTS.md's
-- "Design decisions already made" — R2 free tier is 10 GB / zero egress,
-- so per-container caps keep any one group from exhausting the budget).
-- Defaults match domain::DEFAULT_STORAGE_LIMIT_BYTES / DEFAULT_MEDIA_LIMIT.
ALTER TABLE container
    ADD COLUMN IF NOT EXISTS storage_bytes BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS storage_limit BIGINT NOT NULL DEFAULT 2147483648,
    ADD COLUMN IF NOT EXISTS media_count INT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS media_limit INT NOT NULL DEFAULT 2000;

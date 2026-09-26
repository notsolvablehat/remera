-- View share-link support. The token handed out to clients is an
-- encrypted blob embedding (container_id, share_link_id) — this column
-- is what "rotate" changes to invalidate every previously issued token
-- for the container without needing a separate token-tracking table.
ALTER TABLE container
    ADD COLUMN IF NOT EXISTS share_link_id UUID;

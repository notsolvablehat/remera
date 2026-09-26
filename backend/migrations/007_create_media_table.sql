-- media.sql
-- Individual uploaded files (photos/videos) inside a container. Two-phase
-- upload: a row starts 'pending' (quota already reserved, presigned PUT
-- handed to the client) and flips to 'ready' once the object is verified
-- in R2 via HEAD. 'pending' rows that never complete are cleaned up by
-- the abort endpoint (DELETE .../uploads/{mid}).
CREATE TABLE IF NOT EXISTS media (
    id           UUID PRIMARY KEY,
    container_id UUID NOT NULL REFERENCES container(id) ON DELETE CASCADE,
    uploader_id  TEXT NOT NULL REFERENCES users(id),
    object_key   TEXT NOT NULL UNIQUE,
    filename     TEXT NOT NULL,
    caption      TEXT,
    content_type TEXT NOT NULL,
    size_bytes   BIGINT NOT NULL,
    status       TEXT NOT NULL CHECK (status IN ('pending', 'ready')) DEFAULT 'pending',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS media_container_id_idx ON media (container_id, id);

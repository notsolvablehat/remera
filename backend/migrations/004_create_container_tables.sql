-- containers.sql
CREATE TABLE IF NOT EXISTS container (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id    TEXT NOT NULL REFERENCES users(id),   -- TEXT FK to better-auth's users table
    name        TEXT NOT NULL,
    is_public   BOOLEAN NOT NULL DEFAULT FALSE,
    is_locked   BOOLEAN NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- members.sql
CREATE TABLE IF NOT EXISTS container_member (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    container_id UUID NOT NULL REFERENCES container(id) ON DELETE CASCADE,
    user_id      TEXT NOT NULL REFERENCES users(id),  -- TEXT FK
    role         TEXT NOT NULL CHECK (role IN ('owner', 'editor', 'viewer')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (container_id, user_id)
);

-- invites.sql
CREATE TABLE IF NOT EXISTS invite (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    container_id UUID NOT NULL REFERENCES container(id) ON DELETE CASCADE,
    email        TEXT NOT NULL,
    role         TEXT NOT NULL CHECK (role IN ('editor', 'viewer')),
    token        TEXT NOT NULL UNIQUE,        -- AES-GCM encrypted blob
    accepted_at  TIMESTAMPTZ,
    expires_at   TIMESTAMPTZ NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
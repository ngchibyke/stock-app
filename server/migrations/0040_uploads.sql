-- Photo uploads (plan 7, S2). One row per stored photo, so every place that accepts a photo
-- key can check the caller uploaded it (roadmap: "check that a key belongs to the uploader").
-- Photos already attached to products before this migration keep working: only keys a caller
-- newly attaches are checked.
CREATE TABLE uploads (
    key          text PRIMARY KEY CHECK (key ~ '^photos/[0-9a-f-]{36}\.(jpg|png|webp)$'),
    uploader_id  uuid NOT NULL REFERENCES users (id),
    content_type text NOT NULL CHECK (content_type IN ('image/jpeg', 'image/png', 'image/webp')),
    bytes        integer NOT NULL CHECK (bytes > 0),
    width        integer NOT NULL CHECK (width > 0),
    height       integer NOT NULL CHECK (height > 0),
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX uploads_uploader_idx ON uploads (uploader_id, created_at DESC);

-- Photo uploads (plan 7a, task 5). One row per photo a person asks to upload.
--
-- The flow: the app asks for a slot (`pending`), puts the file in private
-- storage under `raw_key`, and asks the server to finalize it. The server reads
-- the raw file, checks it, re-encodes it (which drops EXIF and GPS), writes the
-- result to `key`, deletes the raw file, and the row becomes `ready`. A resource
-- (a product, a review, a dispute) may then attach the key; `attached_*` records
-- where, so a key cannot be used twice and cannot be claimed by someone else.
--
-- Personal data: the object itself is a photo, which can show faces and
-- surroundings. This table holds only keys, sizes and ids, never the picture.
-- Retention of uploads goes to the DPO's list, and plan 8's erasure must delete
-- a person's uploads with their objects.

CREATE TABLE uploads (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id        uuid NOT NULL REFERENCES users (id),
    purpose        text NOT NULL
                   CHECK (purpose IN ('product', 'store_logo', 'store_banner', 'review', 'dispute')),
    raw_key        text NOT NULL,
    key            text,
    declared_type  text NOT NULL CHECK (declared_type IN ('image/jpeg', 'image/png', 'image/webp')),
    declared_bytes integer NOT NULL CHECK (declared_bytes BETWEEN 1 AND 5242880),
    status         text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'ready', 'failed')),
    -- Why a failed upload failed, as a fixed code (never the decoder's text).
    failure        text,
    attached_kind  text,
    attached_id    uuid,
    created_at     timestamptz NOT NULL DEFAULT now(),
    finalized_at   timestamptz,
    CHECK ((status = 'ready') = (key IS NOT NULL)),
    CHECK ((attached_kind IS NULL) = (attached_id IS NULL))
);

CREATE UNIQUE INDEX uploads_key_idx ON uploads (key) WHERE key IS NOT NULL;
CREATE INDEX uploads_user_idx ON uploads (user_id, created_at);
CREATE INDEX uploads_stale_idx ON uploads (created_at) WHERE status <> 'ready';

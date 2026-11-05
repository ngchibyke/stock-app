-- Moderation shared by chat and reviews (spec section 7).
-- Keep the setting ranges in step with src/settings/service.rs.
-- Personal data: `moderation_flags.original_enc` and `suggestion_enc` hold what a
-- person wrote, encrypted with FieldCipher, readable by the Owner only. Retention
-- (12 months, longer in a dispute) is plan 8's purge job.

-- Separate from seller_strikes, which are about stores missing ship deadlines.
CREATE TABLE moderation_strikes (
    id        bigserial PRIMARY KEY,
    user_id   uuid NOT NULL REFERENCES users (id),
    category  text NOT NULL,
    at        timestamptz NOT NULL DEFAULT now(),
    voided_at timestamptz,
    voided_by uuid REFERENCES users (id)
);
CREATE INDEX moderation_strikes_user_idx ON moderation_strikes (user_id, at);

CREATE TABLE moderation_flags (
    id                 uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id            uuid NOT NULL REFERENCES users (id),
    surface            text NOT NULL CHECK (surface IN ('chat', 'review', 'review_reply')),
    -- The foreign key is added with the table, in 0032.
    conversation_id    uuid,
    order_id           uuid REFERENCES orders (id),
    severity           text NOT NULL CHECK (severity IN ('mild', 'serious', 'unreviewable')),
    category           text NOT NULL,
    source             text NOT NULL CHECK (source IN ('filter', 'tone', 'system')),
    original_enc       bytea NOT NULL,
    suggestion_enc     bytea,
    suggestion_used_at timestamptz,
    strike_id          bigint REFERENCES moderation_strikes (id),
    status             text NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'upheld', 'dismissed')),
    resolved_by        uuid REFERENCES users (id),
    resolved_at        timestamptz,
    created_at         timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX moderation_flags_queue_idx ON moderation_flags (status, created_at);
CREATE INDEX moderation_flags_user_idx ON moderation_flags (user_id, created_at);

-- One row per user who has ever been muted, banned or sent for review.
CREATE TABLE user_moderation (
    user_id         uuid PRIMARY KEY REFERENCES users (id),
    muted_until     timestamptz,
    banned_at       timestamptz,
    banned_by       uuid REFERENCES users (id),
    owner_review_at timestamptz
);
CREATE INDEX user_moderation_review_idx ON user_moderation (owner_review_at)
    WHERE owner_review_at IS NOT NULL AND banned_at IS NULL;

ALTER TABLE platform_settings
    ADD COLUMN strike_window_days int NOT NULL DEFAULT 30 CHECK (strike_window_days BETWEEN 1 AND 365),
    ADD COLUMN mute_after_strikes int NOT NULL DEFAULT 3 CHECK (mute_after_strikes BETWEEN 1 AND 20),
    ADD COLUMN mute_hours int NOT NULL DEFAULT 24 CHECK (mute_hours BETWEEN 1 AND 720),
    ADD COLUMN owner_review_after_strikes int NOT NULL DEFAULT 5
        CHECK (owner_review_after_strikes BETWEEN 2 AND 50),
    ADD CONSTRAINT platform_settings_review_after_mute
        CHECK (owner_review_after_strikes > mute_after_strikes);

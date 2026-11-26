-- Part 7e-1b: seller identity check (NIN plus live selfie, through a provider).
-- The NIN itself is never stored: only `nin_idx`, a keyed HMAC under NIN_INDEX_KEY.

CREATE TABLE identity_checks (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id       uuid NOT NULL REFERENCES users (id),
    provider      text NOT NULL,
    -- The provider's job reference; NULL for a refusal or an owner-review row made without a call.
    provider_ref  text,
    -- NULL on a refused start: a prober leaves no fingerprint of someone else's NIN.
    nin_idx       bytea,
    status        text NOT NULL
        CHECK (status IN ('started', 'passed', 'failed', 'review', 'revoked')),
    -- A code, never text: not_accepted, not_found, face_mismatch, provider_unsure,
    -- name_mismatch, payout_name_mismatch, under_18, attempts_exhausted, expired, fraud, ...
    reason        text CHECK (reason ~ '^[a-z0-9_]{1,40}$'),
    full_name_enc bytea,
    birth_year    integer CHECK (birth_year BETWEEN 1900 AND 2100),
    -- True once the attempt reached (or was due to reach) the provider: counts for the budget.
    called_provider boolean NOT NULL DEFAULT false,
    started_at    timestamptz NOT NULL DEFAULT now(),
    decided_at    timestamptz,
    decided_by    uuid REFERENCES users (id),
    -- Owner-written reason for a review decision or a revoke. Owner-only; never in audit_log.
    decision_note text CHECK (char_length(decision_note) <= 500),
    CONSTRAINT identity_checks_nin_len CHECK (nin_idx IS NULL OR octet_length(nin_idx) = 32)
);
CREATE INDEX identity_checks_user_idx ON identity_checks (user_id, started_at DESC);
CREATE INDEX identity_checks_queue_idx ON identity_checks (started_at) WHERE status = 'review';
CREATE INDEX identity_checks_open_idx ON identity_checks (started_at) WHERE status = 'started';
CREATE UNIQUE INDEX identity_checks_provider_ref_idx
    ON identity_checks (provider, provider_ref) WHERE provider_ref IS NOT NULL;
-- At most one live attempt per user.
CREATE UNIQUE INDEX identity_checks_one_open_idx ON identity_checks (user_id) WHERE status = 'started';
-- One NIN, one account: a NIN held by a passed, revoked or (verified) review row is taken.
CREATE UNIQUE INDEX identity_checks_nin_taken_idx ON identity_checks (nin_idx)
    WHERE nin_idx IS NOT NULL
      AND (status IN ('passed', 'revoked')
           OR (status = 'review' AND reason IS DISTINCT FROM 'attempts_exhausted'));

-- A banned user's fingerprints. One NIN may appear for several banned users.
CREATE TABLE banned_nins (
    nin_idx   bytea NOT NULL CHECK (octet_length(nin_idx) = 32),
    user_id   uuid NOT NULL REFERENCES users (id),
    banned_at timestamptz NOT NULL DEFAULT now(),
    banned_by uuid REFERENCES users (id),
    PRIMARY KEY (nin_idx, user_id)
);
CREATE INDEX banned_nins_user_idx ON banned_nins (user_id);

ALTER TABLE stores ADD COLUMN identity_due_at timestamptz;

ALTER TABLE platform_settings
    ADD COLUMN identity_grace_days integer NOT NULL DEFAULT 14
        CHECK (identity_grace_days BETWEEN 1 AND 90),
    ADD COLUMN identity_attempts_per_day integer NOT NULL DEFAULT 3
        CHECK (identity_attempts_per_day BETWEEN 1 AND 20),
    ADD COLUMN identity_attempts_total integer NOT NULL DEFAULT 5
        CHECK (identity_attempts_total BETWEEN 1 AND 50),
    ADD COLUMN identity_daily_budget integer NOT NULL DEFAULT 200
        CHECK (identity_daily_budget BETWEEN 0 AND 100000),
    ADD COLUMN identity_refusal_delay_ms integer NOT NULL DEFAULT 2000
        CHECK (identity_refusal_delay_ms BETWEEN 0 AND 10000);

-- Existing sellers: stores active when this ships get the grace period.
UPDATE stores SET identity_due_at = now() + make_interval(
        days => (SELECT identity_grace_days FROM platform_settings WHERE id))
WHERE status = 'active' AND deleted_at IS NULL;

ALTER TABLE notifications DROP CONSTRAINT notifications_kind_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check CHECK (kind IN
    ('order_paid', 'order_picked_up', 'order_out_for_delivery', 'order_delivered',
     'confirm_reminder', 'release_tomorrow', 'refund_started', 'funds_released',
     'chat_message', 'dispute_opened', 'return_requested', 'dispute_decided',
     'moderation_warning', 'moderation_muted',
     'store_invite', 'order_new', 'order_ready_for_collection', 'collect_reminder',
     'collection_expired',
     'identity_passed', 'identity_declined', 'identity_due'));

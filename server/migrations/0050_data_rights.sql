-- Part 8: data rights and retention. A person can ask for deletion (30-day cooling-off,
-- then a purge) and for a copy of their data. Windows are Owner settings.

ALTER TABLE users ADD COLUMN erased_at timestamptz;

CREATE TABLE deletion_requests (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         uuid NOT NULL REFERENCES users (id),
    status          text NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'cancelled', 'done')),
    requested_at    timestamptz NOT NULL DEFAULT now(),
    due_at          timestamptz NOT NULL,
    decided_at      timestamptz,
    -- Times the purge found a blocker and waited. A count, never a reason with a name in it.
    deferred_count  integer NOT NULL DEFAULT 0 CHECK (deferred_count >= 0),
    CHECK ((status = 'pending') = (decided_at IS NULL))
);
-- One live request per person.
CREATE UNIQUE INDEX deletion_requests_one_pending_idx
    ON deletion_requests (user_id) WHERE status = 'pending';
CREATE INDEX deletion_requests_due_idx ON deletion_requests (due_at) WHERE status = 'pending';

-- A person's data export. The file is JSON encrypted under the field key; it is deleted
-- (body_enc cleared, status expired) once expires_at passes.
CREATE TABLE data_exports (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      uuid NOT NULL REFERENCES users (id),
    status       text NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'ready', 'failed', 'expired')),
    body_enc     bytea,
    requested_at timestamptz NOT NULL DEFAULT now(),
    ready_at     timestamptz,
    expires_at   timestamptz,
    CHECK ((status = 'ready') = (body_enc IS NOT NULL))
);
CREATE INDEX data_exports_user_idx ON data_exports (user_id, requested_at DESC);
CREATE UNIQUE INDEX data_exports_one_live_idx
    ON data_exports (user_id) WHERE status IN ('pending', 'ready');

-- Retention windows. Defaults are placeholders the DPO confirms before launch.
ALTER TABLE platform_settings
    ADD COLUMN chat_retention_months integer NOT NULL DEFAULT 12
        CHECK (chat_retention_months BETWEEN 1 AND 120),
    ADD COLUMN records_retention_years integer NOT NULL DEFAULT 6
        CHECK (records_retention_years BETWEEN 1 AND 30),
    ADD COLUMN delivery_address_retention_months integer NOT NULL DEFAULT 12
        CHECK (delivery_address_retention_months BETWEEN 1 AND 120),
    ADD COLUMN notice_retention_months integer NOT NULL DEFAULT 6
        CHECK (notice_retention_months BETWEEN 1 AND 60),
    ADD COLUMN push_device_stale_days integer NOT NULL DEFAULT 90
        CHECK (push_device_stale_days BETWEEN 7 AND 730),
    ADD COLUMN attempt_retention_days integer NOT NULL DEFAULT 30
        CHECK (attempt_retention_days BETWEEN 1 AND 365),
    ADD COLUMN deletion_cooling_off_days integer NOT NULL DEFAULT 30
        CHECK (deletion_cooling_off_days BETWEEN 7 AND 60);

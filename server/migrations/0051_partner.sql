-- Part 9: the partner-services seam. A partner is optional: with none configured, or the
-- switch off, nothing here does anything.

-- Signed events the partner sends us. Stored once by `event_id`; nothing handles a kind yet.
CREATE TABLE partner_events (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    event_id     text NOT NULL UNIQUE CHECK (char_length(event_id) BETWEEN 1 AND 100),
    kind         text NOT NULL CHECK (kind ~ '^[a-z0-9_.]{1,60}$'),
    payload      jsonb NOT NULL,
    received_at  timestamptz NOT NULL DEFAULT now(),
    processed_at timestamptz
);
CREATE INDEX partner_events_open_idx ON partner_events (received_at) WHERE processed_at IS NULL;

-- Calls we paid for, per day and feature.
CREATE TABLE partner_usage (
    day     date NOT NULL,
    feature text NOT NULL CHECK (feature ~ '^[a-z0-9_]{1,40}$'),
    calls   integer NOT NULL DEFAULT 0 CHECK (calls >= 0),
    errors  integer NOT NULL DEFAULT 0 CHECK (errors >= 0),
    PRIMARY KEY (day, feature)
);

ALTER TABLE platform_settings
    ADD COLUMN partner_enabled boolean NOT NULL DEFAULT false,
    -- A map of feature name to true or false, for example {"price_history": true}.
    ADD COLUMN partner_features jsonb NOT NULL DEFAULT '{}'
        CHECK (jsonb_typeof(partner_features) = 'object'),
    -- NULL means no cap.
    ADD COLUMN partner_monthly_call_budget integer
        CHECK (partner_monthly_call_budget IS NULL
               OR partner_monthly_call_budget BETWEEN 0 AND 100000000);

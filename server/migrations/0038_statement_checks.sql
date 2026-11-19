-- Daily reconciliation of the provider's statement against the ledger (plan 6). One row per
-- check. A day can be checked again, so `day` is not unique. `unavailable` rows record a try
-- the provider could not answer; the job tries such a day again an hour later.
CREATE TABLE statement_checks (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    day           date NOT NULL,
    source        text NOT NULL,
    status        text NOT NULL CHECK (status IN ('matched', 'mismatch', 'unavailable')),
    ledger        jsonb,
    statement     jsonb,
    diff          jsonb,
    tolerance_kobo bigint NOT NULL CHECK (tolerance_kobo >= 0),
    checked_by    uuid REFERENCES users (id),
    checked_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX statement_checks_day_idx ON statement_checks (day, checked_at);

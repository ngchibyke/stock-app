-- A code is issued for one purpose and verifies only for that purpose.
ALTER TABLE otp_codes
    ADD COLUMN purpose text NOT NULL DEFAULT 'verify_phone'
    CHECK (purpose IN ('verify_phone', 'password_reset'));

-- Password-reset requests per phone, counted the same way for phones with and without an
-- account so the limit cannot tell them apart. Keyed by the blind index, never the phone.
-- Rows older than an hour are pruned on write.
CREATE TABLE password_reset_requests (
    id        bigserial PRIMARY KEY,
    phone_idx bytea NOT NULL,
    at        timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX password_reset_requests_phone_at_idx ON password_reset_requests (phone_idx, at);
CREATE INDEX password_reset_requests_at_idx ON password_reset_requests (at);

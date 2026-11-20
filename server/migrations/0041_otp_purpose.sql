-- What a code is for. A code issued for one purpose never verifies for another.
ALTER TABLE otp_codes
    ADD COLUMN purpose text NOT NULL DEFAULT 'verify_phone'
    CHECK (purpose IN ('verify_phone', 'password_reset'));

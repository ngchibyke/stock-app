-- Buyer delivery addresses (Part 3, Task 2). Inside the FCT only: the area council
-- is an enum. Personal fields are encrypted (FieldCipher); the area council and the
-- postcode stay plaintext. Soft delete only; orders keep their own encrypted snapshot.
CREATE TABLE buyer_addresses (
    id                   uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id              uuid NOT NULL REFERENCES users (id),
    recipient_name_enc   bytea NOT NULL,
    phone_enc            bytea NOT NULL,
    address_line_enc     bytea NOT NULL,
    landmark_enc         bytea NULL,
    area_council         area_council NOT NULL,
    postcode             text NULL CHECK (postcode ~ '^FC[0-9]{2}[A-Z0-9]{7}$'),
    courier_address_code bigint NULL,
    deleted_at           timestamptz NULL,
    created_at           timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX buyer_addresses_user_live_idx
    ON buyer_addresses (user_id, created_at DESC) WHERE deleted_at IS NULL;

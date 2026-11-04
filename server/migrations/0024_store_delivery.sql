-- Store delivery details (Part 3, Task 3). Existing stores keep NULLs; they cannot be
-- checked out for the affected method until the fields are filled. The business phone
-- is personal data and is encrypted (FieldCipher); area council and postcode stay
-- plaintext. The fee is whole kobo, capped at 5,000,000.
ALTER TABLE stores
    ADD COLUMN pickup_area_council         area_council NULL,
    ADD COLUMN pickup_postcode             text NULL CHECK (pickup_postcode ~ '^FC[0-9]{2}[A-Z0-9]{7}$'),
    ADD COLUMN pickup_courier_address_code bigint NULL,
    ADD COLUMN business_phone_enc          bytea NULL,
    ADD COLUMN self_delivery_fee_kobo      bigint NULL
        CHECK (self_delivery_fee_kobo BETWEEN 0 AND 5000000);

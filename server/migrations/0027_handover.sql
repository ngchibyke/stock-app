-- Seller delivery steps and the self-delivery handover code (Part 3, Task 8).
-- `out_for_delivery` (0026) is usable here: it was committed by its own file.
--
-- Personal data: `handover_code_enc` is the buyer's 6-digit handover code,
-- encrypted with FieldCipher (never plaintext). It exists only on self-delivery
-- orders, from `out_for_delivery` until the order is delivered (then cleared).
ALTER TABLE orders
    -- Courier orders: the seller has the parcel ready for collection.
    ADD COLUMN ready_at              timestamptz NULL,
    ADD COLUMN out_for_delivery_at   timestamptz NULL,
    ADD COLUMN handover_code_enc     bytea NULL,
    -- Wrong code entries since the last lock (reset to 0 when a lock starts).
    ADD COLUMN handover_failures     integer NOT NULL DEFAULT 0
        CHECK (handover_failures BETWEEN 0 AND 4),
    ADD COLUMN handover_locked_until timestamptz NULL,
    ADD CONSTRAINT orders_handover_self_only CHECK (
        handover_code_enc IS NULL OR delivery_method = 'self'
    );

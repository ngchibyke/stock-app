-- Collect from a branch (plan 7e-1, task 10). The `collect` method, `ready_at`,
-- `collect_enabled`, `collect_window_days` and the notice kinds came with 0044.
--
-- Personal data: a collect order keeps no copy of the buyer's address (nothing is
-- delivered to it), so `delivery_address_enc` stays NULL there. Its handover code is
-- the buyer's, encrypted with FieldCipher like a self-delivery one: it exists from
-- "ready for collection" until the order is collected or cancelled (then cleared).

ALTER TABLE orders DROP CONSTRAINT orders_handover_self_only;
ALTER TABLE orders ADD CONSTRAINT orders_handover_self_only CHECK (
    handover_code_enc IS NULL OR delivery_method IN ('self', 'collect')
);

ALTER TABLE orders DROP CONSTRAINT orders_delivery_details;
ALTER TABLE orders ADD CONSTRAINT orders_delivery_details CHECK (
    delivery_method IS NULL
    OR ((delivery_address_enc IS NOT NULL OR delivery_address_purged_at IS NOT NULL
         OR delivery_method = 'collect')
        AND delivery_area IS NOT NULL AND promised_date IS NOT NULL)
);

-- When an uncollected order is cancelled: `ready_at + collect_window_days`, set
-- when it is marked ready, so a later settings change never moves it.
ALTER TABLE orders
    ADD COLUMN collect_by timestamptz NULL,
    ADD CONSTRAINT orders_collect_by_collect_only CHECK (
        collect_by IS NULL OR (delivery_method = 'collect' AND ready_at IS NOT NULL)
    );

-- The collect jobs: ready orders still waiting at the counter.
CREATE INDEX orders_waiting_collect_idx ON orders (collect_by, id)
    WHERE collect_by IS NOT NULL AND status = 'paid';

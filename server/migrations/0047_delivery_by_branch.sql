-- Delivery options and checkout by branch group (plan 7e-1, task 8).
-- Personal data: none added (ids, a fingerprint and a time). The store-level pickup
-- columns stop being written: every reader now uses the order's or group's branch.
-- They keep the values they had (read-only legacy copies, encrypted as before);
-- dropping them is on the DPO list with the rest of the branch move.

-- A group's variants, so checkout can rebuild the lines its fingerprint covers.
ALTER TABLE delivery_options ADD COLUMN group_variant_ids uuid[] NULL;
CREATE INDEX delivery_options_branch_idx ON delivery_options (branch_id);
-- Reusing a still-valid quote for the same group.
CREATE INDEX delivery_options_group_idx
    ON delivery_options (buyer_id, store_id, address_id, group_fingerprint, created_at);

-- When a branch's pickup really moved (address, area or postcode): a courier quote
-- made before it is never booked. The first branch takes the store's own stamp.
ALTER TABLE store_branches ADD COLUMN pickup_moved_at timestamptz NULL;
UPDATE store_branches b SET pickup_moved_at = s.pickup_moved_at
FROM stores s WHERE s.id = b.store_id AND b.position = 0;

-- New stores no longer copy their pickup address onto the store row.
ALTER TABLE stores ALTER COLUMN pickup_address_enc DROP NOT NULL;

-- One order per group: a store split across branches has one order per branch in a
-- checkout (one payment).
ALTER TABLE orders DROP CONSTRAINT orders_one_per_store;
ALTER TABLE orders ADD CONSTRAINT orders_one_per_branch UNIQUE (checkout_id, store_id, branch_id);

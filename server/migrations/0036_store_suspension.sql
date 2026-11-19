-- Owner suspension of a store (plan 6). `stores.status = 'suspended'` already hides the
-- store and its products and blocks new checkouts; these columns record who and why.
-- `status` stays the source of truth, so the columns carry no CHECK against it.
ALTER TABLE stores
    ADD COLUMN suspended_at     timestamptz,
    ADD COLUMN suspended_reason text,
    ADD COLUMN suspended_by     uuid REFERENCES users (id);

UPDATE stores SET suspended_at = now() WHERE status = 'suspended';

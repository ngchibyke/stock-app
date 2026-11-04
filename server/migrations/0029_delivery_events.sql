-- Delivery events (Part 3, Task 10): everything that was said about an order's
-- delivery, by the seller, the courier or the system, including updates that
-- changed nothing. Append-only.
--
-- `dedupe_key` is sha256 of the provider, its order id, the state and its time for
-- a courier event (no event id exists), and "{source}:{order_id}:{status}" for the
-- others. `occurred_at` is the sender's claim, NULL when it gave none, and it decides
-- nothing: every deadline runs from our own receipt time.
--
-- No personal data here: statuses and timestamps only.
CREATE TABLE delivery_events (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    order_id    uuid NOT NULL REFERENCES orders (id),
    source      text NOT NULL CHECK (source IN ('seller', 'courier', 'system')),
    status      text NOT NULL,
    occurred_at timestamptz NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    dedupe_key  text NOT NULL UNIQUE
);
CREATE INDEX delivery_events_order_idx ON delivery_events (order_id, received_at);

-- Append-only: reject UPDATE, DELETE and TRUNCATE.
CREATE FUNCTION delivery_events_reject_change() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'delivery_events is append-only (% not allowed)', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER delivery_events_no_update
    BEFORE UPDATE ON delivery_events
    FOR EACH ROW EXECUTE FUNCTION delivery_events_reject_change();
CREATE TRIGGER delivery_events_no_delete
    BEFORE DELETE ON delivery_events
    FOR EACH ROW EXECUTE FUNCTION delivery_events_reject_change();
CREATE TRIGGER delivery_events_no_truncate
    BEFORE TRUNCATE ON delivery_events
    FOR EACH STATEMENT EXECUTE FUNCTION delivery_events_reject_change();

-- ALWAYS: the triggers fire even with session_replication_role = replica.
ALTER TABLE delivery_events ENABLE ALWAYS TRIGGER delivery_events_no_update;
ALTER TABLE delivery_events ENABLE ALWAYS TRIGGER delivery_events_no_delete;
ALTER TABLE delivery_events ENABLE ALWAYS TRIGGER delivery_events_no_truncate;

ALTER TABLE shipments
    -- The last courier event of any kind, and the last `status` answer: the poll
    -- leaves a shipment alone until both are 6 hours old.
    ADD COLUMN last_event_at  timestamptz NULL,
    ADD COLUMN last_polled_at timestamptz NULL,
    -- Why the owner must look at this shipment: our own short code, never
    -- provider text. Stays set until an owner action exists to clear it (plan 6).
    ADD COLUMN attention      text NULL;
CREATE INDEX shipments_poll_idx ON shipments (updated_at, id)
    WHERE status IN ('booked', 'confirmed', 'picked_up', 'in_transit');

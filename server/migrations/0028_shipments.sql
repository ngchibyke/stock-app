-- Courier shipments (Part 3, Task 9). One row per courier order, created when the
-- seller marks it ready. Booking debits the platform's prepaid wallet and the
-- provider has no idempotency key, so this row is the only guard against booking
-- twice: `booking` holds a lease while the POST is in flight, and a row left there
-- past its lease goes to `booking_unknown`, never back to `booking_pending`.
--
-- `cancel_pending` is the refund's outbox: the refund transaction sets it, and the
-- job loop calls the provider after commit, so a crash cannot lose the cancel.
--
-- Personal data: `courier_phone_enc` is the courier's phone, encrypted with
-- FieldCipher (never plaintext). `last_error` holds our own short codes, never
-- provider text.
-- When the store's pickup really moved (address, area or postcode). The saved
-- courier address code cannot answer that: it is also cleared by a phone change
-- and by resaving the same address.
ALTER TABLE stores ADD COLUMN pickup_moved_at timestamptz NULL;

CREATE TYPE shipment_status AS ENUM (
    'booking_pending',
    'booking',
    'booking_unknown',
    'booking_failed',
    'booked',
    'confirmed',
    'picked_up',
    'in_transit',
    'completed',
    'cancel_pending',
    'cancelled',
    'cancel_failed'
);

CREATE TABLE shipments (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    order_id          uuid NOT NULL UNIQUE REFERENCES orders (id),
    -- The provider asked to book; NULL until the first booking attempt.
    provider          text NULL,
    status            shipment_status NOT NULL DEFAULT 'booking_pending',
    provider_order_id text NULL UNIQUE,
    tracking_url      text NULL,
    courier_name      text NULL,
    courier_phone_enc bytea NULL,
    -- What the buyer paid for delivery; `charged_kobo` is what the provider debited.
    quoted_kobo       bigint NOT NULL CHECK (quoted_kobo >= 0),
    charged_kobo      bigint NULL CHECK (charged_kobo >= 0),
    attempts          integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    cancel_attempts   integer NOT NULL DEFAULT 0 CHECK (cancel_attempts >= 0),
    -- When the shipment entered `cancel_pending`.
    cancel_requested_at timestamptz NULL,
    -- The lease and fencing token of a booking or cancel call in flight.
    sending_until     timestamptz NULL,
    next_attempt_at   timestamptz NOT NULL DEFAULT now(),
    last_error        text NULL,
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT shipments_booking_has_lease CHECK (
        status <> 'booking' OR sending_until IS NOT NULL
    ),
    -- Every status past a successful booking names the provider's order.
    CONSTRAINT shipments_booked_has_order CHECK (
        status IN ('booking_pending', 'booking', 'booking_unknown', 'booking_failed', 'cancelled')
        OR provider_order_id IS NOT NULL
    )
);
CREATE INDEX shipments_booking_due_idx ON shipments (created_at, id)
    WHERE status = 'booking_pending';
CREATE INDEX shipments_lease_idx ON shipments (sending_until)
    WHERE status IN ('booking', 'cancel_pending');
CREATE INDEX shipments_attention_idx ON shipments (updated_at, id)
    WHERE status IN ('booking', 'booking_unknown', 'booking_failed', 'cancel_failed');
-- The job's fallback: ready courier orders that have no shipment yet.
CREATE INDEX orders_ready_courier_idx ON orders (ready_at, id)
    WHERE ready_at IS NOT NULL AND delivery_method = 'courier';

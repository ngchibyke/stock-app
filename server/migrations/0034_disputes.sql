-- Disputes (spec section 5): a buyer reports a problem with a shipped order and
-- the order's money stays frozen until the market decides. One per order, ever.
-- Personal data: the parties' statements (encrypted, `body_enc`) and the photo
-- keys they attach. Nothing else here identifies a person beyond ids. Retention
-- of statements goes to the DPO's list.

CREATE TABLE disputes (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    order_id            uuid NOT NULL UNIQUE REFERENCES orders (id),
    buyer_id            uuid NOT NULL REFERENCES users (id),
    store_id            uuid NOT NULL REFERENCES stores (id),
    reason              text NOT NULL CHECK (reason IN
        ('not_received', 'damaged', 'wrong_item', 'not_as_described', 'incomplete', 'other')),
    status              text NOT NULL CHECK (status IN
        ('awaiting_seller', 'awaiting_return', 'awaiting_decision', 'decided', 'withdrawn')),
    opened_at           timestamptz NOT NULL DEFAULT now(),
    seller_respond_by   timestamptz NOT NULL,
    seller_responded_at timestamptz,
    seller_missed_deadline boolean NOT NULL DEFAULT false,
    return_paid_by      text CHECK (return_paid_by IN ('buyer', 'seller')),
    return_requested_at timestamptz,
    return_received_at  timestamptz,
    decision            text CHECK (decision IN ('full_refund', 'partial_refund', 'release')),
    refund_kobo         bigint CHECK (refund_kobo > 0),
    decided_by          uuid REFERENCES users (id),
    decided_at          timestamptz,
    withdrawn_at        timestamptz,
    CHECK ((status = 'decided') = (decision IS NOT NULL)),
    CHECK ((decision = 'partial_refund') = (refund_kobo IS NOT NULL))
);
CREATE INDEX disputes_status_idx ON disputes (status, opened_at);
CREATE INDEX disputes_seller_due_idx ON disputes (seller_respond_by)
    WHERE status = 'awaiting_seller';

CREATE TABLE dispute_statements (
    dispute_id uuid NOT NULL REFERENCES disputes (id),
    party      text NOT NULL CHECK (party IN ('buyer', 'seller')),
    body_enc   bytea NOT NULL,
    photo_keys text[] NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (dispute_id, party)
);

-- A partial refund (a dispute decision) keeps part of the money with the seller.
ALTER TABLE refunds
    ADD COLUMN kind text NOT NULL DEFAULT 'full' CHECK (kind IN ('full', 'partial'));

-- Delivery at checkout (Part 3, Task 6).
-- A delivery option is a server-priced choice the buyer can pick for one store and
-- one address; the client only ever sends its id. It holds no personal data (ids
-- only). Courier options carry the provider's quote (request token, courier and
-- service, what it will charge). Orders snapshot what was chosen, plus the
-- delivery address encrypted with FieldCipher (never plaintext).
CREATE TYPE delivery_method AS ENUM ('courier', 'self');

CREATE TABLE delivery_options (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    buyer_id         uuid NOT NULL REFERENCES users (id),
    store_id         uuid NOT NULL REFERENCES stores (id),
    address_id       uuid NOT NULL REFERENCES buyer_addresses (id),
    method           delivery_method NOT NULL,
    -- What the buyer pays for delivery, in kobo. Courier: the rate's charged amount.
    fee_kobo         bigint NOT NULL CHECK (fee_kobo BETWEEN 0 AND 5000000000),
    -- sha256 hex of the store's sorted (variant_id, quantity) cart lines when quoted;
    -- checkout refuses the option if the cart has changed since.
    cart_fingerprint text NOT NULL CHECK (cart_fingerprint ~ '^[0-9a-f]{64}$'),
    courier_id       text NULL,
    service_code     text NULL,
    request_token    text NULL,
    courier_name     text NULL,
    delivery_eta     text NULL,
    charged_kobo     bigint NULL CHECK (charged_kobo >= 0),
    -- The store (sender) address code the courier quote was made from.
    courier_sender_code bigint NULL,
    promised_date    date NOT NULL,
    expires_at       timestamptz NOT NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT delivery_options_courier_fields CHECK (
        (method = 'courier') = (courier_id IS NOT NULL AND service_code IS NOT NULL
                                AND request_token IS NOT NULL AND charged_kobo IS NOT NULL
                                AND courier_sender_code IS NOT NULL)
    )
);
CREATE INDEX delivery_options_buyer_idx ON delivery_options (buyer_id, expires_at);
-- Reusing a still-valid quote, and the "quote already used" check (one quote = one token).
CREATE INDEX delivery_options_quote_idx
    ON delivery_options (buyer_id, store_id, address_id, cart_fingerprint, created_at);

-- NULL delivery_method: an order from before delivery existed (part 2).
ALTER TABLE orders
    ADD COLUMN delivery_method       delivery_method NULL,
    ADD COLUMN delivery_option_id    uuid NULL REFERENCES delivery_options (id),
    -- FieldCipher-encrypted JSON of the DeliveryAddress chosen at checkout.
    ADD COLUMN delivery_address_enc  bytea NULL,
    ADD COLUMN delivery_area         area_council NULL,
    -- Shown at checkout, recomputed from paid_at when the payment settles (never earlier).
    ADD COLUMN promised_date         date NULL,
    ADD COLUMN courier_request_token text NULL,
    ADD COLUMN courier_id            text NULL,
    ADD COLUMN service_code          text NULL,
    ADD COLUMN courier_quote_kobo    bigint NULL CHECK (courier_quote_kobo >= 0),
    -- The quote's sender address code: booking refuses it if the store has moved since.
    ADD COLUMN courier_sender_code   bigint NULL,
    ADD CONSTRAINT orders_delivery_details CHECK (
        delivery_method IS NULL
        OR (delivery_address_enc IS NOT NULL AND delivery_area IS NOT NULL
            AND promised_date IS NOT NULL)
    ),
    ADD CONSTRAINT orders_courier_details CHECK (
        (delivery_method IS DISTINCT FROM 'courier')
        OR (courier_request_token IS NOT NULL AND courier_id IS NOT NULL
            AND service_code IS NOT NULL AND courier_quote_kobo IS NOT NULL
            AND courier_sender_code IS NOT NULL)
    );
CREATE INDEX orders_delivery_option_idx ON orders (delivery_option_id)
    WHERE delivery_option_id IS NOT NULL;

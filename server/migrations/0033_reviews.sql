-- Reviews (spec section 9). Written to be public, so the text is plaintext.
-- `buyer_id` and `order_id` are never served: the public list shows neither.
-- Personal data: the buyer's id, and whatever they chose to write. Moderation
-- has already kept contact details out of `body` and `reply`.

CREATE TABLE reviews (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    order_id   uuid NOT NULL UNIQUE REFERENCES orders (id),
    store_id   uuid NOT NULL REFERENCES stores (id),
    buyer_id   uuid NOT NULL REFERENCES users (id),
    stars      smallint NOT NULL CHECK (stars BETWEEN 1 AND 5),
    body       text,
    photo_keys text[] NOT NULL DEFAULT '{}',
    reply      text,
    replied_at timestamptz,
    hidden_at  timestamptz,
    hidden_by  uuid REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((reply IS NULL) = (replied_at IS NULL))
);
CREATE INDEX reviews_store_idx ON reviews (store_id, created_at DESC, id DESC)
    WHERE hidden_at IS NULL;

-- `reviews::service::recount` is the only writer of these two.
ALTER TABLE stores
    ADD COLUMN rating_count int NOT NULL DEFAULT 0,
    ADD COLUMN rating_sum bigint NOT NULL DEFAULT 0;

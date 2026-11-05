-- Buyer and seller chat (spec section 7).
-- Personal data: `chat_messages.body_enc` is what a person wrote, encrypted with
-- FieldCipher. A withheld message keeps no body at all: its text lives only in
-- the encrypted flag. Retention (12 months, longer in a dispute) is plan 8's
-- purge job.

CREATE TABLE conversations (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    buyer_id        uuid NOT NULL REFERENCES users (id),
    store_id        uuid NOT NULL REFERENCES stores (id),
    product_id      uuid REFERENCES products (id),
    order_id        uuid REFERENCES orders (id),
    last_seq        bigint NOT NULL DEFAULT 0,
    buyer_read_seq  bigint NOT NULL DEFAULT 0,
    seller_read_seq bigint NOT NULL DEFAULT 0,
    -- Starts at creation, so a new conversation sorts first and pages stably.
    last_message_at timestamptz NOT NULL DEFAULT now(),
    created_at      timestamptz NOT NULL DEFAULT now(),
    CHECK ((product_id IS NULL) <> (order_id IS NULL))
);
CREATE UNIQUE INDEX conversations_product_idx ON conversations (buyer_id, product_id)
    WHERE product_id IS NOT NULL;
CREATE UNIQUE INDEX conversations_order_idx ON conversations (order_id)
    WHERE order_id IS NOT NULL;
CREATE INDEX conversations_buyer_idx ON conversations (buyer_id, last_message_at DESC, id DESC);
CREATE INDEX conversations_store_idx ON conversations (store_id, last_message_at DESC, id DESC);

CREATE TABLE chat_messages (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    conversation_id   uuid NOT NULL REFERENCES conversations (id),
    sender_id         uuid NOT NULL REFERENCES users (id),
    client_message_id uuid NOT NULL,
    status            text NOT NULL CHECK (status IN ('held', 'delivered', 'withheld')),
    seq               bigint,
    body_enc          bytea,
    flag_id           uuid REFERENCES moderation_flags (id),
    attempts          int NOT NULL DEFAULT 0,
    next_attempt_at   timestamptz,
    created_at        timestamptz NOT NULL DEFAULT now(),
    delivered_at      timestamptz,
    UNIQUE (sender_id, client_message_id),
    UNIQUE (conversation_id, seq),
    CHECK ((status IN ('held', 'delivered')) = (body_enc IS NOT NULL)),
    CHECK ((status = 'delivered') = (seq IS NOT NULL))
);
CREATE INDEX chat_messages_held_idx ON chat_messages (next_attempt_at) WHERE status = 'held';
CREATE INDEX chat_messages_sender_idx ON chat_messages (sender_id, created_at);
CREATE INDEX chat_messages_own_idx ON chat_messages (conversation_id, sender_id, created_at)
    WHERE status <> 'delivered';

ALTER TABLE moderation_flags
    ADD CONSTRAINT moderation_flags_conversation_fk
    FOREIGN KEY (conversation_id) REFERENCES conversations (id);

-- Notification outbox, inbox and push devices (plan 5).
-- Personal data: a push token is a device identifier, so it is personal data and
-- is stored encrypted (`token_enc`); `token_idx` is an HMAC blind index used only
-- to find a token again. The inbox holds ids and kinds only: no text, no names,
-- no amounts. Retention of both goes to the DPO's list (plan 8 purges).

CREATE TABLE notifications (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         uuid NOT NULL REFERENCES users (id),
    kind            text NOT NULL CHECK (kind IN
        ('order_paid', 'order_picked_up', 'order_out_for_delivery', 'order_delivered',
         'confirm_reminder', 'release_tomorrow', 'refund_started', 'funds_released',
         'chat_message', 'dispute_opened', 'return_requested', 'dispute_decided',
         'moderation_warning', 'moderation_muted')),
    order_id        uuid REFERENCES orders (id),
    dispute_id      uuid REFERENCES disputes (id),
    conversation_id uuid REFERENCES conversations (id),
    dedupe_key      text NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    read_at         timestamptz,
    push_status     text NOT NULL DEFAULT 'pending' CHECK (push_status IN
        ('pending', 'sending', 'sent', 'failed', 'skipped')),
    push_attempts   int NOT NULL DEFAULT 0,
    next_push_at    timestamptz NOT NULL DEFAULT now(),
    push_until      timestamptz,
    pushed_devices  uuid[] NOT NULL DEFAULT '{}',
    UNIQUE (user_id, dedupe_key)
);
CREATE INDEX notifications_inbox_idx ON notifications (user_id, created_at DESC, id DESC);
CREATE INDEX notifications_push_due_idx ON notifications (next_push_at)
    WHERE push_status = 'pending';
CREATE INDEX notifications_push_sending_idx ON notifications (push_until)
    WHERE push_status = 'sending';

CREATE TABLE push_devices (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         uuid NOT NULL REFERENCES users (id),
    session_id      uuid NOT NULL REFERENCES sessions (id),
    platform        text NOT NULL CHECK (platform IN ('android', 'ios')),
    token_enc       bytea NOT NULL,
    token_idx       bytea NOT NULL UNIQUE,
    created_at      timestamptz NOT NULL DEFAULT now(),
    registered_at   timestamptz NOT NULL DEFAULT now(),
    disabled_at     timestamptz,
    disabled_reason text
);
CREATE INDEX push_devices_user_idx ON push_devices (user_id);
CREATE INDEX push_devices_session_idx ON push_devices (session_id);

ALTER TABLE orders
    ADD COLUMN confirm_reminders_sent smallint NOT NULL DEFAULT 0
        CHECK (confirm_reminders_sent BETWEEN 0 AND 2);

-- Store staff, branches and branch stock (plan 7e-1, task 1).
-- Personal data: an invite holds a phone blind index (HMAC, to match the invitee at
-- sign-up) and its last four digits for the owner's list; never the number itself.
-- A branch's pickup address and business phone are copied as FieldCipher ciphertext,
-- never decrypted here. Membership and invite rows are kept (soft-removed) so the
-- audit trail stays whole; retention goes to the DPO's list.

CREATE TYPE store_member_role AS ENUM ('manager', 'staff');
ALTER TYPE delivery_method ADD VALUE 'collect';

-- One row per place a store hands goods over. Columns and checks copy the store's
-- own pickup fields (0004, 0024); the store's copy stays the display default.
CREATE TABLE store_branches (
    id                          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    store_id                    uuid NOT NULL REFERENCES stores (id),
    name                        text NOT NULL,
    pickup_address_enc          bytea NOT NULL,
    pickup_area_council         area_council NULL,
    pickup_postcode             text NULL CHECK (pickup_postcode ~ '^FC[0-9]{2}[A-Z0-9]{7}$'),
    business_phone_enc          bytea NULL,
    self_delivery_fee_kobo      bigint NULL CHECK (self_delivery_fee_kobo BETWEEN 0 AND 5000000),
    pickup_courier_address_code bigint NULL,
    collect_enabled             boolean NOT NULL DEFAULT false,
    position                    integer NOT NULL CHECK (position >= 0),
    closed_at                   timestamptz NULL,
    created_at                  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT store_branches_position_unique UNIQUE (store_id, position)
);

CREATE TABLE store_members (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    store_id   uuid NOT NULL REFERENCES stores (id),
    user_id    uuid NOT NULL REFERENCES users (id),
    role       store_member_role NOT NULL,
    branch_id  uuid NULL REFERENCES store_branches (id),
    added_by   uuid NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    removed_at timestamptz NULL,
    removed_by uuid NULL REFERENCES users (id),
    CONSTRAINT store_members_branch_only_for_staff CHECK (role = 'staff' OR branch_id IS NULL)
);
CREATE UNIQUE INDEX store_members_live_idx ON store_members (store_id, user_id)
    WHERE removed_at IS NULL;
CREATE INDEX store_members_user_idx ON store_members (user_id) WHERE removed_at IS NULL;

CREATE TABLE store_invites (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    store_id    uuid NOT NULL REFERENCES stores (id),
    phone_idx   bytea NOT NULL,
    phone_last4 text NOT NULL CHECK (phone_last4 ~ '^[0-9]{4}$'),
    role        store_member_role NOT NULL,
    branch_id   uuid NULL REFERENCES store_branches (id),
    invited_by  uuid NOT NULL REFERENCES users (id),
    created_at  timestamptz NOT NULL DEFAULT now(),
    expires_at  timestamptz NOT NULL,
    accepted_at timestamptz NULL,
    declined_at timestamptz NULL,
    revoked_at  timestamptz NULL,
    CONSTRAINT store_invites_branch_only_for_staff CHECK (role = 'staff' OR branch_id IS NULL)
);
CREATE INDEX store_invites_phone_live_idx ON store_invites (phone_idx)
    WHERE accepted_at IS NULL AND declined_at IS NULL AND revoked_at IS NULL;
CREATE INDEX store_invites_store_idx ON store_invites (store_id, created_at DESC);

-- On-hand and reserved units per branch and variant. product_variants keeps its own
-- counts until the stock module (task 5) makes this table the only source.
CREATE TABLE branch_stock (
    branch_id         uuid NOT NULL REFERENCES store_branches (id),
    variant_id        uuid NOT NULL REFERENCES product_variants (id) ON DELETE CASCADE,
    stock             integer NOT NULL,
    reserved_quantity integer NOT NULL DEFAULT 0,
    PRIMARY KEY (branch_id, variant_id),
    CONSTRAINT branch_stock_bounds CHECK (0 <= reserved_quantity AND reserved_quantity <= stock AND stock <= 100000)
);
CREATE INDEX branch_stock_variant_idx ON branch_stock (variant_id);

-- Data move: every store gets one branch (position 0) from its own pickup fields.
INSERT INTO store_branches (store_id, name, pickup_address_enc, pickup_area_council,
                            pickup_postcode, business_phone_enc, self_delivery_fee_kobo,
                            pickup_courier_address_code, position)
SELECT id, name, pickup_address_enc, pickup_area_council, pickup_postcode,
       business_phone_enc, self_delivery_fee_kobo, pickup_courier_address_code, 0
FROM stores;

INSERT INTO branch_stock (branch_id, variant_id, stock, reserved_quantity)
SELECT b.id, v.id, v.stock, v.reserved_quantity
FROM product_variants v
JOIN products p ON p.id = v.product_id
JOIN store_branches b ON b.store_id = p.store_id AND b.position = 0;

ALTER TABLE orders ADD COLUMN branch_id uuid NULL REFERENCES store_branches (id);
UPDATE orders o SET branch_id = b.id
FROM store_branches b WHERE b.store_id = o.store_id AND b.position = 0;
ALTER TABLE orders ALTER COLUMN branch_id SET NOT NULL;
CREATE INDEX orders_branch_idx ON orders (branch_id, created_at);

ALTER TABLE delivery_options
    ADD COLUMN branch_id uuid NULL REFERENCES store_branches (id),
    ADD COLUMN group_fingerprint text NULL;
UPDATE delivery_options d SET branch_id = b.id
FROM store_branches b WHERE b.store_id = d.store_id AND b.position = 0;
ALTER TABLE delivery_options ALTER COLUMN branch_id SET NOT NULL;

ALTER TABLE platform_settings
    ADD COLUMN collect_window_days integer NOT NULL DEFAULT 5
        CHECK (collect_window_days BETWEEN 1 AND 14);

ALTER TABLE notifications DROP CONSTRAINT notifications_kind_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check CHECK (kind IN
    ('order_paid', 'order_picked_up', 'order_out_for_delivery', 'order_delivered',
     'confirm_reminder', 'release_tomorrow', 'refund_started', 'funds_released',
     'chat_message', 'dispute_opened', 'return_requested', 'dispute_decided',
     'moderation_warning', 'moderation_muted',
     'store_invite', 'order_new', 'order_ready_for_collection', 'collect_reminder',
     'collection_expired'));

-- What a buyer can get: units across open branches, and the most any one branch holds.
CREATE VIEW variant_availability AS
SELECT bs.variant_id,
       sum(bs.stock - bs.reserved_quantity)::bigint AS total_available,
       max(bs.stock - bs.reserved_quantity)::bigint AS max_available
FROM branch_stock bs
JOIN store_branches b ON b.id = bs.branch_id AND b.closed_at IS NULL
GROUP BY bs.variant_id;

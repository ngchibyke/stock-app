-- Seed for the market wire check (test/wire/market_wire_test.dart).
--
-- One active store, `flow-test-boutique`, in the fashion district on plot
-- `fashion-1`, with three published products; the
-- Ankara dress comes in two sizes. Every name is invented and there is no
-- personal data. Safe to apply more than once: fixed ids, ON CONFLICT DO NOTHING.
--
-- Apply to a SCRATCH database only, never the development one:
--   docker exec -i market-db psql -U market -d <scratch db> < test/wire/fixtures/market_seed.sql
--
-- ⚠️ The encrypted columns (the owner's phone, the store's pickup address)
-- hold a dummy value, because real ciphertext needs the server's FIELD_KEY.
-- The public catalog and the cart never read them; anything that does
-- (checkout, delivery, the seller's own pages) will fail on this store. Do not
-- use this seed for those.

BEGIN;

INSERT INTO users (id, phone_enc, phone_idx, display_name, password_hash, is_seller, phone_verified_at)
VALUES (
  '0b1e5eed-0000-4000-8000-000000000001',
  '\x00'::bytea,
  decode('f10e7e57000000000000000000000000000000000000000000000000000001', 'hex'),
  'Flow Test Seller',
  '!seed-account-cannot-sign-in',
  true,
  now()
)
ON CONFLICT DO NOTHING;

INSERT INTO stores (id, owner_id, name, slug, description, pickup_address_enc,
                    pickup_state, pickup_area_council, pickup_postcode,
                    delivery_courier, delivery_self, status, district)
VALUES (
  '0b1e5eed-0000-4000-8000-000000000010',
  '0b1e5eed-0000-4000-8000-000000000001',
  'Flow Test Boutique',
  'flow-test-boutique',
  'A made-up store for the market wire check.',
  '\x00'::bytea,
  'FCT', 'amac', 'FC00TEST001',
  true, false, 'active', 'fashion'
)
ON CONFLICT DO NOTHING;

-- The map: an active store stands on a plot (see server/market/layout.json).
INSERT INTO store_plots (store_id, plot_id)
VALUES ('0b1e5eed-0000-4000-8000-000000000010', 'fashion-1')
ON CONFLICT DO NOTHING;

INSERT INTO products (id, store_id, name, description, category, brand, condition,
                      photo_keys, weight_grams, base_price_kobo, is_published)
VALUES
  ('0b1e5eed-0000-4000-8000-000000000101', '0b1e5eed-0000-4000-8000-000000000010',
   'Flowtest Ankara Dress', 'Wax print, knee length.', 'fashion', 'Flow Wears', 'new',
   ARRAY['seed/flowtest-dress.jpg'], 400, 1500000, true),
  ('0b1e5eed-0000-4000-8000-000000000102', '0b1e5eed-0000-4000-8000-000000000010',
   'Flowtest Head Tie', 'Gele, one size.', 'fashion', NULL, 'new',
   ARRAY['seed/flowtest-gele.jpg'], 150, 450000, true),
  ('0b1e5eed-0000-4000-8000-000000000103', '0b1e5eed-0000-4000-8000-000000000010',
   'Flowtest Leather Sandals', 'Hand-stitched.', 'fashion', NULL, 'new',
   ARRAY['seed/flowtest-sandals.jpg'], 600, 900000, true)
ON CONFLICT DO NOTHING;

INSERT INTO product_variants (id, product_id, attributes, stock, price_kobo)
VALUES
  ('0b1e5eed-0000-4000-8000-000000001011', '0b1e5eed-0000-4000-8000-000000000101',
   '{"Size": "M"}', 8, NULL),
  ('0b1e5eed-0000-4000-8000-000000001012', '0b1e5eed-0000-4000-8000-000000000101',
   '{"Size": "L"}', 3, 1700000),
  ('0b1e5eed-0000-4000-8000-000000001021', '0b1e5eed-0000-4000-8000-000000000102',
   '{}', 20, NULL),
  ('0b1e5eed-0000-4000-8000-000000001031', '0b1e5eed-0000-4000-8000-000000000103',
   '{}', 5, NULL)
ON CONFLICT DO NOTHING;

COMMIT;

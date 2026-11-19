-- Product feed and search (plan 7, S1).
-- Trigram indexes make `ILIKE '%text%'` on a name or brand fast.
CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE INDEX products_name_trgm_idx ON products USING gin (lower(name) gin_trgm_ops)
    WHERE deleted_at IS NULL AND is_published;
CREATE INDEX products_brand_trgm_idx ON products USING gin (lower(brand) gin_trgm_ops)
    WHERE deleted_at IS NULL AND is_published AND brand IS NOT NULL;
-- The feed's default order, and its price orders, for the keyset paging.
CREATE INDEX products_feed_idx ON products (created_at DESC, id DESC)
    WHERE deleted_at IS NULL AND is_published;
CREATE INDEX products_feed_price_idx ON products (base_price_kobo, id)
    WHERE deleted_at IS NULL AND is_published;

-- A fixed list the server owns (spec 15 leaves the structure to the builder). Products keep
-- their free-text `category`, so nothing already stored is invalidated; the feed matches it
-- to a category's slug or name ignoring case, and the app's seller screens offer this list.
CREATE TABLE categories (
    slug     text PRIMARY KEY CHECK (slug ~ '^[a-z0-9-]{1,40}$'),
    name     text NOT NULL CHECK (length(name) BETWEEN 1 AND 60),
    position integer NOT NULL UNIQUE
);
INSERT INTO categories (slug, name, position) VALUES
    ('fashion', 'Fashion', 1),
    ('phones-tablets', 'Phones & tablets', 2),
    ('electronics', 'Electronics', 3),
    ('home-kitchen', 'Home & kitchen', 4),
    ('beauty-health', 'Beauty & health', 5),
    ('groceries', 'Groceries', 6),
    ('baby-kids', 'Baby & kids', 7),
    ('sports-fitness', 'Sports & fitness', 8),
    ('books-stationery', 'Books & stationery', 9),
    ('automotive', 'Automotive', 10),
    ('other', 'Other', 11);

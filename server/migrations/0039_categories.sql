-- Product categories (plan 7a, task 1): a fixed list the app can show, so a
-- buyer can browse by category and a seller cannot invent one. Owner-editable
-- from the dashboard later; the seed is the starting list.
--
-- No personal data.

CREATE TABLE categories (
    slug       text PRIMARY KEY CHECK (slug ~ '^[a-z0-9-]{2,40}$'),
    name       text NOT NULL,
    position   int  NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO categories (slug, name, position) VALUES
    ('fashion', 'Fashion', 1),
    ('phones-gadgets', 'Phones and gadgets', 2),
    ('electronics', 'Electronics', 3),
    ('home-kitchen', 'Home and kitchen', 4),
    ('beauty-personal-care', 'Beauty and personal care', 5),
    ('baby-kids', 'Baby and kids', 6),
    ('sports-fitness', 'Sports and fitness', 7),
    ('books-stationery', 'Books and stationery', 8),
    ('food-groceries', 'Food and groceries', 9),
    ('other', 'Other', 10);

-- Old products held free text. A value that is a slug or a name (any case, any
-- spacing) keeps its category; anything else becomes `other`. Kept as a
-- function so the rule is tested.
CREATE FUNCTION category_slug_for(raw text) RETURNS text
LANGUAGE sql STABLE AS $$
    SELECT COALESCE(
        (SELECT slug FROM categories
          WHERE slug = lower(btrim(raw)) OR lower(name) = lower(btrim(raw))
          ORDER BY position LIMIT 1),
        'other')
$$;

UPDATE products SET category = category_slug_for(category)
 WHERE category NOT IN (SELECT slug FROM categories);

ALTER TABLE products
    ADD CONSTRAINT products_category_fk FOREIGN KEY (category) REFERENCES categories (slug);

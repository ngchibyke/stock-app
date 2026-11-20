-- Catalog search (plan 7a, task 2): trigram matching so a buyer's typo ("sandle")
-- still finds the product ("sandal"), on the text the search reads.
--
-- The index expression must match `DOC` in `src/catalog/browse.rs` exactly, or
-- the planner cannot use it: change both together.
--
-- `pg_trgm` is a standard Postgres extension. The role that runs migrations
-- needs permission to create it, or the database owner creates it first.
--
-- No personal data.

CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE INDEX products_search_idx ON products
    USING gin ((lower(name || ' ' || coalesce(brand, '') || ' ' || category)) gin_trgm_ops)
    WHERE is_published AND deleted_at IS NULL;

-- Stock per branch (plan 7e-1, task 5). Personal data: none.
-- branch_stock is the truth; product_variants.stock and reserved_quantity are totals
-- over a store's open branches, written only by src/stock.rs. The 100,000 cap
-- (src/limits.rs MAX_STOCK) now holds per branch (branch_stock_bounds), so a total
-- may reach 20 branches x 100,000.
ALTER TABLE product_variants DROP CONSTRAINT product_variants_stock_cap;
ALTER TABLE product_variants
    ADD CONSTRAINT product_variants_stock_cap CHECK (stock <= 2000000);

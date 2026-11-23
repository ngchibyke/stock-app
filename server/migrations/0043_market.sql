-- The market map: every store belongs to a district and, once active, holds one plot.
ALTER TABLE stores ADD COLUMN district text NOT NULL DEFAULT 'everything-else';

CREATE TABLE store_plots (
    store_id  uuid PRIMARY KEY REFERENCES stores(id),
    plot_id   text NOT NULL UNIQUE,
    placed_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX stores_district_idx ON stores (district) WHERE deleted_at IS NULL;

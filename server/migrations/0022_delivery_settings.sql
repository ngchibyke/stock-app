-- Delivery settings (Part 3, Task 1).
-- The six FCT area councils; the only places we deliver to. Stored as an enum so
-- an address or store cannot carry any other value.
CREATE TYPE area_council AS ENUM ('amac', 'bwari', 'gwagwalada', 'kuje', 'kwali', 'abaji');

ALTER TABLE platform_settings
    -- Minutes after local midnight (Africa/Lagos). Paid at or after this minute
    -- counts as after the cut-off. Default 18:00.
    ADD COLUMN delivery_cutoff_minutes     integer NOT NULL DEFAULT 1080
        CHECK (delivery_cutoff_minutes BETWEEN 0 AND 1439),
    -- How long a delivery option shown at checkout can be used.
    ADD COLUMN courier_quote_ttl_minutes   integer NOT NULL DEFAULT 30
        CHECK (courier_quote_ttl_minutes BETWEEN 5 AND 1440),
    -- Accepted rise of the booked price over the quote, in basis points.
    ADD COLUMN courier_price_tolerance_bps integer NOT NULL DEFAULT 1000
        CHECK (courier_price_tolerance_bps BETWEEN 0 AND 5000),
    -- One platform-wide Shipbubble category (default: "Accessories").
    ADD COLUMN courier_category_id         bigint NOT NULL DEFAULT 90097994
        CHECK (courier_category_id > 0);

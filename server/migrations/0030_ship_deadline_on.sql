-- Task 12: the ship-deadline job goes live now that pickup is tracked.
--
-- `ship_deadline_enforce_from` is the safety rail for turning it on: only
-- orders paid at or after it can be refunded or struck, so the backlog of
-- orders paid before this migration (including part-2 orders with no delivery
-- method) is never mass-refunded. The migration sets it to now(); the owner
-- can only move it later through the settings route (audited), never earlier,
-- and the route refuses a value more than 30 days ahead.
-- Personal data: none.

ALTER TABLE platform_settings
    ADD COLUMN ship_deadline_enforce_from timestamptz NOT NULL DEFAULT now();

ALTER TABLE platform_settings
    ALTER COLUMN jobs_ship_deadline_enabled SET DEFAULT true;
UPDATE platform_settings SET jobs_ship_deadline_enabled = true;

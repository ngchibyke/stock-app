# Market layout

The map the server places stores on, and the rooms they open into. Loaded at start-up from
`MARKET_LAYOUT_DIR` (default `market`, relative to the working directory). A layout that is
missing or unusable (no plots, duplicate ids, plots outside the world or overlapping, a plot
naming an unknown district, a district naming an unknown or slotless room type, a slot outside
0 to 1) stops the server, for every command.

## How the files are made

They are copies of the app's placeholder art layout in `app/assets/market/` (`layout.json` and
`rooms/<type>/room.json`), with one addition: each district carries `store_type`, the room type
its stores get. The app ignores that key. The test `layout_matches_the_apps_bundled_art`
(`server/tests/market_layout.rs`) compares the two, minus `store_type`, and fails if they drift.
It reads `../app/assets/market`, so it needs the whole repository checked out.

If you change one side, change the other the same way. Real art (7f) will replace both.

## Adding plots or a district

- A plot: add an entry to `plots` in `layout.json` (unique `id`, an existing `district`, inside
  the world, not overlapping another plot). Do the same in the app's copy.
- A district: add it to `districts` with a `store_type` that has a folder under `rooms/`, then
  give it plots. The app's copy gets the district without `store_type`.
- Existing stores keep their plots. A new plot is picked up by waiting stores at the next
  start-up, or when a plot in that district is released.

## Placement

A store gets a plot when it first becomes active: the first free plot in its district, in the
order the plots appear in `layout.json`. Reordering plots changes who gets which plot next, not
who already has one.

When a district is full the store stays active and listed everywhere except the map, and
`market.district_full` is written to the audit log. It takes the next plot that frees up
(oldest waiting store first), or one at the next start-up.

# Part 7d: Server additions for the immersive market (design)

Status: draft for the owner's approval, 2026-11-23. Builds on `2026-11-21-immersive-market-design.md` (sections 7, 8, 10) and the app plan `plans/2026-11-23-07c-immersive-market.md`, which fixed the JSON contract this piece implements.

## Intent

The app already draws the whole market from a placeholder source. 7d makes the server answer for real, so the placeholder can go: which store sits on which plot of the map, what its room looks like, and which of its products stand on the shelves. Buyers in Abuja then see the real stores in a market that stays in step as stores join, sell out, get suspended or leave.

## Goals

1. `GET /market` returns the map (districts, plots, tile settings) and the stores that hold plots, in 7c's `MarketJson` shape.
2. `GET /stores/{slug}/room` returns the store's room type, its shelf slots, and the products placed in them, in 7c's `RoomJson` shape.
3. `GET /app/config` gains an `assets` block (`{version, base_url}`) so art can change without an app release.
4. A seller chooses their store's district when creating it; the server gives every active store a plot automatically.
5. The app's placeholder source is replaced by one that calls these routes, proving the contract end to end.

## Non-goals

- Real art, tiles and depth maps (7f). Until then `assets` is null and the app uses its bundled placeholder art.
- Checkout, orders, product upload screens, chat (7e).
- Dashboard screens for moving stores between plots. Plots move only by the seller changing district.
- Letting a seller reorder or pin shelf products. Shelves fill by a fixed rule (below).
- Price tracking, cut-outs, tilt.

## Anti-goals (a failure even if it technically works)

- A room that shows something that cannot be bought (unpublished, sold out, store suspended).
- Two stores on one plot, or a store that vanishes from the market without the owner knowing why.
- Anything about a person in the market, room or asset data, or in logs.
- A read route that writes to the database.

## Constraints

- Conventions of the existing server hold: the `AppError` envelope, `AppQuery`, `parse_id`, an append-only `audit_log` entry for every state change with no personal data in `detail`, no network call inside a database transaction.
- Lock order gains one step: users, stores, **store_plots**, products, variants and so on (plots are locked right after the store they belong to).
- Public reads show only active, non-deleted stores and published, non-deleted, in-stock products, the same visibility rule as the catalog.
- Tests run from `server/` against the `market-db` container (start it if stopped), never two `cargo test` at once.
- No AI attribution in commits or PRs.

## Approaches considered

**Where the map lives.**
- A. All in database tables, editable later in the dashboard. Flexible, but every new art version needs a migration to keep plots and slots in step with the picture.
- **B. The layout in a versioned file, placements in the database. Chosen by the owner.** The plot map and shelf slots are JSON files that ship with the art version, loaded and checked at startup. The database holds only each store's district and which plot it holds. Art and map cannot drift apart, and swapping in real art is a version bump. Adding plots means shipping a new layout file.
- C. Constants in Rust code. Quickest, but every art change is a rebuild and the data is hard to test.

**Who decides a store's place.** The seller picks (chosen by the owner), over the server inferring it from products (a store would move as its stock changes, and a new store has nothing to infer from) or the owner assigning by hand (does not scale).

## The design

### Layout files (the source of truth for the map)

A directory `server/market/` holding the same JSON the app bundles: `layout.json` (version, world, tile size, max level, base image, districts, plots) and `rooms/{type}/room.json` (room and depth image names, light, slots). One addition to 7c's shape: each district also carries `store_type`, the room type its stores get. The app ignores the extra key. Path comes from `MARKET_LAYOUT_DIR` (default `market/` beside the binary's working directory). The server refuses to start if the layout is unusable: no plots, duplicate ids, plots outside the world or overlapping, a plot naming an unknown district, a district naming an unknown or slotless room type, a slot outside 0 to 1. The files are placeholder-art layouts today and are replaced with the real ones in 7f.

### Database

- `stores.district text NOT NULL` (an id from the layout; the layout is checked at write time, not by a foreign key). Existing stores are backfilled to `everything-else`.
- `store_plots (store_id uuid PRIMARY KEY REFERENCES stores, plot_id text NOT NULL UNIQUE, placed_at timestamptz)`. The unique plot column is what makes a double placement impossible.

### Choosing a district

`POST /stores` takes a required `district` (422 `unknown_district` if not in the layout). `PATCH /stores/{id}` may change it. The list of choices is `districts` in `GET /market`, shown to the seller as "What do you mainly sell?". Because three districts share the boutique room, the seller picks the district and the room type follows from it (this is a refinement of "seller picks the type": one field, no ambiguity).

### Placement

- A store gets a plot the first time it becomes `active` (the existing status sync after payout-account verification, and unsuspend leaves its plot untouched): the first free plot in its district, in layout order, inside the same transaction. District full means no plot: the store stays active and listed everywhere except the map, and a plain audit entry `market.district_full` records it.
- A start-up pass places every active store that has no plot (this is also the backfill for stores that exist today), oldest store first. It is idempotent and safe to run on every start.
- Changing district releases the old plot and takes the first free one in the new district; if none is free the store is unplaced. Soft-deleting a store releases its plot. A suspended store keeps its plot but is not shown.
- Concurrent placements in one district are serialised with a per-district advisory lock, with the unique plot column as the backstop.
- Every placement and release is audited (`market.store_placed`, `market.store_unplaced`) with store id, plot id and district only.

### `GET /market`

Public, no auth. Answers the layout (as in `layout.json`) plus `placements: [{plot, store_type, store: StoreCard}]` for active, non-deleted stores that hold a plot; `StoreCard` is the shape the store directory already returns. Short public cache plus an ETag so the app can revalidate cheaply. Read-only.

### `GET /stores/{slug}/room`

Public for active stores; the owner may also open their own store before it is public (so a seller can see their goods early; the screen comes in 7e). Anyone else gets 404 `store_not_found`, including for suspended and deleted stores. Answers `{store, store_type, placed, has_more}`.

Which products stand on the shelves: the store's published, non-deleted products with at least one variant that has stock (`stock - reserved_quantity > 0`), newest first by creation time (stable: shelves do not shuffle when a price is edited), one per slot in slot order, up to the room type's slot count. `has_more` is true when more qualify than there are slots, and the app then links to the list view. Stock is read at request time, so sold-out items drop off the next time the room opens. Short cache.

### `GET /app/config` assets

`assets: {version, base_url}` from `MARKET_ASSETS_VERSION` and `MARKET_ASSETS_BASE_URL`; both set or both unset (unset gives `null`); the base URL must be https; checked at startup like the other `APP_*` settings. Nothing here is per user.

### App side (kept small)

An `ApiMarketSource` calls `/market` and `/stores/{slug}/room` and replaces the placeholder as the default `marketSourceProvider`; the placeholder source stays only as a test fake (the existing disk cache already covers offline use). The app's wire check gets a market seed on a scratch database. The store-creation screen itself is 7e.

### Data protection

Nothing personal is in the market, rooms or assets, so they are safe to cache and put behind a CDN. A store's district is business information, not personal data. Audit entries carry ids only. No slug, id or search text is newly logged. The store owner's identity is never in a placement or room response. Retention: placements are deleted with the store's soft delete; the audit trail follows the existing audit retention. Not legal advice; the existing NDPA review of the store record covers the new column.

## What is left to the builder

Route module layout and names, the SQL for the in-stock product query (reuse the catalog's in-stock expression), cache lifetimes and ETag details, the advisory-lock key scheme, how the startup pass is wired (a function run before the listener binds), test fixture helpers, and whether a script copies the layout files from `app/assets/market/` into `server/market/` (a diff check in CI that the two stay equal is preferred).

## Calls I made (for the owner to check)

1. The seller picks a **district**, and the room type follows from it, because three districts share one room type.
2. `district` is **required** on store creation and changeable by `PATCH`.
3. Existing stores are all backfilled to `everything-else`.
4. District full means **active but unplaced**, not blocked.
5. Shelves fill **newest created first**, in-stock only.
6. The owner can open their own non-public store's room.
7. Layout lives in `server/market/` with a copy of the app's placeholder files, and the server **refuses to start** on a bad layout.
8. The app switch to the real source is part of 7d, so the contract is proved end to end.

## Review focus

1. **Placement races and leaks:** two stores activating at once in a full or nearly full district; changing district while another store takes the last plot; a deleted or suspended store holding a plot.
2. **Room visibility:** suspended, pending, deleted and other people's stores; unpublished, deleted and sold-out products; a reserved-but-unpaid unit.
3. **Bad layout:** every startup rule above has a test that the server refuses to boot.
4. **Contract match:** server JSON parsed by the app's real parser in the wire check, including a store with no logo, no rating and no products.

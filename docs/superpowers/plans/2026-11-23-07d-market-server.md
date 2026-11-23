# Part 7d: Server Additions for the Immersive Market Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The server answers `GET /market`, `GET /stores/{slug}/room` and an `assets` block in `GET /app/config` in 7c's JSON shapes, places every active store on a plot of the map by the district its seller chose, and the app reads these routes instead of its placeholder.

**Architecture:** A new `market` module loads and checks the layout files (`server/market/`) at startup into an immutable `Layout` carried in `AppState`. Placement is plain SQL over a new `store_plots` table (one row per placed store, unique plot), serialised per district with an advisory lock and run inside the transactions that already change a store's status, district or deletion, plus a startup pass for stores that exist today. The two read routes only read. The app gains an `ApiMarketSource` next to the existing placeholder.

**Tech Stack:** Rust, axum 0.8, sqlx 0.8 on Postgres 17, serde_json, the `sha2` crate already in use (ETag); Flutter 3.50.1, `dio`, `flutter_riverpod`.

**Spec:** `docs/superpowers/specs/2026-11-23-07d-market-server-design.md`. Read with the app plan `docs/superpowers/plans/2026-11-23-07c-immersive-market.md` (the contract: Task 1 and "Server routes this plan uses") and 7a's plan for server conventions (`2026-11-20-07a-app-server-additions.md`, Global Constraints).

## Global Constraints

- 7a's server conventions hold: errors are the `AppError` envelope, queries use `AppQuery`, path ids go through `parse_id`, every state change writes `audit_log` with ids only (never a phone, address, code or signed URL), no network call inside a transaction, personal data never logged or put in a URL.
- **Lock order:** users, stores, **store_plots**, products, variants, access_codes, checkouts, orders, disputes, shipments, payouts, then conversations, reviews, uploads.
- **Public reads show only what is public:** active, non-deleted stores; published, non-deleted products with `stock - reserved_quantity > 0` on at least one variant.
- **The room never shows what cannot be bought.** A suspended, deleted or unknown store's room is 404 `store_not_found` for everyone, owner included; a pending store's room is visible to its owner only.
- **The market, rooms and assets carry nothing personal** (no owner, phone, address). A store's district is business information.
- District ids and room type ids are the layout file's (`fashion`, `phones-electronics`, `home-beauty`, `food-groceries`, `everything-else`; `boutique`, `phone_shop`, `grocery`). Default district for existing stores: `everything-else`.
- Reads never write. `Cache-Control: public, max-age=60` plus an `ETag` on `/market`; `public, max-age=30` on the room.
- Tests run from `server/` (check `df -h` first: `target/` regrows; `~/.cargo/bin/cargo.exe clean` if the disk is nearly full). Start the database first: `docker ps -a`, then `docker start market-db` if stopped. Command: `DATABASE_URL=postgres://market:market@localhost:5433/market cargo test`. Never two `cargo test` at once. CI also runs `cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings`; run both before each commit.
- Flutter checks run from `app/`: `flutter analyze` and `flutter test` clean before each commit.
- Commits: conventional messages; never a `Co-Authored-By` line, a "Generated with" footer, a session link or any AI attribution.

## Review Focus

1. **Placement races.** Two stores activating at once in a district with one free plot; a district change racing the last plot. One wins, the other is active and unplaced, no plot is ever held twice. (Task 2)
2. **A store leaving the map.** Soft delete, a district change and suspension: delete frees the plot and the oldest waiting store in that district takes it; suspension keeps the plot but hides the store. (Tasks 2, 3)
3. **Room visibility.** Suspended, pending (owner vs stranger), deleted and unknown slugs; unpublished, deleted, sold-out and reserved-to-zero products. (Task 5)
4. **A bad layout file.** Each startup rule refuses to boot with a plain message, and the shipped files pass. (Task 1)
5. **Contract drift.** The server's JSON parses in the app's real parsers, for a store with no logo, no rating and no products, and for a market with zero placements. (Tasks 4, 5, 7)

---

## File structure

```
server/
  market/                          the layout files (copied from app/assets/market, plus store_type per district)
    layout.json
    rooms/{boutique,phone_shop,grocery}/room.json
  migrations/0043_market.sql       stores.district, store_plots
  src/market/mod.rs                module root, router()
  src/market/layout.rs             Layout: load and check the files
  src/market/placement.rs          place, release, startup pass
  src/market/routes.rs             GET /market, GET /stores/{key}/room
  src/app_config.rs                gains `assets`
  tests/market_layout.rs, market_placement.rs, market.rs, market_room.rs
app/
  lib/features/market/data/api_market_source.dart
  test/features/market/api_market_source_test.dart
```

---

### Task 0: Branch and baseline

- [ ] **Step 1:** The design and this plan are committed on `docs/spec-7d-market-server` (commit the plan first: `docs: add the part 7d plan`). Create the worktree `.claude/worktrees/market-server` on branch `feat/market-server` from that branch, so the documents travel with the code.
- [ ] **Step 2:** Check free disk (`df -h`), start `market-db`, run the full server suite from `server/`. Expected: all pass (1126 at the end of 7a, one MinIO test skipped). Stop and report if not.
- [ ] **Step 3:** From `app/`: `flutter pub get`, `flutter analyze`, `flutter test`. Expected: clean, 446 tests. Stop if not.

### Task 1: The layout files and `Layout`

**Files:** Create `server/market/layout.json`, `server/market/rooms/{boutique,phone_shop,grocery}/room.json`, `server/src/market/mod.rs`, `server/src/market/layout.rs`; modify `server/src/lib.rs` (`pub mod market;`, `AppState.market: Arc<market::layout::Layout>`), `server/src/config.rs` (`market_layout_dir: PathBuf` from `MARKET_LAYOUT_DIR`, default `market`), `server/src/main.rs`, `server/tests/common/mod.rs` (the harness loads `CARGO_MANIFEST_DIR/market`), `server/.env.example`; test `server/tests/market_layout.rs`.

**Interfaces (Produces):**
```rust
pub struct District { pub id: String, pub name: String, pub store_type: String }
pub struct Plot { pub id: String, pub district: String, pub x: f64, pub y: f64, pub w: f64, pub h: f64 }
pub struct Slot { pub id: String /* + x, y, w, h, depth, tilt_y */ }
pub struct RoomType { pub id: String, pub json: serde_json::Value /* the room.json, verbatim */, pub slots: Vec<Slot> }
pub struct Layout { /* private */ }
impl Layout {
    pub fn load(dir: &Path) -> anyhow::Result<Layout>;
    pub fn layout_json(&self) -> &serde_json::Value;          // layout.json verbatim, store_type included
    pub fn district(&self, id: &str) -> Option<&District>;
    pub fn plots_in<'a>(&'a self, district: &str) -> impl Iterator<Item = &'a Plot>; // file order
    pub fn plot(&self, id: &str) -> Option<&Plot>;
    pub fn room_type(&self, id: &str) -> Option<&RoomType>;
    pub fn room_type_for_district(&self, district: &str) -> Option<&RoomType>;
    pub fn district_ids(&self) -> impl Iterator<Item = &str>;
}
```

- [ ] **Step 1: Failing tests** in `tests/market_layout.rs`, each writing a layout into a temp dir (start from a copy of the shipped files and break one thing): `shipped_layout_loads_with_60_plots_5_districts_3_room_types`; then `load` returns an error whose message names the problem for: `no_plots`, `duplicate_plot_ids`, `duplicate_district_ids`, `plot_outside_the_world`, `overlapping_plots`, `plot_names_unknown_district`, `district_names_unknown_room_type`, `room_type_with_no_slots`, `slot_outside_0_to_1`, `missing_file`, `not_json`; plus `layout_matches_the_apps_bundled_art` (compares `server/market/**` with `../app/assets/market/**` after removing each district's `store_type` key; both layout and the three `room.json`).
- [ ] **Step 2:** Run `cargo test --test market_layout`. Expected: FAIL (no module).
- [ ] **Step 3:** Copy the app's `layout.json` and three `room.json` into `server/market/`; add `"store_type"` to each district (`phones-electronics`: `phone_shop`, `food-groceries`: `grocery`, the other three: `boutique`). Implement `Layout::load` with every check above (strict, unlike the app's tolerant parser). `main.rs` loads it before binding and exits with the error message on failure.
- [ ] **Step 4:** Run the file, then the whole suite, `cargo fmt --check`, clippy. Expected: PASS.
- [ ] **Step 5:** Commit `feat: load and check the market layout at startup`.

### Task 2: Plots and placement

**Files:** Create `server/migrations/0043_market.sql`, `server/src/market/placement.rs`; modify `server/src/main.rs` (startup pass); test `server/tests/market_placement.rs`.

**Interfaces:**
- Consumes: `Layout` (Task 1).
- Produces:
```rust
// migration: ALTER TABLE stores ADD COLUMN district text NOT NULL DEFAULT 'everything-else';
//            CREATE TABLE store_plots (store_id uuid PRIMARY KEY REFERENCES stores(id),
//                                      plot_id text NOT NULL UNIQUE, placed_at timestamptz NOT NULL DEFAULT now());
pub async fn place(tx: &mut Transaction<'_, Postgres>, layout: &Layout, store_id: Uuid) -> Result<Option<String>, AppError>;
//   Idempotent. Only an `active`, non-deleted store is placed. Takes pg_advisory_xact_lock for its district,
//   then the first plot (file order) in that district not in store_plots. Returns the plot id, or None when
//   the store is not eligible or the district is full (audits `market.district_full` once per call).
//   Audits `market.store_placed` {plot, district} on success.
pub async fn release(tx: &mut Transaction<'_, Postgres>, layout: &Layout, store_id: Uuid) -> Result<(), AppError>;
//   Deletes the row (audit `market.store_unplaced`), then hands the freed plot to the oldest active unplaced
//   store of the same district via `place`.
pub async fn place_missing(db: &PgPool, layout: &Layout) -> Result<u64, AppError>;
//   Places every active, non-deleted store with no plot, oldest first; one transaction per store; returns how many.
```

- [ ] **Step 1: Failing tests** in `tests/market_placement.rs` (use `make_active_store_with_product` and direct SQL to set `district`): `an_active_store_gets_the_first_free_plot_of_its_district`; `pending_and_deleted_stores_are_not_placed`; `placing_twice_changes_nothing`; `a_full_district_leaves_the_store_active_and_unplaced` (fill the 8-plot `everything-else` with 8 stores, the 9th is unplaced, audit `market.district_full` exists); `two_stores_racing_for_the_last_plot_one_wins` (`tokio::join!`, exactly one row, plots unique); `releasing_a_plot_hands_it_to_the_oldest_waiting_store`; `place_missing_places_existing_stores_oldest_first_and_is_idempotent`; `unique_plot_is_enforced_by_the_database` (a raw second insert of one plot fails).
- [ ] **Step 2:** Run `cargo test --test market_placement`. Expected: FAIL.
- [ ] **Step 3:** Write the migration (also `CREATE INDEX stores_district_idx ON stores (district) WHERE deleted_at IS NULL`) and implement the three functions. Advisory lock key: `hashtext('market_district:' || district)`. `main.rs` calls `place_missing` before `serve` and logs only the count.
- [ ] **Step 4:** Run the file and the whole suite, fmt, clippy. Expected: PASS.
- [ ] **Step 5:** Commit `feat: place active stores on market plots`.

### Task 3: A seller chooses a district; placement follows the store's life

**Files:** Modify `server/src/stores/service.rs` (`NewStore.district: String`, `StorePatch.district: Option<String>`, `create_store`, `update_store`, `delete_store`, the owner view gains `district`), `server/src/stores/routes.rs` (pass `&state.market`), `server/src/payout_accounts/service.rs` (`sync_store_status` and its callers), `server/src/admin/stores.rs` (`unsuspend`), `server/tests/common/mod.rs` and the 15 existing store-creating tests (add `"district": "everything-else"`); test `server/tests/stores.rs` and `server/tests/market_placement.rs`.

**Interfaces:**
- Consumes: `Layout::district`, `placement::{place, release}`.
- Produces: `POST /stores` requires `district` (422 `unknown_district`, "That district does not exist", when it is not in the layout); `PATCH /stores/{id}` accepts `district`; the owner view (`GET /stores/mine`, create and update answers) includes `"district"`.

- [ ] **Step 1: Failing tests** in `tests/stores.rs`: `create_requires_a_known_district` (missing is 422 from the extractor's validation, unknown is `unknown_district`); `owner_view_includes_the_district`; `patch_can_change_the_district`; `patch_with_an_unknown_district_is_rejected`. In `tests/market_placement.rs`: `verifying_the_payout_account_places_the_store`; `a_store_going_back_to_pending_keeps_its_plot`; `unsuspend_places_a_store_that_waited_for_room` (full district, free a plot, unsuspend, now placed); `changing_district_moves_the_store_to_the_new_district` (old plot freed and given to the next waiting store); `changing_district_to_a_full_one_leaves_it_unplaced`; `deleting_a_store_frees_its_plot`; `suspending_keeps_the_plot`.
- [ ] **Step 2:** Run those files. Expected: FAIL.
- [ ] **Step 3:** Implement. `create_store` and `update_store` validate against `layout.district`. `sync_store_status`: after `("pending", true)` call `place`. `unsuspend`: when `next == "active"` call `place`. `update_store`: when the district changed, `release` the old plot then `place` (inside the store's transaction, after the store row lock). `delete_store`: call `release`. Thread `&Layout` from the routes (`state.market`) into these functions.
- [ ] **Step 4:** Run the whole suite, fmt, clippy. Expected: PASS.
- [ ] **Step 5:** Commit `feat: let a seller choose a district and keep plots in step with the store`.

### Task 4: `GET /market`

**Files:** Create `server/src/market/routes.rs`; modify `server/src/market/mod.rs` (`pub fn router() -> Router<AppState>`), `server/src/lib.rs` (`.merge(market::router())`), `server/src/catalog/stores.rs` (make `StoreCard` and a `fn card_for(..)` or an equivalent query reusable by `market`); test `server/tests/market.rs`.

**Interfaces:**
- Consumes: `Layout::layout_json`, `Layout::room_type_for_district`, `catalog::stores::StoreCard`.
- Produces: `GET /market` answers `{ ...layout.json fields, "placements": [{"plot": id, "store_type": id, "store": StoreCard}] }`; placements are active, non-deleted stores in `store_plots`, ordered by plot file order; `ETag` is a hash of the body and `If-None-Match` with it answers 304 with no body; `Cache-Control: public, max-age=60`.

- [ ] **Step 1: Failing tests** in `tests/market.rs`: `market_answers_layout_and_placements_in_the_contract_shape` (keys `version, world, tile_size, max_level, base_image, districts, plots, placements`; a placement has `plot`, `store_type`, `store.{id,slug,name,logo_url,rating_average,rating_count,product_count}`); `suspended_pending_and_deleted_stores_are_not_on_the_map`; `an_unplaced_active_store_is_not_on_the_map_but_is_in_the_directory`; `market_with_no_stores_has_an_empty_placements_list`; `store_type_follows_the_district` (a `phones-electronics` store has `phone_shop`); `etag_round_trip_gives_304`; `market_needs_no_login`; `market_holds_nothing_about_the_owner` (the body contains no owner id, phone or address field).
- [ ] **Step 2:** Run `cargo test --test market`. Expected: FAIL (404).
- [ ] **Step 3:** Implement `GET /market`. Build the body once per request from the cached layout JSON plus one query; compute the ETag over the serialised body.
- [ ] **Step 4:** Run the file and the whole suite, fmt, clippy. Expected: PASS.
- [ ] **Step 5:** Commit `feat: add GET /market`.

### Task 5: `GET /stores/{key}/room`

**Files:** Modify `server/src/market/routes.rs`; test `server/tests/market_room.rs`.

**Interfaces:**
- Consumes: `stores::routes::MaybeAuth` (already `pub(crate)`), `Layout::room_type_for_district`, the catalog's `ProductCard` JSON and its in-stock expression (`EXISTS (SELECT 1 FROM product_variants v WHERE v.product_id = p.id AND v.stock - v.reserved_quantity > 0)`).
- Produces: `GET /stores/{key}/room` (key is the slug) answers `{ "store": StoreCard, "store_type": <room.json verbatim>, "placed": [{"slot": slot id, "product": ProductCard}], "has_more": bool }`. Eligible products: the store's published, non-deleted products with stock, ordered `created_at DESC, id`; the first N (N = the room type's slot count) go to the slots in file order; `has_more` is true when more qualify. Unknown, deleted and suspended stores (for everyone) and pending stores (for anyone but the owner) answer 404 `store_not_found`.

- [ ] **Step 1: Failing tests** in `tests/market_room.rs`: `room_has_the_contract_shape`; `placed_products_fill_slots_in_order_newest_first`; `unpublished_deleted_and_sold_out_products_are_left_out`; `a_unit_reserved_to_zero_leaves_the_shelf`; `more_products_than_slots_sets_has_more_and_caps_placed`; `a_store_with_no_products_has_an_empty_room`; `a_product_with_no_photo_still_appears` (`photo_url` null); `suspended_store_room_is_404_even_for_the_owner`; `pending_store_room_is_visible_to_the_owner_only`; `deleted_and_unknown_slugs_are_404_with_the_same_body`; `room_type_follows_the_district`; `room_needs_no_login_for_an_active_store`; `editing_a_price_does_not_reorder_the_shelf`.
- [ ] **Step 2:** Run `cargo test --test market_room`. Expected: FAIL.
- [ ] **Step 3:** Implement the route and add it to `market::router()`. One query for the cards (limit N + 1), one for the store card.
- [ ] **Step 4:** Run the file and the whole suite, fmt, clippy. Expected: PASS.
- [ ] **Step 5:** Commit `feat: add GET /stores/{slug}/room`.

### Task 6: The `assets` block in app config

**Files:** Modify `server/src/app_config.rs`, `server/.env.example`; test `server/tests/app_config.rs`.

**Interfaces:** `AppClientConfig` gains `assets: Option<MarketAssets>` with `pub struct MarketAssets { pub version: String, pub base_url: String }`, read from `MARKET_ASSETS_VERSION` and `MARKET_ASSETS_BASE_URL` (both or neither; the base URL passes the existing `https_url` check; the version is 1 to 40 characters of letters, digits, `.`, `-`, `_`). `view()` adds `"assets": null` or `{"version", "base_url"}`.

- [ ] **Step 1: Failing tests:** `assets_are_null_when_unset`; `assets_are_served_when_both_are_set`; `only_one_of_the_two_is_refused`; `a_non_https_base_url_is_refused`; `a_bad_version_is_refused`; the existing `full()` fixture test keeps passing.
- [ ] **Step 2:** Run `cargo test --test app_config`. Expected: FAIL.
- [ ] **Step 3:** Implement in `from_lookup` and `view`.
- [ ] **Step 4:** Run the file and the whole suite, fmt, clippy. Expected: PASS.
- [ ] **Step 5:** Commit `feat: add the asset version and base URL to the app config`.

### Task 7: The app reads the real routes

**Files:** Create `app/lib/features/market/data/api_market_source.dart`; modify `app/lib/features/market/data/market_source.dart` (`marketSourceProvider` builds `ApiMarketSource`), `app/test/wire/market_wire_test.dart`, `app/test/wire/fixtures/market_seed.sql` (each seeded store gets a `district`), `app/README.md`; test `app/test/features/market/api_market_source_test.dart`.

**Interfaces:**
- Consumes: `ApiClient` (`apiClientProvider`), `MarketSnapshot.fromJson(json, fromCache:)`, `RoomView.fromJson`, `StoreType.fromJson`, `JsonCache`, the existing `PlaceholderMarketSource` cache keys' pattern.
- Produces: `class ApiMarketSource implements MarketSource { ApiMarketSource({required ApiClient api, required JsonCache cache, required Future<String> Function(String) loadAsset}) }`. `market()` calls `GET /market`, caches the JSON, and on a network error answers from the cache with `fromCache: true`. `room(slug)` calls `GET /stores/{slug}/room`; an `ApiError` 404 with code `store_not_found` is rethrown as is. `storeType(id)` still reads the bundled `assets/market/rooms/{id}/room.json`. `PlaceholderMarketSource` stays, used only by tests.

- [ ] **Step 1: Failing tests** with the app's fake HTTP adapter: `market_parses_the_servers_answer_including_store_type`; `market_falls_back_to_the_cache_when_offline`; `market_with_zero_placements_parses`; `room_parses_placed_products_and_has_more`; `a_missing_store_throws_store_not_found`; `a_malformed_placement_is_skipped_not_fatal`.
- [ ] **Step 2:** Run `flutter test test/features/market/api_market_source_test.dart`. Expected: FAIL.
- [ ] **Step 3:** Implement `ApiMarketSource` and switch the provider. Update the wire test to use it and to assert the seeded store appears in `placements` and its room returns at least one product; update `market_seed.sql` and the wire-check section of `README.md` (the server also needs `MARKET_LAYOUT_DIR` pointing at `server/market`, which is the default when started from `server/`).
- [ ] **Step 4:** Run `flutter analyze` and `flutter test`. Then run the wire check as README describes (scratch database, real server, `WIRE_BASE_URL` and `WIRE_OTP_LOG` set). Expected: all pass. If it cannot be run, say so plainly in the report.
- [ ] **Step 5:** Commit `feat: read the market and rooms from the server`.

### Task 8: Docs and finish

**Files:** Modify `docs/superpowers/plans/2026-10-23-00-roadmap.md` (the Part 7 row: 7d built), `docs/superpowers/plans/2026-11-20-07-mobile-app.md` (sub-plan table: 7d built, with what it added), `server/.env.example` (`MARKET_LAYOUT_DIR`, `MARKET_ASSETS_*` with a comment on when to set them), a short `server/market/README.md` (how the files are made, that they mirror the app's art, the `store_type` addition, how to add plots).

- [ ] **Step 1:** Write the doc changes above. State plainly what was not run (a device, real art).
- [ ] **Step 2:** Run the full server suite, fmt, clippy, `flutter analyze`, `flutter test`. Expected: all clean. Record the counts in the roadmap row.
- [ ] **Step 3:** Write `.superpowers/FOLLOWUPS-07d-market-server.md` (anything deferred or found): at least the unscheduled items from the spec's non-goals and "store creation screen with the district choice arrives in 7e".
- [ ] **Step 4:** Commit `docs: record part 7d`.

# Part 7c: The Immersive Market Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Market tab's placeholder with the immersive market: a zoomable, tiled bird's-eye market with open and "coming soon" plots, one camera move into a store (by zoom or tap), a depth-aware store room whose shelves hold the seller's real products as cards, a product page, a cart carried everywhere, and a plain list view one tap away.

**Architecture:** Everything the market draws comes through one `MarketSource` interface whose JSON shapes are the contract the next server piece (7d) will implement. Until then a `PlaceholderMarketSource` builds the same shapes from bundled placeholder art and layout plus the routes the server already has (`GET /stores`, `GET /products?store=`). Camera, tile and journey logic are pure Dart classes with unit tests; widgets only draw them. The room is a photo with a depth map shifted by a fragment shader, with a flat fallback, and a frame-time monitor that steps down quietly to simpler effects on a weak phone.

**Tech Stack:** Flutter 3.50.1 stable, `flutter_riverpod`, `go_router`, `dio` (all existing); new: `cached_network_image` (remote tiles and photos, on-disk cache), `sensors_plus` (tilt), Flutter `FragmentProgram` (depth shader); dev-only `image` (placeholder art generator). Tests: `flutter_test` with hand-written fakes, golden tests for the journey's key frames, `integration_test` against a real server (env-gated).

**Spec:** `docs/superpowers/specs/2026-11-21-immersive-market-design.md` (sections 3, 6, 7 and 13; section 10 piece 2). Read with the app overview `docs/superpowers/plans/2026-11-20-07-mobile-app.md` (architecture decisions, global constraints) and the 7b plan `docs/superpowers/plans/2026-11-21-07b-app-foundation.md` (the patterns this builds on).

## Decisions taken for this plan (defaults, per the owner's go-ahead 2026-11-23)

- **Market size:** 60 plots in five districts (spec §11 default). Districts: Fashion (16), Phones and electronics (12), Home and beauty (12), Food and groceries (12), Everything else (8).
- **Plot assignment:** automatic. The server (7d) will do it; until then the placeholder source fills plots in plot order with active stores sorted by slug. Stores carry no category today, so in the placeholder a store's district is decorative.
- **Store types:** `boutique`, `phone_shop`, `grocery` (spec §6). Placeholder: a store's type follows its plot's district (Phones and electronics → `phone_shop`, Food and groceries → `grocery`, else `boutique`).
- **Checkout is not in this plan.** The cart page shows lines, quantities and the server's total; its checkout button arrives with the buying piece. "Message the seller" also waits for chat.
- **Numbering.** This piece is **7c**. The overview's unwritten 7c to 7e are renumbered in Task 15: 7d server additions for the market, 7e buying, selling and chat, 7f the art pipeline, 7g release readiness.

## Global Constraints

- Everything in 7b's Global Constraints still holds (Riverpod, `go_router`, `dio`, no `freezed`, integer kobo, strings in `lib/core/strings.dart`, the four list states, 48 dp, 200 percent text, reduced motion, no AI attribution, `flutter analyze` and `flutter test` clean from `app/` before each commit).
- **Budgets (spec §6):** first view of the market in about **1.5 s** on a mid-range Android phone over 4G (a small base picture at once, refined as tiles arrive); the journey aims for **60 fps and accepts no worse than about 45**; a normal session stays within **a few megabytes**; images WebP or JPEG; depth maps small and low resolution.
- **Bundled placeholder art at most 4 MB** in total (`app/assets/market/`), so the install stays light.
- **Nothing waits on a spinner during the journey.** The room's art is ready before the flight lands (bundled or prefetched); product cards fill in as they arrive, with skeleton cards in their slots.
- **Commit rule:** zoom (relative to fit-the-market = 1.0) **≥ 3.4** with the store's centre within **0.2 × the viewport's shortest side** of the screen centre. Re-entry is blocked for **800 ms** after leaving and until zoom falls below **3.0**.
- **Flight:** **1300 ms** in, **1100 ms** out. With reduced motion, or on the `fade` tier, a **250 ms** crossfade and a room that does not move.
- **The room never shows what cannot be bought:** only products with `in_stock == true` take a slot.
- **Personal data:** the market, layouts, rooms and tiles carry none and may be cached on disk. **The cart is personal: never written to disk, never logged.** No slug, id or search text is logged. Search text is sent only as the `q` query value, never in a path.
- **Remote assets only over https**, from the base URL in `/app/config` (`assets.base_url`); anything else falls back to the bundled art.
- **UI feel (owner, 2026-11-23): interactive but simple.** Every tappable control has a pressed state (one shared `Pressable`: scale 0.97 and a tint, 90 ms, none with reduced motion) and primary actions give a light haptic. Spacing only from `Tokens`, generous padding, few elements per screen. No gradients, emoji, card-in-card or filler copy. Don't overbuild.
- No golden image tests (see the ledger ruling): key frames are checked by widget tests on the values that draw them.

## Server routes this plan uses

Already built (7a and earlier); public unless marked.

| Route | Answer the app reads |
|---|---|
| `GET /categories` | `[{slug, name}]` in order |
| `GET /stores?q&sort&cursor&limit` | `{items: [{id, slug, name, logo_url, rating_average, rating_count, product_count}], next_cursor}` |
| `GET /stores/{slug}` | `{id, name, slug, description, status, rating_average, rating_count, logo_url, banner_url, ...}` |
| `GET /products?q&category&store&sort&cursor&limit` | `{items: [ProductCard], next_cursor}`; `ProductCard {id, name, category, condition, price_from_kobo, photo_url, in_stock, store: {id, slug, name, rating_average, rating_count}}`; `photo_url` may be null |
| `GET /products/{id}` | product fields plus `photo_urls: [String]`, `variants: [{id, attributes: {k: v}, available_stock, price_kobo}]` (a null `price_kobo` means `base_price_kobo`) |
| `GET /cart` (auth) | `{items: [{variant_id, product_id, product_name, attributes, store_id, store_name, unit_price_kobo, quantity, line_total_kobo, available}], total_kobo}` |
| `PUT /cart/items` (auth) | body `{variant_id, quantity}` (0 removes); answers the whole cart |

The contract 7d will add (the placeholder source produces exactly these shapes, from bundled files):

| Route | Answer |
|---|---|
| `GET /market` | `MarketJson` (Task 1) |
| `GET /stores/{slug}/room` | `RoomJson` (Task 1) |
| `GET /app/config` gains `assets` | `{version: String, base_url: https String}`; null or absent means use the bundled art |

## Review Focus

1. **Gestures during a flight.** A pinch, a second tap on another store, a tab switch or the system Back while the camera is moving: the flight is not interrupted except by Back (which reverses it), never enters twice, and never leaves the camera stuck between market and room. (Task 7)
2. **A store that changes underneath the buyer.** The store is suspended between the market and the room (room answers 404), or a product is unpublished or sells out between the room and "Add to cart" (422 or `available: false`): the buyer gets a plain sentence and a way back, never a crash or a ghost card. (Tasks 3, 11, 12)
3. **Slow or no network.** The room's products have not arrived when the flight lands, or the phone goes offline mid-session: art shows at once, cards show as skeletons, a failed fetch has a retry, and the market opens from its cache with an "offline" line. (Tasks 3, 8, 14)
4. **Odd data.** No stores at all (every plot "coming soon"), more stores than plots, a product with no photo, a 120-character name, a null rating, a malformed plot row or slot: each draws something sensible, and one bad row never blanks the market. (Tasks 1, 3, 6, 10)
5. **Screen readers and big text.** TalkBack moving across 60 dots at the widest zoom (each dot labelled, reachable, 48 dp regardless of zoom, in district then name order); cards and price tags at 200 percent; reduced motion. (Tasks 6, 10, 13)

---

## File structure

```
app/
  assets/market/                       placeholder art (generated, Task 2)
    layout.json                        MarketJson without placements
    base.jpg                           256 px whole-market picture
    tiles/{level}/{x}_{y}.jpg          tile pyramid
    rooms/{type}/room.jpg, depth.png, room.json   StoreTypeJson
  shaders/depth_parallax.frag          Task 9
  tool/placeholder_art.dart            Task 2 (dev only)
  lib/features/catalog/                shared browse data (Task 3)
    catalog_models.dart, catalog_repository.dart
  lib/features/market/
    model/market.dart                  layout, plots, districts, placements (Task 1)
    model/room.dart                    store types, slots, rooms (Task 1)
    data/market_source.dart            interface + providers (Task 3)
    data/placeholder_market_source.dart, json_cache.dart  (Task 3)
    data/asset_resolver.dart           (Task 4)
    camera/market_camera.dart, tiles.dart   (Task 5)
    view/market_view.dart, tile_layer.dart, plot_markers.dart  (Task 6)
    journey/journey_controller.dart, journey_view.dart  (Tasks 7, 8)
    room/depth_room.dart, shelf_card.dart, room_page.dart  (Tasks 9, 10)
    perf/render_tier.dart              (Task 11)
    market_page.dart                   (Task 14)
  lib/features/product/product_page.dart          (Task 12)
  lib/features/cart/cart.dart, cart_repository.dart, cart_button.dart, cart_page.dart  (Task 13)
  lib/features/list/list_page.dart, store_page.dart  (Task 14)
  integration_test/market_flow_test.dart          (Task 15)
```

---

### Task 0: Branch, toolchain and golden baselines

**Files:** Create `app/dart_test.yaml`, `app/test/support/golden.dart`, `app/tool/goldens.sh`.

- [ ] **Step 1:** Worktree `.claude/worktrees/immersive-market` on branch `feat/immersive-market` from `origin/main`.
- [ ] **Step 2:** From `app/`: `flutter pub get`, `dart run build_runner build --delete-conflicting-outputs`, `flutter analyze`, `flutter test`. Expected: clean, 280 tests pass. Stop if not.
- [ ] **Step 3:** `dart_test.yaml` declares tag `golden`. `golden.dart` exports `goldenTest(String name, WidgetTesterCallback body)` that calls `testWidgets(..., tags: ['golden'], skip: !Platform.isLinux)`. `tool/goldens.sh` runs `flutter test --tags golden --update-goldens` inside `ghcr.io/cirruslabs/flutter:3.50.1` with `app/` mounted (Docker must be running: check `docker ps`, start Docker Desktop if down).
- [ ] **Step 4:** Commit `chore: add golden test tagging for the market`.

### Task 1: Market and room models (the 7d contract)

**Files:** Create `lib/features/market/model/market.dart`, `model/room.dart`, `lib/features/catalog/catalog_models.dart` (only `StoreSummary` and `ProductCard`); test `test/features/market/model_test.dart`.

**Interfaces (Produces):** all parsed by hand, tolerant, like `AppConfig` (a bad row is skipped, never a throw except for a missing top level).
```dart
class District { final String id; final String name; }
class Plot { final String id; final String districtId; final Rect rect; }        // world units
class Placement { final String plotId; final StoreSummary store; final String storeType; }
class StoreSummary { final String id, slug, name; final String? logoUrl; final double? ratingAverage; final int ratingCount; }
class MarketLayout {
  final String version; final Size world; final int tileSize; final int maxLevel;
  final String baseImage; final List<District> districts; final List<Plot> plots;
  static MarketLayout fromJson(Map<String, dynamic> json); // throws FormatException only if world/tileSize/maxLevel unusable
}
class MarketSnapshot { final MarketLayout layout; final List<Placement> placements; final bool fromCache;
  Placement? placementFor(String plotId); }
class ShelfSlot { final String id; final Rect rect; final double depth; final double tiltY; } // rect in 0..1 of the room picture; depth 0 (far)..1 (near); tiltY radians
class StoreType { final String id; final String roomImage, depthImage; final List<ShelfSlot> slots;
  final Color lightTint; final double shadowOpacity; }
class PlacedProduct { final String slotId; final ProductCard product; }
class RoomView { final StoreSummary store; final StoreType type; final List<PlacedProduct> placed; final bool hasMore; }
```
`MarketJson` = `{version, world: {w, h}, tile_size, max_level, base_image, districts: [{id, name}], plots: [{id, district, x, y, w, h}], placements: [{plot, store_type, store: StoreCard}]}`.
`StoreTypeJson` = `{id, room_image, depth_image, light: {tint: "#RRGGBB", shadow_opacity}, slots: [{id, x, y, w, h, depth, tilt_y}]}`.
`RoomJson` = `{store: StoreCard, store_type: StoreTypeJson, placed: [{slot, product: ProductCard}], has_more}`.
`StoreSummary` and `ProductCard` (shape in Task 3's Interfaces) are created here in `lib/features/catalog/catalog_models.dart`, with `fromJson` that skips nothing but tolerates a null `photo_url` and null rating; Task 3 adds the rest of that file.

- [ ] **Step 1: Failing tests:** `parses_the_full_market_json`; `skips_plots_with_missing_or_negative_sizes_and_keeps_the_rest`; `skips_placements_for_unknown_plots_and_duplicate_plots_keeps_first`; `slot_rects_outside_0_1_are_dropped`; `bad_tint_falls_back_to_white_and_shadow_is_clamped_0_to_1`; `room_drops_placed_products_that_are_out_of_stock`; `room_drops_placements_for_unknown_slots`; `null_rating_is_kept_as_null`.
- [ ] **Step 2:** Run `flutter test test/features/market/model_test.dart`. Expected: FAIL (no such files).
- [ ] **Step 3:** Implement the classes above.
- [ ] **Step 4:** Run the file, then the whole suite. Expected: PASS.
- [ ] **Step 5:** Commit `feat: add the market and room models`.

### Task 2: Placeholder art

**Files:** Create `app/tool/placeholder_art.dart`, the generated `app/assets/market/**`; modify `pubspec.yaml` (dev dependency `image`, `flutter: assets:` and `shaders:` entries); test `test/features/market/placeholder_assets_test.dart`.

- [ ] **Step 1: Failing test** `placeholder_assets_test.dart` (reads files from disk, no Flutter binding):
  - `layout_has_60_plots_in_5_districts_with_the_agreed_counts` (16/12/12/12/8), `plots_do_not_overlap_and_lie_inside_the_world`;
  - `every_tile_of_every_level_exists` for world 2048 × 2048, `tile_size` 256, `max_level` 3 (level L has `2^L × 2^L` tiles);
  - `each_store_type_has_room_depth_and_at_least_8_slots` for `boutique`, `phone_shop`, `grocery`; depth PNG at most 180 × 320;
  - `bundled_art_is_at_most_4_MB`.
- [ ] **Step 2:** Run it. Expected: FAIL.
- [ ] **Step 3:** Write `dart run tool/placeholder_art.dart` (deterministic, seeded): draws a top-down market (five district grounds in distinct warm tones, roads, roofs as noisy rectangles over each plot, no text), cuts the pyramid at JPEG quality 70, writes a 256 px `base.jpg`; for each store type, a 720 × 1280 room (back wall, floor, shelves drawn as lighter bands where the slots are) and a 90 × 160 grayscale depth map (floor and shelf fronts nearer, back wall far, matching the slot depths); writes `layout.json` and each `room.json`. It is placeholder art and must look like it: flat shapes, no attempt at realism (the real art is piece 7f).
- [ ] **Step 4:** Run the generator, then the test and the whole suite. Expected: PASS.
- [ ] **Step 5:** Commit `feat: add generated placeholder market art`.

### Task 3: Catalog repository and the placeholder market source

**Files:** Modify `lib/features/catalog/catalog_models.dart` (from Task 1); create `lib/features/catalog/catalog_repository.dart`, `lib/features/market/data/market_source.dart`, `placeholder_market_source.dart`, `json_cache.dart`; tests `test/features/catalog/catalog_repository_test.dart`, `test/features/market/placeholder_market_source_test.dart`; fakes `test/support/fake_catalog_repository.dart`, `test/support/fake_market_source.dart`.

**Interfaces (Produces):**
```dart
class Category { final String slug, name; }
class ProductCard { final String id, name, category, condition; final int priceFromKobo; final String? photoUrl; final bool inStock; final StoreSummary store; }
class Variant { final String id; final Map<String, String> attributes; final int availableStock; final int priceKobo; } // priceKobo resolved from base when null
class ProductDetail { final String id, name, description, condition; final String? brand; final List<String> photoUrls; final List<Variant> variants; final String storeId; }
class StoreDetail { final StoreSummary summary; final String description; final String? bannerUrl; }
class Page<T> { final List<T> items; final String? nextCursor; }

abstract class CatalogRepository {
  Future<List<Category>> categories();
  Future<Page<ProductCard>> products({String? q, String? category, String? store, String? sort, String? cursor, int limit = 24});
  Future<Page<StoreSummary>> stores({String? q, String? sort, String? cursor, int limit = 24});
  Future<StoreDetail> store(String slug);
  Future<ProductDetail> product(String id);
}
final catalogRepositoryProvider = Provider<CatalogRepository>(...); // ApiCatalogRepository(ref.watch(apiClientProvider))

abstract class MarketSource {
  Future<MarketSnapshot> market();
  Future<RoomView> room(String slug); // throws ApiError(404, 'store_not_found') when the store is gone
}
final marketSourceProvider = Provider<MarketSource>(...);         // PlaceholderMarketSource until 7d
final marketProvider = FutureProvider<MarketSnapshot>(...);
final roomProvider = FutureProvider.family<RoomView, String>(...); // keepAlive for 5 minutes, so a prefetch survives until the flight lands
class JsonCache { Future<Map<String, dynamic>?> read(String key); Future<void> write(String key, Map<String, dynamic> json); } // shared_preferences, keys 'market.v1' and 'room.v1.<slug>'
```
Paths: the slug and id are put in the path with `Uri.encodeComponent`; `q` is a query value, trimmed and cut to 80 characters.

`PlaceholderMarketSource.market()`: loads `assets/market/layout.json`, pages `GET /stores?sort=newest` until it has 60 stores or no cursor, sorts by slug, fills plots in plot order, sets the type by district (see Decisions), writes the result to `JsonCache`; on an `ApiError` with `status == 0` (no network) it answers the cached snapshot with `fromCache: true`, or rethrows if there is none. `room(slug)`: `GET /stores/{slug}` for the summary, its type from the cached market placement (`boutique` if unknown), `GET /products?store=<slug>&sort=newest&limit=<slots>` and places the in-stock ones in slot order; `hasMore` is `next_cursor != null` or any product left out.

- [ ] **Step 1: Failing tests (repository, against `FakeAdapter`):** `categories_parse_and_skip_bad_rows`; `products_send_q_as_a_query_value_trimmed_to_80_chars`; `slug_and_id_are_encoded_in_the_path` (`a/b` → `a%2Fb`); `variant_without_price_uses_base_price`; `product_with_null_photo_url_parses`; `a_non_json_answer_is_bad_response`.
- [ ] **Step 2: Failing tests (placeholder source, with a fake catalog and an in-memory `JsonCache`):** `fills_plots_in_order_by_slug`; `more_stores_than_plots_keeps_the_first_60`; `no_stores_gives_an_all_coming_soon_market`; `type_follows_district`; `offline_answers_the_cache_with_fromCache_true`; `offline_with_no_cache_rethrows`; `room_places_only_in_stock_products_in_slot_order`; `room_sets_hasMore_when_products_exceed_slots`; `room_for_a_missing_store_throws_store_not_found`; `nothing_personal_is_cached` (the cache holds only the keys above, and the cart is never read).
- [ ] **Step 3:** Run both files. Expected: FAIL.
- [ ] **Step 4:** Implement.
- [ ] **Step 5:** Run both files, then the whole suite. Expected: PASS.
- [ ] **Step 6:** Commit `feat: add the catalog repository and the placeholder market source`.

### Task 4: Where the art comes from

**Files:** Create `lib/features/market/data/asset_resolver.dart`; modify `lib/core/config/app_config.dart`, `pubspec.yaml` (`cached_network_image`); tests `test/features/market/asset_resolver_test.dart`, extend `test/core/config/app_config_repository_test.dart`.

**Interfaces (Produces):**
```dart
// AppConfig gains: final String? assetsVersion; final String? assetsBaseUrl; (https only, via _httpsUrl)
abstract class AssetResolver {
  ImageProvider tile(int level, int x, int y);
  ImageProvider base();
  ImageProvider room(StoreType t);
  ImageProvider depth(StoreType t);
}
class BundledAssetResolver implements AssetResolver { ... }   // AssetImage('assets/market/...')
class RemoteAssetResolver implements AssetResolver { ... }    // CachedNetworkImageProvider('$base/$version/tiles/$level/${x}_$y.jpg', cacheKey: same)
final assetResolverProvider = Provider<AssetResolver>(...);   // Remote when both config values are set, else Bundled
```
- [ ] **Step 1: Failing tests:** `config_parses_assets_and_ignores_http_or_missing_base_url`; `resolver_is_bundled_without_assets_config`; `remote_tile_url_is_versioned` (`https://cdn.example/v3/tiles/2/1_3.jpg`); `remote_room_paths_use_the_type_files`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the files, then the whole suite. Expected: PASS.
- [ ] **Step 5:** Commit `feat: resolve market art from the bundle or the asset host`.

### Task 5: Camera and tile maths

**Files:** Create `lib/features/market/camera/market_camera.dart`, `camera/tiles.dart`; test `test/features/market/camera_test.dart`.

**Interfaces (Produces):** pure, no Flutter widgets.
```dart
class MarketCamera {
  const MarketCamera({required this.world, required this.viewport, required this.center, required this.zoom});
  final Size world, viewport; final Offset center; final double zoom; // zoom 1.0 = whole world fits the viewport
  double get scale;                          // logical px per world unit = fitScale * zoom
  Offset worldToScreen(Offset w); Offset screenToWorld(Offset s);
  Rect get visibleWorld;
  MarketCamera clamped({double maxZoom});    // zoom in [1, maxZoom]; center keeps the world covering the viewport where it can
  MarketCamera panBy(Offset screenDelta); MarketCamera zoomAround(Offset focalScreen, double factor);
  static double maxZoomFor(Size world, Size viewport, double dpr); // deepest level shown no larger than 1.5 device px per tile px
}
class TileKey { final int level, x, y; }
int levelFor(MarketCamera c, {required int maxLevel, required double dpr}); // clamp(maxLevel + ceil(log2(scale * dpr)), 0, maxLevel)
List<TileKey> visibleTiles(MarketCamera c, {required int level, required int maxLevel, required int tileSize}); // tiles overlapping visibleWorld, plus a one-tile margin, never outside the grid
```
- [ ] **Step 1: Failing tests:** `zoom_1_fits_the_world`; `world_to_screen_round_trips`; `zoom_around_keeps_the_focal_point_still`; `clamped_never_shows_outside_the_world_when_zoomed_in`; `level_for_picks_0_when_whole_and_max_when_deep` (world 2048, viewport 400 × 800, dpr 2.75: zoom 1 → level 1; zoom 8 → level 3); `visible_tiles_cover_the_view_with_a_margin_and_stay_in_grid`; `max_zoom_matches_the_density_rule`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run, then the whole suite. Expected: PASS.
- [ ] **Step 5:** Commit `feat: add the market camera and tile maths`.

### Task 6: The market view

**Files:** Create `lib/features/market/view/market_view.dart`, `tile_layer.dart`, `plot_markers.dart`; modify `lib/core/strings.dart`; test `test/features/market/market_view_test.dart`.

**Interfaces:**
- Consumes: `MarketSnapshot` (Task 1), `AssetResolver` (Task 4), `MarketCamera`, `levelFor`, `visibleTiles` (Task 5).
- Produces: `MarketView({required MarketSnapshot snapshot, required MarketCamera camera, required ValueChanged<MarketCamera> onCamera, required ValueChanged<Placement> onOpenStore, required ValueChanged<Placement> onPrefetch})`.

Behaviour: the base picture always underneath, stretched to the world; tiles of the chosen level on top, each fading in over 150 ms as it decodes (none with reduced motion); one scale gesture pans and zooms (`panBy`, `zoomAround`, `clamped`). Markers are drawn by the app at fixed screen size: an open store is a glowing dot (`tokens.gold` glow; the glow pulses only without reduced motion) with its name and rating shown from zoom 1.6; a plot without a store is a grey dot labelled "Coming soon". Each marker's hit area is at least 48 × 48 dp whatever the zoom; where two overlap, the nearer centre wins. Pressing down on an open dot calls `onPrefetch`; a tap calls `onOpenStore`. Semantics: each open dot is a button labelled `"<name>, <rating> stars, <count> reviews, open"` (or `"no reviews yet"`), each coming-soon dot `"Coming soon, <district>"`, sorted by district then name; markers outside the view are excluded from semantics.

- [ ] **Step 1: Failing widget tests** (fake resolver with in-memory images): `draws_the_base_picture_before_any_tile_arrives`; `requests_only_visible_tiles_at_the_chosen_level`; `open_and_coming_soon_markers_are_labelled`; `names_appear_only_from_zoom_1_6`; `marker_hit_area_is_at_least_48dp_at_zoom_1`; `tap_opens_and_press_prefetches`; `no_stores_shows_every_plot_as_coming_soon`; `a_120_char_name_is_ellipsised_and_fully_in_the_label`; `markers_off_screen_are_not_in_semantics`; `text_at_200_percent_does_not_overflow` (no `FlutterError` from layout).
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run, then the whole suite. Expected: PASS.
- [ ] **Step 5:** Commit `feat: draw the market with tiles and store markers`.

### Task 7: The journey controller

**Files:** Create `lib/features/market/journey/journey_controller.dart`; test `test/features/market/journey_controller_test.dart`.

**Interfaces (Produces):** pure, time injected.
```dart
sealed class JourneyState {}
class AtMarket extends JourneyState {}
class FlyingIn extends JourneyState { final Placement target; final DateTime started; }
class InRoom extends JourneyState { final Placement store; }
class FlyingOut extends JourneyState { final Placement from; final DateTime started; }

class JourneyController extends ChangeNotifier {
  JourneyController({required DateTime Function() now, required Duration flightIn, required Duration flightOut}); // 1300 / 1100 ms, or 250 / 250 for fade
  JourneyState get state;
  void onCamera(MarketCamera c, List<Placement> open); // commits when the Global Constraints' rule holds, the cooldown is over and zoom has dropped below 3.0 since leaving
  bool requestEnter(Placement p);  // a tap; false unless AtMarket and not cooling down
  void tick();                     // moves FlyingIn → InRoom and FlyingOut → AtMarket when their time is up
  bool back();                     // FlyingIn → FlyingOut (reversed from the current progress); InRoom → FlyingOut; else false
  double progress();               // 0..1 of the current flight
  Placement? prefetchCandidate(MarketCamera c, List<Placement> open); // from zoom 2.0: the open plot nearest the screen centre that is on screen
}
```
- [ ] **Step 1: Failing tests:** `zoom_3_4_near_centre_commits`; `zoom_3_4_off_centre_does_not`; `zoom_3_3_does_not`; `tap_enters_with_the_same_target_path`; `a_second_tap_during_flight_is_ignored`; `camera_updates_during_flight_are_ignored`; `back_during_flight_in_reverses_from_current_progress`; `back_in_room_flies_out`; `no_reentry_for_800ms_after_leaving`; `no_reentry_until_zoom_drops_below_3`; `tick_lands_at_exactly_the_flight_time`; `prefetch_candidate_needs_zoom_2_and_picks_the_nearest_on_screen`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run, then the whole suite. Expected: PASS.
- [ ] **Step 5:** Commit `feat: add the journey controller`.

### Task 8: The journey on screen

**Files:** Create `lib/features/market/journey/journey_view.dart`; test `test/features/market/journey_view_test.dart`, goldens `test/features/market/goldens/journey_*.png`.

**Interfaces:**
- Consumes: `JourneyController` (Task 7), `MarketView` (Task 6), `RoomPage` (Task 10; until it lands, a `Widget Function(Placement) roomBuilder` parameter).
- Produces: `JourneyView({required MarketSnapshot snapshot, required JourneyController journey, required Widget Function(Placement) roomBuilder, required RenderTier tier})`.

The path, one curve for both entries (progress p, `Curves.easeInOutCubic`): p 0 to 0.55, the camera zooms from where it is to zoom 8 centred on the plot (sky to block to shopfront); p 0.55 to 0.8, the room crossfades in, scaling from 1.15 to 1.0 (the door); p 0.8 to 1.0, a warm amber overlay (`tokens.gold`, opacity 0.35 to 0) fades out (the light coming up). Out is the same path reversed. On the `fade` tier or with reduced motion: only the crossfade. Driven by a `Ticker` that calls `journey.tick()`. The room's widget is built from the start of the flight so its art is decoded before it shows. `PopScope` routes system Back to `journey.back()` while not `AtMarket`.

- [ ] **Step 1: Failing tests:** `tap_flies_in_and_lands_in_the_room_after_1300ms`; `system_back_mid_flight_reverses_to_the_market`; `reduced_motion_is_a_250ms_crossfade_with_no_zoom`; `room_is_built_before_it_is_visible`; goldens via `goldenTest`: `journey_p0`, `journey_p50`, `journey_p100`.
- [ ] **Step 2:** Run. Expected: FAIL (goldens are skipped off Linux).
- [ ] **Step 3:** Implement. Make the goldens with `tool/goldens.sh` and look at all three images before committing them.
- [ ] **Step 4:** Run, then the whole suite. Expected: PASS.
- [ ] **Step 5:** Commit `feat: fly between the market and a store`.

### Task 9: The depth room

**Files:** Create `app/shaders/depth_parallax.frag`, `lib/features/market/room/depth_room.dart`; modify `pubspec.yaml` (`sensors_plus`); test `test/features/market/depth_room_test.dart`.

**Interfaces (Produces):** `DepthRoom({required ImageProvider room, required ImageProvider depth, required RenderTier tier, required Widget Function(Offset look) overlay})`. `look` is the view offset in -1..1 on each axis, from a horizontal drag (the whole screen width = 2) and, on the `full` tier without reduced motion, device tilt (`sensors_plus` accelerometer, low-pass filtered, ±15° maps to ±1). The overlay builder is how Task 10 places cards that move with the same `look`.

The shader: `uniform` the room and depth samplers, `look` and `strength`; samples depth at `uv`, offsets `uv` by `look * strength * (depth - 0.5)` and samples the room there; `strength` 0.025 (2.5 percent of width, the spec's "keep movement small"). Edges: clamp the shifted `uv` inside 0..1 and scale the picture to 1.05 so no edge ever shows. On the `reduced` tier, or if the shader fails to load, the room is a flat picture that shifts by `look * 0.01` as one layer. With reduced motion `look` stays at zero.

- [ ] **Step 1: Failing tests:** `drag_moves_look_and_releases_back_to_centre` (spring back over 400 ms; none with reduced motion); `look_is_clamped_to_minus_1_1`; `reduced_motion_keeps_look_at_zero_and_ignores_tilt`; `shader_load_failure_falls_back_to_flat` (inject a loader that throws); `reduced_tier_uses_flat`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run, then the whole suite. Expected: PASS.
- [ ] **Step 5:** Commit `feat: add the depth room with a flat fallback`.

### Task 10: Shelf cards and the room page

**Files:** Create `lib/features/market/room/shelf_card.dart`, `room/room_page.dart`; modify `strings.dart`; tests `test/features/market/shelf_card_test.dart`, `test/features/market/room_page_test.dart`.

**Interfaces:**
- Consumes: `roomProvider` (Task 3), `AssetResolver` (Task 4), `DepthRoom` (Task 9), `CartButton` (Task 13; until it lands, a placeholder slot in the top bar).
- Produces: `ShelfCard({required ProductCard product, required ShelfSlot slot, required StoreType type, required Offset look, required VoidCallback onTap})`; `RoomPage({required Placement placement, required VoidCallback onBack, required void Function(String productId) onOpenProduct, required VoidCallback onSeeAll})`.

Card: the product's photo (`photo_url`, or a neutral tile with a bag icon when null) in a rounded card sized to the slot, rotated by `slot.tiltY` with perspective, tinted by `type.lightTint` at 12 percent, a soft shadow at `type.shadowOpacity`, and a price tag `"From ₦<price>"` (`Money.format(priceFromKobo)`). Drift: offset by `look * 6 dp * slot.depth`. Semantics: a button labelled `"<name>, from ₦<price>"`; hit area at least 48 dp even for a small slot. Page: the room art shows at once; while `roomProvider` loads, every slot shows a skeleton card; on error an inline `ErrorState` panel with retry over the room (no spinner); on `store_not_found` the sentence "This store has closed." with "Back to market"; empty (no in-stock products) "Nothing on the shelves yet." with "Back to market". Top bar: Back to market (calls `onBack`), the store name and rating, the cart button. "See all" shows when `hasMore`.

- [ ] **Step 1: Failing tests:** `card_shows_photo_name_label_and_from_price`; `card_without_photo_shows_the_neutral_tile`; `card_drifts_more_when_nearer`; `card_hit_area_is_at_least_48dp`; `price_tag_at_200_percent_does_not_overflow`; `room_shows_skeleton_cards_while_loading`; `room_error_has_retry_and_no_spinner`; `closed_store_says_so_and_goes_back`; `empty_room_says_so`; `see_all_only_when_has_more`; `tapping_a_card_opens_its_product`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run, then the whole suite. Expected: PASS.
- [ ] **Step 5:** Commit `feat: put a store's products on its shelves`.

### Task 11: Render tiers

**Files:** Create `lib/features/market/perf/render_tier.dart`; test `test/features/market/render_tier_test.dart`.

**Interfaces (Produces):**
```dart
enum RenderTier { full, reduced, fade }
class FrameBudget {
  FrameBudget({required RenderTier start, required ValueChanged<RenderTier> onChange});
  void add(Duration frameTotal);   // build + raster of one frame, fed from SchedulerBinding.addTimingsCallback during a flight or room drag only
}
final renderTierProvider = NotifierProvider<RenderTierNotifier, RenderTier>(...); // persisted in shared_preferences 'render_tier.v1'; steps down only, never up, within an install
```
Rule: over a window of 30 frames, if the 90th percentile exceeds 22 ms (about 45 fps), step down one tier and start a new window. The tier also starts at `reduced` when the system asks for reduced motion (the room still shows, without tilt or shader).

- [ ] **Step 1: Failing tests:** `steady_16ms_frames_stay_full`; `p90_over_22ms_steps_down_one_tier`; `never_steps_below_fade`; `never_steps_up`; `a_window_needs_30_frames`; `tier_survives_a_restart`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run, then the whole suite. Expected: PASS.
- [ ] **Step 5:** Commit `feat: step down to simpler effects on slow phones`.

### Task 12: The product page

**Files:** Create `lib/features/product/product_page.dart`; modify `strings.dart`; test `test/features/product/product_page_test.dart`.

**Interfaces:**
- Consumes: `CatalogRepository.product` (Task 3), `cartProvider` notifier's `add(String variantId)` (Task 13; this task and 13 can be built in either order: write against the interface).
- Produces: `ProductPage({required String productId})`.

Shows: a photo pager (the neutral tile when there are none), name, brand, condition, description, the chosen variant's price (`Money.format`), a chooser per attribute key (sorted) built from the variants, stock ("Only <n> left" when 1 to 5, "Sold out" at 0, nothing above 5), the store's name and rating, and "Add to cart" (disabled until a variant is fully chosen or when sold out; a single variant with no attributes is chosen already). After adding: the item flies to the cart button (skipped with reduced motion) and a snackbar "Added to your cart". A 422 from the cart (`product_unavailable`, `insufficient_stock` or any other) shows "This item can no longer be bought" or the server's own message for a stock limit. Four states; loading is a skeleton shaped like the page.

- [ ] **Step 1: Failing tests:** `shows_photos_name_and_the_cheapest_price_before_choosing`; `choosing_size_and_colour_picks_the_variant_and_its_price`; `only_n_left_from_1_to_5_and_sold_out_at_0`; `add_is_disabled_until_a_variant_is_chosen`; `single_variant_without_attributes_is_preselected`; `add_calls_cart_with_the_variant_id_only` (the request body is exactly `{variant_id, quantity}`, never a price); `unavailable_after_open_says_so`; `loading_error_with_retry`; `page_at_200_percent_does_not_overflow`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run, then the whole suite. Expected: PASS.
- [ ] **Step 5:** Commit `feat: add the product page`.

### Task 13: The cart

**Files:** Create `lib/features/cart/cart.dart`, `cart_repository.dart`, `cart_button.dart`, `cart_page.dart`; modify `strings.dart`; tests `test/features/cart/cart_repository_test.dart`, `test/features/cart/cart_page_test.dart`, fake `test/support/fake_cart_repository.dart`.

**Interfaces (Produces):**
```dart
class CartLine { final String variantId, productId, productName, storeId, storeName; final Map<String, String> attributes; final int unitPriceKobo, quantity, lineTotalKobo; final bool available; }
class Cart { final List<CartLine> items; final int totalKobo; int get count; Map<String, List<CartLine>> get byStore; } // count = sum of quantities of available lines
abstract class CartRepository { Future<Cart> get(); Future<Cart> set(String variantId, int quantity); }
final cartProvider = AsyncNotifierProvider<CartNotifier, Cart>(...); // add(variantId) = set(current + 1); setQuantity(variantId, n); refresh()
class CartButton extends ConsumerWidget { const CartButton({super.key}); } // icon with count badge, 48 dp, label "Cart, <n> items"; opens /cart; exposes a GlobalKey via cartButtonKeyProvider for the fly-in target
```
The cart lives only in memory and is dropped on sign-out (watch the session). Cart page: lines grouped by store, each with name, attributes, unit price, a quantity stepper (minus at 1 asks "Remove <name>?"), a line total; unavailable lines greyed with "No longer available" and a Remove action; the server's `total_kobo` at the bottom; the line "Checkout opens soon." in place of a checkout button. Empty: "Your cart is empty" with "Go to the market". Every amount on the page is a server field.

- [ ] **Step 1: Failing tests:** `get_and_set_parse_the_cart`; `set_sends_variant_and_quantity_only`; `count_ignores_unavailable_lines`; `cart_is_dropped_on_sign_out`; `nothing_about_the_cart_is_written_to_preferences` (the fake `SharedPreferences` stays without cart keys); `page_groups_by_store`; `stepper_updates_via_the_server_answer`; `minus_at_1_confirms_removal`; `unavailable_line_is_marked_and_removable`; `total_is_the_server_total`; `empty_state_leads_to_the_market`; `badge_label_reads_the_count`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run, then the whole suite. Expected: PASS.
- [ ] **Step 5:** Commit `feat: add the cart that follows the buyer`.

### Task 14: The market page, the list view and routes

**Files:** Create `lib/features/market/market_page.dart`, `lib/features/list/list_page.dart`, `list/store_page.dart`; modify `lib/router.dart`, `lib/core/paths.dart`, `lib/features/home/shell.dart` (Market tab builds `MarketPage`), delete `lib/features/home/home_page.dart` (the addresses link moves to the account page if it is not already there), `strings.dart`; tests `test/features/market/market_page_test.dart`, `test/features/list/list_page_test.dart`, update `test/router_test.dart`, `test/features/home/shell_test.dart`.

**Routes (Produces, in `paths.dart`):** `/home` market; `/home/store/:slug` the room (a deep link lands in the room without a flight); `/home/product/:id`; `/home/list`; `/home/list/store/:slug` the plain store page; `/cart`. All under the signed-in prefixes.

Market page: `marketProvider` loading shows the base picture with a skeleton strip, not a spinner; error with retry; `fromCache` shows a thin line "You're offline. Showing the market as it was." Always on screen: a search field that opens the list view with focus, a "List" button, and the cart button. Opening a store calls `journey.requestEnter` and `ref.read(roomProvider(slug).future)` (the prefetch), and `onPrefetch` and `prefetchCandidate` also start it.

List view: tabs Products and Stores; search (300 ms debounce, 80 characters at most); category chips from `categories()`; results paged by cursor as the list nears its end; product rows open `/home/product/:id`, store rows open `/home/list/store/:slug` (header, rating, description, the store's products, no flight). Four states each, the empty search result saying "Nothing matches "<q>"" with "Clear search".

- [ ] **Step 1: Failing tests:** `market_tab_shows_the_market`; `offline_snapshot_shows_the_offline_line`; `market_load_error_has_retry`; `opening_a_store_prefetches_its_room`; `list_button_and_search_open_the_list_with_no_flight`; `list_search_debounces_and_sends_q`; `category_chip_filters`; `paging_loads_the_next_cursor_once`; `empty_search_offers_clear`; `store_row_opens_the_plain_store_page`; `deep_link_to_a_store_opens_the_room`; router: `market_routes_need_sign_in`, `unknown_market_path_goes_home`.
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run, then the whole suite. Expected: PASS.
- [ ] **Step 5:** Commit `feat: open the immersive market with its list view`.

### Task 15: End-to-end flow, docs and device checks

**Files:** Create `app/integration_test/market_flow_test.dart`; modify `app/README.md`, `docs/superpowers/plans/2026-11-20-07-mobile-app.md` (renumbering, 7c status), `docs/superpowers/plans/2026-10-23-00-roadmap.md` (part 7 row), `.superpowers/FOLLOWUPS-07c-immersive-market.md` (new, in the worktree).

- [ ] **Step 1:** The server has no seed script, so add `app/integration_test/fixtures/market_seed.sql`: one active store (`slug` `flow-test-boutique`), three published products with variants and stock, one of them with two sizes; invented names only, no personal data; idempotent (`ON CONFLICT DO NOTHING`). Applied with `psql` against the local database before the run.
- [ ] **Step 2:** Integration flow, skipped unless `--dart-define=WIRE_BASE_URL=...` and `--dart-define=WIRE_OTP_LOG=...` are given (the 7b wire check's names; it refuses any host but loopback and the emulator's `10.0.2.2`): sign up a fresh phone (code read from the OTP log as the wire check does), open the list, search a product name, open it, choose a size, add to cart, open the cart and see the line with the server's total; then from the market, tap the store's dot and land in its room with its products on the shelves.
- [ ] **Step 2b:** Start the server's dependencies (`docker ps`; `docker start` the Postgres and MinIO containers if stopped), apply the fixture, run the server with its stdout to the OTP log, run the flow on an Android emulator: `flutter test integration_test/market_flow_test.dart -d emulator-5554 --dart-define=WIRE_BASE_URL=http://10.0.2.2:8080 --dart-define=WIRE_OTP_LOG=<path>`. Expected: PASS. If no emulator can be started, say so and list which checks did not run.
- [ ] **Step 3:** The whole suite from `app/` and the goldens in Docker. Expected: PASS.
- [ ] **Step 4:** README: how the placeholder art is made, how to point the app at remote art, the tiers, and a **device checklist** to run on a real mid-range Android phone (first view under 1.5 s over 4G, flight frame rate with the performance overlay, room drag and tilt, data used in a ten-minute session, TalkBack across the dots, 200 percent text, reduced motion).
- [ ] **Step 5:** Update the overview (7c built; renumbered 7d to 7g) and the roadmap; write the follow-ups file.
- [ ] **Step 6:** Commit `docs: document the immersive market and renumber the app plans`.

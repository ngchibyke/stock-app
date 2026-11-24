# market (the phone app)

The Flutter app for Android and iOS. It talks to the server in `../server`.

Today it does: sign-up with a phone code, sign-in, password recovery, an
account page and an address book, and the **market**: a map of the market you
pan and pinch, a camera flight into a store's room with its products on the
shelves, a plain list view with search, product pages and the cart. Checkout,
orders and selling are not built yet.

## Status: not yet run on a device

**The app has NOT been run on a real device or an emulator.** None exists on
the machine it was built on, and the Android SDK licences are not accepted
there. What has been done:

- the unit and widget tests pass (`flutter test`);
- the real-server wire checks passed (`test/wire`, see below): the app's real
  network code was driven against a real running server, including the market
  path (search, product, sizes, cart totals, the store's plot and its room).

What a first run on a device should check:

- sign-up with a real SMS code, end to end;
- the keyboard and autofill behaviour on each form;
- the code boxes with the system one-time-code suggestion;
- text at 200 percent;
- the shell and the account page;
- the market device checklist below.

## The market

The market screen is drawn from art, not from widgets: a low-resolution
`base.jpg`, map tiles at three zoom levels, and for each store type a room
photo with a grayscale depth map. A `MarketSource` gives the app the layout
(plots, districts, which store stands where) and each room's products.

**Placeholder art.** Until the real art exists (plan 7f) the app ships flat,
obviously-fake art in `assets/market/`, made by a seeded generator:

```
dart run tool/placeholder_art.dart
```

It writes the tiles, `base.jpg`, `layout.json` and every room's image, depth
map and `room.json`. The same seed always gives the same files.

**Remote art.** When `/app/config` returns an `assets` block with an https
`base_url` and a `version` (one plain path segment), the app loads art from
`<base_url>/<version>/` (same file layout as `assets/market/`) and caches it on
disk; otherwise it uses the bundled art. Publish a new version under a new
folder rather than overwriting one: the version is the cache key.

**Placement.** Until the server places stores (plan 7d), the app fills plots
in plot order with active stores sorted by slug.

**Tiers.** A frame-time monitor watches the flight and room drags and steps
down, never up, when the phone cannot keep up:

- `full`: the camera flight and the depth room with its shader;
- `reduced`: the flight, and a flat room that only drifts;
- `fade`: a short fade between the market and a flat room.

The tier is remembered per phone. Reduced motion (the system setting) always
draws as `fade`, without changing the stored tier.

**The list view** (the list button, or the search bar) is the same catalog as
a plain list with search, categories and stores: the way to shop for anyone
who does not want the map, and with TalkBack.

### Device checklist

Run on a real mid-range Android phone (2 to 3 GB of RAM), release build
(`flutter run --release`), signed in, on 4G not Wi-Fi:

- [ ] First view of the market under 1.5 s from opening the tab (the base image
      first, then tiles sharpen).
- [ ] The flight into a store with the performance overlay on
      (`flutter run --profile`, then press `P`): no long red bars; note
      the tier it ends on.
- [ ] Room drag: the depth shifts and settles back; products stay on their
      shelves and open on tap.
- [ ] Data used in a ten-minute session of browsing (Android settings, data
      usage for the app): write the number down.
- [ ] TalkBack: every store dot is read with its name, the list button and the
      search are reachable, and a whole purchase-to-cart is possible from the
      list view alone.
- [ ] Text at 200 percent: the list, product page and cart still read; nothing
      is cut off.
- [ ] Reduced motion (Remove animations in Accessibility): a fade instead of a
      flight, no drift in the room.
- [ ] Airplane mode after one visit: the market shows from cache with the
      offline line; opening a room shows its error with a retry.

## Before the first store upload

- Change the Android `applicationId` and the iOS bundle id. The project name
  `market` and organisation `ng.market` are placeholders until the product is
  named; they produced `ng.market.market` in `android/app/build.gradle.kts`
  and in `ios/Runner.xcodeproj/project.pbxproj`. Neither can be changed after
  the app has been published.
- Set real release signing. The Android release build still uses the debug
  key, which is a template default.
- Add the store link to the app config later. The update notice has no store
  link today because `/app/config` has no store URL field (plan 7e adds it).

## Platform floors

- Android `minSdk` 26
- iOS deployment target 15.0

## Install the toolchain

1. Install Flutter stable 3.50.1 (the version the app was built and tested
   with) and put its `bin` folder on your PATH.
2. For Android, install the Android SDK (Android Studio is the easy way) and
   accept the licences yourself with `flutter doctor --android-licenses`.
3. For iOS (macOS only), install Xcode and CocoaPods.
4. Run `flutter doctor` and fix what it reports.

## Run it

From this folder:

```
flutter pub get
dart run build_runner build --delete-conflicting-outputs
flutter run --dart-define=API_BASE_URL=http://10.0.2.2:8080
```

`API_BASE_URL` is the address of the server. An Android emulator reaches the
computer it runs on as `http://10.0.2.2:8080`. The app refuses plain http to
any other address, even in a debug build, so a real phone cannot use
`http://192.168.x.x`: either forward the port over USB (`adb reverse
tcp:8080 tcp:8080`, then `API_BASE_URL=http://localhost:8080`) or point it
at an https server. For local testing
start the server with `OTP_DEV_ECHO=1` so it prints each one-time code to its
stdout instead of sending an SMS.

## Test it

```
flutter analyze
dart format --output=none --set-exit-if-changed .
flutter test
```

Files ending `.g.dart` are generated by `build_runner` and are not committed:
run `dart run build_runner build --delete-conflicting-outputs` after a fresh
checkout and whenever an annotated model changes.

## Project structure

```
lib/
  main.dart            entry point
  app.dart             the root widget (theme, router, the start-up gate)
  router.dart          every route, and the redirects that depend on the session
  core/
    api/               the HTTP client and the typed API errors
    config/            /app/config, the update notice and maintenance gate
    format/            phone and text formatting
    links/             safe links to outside pages
    session/           sign-in state and secure token storage
    theme/             palette, type and the light and dark themes
    ui/                shared widgets (buttons, fields, code boxes, chips)
  features/
    auth/              sign-up, sign-in, phone code, password recovery
    account/           the account page
    addresses/         the address book and the area-council picker
    home/              the signed-in shell
    market/            the market map, the camera flight, rooms and art
    list/              the list view, search and the store page
    product/           the product page
    cart/              the cart and its button
test/                  mirrors lib/; test/wire holds the real-server checks
tool/                  the placeholder art generator
```

## Wire check

The unit tests use a fake HTTP adapter, so they cannot prove the wire contract.
`test/wire/wire_check_test.dart` drives the app's real network code against a
real running server. It is skipped unless both environment variables below are
set, so the normal `flutter test` stays offline.

1. Create a scratch database (never the development one) and start the server
   from `../server` (the server reads the market layout from
   `MARKET_LAYOUT_DIR`, default `market`, relative to where it is started)
   with `APP_ENV=development`, `OTP_DEV_ECHO=1`, that
   database's `DATABASE_URL`, two different generated `FIELD_KEY` and
   `INDEX_KEY` values, and its stdout redirected to a log file. With
   `OTP_DEV_ECHO=1` the server prints each one-time code to stdout.
2. From this folder, set `WIRE_BASE_URL` (for example `http://127.0.0.1:8080`)
   and `WIRE_OTP_LOG` (the path of that log file), then run:

```
flutter test test/wire/wire_check_test.dart
```

**The market wire check** (`test/wire/market_wire_test.dart`) also needs a
store with products and a plot. Apply the seed to the same scratch database first (it is
safe to apply twice), then run it:

```
docker exec -i market-db psql -U market -d <scratch db> < test/wire/fixtures/market_seed.sql
flutter test test/wire/market_wire_test.dart
```

It signs up a fresh buyer, searches for the seeded dress, checks its two sizes
and prices, adds it to the cart and checks the server's total, then loads the
market (`GET /market`) and the store's room (`GET /stores/{slug}/room`)
through the app's market code. The seed puts its store in the fashion district
on plot `fashion-1` (a `store_plots` row), so it stands on the map. The seed's store
has dummy encrypted fields: it is good for browsing and the cart, not for
checkout.

The check refuses any `WIRE_BASE_URL` whose host is not `localhost`,
`127.0.0.1` or `[::1]`: it signs people up and asks for SMS codes, so it must
never run against a shared, staging or live server. There is no override.

Each run creates test accounts and addresses in the scratch database and uses
fresh phone numbers, so it can be repeated against the same scratch database.
Run one check at a time: the code reader takes the latest code in the log.
The log file holds one-time codes, so keep it outside the repository and
delete it, and drop the scratch database, when finished.

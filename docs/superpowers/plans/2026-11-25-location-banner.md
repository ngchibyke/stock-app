# Location Banner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Phones outside the FCT see a dismissible "coming soon to your location" banner on the market; ordering for Abuja delivery is unchanged.

**Architecture:** A pure `inFct(lat, lng)` check, a `LocationCheck` seam (geolocator in the app, a fake in tests) and a small prefs store, read by a Riverpod provider that the market page uses to show a first-open sheet and the banner.

**Tech Stack:** Flutter, Riverpod 3, geolocator, shared_preferences.

**Spec:** `docs/superpowers/specs/2026-11-25-location-banner-design.md`

## Global Constraints

- The position is never stored, logged or sent anywhere. The only stored keys are `location.asked` (bool) and `location.banner_closed_on` (yyyy-mm-dd).
- Android: `ACCESS_COARSE_LOCATION` only (no fine location). iOS: `NSLocationWhenInUseUsageDescription` = "To check whether we deliver where you are. Your location stays on your phone."
- Copy, verbatim:
  - sheet title "We deliver in Abuja";
  - sheet body "Allow location to see if we reach you.";
  - buttons "Allow" and "Not now";
  - banner "Coming soon to your location. You can still order for delivery anywhere in Abuja."
- FCT box: latitude 8.40 to 9.35, longitude 6.75 to 7.62, inclusive.
- Every control gives press feedback, and spacing uses Tokens (as for the rest of the app).

## Review Focus

1. Location services off, or permission denied forever: no banner, no repeated prompt, and no error shown. (Task 2 tests)
2. A fix that never arrives: times out after 10 seconds and shows no banner. (Task 2 test)
3. Closed today, reopened today: hidden. Reopened tomorrow, still outside: shown again. (Task 2 tests)
4. Already asked and said Not now: never asked again, never prompted by the system. (Task 2 test)
5. Points just inside and outside the box edges. (Task 1 tests)

---

### Task 1: The FCT check

**Files:** Create `app/lib/features/location/fct_area.dart`; test `app/test/features/location/fct_area_test.dart`.

**Produces:** `bool inFct(double lat, double lng)`.

- [ ] Tests:
  - Wuse (9.0765, 7.3986), Gwagwalada (8.9400, 7.0800) and the corners (8.40, 6.75) and (9.35, 7.62) are inside;
  - Lagos (6.5244, 3.3792), Kano (12.0022, 8.5920), (8.39, 7.0) and (9.0, 7.63) are outside.
- [ ] Run and see them fail; implement; run and see them pass.
- [ ] Commit `feat: add the FCT area check`.

### Task 2: Check, store, sheet and banner

**Files:**
- Create `app/lib/features/location/location_check.dart` (`enum Area { inside, outside, unknown }`; `abstract class LocationCheck { Future<bool> requestPermission(); Future<Area> area(); }`; `GeolocatorLocationCheck` with low accuracy and a 10 s timeout, where any failure gives `unknown`);
- create `app/lib/features/location/location_prefs.dart` (`asked`, `markAsked()`, `closedOn`, `closeToday(DateTime now)`);
- create `app/lib/features/location/location_banner.dart` (the first-open sheet and the banner);
- modify `app/lib/features/market/market_page.dart` (show both), `app/lib/core/strings.dart` (copy), `AndroidManifest.xml`, `Info.plist`, `pubspec.yaml`;
- test `app/test/features/location/location_banner_test.dart` with a fake `LocationCheck`, a clock, and `SharedPreferences.setMockInitialValues`.

**Tests:**
- not asked: the sheet shows. Not now marks asked, makes no permission request and shows no banner;
- Allow, outside: the banner shows;
- Allow, inside: no banner;
- unknown (refused or timeout): no banner;
- asked, outside: the banner shows without the sheet;
- closed today: hidden today, shown tomorrow.

- [ ] Write the tests; run them and see them fail; implement; run them and see them pass; run `flutter analyze` and the full `flutter test`.
- [ ] Commit `feat: tell phones outside Abuja that we are coming soon`.

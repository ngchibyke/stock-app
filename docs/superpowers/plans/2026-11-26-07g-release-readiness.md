# Part 7g: Release readiness, plan

Design: `docs/superpowers/specs/2026-11-26-07g-release-readiness-design.md` (approved 2026-11-26).
Branch: `feat/release-readiness`. Every task ends with its tests green and a commit in the repo's style.

## Limits to keep in mind

This container has no Android SDK, no Gradle run and no Mac. Anything in `*.gradle.kts`, the iOS project or the workflows is checked here only by reading, by YAML parsing and by a CI job that builds a debug APK. The first CI run is the first proof. Edits to those files are kept small and standard for that reason.

## Differences from the design

- **No Firebase Gradle plugin in the repository.** The Android project uses a very new Android Gradle plugin (9.x). A Google services plugin version cannot be checked against it from here, and a wrong version would break every build. The workflow writes `google-services.json` from a secret when it exists, and `docs/release/release-howto.md` says to add the plugin once with the real Firebase project and check the build. Push on Android stays off until then, as it is today.
- **iOS push entitlement as a file passed at archive time.** `ios/Runner/Release.entitlements` is given to `xcodebuild` by the release workflow, so debug builds need no push capability and the Xcode project is not rewritten from here.

## Tasks

1. **Brand.** `app/brand.json`; `tool/brand.dart` (apply and check logic), `tool/apply_brand.dart` (CLI with `--check`), `tool/placeholder_icon.dart`; the placeholder icon `assets/brand/icon.png`; every Android and iOS icon size generated from it. Tests on a temporary copy of the platform files.
2. **Release guard.** `lib/core/api/release_guard.dart`: a release build refuses an empty, non-https or development API address, at start-up. Tests.
3. **Android.** Release signing from `key.properties` or environment variables, failing with a plain message when a release task runs without them; `POST_NOTIFICATIONS`; `.gitignore` for keys and Firebase files.
4. **iOS.** `UIBackgroundModes` with `remote-notification`, `ITSAppUsesNonExemptEncryption` false, the release entitlements file.
5. **Workflows.** `release.yml` (manual: checks, a signed AAB, an IPA, uploads when their secrets exist) and an Android debug build job in `ci.yml`. YAML parsed and linted here.
6. **Accessibility.** A widget test that walks the main screens and fails on an unlabelled tap target or one under 48 dp.
7. **Docs.** `docs/release/go-live-costs-and-placeholders.md` (requested by the owner on 2026-11-26, written when the roadmap is done: everything to pay for to begin live testing, and every placeholder to replace); `docs/release/`: how to release, the secrets list, listings, Play data safety, Apple privacy labels, review notes, policy points, one ordered device checklist. App README, roadmap, plan status.

## Checks

`dart run build_runner build`, `dart format`, `flutter analyze`, `flutter test`; `dart run tool/apply_brand.dart --check`. Server and dashboard are not touched.

## Status

Built on `feat/release-readiness`: `app/brand.json` with `tool/brand.dart`, `tool/apply_brand.dart` and the placeholder icon; `lib/core/api/release_guard.dart`; release signing in `android/app/build.gradle.kts`; the iOS push settings and `Release.entitlements`; `.github/workflows/release.yml` and the Android debug-build job in `ci.yml`; `test/accessibility_walk_test.dart` and `test/platform_release_test.dart`; `docs/release/` (including the go-live costs and placeholders list).

Follow-ups on the same branch, 2026-11-27, from the go-live list: the store-link config and the update button, `GET /ready`, the public `/account-deletion` page with a browser test, `docs/release/breach-procedure.md` and `docs/release/operations.md`.

As built, also different from the plan as written: the brand tool and its tests are the only part of the platform edits that run here; Gradle, the iOS project and both workflows are checked by reading, YAML parsing and `actionlint` only.

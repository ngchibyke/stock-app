# How to release the app

Design: `docs/superpowers/specs/2026-11-26-07g-release-readiness-design.md`. **The release workflow has never been run.** It was written and linted (`actionlint`) but not executed, because it needs your accounts and secrets. The first real run will probably need a small fix; read its log from the top.

## 1. What has to exist first (all outside the repository)

| Thing | Where it comes from | Used for |
|---|---|---|
| Google Play developer account | play.google.com/console (one-time fee) | Android listing and testing |
| Apple Developer Program membership | developer.apple.com (yearly fee) | iOS listing and TestFlight |
| A Firebase project | console.firebase.google.com | push notices |
| The production server, on https | your hosting | everything the app does |
| The real name, id and logo | you | the store listing |

## 2. Name, id and icon

Edit `app/brand.json`, replace `app/assets/brand/icon.png` with the real logo (square, at least 1024 px, no transparency needed), then:

```bash
cd app
dart run tool/apply_brand.dart
```

That rewrites the Android label and application id, the iOS display name and bundle ids, and every icon size. CI fails (`apply_brand.dart --check`) if a platform file drifts from `brand.json`.

**The Android application id cannot change after the first upload to Google Play, and neither can the iOS bundle id after the first App Store Connect record.** Settle both before then. The Kotlin `namespace` in `android/app/build.gradle.kts` is separate and does not need to change.

## 3. Secrets (GitHub: Settings, Environments, `release`)

Nothing below is ever committed.

| Secret | What it is |
|---|---|
| `ANDROID_KEYSTORE_BASE64` | the upload keystore, base64 (`base64 -w0 upload.jks`) |
| `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` | its passwords and alias |
| `PLAY_SERVICE_ACCOUNT_JSON` | a Play Console service account with release permission (only to upload) |
| `GOOGLE_SERVICES_JSON` | Firebase's Android file (only for push) |
| `IOS_CERT_P12_BASE64`, `IOS_CERT_PASSWORD` | an Apple Distribution certificate, exported as .p12 |
| `IOS_PROFILE_BASE64` | an App Store provisioning profile for the bundle id |
| `IOS_TEAM_ID` | your Apple team id |
| `APP_STORE_CONNECT_KEY_ID`, `APP_STORE_CONNECT_ISSUER_ID`, `APP_STORE_CONNECT_KEY_P8` | an App Store Connect API key (only to upload) |
| `GOOGLE_SERVICE_INFO_PLIST` | Firebase's iOS file (only for push) |

Make the Android upload key once, and keep the file and its passwords somewhere safe (losing the upload key is recoverable through Play support, losing the app signing key is not, which is why Play App Signing holds that one):

```bash
keytool -genkeypair -v -keystore upload.jks -alias upload -keyalg RSA -keysize 2048 -validity 10000
```

## 4. First time in each store (by hand, once)

- **Play Console:** create the app with the final application id, turn on Play App Signing, fill the store listing, the data-safety form and the content rating (drafts in `docs/release/`), and add yourself to an internal testing list. The first release must be created by hand in the console; after that the workflow can add builds as drafts.
- **App Store Connect:** create the app record with the bundle id, fill the privacy labels and the review notes, and add yourself as an internal TestFlight tester. Turn on the Push Notifications capability for the app id.

## 5. Running the workflow

Actions, Release, Run workflow: give the version name (for example `1.0.0`), the production address (must be `https://`), and tick *upload* only when the store secrets exist. The build number is the run number, so every upload is unique. Each job stops at once with the name of any missing secret.

The workflow runs the app's checks, builds, checks the bundle is **not** signed with the debug key, keeps the AAB and IPA as artifacts for 30 days, and uploads to Play internal testing (as a draft) and TestFlight when asked.

## 6. A release build on your own machine

A release build needs the upload key, or it stops with a message. Put `storeFile`, `storePassword`, `keyAlias` and `keyPassword` in `app/android/key.properties` (git ignores it), or set the `ANDROID_KEYSTORE_*` and `ANDROID_KEY_*` variables. For timing and frame checks without the key, use `flutter run --profile`.

```bash
flutter build appbundle --release --dart-define=API_BASE_URL=https://api.example.ng
```

A release build refuses to start pointed at a missing, non-https or development address.

## 7. Push

The Firebase project and its two files are the only missing steps, plus one thing this repository does not do for you: **add the Google services Gradle plugin on Android** (and check the build) when the real `google-services.json` exists. It was left out because its version cannot be checked against this project's very new Android Gradle plugin from a machine without Android tooling, and a wrong version would break every build. iOS needs the APNs key uploaded in Firebase and the Push Notifications capability on the app id. Until then push stays off and the rest of the app works.

## 8. What nobody has verified

- That the workflow runs end to end (it never has).
- That a signed bundle installs, or that either store accepts it.
- The first CI run of the Android debug-build job is the first proof the Gradle changes compile.
- Everything in `device-checklist.md`.

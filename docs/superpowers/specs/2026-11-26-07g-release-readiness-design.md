# Part 7g: Release readiness, design

Status: draft for owner approval, 2026-11-26. Nothing is planned or built until it is approved.
Builds on: everything in part 7 (the app), part 8 (account deletion and export, which the stores require) and the device checklists in `app/README.md`.

## 1. Intent

Turn a working app into one that can be built signed, uploaded to TestFlight and Google Play internal testing, and reviewed by the stores. This part writes the build pipeline, the signing setup, the icons and the store paperwork, and makes the app's name and id changeable in one place. It cannot prove that a signed build installs or that the stores accept it: that needs your Apple and Google accounts, a Mac runner and a phone, and the design says where.

Decisions made with the owner on 2026-11-26:

- **The name is not decided** (roadmap decision D1). The app keeps a **working name**, "Market", and the placeholder id `ng.market.market`. Every name, id and icon lives in one place, so renaming is one change. An Android application id cannot be changed after the first upload to Google Play, so the real id must be settled before then.
- **Both platforms together:** an Android pipeline and an iOS pipeline are both written now. The iOS one cannot be run or checked until the Apple account, certificates and a macOS runner exist.

## 2. Goals, non-goals, anti-goals

**Goals**

- One file, `app/brand.json`, holds the display name, the Android application id, the iOS bundle id and the icon source. A tool applies it to every platform file and a CI check fails if they drift.
- A release build is signed with the real key or it does not build. No release is ever signed with the debug key.
- One manual workflow builds, signs and uploads: an AAB to Play internal testing, an IPA to TestFlight. Secrets are GitHub secrets; none is in the repository.
- A release build talks to the production server over https only, and refuses to start pointed anywhere else.
- Push is wired the way the platforms need (the Android notification permission, the iOS push entitlement, the Firebase files supplied at build time), so adding the Firebase project is the only missing step.
- One ordered device checklist, merged from the README's, that you or a tester can run and mark.
- Store paperwork drafted from what the app actually does: listings, the Play data-safety answers, Apple's privacy labels, review notes, the privacy policy points.

**Non-goals**

- No crash-reporting SDK. It is a decision for you (section 8); until then crashes are seen only through the stores' own reports.
- No final name, logo or screenshots. Placeholders are marked.
- No marketing video (that could use the motion tools later, if you want one).
- No change to how the app looks or works.

**Anti-goals**

- No secret, keystore, certificate, service account or Firebase file in the repository.
- No upload from a developer's machine: only the workflow uploads.
- No claim that a build works on a device or passes review until someone has run it.

## 3. One place for the name, the id and the icon

`app/brand.json` has `display_name`, `application_id`, `bundle_id`, `version_name` and the path of the icon source. `dart run tool/apply_brand.dart` writes them to the Android manifest label and `applicationId`, the iOS display name and bundle ids, and generates every icon size and the launch image from one source picture. `--check` makes it a CI step that fails when a platform file differs from `brand.json`. A placeholder icon is generated from code, like the placeholder market art, and looks like a placeholder.

## 4. Signing

**Android.** The release `signingConfig` reads the upload keystore from a `key.properties` file or from environment variables. If neither exists, a release build **fails with a plain message**; only debug builds may use the debug key. Play App Signing is assumed: the keystore in the secrets is the upload key, and Google holds the app signing key.

**iOS.** The workflow archives and exports with a distribution certificate and a provisioning profile held as secrets, and uploads with an App Store Connect API key. Fastlane is not added; plain `xcodebuild` and `xcrun` keep it inspectable.

## 5. The release workflow

A new `release.yml`, started by hand with the version name. It runs the app's own checks first, then:

- **Android job** (Linux): build the AAB with the production API address, signed, check the signing and the application id, keep it as a build artifact, and upload to Play internal testing with a service account when that secret exists.
- **iOS job** (macOS runner): archive, export, upload to TestFlight when the API key secret exists.

Build numbers come from the run number, so each upload is unique. Each job stops with a plain message naming the missing secret. A separate, ordinary CI job builds a **debug APK** on every push, so a broken Gradle or manifest change is caught early and cheaply.

## 6. The production address and release safety

The API address is passed at build time. A release build with no address, an address that is not https, or the development address fails at build time and again at start-up. The app's existing rule (only loopback http in the wire checks) is unchanged for debug builds.

## 7. Push, privacy strings and platform details

- Android: the notification permission (needed from Android 13), asked after the first paid order as the app already explains; the Google services plugin applied only when the Firebase file is present.
- iOS: the push entitlement and the remote-notification background mode, `ITSAppUsesNonExemptEncryption` set to false (the app uses only standard https), and the existing permission texts reviewed.
- The Firebase files come from secrets at build time. Without them the app builds and push stays off, as today.

## 8. Store paperwork (drafts for you and legal)

All in `docs/release/`, each marked as a draft for review, not legal advice.

- **Listings:** short and long descriptions, keywords, category, a screenshot shot list, with the name as a marked slot.
- **Play data safety** and **Apple privacy labels**, derived from what the app collects: name and phone, saved addresses, order history, photos the person uploads, chat messages, the identity check done through a provider, push tokens, and approximate location that stays on the phone. Account deletion and data export are in the app (part 8).
- **Review notes** and what a reviewer needs: a demo buyer and seller account and how to reach a server that answers them without real payments.
- **Policy points** the privacy policy must carry (the same list as part 8's notice items), the support address and the account-deletion instructions the stores ask for.
- **Content rating and regulated-product answers:** the questionnaire answers, and a flag that the escrow arrangement is regulated by the CBN and the listing must not overstate it.

## 9. Accessibility and performance pass

- Automated: the existing widget tests at 200% text stay; a small test walks the main screens and fails on a missing label for a button or an input, or a target under 48 dp.
- On a device (cannot be run here): TalkBack and VoiceOver on the main flows, 200% text, and the performance aims from the market design: the first market view in about 1.5 seconds on a mid-range Android phone over 4G, a session within a few megabytes, smooth camera flights. These are in the checklist as measurements to record.

## 10. Open questions (defaults stand unless changed)

- **Crash reporting:** none for now. Options later: Firebase Crashlytics (Google), Sentry. Either needs a DPO look at what it collects.
- **Minimum versions:** Android 8 and iOS 15, as the market design set.
- **Support address and policy URL:** placeholders until you provide them; the server already serves them in `/app/config`.
- **Demo account approach:** a staging server with the fake providers is the simplest for reviewers; production with real payments is not suitable.

## 11. Testing and checks

- `apply_brand` has tests on a copy of the platform files: it changes every place, is idempotent, and `--check` fails on a drifted file.
- A test of the release guard: no address, an http address and the development address are all refused.
- The accessibility walk and the existing app suite run in CI.
- The release workflow is checked only by reading and by `actionlint`; it is run for real when your secrets exist. The Android build itself is proved by the debug APK job in CI, which I cannot run here: the first CI run is the first proof.
- All CI jobs as for every part.

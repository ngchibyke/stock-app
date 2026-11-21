# Part 7b: App Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Flutter app, in `app/`, that a person can install on Android or iPhone, sign up with a phone code, sign in, recover a forgotten password, see and keep their delivery addresses, and sign out, with the design system, API client, session handling, navigation and CI that every later piece builds on.

**Architecture:** One Flutter project. Riverpod providers hold state, with repositories behind small interfaces so tests swap in fakes. One `dio` client adds the bearer header, turns the server's error envelope into a typed `ApiError`, and signs the user out on a 401 to an authenticated call. The router is `go_router` with a pure redirect function that decides where a person belongs from their session and the app gate. Every string is in one file. The immersive market (the next plan) plugs into the navigation shell this plan leaves as a placeholder.

**Tech Stack:** Flutter (stable) and Dart, `flutter_riverpod`, `go_router`, `dio`, `json_serializable` with `build_runner` (no `freezed`), `flutter_secure_storage`, `shared_preferences`, `intl`, `package_info_plus`, `url_launcher`. Tests: `flutter_test` with hand-written fakes; no mocking library.

**Spec:** `docs/superpowers/specs/2026-11-21-immersive-market-design.md` (section 10, piece 1: the app foundation), with the app overview `docs/superpowers/plans/2026-11-20-07-mobile-app.md` (architecture decisions and global constraints) and the server it talks to (parts 1 to 7a, routes listed below). The product spec is `docs/superpowers/specs/2026-10-22-abuja-marketplace-design.md`.

## Global Constraints

- One Flutter codebase in `app/` at the repo root, for Android and iOS. **No web build.** Minimum **Android 8 (API 26)** and **iOS 15** (overview decision D6).
- State is **Riverpod**; navigation is **`go_router`**; HTTP is **`dio`**; models are plain immutable Dart classes with **`json_serializable`** (**no `freezed`**); the session token is kept only in **`flutter_secure_storage`**.
- **Money is integer kobo everywhere**, formatted only at the edge with `intl` (`en_NG`, ₦). Never a `double`. Every amount on screen comes from a server field; the app never sends a price or total.
- Personal data (phone, address, name, tokens, codes, passwords) is **never logged, never put in a URL, never kept in plain storage, and never sent to analytics or crash reports**. Addresses are never cached to disk.
- Every list or fetched screen handles four states: **loading** (a skeleton shaped like the screen), **empty** (with a way forward), **error** (with a retry) and **data**. A screen missing a state is not finished.
- Accessibility: every control has a label, tap targets are **at least 48 dp**, text scales to **200 percent without clipping**, colour is never the only signal, and motion respects the system's reduced-motion setting.
- Offline: the app opens, says plainly that it is offline, and never queues a money action for later.
- **English at launch, every user-visible string in `lib/core/strings.dart`.**
- Nothing nudges a buyer to pay outside the app. (Not exercised in this plan; kept so later plans inherit it.)
- Commits are conventional messages with **no AI attribution of any kind** (no `Co-Authored-By`, no "Generated with" line, no session link).
- Run `flutter analyze` and `flutter test` from `app/` after every task; both must be clean before a commit.

## Server routes this plan uses (already built)

All JSON over HTTPS with `Authorization: Bearer <token>`; every error is `{"error": {"code", "message"}}`.

| Route | Request | Success | Errors the app handles |
|---|---|---|---|
| `POST /auth/sign-up` | `{phone, password, display_name}` | 201 `{user_id, verification_id, resend_after_secs}` | 409 `phone_taken`; 422 `validation_failed`, `invalid_phone`; 429 `otp_throttled` (`error.retry_after_secs`); 503 `otp_unavailable` |
| `POST /auth/verify-phone` | `{verification_id, code}` | 200 `{token}` | 422 `otp_invalid`, `otp_expired`, `otp_locked` |
| `POST /auth/sign-in` | `{phone, password}` | 200 `{token}` | 401 `invalid_credentials`; 429 `too_many_attempts`; **403 `phone_unverified` with `verification_id` at the top level of the body** (not inside `error`); 429 `otp_throttled`; 503 `otp_unavailable` |
| `POST /auth/sign-out` | none | 204 | any (best effort) |
| `GET /me` | none | 200 `{id, display_name, phone, role, is_seller}` | 401 |
| `POST /auth/password-reset` | `{phone}` | 202 `{verification_id, resend_after_secs}` for any phone | 422 `invalid_phone`; 429 `too_many_attempts` |
| `POST /auth/password-reset/confirm` | `{verification_id, code, new_password}` | 204 | 422 `code_invalid`, `validation_failed` |
| `GET /app/config` | none | 200 `{min_version: {android, ios}, terms_url, privacy_url, support_email, maintenance: {active, message}}`; any value may be `null` | none |
| `GET /me/addresses` | none | 200 `[Address]`, newest first | 401 |
| `POST /me/addresses` | `{recipient_name, phone, address_line, landmark?, area_council, postcode?}` | 201 `Address` | 422 `validation_failed`, `invalid_phone`, `outside_service_area`, `invalid_postcode`, `too_many_addresses` |
| `DELETE /me/addresses/{id}` | none | 204 | 404 |

`Address` is `{id, area_council, postcode, courier_address_code, recipient_name, phone, address_line, landmark}`. `area_council` is one of `amac`, `bwari`, `gwagwalada`, `kuje`, `kwali`, `abaji`. Limits: name 100, address line 300, landmark 200 characters; at most 20 addresses; a password is 10 to 128 characters; a display name is 1 to 80. The server's resend of a sign-up code is a sign-in with the password on the unverified account.

## Review Focus

1. **A dead session.** A revoked or expired token must sign the person out once, cleanly, from any screen, with no retry loop, and a corrupt or unreadable secure-storage value must count as signed out, never a crash. (Tasks 4, 5)
2. **Code entry under stress.** Wrong, expired and locked codes; a resend that is throttled; the app backgrounded during the countdown (the timer must use a deadline, not tick counts); a double tap on submit; a pasted code with spaces. (Task 7)
3. **Account enumeration.** "Forgot password" says the same thing for every phone, and sign-in never says whether a phone exists. (Tasks 7, 9)
4. **Offline and slow networks.** No network, a timeout, or a 5xx shows a plain message with a retry on every screen and never a crash; the app-config check fails open (it can never lock someone out because it could not be reached). (Tasks 4, 6)
5. **Awkward input.** Phones typed as `0803...`, `+234 803...`, with spaces; passwords with leading or trailing spaces (never trimmed); very long names and addresses; text at 200 percent; the keyboard covering a field. (Tasks 2, 7, 11)

---

## File structure

```
app/
  pubspec.yaml, analysis_options.yaml, README.md, .gitignore
  lib/
    main.dart                      entry: ProviderScope, runApp
    app.dart                       MaterialApp.router, theme, gate wrapper
    router.dart                    GoRouter + redirectFor
    core/
      strings.dart                 every user-visible string
      theme/tokens.dart, theme.dart
      ui/states.dart               Skeleton, EmptyState, ErrorState, LoadingButton, AppTextField
      format/money.dart, phone.dart
      api/api_error.dart, api_client.dart, providers.dart
      session/me.dart, session_store.dart, session_controller.dart
      config/app_config.dart, app_gate.dart
    features/
      auth/ auth_repository.dart, sign_up_page.dart, verify_page.dart, sign_in_page.dart,
            forgot_page.dart, reset_page.dart, countdown.dart
      account/ account_page.dart
      home/ home_page.dart, shell.dart
      addresses/ address.dart, address_repository.dart, address_list_page.dart, address_form_page.dart
  test/ (mirrors lib/, plus test/support/fakes.dart)
.github/workflows/ci.yml            + an `app` job
```

---

### Task 1: Toolchain, project skeleton and CI

**Files:**
- Create: `app/` (via `flutter create`), `app/analysis_options.yaml`, `app/test/smoke_test.dart`, `app/README.md`
- Modify: `.github/workflows/ci.yml`, `README.md` (a short "The phone app" section under "Running it locally")

**Interfaces:**
- Produces: a buildable project whose `pubspec.yaml` already lists every package in the Tech Stack line; `flutter analyze` and `flutter test` run from `app/`.

- [ ] **Step 1: Install the toolchain on this machine.** There is no Flutter or Dart on this machine; an Android SDK folder exists under `%LOCALAPPDATA%\Android\Sdk`. Install the current stable Flutter SDK, point it at that Android SDK, accept the Android licences, and run `flutter doctor`. Expected: the Flutter and Android toolchain lines are green. (Visual Studio and Xcode lines may stay red: no Windows desktop build, and iOS builds run on a macOS CI runner in plan 7e.) Ask the owner before downloading anything.
- [ ] **Step 2: Create the project.** From the repo root: `flutter create --platforms android,ios --org ng.market --project-name market app`, then delete the generated counter app and its test. `ng.market` and the name "market" are placeholders until the product is named (overview decision D1): record in `app/README.md` that the Android `applicationId` and iOS bundle id must be set to their final values **before the first store upload**, because they cannot be changed after.
- [ ] **Step 3: Pin the platform floors and add the dependencies.** Android `minSdk 26`; iOS deployment target `15.0`; add the Tech Stack packages (`dev_dependencies`: `build_runner`, `json_serializable`, `flutter_lints`).
- [ ] **Step 4: Write the smoke test** `test/smoke_test.dart::app_builds_a_material_app`: pump `MaterialApp(home: SizedBox())` inside a `ProviderScope` and expect it finds a `MaterialApp`. Run `flutter test`; expected: PASS.
- [ ] **Step 5: Add the CI job.** In `.github/workflows/ci.yml` add job `app` (name `App`) on `ubuntu-latest`, `defaults.run.working-directory: app`, steps: checkout, `subosito/flutter-action@v2` (stable channel, cache on), `flutter pub get`, `dart run build_runner build --delete-conflicting-outputs`, `flutter analyze`, `flutter test`. Generated `*.g.dart` files are not committed (add to `app/.gitignore`).
- [ ] **Step 6: Commit.** `feat: add the Flutter app project and its CI job`

---

### Task 2: Design tokens, theme, strings and the shared state widgets

**Files:**
- Create: `app/lib/core/theme/tokens.dart`, `app/lib/core/theme/theme.dart`, `app/lib/core/strings.dart`, `app/lib/core/ui/states.dart`, `app/lib/core/ui/brand_header.dart`, `app/lib/core/ui/code_boxes.dart`
- Test: `app/test/core/ui/states_test.dart`, `app/test/core/ui/brand_header_test.dart`, `app/test/core/ui/code_boxes_test.dart`

**Interfaces:**
- Produces:
  - `class Tokens` with spacing (`s1 = 4`, `s2 = 8`, `s3 = 12`, `s4 = 16`, `s5 = 24`, `s6 = 32`), `radius = 14` (cards and fields), `sheetRadius = 24`, `minTap = 48`, and colour roles for light and dark. **The owner chose palette A, "Abuja Dusk" (2026-11-21), after studying PiggyVest and Chowdeck on iPhone** (see the design kit in the immersive-market design). Light values: `brand #14193D`, `brand2 #232B63`, `accent #E2662D` (primary buttons and focus), `gold #F5B83D`, `bg #FBF7F2`, `card #FFFFFF`, `ink #14193D`, `mute #6B7089`, `line #ECE6DE`, `good #1E9E6A`, `bad #D4383F`; status chip colours are paid (`#E7EAFB` on `#2B3AA8`), on the way (`#FFE9DC` on `#B44A14`), delivered (`#DDF5EA` on `#12805A`), report open (`#FDE2E3` on `#B3262E`). Dark values are derived: `bg #0E1127`, `card #171B3A`, `ink #F3F0EA`, `mute #9AA0BD`, `line #2A2F55`, the same `accent` and `gold`, `brand` lightened to `#8F98F0` for text and icons on dark.
  - `ThemeData lightTheme()`, `ThemeData darkTheme()`; the app follows the system setting. Type is the platform system font (San Francisco on iPhone, Roboto on Android) in heavy weights for headlines (w800) and big numerals; a bundled brand font is a later, separately approved download. Scale: display 32, title 22, body 16, caption 13. Primary buttons are full-width pills in `accent`; secondary buttons are pills with a `line` border; fields have a `line` border that turns `accent` on focus and `bad` on error.
  - `abstract final class Strings` holding every user-visible string as a `static const` or a static method for strings with values.
  - `Skeleton({double? height, double? width})`, `EmptyState({required String title, String? body, String? actionLabel, VoidCallback? onAction})`, `ErrorState({required String message, VoidCallback? onRetry})`, `LoadingButton({required String label, required VoidCallback? onPressed, bool loading = false})`, `AppTextField({required String label, TextEditingController? controller, String? errorText, bool obscure = false, TextInputType? keyboardType, TextInputAction? action, int? maxLength, List<TextInputFormatter>? formatters, ValueChanged<String>? onChanged, ValueChanged<String>? onSubmitted})`.
  - `BrandHeaderScaffold({required String title, String? subtitle, required Widget child, bool showBack = false})`: the kit's entry-screen layout, a `brand` to `brand2` gradient header (wordmark "MARKET" until the product is named, title in display type, subtitle) with the `bg` sheet rising from the bottom with `sheetRadius` top corners; the sheet scrolls and keeps the focused field above the keyboard. Used by sign-in, sign-up, code, forgot and reset screens.
  - `CodeBoxes({int length = 6, required ValueChanged<String> onChanged, ValueChanged<String>? onCompleted, String? errorText})`: the kit's row of digit boxes (46 by 54, `radius`, `accent` border on the active box, `bad` on error), backed by one hidden text field so paste and the system one-time-code autofill work; it accepts only digits, strips pasted spaces, and ignores input beyond `length`. Used by the sign-up code and reset code screens.

- [ ] **Step 1: Write the failing tests** in `states_test.dart` (and `code_boxes_test.dart`, `brand_header_test.dart` for the two kit widgets): `empty_state_shows_title_and_action` (taps the action once); `error_state_retry_is_tappable_and_labelled` (a `Semantics` label is present); `loading_button_ignores_taps_while_loading` (two taps, zero calls); `controls_meet_the_48dp_tap_target` (the button and the retry button have a size of at least 48 by 48); `states_do_not_overflow_at_200_percent_text` (pump each widget with `MediaQuery(textScaler: TextScaler.linear(2.0))` in a 320-wide box and expect no overflow exception); `skeleton_does_not_animate_when_reduced_motion_is_on` (with `MediaQuery(disableAnimations: true)` the skeleton has no running ticker). In `code_boxes_test.dart`: `code_boxes_accept_only_digits_strip_pasted_spaces_and_ignore_overflow` (typing `12a 3456 78` ends as `123456`); `code_boxes_report_completion_once_at_full_length`; `code_boxes_show_the_error_text_and_not_colour_alone`; `code_boxes_boxes_meet_the_tap_target_and_do_not_overflow_at_200_percent_text`. In `brand_header_test.dart`: `the_header_shows_title_and_subtitle_and_the_sheet_holds_the_child`; `the_sheet_scrolls_so_a_focused_field_stays_above_the_keyboard` (pump with `viewInsets.bottom = 300`, no overflow); `the_back_button_is_labelled_and_48dp_when_shown`; `the_theme_gives_primary_buttons_the_accent_pill_and_fields_the_line_border`.
- [ ] **Step 2: Run them.** Expected: FAIL (files missing).
- [ ] **Step 3: Implement** the tokens, themes, `Strings` (include the four-state copy and a generic "Something went wrong. Check your connection and try again." and an "You're offline" message) and the widgets. The skeleton pulses gently unless reduced motion is on.
- [ ] **Step 4: Run the tests.** Expected: PASS. Run `flutter analyze`.
- [ ] **Step 5: Commit.** `feat: add the app theme, strings and state widgets`

---

### Task 3: Money and phone formatting

**Files:**
- Create: `app/lib/core/format/money.dart`, `app/lib/core/format/phone.dart`
- Test: `app/test/core/format/money_test.dart`, `app/test/core/format/phone_test.dart`

**Interfaces:**
- Produces: `String formatNaira(int kobo)`; `String displayPhone(String e164)`; `bool looksLikeNigerianPhone(String input)`.

- [ ] **Step 1: Write the failing tests.** `formatNaira`: `1200000 -> "₦12,000"`, `1250 -> "₦12.50"`, `0 -> "₦0"`, `100 -> "₦1"`, `99 -> "₦0.99"`, `1000000000 -> "₦10,000,000"`; it never goes through a `double` for the whole part (assert a value above 2^53 kobo still prints exactly: `9007199254740993 -> "₦90,071,992,547,409.93"`). `displayPhone`: `"+2348031234567" -> "0803 123 4567"`; anything it cannot parse comes back unchanged. `looksLikeNigerianPhone` accepts `08031234567`, `+2348031234567`, `+234 803 123 4567`, `0803-123-4567`, `2348031234567`, and rejects `12345`, `""`, `0803123456` (too short), `abcdefghijk`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement** with integer arithmetic (whole naira by `~/ 100`, kobo by `% 100`, grouping with `intl`'s `NumberFormat.decimalPattern('en_NG')` applied to the integer part); the phone check strips spaces and hyphens and requires 11 digits starting `0`, or 13 digits starting `234`, optionally with a leading `+`. It is only a hint for the form; the server normalises and decides.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add naira and phone formatting`

---

### Task 4: The API client and typed errors

**Files:**
- Create: `app/lib/core/api/api_error.dart`, `app/lib/core/api/api_client.dart`, `app/lib/core/api/providers.dart`, `app/test/support/fakes.dart`
- Test: `app/test/core/api/api_client_test.dart`

**Interfaces:**
- Produces:
  - `class ApiError implements Exception { int status; String code; String message; int? retryAfterSecs; String? verificationId; bool get isOffline; bool get isTimeout; bool get isServerDown; }` with `ApiError.fromResponse(int status, Object? body)` (reads `error.code`, `error.message`, `error.retry_after_secs`, and a **top-level** `verification_id`), `ApiError.offline()` (code `offline`, status 0) and `ApiError.timeout()` (code `timeout`).
  - `class ApiClient { ApiClient({required String baseUrl, required Future<String?> Function() readToken, required void Function() onUnauthorized, HttpClientAdapter? adapter}); Future<Object?> get(String path, {Map<String, Object?>? query, bool auth = true}); Future<Object?> post(String path, {Object? body, bool auth = true}); Future<Object?> put(String path, {Object? body}); Future<Object?> delete(String path); }` returning decoded JSON (`Map`, `List`) or `null` for 204.
  - `final apiClientProvider` (Riverpod), base URL from `String.fromEnvironment('API_BASE_URL', defaultValue: 'http://10.0.2.2:8080')`.
  - In `test/support/fakes.dart`: `FakeAdapter` (an `HttpClientAdapter` that answers queued `(status, body)` responses, throws a connection error or a timeout on request, and records each request's method, path, headers and body) and `MemorySessionStore` (added in Task 5).

- [ ] **Step 1: Write the failing tests:** `sends_the_bearer_header_only_for_authenticated_calls`; `error_envelope_becomes_an_api_error_with_code_and_message`; `retry_after_secs_is_read_from_the_error_object`; `verification_id_is_read_from_the_top_level_of_a_403`; `a_401_on_an_authenticated_call_signs_out_once` (two parallel 401s call `onUnauthorized` exactly once); `a_401_on_an_unauthenticated_call_does_not_sign_out` (a wrong password); `a_204_is_null`; `no_connection_is_an_offline_error_and_a_timeout_is_a_timeout_error`; `a_non_json_error_body_still_gives_an_api_error` (HTML from a proxy, status 502); `requests_time_out_after_15_seconds`; `a_non_https_base_url_is_refused_outside_debug_builds_and_localhost_hosts` (plain http only for `10.0.2.2`, `localhost`, `127.0.0.1` and only in debug); `no_request_or_response_content_is_logged` (the client installs no log interceptor, asserted by inspecting `dio.interceptors`).
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement** `ApiError` and `ApiClient` over `dio` with a single interceptor for the header and the error mapping; JSON in and out; `Accept: application/json`; connect and receive timeouts 15 seconds; no redirects followed (`followRedirects: false`); the unauthorized callback is guarded so concurrent 401s fire it once until the next token is read.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add the API client with typed errors`

---

### Task 5: Session storage and the session controller

**Files:**
- Create: `app/lib/core/session/me.dart`, `app/lib/core/session/session_store.dart`, `app/lib/core/session/session_controller.dart`
- Modify: `app/test/support/fakes.dart`
- Test: `app/test/core/session/session_controller_test.dart`

**Interfaces:**
- Consumes: `ApiClient`, `ApiError` (Task 4).
- Produces:
  - `class Me { String id; String displayName; String phone; String role; bool isSeller; }` with `Me.fromJson` (keys `id`, `display_name`, `phone`, `role`, `is_seller`).
  - `abstract class SessionStore { Future<String?> read(); Future<void> write(String token); Future<void> clear(); }`; `SecureSessionStore` (over `flutter_secure_storage`, key `session.token`, a read that throws returns `null` and clears the entry); `MemorySessionStore` in the fakes.
  - `sealed class SessionState`: `SessionSignedOut()`, `SessionSignedIn(Me me)`; `sessionProvider` is an `AsyncNotifierProvider<SessionController, SessionState>` whose first value is the restored state (loading while it reads).
  - `SessionController`: `Future<void> signInWithToken(String token)` (stores the token, loads `/me`; if `/me` fails the token is cleared and the error rethrown), `Future<void> signOut()` (best-effort `POST /auth/sign-out`, then clear, whatever the result), `void handleUnauthorized()` (clears and signs out without calling the server).

- [ ] **Step 1: Write the failing tests:** `restores_a_signed_in_session_from_a_stored_token`; `no_stored_token_means_signed_out`; `a_corrupt_or_unreadable_store_means_signed_out_and_is_cleared`; `a_stored_token_the_server_rejects_means_signed_out_and_clears_it` (401 on `/me` at start); `sign_in_with_token_stores_it_and_loads_me`; `a_failed_me_call_after_sign_in_clears_the_token`; `sign_out_clears_even_when_the_server_call_fails`; `an_unauthorized_response_from_any_call_signs_out_once`; `the_token_is_never_written_anywhere_but_the_store` (the fake adapter's recorded URLs and the logs contain no token).
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement** the model, the stores and the controller; wire `ApiClient`'s `readToken` to the store and `onUnauthorized` to `handleUnauthorized`.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add secure session storage and the session controller`

---

### Task 6: App config and the gate

**Files:**
- Create: `app/lib/core/config/app_config.dart`, `app/lib/core/config/app_gate.dart`
- Test: `app/test/core/config/app_gate_test.dart`

**Interfaces:**
- Consumes: `ApiClient` (Task 4); `package_info_plus` for the running version.
- Produces:
  - `class AppConfig { String? minVersionAndroid, minVersionIos, termsUrl, privacyUrl, supportEmail; bool maintenanceActive; String? maintenanceMessage; }` with `AppConfig.fromJson`; `AppConfig.empty` (all null, maintenance off).
  - `class AppConfigRepository { Future<AppConfig> load(); AppConfig? get lastGood; }` (`load` returns the last good value if the call fails, else `AppConfig.empty`; the last good value is kept in `shared_preferences` as JSON, since it holds nothing personal).
  - `enum Gate { open, updateRequired, maintenance }`; `Gate evaluateGate(AppConfig config, {required String platform, required String version})`; `int compareVersions(String a, String b)`.
  - `appConfigProvider` and `gateProvider`.

- [ ] **Step 1: Write the failing tests:** `compare_versions_orders_numerically` (`1.10.0 > 1.9.0`, `1.0.0 == 1.0.0`); `a_version_below_the_minimum_requires_an_update` (per platform: android and ios read their own field); `a_null_minimum_never_blocks`; `maintenance_wins_over_everything_and_carries_its_message`; `an_unreachable_server_fails_open_with_the_last_good_config_or_empty` (never `maintenance` or `updateRequired` from a failure); `a_malformed_version_in_the_config_does_not_block` (`"abc"` is ignored).
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.** The config is fetched once at launch, without auth, and again when the app returns to the foreground; a fresh answer replaces the stored one.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add the app config and version gate`

---

### Task 7: Sign-up, code verification and the auth repository

**Files:**
- Create: `app/lib/features/auth/auth_repository.dart`, `app/lib/features/auth/countdown.dart`, `app/lib/features/auth/sign_up_page.dart`, `app/lib/features/auth/verify_page.dart`
- Test: `app/test/features/auth/auth_repository_test.dart`, `app/test/features/auth/sign_up_flow_test.dart`

**Interfaces:**
- Consumes: `ApiClient`, `ApiError` (Task 4); `SessionController.signInWithToken` (Task 5); `Strings`, `AppTextField`, `LoadingButton` (Task 2); `looksLikeNigerianPhone` (Task 3).
- Produces:
  - `class CodeChallenge { String verificationId; int resendAfterSecs; }`
  - `abstract class AuthRepository { Future<CodeChallenge> signUp({required String phone, required String password, required String displayName}); Future<String> verifyPhone({required String verificationId, required String code}); Future<String> signIn({required String phone, required String password}); Future<CodeChallenge> requestPasswordReset(String phone); Future<void> confirmPasswordReset({required String verificationId, required String code, required String newPassword}); }`. `signIn` returns the token, or throws `PhoneUnverified(CodeChallenge)` for a 403 `phone_unverified` (the id is at the top level of the body).
  - `class Countdown` : a deadline-based timer, `Countdown(Duration total, {Clock? clock})`, `Duration get remaining`, `bool get done`, `Stream<Duration> get ticks`. It is built from `DateTime` deadlines so it stays right after the app is backgrounded.
  - Routes `/sign-up` and `/verify` (the verify page takes a `CodeChallenge`, the phone and, in memory only, the password so it can request a resend).

- [ ] **Step 1: Write the failing repository tests:** `sign_up_posts_the_exact_body_and_returns_the_challenge`; `verify_returns_the_token`; `sign_in_on_an_unverified_account_throws_phone_unverified_with_the_top_level_id`; `a_throttled_sign_up_surfaces_retry_after_secs`; `passwords_are_sent_exactly_as_typed` (a password with leading and trailing spaces reaches the body unchanged).
- [ ] **Step 2: Write the failing flow tests** (widget tests with fakes): `sign_up_validates_before_calling_the_server` (a short password, an empty name, a phone that does not look Nigerian each show a field error and make no call); `phone_taken_shows_the_server_message_and_a_sign_in_link`; `a_good_sign_up_goes_to_the_code_page`; `the_code_screen_uses_CodeBoxes_and_accepts_only_six_digits_and_strips_pasted_spaces`; `submitting_twice_sends_one_request` (double tap); `wrong_expired_and_locked_codes_each_show_their_own_plain_message_and_keep_the_field`; `resend_is_disabled_until_the_deadline_and_enabled_after` (using a fake clock); `the_countdown_survives_the_app_being_backgrounded` (advance the clock by more than the interval without ticks; the resend becomes available); `a_throttled_resend_shows_the_wait_from_retry_after_secs_and_sends_nothing`; `a_provider_outage_503_shows_the_try_again_message`; `a_verified_account_signs_in_and_leaves_the_flow`; `offline_shows_the_offline_message_with_retry`; `text_at_200_percent_does_not_overflow` for both pages.
- [ ] **Step 3: Run them.** Expected: FAIL.
- [ ] **Step 4: Implement** the repository over `ApiClient`, the countdown, and the two pages. Validation mirrors the server (password 10 to 128 characters and never trimmed; display name trimmed, 1 to 80). Resend on the code page is `AuthRepository.signIn` with the in-memory password, and the page adopts the new `verification_id` it gets back in `PhoneUnverified`. The password is held in a widget-level field that is cleared when the page is disposed and is never written to storage or logs.
- [ ] **Step 5: Run the tests.** Expected: PASS.
- [ ] **Step 6: Commit.** `feat: add sign-up and phone code verification`

---

### Task 8: Sign-in and the router

**Files:**
- Create: `app/lib/features/auth/sign_in_page.dart`, `app/lib/router.dart`, `app/lib/app.dart`, `app/lib/main.dart`
- Test: `app/test/router_test.dart`, `app/test/features/auth/sign_in_page_test.dart`

**Interfaces:**
- Consumes: `AuthRepository`, `PhoneUnverified` (Task 7); `sessionProvider` (Task 5); `gateProvider`, `Gate` (Task 6).
- Produces:
  - `String? redirectFor({required AsyncValue<SessionState> session, required Gate gate, required String location})` (pure).
  - `GoRouter buildRouter(ProviderContainer container)`. This task registers `/sign-in`, `/sign-up`, `/verify` and a bare `/home` (a `Scaffold` that says "Home", replaced by Task 10). Each later task adds its own routes in `router.dart`: Task 9 `/forgot` and `/reset`, Task 10 the shell with `/home` and `/account`, Task 11 `/addresses` and `/addresses/new`. `redirectFor` already treats `/sign-in`, `/sign-up`, `/verify`, `/forgot` and `/reset` as the pages a signed-out person may open.
  - `app.dart`: `MarketApp` (a `ConsumerWidget`) using `MaterialApp.router`, the Task 2 themes, and a full-screen notice when the gate is `updateRequired` or `maintenance` (with the store link or the server's message; no navigation behind it).

- [ ] **Step 1: Write the failing tests.** `redirectFor`: `loading_session_stays_on_a_splash`; `signed_out_is_sent_to_sign_in_from_any_protected_page`; `signed_out_can_open_sign_up_forgot_and_reset`; `signed_in_is_sent_home_from_the_auth_pages`; `a_gate_of_update_required_or_maintenance_overrides_every_route`; `an_unknown_location_goes_home_or_to_sign_in`. Sign-in page: `wrong_password_shows_one_message_that_does_not_say_whether_the_phone_exists` (the same text for `invalid_credentials` on any phone); `too_many_attempts_shows_the_wait_message`; `an_unverified_account_goes_to_the_code_page_with_the_returned_id`; `a_throttled_resend_on_an_unverified_account_shows_the_wait_and_does_not_navigate`; `signing_in_twice_quickly_sends_one_request`; `leading_and_trailing_spaces_in_the_password_are_kept`; `offline_shows_the_offline_message_with_retry`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement** the page, the router (the redirect reads the session and gate through a `Listenable` so it re-runs when they change) and `main.dart` (`ProviderScope`; secure-storage and package-info providers overridden for production, plain defaults for tests).
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add sign-in and the router with its redirect`

---

### Task 9: Password recovery

**Files:**
- Create: `app/lib/features/auth/forgot_page.dart`, `app/lib/features/auth/reset_page.dart`
- Test: `app/test/features/auth/recovery_flow_test.dart`

**Interfaces:**
- Consumes: `AuthRepository.requestPasswordReset`, `confirmPasswordReset`, `CodeChallenge`, `Countdown` (Task 7); `looksLikeNigerianPhone` (Task 3).

- [ ] **Step 1: Write the failing tests:** `the_forgot_page_gives_the_same_message_for_any_phone` (two different phones, an unknown one and a known one, show identical text and both go to the reset page); `a_badly_formatted_phone_is_caught_before_the_call`; `too_many_requests_shows_the_wait_message`; `the_reset_page_validates_the_new_password_like_sign_up_and_the_confirmation_must_match`; `a_wrong_or_expired_code_shows_one_message_and_keeps_the_fields` (`code_invalid`); `a_weak_password_rejected_by_the_server_keeps_the_code_field_so_it_can_be_retried`; `a_good_reset_returns_to_sign_in_with_a_confirmation_message_and_no_session`; `resend_waits_for_the_deadline`; `double_submit_sends_one_request`; `the_new_password_is_never_trimmed_or_logged`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement** both pages. The forgot page always moves to the reset page with the returned challenge (a decoy id for unknown phones is indistinguishable by design), and says "If that number has an account, we have sent a code."
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add password recovery`

---

### Task 10: The navigation shell, home placeholder and the account page

**Files:**
- Create: `app/lib/features/home/shell.dart`, `app/lib/features/home/home_page.dart`, `app/lib/features/account/account_page.dart`
- Test: `app/test/features/account/account_page_test.dart`, `app/test/features/home/shell_test.dart`

**Interfaces:**
- Consumes: `sessionProvider`, `Me` (Task 5); `appConfigProvider` (Task 6); `displayPhone` (Task 3); `package_info_plus`; `url_launcher`.
- Produces: `Shell` (a bottom navigation with **Market** and **Account** tabs, the immersive market's future home); `HomePage` is a placeholder stating the market opens here and linking to the address book; `AccountPage` shows the person's name, phone (via `displayPhone`), role, the app version, links to the terms and privacy pages and the support address (each only when its config value is not null, opened outside the app), a link to **Addresses**, and **Sign out**.

- [ ] **Step 1: Write the failing tests:** `the_account_page_shows_name_phone_role_and_version`; `terms_privacy_and_support_rows_appear_only_when_configured`; `sign_out_asks_nothing_twice_and_returns_to_sign_in` (taps once, session cleared, router at `/sign-in`); `the_account_page_has_its_four_states` (loading skeleton while `/me` is being restored, error with retry, data; empty is not applicable and is asserted as absent); `the_shell_keeps_each_tabs_state_when_switching`; `tabs_have_labels_and_48dp_targets`; `text_at_200_percent_does_not_overflow`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement** the shell (`StatefulShellRoute` in `router.dart`) and the pages.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add the shell, home placeholder and account page`

---

### Task 11: The address book

**Files:**
- Create: `app/lib/features/addresses/address.dart`, `app/lib/features/addresses/address_repository.dart`, `app/lib/features/addresses/address_list_page.dart`, `app/lib/features/addresses/address_form_page.dart`
- Test: `app/test/features/addresses/address_repository_test.dart`, `app/test/features/addresses/address_pages_test.dart`

**Interfaces:**
- Consumes: `ApiClient`, `ApiError` (Task 4); `AppTextField`, state widgets (Task 2); `looksLikeNigerianPhone`, `displayPhone` (Task 3).
- Produces:
  - `enum AreaCouncil { amac, bwari, gwagwalada, kuje, kwali, abaji }` with `String get wire` (`amac`, ...) and `String get label` (`AMAC (Abuja Municipal)`, `Bwari`, `Gwagwalada`, `Kuje`, `Kwali`, `Abaji`).
  - `class Address { String id; AreaCouncil areaCouncil; String? postcode; String recipientName; String phone; String addressLine; String? landmark; }` with `Address.fromJson`; `class NewAddress { ... }` with `toJson` (keys as the server's `NewAddress`).
  - `abstract class AddressRepository { Future<List<Address>> list(); Future<Address> create(NewAddress a); Future<void> remove(String id); }`; `addressRepositoryProvider` and an `addressesProvider` (`AsyncNotifier<List<Address>>`) that holds the list **in memory only**.

- [ ] **Step 1: Write the failing repository tests:** `list_parses_every_field_including_nulls`; `create_posts_the_exact_body_with_the_wire_council_name`; `remove_calls_delete_and_the_list_drops_the_row`; `server_errors_map_to_api_errors` (`outside_service_area`, `invalid_postcode`, `too_many_addresses`, `validation_failed`).
- [ ] **Step 2: Write the failing page tests:** `the_list_has_loading_empty_error_and_data_states` (empty offers "Add an address"); `the_list_shows_the_council_label_and_never_a_raw_wire_value`; `delete_asks_to_confirm_and_cancel_does_nothing`; `a_failed_delete_keeps_the_row_and_shows_a_message`; `the_form_validates_lengths_like_the_server` (name 100, line 300, landmark 200; a longer value is stopped at the field, a blank required one shows an error); `the_form_requires_a_council_and_offers_only_the_six`; `a_postcode_is_optional_and_a_malformed_one_shows_the_server_message`; `the_server_error_messages_are_shown_in_plain_words` (`outside_service_area`, `invalid_postcode`, `too_many_addresses`); `saving_twice_sends_one_request`; `the_form_stays_usable_with_the_keyboard_up_and_at_200_percent_text` (the focused field scrolls into view; no overflow); `addresses_are_never_written_to_disk_or_logs` (the shared-preferences and secure-storage fakes receive no writes, and nothing prints); `very_long_names_and_lines_wrap_instead_of_clipping`.
- [ ] **Step 3: Run them.** Expected: FAIL.
- [ ] **Step 4: Implement** the model, repository and two pages. The form has a labelled field for each of recipient name (defaulting to the person's display name), phone (defaulting to their own), address line, landmark (optional), a council picker, and an optional postcode with the hint that the new 11-character code is expected, or leave it blank.
- [ ] **Step 5: Run the tests.** Expected: PASS.
- [ ] **Step 6: Commit.** `feat: add the address book`

---

### Task 12: Wrap-up, documentation and the end-to-end check

**Files:**
- Modify: `app/README.md`, `README.md`, `docs/superpowers/plans/2026-11-20-07-mobile-app.md`, `docs/superpowers/plans/2026-10-23-00-roadmap.md`

- [ ] **Step 1: Run everything.** From `app/`: `dart run build_runner build --delete-conflicting-outputs`, `flutter analyze`, `flutter test`. Expected: clean, all pass, no test skipped.
- [ ] **Step 2: Run it by hand against a real server.** Start the database and the API (`OTP_DEV_ECHO=1`, so the code is printed to the server's stdout), start an Android emulator or plug in a phone (the host is `http://10.0.2.2:8080` from an emulator), run `flutter run`, and walk through: sign up, enter the printed code, land on the shell, add an address, sign out, sign in, forgot password and reset, sign in with the new password. Write down anything that breaks and fix it before going on. If no emulator or device is available on this machine, say so plainly and name this step as not run; do not report it as done.
- [ ] **Step 3: Check for accidental data leaks.** Search `app/lib` for `print(`, `debugPrint(`, `log(` and `LogInterceptor`; none may exist outside tests. Expected: no matches.
- [ ] **Step 4: Update the docs.** `app/README.md`: how to install the toolchain, run, test, and pass `--dart-define=API_BASE_URL=...`; the placeholder application id warning. `README.md` "The phone app" section. In the part 7 overview, mark 7b "built" with its test count and note that 7b's "home, search, store and product pages" moved into the immersive market piece (design `2026-11-21-immersive-market-design.md`). In the roadmap, update the Part 7 row the same way.
- [ ] **Step 5: Commit.** `feat: finish the app foundation` and `docs: document the app foundation`

---

## Self-review

- **Spec coverage:** the spec's piece 1 (foundation: project and tooling, design system, API client and session, navigation, sign-up and recovery, address book) maps to Tasks 1 to 11; the Global Constraints cover the overview's decisions and rules. The immersive market, store rooms, server layout work and selling and buying screens are other pieces (spec section 10) and are not in this plan by design.
- **Placeholders:** none. The only deferred values are the application id and bundle id (the product has no name yet, overview D1), flagged in Task 1 and Task 12.
- **Type consistency:** `CodeChallenge`, `PhoneUnverified`, `ApiError`, `Countdown`, `Gate`, `AppConfig`, `Me`, `SessionState`, `AddressRepository` and `AreaCouncil` are defined in the task that first needs them and used with the same names after.
- **Not run on this machine:** there is no Flutter toolchain, emulator or iPhone here today. Tasks 1 and 12 say so and Task 12's hand check must be reported as not run if it cannot happen.

## Decisions made during execution

- The status chip colour for `delivered` has foreground `#0F6F4E` (was `#12805A`) so it meets AA contrast.
- An `errorText` colour role exists: light `#B3262E`, dark equal to `bad`.
- The start-up gate's failure fallback never carries maintenance: a saved update-required minimum still stands.
- The two API-client seam providers throw until the session layer supplies them.
- The password hand-offs use short-lived providers (`PendingCredential`, `pendingVerificationProvider`, `pendingResetProvider`), never route state.
- The update notice has no store link yet, because `/app/config` has no store URL field (plan 7e adds it).

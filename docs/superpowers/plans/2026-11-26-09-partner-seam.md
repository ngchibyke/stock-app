# Part 9: the partner-services seam, updated plan

This updates `2026-11-20-09-partner-integration.md`, which stays the source for the goal, the global constraints, the draft contract to offer the partner, the review focus and the go-live gates. It was written before 7a to 7e, so its mechanics are corrected here to match today's code. Approved by the owner on 2026-11-26: build the full seam, with a fake partner for tests.
Branch: `feat/partner-seam`.

## What changed since 2026-11-20

- **Migrations** are numbered from `0051` (the next free number once part 8's `0050` lands): `0051_partner.sql` holds the events inbox, the usage table and the settings columns together.
- **Jobs** are an array in `escrow/jobs.rs` (24 with part 8, 25 with this). The new job is `partner_events`; the `job_runs` name test is updated.
- **Settings:** `platform_settings` already has a long `SettingsPatch` and a dashboard form. The partner switches get their own Owner routes, `GET /admin/partner/status` and `PATCH /admin/partner`, as part 8 did for retention. The big struct is not touched.
- **Alerts** come from audit rows named in `admin/alerts.rs`'s `AUDIT_ALERTS`; the budget alert is an audit row, `partner.budget_exhausted`, added to that list.
- **`GET /app/config`** is built from startup config only. It gains `features`, computed per request from the database and the breaker, so its handler now reads state.
- **Webhooks** follow the identity webhook's shape: signature checked before the body is read, a body cap with the envelope for 413, 200 for anything settled.
- **The dashboard** is a card on its Settings page, using the existing kit.

## Tasks

1. **Client, breaker, config.** `partners/{mod,config,client,breaker,fake}.rs`. `PartnerApi` trait; `HttpPartner` (https only, no redirects, 5 s connect and 10 s total timeout, 1 MiB response cap, one retry for idempotent GETs only, never for POSTs); `NoPartner`; `FakePartner`. A breaker that opens after 5 consecutive failures and half-opens after 30 s, with an injectable clock. Config `PARTNER_API_BASE_URL`, `PARTNER_API_KEY`, `PARTNER_WEBHOOK_SECRET`, all three or none; secrets redacted in `Debug`. An outbound payload allow-list: a payload with a field outside it is refused before any network call.
2. **Metering, budget, flags.** The `PartnerGateway` every capability will call through: configured, enabled, budget, breaker, call, then record usage after the call (never inside a transaction). A monthly budget stops calls and writes one `partner.budget_exhausted` audit row per month. Owner routes for enabled, feature flags, the budget and a note, audited. `features` in `/app/config`.
3. **Signed webhooks.** `POST /webhooks/partner`: HMAC-SHA256 over `{timestamp}.{raw body}`, constant-time compare, 5-minute window, 64 KiB cap, each `event_id` stored once. A job marks events processed (a no-op until a capability handles the kind) and prunes old ones.
4. **Dashboard card** on Settings: connection, the enabled switch, feature switches, this month's calls against the budget, the last error code, with loading, empty and error states.
5. **Docs and gates:** the README, the roadmap, the partner's draft contract, the go-live gates from the old plan.

## Tests (from the old plan's review focus)

`marketplace_works_with_no_partner`, `slow_partner_times_out_without_blocking`, `breaker_opens_and_recovers`, `get_retries_once_and_post_never`, `response_is_size_capped`, `https_only_and_no_redirects`, `outbound_payloads_carry_only_allowed_fields`, `key_and_secret_never_appear_in_logs_errors_or_debug`, the config tests; `unsigned_or_badly_signed_webhooks_are_refused`, `old_timestamps_are_refused`, `duplicate_events_are_stored_once`, `oversize_body_is_refused`; `budget_stops_calls_and_raises_an_alert`, `usage_is_metered_per_feature_and_day`; the flag tests and the dashboard component tests.

## Checks

Server from `server/`: `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, the whole suite with `DATABASE_URL` set. Dashboard from `dashboard/`: lint, tests, build, and the Playwright suite. Nothing is called for real: there is no partner yet.

## Status

Built on `feat/partner-seam`: `server/src/partners/` (config, breaker, client, fake, gateway, usage, settings, routes, webhook, jobs), migration `0051_partner.sql`, `server/tests/partner.rs` (16 tests) and `partner_client.rs` (8 tests), the dashboard's `PartnerCard` with 9 component tests and one browser test, and the README and roadmap entries.

As built, also different from the plan as written: `connected` in the status is `null` when not checked (a settings change makes no call, so an idle seam makes none); the status route's last error is the partner's last real failure, not our own refusal to call it.

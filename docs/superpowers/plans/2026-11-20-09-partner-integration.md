# Part 9: Partner Services Integration (an allowance, not a feature)

> **For agentic workers:** REQUIRED SUB-SKILL: use the subagent-driven or inline plan-execution workflow, task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Leave a clean, safe, paid-for seam in the server, the dashboard and the apps, so a service built by the owner's other company (the first use is price intelligence for big stores) can be plugged in later without changing the marketplace core. We build the seam now and keep it small. The partner builds the service. We connect to it and pay for it.

**Status of the pricing idea:** on hold. The design mockups ("Shelf price screens") stay as a reference for what the partner service might show. Nothing in them is built here: no price history, bulk price updates, scheduled changes, margin checks or price alerts.

**Why a seam and not a feature:** we do not know the partner's API yet. What we can fix now is how any partner service is called, secured, switched off and paid for. The capability-specific code (for example "price history for an item") waits for the partner's contract, and is a later plan.

## What the partner service is assumed to be

*(my reading, to be confirmed by the partner)*
- A hosted HTTPS API with a key we hold, sold to us per call or per month.
- It may be called by us (outbound) and may call us (inbound webhooks, for example "a price changed").
- It receives product and price data, never customer data.

## Global Constraints

- Codebase conventions apply: `AppError` envelope, `AppJson`, `AppQuery`, `parse_id`, audit rows for every state change (no secret, no signature, no webhook body text in `detail`), nothing personal logged, **no network call inside a database transaction**.
- **The marketplace never depends on the partner.** With no partner configured, or the partner down, slow, or over budget, every existing feature works and nothing returns an error that comes from the partner. A partner-powered screen shows its own empty or unavailable state.
- **Data minimisation.** What may be sent to the partner: our opaque store id, item ids, barcodes, prices in integer kobo, timestamps. What may never be sent: a name, phone number, address, chat text, statement, token or order contents. A test lists the fields of every outbound payload and fails when a new one appears.
- **Secrets.** `PARTNER_API_KEY` and `PARTNER_WEBHOOK_SECRET` live only in configuration. They are never logged, never in an error, never in an audit row, and `Debug` redacts them.
- **Money** is integer kobo. **Time** is RFC 3339 in UTC.
- **We are the paying customer.** Every outbound call is metered per feature and per day, there is a monthly call budget that stops calls when it is spent (and raises an alert), and the budget and usage are visible to the owner.
- Production does **not** require a partner (it is optional), unlike payments, push and moderation.
- Tests run from `server/`: confirm the database is up with `docker ps -a`, start `market-db` if stopped, then `DATABASE_URL=postgres://market:market@localhost:5433/market cargo test`. Never two `cargo test` processes at once.
- Commits: conventional messages, no AI attribution of any kind.

## Draft contract to offer the partner

A starting point for the partner's own design, not a requirement. Everything the partner can answer differently, they should.

**Outbound (we call them), versioned in the path, bearer key in the `Authorization` header:**
- `POST /v1/prices` : we push a batch of price updates `[{store_ref, barcode, price_kobo, effective_at}]`; idempotent by `Idempotency-Key`.
- `GET /v1/items/{barcode}/history?store_ref=&days=` : price points over time.
- `GET /v1/items/{barcode}/comparison` : the same item across stores, each with its price and when it was last checked.
- `GET /v1/health` : answers 200 with the API version; used for the dashboard's connection status.

**Inbound (they call us), `POST /webhooks/partner`:** signed with HMAC-SHA256 over `{timestamp}.{raw body}`, headers `X-Partner-Timestamp` and `X-Partner-Signature`; events `price.changed`, `price.scheduled` (with the start time, so people can be told ahead), `price.stale`; each with a unique `event_id`.

**Both ways:** errors in `{"error": {"code", "message"}}`; a sandbox environment; stated rate limits and an uptime commitment; a statement of where data is stored; a way to rotate keys without downtime.

## Architecture

A new module `partners`. A low-level `PartnerApi` client does the HTTP: https only (http allowed in tests), no redirects, connect timeout 5 s, total timeout 10 s, response capped at 1 MiB, one retry only for idempotent GETs, a **circuit breaker** (open after 5 consecutive failures, half-open after 30 s), and the same "no provider text in errors or logs" rule as the other adapters. Capability traits (later) sit on top of it. `NoPartner` answers "not configured" without a network call; `FakePartner` is the test double. Inbound events land in a `partner_events` inbox table and are processed by a job, so a webhook always answers fast and a bad event cannot break the request.

## Review Focus

1. **The partner is down or slow and the marketplace suffers.** Any request path that waits on the partner, a breaker that never closes, a job that retries forever. Tests: `marketplace_works_with_no_partner`, `slow_partner_times_out_without_blocking`, `breaker_opens_and_recovers` (Task 1).
2. **A spoofed or replayed webhook.** A wrong signature, an old timestamp, the same event twice, a huge body. Tests: `unsigned_or_badly_signed_webhooks_are_refused`, `old_timestamps_are_refused`, `duplicate_events_are_stored_once`, `oversize_body_is_refused` (Task 2).
3. **Personal data reaching the partner.** A payload that carries a name, phone, address or chat text. Test: `outbound_payloads_carry_only_allowed_fields` (Task 1).
4. **A runaway bill.** A loop that calls the partner without limit. Tests: `budget_stops_calls_and_raises_an_alert`, `usage_is_metered_per_feature_and_day` (Task 4).
5. **A secret in a log.** Tests: `key_and_secret_never_appear_in_logs_errors_or_debug` (Task 1 and 2).

---

### Task 0: Branch and baseline
- [ ] Create the branch from up-to-date main and move this plan onto it with the first commit. Run the full suite; all must pass before any change.

### Task 1: The client, the breaker and the config
**Files:** create `server/src/partners/{mod.rs,client.rs,fake.rs,breaker.rs}`, `server/tests/partner_client.rs`; modify `server/src/config.rs`, `server/src/lib.rs` (`AppState.partner`), `server/src/main.rs`, `server/tests/common/mod.rs`, `server/.env.example`.
- Config: `PARTNER_API_BASE_URL` (https), `PARTNER_API_KEY`, `PARTNER_WEBHOOK_SECRET`; all three together or none. Missing means `NoPartner`.
- [ ] **Step 1: Write the failing tests:** `marketplace_works_with_no_partner`; `https_only_and_no_redirects`; `slow_partner_times_out_without_blocking`; `response_is_size_capped`; `breaker_opens_and_recovers`; `get_retries_once_and_post_never`; `outbound_payloads_carry_only_allowed_fields`; `key_and_secret_never_appear_in_logs_errors_or_debug`; config tests (`partial_config_is_refused`, `base_url_must_be_https`).
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the whole suite.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add the partner API client behind a breaker`

### Task 2: Signed inbound webhooks
**Files:** create `server/migrations/0043_partner_events.sql`, `server/src/partners/webhook.rs`, `server/src/partners/jobs.rs`, `server/tests/partner_webhook.rs`; modify `server/src/escrow/jobs.rs` (job 15).
- `partner_events (id uuid PK, event_id text UNIQUE, kind text, payload jsonb, received_at, processed_at NULL)`. Payload holds no personal data; a body over 64 KiB is refused.
- `POST /webhooks/partner`: verify `X-Partner-Signature` (HMAC-SHA256 of `{timestamp}.{raw body}`, constant-time compare) and refuse a timestamp more than 5 minutes old or in the future; store the event once by `event_id`; answer 200 fast. The job marks events processed (a no-op until a capability handles the kind) and prunes old ones.
- [ ] **Steps 1 to 5** as in Task 1, with the tests listed under Review Focus 2. Commit `feat: receive signed partner webhooks`.

### Task 3: Feature flags and the app config
**Files:** create `server/migrations/0044_partner_settings.sql`, `server/tests/partner_settings.rs`; modify `server/src/settings.rs`, `server/src/admin/` (the settings route), the app-config route from plan 7a.
- `platform_settings` gains `partner_enabled bool DEFAULT false` and `partner_features jsonb DEFAULT '{}'` (a map of feature name to bool, for example `price_history`). Only the Owner changes them, with a note, audited.
- `GET /app/config` gains `features: {name: bool}`: true only when a partner is configured, enabled, the feature is on and the breaker is not open. The apps show a partner-powered screen only when its flag is true.
- [ ] **Steps 1 to 5.** Tests: `flags_need_a_configured_enabled_partner`, `only_the_owner_changes_flags_and_it_is_audited`, `app_config_hides_features_while_the_breaker_is_open`. Commit `feat: switch partner features on and off`.

### Task 4: Metering and the budget
**Files:** create `server/migrations/0045_partner_usage.sql`, `server/src/partners/usage.rs`, `server/tests/partner_usage.rs`.
- `partner_usage (day, feature, calls, errors, PRIMARY KEY (day, feature))`, incremented after each call. `platform_settings.partner_monthly_call_budget` (null means no cap): when the month's calls reach it, calls stop with a clear internal error, one alert is raised (using the alerts mechanism the dashboard already shows), and the apps' flags read false.
- [ ] **Steps 1 to 5.** Tests under Review Focus 4. Commit `feat: meter partner calls and cap the monthly budget`.

### Task 5: The dashboard panel
**Files:** modify the dashboard's Settings page; create a small `Partner` card using the existing kit.
- A card showing: connected or not (from `GET /v1/health` through a staff route `GET /admin/partner/status`), the enabled switch, the feature switches, this month's calls against the budget, and the last error code. Skeleton, empty and error states like every other card.
- [ ] **Steps 1 to 5** with component tests and one browser test. Commit `feat: show the partner connection in the dashboard`.

### Task 6: Docs and go-live gates
- [ ] Document the seam in the README and roadmap; give the partner the draft contract above.
- [ ] Add these go-live gates (for review, not conclusions):
  - The partner's contract in writing: endpoints, sandbox, rate limits, uptime, data location, key rotation.
  - A data-processing agreement if any personal data could ever reach the partner (the design sends none).
  - The commercial terms written down, and the accountant or lawyer asked whether related-party rules apply, because the owner belongs to the partner company.
  - A sandbox test of every call and every webhook before the production key is used, and a decision on the monthly budget.
  - Keys held in the secret store, rotated on a schedule.
- [ ] Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, the full `cargo test`; commit `docs: document the partner integration`.

## Decided and still open

- **Decided:** only we pay the partner. Stores do not pay us for it, so no subscription or entitlement system is part of this plan.
- **Open: the first capability** the partner will offer, so the plan after this one can be written against a real contract.
- **Open: timing.** This plan waits behind the core marketplace (7a to 7e) unless the partner's service is ready sooner.

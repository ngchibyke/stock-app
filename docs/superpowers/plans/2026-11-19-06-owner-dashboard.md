# Part 6: Owner Dashboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The owner and support staff run the market from a web dashboard (spec §11). Parts 1–5 already serve most owner routes (codes, settings, escrow overrides, disputes, moderation). This plan adds the server routes that are still missing, closes the "Plan 6" items carried forward in the roadmap, and then builds the web app on top.

**Architecture:** Two stages, in this order, so the server is finished and tested before any screen depends on it.

1. **Server (Tasks 1–9):** new `/admin/...` routes in `server/src/admin/`, following the existing pattern (`Owner`/`Staff` extractors, `AppJson`, `parse_id`, audit row per state change). Read routes return the shapes the screens need, so the web app holds no business logic.
2. **Web app (Tasks 10–14):** a static single-page app in `dashboard/` that talks to the same API with the existing bearer session. It is a separate build, served by the reverse proxy, not by the Rust server.

**Tech Stack:** Server as before (Rust, axum 0.8, sqlx 0.8, Postgres 17). Web: TypeScript, React, Vite, TanStack Query, Vitest and Playwright. *(my call: the spec leaves the stack to the builder. One framework, no server rendering, no UI kit beyond plain CSS, so the owner can host it as static files.)*

**Spec:** `docs/superpowers/specs/2026-10-22-abuja-marketplace-design.md` §11 (Owner dashboard), plus §5, §7 and §8 for the money, moderation and dispute screens. Also read every "Plan 6" bullet in `docs/superpowers/plans/2026-10-23-00-roadmap.md`.

## What exists and what is missing

| Spec §11 area | Server today | This plan |
|---|---|---|
| Access codes | create, list, revoke | none (screen only) |
| Users & stores | `ban`/`unban`/`unmute` in moderation; payout-account view and verify | search, view, suspend and unsuspend a store, view a user, **staff accounts** (add and remove Support) |
| Orders & payments | backstop list, release, refund, hold, unhold, payouts, checkouts, reconciliation | **order search and detail with ledger**, owner re-book, acknowledgement of alerts |
| Disputes | queue, screen, request-return, decide | none (screen only) |
| Moderation | flags, resolve, review queue, unmute, ban, hide reviews | none (screen only) |
| Settings | read and patch | none (screen only) |
| Audit | rows are written | **read route** so "who, what, when" can be seen |

## Global Constraints

- Every rule in the plan 5 "Global Constraints" still applies: `AppError` envelope, `AppJson`/`AppQuery`, `Path<String>` + `parse_id`, an audit row per state change with no personal text in `detail`, encrypted personal fields, no network call inside a transaction, and the lock order users → stores → products → variants → access_codes → checkouts → orders → disputes → shipments → payouts, then conversations → reviews.
- Roles: **Owner** does everything. **Support** reaches disputes and moderation only, and sees personal data only inside an open dispute (spec §11). Every new route states its role in a doc comment and has a test that Support gets 403 where it must.
- New list routes are paginated (`limit` 1..=100, default 25, keyset on `(created_at, id)`), never offset. No list returns encrypted fields; names and phones are decrypted only on the single-record view, and each such read writes an `admin_read` audit row (existing pattern for staff reads).
- *(my call)* Staff accounts: only the Owner can add or remove Support. The Owner cannot remove themself or the last Owner (409 `last_owner`). A removed staff member's sessions are revoked in the same transaction.
- *(my call)* Suspending a store hides its products from browse and blocks new checkouts for it, and leaves existing orders running. It does not freeze payouts: payouts to a suspended store stay an owner decision (roadmap, plan 2 go-live list), so the route records the suspension and the dashboard shows a banner on the payout screen.
- *(my call)* Alert acknowledgement is a new table, not a flag on `audit_log`, because `audit_log` is append-only. An alert is open until a row exists for its source id, and acknowledging needs a note.
- The dashboard stores its session token in memory plus `sessionStorage` only, sends it as `Authorization: Bearer`, and sets a strict `Content-Security-Policy` in its host config. It never renders statement or chat text as HTML.
- Do not add Owner re-book until the Shipbubble sandbox answers in the README's go-live list are known: re-book reuses the existing booking call and does not guess at them.

## File Structure

```
server/src/admin/
  users.rs          # NEW  user search/view, staff accounts (Tasks 1, 2)
  stores.rs         # NEW  store search/view, suspend/unsuspend (Task 3)
  orders.rs         # EDIT order search + detail with ledger (Task 4)
  alerts.rs         # NEW  list + acknowledge reconciliation alerts (Task 5)
  audit_view.rs     # NEW  audit log read (Task 6)
  shipments.rs      # NEW  owner re-book and re-quote (Task 7)
  payouts.rs        # NEW  payout re-poll for bank reversals (Task 8)
  statement.rs      # NEW  daily reconciliation against Mono (Task 9)
server/migrations/
  0036_staff_and_suspension.sql
  0037_alert_acks.sql
server/tests/
  admin_users.rs  admin_stores.rs  admin_orders.rs  admin_alerts.rs
  admin_audit.rs  admin_rebook.rs  admin_repoll.rs  admin_statement.rs
dashboard/          # NEW web app (Tasks 10–14)
```

---

### Task 0: Branch and baseline

- [ ] **Step 1:** `git fetch origin` then `git switch -c feat/owner-dashboard origin/main`. Commit this plan first.
- [ ] **Step 2:** Start Postgres (`docker start market-db`, then `pg_isready -h localhost -p 5433`). Run the full suite from `server/`. Expected: all pass (968 at the end of plan 5) before any change. If not, stop and report.

### Task 1: User search and view

**Files:** `server/src/admin/users.rs`, `server/tests/admin_users.rs`.

- `GET /admin/users?q=&role=&state=&cursor=&limit=` (Staff). `q` matches the blind index of a normalized phone (exact) or a case-insensitive display-name prefix. Support sees only users who appear in an open dispute (join through `disputes`), which matches spec §11.
- `GET /admin/users/{id}` (Owner). Role, created date, strikes, mute state, ban state, store ids, order counts. Decrypted name and phone, with an `admin_read` audit row.
- [ ] **Step 1:** Failing tests: Owner finds a user by phone and by name prefix; Support is limited to dispute parties; the list never contains a phone; the single view writes an audit row; bad cursor → 422.
- [ ] **Steps 2–5:** run → FAIL, implement, run → PASS, commit `feat: search and view users from the dashboard`.

### Task 2: Staff accounts

**Files:** `server/src/admin/users.rs`, migration `0036_staff_and_suspension.sql`, `server/tests/admin_users.rs`.

- `POST /admin/staff` (Owner) creates a Support account from a phone and name and sets a one-time password the person must change at first sign-in (reuse the CLI's password hashing and the `create-owner` code path, extracted to a function both call).
- `POST /admin/staff/{id}/remove` (Owner): sets the role back to `buyer`, revokes sessions in the same transaction, refuses `last_owner`.
- [ ] **Step 1:** Failing tests: create, duplicate phone 409, Support cannot create (403), remove revokes sessions, last owner 409, self-removal 409.
- [ ] **Steps 2–5:** implement and commit `feat: let the owner add and remove support staff`.

### Task 3: Stores: search, view, suspend

**Files:** `server/src/admin/stores.rs`, migration `0036` (adds `stores.suspended_at`, `suspended_reason`), `server/tests/admin_stores.rs`.

- `GET /admin/stores`, `GET /admin/stores/{id}` (Owner): name, owner id, product count, open order count, payout-account status, suspension.
- `POST /admin/stores/{id}/suspend` (`reason` required, 1..=500 chars) and `/unsuspend` (Owner). Browse excludes suspended stores; checkout refuses a cart line from one with 409 `store_suspended`.
- [ ] **Step 1:** Failing tests, including browse exclusion and checkout refusal, and that an existing paid order still progresses.
- [ ] **Steps 2–5:** implement and commit `feat: suspend and unsuspend stores`.

### Task 4: Order search and detail with ledger

**Files:** `server/src/admin/orders.rs`, `server/tests/admin_orders.rs`.

- `GET /admin/orders?status=&store_id=&from=&to=&cursor=` (Owner). `GET /admin/orders/{id}` (Owner): the order, items, payment, delivery events, refunds, payout, dispute id if any, and the **ledger postings** grouped by posting key with a running sum that must be zero.
- [ ] **Step 1:** Failing tests: filters, keyset paging, a completed order's postings sum to zero, a partial-refund order shows both posting keys.
- [ ] **Steps 2–5:** implement and commit `feat: order search and ledger view`.

### Task 5: Alert acknowledgement

**Files:** `server/src/admin/alerts.rs`, migration `0037_alert_acks.sql`, `server/tests/admin_alerts.rs`.

- Move the 30-day `audit_log` window for late-provider-outcome and payout-amount-mismatch alerts behind `GET /admin/alerts?state=open|acked`. Add `POST /admin/alerts/{id}/ack` with a note. An unacknowledged alert never ages out of the open list.
- Also add acknowledgement for the sticky `orders_attention` and `shipments_attention` rows, and a repeated-handover-lockout alert after the third lockout for one order.
- [ ] **Step 1:** Failing tests: an alert 40 days old is still open until acked; ack needs a note; double ack 409; Support gets 403.
- [ ] **Steps 2–5:** implement and commit `feat: acknowledge reconciliation alerts`.

### Task 6: Audit log view

**Files:** `server/src/admin/audit_view.rs`, `server/tests/admin_audit.rs`.

- `GET /admin/audit?actor_id=&action=&from=&to=&cursor=` (Owner). Read-only; `detail` is returned as stored.
- [ ] **Step 1:** Failing tests: filters, paging, and that no route can write or delete a row.
- [ ] **Steps 2–5:** implement and commit `feat: read the audit log`.

### Task 7: Courier re-book and re-quote

**Files:** `server/src/admin/shipments.rs`, `server/tests/admin_rebook.rs`.

- `POST /admin/shipments/{id}/rebook` (Owner) for `booking_failed`, `booking_unknown` and `cancel_failed`, using the existing booking path with a fresh quote when the old token expired. Idempotent on the shipment, audited, and it refuses while a booking call is in flight.
- This is the gate named in the README: courier delivery for real stores stays off until this exists **and** the sandbox answers are in. Tests use the Shipbubble wiremock from `tests/shipbubble_adapter.rs`.
- [ ] **Step 1:** Failing tests: each failed state re-books; an expired token re-quotes and flags a price difference above the tolerance; a second call while booking is 409.
- [ ] **Steps 2–5:** implement and commit `feat: owner re-book for failed courier shipments`.

### Task 8: Re-poll payouts for bank reversals

**Files:** `server/src/admin/payouts.rs` and the escrow jobs module, `server/tests/admin_repoll.rs`.

- A job (and `POST /admin/payouts/{id}/repoll`) re-reads recent successful payouts from the provider and raises an alert when one shows reversed. Same `PayoutProvider` trait, so the fake provider drives the tests.
- [ ] **Step 1:** Failing tests with an explicit `now`: a payout reversed after success raises one alert; an unchanged payout raises none.
- [ ] **Steps 2–5:** implement and commit `feat: re-poll payouts for bank reversals`.

### Task 9: Daily statement reconciliation

**Files:** `server/src/admin/statement.rs`, `server/tests/admin_statement.rs`.

- Compare the day's provider statement (collections, payouts, fees) against ledger totals, and store the result as an alert when they differ by more than a settings-driven tolerance. Blocked on Mono's statement endpoint: if the sandbox has none, write the trait and fake now and leave the Mono adapter as a documented go-live item.
- [ ] **Steps 1–5:** failing tests with the fake, implement, commit `feat: reconcile the daily provider statement`.

### Task 10: Dashboard scaffold and sign-in

**Files:** `dashboard/` (Vite + React + TypeScript), `dashboard/src/api.ts`, `dashboard/src/auth.tsx`.

- Phone and password sign-in against the existing auth routes, role read from the session, route guards for Owner-only and Staff areas, a typed API client, and one error banner for the `AppError` envelope.
- [ ] **Step 1:** Vitest tests for the client (envelope parsing, 401 → sign-out) and the guards.
- [ ] **Steps 2–4:** implement, `npm test`, `npm run build`. **Step 5:** commit `feat: dashboard scaffold and sign-in`.

### Task 11: Access codes, users, stores, settings screens

- Create (single or multi-use, cap, expiry), list, revoke; user and store search and detail with suspend, ban, unmute; the settings form (platform cut, cut-off time, strike thresholds) with the server's validation errors shown inline.
- [ ] Component tests, then commit `feat: dashboard codes, users, stores and settings`.

### Task 12: Orders, money and reconciliation screens

- Order search and detail with the ledger, backstop queue, payouts (mark paid, requeue), refunds, checkouts (clear flag, resolve), alerts with acknowledgement, attention lists, re-book.
- Every money action asks for a note and shows the exact amount in naira and kobo before confirming.
- [ ] Component tests, then commit `feat: dashboard orders, money and reconciliation`.

### Task 13: Disputes and moderation screens

- The dispute queue and decision screen (full refund, partial refund with an amount field validated against the item total, release, request return with who pays). Flag queue, resolve, review queue, hide and unhide, unmute, ban.
- Statement and chat text is rendered as text only.
- [ ] Component tests, then commit `feat: dashboard disputes and moderation`.

### Task 14: End-to-end, hosting and docs

- Playwright runs the happy path against a server started with fake providers: sign in as Owner, create a code, open a dispute fixture, decide it, see the audit row.
- Add a `dashboard` job to `.github/workflows/ci.yml` (install, lint, test, build). Add a short hosting section to the README (static files behind the reverse proxy, the CSP header, same-origin or CORS for the API).
- Update the roadmap row for part 6 and the README roadmap table.
- [ ] Commit `docs: document the owner dashboard`.

### Final verification

- [ ] Postgres is up. Full `cargo test`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` from `server/` are clean. In `dashboard/`, `npm test`, `npm run lint` and `npm run build` pass.
- [ ] Fix anything found and commit.

## Not in this plan

- Notification preferences, and charging unrecoverable provider fees or return shipping to the side at fault (roadmap, plan 5 carry-forward): both need owner decisions first.
- A full account ban beyond the existing moderation ban, and an editable word list.
- Four-eyes approval, step-up auth and rate limits for owner money overrides: listed under go-live. This plan puts every override behind a note and an audit row but does not add a second approver.

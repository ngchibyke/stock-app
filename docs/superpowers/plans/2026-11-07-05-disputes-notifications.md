# Part 5: Disputes & Notifications Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A buyer can report a problem with an order before its money is released. The report freezes the money, the seller has 48 hours to give their side, and the owner settles it from one screen with a full refund, a partial refund or a release. Both people are told. This plan adds:
- disputes with evidence (text and photos), a seller response window, an optional "return it first" step, and a final owner decision written to the audit log;
- partial refunds, which do not exist yet: part of the item total goes back to the buyer and the rest is released to the seller;
- the server side of the owner's dispute screen: order, payment, ledger, delivery timeline, handover-code status, both sides' evidence, and the order chat with flagged originals (the screens come in plan 6);
- an in-app inbox and FCM push for every event in spec §10, including confirm-delivery reminders;
- a fix carried from plan 2: a buyer can no longer release an order whose payment the provider flagged.

**Architecture:** A new `disputes` module owns the complaint lifecycle and reuses the existing order hold (`orders.held_at`), which already stops auto-release, buyer confirmation and payout. The owner's decision runs in one transaction that locks the order, marks the dispute decided, and calls the escrow engine's in-transaction steps. To make that possible, `release` is split into a pure amount calculation and an in-transaction `release_in`, which the new partial refund reuses. Provider calls still happen after commit.

A new `notify` module is an outbox. Domain code inserts a `notifications` row inside the same transaction as the state change, so a notice can never exist without its event, or the reverse. A separate push loop sends the outbox through a `PushSender` trait (`FcmPush` adapter, `FakePush` test double), following the `ToneChecker` pattern. The inbox stores no text: titles and bodies are fixed copy derived from `kind` when read.

**Tech Stack:** Rust, axum 0.8, sqlx 0.8 on Postgres 17, tokio, reqwest 0.12 (rustls), wiremock. New: `jsonwebtoken = "9.3"`, to sign the Google service-account assertion (RS256) that FCM's OAuth token requires.

**Spec:** `docs/superpowers/specs/2026-10-22-abuja-marketplace-design.md`: §8 "Disputes" and §10 "Notifications", plus §5 (reminders, refunds, the "return first" rule), §7 (the owner reading chat in a dispute), §11 (Owner and Support roles) and §13. Also read the "Carried forward" sections of `docs/superpowers/plans/2026-10-23-00-roadmap.md`.

## Global Constraints

- Codebase conventions apply everywhere, and every task's requirements implicitly include this section:
  - errors use the `AppError` JSON envelope;
  - bodies use `AppJson<T>`, query strings use `AppQuery<T>`;
  - path ids are `Path<String>` parsed with `parse_id`, never `Path<Uuid>`;
  - every state change writes to the append-only `audit_log`, and `detail` never holds statement text, message text or a push token;
  - personal fields are encrypted at rest with `FieldCipher`;
  - personal data, statement text and push tokens are never logged.
- Lock order: users → stores → products → variants → access_codes → checkouts → orders → **disputes** → shipments → payouts, then conversations → reviews. `payout_accounts` stays last. `notifications` are insert-only and never locked by domain code. `push_devices` are only touched by the device routes and the push loop. **Never make a network call inside a database transaction.**
- A dispute may be opened only by the order's buyer, only on an order in `picked_up`, `out_for_delivery` or `delivered` with no payout. Errors:
  - `paid`: 409 `cancel_instead`, because cancelling already gives a full refund;
  - `completed`: 409 `already_released`;
  - any other status: 409 `invalid_order_state`;
  - a second report on the same order, even after a withdrawal: 409 `dispute_exists`.
- *(my call)* **One report per order.** A withdrawal is final, so a buyer cannot toggle a hold to keep delaying a seller's payout. The withdraw copy says so.
- Dispute states: `awaiting_seller` → `awaiting_decision` (seller responded, or 48h passed) ⇄ `awaiting_return` → `decided`. `withdrawn` can be reached from any open state. Open means `awaiting_seller`, `awaiting_return` or `awaiting_decision`.
- `SELLER_RESPONSE_HOURS = 48`, calendar hours, stored per dispute as `seller_respond_by`. The seller responds once. After the deadline the response is closed (409 `response_closed`).
- *(my call)* The owner may not decide while the dispute is `awaiting_seller` and the deadline has not passed (409 `awaiting_seller`), so the seller always gets their 48 hours.
- While a dispute is open:
  - the order stays held;
  - the owner's plain release, refund and unhold routes answer 409 `dispute_open`, so the decision always goes through the dispute;
  - the buyer's "Received" answers 409 `order_held`, as it already does for a held order.
- Decisions, owner only:
  - `full_refund` uses the existing owner refund: items plus delivery, stock not restored, and the platform absorbs the collection fee (existing ruling);
  - `release` uses the existing owner release;
  - `partial_refund` with `refund_kobo` in `1..items_total_kobo`. The delivery fee is refunded only by `full_refund`, because on a courier order the platform has already paid the courier.
- Partial refund amounts, exact:
  - `retained = items_total_kobo − refund_kobo`;
  - `cut = money::cut_kobo(retained, platform_cut_bps)`, using the order's snapshotted bps;
  - the release then runs the existing formula with `items = retained` and that `cut`. Delivery and the collection fee shares are unchanged, because the seller still pays the collection fee (spec §5);
  - ledger: `("refund", refund_id)` posts escrow `−refund_kobo` and refunds `+refund_kobo`, then `("release", order_id)` posts on `retained + delivery`. No `refund_fee` posting;
  - the order ends `completed`, its hold is cleared, and its `refunds` row has `kind = 'partial'`.
- Dispute statements: `MAX_DISPUTE_CHARS = 2000` (characters, after trimming, at least 1) and `MAX_DISPUTE_PHOTOS = 5`, in `limits.rs`.
- *(my call)* Statements are checked with `moderation::filter::scan` only. A `Serious` hit of any category is refused with 422 `statement_refused` and the copy below. There is no strike, no rewrite and **no Claude call**: complaints are expected to be heated, and evidence should not go to a third party.
- Statement copy, exact. Contact details: "Keep contact details and payments inside the app. Remove them and try again." Abuse: "This breaks the market's rules. Describe the problem without insults."
- Withdraw copy, exact (returned as `note` with the confirmation): "You can't report this order again after withdrawing."
- *(my call)* A request to return the item first records who pays for return shipping (`buyer` or `seller`, the side at fault, spec §5). No money moves for return shipping: the parties arrange it. Charging unrecoverable fees to the side at fault is carried forward to plan 6.
- Privacy on the dispute:
  - the seller sees "Buyer", never a name;
  - neither side nor Support ever sees the buyer's phone or address on a dispute;
  - the staff screen shows the delivery **area council** only.
- Staff access to a dispute:
  - Owner: any dispute, including the chat's flagged originals;
  - Support: open disputes only (403 `dispute_closed` otherwise, spec §11), and never flagged originals.
  - Every read writes `admin.dispute_viewed` with `{dispute_id, originals: bool}` before the response is sent.
- Notifications store no text. `title` and `body` come from `Kind` at read time. The push payload is the fixed title and body plus `data: {kind, notification_id}`, and nothing else: no order id, no names, no amounts, no message text. The app fetches details from the inbox.
- Notification copy, exact (`kind`: recipient, title, body):
  - `order_paid`: seller, "Paid — please ship", "A buyer has paid. Prepare the order for delivery within 48 hours."
  - `order_picked_up`: buyer, "Order picked up", "Your order is on its way."
  - `order_out_for_delivery`: buyer, "Out for delivery", "Your order is out for delivery. Open the app to track it."
  - `order_delivered`: buyer, "Delivered", "Your order was marked delivered. Confirm in the app once you hold it, or report a problem."
  - `confirm_reminder`: buyer, "Did your order arrive?", "Confirm in the app once you hold it, or report a problem."
  - `release_tomorrow`: buyer, "Payment releases tomorrow", "The seller will be paid tomorrow. If something is wrong with your order, report it before then."
  - `refund_started`: buyer, "Refund started", "Your refund has started. Banks can take up to 10 working days."
  - `funds_released`: seller, "Payment released", "An order's payment was released. Your payout is on its way."
  - `chat_message`: the other party, "New message", "You have a new message. Open the app to read it."
  - `dispute_opened`: seller, "A buyer reported a problem", "Respond within 48 hours. The order's payment is on hold until it is settled."
  - `return_requested`: both, "Return requested", "The market asked for the item to be returned before deciding. Open the app for details."
  - `dispute_decided`: both, "Report decided", "The market has decided on the reported order. Open the app for details."
  - `moderation_warning`: sender, "Message not sent", "A recent message broke the market's rules. Repeated breaks lead to a mute."
  - `moderation_muted`: sender, "Posting paused", "You can't post for a while because of repeated rule breaks."
- Reminders (spec §5), for `delivered` orders that are not held and have no payout:
  - the delivered notice is the first;
  - `confirm_reminder` at `delivered_at + 24h`, only if that is before `release_at − 24h`;
  - `release_tomorrow` at `release_at − 24h`, only if that is after `delivered_at`;
  - nothing at or after `release_at`. A missed stage is skipped, never sent late.
- Push rules:
  - a push goes only to devices whose session is not revoked and not expired, whose device is not disabled, and whose user is not deleted;
  - a token registered again under another session or account moves to it, so a shared phone never gets the previous user's notices;
  - a notice older than 24h when claimed is `skipped`, not sent;
  - at most 8 push calls in flight;
  - backoff is 30s, doubling, capped at 1h. After 6 attempts the notice is `failed`;
  - FCM 404 `UNREGISTERED` disables that device.
- FCM HTTP v1, exact:
  - Token: a JWT assertion, RS256, with claims `iss = client_email`, `scope = "https://www.googleapis.com/auth/firebase.messaging"`, `aud = token_uri`, `iat = now`, `exp = now + 3600`. It is POSTed form-encoded to `token_uri` with `grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer&assertion=<jwt>`. The access token is cached until 5 minutes before `expires_in` runs out, and dropped on a 401, which is retried once.
  - Send: `POST {base}/v1/projects/{project_id}/messages:send`, default base `https://fcm.googleapis.com`, with header `Authorization: Bearer <access token>` and body `{"message":{"token":…,"notification":{"title":…,"body":…},"data":{"kind":…,"notification_id":…}}}`. No `android` or `apns` blocks.
  - HTTP client rules are the same as `couriers/shipbubble.rs`: rustls, `https_only`, no redirects, connect timeout 5s, total timeout 10s, response body capped at 64 KiB, no provider text or token in errors or logs, no retries inside the adapter apart from the single 401 refresh.
- Production refuses to start without `FCM_SERVICE_ACCOUNT_FILE`. Outside production, with none set, pushes are accepted and dropped (`NoPush`), and a warning is logged at startup.
- Tests run from `server/`: first confirm the database is up with `docker ps -a`, and start the existing container with `docker start market-db` if stopped, then `DATABASE_URL=postgres://market:market@localhost:5433/market cargo test`. Run the whole suite at the end of each task.
- Commits: conventional messages. Never add a `Co-Authored-By` line, a "Generated with" footer, a session link or any other AI attribution.

## Review Focus

1. **A buyer reports a problem in the same instant the money is released:** the auto-release job, the buyer's handover code, or "Received". Exactly one wins. Either the dispute is refused with `already_released`, or the order is held and nothing is paid out. Tests: `dispute_races_auto_release`, `dispute_races_buyer_confirmation` (Task 2).
2. **A partial refund at the edges:** 1 kobo, `items − 1`, and `items` or more (refused). Self-delivery and courier orders, and fees larger than what is left. The per-order ledger nets to zero, money out equals money in, and a payout that would go negative goes `manual` as it does today. Tests: `partial_refund_amounts_match_formula`, `partial_refund_bounds`, `partial_refund_small_remainder_goes_manual` (Task 4).
3. **The owner double-clicks the decision, or two staff decide at once:** one decision, one refund row, one payout. The second request gets 409 naming the state. A provider outage after commit leaves the decision standing, and the existing sweepers retry the money. Tests: `concurrent_decisions_act_once`, `decision_stands_when_refund_provider_is_down` (Task 5).
4. **Push is down, a token is dead, or the user signed out:** the event and its inbox row commit whatever push does; nothing goes to a revoked session's device; a dead token is disabled; a retry never re-sends to a device that already got it. Tests: `push_outage_keeps_inbox_and_retries`, `revoked_session_gets_no_push`, `retry_skips_devices_already_sent` (Task 9).
5. **Personal data leaks through the new surfaces:** Support reads a closed dispute or a flagged original, or a name, address, amount or message text reaches a lock screen through a push payload. Tests: `support_cannot_read_closed_dispute`, `support_never_sees_originals` (Task 6), `push_payload_is_fixed_copy_and_ids` (Task 9).

---

### Task 0: Branch and baseline

- [ ] **Step 1:** Create the branch from up-to-date main: `git fetch origin` then `git switch -c feat/disputes-notifications origin/main`. Move this plan onto it with the first commit.
- [ ] **Step 2:** Start the database (`docker start market-db`) and run the full suite. Expected: all pass (867 at the end of plan 4) before any change. If not, stop and report.

### Task 1: A flagged payment cannot be released by the buyer

Carried forward from plan 2: a provider flag blocks auto-release and payout, but not a buyer-confirmed release. After such a release the owner could no longer refund. This task closes the gap by preventing that release, rather than reversing it afterwards.

**Files:**
- Modify: `server/src/escrow/release.rs`, `server/src/orders/routes.rs`, `server/tests/common/mod.rs`
- Test: `server/tests/escrow.rs`

**Interfaces:**
- Produces, in `tests/common/mod.rs`:
  - `struct ShippedOrder { order_id, checkout_id, store_id, buyer_id: Uuid, buyer_token: String, seller_id: Uuid, seller_token: String }`
  - `TestApp::shipped_order(&self, method: &str /* "self" | "courier" */, status: &str /* "picked_up" | "out_for_delivery" | "delivered" */) -> ShippedOrder`. It builds on the same steps as `tests/escrow.rs::picked_up_order_with_method`, and every later task uses it.
- `release` with `ReleaseReason::BuyerConfirmed` on a checkout with `provider_flagged = true` returns `AppError::Conflict("payment_under_review")`, after the order lock, and changes nothing.

- [ ] **Step 1: Write the failing tests** in `tests/escrow.rs`:
  - `buyer_cannot_release_flagged_payment`: "Received" answers 409 `payment_under_review`. The order is unchanged and there is no payout.
  - `handover_on_flagged_payment_waits_for_the_window`: a correct handover code delivers the order (`release_at` is set), but there is no payout. After `clear_flag` and `release_at` passes, the job releases it.
  - `owner_can_refund_flagged_order_after_handover`: hold, then the owner refunds it. The refund row is created.
- [ ] **Step 2: Run them.** `cargo test --test escrow flagged`. Expected: FAIL.
- [ ] **Step 3: Implement the guard and the fixture.** Add `c.provider_flagged` to `ReleaseRow`'s select. In `received`, map `payment_under_review` straight through. The handover route already logs a failed release and leaves the order to auto-release.
- [ ] **Step 4: Run the tests.** Expected: PASS, with the rest of `escrow.rs` unchanged.
- [ ] **Step 5: Commit.** `fix: stop a buyer releasing a flagged payment`

### Task 2: Opening, viewing and withdrawing a dispute

**Files:**
- Create: `server/migrations/0034_disputes.sql`, `server/src/disputes/mod.rs`, `server/src/disputes/service.rs`, `server/src/disputes/routes.rs`, `server/tests/disputes.rs`
- Modify: `server/src/lib.rs`, `server/src/limits.rs`, `server/src/orders/views.rs` (`OrderCore.dispute`)

**Schema (`0034`):**
- `disputes`:
  - `id uuid PK`, `order_id uuid NOT NULL UNIQUE`, `buyer_id`, `store_id`;
  - `reason text` CHECK in (`not_received`, `damaged`, `wrong_item`, `not_as_described`, `incomplete`, `other`);
  - `status text` CHECK in (`awaiting_seller`, `awaiting_return`, `awaiting_decision`, `decided`, `withdrawn`);
  - `opened_at`, `seller_respond_by`, `seller_responded_at NULL`, `seller_missed_deadline bool DEFAULT false`;
  - `return_paid_by text NULL` CHECK in (`buyer`, `seller`), `return_requested_at NULL`, `return_received_at NULL`;
  - `decision text NULL` CHECK in (`full_refund`, `partial_refund`, `release`), `refund_kobo bigint NULL CHECK (> 0)`, `decided_by NULL`, `decided_at NULL`, `withdrawn_at NULL`;
  - CHECK `(status = 'decided') = (decision IS NOT NULL)`, and CHECK `(decision = 'partial_refund') = (refund_kobo IS NOT NULL)`.
  - Indexes: `(status, opened_at)`, and a partial index on `(seller_respond_by) WHERE status = 'awaiting_seller'`.
- `dispute_statements`: `dispute_id`, `party` (`buyer`, `seller`), `body_enc bytea NOT NULL`, `photo_keys text[] NOT NULL DEFAULT '{}'`, `created_at`, `PRIMARY KEY (dispute_id, party)`.
- `refunds` gains `kind text NOT NULL DEFAULT 'full' CHECK (kind IN ('full', 'partial'))`.
- Header comment: personal data is the statements (encrypted) and photo keys. Retention goes to the DPO's list.

**Interfaces:**
- Produces, in `disputes/service.rs`:
  - `const SELLER_RESPONSE_HOURS: i64 = 48`
  - `enum Reason`, `enum DisputeStatus`, each with `as_str()` and `parse`
  - `struct Statement { body: String, photo_keys: Vec<String> }`. `fn check_statement(s: &Statement) -> Result<(), AppError>` covers the limits and the `scan` rule with the copy from Global Constraints.
  - `async fn open(state: &AppState, buyer: Uuid, order_id: Uuid, reason: Reason, s: Statement) -> Result<DisputeView, AppError>`. It locks the order, then inserts the dispute and the buyer's statement. If the order is not held, it sets `held_at = now(), hold_reason = 'dispute'`. It audits `dispute.opened` with `{reason, prior_status}`.
  - `async fn withdraw(state, buyer, dispute_id) -> Result<DisputeView, AppError>`. It locks the order, then the dispute. Allowed from an open state only (409 `dispute_closed`). It clears the hold only if `hold_reason = 'dispute'`, and audits `dispute.withdrawn`.
  - `async fn view(state, viewer: Uuid, dispute_id) -> Result<DisputeView, AppError>`, for the buyer or the store owner only. Anyone else gets 404.
  - `async fn open_for_order(conn: &mut PgConnection, order_id) -> Result<Option<Uuid>, AppError>`
  - `struct DisputeView { id, order_id, status, reason, opened_at, seller_respond_by, seller_missed_deadline, buyer_statement: StatementView, seller_statement: Option<StatementView>, return_request: Option<ReturnView>, decision: Option<DecisionView> }`. `StatementView { body, photo_keys, at }`, `ReturnView { paid_by, requested_at, received_at }`, `DecisionView { decision, refund_kobo, decided_at }`. The owner's note is never in this view.
- Routes: `POST /orders/{id}/dispute` with body `{reason, body, photo_keys}`, `GET /disputes/{id}`, `POST /disputes/{id}/withdraw` with body exactly `{"confirm": true}` (reuse `orders::routes::confirmed`). The withdraw response adds `note` with the copy from Global Constraints.
- `OrderCore.dispute: Option<DisputeSummary { id, status }>`.

- [ ] **Step 1: Write the failing tests** in `tests/disputes.rs`:
  - `buyer_opens_dispute_and_order_is_held`: 201. The order is held with `hold_reason = 'dispute'`. `seller_respond_by` is 48h after `opened_at`. `GET /orders/{id}` shows the summary.
  - `only_shipped_unreleased_orders_can_be_disputed`: `paid` gives `cancel_instead`, `completed` gives `already_released`, `refunded` gives `invalid_order_state`. A stranger or the seller gets 404.
  - `one_report_per_order`: a second open, including after a withdrawal, gives `dispute_exists`.
  - `statement_limits`: 2001 characters, 6 photos, an empty body and an empty photo key each give 422.
  - `statement_with_phone_number_is_refused_without_strike`: 422 `statement_refused` with the exact copy. There is no `moderation_strikes` row and `FakeTone::calls() == 0`.
  - `statement_is_encrypted`: the plaintext appears in no column.
  - `held_dispute_stops_release`: the auto-release job and "Received" leave the order unpaid.
  - `dispute_races_auto_release` and `dispute_races_buyer_confirmation`: the two run concurrently on a due order. Either the dispute fails with `already_released` and there is one payout, or the dispute exists and there is no payout. Never both, never neither.
  - `withdraw_unholds_only_a_dispute_hold`: an owner hold placed before the dispute survives a withdrawal. A dispute hold is cleared, and a due order then releases on the next tick.
  - `seller_sees_buyer_as_buyer`: the seller's view has the statement and no name, phone or address.
- [ ] **Step 2: Run them.** `cargo test --test disputes`. Expected: FAIL.
- [ ] **Step 3: Implement the migration, module, routes and the order summary.**
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: let buyers report a problem and freeze the order's money`

### Task 3: Seller response, return first, and the response deadline

**Files:**
- Create: `server/src/disputes/jobs.rs`
- Modify: `server/src/disputes/service.rs`, `server/src/disputes/routes.rs`, `server/src/escrow/jobs.rs`, `server/src/admin/routes.rs` (or a new `server/src/admin/disputes.rs` holding the dispute admin routes)
- Test: `server/tests/disputes.rs`, `server/tests/jobs.rs`

**Interfaces:**
- Produces, in `disputes/service.rs`:
  - `async fn respond(state, seller: Uuid, dispute_id, s: Statement) -> Result<DisputeView, AppError>`: from `awaiting_seller` before `seller_respond_by` only. A repeat gives 409 `already_responded`; after the deadline, 409 `response_closed`. It moves the dispute to `awaiting_decision` and audits `dispute.seller_responded`.
  - `async fn request_return(state, actor: Uuid, dispute_id, paid_by: Party, note: &str) -> Result<(), AppError>`: Staff, from `awaiting_decision`. Moves to `awaiting_return` and audits `admin.dispute_return_requested`.
  - `async fn return_received(state, seller, dispute_id) -> Result<DisputeView, AppError>`: from `awaiting_return`. Moves back to `awaiting_decision` and audits `dispute.return_received`.
- Produces, in `disputes/jobs.rs`: `const DISPUTE_DEADLINES: &str = "dispute_deadlines"` and `async fn run(state, cancel) -> Result<Stats, AppError>`. It moves `awaiting_seller` disputes past `seller_respond_by` to `awaiting_decision` with `seller_missed_deadline = true`, audits `dispute.seller_missed_deadline`, and claims with `FOR UPDATE SKIP LOCKED LIMIT BATCH` like the other jobs.
- Routes: `POST /disputes/{id}/response`, `POST /disputes/{id}/return-received`, and `POST /admin/disputes/{id}/request-return` (Staff) with body `{paid_by, note}`. The note is validated with `admin::note`.
- `escrow::jobs` runs `dispute_deadlines` as job 12, after `chat_held`, with its own `JobReport` field and module-doc entry.

- [ ] **Step 1: Write the failing tests.**
  - `seller_responds_once_within_48_hours`.
  - `late_response_is_closed`: the deadline is aged in SQL.
  - `deadline_job_moves_dispute_on`: `seller_missed_deadline` is true and the job has a `job_runs` row.
  - `return_round_trip`: request-return as Support works. A buyer, or a seller of another store, gets 404. Return-received by the seller goes back to `awaiting_decision`. Both views show `paid_by`.
  - `withdraw_during_return`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement the service functions, routes and job.**
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add the seller response window and the return-first step`

### Task 4: An in-transaction release, and partial refunds

**Files:**
- Create: `server/src/escrow/partial.rs`, `server/tests/partial_refund.rs`
- Modify: `server/src/escrow/release.rs`, `server/src/escrow/refund.rs`, `server/src/escrow/mod.rs`, `server/src/admin/money.rs` (`mark_refunded`), `server/src/orders/views.rs`

**Interfaces:**
- Produces, in `release.rs`:
  - `struct ReleaseInputs { items: i64, delivery: i64, self_delivery: bool, cut: i64, share: i64, courier_share: i64 }`
  - `struct ReleaseAmounts { gross, cut_and_delivery, provider_fee, payout_fee, payout, shortfall, seller_delivery: i64 }`
  - `fn release_amounts(i: &ReleaseInputs, s: &FeeSchedule) -> Result<ReleaseAmounts, AppError>`: the existing arithmetic moved out of `release`, unchanged.
  - `pub(crate) enum ReleasedIn { Done { payout_id: Uuid, status: &'static str, send_after: OffsetDateTime }, Unchanged(String) }`
  - `pub(crate) async fn release_in(conn: &mut PgConnection, settings: &Settings, order_id: Uuid, reason: ReleaseReason, items_override: Option<(i64, i64)> /* (items, cut) */) -> Result<ReleasedIn, AppError>`. This is `release` up to, but not including, the commit and the inline send. `release` becomes begin, `release_in`, owner audit and hold clear, commit, send.
- Produces, in `refund.rs`: `pub(crate) async fn owner_begin_in(conn: &mut PgConnection, order_id: Uuid) -> Result<Uuid, AppError>`, which is `begin(conn, order_id, false, "dispute_refund", true)` and returns the refund id.
- Produces, in `partial.rs`:
  - `pub(crate) struct PartialIn { refund_id: Uuid, released: ReleasedIn }`
  - `pub(crate) async fn partial_refund_in(conn: &mut PgConnection, settings: &Settings, order_id: Uuid, refund_kobo: i64) -> Result<PartialIn, AppError>`. It locks the order and requires it to be held, shipped, with no payout and no refund. It checks `1 <= refund_kobo < items_total_kobo` (422 `refund_out_of_range`). Then it inserts the `refunds` row (`kind 'partial'`, reason `dispute_partial`), posts `("refund", id)`, and calls `release_in` with `items_override = Some((retained, cut))` and reason `OwnerDecision`. It clears the hold and audits `order.partial_refund` with `{refund_id, refund_kobo, retained_kobo, cut_kobo}`.
- `send_refund` and `mark_refunded` move the order to `refunded` only when `kind = 'full'`. A partial refund's order stays `completed`, and `mark_refunded` no longer answers `order_state_changed` for it.
- `SellerOrderView.expected_payout_kobo` is the payout's `amount_kobo` once a payout exists.

- [ ] **Step 1: Write the failing tests.**
  - Unit tests in `release.rs`: `release_amounts_matches_existing_cases`, reusing the numbers from `tests/escrow.rs::release_amount_matches_formula` and `non_positive_payout_goes_manual`.
  - In `tests/partial_refund.rs`, which commits `partial_refund_in` in its own transaction and then calls `send_refund` and `send_payout`:
    - `partial_refund_amounts_match_formula`, for self-delivery (the seller keeps the delivery fee) and for courier;
    - `partial_refund_bounds`: 0, `items` and `items + 1` give 422, and 1 and `items − 1` succeed;
    - `partial_refund_small_remainder_goes_manual`: the payout is `manual`, as today;
    - `partial_refund_ledger_balances`: the order's entries net to zero across accounts and escrow ends at 0;
    - `partial_refund_keeps_order_completed`, after the provider's Ok and after `mark_refunded`;
    - `seller_sees_partial_payout`.
- [ ] **Step 2: Run them.** `cargo test --test partial_refund` and `cargo test --lib release`. Expected: FAIL.
- [ ] **Step 3: Refactor `release` into `release_amounts` and `release_in`**, then run `cargo test --test escrow`. Expected: PASS before any new code.
- [ ] **Step 4: Implement `owner_begin_in`, `partial_refund_in`, and the `kind` checks.**
- [ ] **Step 5: Run the tests.** Expected: PASS.
- [ ] **Step 6: Commit.** `refactor: split release into an in-transaction step` and `feat: add partial refunds`

### Task 5: The owner's decision

**Files:**
- Create: `server/src/disputes/decide.rs`, `server/tests/dispute_decisions.rs`
- Modify: `server/src/escrow/release.rs` and `server/src/escrow/refund.rs` (open-dispute guard on the owner paths), `server/src/admin/orders.rs` (`unhold`), the admin dispute routes

**Interfaces:**
- Consumes: `release_in`, `owner_begin_in`, `partial_refund_in`, `send_refund`, `send_payout`, `open_for_order`.
- Produces, in `decide.rs`:
  - `enum Decision { FullRefund, PartialRefund(i64), Release }`
  - `async fn decide(state: &AppState, actor: Uuid, dispute_id: Uuid, decision: Decision, note: &str) -> Result<DisputeView, AppError>`

`decide`, in order:
1. Read the dispute (404 if unknown) and load `settings` (outside the transaction, as `release` does).
2. Begin. Lock the order (`SELECT … FOR UPDATE`).
3. Mark the dispute decided with `UPDATE disputes SET status = 'decided', decision, refund_kobo, decided_by, decided_at WHERE id = $1 AND (status IN ('awaiting_decision', 'awaiting_return') OR (status = 'awaiting_seller' AND seller_respond_by <= now()))`. Keep the outer parentheses: without them the `OR` matches other disputes. If no row changes, roll back and return 409 `awaiting_seller` or `dispute_closed`, naming the state.
4. Call the engine's in-transaction step:
   - `Release` → `release_in` (`OwnerDecision`), then clear the hold;
   - `FullRefund` → `owner_begin_in`;
   - `PartialRefund` → `partial_refund_in`.
   An `Unchanged` result rolls back with 409 `invalid_order_state`.
5. Audit `admin.dispute_decided` with `{dispute_id, decision, refund_kobo, note}`.
6. Commit. Then `send_refund` and/or `send_payout` (when due), logging a failure only, because the sweepers retry.

- Owner paths outside a dispute: `release` with `Some(owner)`, `owner_refund_order` and `unhold` check `open_for_order` after the order lock, and answer 409 `dispute_open`.
- Route: `POST /admin/disputes/{id}/decide`, Owner only, with body `{decision, refund_kobo, note}`. `refund_kobo` is required for `partial_refund` and refused otherwise (422).

- [ ] **Step 1: Write the failing tests** in `tests/dispute_decisions.rs`:
  - `full_refund_decision`: the refund row is `kind 'full'` and the order is `refund_pending`, then `refunded`. The dispute is decided and the audit row has the note.
  - `release_decision`: a payout exists and the hold is cleared.
  - `partial_decision`: the amounts match Task 4.
  - `cannot_decide_before_seller_deadline`, and `can_decide_after_deadline_without_response`.
  - `support_cannot_decide`: 403.
  - `concurrent_decisions_act_once`: two decides race, giving one refund or payout and one 409.
  - `decision_stands_when_refund_provider_is_down`: `FakeProvider` is unavailable. The dispute is decided, the refund is `requested`, and the refund job sends it later.
  - `plain_owner_routes_refuse_open_dispute`: release, refund and unhold give `dispute_open`.
  - `withdrawn_dispute_cannot_be_decided`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement `decide`, the guards and the route.**
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: let the owner settle a dispute`

### Task 6: The staff dispute screen

**Files:**
- Create: `server/src/disputes/staff.rs`, `server/tests/admin_disputes.rs`
- Modify: `server/src/chat/service.rs` (staff transcript), `server/src/delivery/tracking.rs` (staff timeline), the admin dispute routes

**Interfaces:**
- Produces, in `chat/service.rs`:
  - `struct StaffMessage { seq: i64, from: Party, body: String, sent_at }`
  - `struct FlaggedOriginal { from: Party, severity: String, category: String, original: String, at }`
  - `async fn staff_transcript(db, cipher, order_id) -> Result<Vec<StaffMessage>, AppError>`: the order's conversation, delivered messages only, ascending `seq`.
  - `async fn flagged_originals(db, cipher, order_id) -> Result<Vec<FlaggedOriginal>, AppError>`: from `moderation_flags` on that conversation.
- Produces, in `tracking.rs`: `async fn staff_timeline(db, order_id) -> Result<Vec<Step>, AppError>`, the same steps the buyer sees, built by the code `get` uses.
- Produces, in `staff.rs`:
  - `async fn queue(db, status: Option<DisputeStatus>, limit) -> Result<Vec<QueueRow>, AppError>`. Oldest first. `QueueRow { id, order_id, status, reason, opened_at, seller_respond_by, items_total_kobo }`.
  - `async fn screen(state, staff: &Staff, dispute_id) -> Result<StaffDisputeView, AppError>`
  - `StaffDisputeView`:
    - `dispute: DisputeView`;
    - `order: AdminOrderView`, from `admin::views::order`;
    - `payment: { checkout_status, provider, provider_flagged, paid_at, total_kobo }`;
    - `ledger: Vec<{ at, account, amount_kobo, kind }>`;
    - `timeline: Vec<Step>`;
    - `delivery_events: Vec<{ source, status, occurred_at, received_at }>`;
    - `handover: { applies, issued_at, wrong_entries, locked_until, delivered_with_code }`, never the code. `wrong_entries` counts `order.handover_attempt` audit rows that failed. `delivered_with_code` is an `order.delivered` row with `via = handover_code`;
    - `delivery_area: Option<AreaCouncil>`;
    - `chat: Vec<StaffMessage>`;
    - `flagged: Option<Vec<FlaggedOriginal>>`, Owner only.
- Routes (Staff): `GET /admin/disputes?status=&limit=` audits `admin.viewed` with view `disputes`. `GET /admin/disputes/{id}` audits `admin.dispute_viewed` before responding.

- [ ] **Step 1: Write the failing tests** in `tests/admin_disputes.rs`:
  - `owner_screen_has_every_section`: a dispute on a self-delivery order with chat, one flagged message, a wrong handover entry and a seller response. Every section is filled, and `flagged` has the original.
  - `support_reads_open_dispute_without_originals`, and `support_never_sees_originals`: `flagged` is absent from the JSON.
  - `support_cannot_read_closed_dispute`: decided and withdrawn give 403 `dispute_closed`. The Owner still reads them.
  - `screen_has_no_buyer_contact`: the JSON contains neither the buyer's phone nor the address line, only the area council.
  - `every_read_is_audited`, with `originals` true for the Owner and false for Support.
  - `queue_is_oldest_first_and_filters`.
  - `buyers_and_sellers_get_403`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement the transcript, timeline, screen and routes.**
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: serve the owner's dispute screen`

### Task 7: The notification outbox, inbox and devices

**Files:**
- Create: `server/migrations/0035_notifications.sql`, `server/src/notify/mod.rs`, `server/src/notify/kinds.rs`, `server/src/notify/service.rs`, `server/src/notify/routes.rs`, `server/tests/notifications.rs`
- Modify: `server/src/lib.rs`, `server/src/chat/routes.rs` (marking read), `server/src/chat/service.rs`

**Schema (`0035`):**
- `notifications`:
  - `id uuid PK`, `user_id`, `kind text` CHECK in the 14 kinds, `order_id NULL`, `dispute_id NULL`, `conversation_id NULL`, `dedupe_key text NOT NULL`, `created_at`, `read_at NULL`;
  - `push_status text NOT NULL DEFAULT 'pending'` CHECK in (`pending`, `sending`, `sent`, `failed`, `skipped`), `push_attempts int DEFAULT 0`, `next_push_at timestamptz DEFAULT now()`, `push_until NULL`, `pushed_devices uuid[] NOT NULL DEFAULT '{}'`;
  - `UNIQUE (user_id, dedupe_key)`.
  - Indexes: `(user_id, created_at DESC, id DESC)`, a partial index on `(next_push_at) WHERE push_status = 'pending'`, and a partial index on `(push_until) WHERE push_status = 'sending'`.
- `push_devices`: `id uuid PK`, `user_id`, `session_id REFERENCES sessions`, `platform` CHECK in (`android`, `ios`), `token_enc bytea NOT NULL`, `token_idx bytea NOT NULL UNIQUE` (`crypto::blind_index` with `index_key`), `created_at`, `registered_at`, `disabled_at NULL`, `disabled_reason NULL`.
- `orders` gains `confirm_reminders_sent smallint NOT NULL DEFAULT 0 CHECK (BETWEEN 0 AND 2)`.
- Header comment: the push token is a device identifier, so it is personal data and is encrypted. The inbox holds ids and kinds only. Retention of both goes to the DPO's list (plan 8 purges).

**Interfaces:**
- Produces, in `kinds.rs`: `enum Kind` with the 14 variants, plus `as_str()`, `title() -> &'static str` and `body() -> &'static str`, using the exact copy from Global Constraints.
- Produces, in `service.rs`:
  - `struct Notice { user: Uuid, kind: Kind, order_id: Option<Uuid>, dispute_id: Option<Uuid>, conversation_id: Option<Uuid>, dedupe_key: String }`
  - `Notice::order(user, kind, order_id)` with key `"{kind}:{order_id}"`, and `Notice::dispute(user, kind, dispute_id)` with key `"{kind}:{dispute_id}"`
  - `async fn enqueue(conn: &mut PgConnection, n: Notice) -> Result<(), AppError>`, using `ON CONFLICT (user_id, dedupe_key) DO NOTHING`
  - `async fn mark_conversation_read(conn, user, conversation_id) -> Result<(), AppError>`
- Routes:
  - `GET /me/notifications?before=<id>&limit=` returns `{items: [{id, kind, title, body, order_id, dispute_id, conversation_id, created_at, read}], unread: i64}`. Newest first, keyset on `(created_at, id)`, default 30, max 100.
  - `POST /me/notifications/read` takes exactly one of `{"ids": [...]}` (at most 100) or `{"all": true}`. Only the caller's own rows are changed.
  - `PUT /me/push-device` takes `{token, platform}`, with the token 1 to 4096 printable ASCII characters. It upserts by `token_idx`, binding the token to the current user and session, and clears `disabled_at`.
  - `DELETE /me/push-device` disables the current session's devices.
  - Audit `push_device.registered` and `push_device.removed`, with the device id only.
- `POST /chats/{id}/read` also marks that conversation's `chat_message` notices read.

- [ ] **Step 1: Write the failing tests** in `tests/notifications.rs`:
  - `enqueue_is_idempotent`: the same key twice gives one row.
  - `inbox_pages_newest_first_and_counts_unread`.
  - `inbox_copy_comes_from_kind`: each kind's title and body match the table. A unit test in `kinds.rs` checks all 14.
  - `users_only_read_and_mark_their_own`.
  - `read_body_needs_exactly_one_mode`.
  - `device_token_is_encrypted_and_deduped`: the plaintext appears in no column, and registering again from a second account moves the row to that account.
  - `delete_device_disables_current_session_only`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement the migration, module and routes.**
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add the notification outbox, inbox and push devices`

### Task 8: Raising notices at each event, and the reminders job

**Files:**
- Create: `server/src/notify/reminders.rs`
- Modify: `server/src/checkout/settle.rs`, `server/src/orders/service.rs`, `server/src/escrow/refund.rs`, `server/src/escrow/release.rs`, `server/src/chat/send.rs`, `server/src/moderation/strikes.rs`, `server/src/disputes/service.rs`, `server/src/disputes/decide.rs`, `server/src/escrow/jobs.rs`
- Test: `server/tests/notifications.rs`

**Interfaces:**
- Consumes: `enqueue`, `Notice`, `Kind`.
- Each `enqueue` call sits in the transaction that makes the change. The seller is `stores.owner_id` of the order's store.
  - `order_paid`: in `settle.rs` `apply`, per order moved to `paid`.
  - `order_picked_up`, `order_out_for_delivery` and `order_delivered`: in `picked_up_in`, `out_for_delivery_in` and `delivered_in` when they return `true`. These cover seller steps and courier webhooks. The handover-code path uses `set_delivered` and sends no "delivered" notice, because the buyer was there.
  - `refund_started`: in `begin` and `partial_refund_in`, when a refund row is inserted.
  - `funds_released`: in `release_in` on `Done`.
  - `chat_message`: in `deliver_in`, to the other party. The key is `"chat:{conversation_id}:{recipient_read_seq}"`, so every message sent while the recipient has not read the conversation shares one notice.
  - `moderation_warning`: in `apply_strike` when a strike is inserted, keyed `"strike:{strike_id}"`.
  - `moderation_muted`: when a mute starts, keyed `"mute:{user}:{muted_until unix seconds}"`.
  - `dispute_opened` (seller), `return_requested` (both) and `dispute_decided` (both).
- Produces, in `reminders.rs`: `const CONFIRM_REMINDERS: &str = "confirm_reminders"` and `async fn run(state, cancel) -> Result<Stats, AppError>`. It applies the reminder rules from Global Constraints with `now()` from the database, advances `confirm_reminders_sent` under the order lock, and enqueues with keys `"confirm_reminder:{order}"` and `"release_tomorrow:{order}"`. It runs as job 13.

- [ ] **Step 1: Write the failing tests.**
  - `each_event_raises_its_notice`: one test per hook, checking the recipient, the kind and that nobody else got it.
  - `chat_notifications_coalesce_until_read`: three messages give one notice. After the recipient reads the conversation, the next message gives a second.
  - `handover_delivery_sends_no_delivered_notice`.
  - `reminders_follow_the_window`: with `release_after_hours = 72`, the job run at +23h sends nothing, at +25h sends `confirm_reminder`, and at +49h sends `release_tomorrow`. A run at +73h sends nothing more.
  - `missed_stage_is_skipped_not_sent_late`: the first run is at +49h, so only `release_tomorrow` is sent.
  - `held_order_gets_no_reminder`.
  - `rolled_back_change_leaves_no_notice`: a refund that fails its lock leaves no `refund_started`.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement the hooks and the job.**
- [ ] **Step 4: Run the tests.** Expected: PASS, with the full suite green: existing tests that count audit rows or rows per table may need their expectations widened to allow the new notices. Do not weaken anything else.
- [ ] **Step 5: Commit.** `feat: notify buyers and sellers at each order, chat and dispute event`

### Task 9: FCM push and the push loop

**Files:**
- Create: `server/src/notify/push.rs` (trait, `NoPush`), `server/src/notify/fcm.rs`, `server/src/notify/fake.rs`, `server/src/notify/worker.rs`, `server/tests/fcm_push.rs`, `server/tests/push_worker.rs`, `server/tests/fixtures/fcm_test_key.pem` (a throwaway RSA key generated for tests, with a header line saying so)
- Modify: `server/Cargo.toml`, `server/src/config.rs`, `server/src/lib.rs` (`AppState.push: Arc<dyn PushSender>`), `server/src/main.rs`, `server/tests/common/mod.rs` (`app.push: Arc<FakePush>`), `server/.env.example`

**Interfaces:**
- Produces, in `push.rs`:
  - `struct PushMessage<'a> { token: &'a str, title: &'static str, body: &'static str, kind: &'static str, notification_id: Uuid }`, with a hand-written `Debug` that redacts the token
  - `enum PushError { Unregistered, Rejected, Unavailable }`, with no inner detail
  - `#[async_trait] trait PushSender: Send + Sync { async fn send(&self, m: &PushMessage<'_>) -> Result<(), PushError>; }`
  - `struct NoPush`, which always returns Ok
- `fcm.rs`:
  - `struct FcmConfig { project_id: String, client_email: String, private_key: Secret, token_uri: String, base_url: String, connect_timeout: Duration, timeout: Duration, insecure_http_for_tests: bool }`
  - `FcmConfig::from_service_account_file(path) -> anyhow::Result<FcmConfig>`. The file is Google's service-account JSON. `token_uri` must equal `https://oauth2.googleapis.com/token` unless `insecure_http_for_tests`.
  - `struct FcmPush`, with the token cache behind a `tokio::sync::Mutex`.
- `fake.rs`: `FakePush` with `sent() -> Vec<(String /* token */, &'static str /* kind */, Uuid)>`, `set_next(Result<(), PushError>)`, `set_all(...)`, and `unregister(token)`, after which that token gets `Unregistered`. The default answer is Ok.
- `worker.rs`:
  - `async fn run_once(state: &AppState) -> Result<Stats, AppError>`. It claims up to 100 due rows with `FOR UPDATE SKIP LOCKED`, setting `sending` and a 2-minute `push_until`, then commits. For each notice it loads the eligible devices not in `pushed_devices` and sends with at most 8 calls in flight. It writes the outcome only if `push_until` is unchanged (fenced, like refunds).
  - Outcomes:
    - every device Ok, or none left: `sent`;
    - no eligible device ever: `skipped`;
    - older than 24h: `skipped`;
    - any `Unavailable`: back to `pending` with backoff, `push_attempts + 1`, and devices that succeeded added to `pushed_devices`;
    - after 6 attempts: `failed`;
    - `Unregistered`: that device gets `disabled_at` with `disabled_reason 'unregistered'`;
    - `Rejected`: that device is treated as done and its error class is logged.
  - `async fn run_loop(state, cancel)`: `run_once` every 5 seconds via `escrow::jobs::run_every`, started in `main.rs` beside the jobs loop and stopped by the same shutdown token.
- `Config.fcm: Option<FcmConfig>` from `FCM_SERVICE_ACCOUNT_FILE` and `FCM_BASE_URL` (optional, must be https). `main.rs` gets `require_push(app_env, configured)`, mirroring `require_moderation`.

- [ ] **Step 1: Write the failing tests.**
  - In `tests/fcm_push.rs`, with wiremock and the fixture key:
    - `mints_and_caches_the_token`: the assertion's header is RS256, and its claims match Global Constraints when verified with the fixture's public key. Two sends make one token call.
    - `sends_the_exact_body`: the body is exactly the documented shape, with no extra fields.
    - `refreshes_once_on_401`.
    - `maps_errors`: 404 `UNREGISTERED` gives `Unregistered`; 400 gives `Rejected`; 429, 500, 503, a timeout and an over-64-KiB body give `Unavailable`.
    - `errors_and_debug_hide_token_and_key`, with `tracing_test`.
    - Config tests: `production_requires_fcm`, `fcm_base_url_must_be_https`, `token_uri_must_be_google`.
  - In `tests/push_worker.rs`:
    - `push_payload_is_fixed_copy_and_ids`: what reaches `FakePush` is the kind's copy, the kind and the id, and nothing else.
    - `push_outage_keeps_inbox_and_retries`: the inbox row exists. With the fake unavailable, the notice backs off. When the fake is healthy again it is `sent`.
    - `revoked_session_gets_no_push`: sign out and the device is skipped, so the notice is `skipped`.
    - `retry_skips_devices_already_sent`: two devices, one fails once, and the retry sends only to that one.
    - `unregistered_token_is_disabled`.
    - `stale_notice_is_skipped`.
    - `gives_up_after_six_attempts`.
    - `late_answer_after_lease_is_not_recorded`.
- [ ] **Step 2: Run them.** `cargo test --test fcm_push --test push_worker`. Expected: FAIL.
- [ ] **Step 3: Implement the trait, adapter, fake, worker, config and state wiring.** Every `AppState` construction gets `push`.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: send notifications through FCM`

### Task 10: End-to-end path and docs

**Files:**
- Modify: `server/tests/disputes.rs`, `README.md`, `docs/superpowers/plans/2026-10-23-00-roadmap.md`, `docs/superpowers/specs/2026-10-22-abuja-marketplace-design.md`, `server/.env.example`

- [ ] **Step 1: Write the end-to-end test.** `full_path_from_delivery_to_partial_refund`:
  1. A courier order is delivered by webhook, and the buyer gets `order_delivered`.
  2. The buyer reports `damaged` with a photo key, and the seller gets `dispute_opened`.
  3. The seller responds. The owner requests a return paid by the seller, and the seller confirms receipt.
  4. The owner reads the screen and decides a partial refund.
  5. Both parties get `dispute_decided`, and the buyer gets `refund_started`.
  6. `FakePush` received each one. The ledger balances and the audit trail has every step.
- [ ] **Step 2: Update the docs.**
  - **README:** add "Disputes and notifications (part 5)" under "What works today", mark part 5 done in the roadmap table, and add the go-live items below.
  - **Roadmap:** mark plan 5 built with the test count, and add "Carried forward from plan 5":
    - **Plan 6:** the dispute queue and decision screens; charging unrecoverable provider fees, and return shipping, to the side at fault (spec §5); notification preferences, if the owner wants them.
    - **Plan 8:** keep chat, flag originals and dispute statements past 12 months while a dispute is open (`disputes.status` open); purge statements, inbox rows and disabled or stale push devices on retention windows the DPO sets; include disputes and the inbox in a user's export.
    - **Still open from plan 2:** a refund in `processing` has no exit path. A true clawback of a payout already scheduled is not built, because disputes are pre-release and flagged payments can no longer be released by the buyer.
  - **Spec §8 and §10:** record the my-calls:
    - one report per order, final on withdrawal;
    - statements checked offline only, never sent to Claude;
    - the owner waits out the seller's 48 hours;
    - partial refunds come out of the item total only;
    - return shipping is recorded, not charged;
    - the inbox stores no text, and pushes carry fixed copy and ids only;
    - coalesced chat notices;
    - reminder timing.
  - **`.env.example`:** `FCM_SERVICE_ACCOUNT_FILE` and `FCM_BASE_URL`, with placeholders only, and a note that the file is a secret kept outside the repository.
- [ ] **Step 3: Add these go-live gates** to the roadmap and README, as items for review and not as conclusions:
  - DPO and legal:
    - Google (Firebase) as a processor of push tokens, and the cross-border transfer under the NDPA;
    - the privacy notice naming push;
    - whether generic lock-screen copy is enough, or users should be able to opt out per kind.
  - DPO: retention of dispute statements and evidence photos, which may hold personal data of third parties; inbox rows; push devices; and Support's access to statements inside open disputes.
  - Evidence photos are not checked, and can carry phone numbers, as in reviews. The upload plan must check that a key belongs to the uploader.
  - Owner decisions:
    - whether the seller's 48 hours should pause on Sundays and holidays (today they are calendar hours, like the ship deadline);
    - whether a withdrawn report should ever be reopenable;
    - whether a dispute on a courier order in transit should cancel the courier.
- [ ] **Step 4: Run everything.** From `server/`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, then the full `cargo test`. Expected: all pass, with no suite skipped.
- [ ] **Step 5: Commit.** `test: add the dispute end-to-end path` and `docs: document disputes and notifications`

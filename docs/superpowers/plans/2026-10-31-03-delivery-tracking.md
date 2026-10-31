# Part 3: Delivery & Tracking Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Orders actually get delivered inside the FCT, by courier through Shipbubble or by the seller. This plan adds:
- delivery addresses and an FCT-only checkout;
- the delivery fee, charged at checkout;
- an honest promised date, with a 6pm cut-off;
- the seller's pickup and delivery steps, with a handover code;
- courier booking and status webhooks;
- a tracking timeline API.

Those steps unlock the release clock that part 2 built.

**Architecture:** A new `delivery` module and a `couriers` provider layer sit beside the existing `payments` layer:
- the `CourierProvider` trait, with a `FakeCourier` and a `ShipbubbleCourier` adapter;
- the `PostcodeProvider` trait, with a `ManualPostcode` implementation; the NIPOST adapter waits for API access.

Orders gain a delivery method, an encrypted address snapshot, a stored promised date and an `out_for_delivery` status. Courier bookings live in a `shipments` table. They are driven by the existing job loop, using the same patterns as payouts:
- a claim-then-act lease;
- no network calls inside a transaction;
- no blind retry after a request was sent.

Courier status changes go into an append-only `delivery_events` log. Statuses only ever move forward.

**Tech Stack:** Rust, axum 0.8, sqlx 0.8 on Postgres 17, tokio, reqwest 0.12 (rustls), and wiremock in the tests. All of these are already dependencies.

**Spec:** `docs/superpowers/specs/2026-10-22-abuja-marketplace-design.md`, §6 "Delivery". Also read:
- the roadmap section "Carried forward from plan 2";
- the research in `.superpowers/sdd/research-part3-delivery.md`, local and gitignored. The facts this plan relies on are copied inline.

## Global Constraints

- Codebase conventions apply everywhere, and every task's requirements implicitly include this section:
  - errors use the `AppError` JSON envelope;
  - request bodies use `AppJson<T>`, query strings use `AppQuery<T>`;
  - path ids are taken as `Path<String>` and parsed with `parse_id`, never `Path<Uuid>`;
  - money is kobo `i64`, with checked arithmetic and no rounding;
  - every state change writes to the append-only `audit_log`;
  - personal fields are encrypted at rest with `FieldCipher`;
  - personal data, addresses, phone numbers, API keys and raw webhook bodies are never logged.
- Global lock order, unchanged: users → stores → products → variants → access_codes → checkouts → orders → shipments → payouts. `payout_accounts` is locked last. New tables slot in as shown. Never make a network call inside a database transaction.
- Coverage: stores and delivery addresses must be inside the FCT. Buyers elsewhere can browse but not check out. Copy: "Coming to your city soon".
- The FCT is decided by the buyer choosing one of the six area councils: AMAC, Bwari, Gwagwalada, Kuje, Kwali, Abaji. A postcode, if given, must start with `FC`. A Shipbubble-validated address must have a state of FCT. Never infer the FCT from the word "Abuja" in free text: Suleja, Mararaba, Karu and Masaka are outside it.
- The NIPOST postcode is optional. Coverage is still incomplete (launched 1 Oct 2026, full coverage targeted for Dec 2026). Format: 11 characters, starting `FC`, separators optional. Normalise by stripping spaces and `-`, then uppercasing. Valid when it matches `^FC[0-9]{2}[A-Z0-9]{7}$`.
- Next-day promise: an order paid before the cut-off is promised for the next delivery day; after the cut-off, the delivery day after that.
  - The cut-off defaults to 18:00 Africa/Lagos and is owner-configurable.
  - Delivery days skip Sundays and rows in `bank_holidays`. *(My call: couriers do not run on Sundays or public holidays.)*
  - The date shown at checkout is recomputed and snapshotted at payment.
- What counts as "delivered":
  - **Self-delivery:** the seller cannot mark an order delivered without the buyer's handover code, and entering it counts as buyer confirmation, which triggers release.
  - **Courier:** Shipbubble's `completed` webhook sets the order to delivered and starts the 3-day release window. It never releases money by itself.
- Handover code: 6 digits, single use, encrypted at rest. The buyer sees it only while the order is `out_for_delivery`, labelled "Give this only to the person handing you the parcel". After 5 wrong entries, entry is locked for 1 hour.
  - *(My call: codes exist only on self-delivery orders. Couriers do not use our app, so a code on a courier order could not be checked by anyone.)*
- Phone and address reveal: a self-delivering seller sees the buyer's phone number and full address only while the order is `out_for_delivery`. Before that, the seller sees the area council only. Contact details are hidden again once the order is delivered, completed, cancelled or refunded, and every reveal is audited.
  - For courier orders, the seller never sees the buyer's phone or address; Shipbubble receives them for the label.
  - The tracking call button uses the courier's phone (from Shipbubble) or the store's business phone, never the buyer's or seller's personal number.
- Delivery fee: the buyer pays it at checkout. It is either the Shipbubble rate (`rate_card_amount`) or the store's flat self-delivery fee.
  - At release, a **courier** fee goes to `platform_revenue`, because the platform pays Shipbubble from its wallet.
  - A **self-delivery** fee goes to `seller_payable`.
  - The platform cut applies to items only.
- Shipbubble API facts (docs.shipbubble.com):
  - Every request uses `Authorization: Bearer <key>`. Key prefixes are `sb_sandbox` for sandbox and `sb_prod` for live, all on `https://api.shipbubble.com/v1/shipping`.
  - Endpoints:
    - `POST /address/validate`, which returns `address_code`.
    - `POST /fetch_rates`. Its fields include `sender_address_code` and the misspelled `reciever_address_code`. It returns `request_token`, valid for 7 days, and `couriers[]` with `courier_id`, `service_code`, `rate_card_amount`, `total` and `delivery_eta`.
    - `POST /labels` with `request_token`, `service_code` and `courier_id`. It returns `order_id`, `tracking_url` and `payment.shipping_fee`.
    - `POST /labels/cancel/:order_id`.
    - `GET /labels`.
  - Statuses: `pending`, `confirmed`, `picked_up`, `in_transit`, `completed`, `cancelled`.
  - Webhooks are signed with the header `x-ship-signature`: HMAC-SHA512 of the raw body, keyed with the API key. They are retried every 5 minutes, 5 times, and must get a 200 within 15 seconds. Payloads carry no event id.
  - No idempotency key exists for booking.
  - Amounts are NGN and believed to be whole naira, but this is unconfirmed. The unit is set by config (`SHIPBUBBLE_AMOUNT_UNIT`, `naira`|`kobo`), which is required in production.
- Production refuses to start when the Shipbubble key does not start with `sb_prod`, or when `SHIPBUBBLE_AMOUNT_UNIT` is missing. With no key configured, courier delivery is unavailable: stores offering only courier delivery cannot be checked out, and the server still starts.
- Commits: conventional messages. Never add a `Co-Authored-By` line, a "Generated with" footer, a session link or any other AI attribution.

## Review Focus

1. **A courier `completed` webhook arrives before `picked_up`, or after `cancelled`.** Statuses only ever move forward: a later rank wins, `cancelled` only applies before `picked_up`, and the order never moves backwards. Test: `courier_statuses_only_move_forward` (Task 10).
2. **The booking request times out after Shipbubble created the label.** A blind retry would book a second courier and charge twice. A timeout goes to `booking_unknown` and appears on the owner's attention list, with no automatic retry. Test: `booking_timeout_never_rebooks` (Task 9).
3. **The buyer's courier quote is stale or its price changed between checkout and booking.** The booking uses the stored token while it is under 7 days old and records the price it was charged. If the charge is above the quote by more than the tolerance, an attention item is raised; the buyer is never charged again. Test: `booked_price_above_tolerance_is_flagged` (Task 9).
4. **A seller guesses the handover code.** After 5 wrong entries, entry is locked for 1 hour (409 `handover_locked`), and every attempt is audited without the code itself. Test: `handover_code_locks_after_five_wrong` (Task 8).
5. **An order is refunded after the courier was booked.** The refund still happens. A cancel is sent to Shipbubble after commit. If the cancel is refused ("already processed"), the shipment is flagged for the owner, and the refund is never blocked by the courier. Test: `refund_cancels_booked_shipment_or_flags_it` (Task 9).

---

### Task 1: Delivery settings, service area and promised date

**Files:**
- Create: `server/migrations/0022_delivery_settings.sql`, `server/src/delivery/mod.rs`, `server/src/delivery/area.rs`, `server/src/delivery/promise.rs`
- Modify: `server/src/settings/service.rs` (new fields, patch and validation), `server/src/lib.rs` (`pub mod delivery`)
- Test: unit tests in `area.rs` and `promise.rs`, and `server/tests/settings.rs`

**Interfaces:**
- Produces:
  - `delivery::area::AreaCouncil`: an enum of `Amac`, `Bwari`, `Gwagwalada`, `Kuje`, `Kwali`, `Abaji`. It has serde `snake_case`, `sqlx::Type` (`area_council`), and `FromStr` returning 422 `outside_service_area` with the message "Coming to your city soon".
  - `delivery::area::normalize_postcode(&str) -> Result<String, AppError>`. It returns 422 `invalid_postcode`, or `outside_service_area` when the code doesn't start with `FC`.
  - `delivery::promise::promised_date(paid_at: OffsetDateTime, cutoff_minutes: i32, holidays: &[Date]) -> Date`. It works in Africa/Lagos (+01:00, no DST). Load holidays with `delivery::promise::load_holidays(&PgPool) -> Result<Vec<Date>, AppError>`.
  - `Settings` gains these fields, with their ranges in `validate`:
    - `delivery_cutoff_minutes: i32`: default 1080, range 0..=1439.
    - `courier_quote_ttl_minutes: i32`: default 30, range 5..=1440. This is how long a delivery option shown at checkout can be used.
    - `courier_price_tolerance_bps: i32`: default 1000, range 0..=5000.
    - `courier_category_id: i64`: default 90097994, Shipbubble's "Accessories". *(My call: one platform-wide category until products map to courier categories.)*

- [ ] **Step 1: Write the failing tests.**
  - `promised_date_before_cutoff_is_next_day`: Monday 17:59 WAT gives Tuesday.
  - `promised_date_after_cutoff_skips_a_day`: Monday 18:00 gives Wednesday.
  - `promised_date_skips_sunday_and_holidays`: Saturday 10:00 gives Monday, and Saturday 10:00 with Monday a holiday gives Tuesday.
  - `promised_date_uses_lagos_time`: 2026-11-02T16:59Z, which is 17:59 WAT, gives 2026-11-03.
  - `postcode_normalises_and_requires_fc`: `"fc-02 a09db09"` gives `"FC02A09DB09"`, `"LA02A09DB09"` gives `outside_service_area`, and `"FC02"` gives `invalid_postcode`.
  - `area_council_rejects_unknown`: `"suleja"` gives `outside_service_area`.
  - In `tests/settings.rs`, `delivery_settings_round_trip_and_validate`: patching the cut-off to 1440 gives 422.
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test --lib delivery` and `cargo test --test settings delivery`. Expected: compile errors or failures.
- [ ] **Step 3: Implement the migration (new columns with the defaults above), the two modules and the settings fields.**
- [ ] **Step 4: Run the tests again.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add delivery settings, FCT area check and promised date`

### Task 2: Buyer delivery addresses and the postcode layer

**Files:**
- Create: `server/migrations/0023_buyer_addresses.sql`, `server/src/delivery/addresses.rs` (service and routes), `server/src/delivery/postcode.rs`
- Modify: `server/src/lib.rs` (router, and `AppState.postcodes: Arc<dyn PostcodeProvider>`), `server/src/main.rs` (wire `ManualPostcode`)
- Test: `server/tests/addresses.rs`

**Interfaces:**
- Consumes: `AreaCouncil` and `normalize_postcode` from Task 1.
- Produces:
  - The `buyer_addresses` table:
    - `id`, `user_id`
    - `recipient_name_enc`, `phone_enc`, `address_line_enc`: all `bytea`, encrypted
    - `landmark_enc bytea NULL`
    - `area_council area_council NOT NULL`
    - `postcode text NULL CHECK (postcode ~ '^FC[0-9]{2}[A-Z0-9]{7}$')`
    - `courier_address_code bigint NULL`
    - `deleted_at`, `created_at`
  - Routes:
    - `POST /me/addresses` (201) with `{recipient_name, phone, address_line, landmark?, area_council, postcode?}`. The phone is validated with the existing `phone` module.
    - `GET /me/addresses`: the user's own addresses only, decrypted.
    - `DELETE /me/addresses/{id}`: a soft delete via `deleted_at`. An address used by an order keeps its encrypted snapshot on the order (Task 6).
  - `delivery::addresses::load_for_checkout(db, cipher, user, id) -> Result<DeliveryAddress, AppError>`. It returns 404 for another user's address or a deleted one. `DeliveryAddress` holds the decrypted fields, with a `Debug` that redacts them.
  - `trait PostcodeProvider: Send + Sync { async fn check(&self, code: &str) -> Result<PostcodeCheck, ProviderError>; }` with `PostcodeCheck { Valid, Unknown, Invalid }`.
    - `ManualPostcode` returns `Unknown` for any well-formed code.
    - A NIPOST adapter is NOT built: its API is undocumented and needs a key. Leave a `// REVISIT(nipost)` note.

- [ ] **Step 1: Write the failing tests.**
  - `create_list_delete_address`
  - `address_outside_fct_is_refused`: area `"suleja"` gives 422 `outside_service_area` with the message "Coming to your city soon".
  - `postcode_must_be_fct_when_given`
  - `addresses_are_private_to_their_owner`: another user's id gives 404 on GET and DELETE.
  - `address_fields_encrypted_at_rest`: raw SQL shows no plaintext in any `_enc` column.
  - `address_logs_have_no_personal_data`: uses `tracing-test`; the address line is not in the logs.
  - `deleted_address_cannot_be_used`
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test --test addresses`.
- [ ] **Step 3: Implement the migration, the service, the routes and `ManualPostcode`.**
- [ ] **Step 4: Run the tests again.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add buyer delivery addresses inside the FCT`

### Task 3: Store delivery details

**Files:**
- Create: `server/migrations/0024_store_delivery.sql`
- Modify: `server/src/stores/service.rs` (create and patch fields, owner view, redacted `Debug`)
- Test: `server/tests/stores.rs`

**Interfaces:**
- Produces:
  - New `stores` columns:
    - `pickup_area_council area_council NULL`
    - `pickup_postcode text NULL`, with the same CHECK as buyer addresses
    - `pickup_courier_address_code bigint NULL`
    - `business_phone_enc bytea NULL`
    - `self_delivery_fee_kobo bigint NULL CHECK (self_delivery_fee_kobo BETWEEN 0 AND 5000000)`
  - Validation on create and patch:
    - `delivery_self = true` requires `self_delivery_fee_kobo` and `business_phone`.
    - `delivery_courier = true` requires `pickup_area_council`.
    - Missing fields give 422 `delivery_details_required`.
  - Existing stores keep NULLs. They cannot be checked out for the affected method until the fields are filled (Task 6).
  - `stores::service::delivery_profile(db, cipher, store_id) -> Result<StoreDelivery, AppError>`, where `StoreDelivery { courier: bool, self_fee_kobo: Option<i64>, business_phone: Option<String>, pickup_area: Option<AreaCouncil>, pickup_address: String, pickup_courier_address_code: Option<i64> }`.

- [ ] **Step 1: Write the failing tests.**
  - `self_delivery_store_requires_fee_and_phone`
  - `courier_store_requires_pickup_area`
  - `business_phone_encrypted_and_owner_only`: the public store view has no phone and no fee internals; the owner view has both.
  - `self_delivery_fee_capped`: 5,000,001 kobo gives 422.
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test --test stores delivery`.
- [ ] **Step 3: Implement the migration, the validation and `delivery_profile`.**
- [ ] **Step 4: Run the tests again.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add store delivery details for courier and self-delivery`

### Task 4: Courier provider layer and the fake courier

**Files:**
- Create: `server/src/couriers/mod.rs`, `server/src/couriers/fake.rs`
- Modify: `server/src/lib.rs` (`AppState.courier: Option<Arc<dyn CourierProvider>>`), `server/tests/common/mod.rs` (`app.courier` fake handle)
- Test: unit tests in `fake.rs`

**Interfaces:**
- Produces, all in `couriers/mod.rs`. Every enum is `#[non_exhaustive]`.
  - `struct ContactAddress { name, email: Option<String>, phone, address_line, area: AreaCouncil }`, with a redacted `Debug`.
  - `struct Parcel { items: Vec<ParcelItem{name, unit_weight_grams: i32, unit_amount_kobo: i64, quantity: i32}>, length_cm, width_cm, height_cm, category_id: i64 }`.
  - `struct Rate { courier_id: String, service_code: String, courier_name: String, amount_kobo: i64, charged_kobo: i64, delivery_eta: Option<String> }`.
  - `struct RateQuote { request_token: String, rates: Vec<Rate> }`.
  - `struct Booking { provider_order_id: String, tracking_url: Option<String>, courier_name: String, courier_phone: Option<String>, charged_kobo: Option<i64> }`.
  - `enum ShipmentState { Pending, Confirmed, PickedUp, InTransit, Completed, Cancelled }`, with `fn rank(self) -> u8`: 1, 2, 3, 4, 5, and 0 for Cancelled.
  - `struct CourierUpdate { provider_order_id: String, state: ShipmentState, occurred_at: Option<OffsetDateTime>, courier_phone: Option<String>, tracking_url: Option<String> }`.
  - `enum CourierError`, mirroring `ProviderError`: `Rejected{code,message}`, `Timeout`, `Unavailable`, `Inconclusive`, `InvalidSignature`, `NotSupported`.
  - `#[async_trait] trait CourierProvider: Send + Sync` with:
    - `name()`
    - `validate_address(&ContactAddress) -> Result<i64, CourierError>`, returning the provider address code
    - `quote(sender_code: i64, receiver_code: i64, pickup: Date, &Parcel) -> Result<RateQuote, CourierError>`
    - `book(request_token, courier_id, service_code) -> Result<Booking, CourierError>`
    - `cancel(provider_order_id) -> Result<(), CourierError>`
    - `status(provider_order_id) -> Result<CourierUpdate, CourierError>`
    - `parse_webhook(headers: &HeaderMap, body: &[u8]) -> Result<Option<CourierUpdate>, CourierError>`, where `None` means an ignored event.
  - `couriers::parcel_for(items: &[(ProductDims, i32)], category_id) -> Parcel`. Use the largest length and width, and sum height × quantity. Missing dimensions default to 10 cm. *(My call.)*
  - `FakeCourier` with scripted answers (`fail_book_with`, `delay_next_book`, `set_rates`, `push_update`) and call counters, in the style of `payments::fake`.

- [ ] **Step 1: Write the failing tests.**
  - `parcel_for_stacks_heights_and_defaults_dims`
  - `shipment_state_rank_is_forward_order`
  - `fake_records_calls`
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test --lib couriers`.
- [ ] **Step 3: Implement the trait, the types, `parcel_for` and the fake.**
- [ ] **Step 4: Run the tests again.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add courier provider layer with a fake courier`

### Task 5: Shipbubble adapter

**Files:**
- Create: `server/src/couriers/shipbubble.rs`, `server/tests/shipbubble_adapter.rs`
- Modify: `server/src/config.rs` (`SHIPBUBBLE_API_KEY`, `SHIPBUBBLE_BASE_URL` defaulting to `https://api.shipbubble.com/v1/shipping`, `SHIPBUBBLE_AMOUNT_UNIT`, and the production checks), `server/src/main.rs`, `server/.env.example` (placeholders only)
- Test: `server/tests/shipbubble_adapter.rs` (wiremock, no network)

**Interfaces:**
- Consumes: the trait and types from Task 4.
- Produces: `ShipbubbleCourier::new(cfg: ShipbubbleConfig) -> anyhow::Result<Self>`, implementing `CourierProvider` with the endpoints in Global Constraints.
  - **HTTP client:** copy the Mono client's settings exactly.
    - rustls only, connect timeout 5 s, total timeout 15 s, no redirects, HTTPS only
    - response bodies capped at 512 KiB
    - errors passed through `without_url()`
    - the key in a `Secret` newtype with a redacted `Debug`
  - **Amounts:** convert in one function pair, `to_sb_amount` and `from_sb_amount`, according to `SHIPBUBBLE_AMOUNT_UNIT`. Use checked arithmetic. A non-whole naira amount is an error, never rounded.
  - **`book` failures:**
    - a connect failure becomes `Unavailable`;
    - a timeout, a 5xx, a transport error after sending, or a 408, 409 or 425 becomes `Timeout`;
    - any other 4xx becomes `Rejected`;
    - `book` is never retried inside the adapter.
  - **Webhooks:** verify `x-ship-signature` as hex HMAC-SHA512 of the raw bytes, keyed with the API key, using a constant-time compare, before parsing.
    - `shipment.status.changed` and `shipment.cancelled` give `Some(update)`.
    - `shipment.label.created`, `shipment.sla.updated` and the wallet events give `None`; wallet events also log `wallet_alert` with the event name only.
    - An unknown event gives `None`.
    - An unknown status string gives `None` and logs its cause.
  - **Rates:** `amount_kobo` comes from `rate_card_amount`, and `charged_kobo` from `total`.

- [ ] **Step 1: Write the failing tests against wiremock fixtures copied from the documented samples.**
  - `validate_address_returns_code`
  - `quote_maps_couriers_and_amounts`: naira 2785 gives 278,500 kobo; a non-whole naira amount is an error.
  - `book_is_sent_once_and_timeout_is_timeout`: uses `expect(1)` and a delayed response.
  - `book_4xx_is_rejected_and_409_is_timeout`
  - `webhook_bad_signature_is_invalid`
  - `webhook_status_changed_maps_state`
  - `webhook_unknown_event_is_ignored`
  - `debug_never_prints_key`
  - `config_requires_sb_prod_and_unit_in_production`
  - an `#[ignore]` `sandbox_smoke_test`, gated on environment variables
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test --test shipbubble_adapter`.
- [ ] **Step 3: Implement the adapter and the config.**
- [ ] **Step 4: Run the tests again.** Expected: PASS, with 1 ignored.
- [ ] **Step 5: Commit.** `feat: add Shipbubble courier adapter`

### Task 6: Delivery options at checkout

**Files:**
- Create: `server/migrations/0025_order_delivery.sql`, `server/src/delivery/options.rs`
- Modify: `server/src/checkout/service.rs` (`create` takes deliveries), `server/src/checkout/routes.rs` (body), `server/src/checkout/settle.rs` (snapshot `promised_date` at payment)
- Test: `server/tests/checkout.rs`, `server/tests/delivery_options.rs`

**Interfaces:**
- Consumes:
  - Tasks 1 to 4: `promised_date`, `load_for_checkout`, `delivery_profile`, `CourierProvider`, `parcel_for`.
- Produces:
  - **The `delivery_options` table.** It holds what the buyer can pick for one store and one address:
    - `id`, `buyer_id`, `store_id`, `address_id`
    - `method delivery_method` (enum `courier`|`self`)
    - `fee_kobo`
    - `courier_id`, `service_code`, `request_token`, `courier_name`, `delivery_eta`, `charged_kobo`
    - `promised_date`, `expires_at`, `created_at`
  - **`POST /checkout/delivery-options`.** Request: `{address_id}`. Response: `{stores: [{store_id, store_name, options: [{id, method, fee_kobo, courier_name?, delivery_eta?, promised_date}]}]}`, covering every store in the cart.
    - The self-delivery option is offered when the store has `self_delivery_fee_kobo`.
    - Courier options are offered when the store has `delivery_courier` and a courier provider is configured. The address codes for the store and the buyer are fetched lazily and cached on their rows.
    - Options expire after `courier_quote_ttl_minutes`.
    - Provider errors leave that store with self-delivery only. If no option is left, the response includes `"unavailable": "no_delivery_option"` for that store.
  - **The checkout body becomes `{method, address_id, deliveries: [{store_id, option_id}]}`.** Exactly one option per order in the cart is required.
    - Unknown, expired or someone else's options give 422 `delivery_option_invalid`.
    - A store left without an option gives 422 `delivery_option_required`.
    - The checkout `total_kobo` is the items plus every `fee_kobo`.
  - **New `orders` columns:**
    - `delivery_method delivery_method NULL` (NULL on part-2 orders)
    - `delivery_option_id`
    - `delivery_address_enc bytea`, an encrypted JSON snapshot of the `DeliveryAddress`
    - `delivery_area area_council`
    - `promised_date date`
    - `courier_request_token`, `courier_id`, `service_code`, `courier_quote_kobo`
  - **`orders.promised_date`** is recomputed with `promised_date(paid_at, …)` when payment settles, inside the settle transaction.

- [ ] **Step 1: Write the failing tests.**
  - `options_offer_self_and_courier_with_promised_date`
  - `courier_failure_leaves_self_only`
  - `checkout_charges_items_plus_delivery`: exact kobo; the fake payment amount equals the items plus the fees.
  - `checkout_refuses_expired_or_foreign_option`
  - `checkout_requires_one_option_per_store`
  - `checkout_refuses_address_outside_fct`: the address is forced to an invalid area by raw SQL, giving 422.
  - `promised_date_snapshotted_at_payment`: the payment lands after the cut-off, so the date moves one delivery day later than shown.
  - `order_address_snapshot_encrypted`
  - The existing checkout tests are updated to pass a self-delivery option.
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test --test delivery_options` and `cargo test --test checkout`.
- [ ] **Step 3: Implement the migration, `options.rs` and the checkout and settle changes.** The provider calls in `delivery-options` happen before any transaction.
- [ ] **Step 4: Run the full suite.** Run `cargo test`. Expected: all pass. Earlier tests that assumed a delivery fee of 0 now pass an option of fee 0.
- [ ] **Step 5: Commit.** `feat: charge delivery at checkout with courier and self-delivery options`

### Task 7: Route the delivery fee at release

**Files:**
- Modify: `server/src/escrow/release.rs` (ledger split by `delivery_method`), `server/src/orders/views.rs` (`expected_payout_kobo` includes the self-delivery fee)
- Test: `server/tests/escrow.rs`

**Interfaces:**
- Consumes: `orders.delivery_method` from Task 6.
- Produces: at release, `delivery_method = 'self'` sends the delivery fee to `seller_payable`. `courier` or NULL sends it to `platform_revenue`, which is the existing behaviour. The payout amount includes the self-delivery fee. Payout fee and stamp duty are computed on the new payout amount, using the existing rules.

- [ ] **Step 1: Write the failing tests.**
  - `self_delivery_fee_goes_to_seller`: items ₦10,000 and a fee of ₦1,500. Assert the exact `seller_payable`, `platform_revenue` and payout amounts, a per-order sum of 0, and escrow 0.
  - `courier_fee_stays_platform_revenue`
  - `seller_view_expected_payout_includes_self_fee`
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test --test escrow delivery_fee`.
- [ ] **Step 3: Implement the change.**
- [ ] **Step 4: Run the tests again.** Expected: PASS, including the existing `release_ledger_routes_delivery_to_platform_revenue`.
- [ ] **Step 5: Commit.** `fix: pay self-delivery fees to the seller at release`

### Task 8: Seller delivery steps and handover code

**Files:**
- Create: `server/migrations/0026_out_for_delivery.sql` (only `ALTER TYPE order_status ADD VALUE 'out_for_delivery' AFTER 'picked_up'`, alone, because a new enum value can't be used in the transaction that adds it), `server/migrations/0027_handover.sql`, `server/src/delivery/handover.rs`
- Modify: `server/src/orders/service.rs` (transitions), `server/src/orders/routes.rs`, `server/src/escrow/release.rs` (statuses that may release), `server/src/escrow/refund.rs` and `server/src/admin/orders.rs` (the owner's hold and refund accept `out_for_delivery` wherever they accept `picked_up`), `server/src/checkout/settle.rs` (generate the handover code at payment)
- Test: `server/tests/delivery_steps.rs`

**Interfaces:**
- Consumes: `release_order(state, id, ReleaseReason::BuyerConfirmed)`.
- Produces:
  - **Columns:**
    - `orders.ready_at timestamptz NULL`, `orders.out_for_delivery_at timestamptz NULL`
    - `orders.handover_code_enc bytea NULL`, set at payment for `self` orders
    - `orders.handover_failures int NOT NULL DEFAULT 0`, `orders.handover_locked_until timestamptz NULL`
  - **Transitions.** Each one takes the order lock, is idempotent in its target state, and returns 409 `invalid_order_state` otherwise:
    - **Self-delivery:**
      - `paid → picked_up`, via `POST /orders/{id}/picked-up`. Seller only.
      - `picked_up → out_for_delivery`, via `POST /orders/{id}/out-for-delivery`. Seller only.
      - `out_for_delivery → delivered`, via `POST /orders/{id}/delivered` with `{handover_code}`. Seller only. Afterwards, `release_order(BuyerConfirmed)` runs outside the transaction.
    - **Courier:** `POST /orders/{id}/ready` sets `ready_at` on a `paid` order (seller only) and creates the shipment (Task 9). The courier statuses themselves come from Task 10.
  - **Handover check.** `delivery::handover::verify(conn, order_id, code) -> Result<(), AppError>`:
    - when locked, returns 409 `handover_locked`;
    - on a mismatch, increments `handover_failures`; at 5, sets `locked_until = now() + 1h` and resets the counter; returns 422 `handover_code_wrong`;
    - compares in constant time;
    - writes the audit `order.handover_attempt {ok: bool}`, never the code.
  - **Code generation.** `delivery::handover::generate() -> String`: 6 digits from `OsRng`.
  - **Release changes.** `release_order` and buyer "Received" also accept `out_for_delivery`. The existing `mark_delivered` accepts `picked_up` or `out_for_delivery`.
  - **The ship deadline counts as met** for a `self` order once it is `picked_up`.

- [ ] **Step 1: Write the failing tests.**
  - `self_delivery_steps_in_order`: paid → picked_up → out_for_delivery → delivered with the code, then completed with a payout created.
  - `delivered_without_right_code_is_refused`
  - `handover_code_locks_after_five_wrong`: the 6th attempt gives 409 `handover_locked`, even with the right code; after the lock expires (back-dated in SQL) the right code works.
  - `handover_attempt_audit_has_no_code`
  - `only_the_store_owner_can_step`: the buyer, another seller and Support each get 403 or 404.
  - `courier_orders_cannot_use_self_steps`: gives 409.
  - `ready_marks_courier_order`
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test --test delivery_steps`.
- [ ] **Step 3: Implement the migrations, the transitions, the routes and `handover.rs`.**
- [ ] **Step 4: Run the full suite.** Run `cargo test`. Expected: all pass.
- [ ] **Step 5: Commit.** `feat: add seller delivery steps and handover codes`

### Task 9: Courier booking job

**Files:**
- Create: `server/migrations/0028_shipments.sql`, `server/src/delivery/shipments.rs`
- Modify: `server/src/escrow/jobs.rs` (new `book_shipments` job, `JobReport.shipments_booked`), `server/src/escrow/refund.rs` (cancel after commit), `server/src/admin/views.rs` (attention items)
- Test: `server/tests/shipments.rs`

**Interfaces:**
- Consumes: `CourierProvider::book` and `cancel`, the courier columns on `orders` from Task 6, and `ready_at` from Task 8.
- Produces:
  - **The `shipments` table.** `order_id` is UNIQUE.
    - `id`, `order_id`, `provider`
    - `status shipment_status`. The enum is `booking_pending`, `booking`, `booking_unknown`, `booking_failed`, `booked`, `confirmed`, `picked_up`, `in_transit`, `completed`, `cancelled`, `cancel_failed`.
    - `provider_order_id text UNIQUE NULL`, `tracking_url`, `courier_name`, `courier_phone_enc`
    - `quoted_kobo`, `charged_kobo`
    - `attempts`, `sending_until`, `next_attempt_at`, `last_error`
    - `created_at`, `updated_at`
  - **Booking job.** The claim selects `booking_pending` rows due by `next_attempt_at`, excluding held orders and flagged checkouts. It uses `ORDER BY created_at, id FOR UPDATE SKIP LOCKED LIMIT 20` and holds a 2-minute lease in `booking` status. `book` is called outside the transaction, and the outcome is written fenced on the lease. Outcomes:
    - `Ok`: `booked`, storing `charged_kobo`. If it is above `quoted_kobo × (1 + tolerance)`, write the audit `shipment.price_above_quote {quoted, charged}`.
    - `Unavailable`: back to `booking_pending`, backing off from 5 minutes, doubling, capped at 6 hours, for at most 8 attempts, then `booking_failed`.
    - `Timeout` or `Inconclusive`: `booking_unknown`. Never retried automatically.
    - `Rejected`: `booking_failed`, with `last_error = code`.
  - **Stale token.** When the order's `request_token` is 7 days old or more, mark the shipment `booking_failed` with `last_error = 'quote_expired'`. Re-quoting is an owner action in plan 6. *(My call: a seller has 48 hours to be ready, so this needs an owner override path, not a hot path.)*
  - **Refund hook.** After the refund commits, if the shipment is `booked` or `confirmed`, call `cancel` outside any transaction:
    - `Ok`: `cancelled`.
    - Any error: `cancel_failed` with an audit row. The refund itself never depends on the cancel.
  - **Attention items.** `GET /admin/reconciliation` gains `shipments_attention` (`booking_unknown`, `booking_failed`, `cancel_failed`, price above quote) and `wallet_alerts` (from the audit). Support sees no courier phone.

- [ ] **Step 1: Write the failing tests.**
  - `ready_order_gets_booked_once`
  - `booking_timeout_never_rebooks`: assert the fake's book count is 1 after 3 ticks, and the status is `booking_unknown`.
  - `booking_unavailable_backs_off_then_fails_after_8`
  - `booked_price_above_tolerance_is_flagged`
  - `quote_older_than_7_days_fails_booking`
  - `refund_cancels_booked_shipment_or_flags_it`
  - `held_order_not_booked`
  - `shipments_appear_on_attention_list_without_phone_for_support`
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test --test shipments`.
- [ ] **Step 3: Implement the migration, the job, the refund hook and the attention items.**
- [ ] **Step 4: Run the full suite.** Run `cargo test`. Expected: all pass.
- [ ] **Step 5: Commit.** `feat: book couriers for ready orders from the job loop`

### Task 10: Courier webhooks, delivery events and reconciliation poll

**Files:**
- Create: `server/migrations/0029_delivery_events.sql`, `server/src/delivery/events.rs`, `server/src/couriers/webhooks.rs`
- Modify: `server/src/lib.rs` (route `POST /webhooks/couriers/{provider}`, 64 KiB body limit), `server/src/escrow/jobs.rs` (`poll_shipments` job)
- Test: `server/tests/courier_webhooks.rs`

**Interfaces:**
- Consumes: `CourierProvider::parse_webhook` and `status`, the `shipments` table, and `orders::service::mark_picked_up` and `mark_delivered`.
- Produces:
  - **`delivery_events`, an append-only table.** It has the UPDATE, DELETE and TRUNCATE triggers with `ENABLE ALWAYS`, copying the ledger's pattern.
    - Columns: `id`, `order_id`, `source` (`seller`|`courier`|`system`), `status text`, `occurred_at`, `received_at`.
    - `dedupe_key text UNIQUE`. For courier events it is sha256(provider ‖ provider_order_id ‖ state ‖ occurred_at or "").
  - **`delivery::events::apply_courier_update(state, update) -> Result<Applied, AppError>`.** It locks the order, then the shipment, and applies the update only when its rank is above the current shipment rank. `cancelled` applies only below `picked_up`.
    - `PickedUp` calls `mark_picked_up`.
    - `InTransit` sets the order to `out_for_delivery`.
    - `Completed` calls `mark_delivered` with **our receipt time**, so the release window starts when we learned of it and a back-dated status cannot shorten it.
    - `Cancelled` before pickup sets the shipment to `cancelled` with an attention item. The order stays `paid` with `ready_at` kept, so the owner can re-book.
    - Lower-ranked or duplicate updates return `Applied::Ignored`, logged with the cause only.
    - The seller's steps (Task 8) also append `delivery_events` with `source = seller`.
  - **Webhook route.** It verifies the signature through `parse_webhook` (bad signature: 401), records the event, applies it, and answers 200.
    - An unknown `provider_order_id` gets 200 and is logged as `unknown_shipment`.
    - Lock busy gets 503, so the provider retries.
  - **`poll_shipments` job.** For shipments in `booked`, `confirmed`, `picked_up` or `in_transit` with no event for 6 hours, it calls `status()` outside any transaction, at most 20 per tick, and feeds the result through `apply_courier_update`.

- [ ] **Step 1: Write the failing tests.**
  - `courier_statuses_only_move_forward`: `completed` then `picked_up` leaves the order delivered, with 2 events and the second ignored.
  - `completed_starts_release_window_not_release`: the order is delivered with `release_at` 72 hours after receipt, and no payout exists.
  - `duplicate_webhook_is_deduped`
  - `bad_signature_is_401_and_nothing_written`
  - `cancel_after_pickup_is_ignored`
  - `poll_moves_stuck_shipment`
  - `delivery_events_are_append_only`: UPDATE, DELETE and TRUNCATE all fail.
  - `webhook_logs_no_body_or_phone`
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test --test courier_webhooks`.
- [ ] **Step 3: Implement the migration, `events.rs`, the webhook route and the poll job.**
- [ ] **Step 4: Run the full suite.** Run `cargo test`. Expected: all pass.
- [ ] **Step 5: Commit.** `feat: track courier status from signed webhooks and polling`

### Task 11: Tracking API and contact reveal

**Files:**
- Create: `server/src/delivery/tracking.rs`
- Modify: `server/src/orders/routes.rs` (`GET /orders/{id}/tracking`), `server/src/orders/views.rs` (seller contact block)
- Test: `server/tests/tracking.rs`

**Interfaces:**
- Consumes: the order timestamps, `delivery_events`, `shipments` and the handover code.
- Produces: `GET /orders/{id}/tracking`, for the buyer and the store owner only (others get 404). It returns:
  - `promised_date`
  - `status`
  - `steps`, in this fixed order: `paid` "Paid", `preparing` "Seller preparing", `picked_up` "Picked up", `out_for_delivery` "Out for delivery", `delivered` "Delivered", `confirmed` "Confirmed". Each step has `{key, label, at: Option<timestamp>, current: bool}`.
  - `contact`: `{name, phone}` for the courier (courier orders) or the store's business phone (self-delivery), shown to the buyer.
  - `tracking_url`: buyer only.
  - `handover_code`: buyer only, self-delivery orders only, only while `out_for_delivery`, with the note "Give this only to the person handing you the parcel".
  - `delay_note`: optional.

  For the store owner of a self-delivery order, it also returns `buyer_contact {recipient_name, phone, address_line, landmark}`, only while `out_for_delivery`. Each response that reveals contact writes the audit `order.contact_revealed` with no personal data in the detail. Outside that window, it returns only `delivery_area`.

- [ ] **Step 1: Write the failing tests.**
  - `tracking_steps_fill_in_as_order_moves`
  - `handover_code_shown_only_out_for_delivery`
  - `seller_sees_buyer_contact_only_out_for_delivery_and_audited`
  - `courier_order_seller_never_sees_buyer_contact`
  - `contact_hidden_after_delivery`
  - `tracking_private_to_buyer_and_seller`
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test --test tracking`.
- [ ] **Step 3: Implement `tracking.rs` and the route.**
- [ ] **Step 4: Run the tests again.** Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add order tracking timeline and timed contact reveal`

### Task 12: Ship deadline that matches delivery

**Files:**
- Create: `server/migrations/0030_ship_deadline_on.sql`
- Modify: `server/src/escrow/jobs.rs` (ship-deadline claim)
- Test: `server/tests/jobs.rs`

**Interfaces:**
- Produces:
  - **What the ship-deadline job now refunds and strikes:** only orders still `paid` past `ship_by` where the seller did not act. That means a self-delivery order not yet picked up, or a courier order with `ready_at IS NULL`.
  - **A courier order marked ready before `ship_by`** is never struck. If it is still not picked up 24 hours after `ship_by`, write the audit `shipment.pickup_overdue` once, and add it to the attention list.
  - **The migration** sets the `jobs_ship_deadline_enabled` default to true and updates the existing row to true. Plan 2 left this switch off until pickup routes existed. Update the README "Before going live" section to drop that line.

- [ ] **Step 1: Write the failing tests.**
  - `ship_deadline_refunds_unready_self_order`
  - `courier_ready_before_deadline_not_struck`
  - `courier_pickup_overdue_flagged_once`
  - `ship_deadline_on_by_default_after_migration`
- [ ] **Step 2: Run them and confirm they fail.** Run `cargo test --test jobs ship_deadline`.
- [ ] **Step 3: Implement the migration and the claim change.**
- [ ] **Step 4: Run the full suite.** Run `cargo test`. Expected: all pass.
- [ ] **Step 5: Commit.** `feat: enforce ship deadlines now that pickup is tracked`

### Task 13: End-to-end and docs

**Files:**
- Modify: `server/tests/escrow.rs` (two end-to-end tests), `README.md`, `docs/superpowers/plans/2026-10-23-00-roadmap.md`, `docs/superpowers/specs/2026-10-22-abuja-marketplace-design.md` (§6 with the calls marked "My call" in this plan)

- [ ] **Step 1: Write `self_delivery_end_to_end`.** It runs through the HTTP routes:
  1. Create an address.
  2. Fetch delivery options.
  3. Check out with self-delivery.
  4. The payment webhook arrives.
  5. The seller marks picked up, then out for delivery. The buyer sees the code; the seller sees the contact.
  6. The seller marks delivered with the code. The payout is created.

  Assert the payout of exactly items − cut − payout fee − stamp duty + the self fee, the ledger sums, escrow 0, and that the contact is no longer visible.
- [ ] **Step 2: Write `courier_end_to_end`.** It uses `FakeCourier`:
  1. Check out with a courier option.
  2. Payment arrives.
  3. The seller marks the order ready.
  4. The job books the courier.
  5. Webhooks arrive: `picked_up`, `in_transit`, `completed`.
  6. Back-date `release_at`, then run the jobs. The payout is created without the delivery fee, and the fee goes to `platform_revenue`.
- [ ] **Step 3: Run the full suite, clippy and fmt from `server/` with `market-db` up.** Run `cargo test`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`. Expected: all pass and clean.
- [ ] **Step 4: Update the docs.**
  - **README:** add a part 3 summary. Under "Before going live", add:
    - confirm the Shipbubble amount unit, the webhook signing key per environment, and the insufficient-wallet behaviour;
    - fund the wallet;
    - the NIPOST adapter waits on API access.
  - **Roadmap:** part 3 built, plus the carry items:
    - the NIPOST adapter;
    - re-quoting expired courier quotes and the owner re-book action (plan 6);
    - courier `returned` and failed-attempt statuses (unknown);
    - the Terminal Africa fallback;
    - a map while out for delivery, only if the courier provides location;
    - the push notifications and reminders (plan 5).
- [ ] **Step 5: Commit.** `test: add delivery end-to-end paths and document part 3`

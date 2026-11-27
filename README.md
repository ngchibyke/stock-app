# Stock App

A mobile marketplace for Abuja. Individuals open up to two stores inside one shared market, and anyone can sign in to buy from them. Buyers pay in the app; the money is held by the payment provider and released to the seller after delivery is confirmed.

This repository holds the backend API. The Flutter phone app and the owner dashboard come in later parts of the build.

## What works today

The server foundation (part 1 of 8):

- **Accounts:** sign up with a Nigerian phone number and password, confirmed by an SMS code. Sign-in sessions last 30 days and can be revoked. Roles: user, support, owner.
- **Access codes:** the owner issues codes (single-use or shared, optional expiry and usage cap, revocable). Redeeming one turns a user into a seller.
- **Stores:** up to 2 per seller, Abuja (FCT) only, with an encrypted pickup address. Stores stay hidden from the public until payouts are verified (part 2).
- **Products:** photos, condition, brand, and variants such as size and colour, each with its own stock and optional price.
- **Data protection:** phone numbers and addresses are encrypted at rest, lookups use a keyed hash, personal data is never logged, and an append-only audit log records sensitive actions.

Payments and escrow (part 2):

- **Paying:** a cart becomes one checkout paid through **Paystack** Checkout or Mono DirectPay (behind a provider layer; the Owner picks the method and the payout provider in Settings), split into one order per store. Stock is reserved while the buyer pays.
- **Held funds and ledger:** every money movement is a balanced entry in an append-only, double-entry ledger, and the platform takes a 5% cut.
- **Payouts:** sellers register a payout account, which is verified and stored encrypted. Funds are released to the seller (Paystack Transfers or Mono Disburse) after the buyer confirms, or 3 days after delivery.
- **Refunds and cancellations:** refunds are retried by a background sweeper.
- **Background jobs:** expiry, lost-webhook re-verification, auto-release, the 30-day backstop queue and payout polling.
- **Owner admin:** release, refund and hold orders; mark payouts paid or refunds refunded by hand; re-queue reversed payouts. A reconciliation list shows everything that needs attention. Every staff read and override is audited.

Delivery and tracking (part 3):

- **Addresses and coverage:** buyers save delivery addresses and pick one of the six FCT area councils; checkout works only inside the FCT ("Coming to your city soon" elsewhere). The NIPOST postcode is optional and format-checked when given until the NIPOST adapter exists.
- **Delivery options:** at checkout each store offers its own delivery at a flat fee and/or up to three Shipbubble courier rates. Quotes are held on the server, single-use, bound to the cart and address, and expire after 30 minutes. The courier fee goes to the platform (which pays Shipbubble); a self-delivery fee goes to the seller. The 5% cut is on items only.
- **Next-day promise:** a 6pm (owner-configurable) cut-off, skipping Sundays and bank holidays, fixed at payment.
- **Self-delivery:** the seller marks picked up, then out for delivery. The buyer sees a 6-digit handover code only then; the seller sees the buyer's phone and address only then, and each reveal is audited. The seller marks delivered with the code, which counts as the buyer's confirmation and releases the money. Five wrong codes lock entry for an hour.
- **Courier delivery:** the seller marks the order ready and a job books the courier. Signed courier webhooks (with a polling fallback) move the order forward only. A `completed` webhook starts the 3-day release window, counted from when we received it, and never releases money itself. A refund after booking still goes ahead and cancels the shipment afterwards.
- **Tracking timeline:** one endpoint gives the buyer and seller the steps, promised date, courier or store contact and tracking link, with delay notes.
- **Ship deadline:** the 48-hour rule is now enforced for orders paid after `ship_deadline_enforce_from`, and overdue pickups are flagged for the owner.
- **Owner view:** the reconciliation list shows bookings that failed or are unknown, courier cancels after pickup, overdue pickups and paid orders without a delivery method.

Operating the server (part 7g follow-ups):

- **`GET /health`** says the process is up. **`GET /ready`** says whether it can do its work: it answers 503 when the database is unreachable or a background job has stopped or keeps failing, naming only the jobs. Point an uptime monitor at `/ready`; `docs/release/operations.md` has the runbook.
- **`GET /account-deletion`** is a public page, served by the API itself, where a person who cannot open the app signs in and asks for deletion or keeps the account (both app stores ask for such a link). Give the stores `https://<api host>/account-deletion`. If the dashboard is on its own host, it only needs `/auth`, `/me` and `/admin` sent to the API.
- **`APP_STORE_URL_ANDROID` / `APP_STORE_URL_IOS`** name the store pages the app's "Please update" notice opens.

Paystack (added 2026-11-27): `server/src/payments/paystack.rs` is a second payment provider: hosted Checkout, Transfers for seller payouts, refunds, a bank-name lookup and signed webhooks (`POST /webhooks/paystack`). Set `PAYSTACK_SECRET_KEY` (`sk_test_` outside production, `sk_live_` in it), `PAYSTACK_PAYER_EMAIL_DOMAIN` and, in Settings, the method `paystack_checkout` and the payout provider `paystack`. **Tested only against a mock of Paystack's documented API, never a real account**; the plan `docs/superpowers/plans/2026-11-27-paystack-adapter.md` lists what is unconfirmed, and `docs/release/go-live-costs-and-placeholders.md` has the setup steps.

Partner services (part 9):

- **A seam, not a feature.** One gateway (`server/src/partners/`) is the only door to a service built by the owner's other company. No capability exists yet, because the partner's contract does not. With no `PARTNER_*` settings, or the switch off, every call answers "not configured" or "disabled" without a network call, and nothing else in the server changes. A partner is optional, even in production.
- **Safety.** Https only, no redirects, 5 s connect and 10 s total timeouts, responses capped at 1 MiB, one retry for a `GET` and never for a `POST`, and a circuit breaker (open after 5 failures in a row, one test call after 30 s). Only the fields in `partners::client::ALLOWED_OUTBOUND_FIELDS` can be sent (opaque ids, barcodes, kobo, timestamps); a payload with any other field is refused before a network call. The key and the webhook secret are never logged or shown.
- **Paid for.** Every call is counted per feature and day, and the Owner can set a monthly call budget. When it is spent calls stop and one alert a month is raised in the dashboard's alerts.
- **Switches.** The Owner turns the partner on and off and names features, with a note for every change (audited), from the Partner card on the dashboard's Settings page, or `GET /admin/partner/status` and `PATCH /admin/partner`. `GET /app/config` carries `features`: a name is true only while the partner is configured, switched on, within budget and the breaker is closed.
- **Webhooks.** `POST /webhooks/partner`, signed with HMAC-SHA256 over `{timestamp}.{raw body}` (`X-Partner-Timestamp` in unix seconds, `X-Partner-Signature` in hex), refused when the timestamp is more than 5 minutes off or the body is over 64 KiB, each `event_id` stored once.

Chat, moderation and reviews (part 4):

- **Chat:** a buyer and seller talk about a product or an order. Messages are sent over REST and pushed over a WebSocket; a client that missed a push reads from its last sequence number. Bodies are encrypted at rest.
- **Moderation before delivery:** a word filter, scam and contact-detail patterns, then a Claude Haiku tone check. Serious text is blocked with a reason and a strike; mild text comes back with a suggested rewrite the sender can send in one tap. The original is never delivered.
- **Outages:** if the tone check is down a message is held, then delivered in order once it recovers. After 6 hours it is withheld with no strike, and the sender is told. It is also withheld if the chat closed or the sender was muted or banned while it waited.
- **Strikes:** a warning, then a mute after 3 strikes in 30 days, then owner review after 5. All are owner-configurable, and a mute never touches orders or money.
- **Reviews:** one per completed delivery, with stars, optional text and photos, moderated like chat, and one seller reply. Store pages show the rating; the public list never shows who the buyer is.
- **Moderation desk:** staff list flags as metadata, resolve them and hide reviews. Only the owner reads a flagged original, always audited, and only the owner unmutes or bans. A ban stops posting only.

Disputes and notifications (part 5):

- **Reporting a problem:** a buyer can report an order that has shipped but not been paid out, with a reason, a statement and up to five photos. The order's money is frozen at once. One report per order, and withdrawing it is final.
- **The seller's side:** the seller has 48 hours to answer once. After that the report moves on without them. Staff can ask for the item to be returned first and record who pays for the return.
- **The decision:** the owner settles it with a full refund, a partial refund or a release. A partial refund comes out of the item total; the rest is released to the seller with the 5% cut taken on what they keep. While a report is open, the plain release, refund and unhold routes refuse the order, so every decision goes through the report and into the audit log.
- **The dispute screen (server side):** order, payment, ledger, delivery timeline and events, handover-code status, both statements and the order chat. Only the owner sees flagged originals. Support sees open disputes only, and nobody sees the buyer's phone or address there. Every read is audited.
- **Inbox:** every event (paid, picked up, out for delivery, delivered, refund started, payment released, new message, report opened, return requested, report decided, moderation warning, mute) writes an inbox row in the same transaction as the change. The inbox stores no text: titles and bodies are fixed copy.
- **Reminders:** a "did your order arrive?" notice a day after delivery and a "payment releases tomorrow" notice a day before release. A missed reminder is skipped, never sent late.
- **Push:** notices go out through Firebase Cloud Messaging with the fixed copy and two ids, and nothing else: no order, name, amount or message text reaches a lock screen. A signed-out session gets no push, a dead token is disabled, and an outage is retried with backoff without re-sending to devices that already got it.
- **Flagged payments:** a buyer can no longer release an order whose payment the provider flagged.

App support (part 7a):

- **Catalog:** a fixed list of ten categories (`GET /categories`), public browse and search over published products (`GET /products`: text search that forgives typos, category, condition and price filters, sorting, stable paging), and a store directory (`GET /stores`). Products from a suspended or unpublished store never appear.
- **Photos:** a seller or buyer asks for an upload slot (`POST /uploads`), the phone `PUT`s the file straight to object storage with the signed link, then finalizes it (`POST /uploads/{id}/finalize`). Finalizing makes the server re-encode the photo as a JPEG at most 2048 px on a side, which drops EXIF and GPS data, and refuses anything that is not a real image, is over 5 MiB, or would decode to more than 40 million pixels. The finished key is then attached to a product, store, review or dispute, and the server checks that the key is the caller's own finished upload of the right kind and is not already used elsewhere. Public photos are served from a public bucket by address; dispute evidence lives in a private bucket and is reached only through `GET /disputes/{id}/photo?key=...`, which hands the buyer, the seller or staff a link that works for 60 seconds.
- **Password reset:** by SMS code. The request answers the same way for every phone, so it cannot be used to find out who has an account. Confirming sets the new password, ends every session and clears the sign-in lockout.
- **SMS codes:** sign-up and reset codes go out through an SMS provider behind a small trait (fixed message text, never the phone or code in a log). A throttled request says so (`429 otp_throttled`, with `retry_after_secs`) and never hands back a verification id for a code that was not sent. The provider is Termii (`SMS_PROVIDER=termii`), on its do-not-disturb route so codes reach numbers that opted out of promotions.
- **App config:** `GET /app/config` tells the app the minimum version per platform, the terms and privacy links, the support address and any maintenance notice (settings `APP_*`).

## Owner dashboard

`dashboard/` is the owner and support web app (React, Vite, TypeScript; every screen is animated with the `motion` library and respects the system's reduced-motion setting). It is a static build that talks to this API with the normal bearer session. Owner sees everything; Support sees disputes and moderation only.

```bash
cd dashboard
npm install
npm run dev      # http://localhost:5173, proxies /auth, /me and /admin to the API on :8080
npm test         # unit and component tests
npm run build    # static files in dashboard/dist
npm run e2e      # browser tests against the real server (needs Postgres; see below)
```

**Design kit.** The dashboard's pieces live in `dashboard/src/ui/`: interactive icons (each with its own hover, press and selected motion), buttons, a dialog and confirmation with a note, a drawer, a tooltip, an action menu, a command palette (Ctrl or ⌘ + K), banners, checkboxes, tabs, badges, avatars, and skeleton loaders shaped like the screens they stand in for (tables, stat grids, detail panels, forms, the app frame). Dialogs, drawers, menus and the palette share one focus rule: Esc closes, Tab stays inside, the page behind does not scroll, and focus returns to where it was. The shell works on phones (a top bar and a sliding menu) and keeps a slim rail on tablets. `npm run dev` also serves `/kit`, a page that shows every piece side by side in light and dark; production builds do not include it.

**Hosting.** Serve `dashboard/dist` and the API from **one origin** behind a reverse proxy (the server has no CORS): send `/auth`, `/me` and `/admin` to the API and everything else to the static files, with `index.html` as the fallback for unknown paths. Add a strict `Content-Security-Policy`, for example `default-src 'self'; style-src 'self' https://fonts.googleapis.com 'unsafe-inline'; font-src https://fonts.gstatic.com; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'`, and serve it over HTTPS only. The session token lives in `sessionStorage` and in memory, and the app never renders statement or chat text as HTML.

**Browser tests.** `npm run e2e` builds and starts the real server on port 8099 against `E2E_DATABASE_URL` (default `postgres://market:market@localhost:5432/market`), creates a test Owner, and drives the built app in Chromium. No payment, courier or chat provider is configured, so these cover sign-in, access codes, staff, settings, users and the audit log. The dispute and money screens are covered by component tests and by the server's own suite. Set `E2E_CHROMIUM` to use an existing Chromium, otherwise run `npx playwright install chromium` once.

## Roadmap

| Part | What it adds |
|---|---|
| 1 | Server foundation (done) |
| 2 | Payments and escrow (Mono first, behind a switchable provider layer): cart, orders, held funds, release, refunds, 5% platform cut (done) |
| 3 | Delivery and tracking: FCT addresses, Shipbubble couriers, self-delivery, handover codes, tracking timeline (done) |
| 4 | Moderated chat and reviews (done) |
| 5 | Disputes and notifications (done) |
| 6 | Owner web dashboard (done) |
| 7 | Flutter phone app (Android and iPhone). 7a, the server additions the app needs, is done; 7b to 7e are not written |
| 8 | Data export, deletion and retention |

The design is in [docs/superpowers/specs/2026-10-22-abuja-marketplace-design.md](docs/superpowers/specs/2026-10-22-abuja-marketplace-design.md). The build plan and the items each later part must handle are in [docs/superpowers/plans/2026-10-23-00-roadmap.md](docs/superpowers/plans/2026-10-23-00-roadmap.md).

## Running it locally

You need Rust (stable) and Docker.

1. Start the database (Postgres 17 on `localhost:5433`):

   ```bash
   docker compose up -d db
   ```

2. Copy `server/.env.example` to `server/.env` and fill in `FIELD_KEY` and `INDEX_KEY` with two different random 32-byte keys, base64-encoded:

   ```bash
   openssl rand -base64 32
   ```

3. Start the API (it listens on `127.0.0.1:8080`):

   ```bash
   cd server && cargo run -- serve
   ```

4. Create the owner account (you'll be asked for the password):

   ```bash
   cd server && cargo run -- create-owner --phone 0803XXXXXXX --name "Your Name"
   ```

### The phone app

The Android and iOS app lives in `app/` (Flutter). Today it does sign-up with a phone code, sign-in, password recovery, an account page and an address book. The market itself (browsing, stores, buying and selling) is not built yet.

**The app has NOT been run on a real device or emulator** (none exists on the build machine and the Android licences are not accepted there). The unit and widget tests and the real-server wire check (`app/test/wire`) pass. A first device run should check sign-up with a real SMS code, keyboard and autofill behaviour, the code boxes with the system one-time-code suggestion, text at 200 percent, and the shell and account page.

Install Flutter stable 3.50.1, then from `app/` run `flutter pub get`, `dart run build_runner build --delete-conflicting-outputs`, `flutter analyze` and `flutter test`. See [app/README.md](app/README.md) for how to run it, the wire check, and the list of things to change before the first store upload (placeholder app id, release signing, store link).

### Trying photo uploads by hand (optional)

Uploads need an S3-compatible store. For local work `docker-compose.yml` has an optional MinIO with a public and a private bucket:

```bash
docker compose --profile storage up -d minio minio-setup
```

MinIO no longer publishes images on Docker Hub, so the compose file uses whatever `minio/minio:latest` you already have locally (or the image named by `MINIO_IMAGE`). Check where an image comes from before you run it. Then put the local values from `server/.env.example` into `server/.env` (plain http is accepted only for localhost, and only when `APP_ENV` is development or test). The credentials in the compose file are public and local only.

## Tests

The tests run against the local database:

```bash
cd server && DATABASE_URL=postgres://market:market@localhost:5433/market cargo test
```

One test talks to a real MinIO and is skipped by default; with the storage profile above running, `cargo test --test s3_presign -- --ignored against_a_local_minio` checks that a real store accepts the server's signatures, refuses a wrong content type, and keeps the private bucket private.

Every push and pull request runs format, lint, the full test suite against Postgres, and a dependency security audit (`.github/workflows/ci.yml`).

## Before going live

These are tracked in the roadmap:

- Rate limits per network address on sign-up and sign-in.
- A restricted production database role.
- Legal review of the escrow setup, and data-protection (DPO) review.
- The ship-deadline job is on (migration 0030) and only refunds or strikes orders paid at or after `platform_settings.ship_deadline_enforce_from`, which that migration sets to the moment it ran. After deploying, check the owner's reconciliation view: `orders_attention` lists paid orders with no delivery method (the job never touches them), and `shipments_attention` lists ready courier orders whose pickup is overdue.
- Confirm with Mono which unit (kobo or naira) each product's `amount` fields use (`MONO_DIRECTPAY_AMOUNT_UNIT`, `MONO_DISBURSE_AMOUNT_UNIT`), then run a sandbox smoke test of a payment, a payout and a webhook before switching to live keys.
- DPO review of whether the Owner should keep seeing buyer display names on order reads. Support already does not, and every staff read is audited.
- Do not switch courier delivery on for real stores until the sandbox answers below are in. The owner re-book path now exists (plan 6), but a stale quote still needs a re-quote, which waits on those answers. Today an empty Shipbubble wallet can fail ready orders for good: they stay paid with the buyer's money held until someone acts.
- Run a Shipbubble sandbox smoke test before live keys: the amount unit (`SHIPBUBBLE_AMOUNT_UNIT`, naira or kobo), the webhook signing key per environment and the exact bytes signed, address validation, the status field on `shipment.cancelled`, whether a used `request_token` can book again, what a 429 means, and what an insufficient wallet returns (it may look like any other 400).
- Two more sandbox checks are a gate on courier go-live: book a token whose `pickup_date` passed 1 and 2 days ago (the seller has 48 hours to mark ready, and booking replays the token from the quote; if it is refused, quote again at booking or quote for the end of the ready window), and find out whether a cancelled label credits the wallet. The ledger books every courier charge to `courier_costs` and posts no credit on a cancel until this answer is in; if the wallet is credited, post the reverse under `("courier_refund", shipment_id)`.
- Set `SHIPBUBBLE_CONTACT_EMAIL` (a platform operations address, sent when a contact has no email) and fund the Shipbubble wallet. The server refuses an `sb_prod` key outside production and a production start without the amount unit.
- The NIPOST postcode adapter waits on API access; until then the postcode is only format-checked.
- Accountant: is delivery vatable at 7.5%, and who is merchant of record for courier delivery.
- DPO and legal review: the Shipbubble processor agreement and data location; a privacy notice that names couriers; the seller seeing the buyer's address while out for delivery; retention of `orders.delivery_address_enc` (the buyer's delivery contact, kept today on every order, including never-paid and refunded ones), `courier_phone_enc`, `delivery_events`, soft-deleted addresses and audit rows; the old business phone that can appear on labels for orders quoted before a phone change; the buyer's name, phone and address going to Shipbubble when options are requested, before the buyer chooses courier or pays; plaintext `buyer_addresses.postcode`, which may locate a single property; and plaintext tracking URLs, which may be bearer links showing the address.
- Owner decisions: whether a handover code should release after a delay or have a no-code fallback, and whether the 48-hour ship deadline should pause on holidays and Sundays.
- Anthropic's data terms: confirm in writing that chat text sent to `claude-haiku-5-5` is not used for training, and arrange zero data retention, which spec section 7 requires. That is an account arrangement, not a code setting.
- DPO and legal: Anthropic as a processor, the cross-border transfer under the NDPA, and the privacy notice covering automated checks on chats.
- DPO: strikes and mutes are automated decisions about a person. The server gives a reason and a human route (dismiss, unmute), but whether that is sufficient under the NDPA needs review.
- Native-speaker review of the word lists and the eval fixture, and a run of the language eval (`server/tests/tone_eval.rs`), with the owner choosing the accuracy bar before launch.
- Photos are re-encoded but not looked at: nothing detects a phone number written on a product or review photo, or an inappropriate image. Decide before launch whether to add a moderation step. Ownership is checked: a photo key must be the caller's own finished upload.
- Owner decisions: whether users can report a message that got through (not in the spec, so not built); whether chat should close some time after an order ends; whether sellers may see buyer names.
- DPO and legal: Google (Firebase Cloud Messaging) as a processor of push tokens, and the cross-border transfer under the NDPA; the privacy notice naming push; and whether generic lock-screen copy is enough, or users should be able to opt out per kind of notice.
- DPO: retention of dispute statements and evidence photos (which may hold personal data of third parties), inbox rows and push devices; Support's access to statements inside open disputes; and staff free-text notes, which are kept in the audit log for every admin action, now including return requests and dispute decisions.
- Evidence photos on a dispute are not looked at and can carry phone numbers, as in reviews. Ownership of the key is checked (part 7a).
- Owner decisions on disputes: whether the seller's 48 hours should pause on Sundays and holidays (today they are calendar hours, like the ship deadline); whether a withdrawn report should ever be reopenable; and whether a dispute on a courier order still in transit should cancel the courier.
- The daily statement check (plan 6) has no Mono adapter: confirm whether Mono offers a statement API, and its shape and units, then write `StatementSource` for it and set the tolerance (`server/src/statement`). Until then production installs no source and the job does nothing.
- Set `FCM_SERVICE_ACCOUNT_FILE` to the Firebase service-account file, kept outside the repository. Production refuses to start without it.
- DPO and legal (part 7a), for review and not as conclusions: the object store as a processor of photos, its region and any cross-border transfer; retention of uploads, and a purge on account erasure (part 8); photos that may show faces or other people; and whether evidence photos need access logging beyond staff reads.
- Termii (the SMS provider) as a processor of phone numbers. Before going live: get the sender ID approved (the company that registers it should be registered in Nigeria), ask Termii support to switch on the do-not-disturb route for the account (the server sends every code on it), confirm the per-message price, and run a sandbox test: a real code to a number on the do-not-disturb list, and what Termii answers to a bad number, an empty balance and a bad key (Termii does not document its error bodies, so the adapter sorts failures by HTTP status). Production refuses to start without `SMS_PROVIDER`.
- The production database role needs permission to create the `pg_trgm` extension (catalog search), or the database owner creates it before the first deploy.
- Set the photo storage variables (`S3_*`, `UPLOADS_PUBLIC_BASE_URL`), kept outside the repository: production refuses to start without them. Also set `SMS_PROVIDER=termii`, `TERMII_API_KEY` (a secret) and `TERMII_SENDER_ID`, and the `APP_*` settings (`APP_MIN_VERSION_ANDROID`, `APP_MIN_VERSION_IOS`, `APP_TERMS_URL`, `APP_PRIVACY_URL`, `APP_SUPPORT_EMAIL`, `APP_MAINTENANCE_MESSAGE`).

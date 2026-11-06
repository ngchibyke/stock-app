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

- **Paying:** a cart becomes one checkout paid through Mono DirectPay (behind a provider layer that can be switched), split into one order per store. Stock is reserved while the buyer pays.
- **Held funds and ledger:** every money movement is a balanced entry in an append-only, double-entry ledger, and the platform takes a 5% cut.
- **Payouts:** sellers register a payout account, which is verified and stored encrypted. Funds are released to the seller through Mono Disburse after the buyer confirms, or 3 days after delivery.
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

Chat, moderation and reviews (part 4):

- **Chat:** a buyer and seller talk about a product or an order. Messages are sent over REST and pushed over a WebSocket; a client that missed a push reads from its last sequence number. Bodies are encrypted at rest.
- **Moderation before delivery:** a word filter, scam and contact-detail patterns, then a Claude Haiku tone check. Serious text is blocked with a reason and a strike; mild text comes back with a suggested rewrite the sender can send in one tap. The original is never delivered.
- **Outages:** if the tone check is down a message is held, then delivered in order once it recovers. After 6 hours it is withheld with no strike, and the sender is told. It is also withheld if the chat closed or the sender was muted or banned while it waited.
- **Strikes:** a warning, then a mute after 3 strikes in 30 days, then owner review after 5. All are owner-configurable, and a mute never touches orders or money.
- **Reviews:** one per completed delivery, with stars, optional text and photos, moderated like chat, and one seller reply. Store pages show the rating; the public list never shows who the buyer is.
- **Moderation desk:** staff list flags as metadata, resolve them and hide reviews. Only the owner reads a flagged original, always audited, and only the owner unmutes or bans. A ban stops posting only.

## Roadmap

| Part | What it adds |
|---|---|
| 1 | Server foundation (done) |
| 2 | Payments and escrow (Mono first, behind a switchable provider layer): cart, orders, held funds, release, refunds, 5% platform cut (done) |
| 3 | Delivery and tracking: FCT addresses, Shipbubble couriers, self-delivery, handover codes, tracking timeline (done) |
| 4 | Moderated chat and reviews (done) |
| 5 | Disputes and notifications |
| 6 | Owner web dashboard |
| 7 | Flutter phone app (Android and iPhone) |
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

## Tests

The tests run against the local database:

```bash
cd server && DATABASE_URL=postgres://market:market@localhost:5433/market cargo test
```

Every push and pull request runs format, lint, the full test suite against Postgres, and a dependency security audit (`.github/workflows/ci.yml`).

## Before going live

These are tracked in the roadmap:

- Rate limits per network address on sign-up and sign-in.
- A restricted production database role.
- Legal review of the escrow setup, and data-protection (DPO) review.
- The ship-deadline job is on (migration 0030) and only refunds or strikes orders paid at or after `platform_settings.ship_deadline_enforce_from`, which that migration sets to the moment it ran. After deploying, check the owner's reconciliation view: `orders_attention` lists paid orders with no delivery method (the job never touches them), and `shipments_attention` lists ready courier orders whose pickup is overdue.
- Confirm with Mono which unit (kobo or naira) each product's `amount` fields use (`MONO_DIRECTPAY_AMOUNT_UNIT`, `MONO_DISBURSE_AMOUNT_UNIT`), then run a sandbox smoke test of a payment, a payout and a webhook before switching to live keys.
- DPO review of whether the Owner should keep seeing buyer display names on order reads. Support already does not, and every staff read is audited.
- Do not switch courier delivery on for real stores until an owner re-book path exists (plan 6). Today an empty Shipbubble wallet can fail ready orders for good: they stay paid with the buyer's money held until someone acts.
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
- Photos in reviews are not checked and can carry phone numbers. Decide before launch. The upload plan must also check that a key belongs to the uploader.
- Owner decisions: whether users can report a message that got through (not in the spec, so not built); whether chat should close some time after an order ends; whether sellers may see buyer names.

# Marketplace App — Design

Date: 2026-10-22
Status: Approved by owner 2026-10-23. No fixed launch date; build the full version as designed (no reduced first release).
Working name: "the Market" (no product name chosen yet)

## 1. Intent

A mobile marketplace where individuals — mainly business owners and marketers — run their own stores inside one shared market, and the general public signs in to buy from them. Launch market is **Abuja (FCT)**, chosen to win one city first.

What makes it different from "just another shop app":

- **Buyer protection by default.** Buyers pay inside the app; the money is held by a licensed payment provider and only released to the seller after delivery is confirmed (or a short window passes with no complaint).
- **Fast, trackable delivery.** Next-day delivery within Abuja, with a clear, honest tracking screen.
- **Curated sellers.** Opening a store requires an access code issued by the owner.
- **Clean, respectful conversations.** All buyer–seller chat is moderated for abuse, tone and scam attempts.

### Success looks like

- Abuja buyers trust it enough to pay up-front to sellers they've never met.
- Most orders placed before the daily cut-off arrive the next day.
- Sellers get paid within about 4–5 days of a sale.
- Disputes are rare and the owner can settle each one from a single screen.

### Anti-goals (would be failure even if it "works")

- A UI that looks templated or "vibe coded". Tracking and checkout especially must feel deliberate and polished.
- Any path where a buyer is pushed to pay a seller outside the app.
- Card numbers, BVN/NIN or bank credentials stored by our system.
- Personal data (addresses, phone numbers, chats) visible to anyone who doesn't need it.

## 2. People and roles

| Role | Who | Can do |
|---|---|---|
| **Buyer** | Anyone who signs up | Browse, chat with sellers, buy, track, confirm delivery, complain, review |
| **Seller** | A user who redeemed an access code | Everything a buyer can, plus own **up to 2 stores** |
| **Owner/Admin** | The business running the market | Web dashboard: codes, users, stores, orders, disputes, flagged chats, platform cut |

One account type; "seller" is an unlock on a normal account, not a separate login.

## 3. Accounts and access

- **Sign up / sign in** in the mobile app (email or phone + password; phone verified by OTP). *(builder's call on exact auth method; passwords hashed with Argon2, never logged)*
- **Shopping is open**: any signed-up user can browse and buy.
- **Selling needs an access code** issued by the owner from the dashboard.
  - Codes can be single-use or multi-use with an optional expiry and usage cap. *(my call — covers both "one code per person" and "shared code" options)*
  - Owner can revoke a code; already-created stores are unaffected.
- **Store limit: 2 per user**, enforced on the server.

## 4. Stores and products

**Store:** name, logo, banner, description, Abuja pickup address (with NIPOST postcode), delivery method (courier / self-delivery / both), bank account for payouts.

**Before a store can accept orders:**
1. Access code redeemed.
2. Payout bank account verified through the payment provider (account-name match).
3. Optional extra identity check (BVN/NIN) **done through the provider** — our system stores only the pass/fail result and provider reference, never the number.

**Product ("everything a buyer needs to trust it"):**
name, price (₦), multiple photos, description, category, brand, condition (new / used / refurbished), variants (e.g. size, colour) each with its own stock count and optional price, weight/size (needed for courier rates), store info.

Stock is decremented at payment and restored on refund/cancellation.

## 5. Payments (escrow via provider)

**Providers (owner decision, 2026-10-23):** payments sit behind a small **payment-provider layer**, so the escrow logic never depends on which company moves the money.
- **Launch (Abuja pilot): Mono.** Buyers pay from their bank account with Mono **DirectPay**; sellers are paid with Mono **Disburse**.
- **Before growing beyond the pilot: add Paystack** as a second checkout option (card, transfer, USSD). Payouts stay on Mono unless the owner switches them.
- **Switching is a setting, not a code change:** the owner dashboard turns each payment method on or off and chooses the payout provider. An order always finishes with the provider it was paid through.
- Card and bank-login details never touch our servers.

**Flow:**
1. Buyer pays in-app → money is collected for the platform (with Mono it settles to the platform's business account the next working day).
2. Seller notified: "Paid — please ship." Seller sees the order, **not** the money.
3. Delivery happens (section 6).
4. **Release** to the seller's verified bank account via provider payout, minus the platform cut, when either:
   - buyer confirms (handover code at the door, or "Received" button), **or**
   - **3 days after the order is marked delivered** with no complaint.
5. **Backstop:** if an order is never marked delivered, nothing auto-releases. At **30 days from payment** it is surfaced to the owner for a decision; buyer may request a refund.
6. **Complaint** before release → funds frozen until the owner decides (section 8).

**Reminders to confirm:** when marked delivered, next day, and a final "payment will be released tomorrow" notice.

**Platform cut:** **5%** of the item total (owner decision), adjustable in the dashboard, applied at release.

**Who pays what** *(my recommendation, owner set the 5% cut)*:
- **Buyer** pays the delivery fee, added at checkout (courier rate from Shipbubble, or the seller's own flat fee for self-delivery). The delivery portion is collected by the platform; for courier orders it pays Shipbubble from a funded platform wallet.
- **Seller** pays the payment-provider collection fee and the payout transfer fee, deducted from the payout.
- **Seller payout = item total − 5% cut − provider fees.** The payout screen shows this breakdown.
- **Refunds:** full refund returns item total + delivery to the buyer; the platform waives its cut on refunded orders. Provider fees that can't be recovered are charged to the side at fault, as decided by the owner.

**Legal flag:** holding funds for third parties is regulated by the CBN. With Mono, held funds sit in the platform's own business account until released, which is the arrangement most in need of review. **A Nigerian lawyer/compliance adviser must confirm the setup before launch** (for example a dedicated escrow/trust account or a licensed escrow partner).

All money movements are written to an **append-only ledger** (payment, hold, release, refund, cut) for audit.

**Multi-store cart** *(my call)*: a cart may hold items from several stores. Checkout is one payment, split into **one order per store**. Each order has its own delivery, handover code, release clock, dispute and refund.

**Cancellations and non-shipment** *(my call)*:
- Buyer can cancel for a full refund until the order is picked up.
- Seller must hand over / mark picked up within **48 hours** of payment, or the order is auto-cancelled and refunded, and the seller gets a strike.
- In a dispute, the owner decides whether the item must be returned first; return shipping is paid by whichever side is at fault.

## 6. Delivery

**Coverage at launch:** stores and delivery addresses must be inside the FCT. Buyers elsewhere can browse but not check out ("Coming to your city soon").

**Addresses:** the buyer saves an address and chooses one of the six FCT **area councils** (AMAC, Bwari, Gwagwalada, Kuje, Kwali, Abaji); that choice, never the word "Abuja" in free text, decides whether the address is inside the FCT (Suleja, Mararaba, Karu and Masaka are not). The **NIPOST 11-character digital postcode** (live nationwide since 1 Oct 2026, coverage still incomplete) is **optional**: when given it is normalised (spaces and `-` stripped, upper-cased) and must match `FC`, two digits, then 7 letters or digits. It is meant to be checked through the NIPOST postcode API (`api.postcode.gov.ng`), behind an interface with a manual fallback; until the adapter exists it is format-checked only. Free basic lookup/validity; deeper levels need KYB and credits. **Owner must request developer access** (g2g@postcode.gov.ng / "Integrate with us" on postcode.gov.ng). Official API docs were not found publicly, so the NIPOST adapter is not built yet and the manual fallback is what runs.

**Delivery methods (chosen per store):**
- **Courier via Shipbubble** — one integration covering 20+ Nigerian couriers (GIG, Kwik, etc.): rates at checkout, label/pickup booking, status updates via signed webhooks (HMAC-SHA512, `x-ship-signature`) with a polling fallback. Terminal Africa is the fallback aggregator.
- **Self-delivery** — seller taps *Picked up → Out for delivery → Delivered*.

**Delivery options at checkout** *(my call)*: the server quotes and owns every amount; the app only sends back the option the buyer chose.
- A store offers self-delivery (its own flat fee) and/or courier (up to the 3 cheapest Shipbubble rates, priced at what Shipbubble will charge). One platform-wide parcel category is used until products map to courier categories; parcel size is the largest length and width and the summed height, with 10 cm for any missing dimension.
- Each option is **single-use** and **bound to the buyer, store, address and the exact cart** (a fingerprint of its variants and quantities), and expires after 30 minutes. If the cart or address changes, or a live or paid order already used the option, checkout refuses it and the buyer asks for fresh options. Fresh quotes are capped per buyer per hour.
- The buyer pays items plus the delivery fee. The platform cut (5%) is on items only. A **self-delivery** fee goes to the seller at release; a **courier** fee goes to platform revenue, because the platform pays Shipbubble from its wallet.

**Courier booking** *(my call)*: the seller marks the order *ready*; a background job then books the courier from the quote stored at checkout.
- A quote 7 days old or more, or a store whose pickup location moved after the quote, fails the booking for the owner to re-quote (an owner action, plan 6), rather than booking a stale price.
- The courier has no idempotency key, so a booking that times out is **never retried automatically**: it is held as *unknown* for the owner to check. A booked price above the quote by more than a tolerance raises an attention item; the buyer is never charged twice.
- A refund after booking always goes ahead; the courier cancel is sent afterwards and flagged for the owner if the courier refuses it.
- Courier statuses only move forward (a late or repeated webhook cannot move an order back), and a cancellation after pickup is flagged for the owner, not applied.

**Handover code** *(my call)*: only **self-delivery** orders have one, because couriers do not use our app and nobody could check a code on a courier order. The 6-digit code is created when the seller marks the order **out for delivery** (not at payment, so it cannot be learned early), stored encrypted, shown to the buyer only while the order is out for delivery ("Give this only to the person handing you the parcel"), and cleared once used. The person delivering enters it on handover. Entering it = buyer confirmation (triggers release immediately). After 5 wrong entries entry is locked for 1 hour, and every attempt is audited without the code itself.

**What counts as "delivered"** *(my call, protects buyers)*:
- **Self-delivery:** the seller **cannot** mark Delivered without the buyer's handover code (or the buyer pressing "Received"). No code → no release clock.
- **Courier:** couriers won't use our app, so Shipbubble's "delivered" webhook starts the 3-day release window; the buyer can still complain within it. The window counts from when **our server received** the webhook, not from the time the courier states, so a back-dated status cannot shorten it. The webhook never releases money by itself; the release job does when the window ends.

**Phone numbers** *(my call)*: chat blocks phone numbers. A self-delivering seller sees the buyer's phone and full address only while the order is "Out for delivery" (before that, the area council only); both are hidden again once it is delivered, completed, cancelled or refunded, and each reveal is audited. For courier orders the seller never sees the buyer's phone or address; Shipbubble receives them for the label. The tracking call button calls the courier's phone (from Shipbubble) or the store's business line, not a personal number shared in chat.

**Next-day promise:** orders paid before a daily **cut-off (6pm Africa/Lagos, owner-configurable)** are promised for next-day delivery; after cut-off, the day after. Delivery days skip Sundays and the public holidays in the owner-editable holiday list *(my call: couriers do not run on those days)*. Shown honestly at checkout, and recomputed and fixed at payment.

**Ship deadline** *(my call)*: the 48 hours in section 5 are **calendar hours** from payment; Sundays and holidays do not pause them. An unshipped order is refunded and the seller gets a strike. For self-delivery, "shipped" means the seller marked it picked up; for courier, it means marked ready (the order is not struck if the booking then fails or the courier cancels; the owner resolves those). The rule applies only to orders paid on or after an **enforce-from** date (set when the job was switched on), so older orders are never swept up. Orders with no delivery method (from before delivery tracking) are excluded and listed for the owner.

### Tracking screen (must be polished, not templated)

Based on Baymard order-tracking research and established delivery-app patterns:
1. **Top:** expected delivery day + current status, large and unambiguous.
2. **Timeline:** vertical, timestamped steps (Paid → Seller preparing → Picked up → Out for delivery → Delivered → Confirmed).
3. **Courier/seller name + call button** — buyers never leave the app to track.
4. **Handover code** clearly visible on "Out for delivery" (self-delivery only).
5. **Map** only during "Out for delivery" (and only if the courier provides location) — not a constant, battery-heavy map.
6. Order details below.

## 7. Chat (moderated)

- Buyer ↔ seller chat, always tied to a product or an order.
- Every message is checked **before delivery**:
  - **Word filter** (instant) for vulgar/abusive words.
  - **AI tone check** (Claude Haiku, `claude-haiku-5-5`) for insults, threats, harassment, aggression without "bad words".
  - **Scam patterns**: phone numbers, bank account numbers, "pay me directly", moving to WhatsApp etc.
  - Must handle **English, Pidgin, Hausa, Yoruba, Igbo** and mixtures.
- **Outcomes:**
  - **Serious** (threats, slurs, sexual content, scam/off-platform contact) → **blocked**, sender warned with reason, strike recorded.
  - **Mild** (rude tone, mild swearing, shouting) → sender is shown a **suggested nicer version** with one-tap "Send suggested" or edit. The original is never delivered.
- **Strikes:** warning → temporary mute → owner review for ban. *(my guess on thresholds; builder sets defaults, owner-configurable)*
- Originals of flagged messages are kept, visible **only to the owner**, for disputes.
- Owner can read an order's chat when a dispute is open.

**Data protection:** chat is personal data processed by a third party (AI provider, possibly outside Nigeria). Requires: disclosure in the privacy notice, provider configured not to retain/train on data, only message text sent (no names/IDs), retention of **12 months** (longer while a dispute is open). **DPO/legal sign-off needed.**

**Decisions made while building (plan 4):**

- *(my call)* Messages are sent over REST and pushed over a WebSocket, which only pushes. Outcomes, idempotency and rate limits therefore live in one place, and a client that missed a push reads from its last sequence number.
- *(my call)* If the tone check is down, a message is held and retried with backoff, oldest first per sender and conversation. After 6 hours it is withheld with no strike and the sender is told. A held message is also withheld if the chat closed or the sender was muted or banned while it waited.
- *(my call)* Strike defaults, owner-configurable: a strike counts for 30 days, a 24-hour mute after 3, owner review after 5, and at most one strike per 10 minutes.
- *(my call)* A mute or posting ban stops posting only, never reading, orders or money. A review has no held state: during an outage nothing is stored and the buyer tries again.

## 8. Disputes

1. Buyer opens a complaint (reason + photos) before funds are released → funds frozen.
2. Seller has **48 hours** to respond with their side and evidence.
3. Owner sees one screen: order, payment, delivery timeline, handover-code status, both sides' evidence, the order chat (including flagged originals).
4. Owner decides: **full refund**, **partial refund** (amount set), or **release to seller**.
5. Both parties notified; decision written to the audit log permanently.

## 9. Reviews

- Only buyers with a **confirmed delivery** can review that order's store.
- 1–5 stars, text, optional photos.
- Seller may reply once.
- Reviews pass the same word/tone/scam checks as chat.
- Store page shows average rating and count.

## 10. Notifications

Push (plus in-app inbox) for: payment received (seller), order status changes (buyer), new chat message, confirm-delivery reminders, funds released (seller), dispute opened/decided, moderation warnings.

## 11. Owner dashboard (web)

- Access codes: create (single/multi-use, cap, expiry), list, revoke.
- Users & stores: search, view, suspend, ban.
- Orders & payments: status, ledger view, stuck orders (30-day backstop queue).
- Disputes: queue + decision screen (section 8).
- Moderation: flagged messages/reviews, strikes, bans.
- Settings: platform cut %, delivery cut-off time, strike thresholds.
- Every admin action is audit-logged (who, what, when).
- **Several admins with roles** *(my call)*: **Owner** (everything, including settings and adding admins) and **Support** (disputes and moderation only; can see personal data only inside an open dispute).

## 12. How it's built

| Part | Choice | Decided by |
|---|---|---|
| Mobile app (Android + iOS) | **Flutter** | Owner chose Flutter/RN; Flutter is my call |
| Server/API | **Rust** (e.g. Axum) | Owner |
| Database | **PostgreSQL** | My call |
| Owner dashboard | Web app | My call; stack left to builder |
| Payments | Provider layer; Mono (DirectPay + Disburse) at launch, Paystack added later; 5% platform cut | Owner chose escrow, the 5% cut, Mono first and easy switching |
| Couriers | Shipbubble (Terminal Africa fallback) | My call from research |
| Postcodes | NIPOST postcode API | Owner |
| Chat moderation AI | Claude Haiku | My call |
| Photos | Object storage (S3-compatible) | Builder's call |
| Push | Firebase Cloud Messaging | Builder's call |

## 13. Data protection (NDPA 2023 primary)

- **Minimise:** collect only what each feature needs. No BVN/NIN/card numbers stored.
- **Access:** buyer address and phone visible only to the seller of that order and its courier, and only while the order is active.
- **Encryption:** TLS everywhere; sensitive columns (addresses, phone numbers) encrypted at rest.
- **No PII in logs or URLs.**
- **Audit:** append-only logs for money movements, admin actions and access to personal data in disputes.
- **Data-subject rights:** users can download their data and request deletion; deletion is soft-delete followed by purge after the retention period, except records that must be kept for financial/legal reasons.
- **Third parties:** Mono (and later Paystack), Shipbubble, NIPOST, AI moderation provider, push provider — all listed in the privacy notice.
- **Needs DPO/legal review:** escrow arrangement (CBN), AI processing of chats, retention periods.

## 14. Not in the first version (non-goals)

- Cities outside Abuja.
- A web storefront for buyers (mobile only).
- Cash on delivery.
- More than 2 stores per user.
- Seller ads/promoted listings.

## 15. Left to the builder

- Exact auth method details (email vs phone primary).
- Screen-by-screen visual design, within the "polished, not templated" bar.
- Database schema, API shape, job scheduling for auto-release and reminders.
- Default strike thresholds.
- Search and category structure.

# Part 7e-3: Buying screens in the app, design

Status: draft for owner approval, 2026-11-26. Nothing is planned or built until it is approved.
Builds on: 7b (account, addresses), 7c (market, product page, cart), 7e-1 (branches, collect), 7e-2 (selling area, handover codes). Read with `2026-11-21-immersive-market-design.md`.

## 1. Intent

A buyer can check out, pay, follow an order, confirm it arrived or report a problem, review the store, and hear about it all, from the app. The server routes already exist; this part is screens, the data layer and push. Today the cart page ends in a "checkout soon" line.

## 2. Goals, non-goals, anti-goals

**Goals**
1. Cart to paid order in few taps: pick an address, see delivery options per store (and per branch when a store splits), pay, land on the order.
2. The app never decides a payment succeeded: it polls `GET /checkout/{id}`, which re-verifies with the provider.
3. Orders and tracking show the real steps for each delivery method, including collect from a branch (pickup point, handover code).
4. The buyer gives the handover code to the seller only when they hold the goods; the code is shown on the tracking screen while it is valid and never put in a notification, log or route.
5. Received, cancel, review, dispute (with photos, withdraw) and the notices inbox, with push.
6. Fast on mid-range phones; every control gives press feedback; `Tokens` spacing; plain words.

**Non-goals**
- Chat (7e-4). Selling screens (7e-2, built). Art (7f). Paying outside the app, saved cards, wallets.
- A second payment provider. The payment view loads whatever `payment_url` the server returns.

**Anti-goals (failure even if it works)**
- Trusting the payment redirect instead of the server's answer.
- Personal data (address, phone, handover code, payment URL) in logs, analytics, notification text or deep links.
- A double charge from a double tap: one idempotency key per checkout attempt, reused on retry.
- Seller contact shown beyond what the server returns for the buyer.

## 3. Flow

```
Cart ─► Checkout: address ─► delivery options (per store/group) ─► review totals ─► Pay (webview) ─► Result
Orders list ─► Order: steps, tracking, handover code / pickup point ─► Received | Cancel | Report a problem | Review
Inbox (bell in Account) ─► opens the order or dispute
```

## 4. Screens

**A. Checkout.** From the cart's "Checkout" button.
1. *Address*: choose from the address book (7b), or add one. Out-of-area addresses are refused by the server with its message.
2. *Delivery options*: `POST /checkout/delivery-options {address_id}`. For each store, each fulfilment group (a branch and its lines) shows its options (courier, the seller's delivery, collect from branch) with fee and promised date. A store whose goods split across branches shows the groups plainly ("sent from two branches"). Lines no branch can fill whole show "only N available" with a way to change the quantity.
3. *Review*: items, delivery fees, total. One method for the whole checkout, as the server takes `{method, address_id, groups[]}`.
4. *Pay*: `POST /checkout` with an `Idempotency-Key` (random UUID made when the screen opens, reused on retry). Errors: 409 `allocation_changed` (options went stale: refresh and show what changed, nothing was reserved), `checkout_busy`, too many pending checkouts, 502 (try again).
5. *Payment view*: `webview_flutter` loads `payment_url` (https only, host checked against the one the server sent). The app watches for the redirect to the configured return URL, closes the view, then polls `GET /checkout/{id}` every 3 s for up to 2 minutes. Result states: paid (orders listed), still pending ("we will tell you"), failed or expired (stock released, try again). Closing the view early also lands on the polling screen, never on a guess.

**B. Orders.** List (paged, newest first) with status chip, store, total. Detail: items, totals, status, the seller's name only.

**C. Tracking.** `GET /orders/{id}/tracking`: the steps with times, promised date, delay note, courier contact or tracking link when given (https only, opened in the browser). For self delivery out for delivery: the buyer's **handover code**, with the server's note ("give it only when you hold your goods"). For collect: the pickup point (branch address and business phone, shown only after payment, as 7e-1 says) and the code. Pull to refresh; refreshes on return.

**D. Received and cancel.** "I received it" (confirms; the money is released). "Cancel" while the order is still unshipped, with a confirmation that says a full refund follows (takes up to 10 working days). Errors in plain words.

**E. Report a problem.** Reason (the server's list), a statement, up to a few photos with the same guided capture and upload as 7e-2 (purpose `dispute`), then a status page: the seller's answer when there is one, a return request if the market asks for one, the decision, and "withdraw" while open.

**F. Review.** After completion: stars, text, up to a few photos (purpose `review`), with the tone check's suggestion shown if the server returns one (`use_suggestion`).

**G. Inbox and push.** `GET /me/notifications`: the server's fixed titles and bodies, unread marks, tap opens the order or dispute. Push uses `firebase_messaging`: ask permission at the first paid order (not at start), register the token (`PUT /me/push-device`), remove it on sign-out. A tapped notification routes by `kind` and ids from the payload only; the text comes from the inbox. Foreground pushes show an in-app banner.

## 5. Structure and data layer

- `app/lib/features/checkout/`, `orders/`, `inbox/`, `payments/` (webview and polling). One repository interface per area with API implementation and fakes, as in 7e-2. Hand-parsed models; money in integer kobo.
- Payment return URL and allowed host come from the checkout answer and `GET /app/config`; the webview blocks every other navigation and does not run with a persistent profile.
- Idempotency key, selected options and the payment URL live in memory only.
- Copy in a `BuyStrings` file. New package: `webview_flutter`, and Firebase packages for push.

## 6. Testing and checks

- Unit: option grouping and totals, idempotency key reuse, redirect and host checks, polling state machine, notification routing by kind.
- Widget tests with fakes for each screen's states, the 409 `allocation_changed` refresh, the "never claims paid" rule, code shown only while valid, collect pickup display, press feedback, 200% text.
- Wire check against a real local server: address, options, checkout creation (the payment itself cannot run locally: the server's fake provider has no web page, so the check stops at `payment_url` and then simulates the provider's webhook if the fake provider can be switched on; otherwise it stops at creation), orders list and tracking for an order made paid through the server's test helper SQL.
- CI-equivalent: `dart format`, `flutter analyze`, `flutter test`. Not run on a device. Real payments and push delivery cannot be tested here and are go-live checks (Mono sandbox, Firebase project).

## 7. Data protection (for DPO / legal review, not legal advice)

- Addresses are read from the server per screen and held in memory; the chosen one is sent by id.
- The handover code is displayed, not stored, not copied to the clipboard by the app, and excluded from screenshots on Android (`FLAG_SECURE`) while shown.
- Push payloads carry kind and ids only; notification text is the server's fixed copy.
- The webview does not persist cookies or storage; the payment page belongs to the provider, who is the processor for it.
- Dispute and review photos are stripped of location tags before upload (same pipeline as 7e-2).
- Push needs a notice to the buyer about what is sent to Google/Apple (token only); wording for legal.

## 8. Open questions (defaults stand unless changed)

- Ask for push permission at the first paid order (default) or at first launch.
- Reviews open on the order page only (default), not from the store page.
- Photo limits for disputes and reviews: the server's limits.

## 9. Left to the builder

Folder and class names, skeleton shapes, exact copy outside the strings above, bell icon placement.

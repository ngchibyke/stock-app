# Part 7e-2: Selling screens in the app, design

Status: draft for owner approval, 2026-11-26. Nothing is planned or built until it is approved.
Builds on: 7e-1 (staff and branches), 7e-1b (identity check), 7c/7d (the market and its room). Read with `2026-11-21-immersive-market-design.md` (look and feel) and the 7e-1 and 7e-1b specs.

## 1. Intent

A person can become a seller, open a store, get paid, list products with good photos, run orders with their team across branches, answer reviews and disputes, and walk into their own store, all from the app. The server for every step already exists; this part is screens and the app's data layer. Owner decisions so far (2026-11-26): the seller side is a separate area, the camera guide is a tips sheet plus a framing overlay, and the work is one branch with staged commits.

## 2. Goals, non-goals, anti-goals

**Goals**
1. The whole selling path works end to end against the real server: access code, identity check, store, payout account, first product, first order.
2. Owners, managers and staff each see only what their role may do (the server decides; the app hides what would be refused and still handles the 403).
3. Product photos come out square, lit and consistent, so the market rooms look good.
4. Fast on mid-range phones (Android 8, iOS 15): lists are paged, photos are resized before upload, nothing blocks on a spinner that could be a placeholder.
5. Every control gives press feedback (`Pressable`/Material buttons), spacing comes from `Tokens`, colours from `context.tokens` ("Abuja Dusk").

**Non-goals**
- Chat (7e-4), buying screens and push handling (7e-3), art (7f).
- Editing the store's market placement, or any admin function.
- Bulk product import, discounts, analytics.
- A real identity provider SDK (not chosen); the capture sits behind an interface with a development fake.
- Running on a device: it is not possible here. Work is verified by `flutter analyze`, `flutter test` and wire checks against a real local server only.

**Anti-goals (a failure even if it works)**
- Anything personal (NIN, selfie, phone, payout account number, buyer address) written to logs, analytics, crash text or kept longer than the screen needs. The NIN field is held in memory only, cleared on leaving.
- A seller told one thing and shown another: when they turn collect on they must be told buyers will see the branch address and business phone after paying.
- Templated, "AI-looking" screens: plain words, our colours, real states for empty, loading and error.

## 3. Shape: a separate Sell area

Account gets one entry:
- Not a seller and not a member: **Sell on Abuja Market** (starts the access-code path).
- Seller or team member: **My stores** (opens the Sell area).

The Sell area is its own shell with a bottom bar and a back-to-shopping action: **Orders · Products · Store · Team** (Team only where the role allows). It remembers the chosen store (a seller may own two; a member may belong to several). The store switcher is a sheet from the header.

```
Account ── Sell on Abuja Market ─► code ─► identity check ─► create store ─► payout account ─► Sell area
        └─ My stores ──────────────────────────────────────► Sell area (store switcher)
Sell area:  Orders | Products | Store | Team
```

New routes live under `/sell/...` and are signed-in only. The router's pure redirect keeps working: unknown `/sell` paths go home.

## 4. Screens

**A. Become a seller.** Enter the access code (existing `POST /seller/redeem`; digits and letters as issued, one field, paste allowed). Wrong, expired or used up codes show the server's one message. On success: the identity step, then create store. A person who is already a seller skips it.

**B. Identity check** (hosts 7e-1b). States from `GET /me/identity`: unverified, in progress, passed, review, failed, revoked.
1. *Notice* (plain words; **wording needs legal review**, kept in one constants file): why we ask (stop scams and fake stores), that the NIN and selfie go to the provider to check against NIMC, what we keep (result, verified name, a fingerprint of the NIN that cannot be turned back), that a fingerprint is kept after closing only for fraud bans, who to contact. An "I agree" control, then Continue.
2. *NIN entry*: digits-only keyboard, input formatter that drops non-digits and stops at 11, Continue enabled only at 11, never shown in logs or restored after leaving, cleared on pause.
3. *Selfie capture*: `SelfieCapture` interface (`start(sessionToken) -> done/cancelled/failed`). Development uses a fake that completes after a tap; the real provider SDK is plugged in at go-live.
4. *Waiting*: polls `GET /me/identity` (every 3 s, up to 2 minutes, then "we'll tell you", push notice `identity_passed` / `identity_declined` already exists).
5. *Result*: passed (continue), review (we are checking, nothing to do), failed or refused ("We could not verify this NIN" plus Contact support; never says why), revoked (contact support), unavailable (503: try later).
Errors map: 422 bad NIN, 409 `identity_in_progress` / `identity_in_review` / `identity_already_verified` / `identity_not_accepted`, 429 `identity_rate_limited`, 503 `identity_unavailable`.
Members who were invited get the same flow from a banner on their orders tab ("Confirm your identity to act on orders", the server answers `identity_required`). Owners with stores inside the grace period see a banner with the date (`required_by`).

**C. Create a store.** Name, district choice (the market's districts from the server, one tap each with a short description), pickup address, area council, business phone, delivery options (courier, self-delivery with fee, collect off by default), then payout account. Uses `POST /stores`. Pickup fields at creation go to the store's first branch; **after creation the app edits pickup details only through Branches**, never `PATCH /stores` (that path may be deprecated).

**D. Payout account.** Bank (list), 10-digit account number (digits only), password re-entry when changing, shows the resolved name and the state (verified, pending review with the reason in plain words). A store waits (`pending`) until the payout account is verified and the owner's identity has passed; the screen says which of the two is missing.

**E. Products.** List (paged, search, status), create and edit: title, description, category, price in naira (integer kobo underneath), variants (attributes and stock per branch), publish, delete. Photos (section 5). Stock edits by branch for staff tied to a branch (the server enforces; the app shows only that branch's counts).

**F. Orders.** Tabs: To ship, On the way, Done. A row shows items, branch, delivery method, age. Detail shows steps for the method with one primary action at a time: self delivery (picked up, out for delivery, delivered), courier (ready), collect (ready, collected by entering the buyer's code). Entering a handover code is six digits, locked-out messages from the server shown plainly. No buyer phone or address is shown beyond what the server returns for that role. Who did each step is shown (`done_by`).

**G. Store settings.** Details, delivery methods, a **collect** switch per branch with the notice: "Buyers will see this branch's address and business phone after they pay." (confirm before turning on). Closing a store, deleting it.

**H. Branches** (owner): list (up to 20 open), add, edit address, phone, fee and collect, close (refused while orders are open: show the server's reason).

**I. Team** (owner, managers read): members and roles, invite by phone (manager or staff, optionally tied to a branch), pending invites, remove, move staff. The invited person sees invites in Account (`GET /me/invites`, accept or decline). Phone entry follows the sign-up phone field rules.

**J. Reviews and disputes.** Reviews of the store with a reply box (`ReplyReviews`: owner and managers). Disputes on the seller's orders: read the buyer's statement and photos (private, fetched with the session), answer within the deadline (`POST /disputes/{id}/response`, with photos), mark a return received. Notices for new disputes arrive through the existing inbox (7e-3 wires push taps).

**K. Walk into my store.** A button on Store (when the store is active) that opens the store's room through the existing market route (`/stores/{slug}/room`) with a back action to Store.

## 5. Photos and the camera guide

- Packages: `camera` for the guided capture, `image_picker` for the gallery. Both ask permission only when first used, with a one-line reason; a denial leaves the gallery path working.
- **Guide sheet** (first use, reopenable): plain background, window light not behind the product, fill the frame, one product per photo, shoot front, back, detail and in use.
- **Framing overlay** on the live camera: a square crop outline with thirds, a one-line tip that changes per shot (Front, Back, Detail, In use), tap to focus, flash toggle, shutter with press feedback and a light haptic.
- Capture output is cropped to the square, resized so the long edge is 1600 px, JPEG quality 82 (a typical photo lands near 300 KB), EXIF stripped (including location) before upload. This is on the main thread's budget only for the crop; resizing runs in an isolate.
- Upload: the server's presign flow (`POST /uploads`, direct upload to the returned URL, `POST /uploads/{id}/finalize`), up to the server's per-product photo limit, with per-photo progress, retry and remove, and the first photo as the cover. A photo that fails never blocks saving the other fields.
- Photos are the seller's own business content, but may show people or places by accident: a hint tells them to keep faces and house numbers out of frame, and nothing is kept on the phone after upload.

## 6. Data layer and structure

- `app/lib/features/sell/` with sub-folders `become`, `identity`, `store`, `payout`, `products`, `orders`, `team`, `branches`, `reviews`, `disputes`, and `shell`. One repository interface per area (as `AddressRepository`), a Dio implementation and a fake in `test/support` for widget tests.
- Models are plain classes with `json_serializable`; money stays integer kobo (`core/format/money.dart`); the nin and tokens are never in `toString`.
- Riverpod providers per repository; the selected store id lives in one provider and is validated against `GET /stores/mine` on each refresh (a removed member loses the store at once).
- Role-based UI is derived from the role the server returns for each store; every action also handles 403 `not_allowed` and `identity_required` with a plain message.
- Copy lives in `core/strings.dart` (existing convention), identity notice in its own file for legal.

## 7. Testing and checks

- Unit tests: NIN formatter and validation, error mapping, name and money formatting, photo crop and resize maths, role-to-UI mapping.
- Widget tests with fakes for each screen's loading, empty, error, success and role variants, press-feedback checks on the primary controls, and "no NIN in any rendered text, route or log" checks.
- Wire checks (separate, as 7b to 7d) against a real local server: redeem a code, identity with the fake provider, create a store, payout, create a product with an upload against a local store, take an order through its steps.
- CI-equivalent: `dart format`, `flutter analyze`, `flutter test`. Never claimed run on a device.

## 8. Data protection (for DPO / legal review, not legal advice)

- NIN: memory only; no persistence, no deep link, no clipboard history prompt (field marked sensitive, autofill off).
- Selfie: captured and sent by the provider's SDK straight to the provider; the app never holds an image.
- Product photos: EXIF location stripped before upload.
- Notice wording and the consent control need legal review; text lives in one file.
- The camera and photo permissions need purpose strings in the iOS and Android manifests, reviewed with the store listings (7g).

## 9. Open questions (defaults stand unless changed)

- Whether a store may have collect on with only one branch (default yes).
- Order list refresh: pull to refresh and refresh on return (default), push-driven updates come with 7e-3.
- Photos per product: the server's limit (default).

## 10. Left to the builder

Folder and class names, exact copy outside the strings above, loading skeleton shapes, the bottom bar icons.

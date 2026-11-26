# Part 7e-1: Store staff and branches (server), design

Status: draft for owner review, 2026-11-25.

## 1. Intent

The launch market is big stores in Abuja. Two things about them do not fit the server today:

- **Several people run one store.** Owners, managers, packers and the person at the door entering the handover code. Today a store has one `owner_id` and only that account can touch it.
- **One store has several locations** (say Wuse and Garki). Today a store has one pickup address and one stock count per variant.

The owner asked for the app to "handle multi-location marts efficiently in Abuja". For a buyer that means: one store, delivered from the nearest branch that has the goods, with stock that is true. For the store it means each branch's staff see their own orders and stock.

This piece is **server only**. It has to land before the selling screens (7e-2), which build on it.

### Where this sits

Part 7e (in `2026-11-20-07-mobile-app.md`) was "buying, selling and chat". It is split, in this order (owner decision, 2026-11-25):

| # | Piece |
|---|---|
| **7e-1** | **Store staff and branches (server): this design** |
| 7e-2 | Selling (app): become a seller, open a store with the district choice, payout account, products with photos and the camera guide, branches and team screens, orders and their steps, reviews and dispute answers, "walk into my store" |
| 7e-3 | Buying (app): checkout and the payment view, orders, tracking, handover code, received, cancel, review, disputes, inbox and push |
| 7e-4 | Chat (app), with the moderation states; it uses this piece's roles to decide who answers buyers |

7f (art) and 7g (release) keep their numbers. The overview and roadmap are updated to match when this plan is written.

## 2. Goals, non-goals, anti-goals

**Goals**

1. A store has a **team**: the owner plus invited members, each a **manager** or **staff**. Staff can be tied to one branch.
2. A store has one or more **branches**, each with its own address, phone, self-delivery fee and stock per variant.
3. At checkout the server **chooses the branch** for each store in the cart: the nearest branch that can fill it, or a split across branches when none can. The buyer sees the branches, delivery fees and dates before paying.
4. Every seller-side action records **which person did it**.
5. A store with one branch and no team behaves **exactly as today**. Every existing server test still passes, changed only where a response shape gains fields, with one exception: the checkout and delivery-options request and response shapes change (section 7), and the tests that use them (at least `tests/common/mod.rs` and `tests/escrow.rs`) are rewritten to the new shape. No client uses checkout yet, so no compatibility path for `{store_id, option_id}` is kept. **(my call)**

**Non-goals (not in 7e-1)**

- Transferring store ownership.
- Different prices per branch. Price stays per variant, store-wide.
- Moving stock between branches as a tracked transfer. The seller edits each branch's count.
- Opening hours, shifts, per-person custom permissions.
- A plot per branch on the market map. A store keeps one plot and one room.
- Any app screens. 7e-2 builds them on this.
- Chat routing to staff (7e-4). Until then chat stays with the owner, as today.

**Anti-goals (failure even if it works)**

- A small seller with one location sees any added complexity. Branches and team are invisible until the seller adds a second branch or a member.
- The invite flow reveals whether a phone number has an account.
- A buyer pays a delivery fee or gets a split they were not shown before paying.
- A removed member keeps any access, even for one request.
- Stock goes negative, or the store-wide "available" number disagrees with the sum of its branches.

## 3. Constraints

- Rust/Axum server, Postgres, sqlx, the existing migration sequence (next is `0044`).
- The existing lock order is extended, never reordered: users, stores, **store_members**, store_plots, **store_branches**, products, variants, **branch_stock**, and so on. Builder confirms the exact slot against the code's documented order.
- Personal data rules (NDPA 2023 primary): addresses and phones encrypted with `FieldCipher` as `stores` does today; phone lookups by the existing blind index (`phone_idx`), never the phone; nothing personal in logs or URLs; audit log is append-only (it already rejects update and delete).
- Money in integer kobo. The server prices everything; the client sends option ids only.
- Courier calls cost money and time. A checkout must not fan out unbounded courier quotes.

## 4. Approaches considered

**A. Branches inside one store (chosen by the owner).** One store, one room, one plot. Branches hold address and stock. Checkout picks the branch. Most work: stock, reservations, delivery options, checkout and orders all change, and the money path must stay correct.

**B. Linked stores under one brand.** Each location is a full store; a brand groups them with a shared team and a copied product list. Checkout and delivery are untouched, so it is cheaper. Rejected: buyers see the same chain several times and pick a location themselves, the opposite of "efficient" for a buyer, and the two-store limit has to bend for chains.

**C. One stock count, several pickup points.** Dropped early: stock is wrong at every branch and delivery quotes are wrong, which fails "efficiently" from the start.

## 5. Team (members and roles)

### Model

- `stores.owner_id` stays. The owner is always a full member and is never a row that can be removed.
- New table `store_members`: `store_id`, `user_id`, `role` (`manager` | `staff`), `branch_id` (NULL means all branches), `added_by`, `created_at`, `removed_at`, `removed_by`. One live row per (store, user). Removal is a soft end (`removed_at`), so history and audit stay readable.
- New table `store_invites`: `store_id`, `phone_idx` (blind index of the invited phone), `phone_last4` (for the owner's list, nothing more), `role`, `branch_id`, `invited_by`, `created_at`, `expires_at` (7 days, **my call**), `accepted_at`, `revoked_at`. The phone itself is **not** stored: the owner typed it and only needs to recognise it.

Managers are always store-wide. Staff may be store-wide or tied to one branch. **(my call)**

### What each role can do (owner-approved)

| Action | Owner | Manager | Staff |
|---|---|---|---|
| Payout account, delete store, manage the team, add or close branches | ✓ | | |
| Store details, product prices, publish or unpublish | ✓ | ✓ | |
| Add or edit products (not price or publish) and stock | ✓ | ✓ | ✓ (stock only at their branch if branch-tied) |
| Order steps and handover code | ✓ | ✓ | ✓ (only their branch's orders if branch-tied) |
| Reply to reviews, answer disputes | ✓ | ✓ | |
| See the store's orders | all | all | their branch's, or all if store-wide |
| Walk into the store before it is public | ✓ | ✓ | ✓ |

**Price and publish are manager actions.** The API forces a price at creation (`base_price_kobo` on `POST /stores/{key}/products`, `price_kobo` per variant in `PUT /products/{id}/variants`), so the rule is about what is live:

- Staff may create a product with its prices; it is created unpublished, and only an owner or manager can publish it.
- Staff may change prices, and add or remove variants, only while the product is unpublished. On a published product, a request from staff that changes any price, adds or removes a variant, or changes the published flag is refused with 403 `not_allowed` and a plain message. Stock and the other fields (name, photos, description and so on) stay open to staff.
- Deleting a product is owner and manager.

**(my call, following the table: what goes live, and at what price, is a manager's decision)**

**Store details split by role.** Through `PATCH /stores/{key}`, managers may change name, description, logo and banner. Everything that touches money, delivery or the market plot is owner-only: pickup fields (which edit the first branch), the self-delivery fee, the `delivery_courier` and `delivery_self` flags, the business phone and `district`. A manager's request that includes an owner-only field is refused whole with 403 `not_allowed`. **(my call: consistent with branches being owner-only)**

**Dispute steps.** Answering a dispute (`/response`) is owner and manager. Confirming a returned parcel arrived (`/return-received`) is any member who may act on that order, because the parcel arrives at the branch. **(my call)**

### Invites

- `POST /stores/{key}/invites` (owner): phone, role, optional branch. Answers the same 201 whether or not the phone has an account, so it cannot be used to probe for accounts. Rate-limited per store **(my call: 20 per day)**.
- `GET /me/invites` (any signed-in user): invites matching the caller's own `phone_idx`, live and unexpired, with store name and role. Because it matches on the caller's own verified phone, it works whether they signed up before or after the invite.
- `POST /me/invites/{id}/accept` and `/decline`.
- `GET /stores/{key}/team` (owner and managers read; owner manages): members and pending invites.
- `PATCH /stores/{key}/members/{user_id}` (owner): role or branch. `DELETE` ends membership.
- An owner cannot invite themselves; an existing member gets 409 `already_member`. A store's team is capped **(my call: 50 members)**.
- Accepting does **not** make someone a seller (`is_seller` stays as it was) and does not count toward their two-store limit. The limit counts stores a user **owns**.
- A member cannot buy from that store, the same rule as the owner today (checkout and delivery options refuse it).
- A deleted user's memberships end with the account.

### One permission check

The ~45 places that compare `owner_id` to the caller are replaced by one function in a new module (`stores::access`, name the builder's), roughly `require(store, user, Action) -> Membership`, and an SQL fragment for list queries ("stores this user can act on for X"). Rules:

- Checked on every request against the database. No permission is cached in the session or a token, so removal takes effect on the next request.
- A non-member gets the same 404 a stranger gets today (never "exists but forbidden"). A member without the right role gets 403 `not_allowed`.
- Buyer-side reads that today exclude the owner ("not your own store") use "not a member".
- Routes that are about the owner's own account (`GET /stores/mine`) become "stores I own or work at", each with my role and branch.

## 6. Branches

### Model

- New table `store_branches`: `store_id`, `name` (e.g. "Garki"), `pickup_address_enc`, `pickup_area_council`, `pickup_postcode`, `business_phone_enc`, `self_delivery_fee_kobo`, `pickup_courier_address_code` (the cache that lives on `stores` today), `position` (display order), `closed_at`, `created_at`. Cap **(my call: 20 per store)**.
- The store's delivery method flags (`delivery_courier`, `delivery_self`) stay store-wide. **(my call: one policy per store; per-branch can come later)**
- Migration: every existing store gets one branch named after the store, holding its current pickup fields and courier code. Store-level pickup columns stop being written; the builder removes or keeps them read-only, but no code reads them after this piece.
- `POST /stores` keeps its shape and creates the first branch from the pickup fields it already takes. `PATCH /stores/{key}` pickup fields now edit the first branch for compatibility; new routes manage branches:
  - `GET /stores/{key}/branches`, `POST` (owner), `PATCH /stores/{key}/branches/{id}` (owner), `POST .../close` (owner).
- **Closing** a branch: refused (409 `branch_has_open_orders`) while it has orders not yet delivered or cancelled. Once closed, its stock is no longer sold and it is not offered at checkout. The last open branch cannot be closed. Staff tied to a closed branch lose order and stock access until moved.

### Stock

- New table `branch_stock`: `branch_id`, `variant_id`, `stock`, `reserved_quantity`, with the same checks as today (`0 <= reserved <= stock <= 100000`). This is the source of truth.
- `product_variants.stock` and `reserved_quantity` become **store-wide totals over open branches**, kept equal to the sum inside the same transaction as every change. Existing readers (`BUYABLE`, the room's shelves, catalog, cart) keep working unchanged. The builder may instead turn them into a view or computed column if that is simpler and as fast; the invariant is what matters, and it gets its own test (section 10).
- Migration: each variant's stock and reservations move to the store's first branch.
- **What a buyer sees as available.** A line is never split across branches (section 7), so the buyer-facing figure is the **largest amount available at any one open branch**, not the total. Wuse has 3 and Garki has 2: the buyer sees 3 and the cart caps the quantity at 3. The catalog, room, cart and `BUYABLE` read this per-branch maximum (kept beside the total, or computed; the builder's choice). The store-wide total is for the seller's own views. "In stock" (more than zero anywhere) is unchanged either way. **(my call)**
- `PUT /products/{id}/variants` today sets `stock` per variant. It gains an optional per-branch form: `stock: {branch_id: count}`. The plain number still works for a one-branch store and is refused (422 `branch_required`) once a store has two open branches.
- A new `PUT /branches/{id}/stock` sets counts for many variants at one branch at once (what a branch-tied staff member uses after a delivery arrives).

## 7. Checkout: choosing the branch

Today: `POST /checkout/delivery-options` answers options per store; the buyer picks one per store; `POST /checkout` makes one order per store in one checkout and payment.

After: the unit is a **fulfilment group**: (store, branch, the cart lines that branch sends). A store with one branch has exactly one group, which is today's behaviour.

### Allocation rule (my call; the builder may refine within it)

For each store in the cart, given the buyer's chosen address:

1. Candidate branches: open, with stock for the lines.
2. If one or more branches can fill **every** line: rank them by same area council as the buyer first, then by position. Quote delivery from the top **three** at most, and pick the cheapest fee, then the earliest promised date. One group.
3. If none can fill everything: take the branch that fills the most lines (same council first, then position), assign those lines, repeat on what is left. At most **three** groups per store.
5. **Lines no branch can fill.** Today `delivery-options` does not check stock and `POST /checkout` refuses the whole cart with 409 `out_of_stock` if any line is short. After this piece, `delivery-options` marks such lines `unavailable` (with what is available, if anything) and puts them in no group, so the buyer sees it before choosing. `POST /checkout` keeps today's rule: while any cart line is unavailable, it refuses with 409 `out_of_stock`, and the buyer removes or reduces the line first. A checkout never buys part of a cart silently. **(my call: the smallest change to the money path, and the buyer always pays for exactly what they saw)**
4. A line is never split across branches (quantity 3 comes from one branch). **(my call: simpler for packing and refunds)**

Area council is the only location the server has for both sides (no coordinates), and AMAC covers most of central Abuja, so "nearest" is approximate. That is why the rule falls back to the cheapest quoted fee among the top candidates. Coordinates or postcode-district ranking can refine it later without changing the API.

### API

- `POST /checkout/delivery-options` answers, per store, a list of **groups**, each with `group_id`, `branch: {id, name, area_council}` (no address, no phone), its lines, and its delivery options. The `delivery_options` row gains `branch_id` and the group's line fingerprint.
- `POST /checkout` takes one `{group_id, option_id}` per group (replacing `{store_id, option_id}` per store). The server re-runs the allocation check under the stock locks: if stock moved and the groups no longer hold, it answers 409 `allocation_changed` and the app asks for options again. It never silently re-routes after the buyer saw a price.
- One order per group. `orders` gains `branch_id`. Reservations are taken from `branch_stock` (and the store total) for that branch only. Expiry, refund and cancel restore to the same branch.
- Payment is still one checkout, one payment; payouts and the ledger stay per order and per store, so splitting by branch needs no ledger change.

The 7c app has a cart but no checkout yet (it shows "checkout coming soon"), so the new shape breaks no client.

### Delivery and handover

- Courier quotes, booking, shipments and the "store has moved since quote" check use the order's **branch** address code, not the store's.
- Self-delivery uses the branch's fee.
- Promised dates, deadlines and the escrow clocks are unchanged; they are per order already.
- The handover code is entered by any member who may act on that order.

## 7a. Collect from a branch

Owner decision (2026-11-25): buyers can collect from a branch at launch; the screens are in 7e-3. The server side is here because it changes the same checkout and delivery code. **The calls below are mine unless marked.**

- **A new delivery method, `collect`**, beside `courier` and `self`. Fee 0. A store turns it on per branch (`collect_enabled` on `store_branches`, owner-only, off by default), because not every branch has a counter for it.
- **Where it is offered.** Each group from the allocation (section 7) whose branch has collect turned on gets a collect option alongside its delivery options. The buyer does not pick a different branch for collection; the allocation already chose the nearest branch that has the goods. Picking a branch is a later refinement if buyers ask for it.
- **What the buyer sees.** Before paying: the branch name and area council (as for delivery). After paying: the branch's address and business phone, so they can find it. This is the business's own pickup address, which a seller already gives the courier; sellers are told in 7e-2, when they turn collect on, that buyers will see it.
- **Order steps.** Paid, then **ready for collection** (a member at that branch marks it), then **collected**: the buyer shows their handover code at the counter and a member enters it, exactly as a delivery handover works today. That ends delivery and starts the existing release clock.
- **Promised date** is when the order will be ready: the existing ship-deadline logic, without a courier pickup date.
- **Not collected.** If the order is not collected within **5 days** of being ready, it is cancelled with a full refund, stock goes back to that branch, and the seller gets no strike, since the seller did their part. Reminders go to the buyer on day 1 and day 4. The 5 days is a platform setting, like the other deadlines.
- **Disputes and reviews** work as for any order.
- Buyer contact: the branch sees the buyer's name on a collect order, never the phone. A member who needs to reach the buyer uses chat (7e-4).

## 8. Seeing personal data, and the audit trail

- **No new disclosure.** Members inherit today's rule exactly: a seller never sees the buyer's address or phone on courier orders, and on self-delivery orders sees them only while the order is `out_for_delivery`. Within the team, only members who may act on that order get them.
- The existing audit, `order.contact_revealed` on the first reveal per (order, viewer), now applies per member, so it names which person saw the contact. **(my call: an earlier draft had a packing view showing the address; that would have been new disclosure of personal data, so it is dropped)**
- Every seller-side write records `actor_id` (the member) in the audit log, which already has the column. Today's entries assume actor = owner; after this piece they name the real person. Team changes and invites are audited (`store.member_added`, `store.member_removed`, `store.member_role_changed`, `store.invite_sent`, `store.invite_revoked`), with the invited phone never in the entry (last 4 at most).
- Order step responses and the seller order list say who did each step ("marked ready by Amina"), using the member's display name.

## 9. Notifications

Today seller notifications go to the owner. After:

- New order, ship deadline close, order cancelled: the owner, every manager, and staff who may act on that order (store-wide staff, or staff of that branch).
- Disputes and reviews: owner and managers.
- Payout account and store status: owner only.
- An invite: the invitee gets an in-app notification if they have an account (push in 7e-3). No SMS for invites **(my call: an SMS would tell a non-user that someone typed their number; the owner can tell staff themselves)**.

## 10. Testing

- **Regression first.** The whole existing server suite passes. A one-branch, no-team store's responses match today's except for added fields.
- Permission matrix: a table-driven test over every seller route × {owner, manager, store-wide staff, branch staff at the right branch, branch staff at another branch, removed member, stranger}, asserting allowed, 403 or 404.
- Removal takes effect on the very next request.
- Invites: same response for known and unknown phones; accept only by the matching phone; expiry; revoke; already a member; cap.
- Stock invariant: after any sequence of reserve, commit, expire, refund, cancel and edit across branches, each variant's total equals the sum over open branches, and nothing is negative. A property-style test is a good fit.
- Allocation: one branch fills all; two can, cheapest wins; split across two and three; partly unavailable; at most three quotes per store; `allocation_changed` when stock moves between options and checkout.
- Concurrency: two checkouts racing for the last unit at one branch, as the existing reservation tests do.
- Delivery: courier quote, booking and moved-address checks use the branch; closing a branch with open orders is refused.
- Audit: actor is the member; `order.contact_revealed` names the member and holds no phone.
- Disclosure: a member never sees the buyer's contact on a courier order, nor before `out_for_delivery` on a self-delivery order, and branch staff never see another branch's.
- Buyer availability: the per-branch maximum, not the total, in catalog, room and cart; unavailable lines in `delivery-options`; checkout refuses while any line is unavailable.
- Staff product rules: price, variant and publish changes refused on published products, allowed on unpublished ones.
- Collect: offered only where the branch has it on; ready, then collected by handover code; the 5-day auto-cancel refunds in full, restores branch stock and gives no strike; the address and phone are shown only after payment.
- Migration test: a database with stores, stock, orders and reservations from before migrates to one branch each with identical numbers.

## 11. Left to the builder

- Module layout and names; whether the variant totals are maintained columns or a view.
- The exact SQL fragment shape for "stores I can act on".
- How the allocation queries are batched, within the three-quote cap.
- Whether `PATCH /stores` pickup fields keep editing the first branch long-term or are deprecated in 7e-2.
- Small error codes and copy, in the existing envelope and tone.
- Admin dashboard: show a store's branches and team read-only on the store detail page. Small, and staff need it to answer support questions; anything more waits.

## 12. Open questions for the owner

- None. **Answered 2026-11-25: the store's own team answers buyers in chat** (owner, managers and staff; branch-tied staff see chats about their branch's orders, plus product questions that are not tied to an order). Built in 7e-4 on this piece's roles; until then chat stays with the owner.

## 13. Data protection notes (for the DPO list)

- New personal data: team membership (who works where) and the last four digits of an invited phone. Minimal by design.
- With collect, a branch's pickup address and business phone are shown to buyers who paid for a collect order. It is business data, but a small seller's "branch" may be a home; the seller is told before turning collect on.
- More people can now see buyers' addresses and phones. Mitigations: role and branch scoping, the logged phone reveal, immediate removal.
- Retention of ended memberships and expired invites: proposed to keep membership history as long as the orders it touched (it explains the audit trail), and to delete expired or declined invites after 30 days. Needs the DPO's confirmation.

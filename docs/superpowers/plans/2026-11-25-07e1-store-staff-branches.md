# Part 7e-1: Store Staff and Branches (Server) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A store has a team (owner, managers, staff, optionally tied to a branch) and one or more branches with their own address and stock; checkout sends each store's goods from the right branch (or splits across up to three), and buyers can collect from a branch, while a one-branch, no-team store behaves exactly as today.

**Architecture:** Four new tables (`store_members`, `store_invites`, `store_branches`, `branch_stock`) and a migration that gives every store one branch holding its current pickup fields and stock. One access module replaces every `owner_id == caller` check. One stock module is the only writer of stock, keeping `branch_stock` (truth) and the variant totals in step. A pure allocation function picks branches; delivery options and checkout work on fulfilment groups (store, branch, lines). Collect is a third delivery method on the same order steps.

**Tech Stack:** Rust, axum 0.8, sqlx 0.8 on Postgres 17, the existing `FieldCipher`, `audit::record`, `notify::enqueue` and jobs loop. Dashboard (React) for the read-only admin view only.

**Spec:** `docs/superpowers/specs/2026-11-25-07e1-store-staff-branches-design.md`. Server conventions are 7a's (`docs/superpowers/plans/2026-11-20-07a-app-server-additions.md`, Global Constraints).

## Global Constraints

- 7a's conventions hold: errors are the `AppError` envelope, path ids through `parse_id`, every state change writes `audit_log` with ids only (never a phone, address or code), no network call inside a transaction, nothing personal in logs or URLs.
- **Lock order:** users, stores, **store_members**, store_plots, **store_branches**, products, variants, **branch_stock**, access_codes, checkouts, orders, disputes, shipments, payouts, then conversations, reviews, uploads.
- Roles: `manager` and `staff`; the owner is `stores.owner_id`, never a member row. Managers are always store-wide; staff are store-wide or tied to one branch.
- Caps: 20 open branches per store; 50 live members per store; 20 invites per store per rolling 24 hours; invites expire after 7 days; at most 3 courier quotes and at most 3 groups per store per delivery-options call.
- Non-member: the same 404 a stranger gets today. Member without the right: 403 `not_allowed`. Permissions are read from the database on every request; nothing cached in sessions.
- A cart line is never split across branches. Buyer-facing availability is the largest amount available at one open branch.
- No new disclosure of buyer contact: members inherit today's rule (none on courier orders; self-delivery only while `out_for_delivery`), limited to members who may act on that order. Collect orders: the branch sees the buyer's name, never the phone.
- Collect: fee 0, per-branch `collect_enabled` (owner, default off); auto-cancel with full refund and no strike 5 days after ready (`platform_settings.collect_window_days`, default 5); buyer reminders at day 1 and day 4.
- The two-store limit counts owned stores only. Accepting an invite never changes `is_seller`. A member cannot buy from their store.
- Tests run from `server/`. First `df -h` (run `~/.cargo/bin/cargo.exe clean` if under 20 GB free), `docker ps -a`, `docker start market-db` if stopped. Command: `DATABASE_URL=postgres://market:market@localhost:5433/market ~/.cargo/bin/cargo.exe test`. Never two `cargo test` at once. Run `cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings` before each commit.
- Commits: conventional messages; never a `Co-Authored-By` line, a "Generated with" footer, a session link or any AI attribution.

## Review Focus

1. **A removed member mid-flight.** A member removed between loading an order list and posting an order step gets 404 on the step; a staff member moved to another branch loses the old branch's orders on the next request. (Task 3)
2. **Stock racing across branches.** Two buyers checking out the last unit at one branch while the seller edits that branch's count; totals never drift from the branch sum and nothing goes negative. (Tasks 5, 8)
3. **Options gone stale.** Stock moves between delivery-options and checkout so a group no longer holds: 409 `allocation_changed`, nothing reserved, never a silent re-route. Also a branch closed in between. (Task 8)
4. **Invite probing.** Responses and timing-relevant work are the same for a phone with and without an account; an invite cannot be accepted by anyone but the phone's owner, including after that phone signs up later. (Task 4)
5. **An uncollected order racing a late collection.** The handover code entered while the auto-cancel job runs: exactly one wins; a collected order is never refunded, a refunded one never marked collected. (Task 10)

---

## File structure

```
server/
  migrations/0044_staff_branches.sql     tables, enum values, settings, data migration
  src/stores/access.rs                    Role, Action, Membership, require(), member SQL
  src/stores/team.rs                      invites and members (service + routes)
  src/stores/branches.rs                  branches (service + routes)
  src/stock.rs                            the only stock writer; availability SQL
  src/delivery/allocate.rs                pure allocation
  src/delivery/collect.rs                 collect option, ready, collected, expiry job
  (modified) stores/service.rs, products/service.rs, payout_accounts/service.rs,
  orders/{routes,views,service}.rs, reviews/service.rs, disputes/service.rs,
  delivery/{options,tracking,shipments,handover}.rs, checkout/service.rs,
  cart/service.rs, catalog/browse.rs, market/routes.rs, notify/{kinds,service}.rs,
  escrow/jobs.rs, admin/stores.rs
  tests/staff_access.rs, team.rs, branches.rs, branch_stock.rs, allocation.rs,
        checkout_groups.rs, collect.rs, staff_branches_migration.rs
dashboard/src/...                         store detail: branches and team, read-only
```

Chat (`src/chat/*`) is **not** changed: chat stays with the owner until 7e-4.

---

### Task 0: Branch and baseline

- [ ] **Step 1:** Commit this plan on `docs/spec-7e1-staff-branches` (`docs: add the part 7e-1 plan`). Create worktree `.claude/worktrees/staff-branches` on branch `feat/staff-branches` from that branch.
- [ ] **Step 2:** Check disk, start `market-db`, run the full server suite from the worktree's `server/`. Expected: all pass (1193 at the end of 7d). Stop and report if not.

### Task 1: Migration and data move

**Files:** Create `server/migrations/0044_staff_branches.sql`; test `server/tests/staff_branches_migration.rs`.

**Produces (schema):**
- `store_member_role` enum (`manager`, `staff`).
- `store_members (id, store_id, user_id, role, branch_id NULL, added_by, created_at, removed_at NULL, removed_by NULL)`; unique live row per (store_id, user_id) via a partial unique index `WHERE removed_at IS NULL`; check `role = 'staff' OR branch_id IS NULL`.
- `store_invites (id, store_id, phone_idx bytea, phone_last4 text CHECK ~ '^[0-9]{4}$', role, branch_id NULL, invited_by, created_at, expires_at, accepted_at, declined_at, revoked_at)`; index on `(phone_idx)` for live invites.
- `store_branches (id, store_id, name, pickup_address_enc, pickup_area_council, pickup_postcode, business_phone_enc, self_delivery_fee_kobo, pickup_courier_address_code, collect_enabled bool default false, position int, closed_at, created_at)`, same checks as the store columns they copy (0024).
- `branch_stock (branch_id, variant_id, stock, reserved_quantity, PRIMARY KEY (branch_id, variant_id))` with `0 <= reserved_quantity <= stock <= 100000`.
- `orders.branch_id`, `delivery_options.branch_id` (both NOT NULL after back-fill), `delivery_options.group_fingerprint text`.
- `ALTER TYPE delivery_method ADD VALUE 'collect'`.
- `platform_settings.collect_window_days int NOT NULL DEFAULT 5 CHECK (BETWEEN 1 AND 14)`.
- Notification `kind` check gains `store_invite`, `order_new` (seller side, see Task 9), `order_ready_for_collection`, `collect_reminder`, `collection_expired`.
- View `variant_availability (variant_id, total_available, max_available)` over `branch_stock` joined to open branches.
- Data: one branch per store (name = store name, position 0, pickup fields and courier code copied); one `branch_stock` row per variant at that branch with the variant's stock and reservations; `orders.branch_id` and `delivery_options.branch_id` set to that branch.

- [ ] **Step 1: Write the failing test** `migrated_store_has_one_branch_with_identical_stock`: in a fresh DB migrated to 0043, insert a store with pickup fields, a product with two variants (stock 7 and 3, reserved 2 and 0), a paid order and a delivery option; run 0044; assert one branch with the store's area council and postcode, `branch_stock` equals (7,2) and (3,0), order and option point at it, and `variant_availability.max_available` is 5 and 3. Use sqlx's migrator with a target version (`Migrator::run` over a filtered set) as the 7d migration tests do, or the builder's equivalent.
- [ ] **Step 2:** Run it; expect FAIL (no table).
- [ ] **Step 3:** Write the migration.
- [ ] **Step 4:** Run the test and the full suite; expect PASS.
- [ ] **Step 5:** Commit `feat: add store members, branches and branch stock tables`.

### Task 2: The access module

**Files:** Create `server/src/stores/access.rs`; modify `server/src/stores/mod.rs`; test `server/tests/staff_access.rs` (unit tests for `can` live in `access.rs`).

**Produces:**
```rust
pub enum Role { Owner, Manager, Staff }
pub enum Action {
    ManageTeam, ManageBranches, ManagePayout, DeleteStore, EditOwnerFields,
    EditStoreDetails, PublishProducts, EditLivePrices, DeleteProducts, AnswerDisputes, ReplyReviews,
    EditProducts, EditStock(Uuid /* branch */), ActOnOrder(Uuid /* branch */), ViewStore,
}
pub struct Membership { pub store_id: Uuid, pub user_id: Uuid, pub role: Role, pub branch_id: Option<Uuid> }
impl Membership {
    pub fn can(&self, action: Action) -> bool;
    pub fn covers_branch(&self, branch: Uuid) -> bool; // true unless staff tied to another branch
}
pub async fn membership(conn: &mut PgConnection, store_id: Uuid, user: Uuid) -> Result<Option<Membership>, AppError>;
/// 404 for non-members and deleted stores, 403 `not_allowed` when `!can(action)`.
pub async fn require(conn: &mut PgConnection, store_id: Uuid, user: Uuid, action: Action) -> Result<Membership, AppError>;
/// SQL boolean: the user at `$user_param` is the owner or a live member of store alias `s`.
pub fn is_member_sql(user_param: &str) -> String;
/// As above, and covers the branch in column `branch_col` (owner, manager, store-wide staff, or staff of that branch).
pub fn covers_branch_sql(user_param: &str, branch_col: &str) -> String;
```
The `can` table is the spec's section 5 table: Owner everything; Manager everything except ManageTeam, ManageBranches, ManagePayout, DeleteStore, EditOwnerFields; Staff only EditProducts, EditStock(b) and ActOnOrder(b) where `covers_branch(b)`, and ViewStore.

- [ ] **Step 1: Write failing unit tests** `can_matrix`: for each (role, branch tie) × each Action, assert the table above; staff tied to branch A: `EditStock(A)` true, `EditStock(B)` false.
- [ ] **Step 2: Write failing integration tests** `require_404_for_stranger_and_removed_member`, `require_403_for_staff_on_manager_action`, `removal_takes_effect_on_next_call` (insert member, `require` Ok, set `removed_at`, `require` 404).
- [ ] **Step 3:** Run; expect FAIL.
- [ ] **Step 4:** Implement.
- [ ] **Step 5:** Run; expect PASS. Commit `feat: add store membership checks`.

### Task 3: Replace every owner check

**Files:** Modify `stores/service.rs`, `products/service.rs`, `payout_accounts/service.rs`, `orders/{routes,views,service}.rs`, `reviews/service.rs`, `disputes/service.rs`, `delivery/tracking.rs`, `market/routes.rs`, `cart/service.rs`, `checkout/service.rs`, `delivery/options.rs`; test `server/tests/staff_access.rs`.

**Consumes:** Task 2. **Produces:** `GET /stores/mine` returns owned and member stores, each with `my_role` (`owner`|`manager`|`staff`) and `my_branch_id`; order step and list responses carry `done_by: {user_id, display_name}` per step where the audit has one.

Rules to apply (spec section 5):
- Every `owner_id = $caller` in the files above becomes `require(.., Action)` or `is_member_sql`/`covers_branch_sql`. Buyer-side "not your own store" checks (cart, checkout, delivery options) become "not a member".
- `PATCH /stores/{key}`: a body containing any of `pickup_*`, `self_delivery_fee_kobo`, `delivery_courier`, `delivery_self`, `business_phone`, `district` needs `EditOwnerFields`, else `EditStoreDetails`. Refused whole.
- Products: create and edit need `EditProducts`. On a **published** product, a change to any price, adding or removing a variant, or changing `is_published` needs `EditLivePrices`/`PublishProducts`; on an unpublished product, `EditProducts` suffices for prices and variants. Publishing always needs `PublishProducts`. Delete needs `DeleteProducts`.
- Orders: list and steps use `ActOnOrder(order.branch_id)`; staff tied to another branch get 404 for that order.
- Contact reveal: today's rule unchanged; viewer must cover the order's branch; `order.contact_revealed` audit now per (order, member).
- Disputes: `/response` needs `AnswerDisputes`; `/return-received` needs `ActOnOrder(branch)`. Reviews reply needs `ReplyReviews`. Payout account needs `ManagePayout`. Store delete needs `DeleteStore`. Room preview of a pending store: any member.
- Audit `actor_id` is the acting member on every seller write.

- [ ] **Step 1: Write the failing table-driven test** `seller_route_permission_matrix`: build one store with two branches (insert the second branch row directly), an owner, a manager, a store-wide staff, a staff at branch A, a staff at branch B, a removed member and a stranger; a published and an unpublished product; a paid order at branch A; a dispute and a review on it. For each (route, actor) assert the status from the rules above (2xx, 403 `not_allowed`, 404). Include: price change on published product by staff → 403; on unpublished → 200; publish by staff → 403; manager `PATCH` with `district` → 403, with `name` → 200; staff B on order A step → 404; `/return-received` by staff A → 200 (set the dispute into that state via SQL).
- [ ] **Step 2: Write failing tests** `member_cannot_buy_from_own_store`, `stores_mine_lists_member_stores_with_role`, `step_records_member_as_actor`, `contact_reveal_audited_per_member`, `member_never_sees_contact_on_courier_order`.
- [ ] **Step 3:** Run; expect FAIL.
- [ ] **Step 4:** Implement, file by file; after each file run the existing suite for that area.
- [ ] **Step 5:** Full suite PASS (existing tests unchanged). Commit `feat: let store members act by role`.

### Task 4: Team: invites and members

**Files:** Create `server/src/stores/team.rs`; modify `stores/mod.rs` (routes), `notify/kinds.rs` (`StoreInvite`), account deletion path (end memberships); test `server/tests/team.rs`.

**Consumes:** Task 2. **Produces routes:**
- `POST /stores/{key}/invites` `{phone, role, branch_id?}` → always `201 {id}` for a valid request; `ManageTeam`.
- `DELETE /stores/{key}/invites/{id}` (revoke); `ManageTeam`.
- `GET /me/invites` → `[{id, store: {id, name, slug, logo_url}, role, branch: {id, name}?, expires_at}]` for live invites matching the caller's `phone_idx`.
- `POST /me/invites/{id}/accept`, `POST /me/invites/{id}/decline` → 404 unless it matches the caller's phone and is live.
- `GET /stores/{key}/team` (owner, manager) → `{members: [{user_id, display_name, role, branch}], invites: [{id, phone_last4, role, branch, expires_at}]}`.
- `PATCH /stores/{key}/members/{user_id}` `{role?, branch_id?}`, `DELETE /stores/{key}/members/{user_id}`; `ManageTeam`.

Errors: 409 `already_member`, 422 `cannot_invite_self`, 422 `staff_only_branch` (manager with branch), 422 `branch_closed`, 409 `team_full` (50), 429 `too_many_invites` (20/24h). Audit: `store.invite_sent` (detail `{phone_last4, role}`), `store.invite_revoked`, `store.member_added`, `store.member_removed`, `store.member_role_changed`. An invite to a phone with an account enqueues `Kind::StoreInvite` to that user; no SMS ever. A jobs-loop job `purge_invites` deletes invites expired, declined or revoked more than 30 days ago (spec section 13; the DPO confirms the period before launch).

- [ ] **Step 1: Write failing tests:** `invite_same_response_for_known_and_unknown_phone` (status and body shape equal; only the known one gets a notification row), `invite_visible_after_later_sign_up`, `accept_by_other_user_is_404`, `expired_and_revoked_invites_are_404`, `accept_makes_member_not_seller` (`is_seller` unchanged; two-store limit unaffected), `already_member_409`, `team_cap_and_invite_rate_limit`, `remove_member_ends_access` (next order step 404), `deleted_account_ends_membership`, `audit_never_holds_full_phone`, `old_invites_purged_after_30_days`.
- [ ] **Step 2:** Run; FAIL. **Step 3:** Implement. **Step 4:** PASS. **Step 5:** Commit `feat: invite and manage store team members`.

### Task 5: The stock module

**Files:** Create `server/src/stock.rs`; modify every stock writer (`products/service.rs` variants PUT, `checkout/service.rs` reserve/commit/expire, refund and cancel paths in `escrow/jobs.rs`, `orders/service.rs`, `admin/money.rs`), and buyer readers (`products/service.rs` available_stock, `cart/service.rs`, `catalog/browse.rs`, `market/routes.rs`); test `server/tests/branch_stock.rs`.

**Produces:**
```rust
/// The only writer. Locks the variant row then the branch_stock row (lock order), applies
/// the deltas to both, and errors 409 `out_of_stock` if a check would fail.
pub async fn apply(conn: &mut PgConnection, branch: Uuid, variant: Uuid, d_stock: i32, d_reserved: i32) -> Result<(), AppError>;
pub async fn set_stock(conn: &mut PgConnection, branch: Uuid, variant: Uuid, stock: i32) -> Result<(), AppError>;
/// Buyer-facing: largest `stock - reserved` at one open branch.
pub const MAX_AVAILABLE_SQL: &str; // subquery fragment over variant_availability keyed by `v.id`
```
`PUT /products/{id}/variants` accepts `stock` as a number (one open branch only; else 422 `branch_required`) or `{branch_id: count}`. New `PUT /branches/{id}/stock` `{items: [{variant_id, stock}]}` needs `EditStock(branch)`. The seller's own product view shows per-branch counts and the total; buyers see `MAX_AVAILABLE_SQL`. Closing a branch (Task 6) removes its stock from totals via `apply`.

- [ ] **Step 1: Write failing tests:** `totals_equal_branch_sum_after_any_sequence` (a seeded random sequence of reserve, commit, expire, refund, cancel and set across two branches, 200 steps, checking the invariant after each), `buyer_sees_max_at_one_branch` (3 and 2 → product page 3, cart caps at 3), `plain_stock_refused_with_two_branches`, `staff_sets_stock_only_at_own_branch`, `never_negative_under_race` (two concurrent reserves of the last unit at one branch: one 409).
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** Full suite PASS. **Step 5:** Commit `feat: keep stock per branch`.

### Task 6: Branches

**Files:** Create `server/src/stores/branches.rs`; modify `stores/service.rs` (`POST /stores` creates branch 0; `PATCH` pickup fields edit branch at position 0), `delivery/options.rs` (courier address code cache per branch); test `server/tests/branches.rs`.

**Produces routes** (owner, `ManageBranches`; `GET` any member): `GET /stores/{key}/branches`, `POST /stores/{key}/branches` `{name, pickup_address, pickup_area_council, pickup_postcode?, business_phone, self_delivery_fee_kobo?, collect_enabled?}`, `PATCH /stores/{key}/branches/{id}`, `POST /stores/{key}/branches/{id}/close`. Responses to members carry the address and phone decrypted only for owner and managers; staff get name and area council. Errors: 409 `branch_limit` (20), 409 `branch_has_open_orders`, 409 `last_branch`. A moved address resets that branch's courier code. Audit `store.branch_added|updated|closed`.

- [ ] **Step 1: Write failing tests:** `create_store_makes_first_branch`, `patch_store_pickup_edits_first_branch`, `add_branch_and_cap`, `close_refused_with_open_orders_and_for_last_branch`, `closed_branch_stock_not_sold` (max_available drops), `staff_tied_to_closed_branch_loses_access`, `moved_branch_resets_courier_code`.
- [ ] **Step 2–4:** FAIL, implement, PASS. **Step 5:** Commit `feat: add store branches`.

### Task 7: Allocation

**Files:** Create `server/src/delivery/allocate.rs` (pure); test unit tests in the file plus `server/tests/allocation.rs` for the DB loader.

**Produces:**
```rust
pub struct Line { pub variant_id: Uuid, pub quantity: i32 }
pub struct Candidate { pub branch_id: Uuid, pub area: AreaCouncil, pub position: i32, pub available: HashMap<Uuid, i32> }
pub struct GroupPlan { pub branch_id: Uuid, pub lines: Vec<Line> }
pub struct Allocation {
    /// Full-fill case: up to 3 branches to quote, best first; caller picks by fee then date.
    pub full_fill: Vec<Uuid>,
    /// Split case (full_fill empty): up to 3 groups.
    pub groups: Vec<GroupPlan>,
    pub unavailable: Vec<Line>, // with quantity = what is missing
}
pub fn allocate(lines: &[Line], candidates: &[Candidate], buyer_area: AreaCouncil) -> Allocation;
pub fn group_fingerprint(branch: Uuid, lines: &[Line]) -> String; // sha256 hex of branch + sorted lines
pub async fn load_candidates(conn: &mut PgConnection, store: Uuid, variants: &[Uuid]) -> Result<Vec<Candidate>, AppError>;
```
Ranking: same area council as buyer first, then `position`. Split: repeatedly take the candidate filling the most remaining lines (ties by ranking); a line goes whole to one branch.

- [ ] **Step 1: Write failing unit tests:** `one_branch_fills_all`, `same_council_ranked_first`, `at_most_three_full_fill_candidates`, `split_across_two`, `split_capped_at_three_groups_rest_unavailable`, `line_never_split` (3 wanted, 2+2 at two branches → unavailable), `fingerprint_stable_under_line_order`.
- [ ] **Step 2–4:** FAIL, implement, PASS. **Step 5:** Commit `feat: choose branches for a cart`.

### Task 8: Delivery options and checkout by group

**Files:** Modify `delivery/options.rs`, `checkout/service.rs`, `delivery/shipments.rs`, `couriers` call sites, `tests/common/mod.rs` (`delivery_options`, `checkout_body` helpers), `tests/escrow.rs` and other tests using the old shapes; test `server/tests/checkout_groups.rs`.

**Consumes:** Tasks 5, 6, 7. **Produces:**
- `POST /checkout/delivery-options` → `{stores: [{store_id, groups: [{group_id, branch: {id, name, area_council}, lines: [{variant_id, quantity}], options: [...]}], unavailable: [{variant_id, wanted, available}]}]}`. `group_id` = `group_fingerprint`. Full-fill: quote each of `full_fill` (courier and self per branch), keep the branch with the cheapest option (then earliest promised date, then rank), return one group. Option rows store `branch_id` and `group_fingerprint`.
- `POST /checkout` `{address_id, groups: [{group_id, option_id}]}`. Under locks: the chosen groups must partition each store's cart lines exactly and every branch must still have stock for its lines, else 409 `allocation_changed`; any unavailable line, 409 `out_of_stock` (today's rule). One order per group with `branch_id`; reservations through `stock::apply` at that branch.
- Courier quote, booking, the moved-address check and self-delivery fee use the order's branch.

- [ ] **Step 1: Write failing tests:** `single_branch_store_one_group_same_totals_as_before`, `cheapest_of_three_branches_wins`, `split_makes_one_order_per_branch_one_payment`, `unavailable_lines_listed_and_checkout_refused`, `allocation_changed_when_stock_moves`, `allocation_changed_when_branch_closes`, `booking_uses_branch_address_code`, `self_delivery_fee_from_branch`, `quotes_capped_at_three_per_store` (count fake courier calls).
- [ ] **Step 2:** Update the shared helpers and existing tests to the new shape; run them; expect them to fail only on the shape.
- [ ] **Step 3:** Implement. **Step 4:** Full suite PASS. **Step 5:** Commit `feat: deliver each store's goods from the right branch`.

### Task 9: Team notifications

**Files:** Modify `notify/service.rs` (`tell_seller` becomes `tell_team(conn, order_id, audience)`), `notify/kinds.rs`, the callers in orders, escrow jobs, disputes, reviews, payout accounts; test additions in `server/tests/team.rs`.

**Produces:** `pub enum Audience { Order, Managers, OwnerOnly }`. `Order`: owner, managers, staff covering the order's branch. `Managers`: owner and managers (disputes, reviews). `OwnerOnly`: payout account and store status. New-order notice to the team uses `Kind::OrderNew` (the buyer keeps `OrderPaid`).

- [ ] **Step 1: Write failing tests:** `new_order_reaches_owner_managers_and_branch_staff_only`, `dispute_reaches_owner_and_managers`, `payout_notice_owner_only`, `removed_member_gets_nothing`.
- [ ] **Step 2–4:** FAIL, implement, PASS. **Step 5:** Commit `feat: notify the store team by role and branch`.

### Task 10: Collect from a branch

**Files:** Create `server/src/delivery/collect.rs`; modify `delivery/options.rs` (collect option per group when the branch has `collect_enabled`), `orders/routes.rs` (`/ready` and `/delivered` accept collect orders), `delivery/handover.rs` call site, `delivery/tracking.rs` (branch address and phone for a paid collect order's buyer), `escrow/jobs.rs` (new job `expire_collections`), `orders/views.rs` (no buyer phone on collect); test `server/tests/collect.rs`.

**Produces:** method `collect`, fee 0, promised date = ship-deadline date without courier pickup. Steps: `POST /orders/{id}/ready` sets `ready_at` and notifies the buyer (`OrderReadyForCollection`); `POST /orders/{id}/delivered` with `{code}` verifies the handover code (same lockout rules) and moves to `delivered`. Job `expire_collections`: orders with method `collect`, `ready_at` older than `collect_window_days`, still not delivered → full refund, stock back to that branch via `stock::apply`, no `seller_strikes` row, notify buyer (`CollectionExpired`) and team. Reminders (`CollectReminder`) at 1 and 4 days after ready, deduped. Audit `order.collected`, `order.collection_expired`.

- [ ] **Step 1: Write failing tests:** `collect_offered_only_when_branch_enabled`, `collect_fee_zero`, `address_and_phone_shown_to_buyer_only_after_payment`, `branch_never_sees_buyer_phone_on_collect`, `ready_then_collected_with_code`, `wrong_code_lockout_applies`, `expires_after_window_full_refund_no_strike_stock_back`, `reminders_day_1_and_4_once_each`, `late_collection_races_expiry_one_wins` (run the job and the delivered call concurrently; assert exactly one of refunded or delivered).
- [ ] **Step 2–4:** FAIL, implement, PASS. **Step 5:** Commit `feat: let buyers collect from a branch`.

### Task 11: Admin view

**Files:** Modify `server/src/admin/stores.rs` (store detail gains `branches: [{id, name, area_council, closed_at, collect_enabled}]` and `team: [{user_id, display_name, role, branch_id, removed_at}]`); the dashboard store detail page shows both as read-only tables; tests in the existing admin store test file and the dashboard's test for that page.

- [ ] **Step 1:** Failing tests: server detail includes branches and team, no addresses or phones; dashboard renders both, with empty states.
- [ ] **Step 2–4:** FAIL, implement, PASS (`npm test` and the dashboard lint in `dashboard/`). **Step 5:** Commit `feat: show branches and team on the admin store page`.

### Task 12: Records and the whole suite

- [ ] **Step 1:** Update `docs/superpowers/plans/2026-11-20-07-mobile-app.md` (sub-plan table: 7e-1 to 7e-4 as in the spec section 1, 7e-1 built) and the roadmap; add the new routes to the overview's route table; add to the go-live handbook list: nothing new to plug in (no new provider).
- [ ] **Step 2:** Full server suite, `fmt --check`, `clippy`, dashboard tests. Record the counts in the overview.
- [ ] **Step 3:** Commit `docs: record part 7e-1`.

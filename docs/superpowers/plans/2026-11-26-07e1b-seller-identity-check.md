# Part 7e-1b: Seller Identity Check (Server and Dashboard) Implementation Plan

**Goal:** A seller proves who they are once (NIN plus live selfie, through a provider behind a trait). A store cannot take orders until its owner passes; managers and staff cannot act on orders until they pass. One NIN is one account, and a banned person's NIN stays blocked. The NIN is never stored.

**Spec (owner-confirmed 2026-11-26, section 10 defaults stand):** `docs/superpowers/specs/2026-11-25-07e1b-seller-identity-check-design.md`. Builds on 7e-1 (`feat/staff-branches`, PR 22). Server conventions are 7a's.

## Global constraints

- No NIN, selfie, phone or verified name in logs, URLs, errors, audit rows or `Debug` output. The NIN is a `Nin` newtype with a redacted `Debug`, no `Display`, and lives in memory for one call. The verified name is stored only with `FieldCipher`.
- Audit rows carry ids, statuses and reason **codes** only. Owner-written review notes live on `identity_checks.decision_note`, never in `audit_log`.
- `NIN_INDEX_KEY` is its own key (differs from `FIELD_KEY` and `INDEX_KEY`). Required in production; optional elsewhere (starts then answer 503 `identity_unavailable`). Do not rotate it.
- The fake provider is refused in production. A provider name this build does not know is a config error.
- No network call inside a transaction. Lock order: users, stores, **identity_checks**, the rest.
- Tests: Postgres 17 on port 5433, whole suite from `server/`. Commits are conventional and carry no AI attribution.

## Calls made where the spec leaves room

1. **Name match is a check-level outcome** (spec section 5). A mismatch at result time gives `review` / `name_mismatch`. A payout account verified or changed later re-runs it; a mismatch flips the owner's passed check to `review` / `payout_name_mismatch`, which sends their stores to `pending` until the owner decides.
2. **Fingerprints.** `nin_idx` is stored on every attempt that reached the provider. A refused start (banned or in use elsewhere) stores **no** fingerprint, so probing a victim's NIN leaves nothing on the prober. A ban blocks the fingerprints of attempts with status other than `failed`. This is narrower than the spec's "every NIN ever attempted": a failed face match is most often someone trying another person's NIN, and blocking it would block the victim. Listed for legal review.
3. **Who holds a NIN.** Another account's `passed`, `revoked` or `review` row (except an `attempts_exhausted` review, which is unverified) makes the NIN unavailable. Enforced by a partial unique index as well as at start.
4. **Revoked users cannot restart** their own check; the owner handles them in the dashboard.
5. **Caps.** 3 starts per user per 24 hours (429 `identity_rate_limited`). The sixth attempt in total creates a `review` / `attempts_exhausted` row and answers 409 `identity_in_review`; the owner decides. Daily provider budget returns 503 `identity_unavailable` and writes an `identity.budget_exhausted` audit row for the alerts list.
6. **Refusal delay.** Refusals for banned or used NINs wait `platform_settings.identity_refusal_delay_ms` (default 2000) so timing matches a provider refusal. Tests set it to 0.
7. **Grace.** The migration stamps `stores.identity_due_at = now() + identity_grace_days` on every store that is `active` at that moment. A job moves stores past due with an unverified owner to `pending` (plot kept) and sends one reminder a week.
8. **Banning** also suspends the user's owned stores (`suspended_reason = 'user_banned'`); unbanning restores only those.

## Tasks (test first)

1. **Migration `0049_seller_identity.sql`:** `identity_checks`, `banned_nins`, `stores.identity_due_at`, settings columns, notice kinds `identity_passed`, `identity_declined`, `identity_due`.
2. **Pure code:** `Nin` (exactly 11 digits), name matching with initials, age from date of birth, keyed fingerprint. Unit tests.
3. **Provider seam and fake:** `IdentityProvider` trait, `FakeIdentity` (signed webhook, scripted outcomes, refuses production). Config and `AppState` wiring, `.env.example`.
4. **Start:** `POST /me/identity/start`, `GET /me/identity`. Caps, ban and duplicate refusal, budget, delay.
5. **Result:** `POST /webhooks/identity/{provider}`, poll job, stale-attempt expiry. Passed, failed, review rules. Duplicate and replayed deliveries change nothing.
6. **Effects:** store activation needs payout verified **and** owner clear (`payout_accounts::sync_store_status`, admin unsuspend); members refused with 403 `identity_required` on order actions and stock edits; notices.
7. **Payout re-check** on payout account verify or change.
8. **Bans:** ban blocks fingerprints and suspends stores; unban reverses.
9. **Admin:** per-user status and history (verified name to Owner only), review queue, decide pass or fail with a required reason, revoke with a required reason.
10. **Grace job** and expiry. Existing tests: `make_seller` passes identity so earlier suites keep their meaning.
11. **Dashboard:** identity panel on the user page, review queue page, ban confirmation notes the NIN block.
12. **Docs:** roadmap go-live items, DPO/legal list. Full CI-equivalent run.

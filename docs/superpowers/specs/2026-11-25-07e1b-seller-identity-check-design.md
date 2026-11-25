# Part 7e-1b: Seller identity check (NIN plus live selfie), design

Status: draft for owner review, 2026-11-25.

## 1. Intent

Owner, 2026-11-25: identity checks are there **to stop scams and fake stores, and so we can track culprits**. In practice, tracking means **a banned scammer cannot come back with a new account on the same NIN**.

Buyers' money already sits in escrow until delivery, so the risk is on the selling side. Sellers prove who they are once, with their NIN and a live selfie that a licensed provider matches against the NIMC photo. That stops people using a relative's or a stolen NIN.

## 2. Where it sits

| Part | What |
|---|---|
| 7e-1 | Store staff and branches (server), in progress |
| **7e-1b** | **This design: seller identity check (server and dashboard)** |
| 7e-2 | Selling screens (app). Its seller onboarding hosts the check's screens: the notice, NIN entry and the provider's selfie capture |

This part builds on 7e-1's roles: managers and staff verify too.

## 3. Decisions the owner made

- **Who:** sellers. Buyers are not asked.
- **How:** NIN plus a live selfie, through a NIMC-licensed provider (option 1 of 3. The others were number-only, where anyone who knows another person's details passes, and any document, which breaks the ban).
- **NIN only.** One person has one NIN but several passport and licence numbers, so accepting other documents would let a banned person return. This drops "any ID that shows you are Nigerian". A NIN does not prove citizenship, but citizenship is not what this check is for.
- **Ban by NIN.** When a user is banned, their NIN is blocked. Any later account that tries to verify with it is refused.
- **Never store the NIN.** We keep a keyed fingerprint (HMAC) that can recognise a NIN but cannot be read back. This goes slightly beyond the 2026-10-22 design ("never the number"), so it needs DPO review before go-live (section 9).

## 4. Goals, non-goals, anti-goals

**Goals**
1. A store cannot become `active` (take orders) until its owner has passed the check.
2. A manager or staff member cannot act on orders until they have passed the check. Owners keep managing their store meanwhile.
3. One NIN, one account: the same NIN cannot verify two accounts. This also stops anyone getting around the two-store limit with a second account.
4. A banned user's NIN is blocked. Unbanning lifts the block.
5. Each check is paid for only when it can succeed: ban and duplicate checks run on our side before the provider is called.
6. The provider can be swapped (a trait, like payments and couriers), with a fake provider for development and tests.

**Non-goals**
- Checking buyers.
- Proving citizenship.
- Any other document type.
- Storing selfies or NIMC photos. The provider's SDK captures the selfie and sends it straight to the provider, so our server never holds an image.
- App screens (they belong in 7e-2).

**Anti-goals (failure even if it works)**
- The NIN or selfie appears anywhere on our side: logs, audit rows, URLs, error messages, the database, backups.
- An honest seller is locked out with no way through. Every automatic refusal has a route to a human (the owner review queue).
- Responses that tell someone whether a given NIN is banned or already in use by another account (section 6).

## 5. How it works

**Flow (one attempt)**
1. The app shows the notice (section 8), and the seller agrees.
2. The app sends the NIN to `POST /me/identity/start` over TLS. The server:
   - validates the format: exactly 11 digits, nothing else. Anything longer or shorter, or with letters, is refused before any other work (owner, 2026-11-25). The app's NIN field (7e-2) accepts digits only and stops at 11, the same as the phone fields;
   - computes `nin_idx = HMAC(NIN_INDEX_KEY, nin)`, using a key of its own, separate from the phone `INDEX_KEY`;
   - refuses with no provider call if `nin_idx` is banned, or belongs to another account's passed check (both give the same response, see section 6);
   - checks the attempt caps;
   - creates a provider job, passing the NIN, and keeps the NIN in memory only for that call;
   - returns the provider's session token for the app's SDK.
3. The app runs the provider's SDK: liveness check and selfie capture, sent straight to the provider.
4. The provider reports the result by **signed webhook**, with replay protection like Mono's and Shipbubble's. A poll job backs it up for jobs with no webhook after a timeout.
5. The server records the outcome:
   - **passed** (the NIN is valid, the face matches and the person is alive);
   - **failed** (the NIN is not found, or the face does not match);
   - **review** (the provider is unsure, the verified name does not match the payout account name, or the seller is under 18).

**What is stored per user** (new table `identity_checks`: one row per attempt, plus a current-status view or column on users):
- status;
- provider name and the provider's job reference;
- `nin_idx` (on a passed check only, unique among passed checks);
- verified full name, encrypted with `FieldCipher`;
- date of birth year only, used for the 18+ rule (my call: the full date adds nothing to this purpose);
- timestamps, and who decided a review.

The provider's photo, address and other NIMC fields are never requested or kept, beyond what the provider returns by default and we discard.

**Name match with the payout account (my call).** The verified name is compared with the bank account name already checked for payouts. A clear match passes. A mismatch is sent to owner review, not refused: names differ in order, initials and spelling. This closes the gap of a verified person paying out to someone else's (mule) account.

**18 and over (my call).** A seller under 18 goes to review. It is the owner's decision whether minors may sell.

**Store activation.** Today a store turns `active` when its payout account is verified (`payout_accounts::sync_store_status`). It will need both: payout verified **and** owner identity passed. Losing either (account unverified, or identity revoked) sends it back to `pending`, keeping its market plot, as today.

**Members (7e-1).** The access module's `require()` refuses order actions (steps, handover, stock edits) to members whose check has not passed, with 403 `identity_required`. Viewing stays allowed, so they can see what is waiting.

**Bans.** The existing ban (`admin/moderation.rs` `ban`) also inserts the user's `nin_idx`, if any, into `banned_nins (nin_idx, user_id, banned_at, banned_by)`. Unban removes it. Banning also suspends the user's owned stores, using the existing suspension (my call: a banned scammer's store should not keep selling).

**Caps (cost control, my call; values in `platform_settings`):**
- 3 attempts per user per 24 hours;
- 5 attempts per user in total, after which the user goes to owner review, not refused for good;
- a per-day platform budget on provider calls, after which starts return 503 `identity_unavailable` and the owner is alerted.

**Existing sellers.** Stores that are active when this ships get a grace period (`platform_settings.identity_grace_days`, default 14). Their owners are notified. After the grace period, unverified stores go `pending`. The plot is kept, and open orders still complete.

**Provider seam.** An `IdentityProvider` trait (start job, parse webhook, fetch result) with a fake implementation:
- the fake refuses to start in production;
- a live provider key is refused outside production, as with Mono and Shipbubble;
- config names follow the existing pattern (`IDENTITY_PROVIDER`, `IDENTITY_API_KEY`, `IDENTITY_WEBHOOK_SECRET`, `NIN_INDEX_KEY`; the builder confirms the final names in `config.rs` and `.env.example`).

The provider is chosen when the owner opens an account. Candidates: Smile ID, Prembly, Dojah, YouVerify. Confirm NIMC licensing first.

**Dashboard.**
- Each user's verification status and history: status, dates, provider reference, the verified name to Owner role only.
- A review queue with pass/fail decisions and a required reason.
- Banning shows that the NIN will be blocked.
- Support role sees status only, never the name.

**Audit.** Every start, result, review decision, ban block and unblock writes `audit_log` with ids and status only.

**Fingerprints on every attempt (my call).** `nin_idx` is stored on every attempt row, not only passed ones. Uniqueness ("one NIN, one account") applies among passed checks only. A ban blocks **every** `nin_idx` the user has ever attempted, so a scammer banned before passing still cannot come back with that NIN. Note that an unverified seller cannot take orders anyway (store `pending`, members refused).

**Banned list keyed by fingerprint (my call).** `banned_nins` can hold several rows for one `nin_idx` (one per banned user). A NIN is blocked while any row for it exists. Unbanning a user removes only that user's rows.

**Revoking (my call).** The owner (Owner role) can revoke a passed check from the dashboard, with a required reason, when fraud is found after the fact. Revoking:
- returns the user to unverified;
- sends their owned stores to `pending` (the plot is kept, and open orders still complete, as for grace expiry);
- stops them acting as a member.

The fingerprint stays on the user's rows, so the NIN is not freed for another account. For a scammer, the right action is a ban, which also suspends their stores.

**Account erasure (my call).** When an unbanned account is erased (Part 8), its attempt rows and fingerprints are deleted, which frees the NIN. A banned account's fingerprints stay in `banned_nins` while the ban stands (section 9).

**One owner per store.** A store has exactly one owner (`stores.owner_id`), and 7e-1 adds no ownership transfer. "Owner passed" means that user. If transfer is ever added, the store re-checks against its new owner.

**Name match rule (my call).** Both names are lower-cased, stripped of punctuation and titles, and split into words. It is a match when every word of the shorter name appears in the longer one, in any order, with initials matching a word's first letter. Anything else goes to review.

The comparison runs in memory on the server, using the decrypted payout account name, whichever of the two (identity, payout account) is verified second. Nothing is written except the outcome. A payout account added or changed later re-runs the comparison, and a mismatch then sends the store to `pending` until reviewed.

**Grace clock (my call).** When this ships, each store that is `active` at that moment gets `identity_due_at = ship time + identity_grace_days`, stored per store. Stores that go active after ship get no grace period: they need the check first.

## 6. Not leaking whether a NIN is known

A start refused for a banned NIN, or for a NIN already passed on another account, returns the same 409 `identity_not_accepted` ("We could not verify this NIN. Contact support.") that a provider refusal produces.

To keep timing from telling them apart, the server holds a refusal until a typical provider round-trip has passed (a configured delay, about 2 seconds, my call). Every start counts against the attempt caps, whether or not it reached the provider. The caps apply the same to everyone, so they reveal nothing, and probing many NINs runs out after 3 a day.

The real reason goes to the audit log and the owner review queue. The app's refused state (7e-2) is one screen: "We could not verify this NIN" plus a contact-support action.

## 7. Constraints

- 7a's server conventions hold: `AppError` envelope, `parse_id`, audit with ids only, no network calls inside a transaction, nothing personal in logs or URLs. The NIN goes in a request body, never a path or query string.
- The NIN is never written to disk on our side. The HMAC uses its own key. Rotating that key means every passed seller re-verifies, so document it as a do-not-rotate key, backed up with the others.
- Provider calls get a timeout and a size cap. A provider outage leaves sellers `pending` with a clear message, never stuck mid-attempt (a stale attempt expires after 30 minutes, my call).
- Money is not involved, apart from the provider's per-check cost, which is metered for the budget.

## 8. The notice the seller sees (content for legal review)

Before the check, in plain words:
- why we ask (to stop scams and fake stores);
- that the NIN and selfie go to [provider] to check against NIMC records;
- that we keep the result, the verified name and a fingerprint of the NIN that cannot be turned back into the number;
- that the fingerprint is kept after an account is closed only if the account was banned for fraud;
- who to contact.

They agree before starting. The wording is drafted in 7e-2 and reviewed by legal.

## 9. Data protection (flag for DPO / legal review; not legal advice)

- **Lawful basis:** fraud prevention, as legitimate interest or contract, plus explicit agreement to biometric processing by the provider. A selfie face match is biometric data, a sensitive category under the NDPA. Needs DPO sign-off.
- **The NIN fingerprint** is personal data (pseudonymised, not anonymous). Retention to propose:
  - for passed, unbanned users, deleted when the account is erased (Part 8);
  - for banned users, kept while the ban stands, as a fraud-prevention record, with a review period the DPO sets.
- **The provider** is a data processor: a processing agreement is needed. If it processes outside Nigeria, NDPA cross-border transfer rules apply. Ask the provider where data is processed and how long it keeps the selfie.
- **NIMC's rules** on NIN use by licensed verifiers apply to the provider. Confirm with them what we may store.
- **A data protection impact assessment** is likely required (biometrics at scale). Recommend one before go-live.
- **Data-subject rights:** the user can see their status and verified name (access) and ask for review (rectification, through the queue). Erasure follows the retention above.

## 10. Owner decisions still open

None block the build. The defaults above stand unless changed:
- whether a seller under 18 may ever sell (default: review queue, owner decides each case);
- the grace period for existing sellers (default 14 days);
- the provider choice (an owner action; the build uses the fake provider).

## 11. Left to the builder

Table and column names, whether status lives on `users` or a view, the poll-job interval, the exact refusal delay, the review-queue UI layout, and error copy beyond `identity_not_accepted`.

# Part 8: Data rights and retention, design

Status: draft for owner approval, 2026-11-26. Nothing is planned or built until it is approved.
Builds on: parts 1 to 7e (every table that holds personal data). Read with spec section 13 of `2026-10-22-abuja-marketplace-design.md` and the "Plan 8" and "Carried forward" lists in `docs/superpowers/plans/2026-10-23-00-roadmap.md`.

## 1. Intent

A person can download their data and delete their account from the app, and the server removes personal data on a schedule instead of keeping it for ever. Spec section 13 says: users can download their data and request deletion; deletion is soft-delete followed by purge, except records that must be kept for financial or legal reasons. Apple and Google both require deletion from inside the app, so this part blocks launch.

Decisions made with the owner on 2026-11-26:

- Deletion is requested in the app with the password, has a **30-day cooling-off**, then a purge job erases the person.
- Deletion is **blocked** while any of these is open: an unfinished order or escrow money, an open dispute or return, an active store the person owns, or a refund or payout still moving.
- When someone is erased, the chat messages they wrote **stay for the other side** with the author shown as "Deleted user", until the chat retention window ends.
- Money and order records are kept **6 years** with the person unlinked. 6 years is a placeholder, not legal advice: it is an owner setting that the DPO and legal confirm before launch.
- An export is generated in the app on request, as one file, available for 7 days.
- Scope: server purge jobs and retention settings, the user data export, and the two app screens. A dashboard queue of requests and the privacy notice text are not in this part.

## 2. Goals, non-goals, anti-goals

**Goals**

- Retention windows are settings the owner can change within bounds, so the DPO's answer is a setting, not a release.
- Erasure removes personal data completely or unlinks it from the person. Nothing is half done: a request either finishes or is retried.
- A new table that references a user cannot be forgotten. A test fails until the table is listed in the erasure inventory (section 4).
- Nothing personal in logs, URLs, push text or audit rows. The export file is only ever downloaded by the person it belongs to.

**Non-goals**

- No dashboard queue and no privacy notice text here (the list of statements the notice must carry is in section 8).
- No Support-initiated erasure. A ban is not erasure, and banned people's NIN fingerprints stay (section 4).
- No photo bytes in the export in this version (their keys and dates are included).
- No change to what chat or reviews show while a person is not deleted.

**Anti-goals**

- No silent permanent deletion: the person confirms with the password and has 30 days to change their mind.
- No hard delete of a user row. The row stays as an empty shell so foreign keys and the ledger stay valid.
- No export or erasure that bypasses the same blockers or checks the rest of the app uses.

## 3. Deletion

1. **Request.** `POST /me/deletion` with the password. The account is disabled at once: all sessions end, nothing can be bought, sold or sent. The reply has the date the purge is due (30 days, a setting from 7 to 60).
2. **Blocked.** If anything in section 1's list is open, the answer is 409 `deletion_blocked` with a list of codes (`open_order`, `open_dispute`, `owns_store`, `money_in_flight`) and nothing changes. The app turns each code into plain words and what to do.
3. **Cooling-off.** Signing in works but shows one screen: "Your account will be deleted on {date}" with a **Keep my account** button. Every other route answers 403 `deletion_pending`, except the status route and sign-out. Keeping the account is `DELETE /me/deletion`.
4. **Purge.** A job erases accounts whose date has passed. It checks the blockers again first. If one has appeared (for example the other side opened a dispute on an old order), the account is not erased, the job tries again daily and writes an audit row without personal data.
5. **After.** The phone number is free to sign up again. The person cannot sign in. Orders, reviews and chats they were part of remain as described in section 4.

A person who is a store member (not the owner) leaves freely: the request ends their memberships. An owner must close the store or hand it over first (`owns_store`).

## 4. The erasure inventory

Each table that references a user, or holds personal data, has one fate. Retention windows are settings (section 6).

| Fate | Data |
|---|---|
| **Deleted** at purge | sessions, `otp_codes`, `password_reset_requests`, `sign_in_failures`, `redeem_attempts`, `push_devices`, `notifications`, `cart_items`, `delivery_options`, `buyer_addresses`, `uploads` (rows and the stored photos), `identity_checks` name and birth year, store invites sent to their number |
| **Anonymised** on the user row | `display_name` becomes "Deleted user", `phone_enc` and `phone_idx` are cleared (so the number can sign up again), the password hash is cleared, status becomes `erased` |
| **Unlinked, kept to the end of a window** | `orders`, ledger, `payouts`, refunds, `shipments`: kept 6 years with `delivery_address_enc`, courier phone and handover code cleared at purge (the order already sets `delivery_address_purged_at` for this) |
| **Kept for the other side, author unlinked** | `chat_messages` and `conversations` until the chat window ends; shown as "Deleted user"; `moderation_flags` originals and suggestions go with the same window |
| **Kept, never shown with a name** | `reviews` (the public list already hides the buyer), dispute statements until the records window |
| **Kept while a ban stands** | `banned_nins` fingerprints and the moderation strikes of a banned user. An unbanned user's fingerprint is deleted at purge |
| **Ended** | `store_members` (a SQL trigger already ends them and audits it) |
| **Never personal by design** | `audit_log` rows: they hold ids and codes, not names, phones, NINs, selfies or message text. Kept per the records window |

**Guard test.** A server test reads the schema, finds every column that references `users` or is named like personal data (`*_enc`, `phone*`, `display_name`, `body`), and fails if one is not in the inventory above. Adding a table without deciding its fate breaks the build.

## 5. Export

1. `POST /me/export` with the password. One request per 24 hours. A background job builds a single JSON file.
2. The file holds: the profile, addresses, orders and their lines, chats (the person's own messages and the other side's messages in their conversations), reviews, disputes and their statements, notices, the identity result state (not the NIN), store memberships, and the keys and dates of uploaded photos.
3. `GET /me/export` says `pending`, `ready` (with the time it expires) or `none`. `GET /me/export/file` returns the file only to the same person, with `Cache-Control: no-store`. The file is stored encrypted with the field key and deleted after 7 days.
4. The request, the build and the download write audit rows with the request id and nothing else.

For someone in the cooling-off period the export still works, so the person can take their data before it goes.

## 6. Retention settings

Added to `platform_settings` with bounds, editable by the Owner through the existing settings route. Defaults are placeholders for the DPO to set, not conclusions.

| Setting | Default | Notes |
|---|---|---|
| `chat_retention_months` | 12 | From the spec; longer while a dispute on the order is open |
| `records_retention_years` | 6 | Orders, ledger, payouts, refunds, audit, dispute statements |
| `delivery_address_retention_months` | 12 | After the order's final state; a cancelled order waits out the 48h reverify window |
| `notice_retention_months` | 6 | Inbox rows |
| `push_device_stale_days` | 90 | Devices not seen since |
| `attempt_retention_days` | 30 | Sign-in, redeem and identity attempt rows |
| `deletion_cooling_off_days` | 30 | 7 to 60 |

The export lifetime (7 days) and one export per day are fixed in code.

## 7. Screens (app)

In Account, a new **Privacy and data** page with two rows.

- **Download my data.** Explains what is in the file, asks for the password, then shows pending, ready (with the expiry) or the last result. Saving or sharing uses the system sheet. The file is not kept in the app.
- **Delete my account.** Explains what is erased, what is kept and why, and that there are 30 days to change your mind. Blockers are listed in plain words with what to finish. Then the password, then a final confirm button. The pending screen from section 3 follows.

All states are designed with the rest of the app's rules: 48 dp targets, 200% text, no colour as the only signal, plain wording, nothing personal in error text.

## 8. Data protection (for DPO and legal review, not legal advice)

- The retention defaults in section 6, especially the 6 years for money and order records, and whether any of them must be longer or may be shorter.
- Whether anonymised chat text, kept for the other side until the window ends, is acceptable, and whether dispute statements need the same window as records.
- The privacy notice must say: what is collected, how long each kind is kept, that a store's team may answer buyers, that chats are checked by an AI provider, how to download and delete, and that some records are kept after deletion and why.
- Whether the banned-NIN fingerprint's retention while a ban stands has a review period.
- The identity provider's and the object store's own retention of what they hold (selfie, photos), which this part cannot reach.
- The 72-hour breach notification procedure is separate from this part and is still an owner action.

## 9. Open questions (defaults stand unless changed)

- Should the export include the photo files, not just their keys and dates? Default: no, for size.
- Should a person who is only a store member be able to export store-side data (orders they handled)? Default: only their own account data.
- Should a person be told by SMS that their deletion is due in a few days? Default: no SMS; the app shows it when they sign in, and a notice is created.

## 10. Structure and testing (left to the plan)

- Server: a `privacy` module (deletion requests, blockers, purge, export, retention jobs); migrations for `deletion_requests`, `data_exports` and the new settings. The purge job runs in the existing jobs tick.
- App: `features/privacy/` with the repository, controller and two pages; a `deletion_pending` gate in the router.
- Tests: every blocker; the cooling-off state and the 403 gate; restoring; the purge of each table in the inventory with a person who has data in all of them; the guard test; the export's contents and who may download it; the retention jobs at their boundaries; a real-server wire check of request, status and cancel.
- Checks: all CI jobs as for every part (fmt, clippy `-D warnings`, the whole server suite, `cargo audit`, dashboard lint, test and build if touched, app format, analyze and test).

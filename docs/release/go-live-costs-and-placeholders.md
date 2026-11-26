# Go-live: what to pay for, and what to replace

Written 2026-11-26, when the roadmap's build parts (7a to 9) were done. **This is a checklist from the code and the plans, not advice, and it carries no prices except the two store fees. Prices change; get a current quote from each provider.** Legal, tax and data-protection items are flagged for your lawyer, accountant and DPO.

## Read this first: two things block a production server

1. **A real identity-provider adapter does not exist.** The only provider the server knows is `fake` (`KNOWN_IDENTITY_PROVIDERS` in `server/src/config.rs`), and the server **refuses to run with `APP_ENV=production` without a real one**. The app's selfie step also has no provider SDK plugged in (`UnavailableSelfieCapture`). Choosing a licensed NIN-and-selfie verification provider, and building its adapter and the app's SDK step, is the biggest unbuilt item.
2. **Seven things the production server will not start without:** a payment provider (Mono), a tone-check key (Anthropic), push (Firebase), photo storage (S3-compatible), an SMS sender (Termii), the identity provider above, and `NIN_INDEX_KEY`. Each has an `.env.example` entry.

So "live testing" has two levels. **Level 1** is a closed test on a staging server with the fake providers (`APP_ENV=development`): near-zero provider cost, no real money, good for phones and testers. **Level 2** is real: real SMS, real payments, real identity checks, real stores. Everything below says which level needs it.

---

## Part A: what to pay for

### Needed for Level 1 (closed test with the fake providers, no real money)

| What | Why | Kind | Note |
|---|---|---|---|
| A server to run the Rust API and a Postgres 17 database | the staging backend; phones and the dashboard talk to it | monthly | a small VPS or a managed database; needs https |
| A domain name and an https certificate | the app refuses non-https addresses in a release build | yearly (certificates are usually free) | also for the dashboard |
| Google Play developer account | Play internal testing | one-time (about US$25, confirm) | Android |
| Apple Developer Program | TestFlight | yearly (about US$99, confirm) | iPhone |
| GitHub Actions minutes | the iOS build runs on a macOS runner, billed at a higher rate than Linux | usage | the Android build is cheap; check your plan |
| A mid-range Android phone and an iPhone | the device checklist | one-time | you already have an Android phone |
| Mono sandbox account | checkout needs a payment provider even for review | free sandbox | the server has no payment provider of its own |
| Firebase project | push notices | free tier | needed for the push checks |

### Added for Level 2 (real money, real people)

| What | Why | Kind | Note |
|---|---|---|---|
| **Mono live account** (DirectPay and Disburse) | collect payments, hold them, pay sellers out | per transaction, plus setup | Disburse needs a source bank account approved by mandate; confirm the amount units in writing |
| **Business bank account** | the account Mono debits and credits | bank fees | the escrow arrangement is regulated: see legal |
| **Termii (SMS)** | sign-up and reset codes | per SMS | register a sender ID early: approval can take weeks |
| **Identity provider** (NIN and selfie) | verify every seller and team member | per check, usually | not chosen; see "blocks" above |
| **Shipbubble** (courier) | book couriers for delivery orders | wallet funded in advance, then per shipment | production needs the key and a funded wallet |
| **Anthropic API** | the chat tone check (`claude-haiku-5-5`) | per use | get zero data retention and a no-training statement in writing |
| **Object storage and a CDN** (S3-compatible) | product, review and dispute photos, and the market art | per GB stored and served | also serves `market/<version>/` |
| **Firebase** | push delivery (FCM) | free tier | the iOS key comes from your Apple account |
| **Production hosting, backups, monitoring** | the live API, the dashboard (one origin behind a reverse proxy) | monthly | automated database backups and a restore test are yours to set up; no monitoring or alerting exists yet |
| **The market art** | real renders to replace the placeholder | one-time | Unreal work you offered, or hire an artist; renders must not show real people |
| **A support channel** | a support email and someone to answer it | monthly | the app shows `APP_SUPPORT_EMAIL` |
| **Crash reporting** (optional) | see crashes from real phones | free tier or monthly | not chosen; needs a DPO look |

### Legal, tax and compliance (paid professional time, before real money)

- **A lawyer: the escrow arrangement.** Holding funds for others is regulated by the CBN. Have the arrangement, the terms of service and the marketplace's role reviewed, and the exact words used in the store listing ("held until delivery").
- **A data-protection review (DPO and lawyer):** the privacy policy and in-app notice, the retention windows, transfers outside Nigeria (the AI provider, Firebase and possibly the object store), a data-processing agreement with each processor, whether the controller must register with the Nigeria Data Protection Commission and file annual compliance reports, and the 72-hour breach-notification procedure (not written yet).
- **An accountant:** VAT and stamp duty on fees (the server charges a stamp duty line), what the platform cut is taxed as, and whether related-party rules apply to the partner company because you belong to it.
- **Terms of service** and a **refund and dispute policy** written in plain language (the in-app flows exist; the written terms do not).

### Later, optional

- **The partner company's pricing service:** per call or per month, with the monthly call budget you set. Nothing is called until a contract exists.
- **A second payment provider (Paystack),** a postcode data feed, a **web page for account deletion** (the stores require one), and a staging server kept running for reviewers.

---

## Part B: placeholders to replace

### Name, brand and store

| Placeholder | Where | Replace with |
|---|---|---|
| Product name "Market" (display name) and the wordmark `MARKET` | `app/brand.json`, `app/lib/core/strings.dart` (`wordmark`), the splash page | the real name; then `dart run tool/apply_brand.dart` |
| "Sell on Abuja Market" | `app/lib/features/sell/sell_strings.dart` | the real name (it is hard-coded text) |
| Android application id and iOS bundle id `ng.market.market` | `app/brand.json` | your real ids. **Cannot change after the first store upload** |
| The placeholder icon (a gold M on dusk) | `app/assets/brand/icon.png` | the real logo, square, at least 1024 px |
| `{{APP_NAME}}`, short and long descriptions, keywords | `docs/release/store-listing.md` | final text; real screenshots (not of placeholder art) |
| Demo account phones and passwords | `docs/release/review-notes.md` | accounts on the staging server |
| Support email, privacy URL, terms URL | `APP_SUPPORT_EMAIL`, `APP_PRIVACY_URL`, `APP_TERMS_URL` | real addresses; both stores need the privacy URL |
| The dashboard's name and look | `dashboard/` | the real name (it says "Market" in places) |

### Art

| Placeholder | Where | Replace with |
|---|---|---|
| The placeholder market (flat roofs, a seeded generator) | `app/assets/market/` | the real art, published with `server art ...` (`server/market/README.md`) |
| The layout (60 plots in 5 districts) and the three room types | `server/market/` and the app's bundled copy | the pack's `layout.json` and `rooms/*/room.json`, in both places, in the same release |
| `MARKET_ASSETS_VERSION` and `MARKET_ASSETS_BASE_URL` | server settings | the published version and its public address |

### Server settings (`server/.env.example` lists every one)

| Setting | Replace with |
|---|---|
| `APP_ENV` | `production` (and `OTP_DEV_ECHO=0`; the server refuses the echo outside development) |
| `FIELD_KEY`, `INDEX_KEY`, `NIN_INDEX_KEY` | three different, random, 32-byte keys, kept in a secret store. **Losing `FIELD_KEY` makes encrypted data unreadable; back it up** |
| `DATABASE_URL` | the production database |
| `MONO_SECRET_KEY` (a `live_sk_` key in production), `MONO_WEBHOOK_SECRET`, `MONO_DISBURSE_*`, `MONO_PAYOUT_RECIPIENT_EMAIL`, `PUBLIC_REDIRECT_URL` | from your Mono account; the redirect page must exist |
| `SHIPBUBBLE_API_KEY`, `SHIPBUBBLE_CONTACT_EMAIL` | from Shipbubble |
| `ANTHROPIC_API_KEY`, `MODERATION_MODEL` | from Anthropic |
| `FCM_SERVICE_ACCOUNT_FILE` | the Firebase service account, kept outside the repository |
| `S3_*`, `UPLOADS_PUBLIC_BASE_URL` | the object store and its public address |
| `SMS_PROVIDER`, `TERMII_API_KEY`, `TERMII_SENDER_ID` | Termii, with an approved sender ID |
| `IDENTITY_PROVIDER`, `IDENTITY_API_KEY`, `IDENTITY_WEBHOOK_SECRET` | a real provider (needs its adapter built first) |
| `APP_MIN_VERSION_ANDROID`, `APP_MIN_VERSION_IOS`, `APP_MAINTENANCE_MESSAGE` | set when you need to force an update |
| `PARTNER_*` | only when the partner's contract exists |

### Business defaults the Owner confirms (dashboard, Settings)

These are placeholders from the build, not decisions: **platform cut 5%**, **collection fee ₦35 (₦50 above ₦50,000)**, **stamp duty ₦50 on orders over ₦10,000**, checkout expires after 30 minutes, sellers have 48 hours to ship, auto-release 72 hours after delivery, backstop 30 days, payout settlement buffer 2 hours, **three strikes in 30 days mute a person**. The data-retention windows (chat 12 months, records 6 years, addresses 12 months, notices 6 months, deletion cooling-off 30 days) are placeholders for the DPO. Per-day and per-minute limits in `server/src/limits.rs` (for example 20 new conversations a day) are starting values.

### Stubs: not placeholders, things that do not exist yet

- **The real identity-provider adapter and the app's selfie SDK step** (see the top of this page).
- **The NIPOST postcode check:** `ManualPostcode` accepts what people type until NIPOST publishes a usable source.
- **The provider's daily statement** for reconciliation: `statements` is `None` until Mono's statement endpoint is confirmed.
- **Firebase files and the Android Google services Gradle plugin:** push is off until they are added (`docs/release/release-howto.md`).
- **The store link in the app's update notice,** deep links and the app's link domain, a **web page for account deletion,** the **breach-notification procedure,** and **key rotation** (today: change the setting and restart).
- **Crash reporting,** and **monitoring and alerting** for the server.
- **The release workflow has never run,** and nothing has been run on an iPhone.

---

## Suggested order

1. Decide the name and the ids; replace the logo.
2. Stand up staging (server, database, domain, https, Mono sandbox, Firebase); run the device checklist on your Android phone and an iPhone.
3. Choose the identity provider; build its adapter and the app step.
4. Lawyer, DPO and accountant reviews, in parallel, from the start: they take the longest.
5. Open the Play and Apple accounts and the Mono, Termii and Shipbubble live accounts; register the SMS sender ID early.
6. Replace the art; publish it; verify it.
7. Fill the store forms from the drafts; release to internal testing; then a small, real, supervised pilot.

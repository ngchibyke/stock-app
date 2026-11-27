# Go-live: what to pay for, and what to replace

Written 2026-11-26, when the roadmap's build parts (7a to 9) were done. **This is a checklist from the code and the plans, not advice, and it carries no prices except the two store fees. Prices change; get a current quote from each provider.** Legal, tax and data-protection items are flagged for your lawyer, accountant and DPO.

## Read this first: two things block a production server

1. **The identity provider is Smile ID, built but not yet tried against a real account.** The server adapter exists (`server/src/identity/smile.rs`, `IDENTITY_PROVIDER=smile_id`) and is tested against a mock. The app's selfie step still has no SDK plugged in (`UnavailableSelfieCapture`), so no seller can be verified in the app yet. See "Setting up Smile ID" below for what to do and what is unconfirmed.
2. **Seven things the production server will not start without:** a payment provider (**Paystack**, or Mono), a tone-check key (Anthropic), push (Firebase), photo storage (S3-compatible), an SMS sender (Termii), the identity provider above, and `NIN_INDEX_KEY`. Each has an `.env.example` entry.

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
| **Paystack test account** | checkout needs a payment provider even for review | free: test keys (`sk_test_...`) move no real money | the Paystack adapter is built but has not been tried against a real or sandbox account |
| Firebase project | push notices | free tier | needed for the push checks |

### Added for Level 2 (real money, real people)

| What | Why | Kind | Note |
|---|---|---|---|
| **Paystack live account** | collect payments (Checkout), pay sellers out (Transfers) and refund | a fee per transaction and per transfer; get the current rates and who bears each | needs business verification (registration documents, director ID, a settlement bank account). **Ask Paystack in writing whether a marketplace that holds money until the buyer confirms delivery is allowed**, and fund the transfer balance. See "Setting up Paystack" below |
| **Business bank account** | the account Paystack settles to | bank fees | the escrow arrangement is regulated: see legal |
| **Termii (SMS)** | sign-up and reset codes | per SMS | register a sender ID early: approval can take weeks |
| **Identity provider: Smile ID** (NIN and selfie) | verify every seller and team member | per check; **price not confirmed**, ask Smile ID | adapter built, account needed; see "Setting up Smile ID" |
| **Shipbubble** (courier) | book couriers for delivery orders | wallet funded in advance, then per shipment | production needs the key and a funded wallet |
| **Anthropic API** | the chat tone check (`claude-haiku-5-5`) | per use | get zero data retention and a no-training statement in writing |
| **Object storage and a CDN** (S3-compatible) | product, review and dispute photos, and the market art | per GB stored and served | also serves `market/<version>/` |
| **Firebase** | push delivery (FCM) | free tier | the iOS key comes from your Apple account |
| **Production hosting and backups** | the live API, the dashboard (one origin behind a reverse proxy) | monthly | automated database backups and a restore test are yours to set up |
| **An uptime monitor** | polls `/ready` and pages you when the server or its jobs stop (`docs/release/operations.md`) | free tier or monthly | the server's side is built; the subscription and the paging are yours |
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
- **A second payment provider (Paystack),** a postcode data feed, and a staging server kept running for reviewers.

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
| `PAYSTACK_SECRET_KEY` (an `sk_live_` key in production, `sk_test_` anywhere else), `PAYSTACK_PAYER_EMAIL_DOMAIN`, `PUBLIC_REDIRECT_URL` | from your Paystack account. The payer email domain is one you own (buyers have no email; Paystack needs one, so a synthetic address on that domain is sent, and no name). The redirect page must exist. The webhook is verified with the same secret key. `PAYSTACK_BASE_URL` is optional |
| `MONO_*` | only if you also keep Mono; leave them empty for Paystack alone |
| `SHIPBUBBLE_API_KEY`, `SHIPBUBBLE_CONTACT_EMAIL` | from Shipbubble |
| `ANTHROPIC_API_KEY`, `MODERATION_MODEL` | from Anthropic |
| `FCM_SERVICE_ACCOUNT_FILE` | the Firebase service account, kept outside the repository |
| `S3_*`, `UPLOADS_PUBLIC_BASE_URL` | the object store and its public address |
| `SMS_PROVIDER`, `TERMII_API_KEY`, `TERMII_SENDER_ID` | Termii, with an approved sender ID |
| `IDENTITY_PROVIDER=smile_id`, `SMILE_ID_PARTNER_ID`, `SMILE_ID_API_KEY`, `SMILE_ID_CALLBACK_URL`, `SMILE_ID_PRIVACY_URL`, optional `SMILE_ID_BASE_URL` | Smile ID (see "Setting up Smile ID") |
| `APP_MIN_VERSION_ANDROID`, `APP_MIN_VERSION_IOS`, `APP_MAINTENANCE_MESSAGE` | set when you need to force an update |
| `APP_STORE_URL_ANDROID`, `APP_STORE_URL_IOS` | the store pages, after the first store release; the "please update" notice opens them |
| `PARTNER_*` | only when the partner's contract exists |

### Business defaults the Owner confirms (dashboard, Settings)

These are placeholders from the build, not decisions: **platform cut 5%**, **collection fee ₦35 (₦50 above ₦50,000)**, **stamp duty ₦50 on orders over ₦10,000**, checkout expires after 30 minutes, sellers have 48 hours to ship, auto-release 72 hours after delivery, backstop 30 days, payout settlement buffer 2 hours, **three strikes in 30 days mute a person**. The data-retention windows (chat 12 months, records 6 years, addresses 12 months, notices 6 months, deletion cooling-off 30 days) are placeholders for the DPO. Per-day and per-minute limits in `server/src/limits.rs` (for example 20 new conversations a day) are starting values.

### Stubs: not placeholders, things that do not exist yet

- **The app's Smile ID SDK step** (`usesmileid` Flutter package) and a real-sandbox run of the adapter (see "Setting up Smile ID").
- **The NIPOST postcode check:** `ManualPostcode` accepts what people type until NIPOST publishes a usable source.
- **The provider's daily statement** for reconciliation: `statements` is `None` until a provider's statement endpoint is confirmed. **Paystack refund webhooks** are not handled either (a refund stays "processing" until the owner marks it, as with Mono).
- **Firebase files and the Android Google services Gradle plugin:** push is off until they are added (`docs/release/release-howto.md`).
- **Deep links and the app's link domain** (they need the final domain and name), and **key rotation** (today: change the setting and restart; the three data keys cannot be rotated in place).
- **Crash reporting** (your choice, with a DPO look).
- **An external uptime monitor and paging** (the checks exist, see below).
- **The release workflow has never run,** and nothing has been run on an iPhone.

---

## Built since the first version of this page (2026-11-27)

| Was unbuilt | Now | What is still yours |
|---|---|---|
| A store link for the update notice | `APP_STORE_URL_ANDROID` and `APP_STORE_URL_IOS` in the server's settings; the app's "Please update" notice has an **Update now** button when its platform's page is named | set them after the first store release |
| A web page for account deletion (both stores ask for one) | `https://<api host>/account-deletion`: a person who cannot open the app signs in and asks for deletion, or keeps the account; strict content-security policy, nothing stored in the browser; tested in a real browser | give the stores that address; the support email shows on it when `APP_SUPPORT_EMAIL` is set |
| A breach-notification procedure | `docs/release/breach-procedure.md`: roles, the 72-hour clock, containment, assessment, notices and a register | **fill in the names, have the DPO and a lawyer adopt it**, and run an exercise |
| A second payment provider | **Paystack**: collection, payouts, refunds, webhooks and a hosted checkout page (`paystack_checkout`); production accepts Mono or Paystack | every step in "Setting up Paystack": none has been run against a real account |
| Monitoring and alerting basics | `GET /ready` (database and background jobs) and `docs/release/operations.md` with a runbook | subscribe an uptime monitor to `/ready`, set up database backups and a restore test, and watch the dashboard's Alerts |

## Setting up Paystack

The server supports Paystack as of 2026-11-27 (the plan is `docs/superpowers/plans/2026-11-27-paystack-adapter.md`). **It has been tested only against a mock of Paystack's documented API, never against a real or sandbox account.** The plan lists every Paystack fact it relies on, and which are unconfirmed. Do this in order:

1. Create the Paystack account and take the **test keys**. Set `PAYSTACK_SECRET_KEY=sk_test_...` and `PAYSTACK_PAYER_EMAIL_DOMAIN` (a domain you own) on the staging server.
2. In the Paystack dashboard set the **webhook URL** to `https://<api host>/webhooks/paystack`. The server checks each call's `x-paystack-signature` against the secret key.
3. In the server's dashboard Settings choose **`paystack_checkout`** as the payment method and **`paystack`** as the payout provider (the defaults are Mono's names, so this step is needed).
4. In the Paystack dashboard **turn off the OTP requirement for transfers** made with the API, and enable transfers. Otherwise every payout waits at "otp" for a person to type a code. If your account requires the OTP, the server can finish a transfer with it, but nothing asks for it automatically.
5. Run the sandbox smoke test: a test payment from the app (the page opens on `checkout.paystack.com`, the order turns paid), a refund, and a payout to a test bank account. **Confirm in the real answers the shapes the plan marks unconfirmed:** the transfer fee field, the refund listing, the bank-name lookup.
6. For live: business verification, `sk_live_` key, fund the transfer balance, repeat step 5 with a very small real payment to an account you own, and check the settlement and the fees against your bank statement.

**Ask Paystack, in writing:** is a marketplace that holds money until delivery allowed, can they pay many sellers (Transfers or split payments) and at what cost, what do refunds cost, what are the settlement time and limits, is there a daily statement to reconcile against.

## Suggested order

1. Decide the name and the ids; replace the logo.
2. Stand up staging (server, database, domain, https, Paystack test keys, Firebase); run the device checklist on your Android phone and an iPhone.
3. Open a Smile ID account, get NIMC `enterprise_id` registered in its dashboard, run the sandbox, then build the app's SDK step.
4. Lawyer, DPO and accountant reviews, in parallel, from the start: they take the longest.
5. Open the Play and Apple accounts and the Paystack, Termii and Shipbubble live accounts; register the SMS sender ID early.
6. Replace the art; publish it; verify it.
7. Fill the store forms from the drafts; release to internal testing; then a small, real, supervised pilot.


## Setting up Smile ID

Built from Smile ID's public v3 documentation (docs.usesmileid.com, read 2026-11-27); not yet run against a real account. The seller types the NIN in the app; the server asks Smile ID for a short-lived token bound to that NIN and the seller's name; the app's Smile ID SDK sends the selfie straight to Smile ID (it never reaches our server); Smile ID calls our webhook with the verdict, and we keep only a keyed NIN fingerprint, the verified name (encrypted) and the year of birth.

**What you do**
1. Open a Smile ID account and ask for the price per Biometric KYC check, in writing, along with a data-processing agreement and where NINs and selfies are stored and for how long.
2. Register your **NIMC `enterprise_id`** in the Smile ID dashboard. Smile ID's docs say the `NIN_V2` type needs one. Without it Nigerian NIN checks will not run.
3. Register the webhook domain for sandbox and production (Smile ID checks callback domains), and set `SMILE_ID_CALLBACK_URL` to `https://<api host>/webhooks/identity/smile_id`.
4. Set the five `SMILE_ID_*` values in `server/.env.example`'s style. Sandbox keys and live keys are different; production refuses a sandbox URL.

**Unconfirmed: settle these in the sandbox before going live**
- The token request's `payload` field is sent as a JSON string in the multipart form; its exact shape is not in the docs I could read.
- Whether Smile ID accepts our own `user_id` (we use `user_<attempt id>`) and echoes it in the webhook's `partner_params`. The webhook match depends on this.
- The webhook signature: HMAC-SHA256 under the API key over timestamp + partner id + `sid_request`, base64. The docs give no separator; none is used. A wrong guess shows as every webhook being refused.
- Biometric KYC lists `user_details` with an email or phone as required. We send the name only.
- `NIN_V2` against `NIN_SLIP`, and virtual NINs (`V_NIN`, 16 characters), which the app does not accept yet.
- The status endpoint needs a job id only the webhook carries, so the poll backup cannot ask for a result. A missed webhook means the attempt times out and the seller tries again.
- Whether the SDK needs the NIN passed on the phone as well as in the token. If it does, the app keeps it in memory only for the capture.

**Still to build:** the app step. Add the `usesmileid` package (needs Flutter 3.44, Dart 3.12), implement `SelfieCapture` with `UseSmileIDBuilder` using the `sdk` block the start call now returns (`partner_id`, `id_type`, `country`, `sandbox`) and the session token, and test it on a device with the sandbox.

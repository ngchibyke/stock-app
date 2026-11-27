# Paystack payment provider, plan

Decided 2026-11-27: the owner will use **Paystack** (the roadmap listed it as a later second checkout option; the server so far had only Mono). Branch: `feat/paystack`. The `PaymentProvider` trait already fits (it even has `finalize_payout` for Paystack's transfer OTP), so this adds an adapter and its wiring, not a new design. Mono stays; either or both can be configured.

## What it does

- **Collection:** Paystack Checkout (`POST /transaction/initialize`, a hosted page the app's webview opens, `GET /transaction/verify/:reference`). Method name `paystack_checkout`, payment-page hosts `paystack.com` and `paystack.co`.
- **Seller payouts:** Paystack Transfers: create a `nuban` transfer recipient, then a transfer from the balance; status by `GET /transfer/verify/:reference`; the optional OTP step via `finalize_transfer`.
- **Refunds:** `POST /refund` on the transaction reference.
- **Account name check:** `GET /bank/resolve`.
- **Webhooks:** `x-paystack-signature`, an HMAC-SHA512 of the raw body under the secret key. `charge.success` triggers a verify; `transfer.success`, `transfer.failed` and `transfer.reversed` settle a payout (which re-fetches the status, as for Mono). Everything else is ignored.

## Rules kept (same as the other adapters)

Request and response bodies never logged; the secret key in `Secret`; https only, no redirects, timeouts; a write is sent once (no retry) and a failure after sending is `Timeout`, not "not done"; an unknown status is `Pending`, never terminal; a payout is looked up by reference before being created again; amounts are integer kobo (Paystack's unit is already kobo); data sent is minimised.

## Paystack facts relied on (from Paystack's public documentation, to be confirmed against the account and sandbox before real money)

| Fact | Used for | Status |
|---|---|---|
| Base `https://api.paystack.co`, `Authorization: Bearer sk_...`, amounts in kobo, currency `NGN` | everything | documented |
| `transaction/initialize` needs an **email**; answers `data.authorization_url`, `data.access_code`, `data.reference` | collection | documented; **our buyers have no email**, so a synthetic payer email on a domain the platform owns is sent (`PAYSTACK_PAYER_EMAIL_DOMAIN`), and no name is sent |
| `transaction/verify` answers `data.status` (`success`, `failed`, `abandoned`, `ongoing`, `pending`, `processing`, `queued`, `reversed`), `data.amount`, `data.fees`, `data.currency` | verify | documented; `abandoned` and the in-progress ones map to Pending, an unknown status to Pending |
| Webhook signature is HMAC-SHA512 of the body under the secret key | webhooks | documented |
| **Transfer references must be lowercase letters, digits, `-` and `_`, 16 to 50 characters** | payouts | documented; our references are uppercase, so the adapter lowercases them on the way out and uppercases the ones in webhooks |
| Transfers need a **recipient code** first; the answer has `data.transfer_code` and a status (`otp`, `pending`, `success`, `failed`) | payouts | documented; a transfer at `otp` stays Pending until `finalize_payout`. **Turn the OTP requirement off for API transfers in the Paystack dashboard** or payouts wait for a person |
| The transfer's fee is not returned the way Mono's is | payout fee | **unconfirmed**: the adapter reads `fee_charged` or `fees_charged` when present, else 0 |
| Refund creation has **no idempotency key** | refunds | the adapter lists the transaction's refunds first and reuses ours (marked in `merchant_note` with our refund reference) before creating |
| Refund status updates arrive as `refund.*` webhooks | refunds | the engine has no refund-webhook path yet (it did not for Mono either): a refund stays "processing" until the owner marks it |

## Tasks

1. `server/src/payments/paystack.rs`: config, client, the trait implementation, webhook verification.
2. Wiring: `PAYSTACK_SECRET_KEY` (`sk_live_` in production, `sk_test_` otherwise, a mismatch refused), `PAYSTACK_BASE_URL`, `PAYSTACK_PAYER_EMAIL_DOMAIN`; production needs *a* payment provider (Mono or Paystack); `paystack_checkout` in `ALL_METHODS`; `paystack` among the payout providers; `main` registers it.
3. Tests against a local mock (wiremock) for each call and each error rule, plus the webhook signature, plus the end-to-end checkout and payout through the real routes with a Paystack-shaped mock.
4. Docs: the go-live list (Mono replaced by Paystack), `.env.example`, README.

## Not in scope

A Paystack statement source for reconciliation (the endpoint is not confirmed), refund webhooks, split payments, and subscriptions.

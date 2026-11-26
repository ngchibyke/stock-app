# Notes for the store reviewers, draft

**Draft for the owner. Fill the placeholders. Not legal advice.**

## What the reviewer needs

Both stores ask for a way to use the whole app without real money and without waiting for an SMS.

- **A staging server** that answers over https, with the fake identity provider and the fake tone check (`IDENTITY_PROVIDER=fake`, `TONE_PROVIDER=fake`), a payment provider **sandbox** (the server has no payment provider of its own: the Mono sandbox keys are needed for checkout to work at all), and a seeded market. Production, with real payments, is not suitable for review.
- **Pre-made, already verified accounts**, because signing up needs an SMS code the reviewer cannot read. Sign-in with a verified account needs only the phone number and password:
  - Buyer: phone `{{DEMO_BUYER_PHONE}}`, password `{{DEMO_BUYER_PASSWORD}}`
  - Seller (owner of a demo store): phone `{{DEMO_SELLER_PHONE}}`, password `{{DEMO_SELLER_PASSWORD}}`
  - Do not reuse these passwords anywhere. Rotate them after review.
- **The build the reviewer tests must point at the staging address.** A release build refuses any non-https address, so staging needs https too.

## Notes to paste into the review form (edit)

> `{{APP_NAME}}` is a marketplace for shops in Abuja, Nigeria. Sign in with the demo buyer account above to browse the market, add to cart and check out (the payment page is a provider sandbox, no real money). Use the demo seller account to see the selling area. Account deletion and data export are under Account, Privacy and data. The app is for adults. Location is read only on the phone, to say whether we deliver there, and is never sent. The camera is used only to take product photos.

## Age rating

For adults: sellers must be 18 or older, and the app has user-to-user messages that are checked for tone before delivery. Answer the content questionnaires as: no violence, no mature content, no gambling, user-generated content: yes (moderated), real-money transactions: yes (physical goods), unrestricted web access: no.

## Gaps before this can be sent

- No staging server or demo accounts exist yet.
- The identity selfie step has no provider SDK plugged in (`UnavailableSelfieCapture`); the demo seller account is therefore already verified.
- The store link for the in-app update notice needs a field in the server's app config (not built).

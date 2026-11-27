# Google Play data safety, draft answers

**Draft for the owner and legal, derived from what the app and server actually do on 2026-11-26. Not legal advice. Re-check every row against the final build and the final providers before submitting.** Google's form changes; use these as the source for the answers.

## Overview answers

| Question | Answer |
|---|---|
| Does the app collect or share any of the required user data types? | Yes |
| Is all of the user data collected by the app encrypted in transit? | Yes (https only; a release build refuses anything else) |
| Do you provide a way for users to request that their data be deleted? | Yes: in the app (Account, Privacy and data, Delete my account), with a 30-day cooling-off |
| Does the app follow the Families policy? | No, it is not for children |
| Independent security review | No |

**A web page for deletion is also required by Play** (a link users can open without the app, for example if they uninstalled it). It exists: `https://<your api host>/account-deletion` (served by the server). Give Play that address, with your real host.

## Data types

"Collected" means sent off the device to our server. "Shared" means passed to a third party for their own use; passing data to a service that processes it for us is a processor, which Google asks you to declare separately in the policy. The right choice is for legal to confirm.

| Type | Collected | Purpose | Passed to others | Notes |
|---|---|---|---|---|
| Name | Yes | Account, delivery | Courier and seller, for delivery | display name, recipient name on an address |
| Phone number | Yes | Account, sign-in codes, delivery | SMS provider (codes), courier and seller (delivery) | |
| Address | Yes | Delivery | Courier, and the seller when the seller delivers | saved addresses |
| Email address | No | | | |
| Other personal info (government ID number, date of birth) | Sellers only | Identity check | The identity provider | the NIN is never stored by us, only a keyed fingerprint; birth year only |
| Photos | Yes | Product, review and dispute photos | Public for product and review photos; dispute photos private | stored in an object store (a processor) |
| Messages (in-app messages) | Yes | Chat between buyer and seller | An AI provider checks the text for tone (text only, no names) | encrypted at rest |
| Purchase history | Yes | Orders, disputes | Payment provider, courier | |
| Payment info | Not by the app | | | card details are typed into the payment provider's own page; the app and server never see them |
| Bank account (sellers) | Yes | Payouts | Payment provider | account number is encrypted |
| Approximate location | No | | | read on the phone to say whether we deliver there, and not sent |
| Precise location | No | | | |
| Search history | Yes | Search the market | | queries sent to our server |
| Device or other IDs | Yes | Push notices | Google (Firebase) | the push token |
| App interactions, crash logs, diagnostics | No | | | no analytics or crash SDK today |
| Contacts, calendar, health, files | No | | | |

## Security practices to state

- Data in transit is encrypted.
- Personal fields (phone, address, messages, payout account) are encrypted at rest on the server.
- Users can request deletion and can download their data.
- No data is sold, and none is used for advertising.

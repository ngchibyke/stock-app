# Apple App Privacy labels, draft answers

**Draft for the owner and legal, from what the app and server do on 2026-11-26. Not legal advice. Check each against the final build and providers.**

## Tracking

**The app does not track users** (no ads, no data brokers, no cross-app identifiers). Answer "No" to tracking; no App Tracking Transparency prompt is needed.

## Data linked to the user (collected, for App Functionality only)

| Apple category | Types | Notes |
|---|---|---|
| Contact Info | Name, Phone Number, Physical Address | account, sign-in codes, delivery |
| Financial Info | Other Financial Info | sellers' payout account; card details are never collected by the app |
| Purchases | Purchase History | orders |
| User Content | Photos or Videos, Other User Content | product, review and dispute photos, chat messages, reviews |
| Search History | Search History | market search queries |
| Identifiers | User ID, Device ID | the account id; the push token |
| Sensitive Info / Other Data | Government ID number and a selfie (sellers) | handled by an identity provider; the NIN is not stored by us. Legal to decide which Apple category applies, and whether the selfie counts as biometric data |

## Data not collected

Location (approximate location is read on the phone and not sent), Health and Fitness, Browsing History, Usage Data, Diagnostics, Contacts.

## Other App Store answers

- **Export compliance:** `ITSAppUsesNonExemptEncryption` is false in `Info.plist` (standard https only); answer the questionnaire accordingly.
- **Account deletion:** required for apps with sign-up; it is in the app (Account, Privacy and data).
- **Permissions texts** (already in the app): camera for product photos, photo library to choose product photos, location to check whether we deliver where you are.

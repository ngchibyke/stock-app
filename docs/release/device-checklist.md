# Device checklist

One ordered list, merged from the app README's separate checklists, to run on real phones before a wider release. Mark each line and write down numbers where asked. Nothing here has been run on a device by the people who built the app.

**Phones:** at least one mid-range Android (2 to 3 GB of RAM, Android 8 or newer, the lowest you support) and one iPhone (iOS 15 or newer). **Server:** the staging server with the fake providers (`IDENTITY_PROVIDER=fake`, `TONE_PROVIDER=fake`), reached over https, so no real money moves. Build with `flutter run --profile` for measurements, or install the signed internal-testing build.

## 1. Install and sign-up
- [ ] The app installs and opens; the icon and the name look right (placeholders are marked).
- [ ] Sign-up with a real SMS code, end to end; the system one-time-code suggestion fills the code boxes.
- [ ] Sign-in, sign-out, and a password reset with a code.
- [ ] The keyboard and autofill on every form; no field suggests or stores the NIN or the password wrongly.
- [ ] Text at 200 percent on the account page and the forms: nothing is cut off.

## 2. The market (Android, 4G not Wi-Fi)
- [ ] First view of the market under 1.5 s from opening the tab (the base image first, then tiles sharpen): ____ s
- [ ] The flight into a store with the performance overlay on: no long red bars. Tier it ends on: ____
- [ ] Room drag: the depth shifts and settles back; products stay on their shelves and open on tap.
- [ ] Data used in a ten-minute browse (Android data usage for the app): ____ MB
- [ ] Reduced motion on: a fade instead of a flight, no drift in the room.
- [ ] Airplane mode after one visit: the market shows from cache with the offline line; a room shows its error with a retry.
- [ ] The list view, search, a product page and the cart, with no map at all.

## 3. Buying
- [ ] A sandbox payment end to end through the real webview: the provider's page, a 3-D Secure step, the return to the app closing the view. "Paid" shows only after the server says so.
- [ ] The result screens: paid, pending after the two-minute wait, failed.
- [ ] The handover code on the order screen: Android keeps it out of screenshots and the recent-apps picture. iOS has no equivalent yet: note what you see.
- [ ] Stock changing between the options and Pay: the options refresh and nothing is charged.
- [ ] An order's tracking, received, cancel, a review with photos, and a dispute with evidence photos.

## 4. Selling
- [ ] Become a seller with an access code; the identity check (fake provider: `--dart-define=IDENTITY_FAKE=true`).
- [ ] The guided camera: the permission wording, the square frame, tap to focus, flash, a denied permission falling back to the gallery.
- [ ] A photo of about 4000 px is cut, shrunk and uploaded in a few seconds without the screen freezing.
- [ ] The NIN field: numeric keyboard, no suggestions, cleared when the app goes to the background.
- [ ] A store, branches, a team member invited by phone, a product with stock per branch, an order moved through its steps.
- [ ] Entering a buyer's handover code, and the lock message after repeated wrong codes.

## 5. Chat
- [ ] A message from the other side arrives without a refresh.
- [ ] Flight mode on and off shows "Reconnecting" and then catches up, nothing twice.
- [ ] A blocked message (reason, strike, Edit puts the text back), a rewrite offered and sent, a held message, a muted sender.
- [ ] The team side: owner, manager and staff see the same conversation; the buyer never sees which member wrote; unverified staff are told before answering.
- [ ] The keyboard and composer on a small phone, at 200 percent.

## 6. Push
- [ ] The explanation after the first paid order, then the system permission (Android 13+ asks for notifications).
- [ ] A notice arrives with the app closed; tapping it opens the right order or conversation.
- [ ] The notice text is the fixed wording, never a message or an address.

## 7. Privacy and account
- [ ] Download my data: ask with the password, come back, Save or share opens the system sheet with `my-data.json`; it opens as readable JSON with your own data.
- [ ] Delete my account: the blockers are listed in words; with none, the waiting screen shows the date and nothing else opens, also after killing and reopening the app; Keep my account restores everything.
- [ ] Signing in on a second phone is signed out when the deletion is requested.

## 8. Accessibility
- [ ] TalkBack (Android) and VoiceOver (iPhone) through: sign-in, the market list, a product, the cart, checkout, an order, chat, the privacy page. Every control is read with a name.
- [ ] Every store dot on the map is read with its name.
- [ ] Large touch targets: nothing is hard to tap with one thumb.

## 9. Release specifics
- [ ] The signed build from Play internal testing installs from the store link and opens against the production address; a build pointed anywhere else refuses to start.
- [ ] The same from TestFlight.
- [ ] Rotate to landscape and back; background and resume; low battery mode.
- [ ] Update notice: set the minimum version in the server's app config above the installed version and see the notice.

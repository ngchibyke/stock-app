# "Coming soon to your location" banner, short design

Status: approved by the owner, 2026-11-25.

**Intent.** Owner: people anywhere can browse and order, as long as the delivery address is inside Abuja (the FCT). People whose phone is outside Abuja are told "coming soon to your location". The FCT-only delivery rule already exists at the address step and stays the real gate.

**Approaches.**
- **On-device rough location (chosen).** Checked against the FCT area on the phone; nothing leaves the phone.
- **Guessing from the internet connection (rejected).** Nigerian mobile data often appears to come from Lagos, so it would mislabel Abuja shoppers. It would also send the IP to a lookup service.
- **Asking "where are you?" (rejected).** An extra question for every user, for a hint.

**Design.**
- **First open** of the market: a short sheet, "We deliver in Abuja. Allow location to see if we reach you." with Allow and Not now. Either answer is remembered, and we never ask again. Allow leads to the system prompt for approximate location.
- **The check.** One rough fix (low accuracy, 10-second timeout), compared with an FCT bounding box. The box is a slight over-estimate near the borders (Suleja, for example, falls inside it). That is acceptable for a hint: such users simply see no banner, and the address rule still decides delivery.
- **No fix** (refused, timed out or off): no banner.
- **Outside the FCT:** a banner on the market screen. "Coming soon to your location. You can still order for delivery anywhere in Abuja." It has a close button, and comes back on a later day if the phone is still outside.
- **Data protection.** The position is used once, in memory, and never stored or sent. What is stored, on the phone only: whether we asked, and the day the banner was last closed. The privacy notice (Part 8) should mention the check.

**Left to the builder.** Widget layout within the app's Tokens and the press-feedback rules; the package version.

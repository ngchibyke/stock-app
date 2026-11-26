# Part 7e-4: Chat in the app, design

Status: draft for owner approval, 2026-11-26. Nothing is planned or built until it is approved.
Builds on: 7e-1 (store roles), 7e-2 (selling area), 7e-3 (buying screens, notices). Read with the chat and moderation parts of `2026-10-22-abuja-marketplace-design.md` and plan 4 (`2026-11-05-04-chat-moderation-reviews.md`).

## 1. Intent

Buyers and sellers talk in the app, in text only, with every message checked before the other side sees it. The server already does the chat and the moderation. This part adds the screens, and one server change the owner asked for on 2026-11-25: **the store's own team answers buyers** (owner, managers and staff), not only the owner. Branch-tied staff see chats about their branch's orders, plus product questions that are not tied to an order. Until now chat belonged to the store owner alone.

## 2. Goals, non-goals, anti-goals

**Goals**
1. A buyer asks about a product or an order from the product page or the order page; a seller side sees the conversation list and replies.
2. Every moderation outcome is shown honestly and kindly: delivered, held for a check, blocked with a reason, rewrite with a suggestion that sends as stored, and a mute with its end time.
3. Live delivery over the existing WebSocket, with a polling fallback from the last sequence number, so nothing is lost on a bad connection.
4. The store's team can answer, with the same rights as 7e-1 (read from the database on every request), and a removed member loses access at once.
5. Fast on mid-range phones: paged lists, only the open conversation is live-rendered, no spinner where a placeholder will do.

**Non-goals**
- Photos, voice, calls, read receipts per member, typing indicators, group chats.
- Moderators' tools (the dashboard already has them).
- Chat with Support. Disputes keep their own path; staff read an order's transcript only through a dispute, as today.
- Any way to move a conversation out of the app.

**Anti-goals (failure even if it works)**
- A phone number, address, link or payment instruction getting through the app's own checks: the server's filter stays the authority, the app only reports what it says.
- Telling a buyer which team member answered. Buyers see the store's name. Members see who on the team wrote (display name), as the order steps already do.
- The app deciding a message was sent: only the server's outcome counts.
- A message resent twice: each send carries a `client_message_id`, so a retry is the same message.

## 3. Server change: the team answers

Today `chat::service::access` lets in only the buyer or `stores.owner_id`. Change it, and the places that name the owner as the seller:

- **Access.** A caller is the seller side when they are the owner or a live member of the store (7e-1's `access` module), with these limits for **staff tied to a branch**: only conversations tied to an order of that branch (`orders.branch_id`), or to a product (no order). Store-wide staff, managers and the owner see all. Non-members get the same 404 as strangers. A staff member tied to a branch that has closed sees product chats only.
- **Identity gate.** As for order actions (7e-1b), a manager or staff member must have passed the identity check to **send**; reading is allowed. 403 `identity_required`.
- **Listing.** `GET /chats` for a member lists the store's conversations they may see (with the owner's own view unchanged for a buyer). The label "with" stays the buyer label; for members it adds nothing personal.
- **Sender.** A message already stores its `sender_id`, so who wrote is known. Messages from the store show `from: seller` to the buyer, as today; to members the view adds `by` (display name) on seller messages.
- **Moderation by person.** Strikes, mutes and rate limits stay per sending user. A muted member cannot post; the owner and others still can.
- **Read marker.** One read marker per side of a conversation (the store shares it), so an answered chat does not stay "unread" for the rest of the team. Notices clear for the member who read; others' stay until they open it (default, see section 9).
- **Notices.** `chat_message` goes to the buyer, or, for a buyer's message, to the store's team **by role and branch**, the same audience rule as new orders (owner, managers, and staff covering the conversation's branch; product chats go to the owner, managers and store-wide staff).
- **Opening.** A member does not open conversations (buyers do); an order chat can be opened by the owner or team once the order is paid (as the owner can today).
- **Audit.** Ids only. Reading a transcript as a member writes no audit row (it is their own store's chat); staff access through a dispute is unchanged and audited.
- **Tests.** Role and branch matrix, removed member mid-session, closed branch, identity gate, notice audience, shared read marker, buyer view unchanged, a socket for a member receiving only conversations they may see.

## 4. Screens

**Buyer**
- *Entry points:* "Ask the seller" on the product page, "Message the seller" on the order page (an order chat once paid), and **Messages** in Account (with an unread count, next to Notices).
- *Messages list:* conversations, newest first, store name, last time, unread dot; the order or product it is about as a small line. Paged.
- *Conversation:* bubbles, oldest at the top, a composer with a 1,000-character cap (the server's `MAX_CHAT_CHARS`), send button with press feedback, and a short line under the composer: "Keep contact details and payments in the app." A read-only banner when the conversation is no longer writable (store closed or account gone).

**Seller side (7e-2's Sell area)**
- A new **Chats** tab in the bottom bar (Orders, Chats, Products, Store, Team), for everyone with a store role, showing the conversations the role may see, with the buyer's label, what it is about (product or order, with the branch for orders), unread dot, and who last answered.
- The same conversation screen as the buyer's, from the store's side. Members who have not passed the identity check see the existing banner and a disabled composer with the plain reason.

**Moderation states (both sides)**
- *Delivered:* the bubble appears with the server's time.
- *Held for a check:* the bubble shows "Checking…" and, once the job decides, becomes delivered or blocked; the sender is told by a frame or the next poll. Held messages show only to their sender.
- *Blocked:* the sender sees the server's reason copy ("Keep contact details and payments inside the app. Remove them and try again.") in plain text, never the filter's category, and the message stays out of the other side's view.
- *Rewrite:* the server offers a rewritten text. The sender sees it, can tap "Send this instead" (sends the stored suggestion by id, no text from the app) or edit their own.
- *Strikes and mutes:* after a warning the sender sees how many strikes and what happens next in kind words; while muted the composer is disabled with the end time.
- *Rate limits:* 429 is shown as "Slow down for a moment".

## 5. Live delivery and offline

- **Socket:** `GET /chats/ws` with the bearer header (the app's mobile `IOWebSocketChannel`, never a token in the URL). Frames are `message` and `moderation`. At most five sockets per user; the app keeps one, opened while the app is in front and closed in the background. Foreground reconnect uses backoff with jitter, and the session is rechecked by the server on a timer.
- **Fallback and catch-up:** on open and on every reconnect, `GET /chats/{id}/messages?after=<last seq>` fills the gap and returns the sender's own pending messages. If the socket cannot connect, the open conversation polls every 5 seconds and the list every 30.
- **Send:** `POST /chats/{id}/messages` with a fresh `client_message_id`; the bubble shows "Sending…" until the answer. A network failure keeps the draft with "Not sent, tap to retry" and retries with the **same** id. Nothing is stored on disk (no outbox across app restarts), which keeps personal text off the phone.
- **Read:** `POST /chats/{id}/read {seq}` when the conversation is on screen and the last message is seen.
- **Push:** the `chat_message` notice opens the conversation by id (7e-3's push routing gains this kind); the text of a message is never in a push.

## 6. Structure

- `app/lib/features/chat/`: models, repository (`ChatRepository` with an API implementation and a fake), a `ChatSocket` interface (real implementation over `web_socket_channel`, fake for tests), a controller per open conversation merging history, socket frames and the sender's pending items by `seq` and `client_message_id`, the screens, and `ChatStrings`.
- New package: `web_socket_channel`.
- 7e-2's shell gains the Chats tab; 7e-3's notice routing gains `conversation_id`.

## 7. Testing and checks

- Server: the matrix in section 3, run against Postgres 17 with the whole suite; fmt and clippy.
- App: unit tests for merging (out-of-order frames, duplicates, gaps), the retry rule (same id), moderation state mapping, cap and trimming; widget tests with a fake repository and socket for each state and role, the identity banner, read-only, mute, rewrite-send; press feedback and 200% text.
- Wire check against a real local server: a buyer opens a product chat, the owner and a staff member (branch-tied and store-wide) see or do not see it, a message through each outcome the offline filter can produce, the socket receiving a frame, reconnect catch-up. The Claude tone check cannot run locally (it needs an API key), so held messages and rewrites are covered by the server's own tests and the app's fakes.
- CI-equivalent for server, dashboard and app. Not run on a device.

## 8. Data protection (for DPO / legal review, not legal advice)

- Messages are personal data and may carry what people should not share: the server filters before delivery, stores text encrypted, and flagged originals are readable by the Owner only, audited (unchanged).
- The app keeps no transcript on disk and sends nothing to logs or analytics; the socket URL carries no token or id.
- Team access widens who can read a buyer's messages to the store's team. The buyer sees only the store's name; members see buyer labels, not phones. The DPO should confirm that is acceptable and that the notice to buyers says a store's team may answer.
- A removed member's access ends on their next request; their past messages stay with the conversation (record of the store's side), with their display name to be anonymised in the erasure path (part 8).

## 9. Open questions (defaults stand unless changed)

- Shared read marker for the team (default), or one per member.
- Chats tab for staff tied to a branch too (default: yes, filtered to what they may see).
- A character cap of 1,000 as the server's (default).

## 10. Left to the builder

Class and file names, the bubble look within "Abuja Dusk", the exact "slow down" wording, how many pages of history load before "load earlier".

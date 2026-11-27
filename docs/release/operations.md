# Running the server: monitoring and a runbook

For the person who keeps it running. What is built is small and deliberate: two public checks, the dashboard's alerts and fixed audit rows. External monitoring and paging are yours to subscribe to.

## The two checks

| Check | Answers | Use |
|---|---|---|
| `GET /health` | `200 {"status":"ok"}` whenever the process is up | a liveness probe: restart the process if it stops answering |
| `GET /ready` | `200 {"status":"ok"}`, or `503 {"status":"degraded", "database": false, "stale_jobs": [...], "failing_jobs": [...]}` | an uptime monitor: alert when it is not 200 |

`/ready` is public and cheap. It names only jobs, never a person, an order or an error. A job is **stale** when it has not run for 5 minutes (the loop ticks each minute), and **failing** when it reports failures and has not succeeded for 15 minutes. A fresh server with no job history is ready.

**Point a monitor at `https://<your api host>/ready` every minute** and alert on two failures in a row to your phone or email. Any uptime monitor works, many have a free tier. Also watch the TLS certificate's expiry date.

## What else to watch (nothing here is automatic)

- **The database:** automated backups, **a tested restore** (a backup you have never restored is a hope), free disk, connection count.
- **The host:** CPU, memory and disk of the API and database machines.
- **The dashboard's Alerts page** (`/admin/alerts`): payout mismatches, courier wallet warnings, statement mismatches, the identity budget, and the partner call budget. Check it daily at first.
- **Provider balances and limits:** the Shipbubble wallet (a low-balance alert is raised), Paystack settlements and the transfer balance (a payout fails if the balance is empty), SMS credit, the Anthropic usage limit, Firebase quotas.
- **Backups of the three keys** (`FIELD_KEY`, `INDEX_KEY`, `NIN_INDEX_KEY`) somewhere that survives losing the server. Without `FIELD_KEY` the encrypted data cannot be read, ever.

## Runbook

| Symptom | Likely cause | Do |
|---|---|---|
| `/ready` is 503, `database` false | the database is down, full or unreachable | check the database host, disk and connections; restart; the API recovers by itself when the pool reconnects |
| `/ready` is 503 with `stale_jobs` | the job loop stopped or is stuck (a long call, a panic, the process was starved) | look at the server log around the time of the last run; restart the process; the job runs again on the next tick |
| `failing_jobs` names `payouts`, `refunds` or `book_shipments` | a provider is down, a key expired or a balance is empty | check the provider's status and keys; the job retries with backoff by itself; see the dashboard's alerts |
| `failing_jobs` names `erase_accounts` or `build_exports` | an object-store outage (erasure deletes photos first) or a database problem | check storage; the job retries every tick and an erasure that cannot finish leaves the database untouched |
| Chat messages stay "checking" | the tone check (Anthropic) is down or the key is wrong | check the key and the provider status; held messages are delivered in order when it recovers, and are withheld after 6 hours |
| Pushes do not arrive | Firebase key or project problem, or the person revoked permission | check `FCM_SERVICE_ACCOUNT_FILE` and Firebase; notices still appear in the app's inbox |
| People cannot sign up | the SMS provider is out of credit or the sender ID is blocked | check Termii; codes are the only way in |
| Many sign-in failures | an attack, or a wrong release | the server rate-limits sign-in; check the audit log and logs |

## The public pages and where to put them

The app's server host serves, besides the API, three public pages that need no sign-in: `/health`, `/ready` and **`/account-deletion`** (the page both stores ask for: a person who cannot open the app signs in and asks for deletion, or changes their mind). Give the stores `https://<your api host>/account-deletion` as the data-deletion link. If the dashboard is on its own host behind a reverse proxy, it only needs `/auth`, `/me` and `/admin` sent to the API; keep the public pages on the API host.

## Logs

The server's logs carry no names, phones, addresses, message text, OTP codes (outside development), tokens or keys, by rule, and tests check the main ones. Do not add request-body logging at the proxy. Keep access logs short-lived and restricted.

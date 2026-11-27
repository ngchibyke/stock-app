# Personal data breach procedure, draft

**Draft for the owner, the DPO and a lawyer to review and adopt. Not legal advice. Check the notification duties, the deadline and the regulator's current form against the Nigeria Data Protection Act 2023 and the Commission's guidance with counsel before relying on this.** The deadline this is built around is **72 hours** from becoming aware of a breach that is likely to put people's rights and freedoms at risk, for notifying the Nigeria Data Protection Commission (NDPC); people affected are told without undue delay when the risk is high.

A *personal data breach* is any loss, theft, unauthorised access, alteration, disclosure or destruction of personal data, accidental or on purpose: a leaked key, a database copy taken, a provider's incident, a lost phone with a session, a staff member looking where they should not.

## 1. Who does what (fill in names and phone numbers)

| Role | Person | Does |
|---|---|---|
| Incident lead | `{{NAME, PHONE}}` | owns the incident, keeps the clock and the log, makes the calls below |
| Technical lead | `{{NAME, PHONE}}` | contains, investigates and preserves evidence |
| DPO / privacy lead | `{{NAME, PHONE}}` | assesses risk, drafts the notices, files with the NDPC |
| Legal counsel | `{{NAME, PHONE}}` | advises on duties, wording and contracts |
| Communications | `{{NAME, PHONE}}` | people, stores, partners and the press |

Keep this page, the phone numbers and the regulator's contact somewhere that does not depend on our own systems being up.

## 2. The clock

Write down **the moment anyone in the team first became aware**. The 72 hours run from that moment, not from when the cause is understood. If it takes longer to find out what happened, notify with what is known and add detail in stages. Keep a running log with a time for every decision.

## 3. Steps

1. **Detect and report.** Anyone who suspects a breach tells the incident lead at once. Signals: the `/ready` monitor or dashboard alerts, odd rows in the audit log, a provider's warning (Paystack, Mono, Shipbubble, the identity provider, the AI provider, the object store, Firebase), a user report, a lost or stolen device with access.
2. **Contain** (technical lead). Stop it getting worse, without destroying evidence:
   - Revoke sessions: `UPDATE sessions SET revoked_at = now() WHERE revoked_at IS NULL;` (everyone is signed out; or narrow it to the people involved).
   - Rotate what leaked: provider keys and webhook secrets are changed at the provider and in the server's settings, then the server is restarted. **`FIELD_KEY`, `INDEX_KEY` and `NIN_INDEX_KEY` cannot be rotated in place** (the stored data is encrypted or indexed under them): a leaked field key is a data-recovery project, so decide with the DPO before touching it, and keep the old key until data is re-encrypted.
   - Switch off a feature or the partner seam (Settings), or put the server in maintenance (`APP_MAINTENANCE_MESSAGE`), if that stops the exposure.
   - Preserve evidence: take a copy of the logs and a database snapshot before changing anything else. Do not delete rows.
3. **Assess** (DPO with the technical lead). Answer, from the audit log and the systems involved: what data (the categories below), whose, how many people, what period, whether it was encrypted, whether it was only exposed or actually taken, and what harm could follow (fraud, identity theft, being found at a delivery address, financial loss, embarrassment).
4. **Decide** (incident lead with the DPO and counsel). Record the decision and the reasons, including a decision *not* to notify, which is allowed only when the breach is unlikely to risk people's rights and freedoms.
5. **Notify the NDPC** within 72 hours when notification is required (section 4).
6. **Tell the people affected** without undue delay when the risk is high (section 5). Tell them what to do to protect themselves.
7. **Tell processors and partners** who need to know, and check each processor's own duty to tell us (Mono, Shipbubble, the identity provider, the AI provider, the object store, Firebase, the SMS provider). Check the contracts and data-processing agreements.
8. **Recover and learn.** Fix the cause, restore from clean data, add a test or a check so it cannot recur, and hold a short review within two weeks. Update this procedure.
9. **Record it** in the breach register, even if nobody was notified (section 6).

## 4. What the notification to the NDPC must say

Confirm the current required content with counsel; it generally covers:

- the nature of the breach, with the categories and approximate number of people and of records;
- the name and contact of the DPO or privacy lead;
- the likely consequences;
- the measures taken or proposed to deal with it and to reduce harm;
- when it happened, when we became aware, and the reason for any delay beyond 72 hours.

## 5. What to tell the people affected

Plain words, no jargon, in the app (a notice) and by the means we hold (SMS to the phone number). Say what happened, what data was involved, what we have done, what they should do (for example change their password, watch for calls claiming to be us, never share a handover code or OTP), and how to reach us. Never put personal data of anyone else in the message.

> **Template (edit):** On `{{DATE}}` we found that `{{WHAT HAPPENED, IN ONE SENTENCE}}`. The information involved was `{{CATEGORIES}}`. We have `{{WHAT WE DID}}`. Please `{{WHAT THEY SHOULD DO}}`. If you have questions, write to `{{SUPPORT EMAIL}}`. We are sorry.

## 6. The breach register

Keep one line per incident, whether or not it was notified: the date we became aware, a short description, categories and numbers of data, the effect, the decision and its reasons, who was told and when, and what was changed afterwards. The register itself holds no personal data of the people affected.

## 7. What our systems can tell us

- **`audit_log`** records who did what to which id for staff and money actions, with ids and fixed codes only (never names, phones, addresses or message text). It shows administrative access and changes, not every read.
- **Only staff reads of sensitive evidence** (disputes) are audited today. There is **no record of which database rows were read** by someone with direct database access: this is a limit to tell the DPO about, and a reason to keep database access small and logged by the host.
- **Sessions** show when someone signed in and whether a session was revoked, not what they did.
- **What is encrypted at rest:** phone, addresses, chat text, dispute statements, payout account numbers and the identity result's name. **What is not:** order totals, store names, product text and the pseudonymous ids. The NIN is never stored.
- **Logs** carry no personal data by rule; check the host's own access logs separately.

## 8. Before you need it

- [ ] Fill in section 1 and print it.
- [ ] Agree the notification channel with the NDPC and who files.
- [ ] Have a data-processing agreement with every processor that includes their duty to tell us quickly.
- [ ] Run a tabletop exercise once a year: pick a scenario, walk the steps against a stopwatch.
- [ ] Keep encrypted backups of the three keys in a place that survives the loss of the server, and test restoring from a backup.

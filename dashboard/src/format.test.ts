import { describe, expect, it } from "vitest";
import { ALERT_LABELS, ago, words } from "./format";

describe("format helpers", () => {
  it("says how long ago", () => {
    const now = Date.parse("2026-11-19T12:00:00Z");
    expect(ago("2026-11-19T11:59:40Z", now)).toBe("just now");
    expect(ago("2026-11-19T11:30:00Z", now)).toBe("30 min ago");
    expect(ago("2026-11-19T06:00:00Z", now)).toBe("6 hr ago");
    expect(ago("2026-11-16T12:00:00Z", now)).toBe("3 days ago");
  });

  it("turns codes into words", () => {
    expect(words("booking_failed")).toBe("booking failed");
    expect(words("payout.amount_mismatch")).toBe("payout amount mismatch");
  });

  it("has plain wording for every alert the server can raise", () => {
    for (const kind of [
      "payout.provider_outcome_after_close", "payout.amount_mismatch", "payout.reversed_after_success", "courier.wallet_alert",
      "statement.mismatch", "order.no_delivery_method", "order.quote_reused", "order.repeated_handover_lockout",
    ]) {
      expect(ALERT_LABELS[kind], kind).toBeDefined();
      expect(ALERT_LABELS[kind].hint.length).toBeGreaterThan(10);
    }
  });
});

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, formatKobo, getToken, request, setToken, setUnauthorizedHandler, withQuery } from "./api";

const reply = (status: number, body?: unknown) =>
  Promise.resolve(new Response(body === undefined ? null : JSON.stringify(body), { status }));

describe("api client", () => {
  beforeEach(() => setToken(null));
  afterEach(() => {
    vi.restoreAllMocks();
    setUnauthorizedHandler(null);
  });

  it("builds query strings and skips empty values", () => {
    expect(withQuery("/admin/orders", { status: "paid", q: "", n: undefined, limit: 5 })).toBe("/admin/orders?status=paid&limit=5");
    expect(withQuery("/x")).toBe("/x");
  });

  it("sends the bearer token and a JSON body", async () => {
    setToken("tok");
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(() => reply(200, { ok: true }));
    await request("POST", "/admin/x", { body: { a: 1 } });
    const [, init] = fetchMock.mock.calls[0];
    const headers = init?.headers as Record<string, string>;
    expect(headers.Authorization).toBe("Bearer tok");
    expect(headers["Content-Type"]).toBe("application/json");
    expect(init?.body).toBe('{"a":1}');
  });

  it("sends no token for an unauthenticated call", async () => {
    setToken("tok");
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(() => reply(200, {}));
    await request("POST", "/auth/sign-in", { body: {}, auth: false });
    const headers = fetchMock.mock.calls[0][1]?.headers as Record<string, string>;
    expect(headers.Authorization).toBeUndefined();
  });

  it("turns the error envelope into an ApiError", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(() => reply(409, { error: { code: "already_acked", message: "Conflict with existing data" } }));
    const err = await request("POST", "/admin/alerts/ack").catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ status: 409, code: "already_acked", message: "Conflict with existing data" });
  });

  it("falls back when the error body is not JSON", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(() => Promise.resolve(new Response("<html>", { status: 502 })));
    const err = await request("GET", "/admin/x").catch((e) => e);
    expect(err).toMatchObject({ status: 502, code: "unknown" });
  });

  it("signs out on a 401 for an authenticated call, but not for sign-in", async () => {
    const handler = vi.fn();
    setUnauthorizedHandler(handler);
    setToken("tok");
    vi.spyOn(globalThis, "fetch").mockImplementation(() => reply(401, { error: { code: "unauthorized", message: "no" } }));
    await request("GET", "/me").catch(() => {});
    expect(handler).toHaveBeenCalledTimes(1);
    await request("POST", "/auth/sign-in", { auth: false }).catch(() => {});
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it("reports a network failure plainly", async () => {
    vi.spyOn(globalThis, "fetch").mockRejectedValue(new TypeError("fetch failed"));
    const err = await request("GET", "/me").catch((e) => e);
    expect(err).toMatchObject({ status: 0, code: "network" });
  });

  it("returns undefined for 204", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(() => reply(204));
    await expect(request("POST", "/auth/sign-out")).resolves.toBeUndefined();
  });

  it("keeps the token in sessionStorage", () => {
    setToken("abc");
    expect(sessionStorage.getItem("market.token")).toBe("abc");
    expect(getToken()).toBe("abc");
    setToken(null);
    expect(getToken()).toBeNull();
  });
});

describe("formatKobo", () => {
  it("formats naira with grouping", () => {
    expect(formatKobo(123450)).toBe("₦1,234.50");
    expect(formatKobo(5)).toBe("₦0.05");
    expect(formatKobo(-250000)).toBe("-₦2,500.00");
    expect(formatKobo(null)).toBe("–");
  });
});

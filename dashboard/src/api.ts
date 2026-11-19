// The typed API client. The server answers errors with
// `{ "error": { "code": "...", "message": "..." } }`; every non-2xx becomes an
// ApiError. A 401 on an authenticated call signs the user out (the session is
// gone), through the handler the auth provider registers.

export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

const TOKEN_KEY = "market.token";

let memoryToken: string | null = null;
let onUnauthorized: (() => void) | null = null;

export function getToken(): string | null {
  if (memoryToken) return memoryToken;
  const stored = sessionStorage.getItem(TOKEN_KEY);
  if (stored) memoryToken = stored;
  return memoryToken;
}

export function setToken(token: string | null): void {
  memoryToken = token;
  if (token) sessionStorage.setItem(TOKEN_KEY, token);
  else sessionStorage.removeItem(TOKEN_KEY);
}

export function setUnauthorizedHandler(handler: (() => void) | null): void {
  onUnauthorized = handler;
}

type Query = Record<string, string | number | boolean | null | undefined>;

export interface RequestOptions {
  body?: unknown;
  query?: Query;
  /** False for calls made before there is a session (sign-in). */
  auth?: boolean;
  signal?: AbortSignal;
}

export function withQuery(path: string, query?: Query): string {
  if (!query) return path;
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined || value === null || value === "") continue;
    params.set(key, String(value));
  }
  const qs = params.toString();
  return qs ? `${path}?${qs}` : path;
}

export async function request<T>(
  method: "GET" | "POST" | "PATCH" | "PUT" | "DELETE",
  path: string,
  options: RequestOptions = {},
): Promise<T> {
  const { body, query, auth = true, signal } = options;
  const headers: Record<string, string> = { Accept: "application/json" };
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const token = auth ? getToken() : null;
  if (token) headers.Authorization = `Bearer ${token}`;

  let res: Response;
  try {
    res = await fetch(withQuery(path, query), {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      signal,
    });
  } catch (e) {
    if (e instanceof DOMException && e.name === "AbortError") throw e;
    throw new ApiError(0, "network", "Could not reach the server. Check your connection.");
  }

  if (res.status === 204) return undefined as T;
  let data: unknown = null;
  const text = await res.text();
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = null;
    }
  }
  if (!res.ok) {
    const err = (data as { error?: { code?: string; message?: string } } | null)?.error;
    if (res.status === 401 && auth && token) onUnauthorized?.();
    throw new ApiError(
      res.status,
      err?.code ?? "unknown",
      err?.message ?? `The server answered ${res.status}.`,
    );
  }
  return data as T;
}

export const api = {
  get: <T>(path: string, query?: Query, signal?: AbortSignal) =>
    request<T>("GET", path, { query, signal }),
  post: <T>(path: string, body?: unknown) => request<T>("POST", path, { body: body ?? {} }),
  patch: <T>(path: string, body: unknown) => request<T>("PATCH", path, { body }),
};

/** Kobo as naira with grouping, e.g. 123450 -> "₦1,234.50". */
export function formatKobo(kobo: number | null | undefined): string {
  if (kobo === null || kobo === undefined) return "–";
  const sign = kobo < 0 ? "-" : "";
  const abs = Math.abs(kobo);
  const naira = Math.floor(abs / 100);
  const rest = String(abs % 100).padStart(2, "0");
  return `${sign}₦${naira.toLocaleString("en-NG")}.${rest}`;
}

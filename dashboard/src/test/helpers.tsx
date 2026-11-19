import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import { type ReactElement } from "react";
import { MemoryRouter } from "react-router-dom";
import { vi } from "vitest";
import { AuthContext, type Role } from "../auth";
import { ToastProvider } from "../ui/Toasts";

export type Reply = [number, unknown] | [number];
export type Handler = (body: unknown, url: URL) => Reply | Promise<Reply>;

export interface Call {
  method: string;
  path: string;
  query: URLSearchParams;
  body: unknown;
}

/**
 * Mocks fetch. Handlers are keyed "METHOD /path" (the query string is ignored).
 * Returns the list of calls made, so a test can check what was sent.
 */
export function mockApi(handlers: Record<string, Handler>): Call[] {
  const calls: Call[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    const url = new URL(String(input), "http://localhost");
    const method = (init?.method ?? "GET").toUpperCase();
    const body = init?.body ? JSON.parse(String(init.body)) : undefined;
    calls.push({ method, path: url.pathname, query: url.searchParams, body });
    const handler = handlers[`${method} ${url.pathname}`];
    if (!handler) return new Response(JSON.stringify({ error: { code: "not_found", message: `no handler for ${method} ${url.pathname}` } }), { status: 404 });
    const [status, json] = await handler(body, url);
    return new Response(json === undefined ? null : JSON.stringify(json), { status });
  });
  return calls;
}

/** Renders a page as a signed-in `role` (the Owner by default). */
export function renderPage(ui: ReactElement, route = "/", role: Role = "owner") {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } });
  const auth = {
    user: { id: "me", display_name: "Test Admin", phone: "+2348000000000", role },
    loading: false,
    signIn: async () => {},
    signOut: async () => {},
  };
  return render(
    <AuthContext.Provider value={auth}>
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={[route]}>
          <ToastProvider>{ui}</ToastProvider>
        </MemoryRouter>
      </QueryClientProvider>
    </AuthContext.Provider>,
  );
}

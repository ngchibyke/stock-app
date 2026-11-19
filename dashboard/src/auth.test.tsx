import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setToken } from "./api";
import { AuthProvider, RequireAuth, RequireOwner, useAuth } from "./auth";
import { Login } from "./pages/Login";

const json = (status: number, body: unknown) => Promise.resolve(new Response(JSON.stringify(body), { status }));

function mockServer(role: "owner" | "support" | "user") {
  return vi.spyOn(globalThis, "fetch").mockImplementation((input, init) => {
    const url = String(input);
    if (url.endsWith("/auth/sign-in")) {
      const body = JSON.parse(String(init?.body));
      return body.password === "right-password" ? json(200, { token: "tok" }) : json(401, { error: { code: "invalid_credentials", message: "bad" } });
    }
    if (url.endsWith("/me")) return json(200, { id: "1", display_name: "Ada Obi", phone: "+2348031112222", role });
    if (url.endsWith("/auth/sign-out")) return Promise.resolve(new Response(null, { status: 204 }));
    return json(404, { error: { code: "not_found", message: "no" } });
  });
}

function app(start: string) {
  return render(
    <MemoryRouter initialEntries={[start]}>
      <AuthProvider>
        <Routes>
          <Route path="/login" element={<Login />} />
          <Route element={<RequireAuth />}>
            <Route path="/" element={<div>home page</div>} />
            <Route element={<RequireOwner />}>
              <Route path="/owner-only" element={<div>owner page</div>} />
            </Route>
            <Route path="/disputes" element={<div>disputes page</div>} />
          </Route>
        </Routes>
      </AuthProvider>
    </MemoryRouter>,
  );
}

afterEach(() => {
  vi.restoreAllMocks();
  setToken(null);
});

describe("guards", () => {
  it("sends a signed-out visitor to the sign-in page", async () => {
    app("/");
    expect(await screen.findByRole("form", { name: "Sign in" })).toBeInTheDocument();
  });

  it("lets a stored staff session straight in", async () => {
    setToken("tok");
    mockServer("owner");
    app("/");
    expect(await screen.findByText("home page")).toBeInTheDocument();
  });

  it("drops a stored session that belongs to an ordinary user", async () => {
    setToken("tok");
    mockServer("user");
    app("/");
    expect(await screen.findByRole("form", { name: "Sign in" })).toBeInTheDocument();
  });

  it("keeps Support out of owner-only pages and sends them to disputes", async () => {
    setToken("tok");
    mockServer("support");
    app("/owner-only");
    expect(await screen.findByText("disputes page")).toBeInTheDocument();
    expect(screen.queryByText("owner page")).not.toBeInTheDocument();
  });

  it("lets the Owner into owner-only pages", async () => {
    setToken("tok");
    mockServer("owner");
    app("/owner-only");
    expect(await screen.findByText("owner page")).toBeInTheDocument();
  });
});

function SignOutButton() {
  const { signOut } = useAuth();
  return <button onClick={() => void signOut()}>sign me out</button>;
}

describe("sign-out", () => {
  it("forgets where the person was, so the next sign-in starts at the overview", async () => {
    setToken("tok");
    mockServer("owner");
    render(
      <MemoryRouter initialEntries={["/owner-only"]}>
        <AuthProvider>
          <Routes>
            <Route path="/login" element={<Login />} />
            <Route element={<RequireAuth />}>
              <Route path="/" element={<div>home page</div>} />
              <Route path="/owner-only" element={<><div>owner page</div><SignOutButton /></>} />
            </Route>
          </Routes>
        </AuthProvider>
      </MemoryRouter>,
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "sign me out" }));
    await user.type(await screen.findByLabelText("Phone number"), "08031112222");
    await user.type(screen.getByLabelText("Password", { exact: true }), "right-password");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByText("home page")).toBeInTheDocument();
  });

  it("still returns to the page you were on when a session simply expires", async () => {
    setToken("tok");
    vi.spyOn(globalThis, "fetch").mockImplementation((input) =>
      String(input).endsWith("/me") ? json(401, { error: { code: "unauthorized", message: "gone" } }) : json(404, {}),
    );
    app("/owner-only");
    expect(await screen.findByRole("form", { name: "Sign in" })).toBeInTheDocument();
  });
});

describe("sign-in", () => {
  it("signs a staff member in and goes to the page they asked for", async () => {
    mockServer("owner");
    const user = userEvent.setup();
    app("/owner-only");
    await user.type(await screen.findByLabelText("Phone number"), "08031112222");
    await user.type(screen.getByLabelText("Password", { exact: true }), "right-password");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByText("owner page")).toBeInTheDocument();
  });

  it("shows a plain message for a wrong password", async () => {
    mockServer("owner");
    const user = userEvent.setup();
    app("/");
    await user.type(await screen.findByLabelText("Phone number"), "08031112222");
    await user.type(screen.getByLabelText("Password", { exact: true }), "nope");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("do not match");
  });

  it("refuses an ordinary account and signs it out again", async () => {
    const fetchMock = mockServer("user");
    const user = userEvent.setup();
    app("/");
    await user.type(await screen.findByLabelText("Phone number"), "08031112222");
    await user.type(screen.getByLabelText("Password", { exact: true }), "right-password");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("not an Owner or Support");
    await waitFor(() => expect(fetchMock.mock.calls.some(([u]) => String(u).endsWith("/auth/sign-out"))).toBe(true));
    expect(screen.queryByText("home page")).not.toBeInTheDocument();
  });

  it("can show and hide the password", async () => {
    mockServer("owner");
    const user = userEvent.setup();
    app("/");
    const field = await screen.findByLabelText("Password");
    expect(field).toHaveAttribute("type", "password");
    await user.click(screen.getByRole("button", { name: "Show password" }));
    expect(field).toHaveAttribute("type", "text");
  });
});

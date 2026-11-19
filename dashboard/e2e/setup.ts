// Starts the real server and makes sure the test Owner exists. The Owner is made
// the way a person would: sign up, verify the phone with the code the dev server
// prints, then promote the account in the database (there is no API for that on
// purpose; the CLI's `create-owner` asks for a password on a terminal).
import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const OWNER = { phone: "08011112222", name: "Chief Owner", password: "e2e-owner-password-1" };

const API_PORT = process.env.E2E_API_PORT ?? "8099";
const API = `http://127.0.0.1:${API_PORT}`;
const DB = process.env.E2E_DATABASE_URL ?? "postgres://market:market@localhost:5432/market";
// 32 bytes each, base64. Throwaway keys for a throwaway database: never reuse them.
const FIELD_KEY = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";
const INDEX_KEY = "CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk=";

const serverDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../server");

async function waitFor<T>(what: string, fn: () => Promise<T | undefined>, ms = 60_000): Promise<T> {
  const end = Date.now() + ms;
  for (;;) {
    try {
      const v = await fn();
      if (v !== undefined) return v;
    } catch {
      // not ready yet
    }
    if (Date.now() > end) throw new Error(`Timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 250));
  }
}

async function post(pathname: string, body: unknown): Promise<Response> {
  return fetch(`${API}${pathname}`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
}

export default async function globalSetup(): Promise<() => Promise<void>> {
  if (!process.env.E2E_SERVER_BIN) {
    execFileSync("cargo", ["build", "--quiet"], { cwd: serverDir, stdio: "inherit", timeout: 600_000 });
  }
  const bin = process.env.E2E_SERVER_BIN ?? path.join(serverDir, "target/debug/server");
  let output = "";
  const server: ChildProcess = spawn(bin, [], {
    cwd: serverDir,
    env: { ...process.env, DATABASE_URL: DB, BIND_ADDR: `127.0.0.1:${API_PORT}`, FIELD_KEY, INDEX_KEY, OTP_DEV_ECHO: "1", APP_ENV: "development", RUST_LOG: "warn" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  server.stdout?.on("data", (d: Buffer) => (output += d.toString()));
  server.stderr?.on("data", (d: Buffer) => (output += d.toString()));
  const stop = async () => {
    server.kill("SIGTERM");
  };

  try {
    await waitFor("the server", async () => ((await fetch(`${API}/health`)).ok ? true : undefined));

    // Already an Owner from an earlier run? Then signing in works and we are done.
    const signIn = await post("/auth/sign-in", { phone: OWNER.phone, password: OWNER.password });
    if (!signIn.ok) {
      const up = await post("/auth/sign-up", { phone: OWNER.phone, password: OWNER.password, display_name: OWNER.name });
      if (up.status !== 201) throw new Error(`sign-up answered ${up.status}: ${await up.text()}`);
      const { verification_id } = (await up.json()) as { verification_id: string };
      const code = await waitFor("the OTP code", async () => /\[dev\] OTP code: (\d+)/.exec(output)?.[1]);
      const verify = await post("/auth/verify-phone", { verification_id, code });
      if (!verify.ok) throw new Error(`verify answered ${verify.status}`);
    }
    // Promote the account that just signed in (safe to repeat).
    const { token } = (await (await post("/auth/sign-in", { phone: OWNER.phone, password: OWNER.password })).json()) as { token: string };
    const me = (await (await fetch(`${API}/me`, { headers: { Authorization: `Bearer ${token}` } })).json()) as { id: string };
    execFileSync("psql", [DB, "-v", "ON_ERROR_STOP=1", "-c", `UPDATE users SET role = 'owner' WHERE id = '${me.id}'`], { stdio: "pipe" });
  } catch (e) {
    await stop();
    throw new Error(`${String(e)}\n--- server output ---\n${output}`);
  }
  return stop;
}

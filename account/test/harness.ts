import { spawn, type ChildProcess } from "node:child_process";
import { request as httpRequest } from "node:http";
import { createHash, randomBytes } from "node:crypto";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const tmp = path.resolve(root, "..", "tmp-test");

export const CLIENT_ID = "nuzky-desktop";
export const SCOPE = "openid email profile offline_access nuzky";

export type Stack = { url: string; google: string; mail: string; stop: () => Promise<void> };

export async function startStack(options: { refreshReuseSeconds?: number; unverifiedTtlSeconds?: number } = {}): Promise<Stack> {
  mkdirSync(tmp, { recursive: true });
  const data = mkdtempSync(path.join(tmp, "account-test-"));
  const args = [path.join(root, "scripts", "e2e-server.mjs"), "--data", data];
  if (options.refreshReuseSeconds) args.push("--refresh-reuse-seconds", String(options.refreshReuseSeconds));
  if (options.unverifiedTtlSeconds) args.push("--unverified-ttl-seconds", String(options.unverifiedTtlSeconds));
  const child: ChildProcess = spawn(process.execPath, args, { stdio: ["ignore", "pipe", "pipe"] });
  const log: string[] = [];
  child.stderr!.on("data", (chunk) => log.push(String(chunk)));
  const line = await new Promise<string>((resolve, reject) => {
    createInterface({ input: child.stdout! }).once("line", resolve);
    child.once("exit", (code) => reject(new Error(`e2e-server exited with ${code}:\n${log.join("")}`)));
  });
  const { url, google, mail } = JSON.parse(line);
  return {
    url,
    google,
    mail,
    stop: async () => {
      const exited = new Promise((resolve) => child.once("exit", resolve));
      child.kill("SIGTERM");
      await exited;
      rmSync(data, { recursive: true, force: true });
    },
  };
}

export function uniqueEmail(label: string): string {
  return `${label}-${randomBytes(4).toString("hex")}@example.com`;
}

export function uniqueIp(): string {
  const bytes = randomBytes(3);
  return `10.${bytes[0]}.${bytes[1]}.${bytes[2]}`;
}

export type Mail = { to: string; subject: string; text: string; code: string | null };

export async function waitForMail(dir: string, to: string, after = 0): Promise<Mail[]> {
  const deadline = Date.now() + 5000;
  for (;;) {
    const messages = readdirSync(dir)
      .filter((name) => name.endsWith(".json"))
      .sort()
      .map((name) => JSON.parse(readFileSync(path.join(dir, name), "utf8")) as Mail)
      .filter((message) => message.to === to);
    if (messages.length > after) return messages;
    if (Date.now() > deadline) throw new Error(`No new mail for ${to}`);
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
}

export function mailFor(dir: string, to: string): Mail[] {
  return readdirSync(dir)
    .filter((name) => name.endsWith(".json"))
    .map((name) => JSON.parse(readFileSync(path.join(dir, name), "utf8")) as Mail)
    .filter((message) => message.to === to);
}

export function rawRequest(url: URL, method: string, headers: Record<string, string>, body?: string): Promise<Response> {
  return new Promise((resolve, reject) => {
    const req = httpRequest(
      url,
      { method, headers: body === undefined ? headers : { ...headers, "content-length": String(Buffer.byteLength(body)) } },
      (res) => {
        const chunks: Buffer[] = [];
        res.on("data", (chunk) => chunks.push(chunk));
        res.on("end", () => {
          const responseHeaders = new Headers();
          for (const [name, value] of Object.entries(res.headers))
            for (const one of [value ?? []].flat()) responseHeaders.append(name, one);
          const status = res.statusCode ?? 0;
          const text = Buffer.concat(chunks);
          resolve(new Response([204, 304].includes(status) ? null : text, { status, headers: responseHeaders }));
        });
      },
    );
    req.on("error", reject);
    if (body !== undefined) req.write(body);
    req.end();
  });
}

export class Browser {
  readonly cookies = new Map<string, string>();
  readonly ip = uniqueIp();
  readonly base: string;
  constructor(base: string) {
    this.base = base;
  }

  private store(response: Response) {
    for (const cookie of response.headers.getSetCookie()) {
      const [pair, ...attributes] = cookie.split(";");
      const name = pair.slice(0, pair.indexOf("="));
      const value = pair.slice(pair.indexOf("=") + 1);
      const expired = attributes.some((a) => /^\s*max-age=0\s*$/i.test(a)) || value === "";
      if (expired) this.cookies.delete(name);
      else this.cookies.set(name, value);
    }
  }

  private headers(extra: Record<string, string> = {}): Record<string, string> {
    const cookie = [...this.cookies].map(([name, value]) => `${name}=${value}`).join("; ");
    return { "x-forwarded-for": this.ip, ...(cookie ? { cookie } : {}), ...extra };
  }

  async get(url: string): Promise<Response> {
    const target = new URL(url, this.base);
    const sameHost = target.origin === this.base;
    const response = await rawRequest(target, "GET", sameHost ? this.headers({ accept: "text/html" }) : { accept: "text/html" });
    if (sameHost) this.store(response);
    return response;
  }

  async follow(url: string, stopAt: (url: URL) => boolean): Promise<{ final: URL; hops: URL[] }> {
    const hops: URL[] = [];
    let next = new URL(url, this.base);
    for (let i = 0; i < 20; i++) {
      hops.push(next);
      if (stopAt(next)) return { final: next, hops };
      const response = await this.get(next.toString());
      const location = response.headers.get("location");
      if (!location) throw new Error(`${next} answered ${response.status} without a redirect: ${(await response.text()).slice(0, 300)}`);
      next = new URL(location, next);
    }
    throw new Error("Too many redirects");
  }

  async post(endpoint: string, body: Record<string, unknown>, origin: string | null = this.base): Promise<Response> {
    const headers = this.headers({ "content-type": "application/json", accept: "application/json", ...(origin ? { origin } : {}) });
    const response = await rawRequest(new URL(endpoint, this.base), "POST", headers, JSON.stringify(body));
    this.store(response);
    return response;
  }
}

export function pkce() {
  const verifier = randomBytes(32).toString("base64url");
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  return { verifier, challenge };
}

export function authorizeUrl(base: string, redirectUri: string, extra: Record<string, string> = {}) {
  const { verifier, challenge } = pkce();
  const state = randomBytes(8).toString("hex");
  const params = new URLSearchParams({
    response_type: "code",
    client_id: CLIENT_ID,
    redirect_uri: redirectUri,
    scope: SCOPE,
    state,
    nonce: randomBytes(8).toString("hex"),
    code_challenge: challenge,
    code_challenge_method: "S256",
    ...extra,
  });
  return { url: `${base}/api/auth/oauth2/authorize?${params}`, verifier, state };
}

export function loopback(port = 49152 + Math.floor(Math.random() * 10000)) {
  return `http://127.0.0.1:${port}/auth/callback`;
}

export async function form(base: string, endpoint: string, fields: Record<string, string>) {
  const response = await fetch(`${base}/api/auth/${endpoint}`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded", accept: "application/json" },
    body: new URLSearchParams(fields),
  });
  const text = await response.text();
  let body: Record<string, unknown> = {};
  try {
    body = JSON.parse(text);
  } catch {}
  return { status: response.status, body };
}

export async function exchange(base: string, code: string, redirectUri: string, verifier: string) {
  return form(base, "oauth2/token", {
    grant_type: "authorization_code",
    code,
    redirect_uri: redirectUri,
    client_id: CLIENT_ID,
    code_verifier: verifier,
  });
}

export async function refresh(base: string, refreshToken: string) {
  return form(base, "oauth2/token", { grant_type: "refresh_token", refresh_token: refreshToken, client_id: CLIENT_ID });
}

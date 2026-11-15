import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function start(env: Record<string, string>) {
  mkdirSync(path.join(root, "..", "tmp-test"), { recursive: true });
  const data = mkdtempSync(path.join(root, "..", "tmp-test", "account-startup-"));
  const server = spawn(process.execPath, [path.join(root, ".next", "standalone", "server.js")], {
    env: { PATH: process.env.PATH!, NODE_ENV: "production", PORT: "0", HOSTNAME: "127.0.0.1", DATA_DIR: data, ...env },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  server.stdout.on("data", (chunk) => (output += chunk));
  server.stderr.on("data", (chunk) => (output += chunk));
  return new Promise<{ code: number | null; output: string }>((resolve) => {
    const timer = setTimeout(() => server.kill("SIGKILL"), 15_000);
    server.once("exit", (code) => {
      clearTimeout(timer);
      rmSync(data, { recursive: true, force: true });
      resolve({ code, output });
    });
  });
}

test("production refuses to start with test settings", async () => {
  const { code, output } = await start({
    BASE_URL: "http://account.nuzky.app",
    AUTH_SECRET: "short",
    GOOGLE_CLIENT_ID: "id",
    GOOGLE_CLIENT_SECRET: "secret",
    GOOGLE_DISCOVERY_URL: "http://127.0.0.1:9/.well-known/openid-configuration",
    MAIL_TRANSPORT: "file",
    MAIL_DIR: "/tmp/nuzky-mail",
    PWNED_PASSWORDS_URL: "http://127.0.0.1:9",
    ACCESS_TOKEN_SECONDS: "60",
    TELEMETRY_RETENTION_SECONDS: "60",
  });
  assert.equal(code, 1);
  for (const problem of [
    "Refusing to start in production",
    "BASE_URL must start with https://",
    "AUTH_SECRET must be at least 32 characters",
    "GOOGLE_DISCOVERY_URL must be https://accounts.google.com/.well-known/openid-configuration",
    "MAIL_TRANSPORT must be smtp",
    "PWNED_PASSWORDS_URL is only for ACCOUNT_MODE=test",
    "ACCESS_TOKEN_SECONDS is only for ACCOUNT_MODE=test",
    "TELEMETRY_RETENTION_SECONDS is only for ACCOUNT_MODE=test",
  ]) {
    assert.ok(output.includes(problem), `missing "${problem}" in:\n${output}`);
  }
});

test("production refuses to start without a secret", async () => {
  const { code, output } = await start({ BASE_URL: "https://account.nuzky.app" });
  assert.equal(code, 1);
  assert.match(output, /AUTH_SECRET is not set/);
});

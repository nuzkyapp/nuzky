#!/usr/bin/env node
import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { startMockGoogle } from "./mock-google.mjs";

const GOOGLE_CLIENT_ID = "nuzky-account-test";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const serverScript = path.join(root, ".next", "standalone", "server.js");

const { values } = parseArgs({
  options: {
    data: { type: "string" },
    port: { type: "string" },
    "refresh-reuse-seconds": { type: "string" },
    "unverified-ttl-seconds": { type: "string" },
  },
});

function fail(message) {
  console.error(message);
  process.exit(1);
}

if (!values.data) fail("Usage: node account/scripts/e2e-server.mjs --data <dir> [--port <port>]");
if (!existsSync(serverScript)) fail(`${serverScript} is missing. Run npm ci and npm run build in account/ first.`);

const dataDir = path.resolve(values.data);
const mailDir = path.join(dataDir, "mail");
mkdirSync(mailDir, { recursive: true });
for (const name of ["account.db", "account.db-wal", "account.db-shm"]) rmSync(path.join(dataDir, name), { force: true });
for (const name of readdirSync(mailDir)) if (name.endsWith(".json")) rmSync(path.join(mailDir, name));

function freePort() {
  return new Promise((resolve, reject) => {
    const probe = createServer();
    probe.once("error", reject);
    probe.listen(0, "127.0.0.1", () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });
}

const google = await startMockGoogle({ clientId: GOOGLE_CLIENT_ID });
const port = values.port ? Number(values.port) : await freePort();
const url = `http://127.0.0.1:${port}`;

const env = {
  PATH: process.env.PATH,
  NODE_ENV: "production",
  PORT: String(port),
  HOSTNAME: "127.0.0.1",
  ACCOUNT_MODE: "test",
  BASE_URL: url,
  AUTH_SECRET: randomBytes(32).toString("hex"),
  DATA_DIR: dataDir,
  GOOGLE_CLIENT_ID,
  GOOGLE_CLIENT_SECRET: "nuzky-account-test-secret",
  GOOGLE_DISCOVERY_URL: google.discoveryUrl,
  MAIL_TRANSPORT: "file",
  MAIL_DIR: mailDir,
  PWNED_PASSWORDS_URL: google.url,
  NEXT_TELEMETRY_DISABLED: "1",
};
if (values["refresh-reuse-seconds"]) env.REFRESH_REUSE_SECONDS = values["refresh-reuse-seconds"];
if (values["unverified-ttl-seconds"]) env.UNVERIFIED_TTL_SECONDS = values["unverified-ttl-seconds"];

const server = spawn(process.execPath, [serverScript], { cwd: path.dirname(serverScript), env, stdio: ["ignore", "pipe", "pipe"] });
server.stdout.pipe(process.stderr);
server.stderr.pipe(process.stderr);

let stopping = false;
async function stop(code) {
  if (stopping) return;
  stopping = true;
  if (server.exitCode === null && server.signalCode === null) {
    server.kill("SIGTERM");
    await new Promise((resolve) => {
      const force = setTimeout(() => server.kill("SIGKILL"), 5000);
      server.once("exit", () => {
        clearTimeout(force);
        resolve();
      });
    });
  }
  await google.stop();
  process.exit(code);
}

server.once("exit", (code, signal) => {
  if (!stopping) console.error(`The account server stopped (${signal ?? `exit code ${code}`}).`);
  stop(stopping ? 0 : 1);
});
process.once("SIGTERM", () => stop(0));
process.once("SIGINT", () => stop(0));

const deadline = Date.now() + 30_000;
for (;;) {
  try {
    const response = await fetch(`${url}/api/auth/.well-known/openid-configuration`);
    if (response.ok) break;
  } catch {}
  if (Date.now() > deadline) {
    console.error("The account server did not answer within 30 seconds.");
    await stop(1);
  }
  await new Promise((resolve) => setTimeout(resolve, 100));
}

process.stdout.write(`${JSON.stringify({ url, google: google.url, mail: mailDir })}\n`);

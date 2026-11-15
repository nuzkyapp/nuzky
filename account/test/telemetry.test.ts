import assert from "node:assert/strict";
import { createHash, randomUUID } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { request as httpRequest } from "node:http";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { after, before, describe, test } from "node:test";
import { Browser, rawRequest, startStack, uniqueEmail, uniqueIp, waitForMail, type Stack } from "./harness.ts";

const DAY = 24 * 60 * 60 * 1000;
const ADMIN = uniqueEmail("admin");
let stack: Stack;

before(async () => {
  stack = await startStack({ adminEmails: [ADMIN] });
});
after(async () => {
  await stack?.stop();
});

function telemetryDb(on: Stack) {
  return path.join(path.dirname(on.mail), "telemetry.db");
}

function stored(install: string, on = stack) {
  const database = new DatabaseSync(telemetryDb(on), { readOnly: true });
  const rows = database
    .prepare("SELECT kind, at, version, system, data FROM events WHERE install = ? ORDER BY id")
    .all(createHash("sha256").update(install).digest("hex")) as Record<string, string | number>[];
  database.close();
  return rows;
}

function event(kind: string, data: Record<string, unknown> = {}, at = Date.now()) {
  return { kind, at: new Date(at).toISOString(), data };
}

function batch(install: string, events: unknown[]) {
  return { install, version: "0.2.0", system: "macOS 26.1 arm64", events };
}

function send(endpoint: string, body: unknown, { ip = uniqueIp(), type = "application/json", on = stack } = {}) {
  const text = typeof body === "string" ? body : JSON.stringify(body);
  return rawRequest(new URL(endpoint, on.url), "POST", { "content-type": type, "x-forwarded-for": ip }, text);
}

/** The status, or "reset" when the server stopped reading before the client had sent everything, which then loses the answer. */
function streamed(endpoint: string, body: string): Promise<number | "reset"> {
  return new Promise((resolve, reject) => {
    const request = httpRequest(
      new URL(endpoint, stack.url),
      { method: "POST", headers: { "content-type": "application/json", "x-forwarded-for": uniqueIp() } },
      (response) => {
        response.resume();
        resolve(response.statusCode!);
      },
    );
    request.on("error", (error: NodeJS.ErrnoException) => (error.code === "ECONNRESET" || error.code === "EPIPE" ? resolve("reset") : reject(error)));
    const bytes = Buffer.from(body);
    for (let i = 0; i < bytes.length; i += 16 * 1024) request.write(bytes.subarray(i, i + 16 * 1024));
    request.end();
  });
}

async function signedIn(email: string) {
  const browser = new Browser(stack.url);
  assert.equal((await browser.post("/api/auth/sign-up/email", { email, password: "correct horse battery", name: "" })).status, 200);
  const [mail] = await waitForMail(stack.mail, email);
  assert.equal((await browser.post("/api/auth/email-otp/verify-email", { email, otp: mail.code })).status, 200);
  return browser;
}

describe("receiving", () => {
  test("a valid batch is stored under the hashed install, unknown kinds too, and neither the id nor the address is in the file", async () => {
    const install = randomUUID();
    const ip = uniqueIp();
    const response = await send(
      "/api/telemetry",
      batch(install, [
        event("error", { source: "export", message: "Encoder failed", actions: ["export"] }),
        event("start", { graphics: "Apple M3", memoryGb: 16, crashedBefore: false }),
        event("brandNewKind", { anything: [1, { nested: true }] }),
      ]),
      { ip },
    );
    assert.equal(response.status, 200);
    assert.deepEqual(await response.json(), { stored: 3, skipped: 0 });

    const rows = stored(install);
    assert.deepEqual(
      rows.map((row) => row.kind),
      ["error", "start", "brandNewKind"],
    );
    assert.equal(rows[0].version, "0.2.0");
    assert.equal(rows[0].system, "macOS 26.1 arm64");
    assert.deepEqual(JSON.parse(String(rows[2].data)), { anything: [1, { nested: true }] });

    for (const file of ["", "-wal", "-shm"].map((suffix) => telemetryDb(stack) + suffix).filter(existsSync)) {
      const bytes = readFileSync(file).toString("latin1");
      assert.ok(!bytes.includes(install), `the install id is in ${path.basename(file)}`);
      assert.ok(!bytes.includes(ip), `the client address is in ${path.basename(file)}`);
    }
  });

  test("events older than 30 days or more than a day ahead are skipped", async () => {
    const install = randomUUID();
    const now = Date.now();
    const response = await send(
      "/api/telemetry",
      batch(install, [
        event("usage", { n: 1 }, now - 31 * DAY),
        event("usage", { n: 2 }, now - 29 * DAY),
        event("usage", { n: 3 }, now + 12 * 60 * 60 * 1000),
        event("usage", { n: 4 }, now + 2 * DAY),
      ]),
    );
    assert.equal(response.status, 200);
    assert.deepEqual(await response.json(), { stored: 2, skipped: 2 });
    assert.deepEqual(
      stored(install).map((row) => JSON.parse(String(row.data)).n),
      [2, 3],
    );
  });

  test("anything but JSON answers 415", async () => {
    const install = randomUUID();
    const body = batch(install, [event("usage")]);
    assert.equal((await send("/api/telemetry", body, { type: "text/plain" })).status, 415);
    assert.equal((await send("/api/telemetry", body, { type: "application/x-www-form-urlencoded" })).status, 415);
    assert.equal((await send("/api/telemetry/delete", { install }, { type: "text/plain" })).status, 415);
    assert.equal((await send("/api/telemetry", body, { type: "application/json; charset=utf-8" })).status, 200);
  });

  test("a body over 512 KiB answers 413, and one that streams without a length is refused", async () => {
    const install = randomUUID();
    const big = JSON.stringify(
      batch(
        install,
        Array.from({ length: 9 }, () => event("report", { text: "x".repeat(63 * 1024) })),
      ),
    );
    assert.ok(big.length > 512 * 1024);
    assert.ok([413, "reset"].includes(await streamed("/api/telemetry", big)));
    assert.equal((await send("/api/telemetry", big)).status, 413);
    assert.equal(stored(install).length, 0);
  });

  test("an invalid batch answers 400 and stores nothing", async () => {
    const install = randomUUID();
    const valid = event("usage", { actions: { split: 1 } });
    const cases: [string, unknown][] = [
      ["not JSON", "{"],
      ["an array", [batch(install, [valid])]],
      ["an id that is not a UUID", batch("install-1", [valid])],
      ["an uppercase id", batch(install.toUpperCase(), [valid])],
      ["a UUID v1", batch("c232ab00-9414-11ec-b3c8-9f6bdeced846", [valid])],
      ["no events", batch(install, [])],
      [
        "501 events",
        batch(
          install,
          Array.from({ length: 501 }, () => valid),
        ),
      ],
      ["data over 64 KiB", batch(install, [valid, event("report", { text: "x".repeat(64 * 1024) })])],
      ["data that is not an object", batch(install, [valid, { ...valid, data: ["a"] }])],
      ["a kind with a dash", batch(install, [valid, event("bad-kind")])],
      ["a time that is not ISO 8601", batch(install, [valid, { ...valid, at: "yesterday" }])],
      ["a long version", { ...batch(install, [valid]), version: "1".repeat(65) }],
    ];
    for (const [name, body] of cases) {
      const response = await send("/api/telemetry", body);
      assert.equal(response.status, 400, name);
      assert.deepEqual(await response.json(), { code: "INVALID" }, name);
    }
    assert.equal(stored(install).length, 0);
  });
});

describe("limits", () => {
  test("an install sends 30 batches an hour, then 429 with Retry-After, and can still delete", async () => {
    const install = randomUUID();
    const ip = uniqueIp();
    for (let i = 0; i < 30; i++) assert.equal((await send("/api/telemetry", batch(install, [event("usage")]), { ip })).status, 200);

    const limited = await send("/api/telemetry", batch(install, [event("usage")]), { ip });
    assert.equal(limited.status, 429);
    const retryAfter = Number(limited.headers.get("retry-after"));
    assert.ok(retryAfter > 3500 && retryAfter <= 3600, `Retry-After ${retryAfter}`);
    assert.deepEqual(await (await send("/api/telemetry/delete", { install }, { ip })).json(), { deleted: 30 });
    assert.equal((await send("/api/telemetry", batch(randomUUID(), [event("usage")]), { ip })).status, 200);
  });

  test("an install stores at most 5000 events in 24 hours", async () => {
    const install = randomUUID();
    const many = Array.from({ length: 500 }, () => event("usage"));
    for (let i = 0; i < 10; i++) assert.equal((await send("/api/telemetry", batch(install, many))).status, 200);
    const limited = await send("/api/telemetry", batch(install, [event("usage")]));
    assert.equal(limited.status, 429);
    const retryAfter = Number(limited.headers.get("retry-after"));
    assert.ok(retryAfter > 86_000 && retryAfter <= 86_400, `Retry-After ${retryAfter}`);
    assert.equal(stored(install).length, 5000);
  });
});

describe("deleting", () => {
  test("delete removes every event of that install and nothing else", async () => {
    const [mine, theirs] = [randomUUID(), randomUUID()];
    for (const install of [mine, theirs])
      assert.equal((await send("/api/telemetry", batch(install, [event("usage"), event("start")]))).status, 200);

    const deleted = await send("/api/telemetry/delete", { install: mine });
    assert.equal(deleted.status, 200);
    assert.deepEqual(await deleted.json(), { deleted: 2 });
    assert.equal(stored(mine).length, 0);
    assert.equal(stored(theirs).length, 2);
    assert.deepEqual(await (await send("/api/telemetry/delete", { install: mine })).json(), { deleted: 0 });
    assert.equal((await send("/api/telemetry/delete", { install: "nope" })).status, 400);
  });
});

describe("admin page", () => {
  test("/admin is a 404 signed out and for other accounts, and shows a stored error's message as text to an admin", async () => {
    const marker = `<img src=x onerror=alert(1)> crash ${randomUUID()}`;
    assert.equal((await send("/api/telemetry", batch(randomUUID(), [event("error", { source: "preview", message: marker })]))).status, 200);

    const signedOut = await new Browser(stack.url).get("/admin");
    assert.equal(signedOut.status, 404);
    assert.ok(!(await signedOut.text()).includes("crash"));

    const someone = await (await signedIn(uniqueEmail("someone"))).get("/admin");
    assert.equal(someone.status, 404);
    assert.ok(!(await someone.text()).includes("crash"));

    const admin = await signedIn(ADMIN);
    for (const days of ["", "?days=30"]) {
      const page = await admin.get(`/admin${days}`);
      assert.equal(page.status, 200);
      const html = await page.text();
      assert.ok(html.includes(marker.replace("<", "&lt;").replace(">", "&gt;")), "the message is shown");
      assert.ok(!html.includes("<img src=x"), "the message is not markup");
    }
  });
});

describe("retention", () => {
  let short: Stack;
  before(async () => {
    short = await startStack({ telemetryRetentionSeconds: 1 });
  });
  after(async () => {
    await short?.stop();
  });

  test("rows older than the retention are deleted", async () => {
    const install = randomUUID();
    assert.equal((await send("/api/telemetry", batch(install, [event("usage")]), { on: short })).status, 200);
    assert.equal(stored(install, short).length, 1);
    const deadline = Date.now() + 5000;
    while (stored(install, short).length) {
      assert.ok(Date.now() < deadline, "the row is still there after 5 s");
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
  });
});

import { createHash } from "node:crypto";
import { DatabaseSync } from "node:sqlite";
import { getIP } from "better-auth/api";
import { getServer } from "./server";

const HOUR_MS = 60 * 60 * 1000;
const DAY_MS = 24 * HOUR_MS;
const MAX_BODY_BYTES = 512 * 1024;
const MAX_DATA_BYTES = 64 * 1024;
const MAX_EVENTS = 500;
const MAX_DATABASE_BYTES = 1024 ** 3;
const INSTALL_REQUESTS_PER_HOUR = 30;
const IP_REQUESTS_PER_HOUR = 120;
const INSTALL_EVENTS_PER_DAY = 5000;
const OLDEST_EVENT_MS = 30 * DAY_MS;
const NEWEST_EVENT_MS = DAY_MS;
const INSTALL = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const KIND = /^[a-z][A-Za-z]{0,31}$/;
const UTC_TIME = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{1,9})?(Z|\+00:00)$/;

type Window = { start: number; count: number };
type Telemetry = { database: DatabaseSync; windows: Map<string, Window> };
type Event = { kind: string; at: number; data: string };

const state = globalThis as typeof globalThis & { nuzkyTelemetry?: Promise<Telemetry> };

export function getTelemetry(): Promise<Telemetry> {
  state.nuzkyTelemetry ??= open();
  return state.nuzkyTelemetry;
}

async function open(): Promise<Telemetry> {
  const { config } = await getServer();
  const database = new DatabaseSync(config.telemetryDatabasePath);
  database.exec(`
    PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA busy_timeout = 5000;
    CREATE TABLE IF NOT EXISTS events (id INTEGER PRIMARY KEY, install TEXT NOT NULL, kind TEXT NOT NULL, at INTEGER NOT NULL,
      received INTEGER NOT NULL, version TEXT NOT NULL, system TEXT NOT NULL, data TEXT NOT NULL);
    CREATE INDEX IF NOT EXISTS events_install ON events (install, received);
    CREATE INDEX IF NOT EXISTS events_kind_at ON events (kind, at);
    CREATE INDEX IF NOT EXISTS events_received ON events (received);
  `);
  const telemetry: Telemetry = { database, windows: new Map() };
  const sweep = () => {
    const now = Date.now();
    for (const [key, window] of telemetry.windows) if (now - window.start >= HOUR_MS) telemetry.windows.delete(key);
    database.prepare("DELETE FROM events WHERE received < ?").run(now - config.telemetryRetentionSeconds * 1000);
  };
  sweep();
  const timer = setInterval(
    () => {
      try {
        sweep();
      } catch (error) {
        console.error("Deleting old telemetry failed", error);
      }
    },
    Math.min(config.telemetryRetentionSeconds, 60 * 60) * 1000,
  );
  timer.unref();
  return telemetry;
}

function waitSeconds(telemetry: Telemetry, key: string, max: number, now: number): number {
  let window = telemetry.windows.get(key);
  if (!window || now - window.start >= HOUR_MS) {
    window = { start: now, count: 0 };
    telemetry.windows.set(key, window);
  }
  if (window.count >= max) return Math.max(1, Math.ceil((window.start + HOUR_MS - now) / 1000));
  window.count++;
  return 0;
}

function tooMany(seconds: number): Response {
  return Response.json({ code: "RATE_LIMITED" }, { status: 429, headers: { "retry-after": String(seconds) } });
}

function tooLarge(): Response {
  return Response.json({ code: "TOO_LARGE" }, { status: 413 });
}

function invalid(): Response {
  return Response.json({ code: "INVALID" }, { status: 400 });
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hashInstall(install: unknown): string | null {
  return typeof install === "string" && INSTALL.test(install) ? createHash("sha256").update(install).digest("hex") : null;
}

async function readJson(request: Request, telemetry: Telemetry, now: number): Promise<{ body: unknown } | Response> {
  const ip = getIP(request, (await getServer()).auth.options) ?? "";
  const wait = waitSeconds(telemetry, `ip:${ip}`, IP_REQUESTS_PER_HOUR, now);
  if (wait) return tooMany(wait);
  if (request.headers.get("content-type")?.split(";")[0].trim().toLowerCase() !== "application/json")
    return Response.json({ code: "UNSUPPORTED_MEDIA_TYPE" }, { status: 415 });
  if (Number(request.headers.get("content-length")) > MAX_BODY_BYTES) return tooLarge();
  const chunks: Uint8Array[] = [];
  let size = 0;
  const reader = request.body?.getReader();
  for (let chunk = await reader?.read(); chunk && !chunk.done; chunk = await reader!.read()) {
    size += chunk.value.byteLength;
    if (size > MAX_BODY_BYTES) {
      await reader!.cancel();
      return tooLarge();
    }
    chunks.push(chunk.value);
  }
  try {
    return { body: JSON.parse(Buffer.concat(chunks).toString("utf8")) };
  } catch {
    return invalid();
  }
}

function readEvents(telemetry: Telemetry, events: unknown): Event[] | null {
  if (!Array.isArray(events) || events.length < 1 || events.length > MAX_EVENTS) return null;
  const sqliteAccepts = telemetry.database.prepare("SELECT json_valid(?) AS ok");
  const read: Event[] = [];
  for (const event of events) {
    if (!isObject(event) || typeof event.kind !== "string" || !KIND.test(event.kind)) return null;
    const at = typeof event.at === "string" && UTC_TIME.test(event.at) ? Date.parse(event.at) : NaN;
    if (Number.isNaN(at) || !isObject(event.data)) return null;
    const data = JSON.stringify(event.data);
    if (Buffer.byteLength(data) > MAX_DATA_BYTES || !sqliteAccepts.get(data)?.ok) return null;
    read.push({ kind: event.kind, at, data });
  }
  return read;
}

export async function receive(request: Request): Promise<Response> {
  const telemetry = await getTelemetry();
  const now = Date.now();
  const read = await readJson(request, telemetry, now);
  if (read instanceof Response) return read;
  const { database } = telemetry;
  const used = database
    .prepare(
      "SELECT (page_count - freelist_count) * page_size AS bytes FROM pragma_page_count(), pragma_freelist_count(), pragma_page_size()",
    )
    .get() as { bytes: number };
  if (used.bytes > MAX_DATABASE_BYTES) return Response.json({ code: "FULL" }, { status: 503, headers: { "retry-after": "3600" } });

  const body = read.body;
  if (!isObject(body)) return invalid();
  const install = hashInstall(body.install);
  const { version, system } = body;
  if (!install || typeof version !== "string" || version.length > 64 || typeof system !== "string" || system.length > 128) return invalid();
  const events = readEvents(telemetry, body.events);
  if (!events) return invalid();

  const wait = waitSeconds(telemetry, `install:${install}`, INSTALL_REQUESTS_PER_HOUR, now);
  if (wait) return tooMany(wait);
  const fresh = events.filter((event) => event.at >= now - OLDEST_EVENT_MS && event.at <= now + NEWEST_EVENT_MS);
  const { count } = database
    .prepare("SELECT count(*) AS count FROM events WHERE install = ? AND received > ?")
    .get(install, now - DAY_MS) as { count: number };
  const over = count + fresh.length - INSTALL_EVENTS_PER_DAY;
  if (over > 0) {
    const { received } = database
      .prepare("SELECT received FROM events WHERE install = ? AND received > ? ORDER BY received LIMIT 1 OFFSET ?")
      .get(install, now - DAY_MS, over - 1) as { received: number };
    return tooMany(Math.max(1, Math.ceil((received + DAY_MS - now) / 1000)));
  }

  const insert = database.prepare("INSERT INTO events (install, kind, at, received, version, system, data) VALUES (?, ?, ?, ?, ?, ?, ?)");
  database.exec("BEGIN");
  try {
    for (const event of fresh) insert.run(install, event.kind, event.at, now, version, system, event.data);
    database.exec("COMMIT");
  } catch (error) {
    database.exec("ROLLBACK");
    throw error;
  }
  return Response.json({ stored: fresh.length, skipped: events.length - fresh.length });
}

export async function forget(request: Request): Promise<Response> {
  const telemetry = await getTelemetry();
  const now = Date.now();
  const read = await readJson(request, telemetry, now);
  if (read instanceof Response) return read;
  const install = isObject(read.body) ? hashInstall(read.body.install) : null;
  if (!install) return invalid();
  const wait = waitSeconds(telemetry, `install:${install}`, INSTALL_REQUESTS_PER_HOUR, now);
  if (wait) return tooMany(wait);
  const { changes } = telemetry.database.prepare("DELETE FROM events WHERE install = ?").run(install);
  return Response.json({ deleted: Number(changes) });
}

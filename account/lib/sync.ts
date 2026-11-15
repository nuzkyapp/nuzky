import type { DatabaseSync } from "node:sqlite";

/** What the desktop app keeps in sync for an account: Your style and My caption styles. */
export const KINDS = ["style", "caption-styles"] as const;
export type Kind = (typeof KINDS)[number];

/** A document's JSON, which holds EDIT.md or a few caption styles, a few kilobytes as a rule. */
export const MAX_BODY_BYTES = 256 * 1024;

/** `revision` 0 with no body: the account keeps none yet. */
export type Document = { revision: number; body: unknown };

/** Which computer stored a revision and what changed, as the app says. */
export type Origin = { computer: string; computerId: string; label: string };

/** A revision in the history, newest first; `at` is ISO 8601. */
export type Revision = Origin & { revision: number; at: string };

/** Revisions older than this go, but never the one a document is at now. */
export const HISTORY_DAYS = 90;
/** Revisions kept of each document at most. */
export const HISTORY_LIMIT = 500;

export function isKind(value: string): value is Kind {
  return (KINDS as readonly string[]).includes(value);
}

/** One row per account and kind and one per revision of it, removed with the account. */
export function createSyncTable(database: DatabaseSync) {
  database.exec(`CREATE TABLE IF NOT EXISTS sync_document (
    user_id TEXT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    revision INTEGER NOT NULL,
    body TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, kind)
  ) STRICT;
  CREATE TABLE IF NOT EXISTS sync_revision (
    user_id TEXT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    revision INTEGER NOT NULL,
    body TEXT NOT NULL,
    computer TEXT NOT NULL,
    computer_id TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (user_id, kind, revision)
  ) STRICT`);
}

export function readDocument(database: DatabaseSync, userId: string, kind: Kind): Document {
  const row = database.prepare("SELECT revision, body FROM sync_document WHERE user_id = ? AND kind = ?").get(userId, kind) as
    | { revision: number; body: string }
    | undefined;
  return row ? { revision: row.revision, body: JSON.parse(row.body) } : { revision: 0, body: null };
}

/**
 * Stores `body` as the next revision when the account's document is still at `revision`, and keeps it in the history.
 * Otherwise someone changed it meanwhile, and the answer is the document as it is now.
 */
export function writeDocument(
  database: DatabaseSync,
  userId: string,
  kind: Kind,
  revision: number,
  body: unknown,
  origin: Origin,
): { stored: number } | { current: Document } {
  const text = JSON.stringify(body);
  const now = new Date().toISOString();
  database.exec("BEGIN IMMEDIATE");
  try {
    const changed =
      revision === 0
        ? database
            .prepare("INSERT INTO sync_document (user_id, kind, revision, body, updated_at) VALUES (?, ?, 1, ?, ?) ON CONFLICT DO NOTHING")
            .run(userId, kind, text, now).changes
        : database
            .prepare("UPDATE sync_document SET revision = revision + 1, body = ?, updated_at = ? WHERE user_id = ? AND kind = ? AND revision = ?")
            .run(text, now, userId, kind, revision).changes;
    if (changed === 1)
      database
        .prepare(
          "INSERT OR REPLACE INTO sync_revision (user_id, kind, revision, body, computer, computer_id, label, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .run(userId, kind, revision + 1, text, origin.computer, origin.computerId, origin.label, now);
    database.exec("COMMIT");
    return changed === 1 ? { stored: revision + 1 } : { current: readDocument(database, userId, kind) };
  } catch (error) {
    database.exec("ROLLBACK");
    throw error;
  }
}

const REVISION_COLUMNS = "revision, computer, computer_id AS computerId, label, created_at AS at";

/** The document's revisions, newest first, without their bodies. */
export function history(database: DatabaseSync, userId: string, kind: Kind): Revision[] {
  return database
    .prepare(`SELECT ${REVISION_COLUMNS} FROM sync_revision WHERE user_id = ? AND kind = ? ORDER BY revision DESC LIMIT ?`)
    .all(userId, kind, HISTORY_LIMIT) as Revision[];
}

/** One revision with its body, or null when it is not kept. */
export function readRevision(database: DatabaseSync, userId: string, kind: Kind, revision: number): (Revision & { body: unknown }) | null {
  const row = database
    .prepare(`SELECT ${REVISION_COLUMNS}, body FROM sync_revision WHERE user_id = ? AND kind = ? AND revision = ?`)
    .get(userId, kind, revision) as (Revision & { body: string }) | undefined;
  return row ? { ...row, body: JSON.parse(row.body) } : null;
}

/** Forgets revisions older than `days` or beyond the newest HISTORY_LIMIT, but never the one a document is at now. */
export function pruneHistory(database: DatabaseSync, days = HISTORY_DAYS) {
  const before = new Date(Date.now() - days * 24 * 60 * 60 * 1000).toISOString();
  database
    .prepare(
      `DELETE FROM sync_revision WHERE rowid IN (
        SELECT r.rowid FROM (
          SELECT rowid, user_id, kind, revision, created_at,
            ROW_NUMBER() OVER (PARTITION BY user_id, kind ORDER BY revision DESC) AS newest
          FROM sync_revision
        ) r
        JOIN sync_document d ON d.user_id = r.user_id AND d.kind = r.kind
        WHERE r.revision <> d.revision AND (r.created_at < ? OR r.newest > ?)
      )`,
    )
    .run(before, HISTORY_LIMIT);
}

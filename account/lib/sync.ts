import type { DatabaseSync } from "node:sqlite";

/** What the desktop app keeps in sync for an account: Your style and My caption styles. */
export const KINDS = ["style", "caption-styles"] as const;
export type Kind = (typeof KINDS)[number];

/** A document's JSON, which holds EDIT.md or a few caption styles, a few kilobytes as a rule. */
export const MAX_BODY_BYTES = 256 * 1024;

/** `revision` 0 with no body: the account keeps none yet. */
export type Document = { revision: number; body: unknown };

export function isKind(value: string): value is Kind {
  return (KINDS as readonly string[]).includes(value);
}

/** One row per account and kind, removed with the account. */
export function createSyncTable(database: DatabaseSync) {
  database.exec(`CREATE TABLE IF NOT EXISTS sync_document (
    user_id TEXT NOT NULL REFERENCES "user"(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    revision INTEGER NOT NULL,
    body TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, kind)
  ) STRICT`);
}

export function readDocument(database: DatabaseSync, userId: string, kind: Kind): Document {
  const row = database.prepare("SELECT revision, body FROM sync_document WHERE user_id = ? AND kind = ?").get(userId, kind) as
    | { revision: number; body: string }
    | undefined;
  return row ? { revision: row.revision, body: JSON.parse(row.body) } : { revision: 0, body: null };
}

/**
 * Stores `body` as the next revision when the account's document is still at `revision`. Otherwise someone changed it
 * meanwhile, and the answer is the document as it is now.
 */
export function writeDocument(
  database: DatabaseSync,
  userId: string,
  kind: Kind,
  revision: number,
  body: unknown,
): { stored: number } | { current: Document } {
  const text = JSON.stringify(body);
  const now = new Date().toISOString();
  const changed =
    revision === 0
      ? database
          .prepare("INSERT INTO sync_document (user_id, kind, revision, body, updated_at) VALUES (?, ?, 1, ?, ?) ON CONFLICT DO NOTHING")
          .run(userId, kind, text, now).changes
      : database
          .prepare("UPDATE sync_document SET revision = revision + 1, body = ?, updated_at = ? WHERE user_id = ? AND kind = ? AND revision = ?")
          .run(text, now, userId, kind, revision).changes;
  return changed === 1 ? { stored: revision + 1 } : { current: readDocument(database, userId, kind) };
}

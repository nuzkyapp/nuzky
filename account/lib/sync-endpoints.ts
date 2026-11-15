import type { DatabaseSync } from "node:sqlite";
import { getOAuthProviderApi } from "@better-auth/oauth-provider";
import type { BetterAuthPlugin, GenericEndpointContext } from "better-auth";
import { createAuthEndpoint } from "better-auth/api";
import { history, isKind, KINDS, readDocument, readRevision, writeDocument, type Origin } from "./sync";

/** Every Nuzky app asks once a minute, and a team behind one address shares the limit. */
const REQUESTS_PER_MINUTE = 120;

function answer(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return Response.json(body, { status, headers: { "Cache-Control": "no-store", ...headers } });
}

/** What the app says of where a revision came from: short texts, empty when it says nothing. */
function origin(value: unknown): Origin | null {
  const given = (value ?? {}) as Record<string, unknown>;
  if (typeof given !== "object" || Array.isArray(given)) return null;
  const text = (key: string, max: number) => {
    const field = given[key] ?? "";
    return typeof field === "string" && field.length <= max ? field : null;
  };
  const [computer, computerId, label] = [text("computer", 80), text("computerId", 80), text("label", 200)];
  return computer === null || computerId === null || label === null ? null : { computer, computerId, label };
}

function unauthorized(): Response {
  return answer({ code: "UNAUTHORIZED" }, 401, { "WWW-Authenticate": 'Bearer error="invalid_token"' });
}

/**
 * The account's documents for the desktop app `clientId`, which reaches them with an access token that has the `nuzky`
 * scope: `GET /sync?style=3` answers each kind's revision, with its body only where the app's revision is another, and
 * `PUT /sync/<kind>` stores a new revision over the one the app last saw. `GET /sync/<kind>/history` lists the revisions
 * kept, and `GET /sync/<kind>/history/<revision>` answers one with its body.
 */
export function sync(database: DatabaseSync, clientId: string) {
  async function owner(ctx: GenericEndpointContext): Promise<string | null> {
    const token = /^Bearer (\S+)$/.exec(ctx.headers?.get("authorization") ?? "")?.[1];
    const provider = ctx.context.getPlugin("oauth-provider");
    if (!token || !provider?.options) return null;
    try {
      const payload = await getOAuthProviderApi(ctx, provider.options).requireActiveAccessToken(token);
      const scopes = typeof payload.scope === "string" ? payload.scope.split(" ") : [];
      if (payload.client_id !== clientId || !scopes.includes("nuzky") || typeof payload.sub !== "string") return null;
      return payload.sub;
    } catch {
      return null;
    }
  }

  return {
    id: "nuzky-sync",
    endpoints: {
      readSync: createAuthEndpoint("/sync", { method: "GET", scope: "http" }, async (ctx) => {
        const userId = await owner(ctx);
        if (!userId) return unauthorized();
        const known = new URL(ctx.request?.url ?? "http://localhost/").searchParams;
        const documents: Record<string, { revision: number; body?: unknown }> = {};
        for (const kind of KINDS) {
          const document = readDocument(database, userId, kind);
          documents[kind] = known.get(kind) === String(document.revision) ? { revision: document.revision } : document;
        }
        return answer({ documents });
      }),
      writeSync: createAuthEndpoint("/sync/:kind", { method: "PUT", scope: "http" }, async (ctx) => {
        const kind = ctx.params?.kind ?? "";
        if (!isKind(kind)) return answer({ code: "NOT_FOUND" }, 404);
        const userId = await owner(ctx);
        if (!userId) return unauthorized();
        const { revision, body, origin: from } = (ctx.body ?? {}) as { revision?: unknown; body?: unknown; origin?: unknown };
        if (typeof revision !== "number" || !Number.isSafeInteger(revision) || revision < 0) return answer({ code: "INVALID_REQUEST" }, 400);
        if (typeof body !== "object" || body === null || Array.isArray(body)) return answer({ code: "INVALID_REQUEST" }, 400);
        const where = origin(from);
        if (!where) return answer({ code: "INVALID_REQUEST" }, 400);
        const result = writeDocument(database, userId, kind, revision, body, where);
        return "stored" in result ? answer({ revision: result.stored }) : answer({ code: "CONFLICT", ...result.current }, 409);
      }),
      readHistory: createAuthEndpoint("/sync/:kind/history", { method: "GET", scope: "http" }, async (ctx) => {
        const kind = ctx.params?.kind ?? "";
        if (!isKind(kind)) return answer({ code: "NOT_FOUND" }, 404);
        const userId = await owner(ctx);
        if (!userId) return unauthorized();
        return answer({ revisions: history(database, userId, kind) });
      }),
      readRevision: createAuthEndpoint("/sync/:kind/history/:revision", { method: "GET", scope: "http" }, async (ctx) => {
        const kind = ctx.params?.kind ?? "";
        const revision = Number(ctx.params?.revision);
        if (!isKind(kind) || !Number.isSafeInteger(revision) || revision < 1) return answer({ code: "NOT_FOUND" }, 404);
        const userId = await owner(ctx);
        if (!userId) return unauthorized();
        const found = readRevision(database, userId, kind, revision);
        return found ? answer(found) : answer({ code: "NOT_FOUND" }, 404);
      }),
    },
    rateLimit: [{ pathMatcher: (path: string) => path === "/sync" || path.startsWith("/sync/"), window: 60, max: REQUESTS_PER_MINUTE }],
  } satisfies BetterAuthPlugin;
}

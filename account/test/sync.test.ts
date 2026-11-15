import assert from "node:assert/strict";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { after, before, describe, test } from "node:test";
import { authorizeUrl, Browser, exchange, form, CLIENT_ID, loopback, SCOPE, startStack, uniqueEmail, waitForMail, type Stack } from "./harness.ts";

const PASSWORD = "correct horse battery";
let stack: Stack;

before(async () => {
  stack = await startStack();
});
after(async () => {
  await stack?.stop();
});

/** Signs up by email as the README describes and returns the browser and the app's tokens. */
async function signedUp(email = uniqueEmail("sync"), scope = SCOPE) {
  const browser = new Browser(stack.url);
  const redirectUri = loopback();
  const { url, verifier } = authorizeUrl(stack.url, redirectUri, { scope });
  const signIn = new URL((await browser.get(url)).headers.get("location")!, stack.url);
  assert.equal((await browser.post("/api/auth/sign-up/email", { email, password: PASSWORD, name: "" })).status, 200);
  const [mail] = await waitForMail(stack.mail, email);
  const verified = await browser.post("/api/auth/email-otp/verify-email", { email, otp: mail.code, oauth_query: signIn.search.slice(1) });
  const code = new URL(((await verified.json()) as { url: string }).url).searchParams.get("code")!;
  const tokens = await exchange(stack.url, code, redirectUri, verifier);
  assert.equal(tokens.status, 200, JSON.stringify(tokens.body));
  return { browser, accessToken: tokens.body.access_token as string, refreshToken: tokens.body.refresh_token as string };
}

async function read(accessToken: string, known = "") {
  const response = await fetch(`${stack.url}/api/auth/sync${known}`, { headers: { authorization: `Bearer ${accessToken}` } });
  return { status: response.status, body: (await response.json()) as Record<string, any> };
}

async function write(accessToken: string, kind: string, revision: number, body: unknown) {
  const response = await fetch(`${stack.url}/api/auth/sync/${kind}`, {
    method: "PUT",
    headers: { authorization: `Bearer ${accessToken}`, "content-type": "application/json" },
    body: JSON.stringify({ revision, body }),
  });
  return { status: response.status, body: (await response.json()) as Record<string, any> };
}

const STYLE = { text: "# How I edit\n\nCut every pause longer than 0.4 s.\n" };

describe("sync", () => {
  test("a new account keeps nothing yet, and a stored revision comes back only to an app that has another", async () => {
    const { accessToken } = await signedUp();
    const empty = await read(accessToken);
    assert.equal(empty.status, 200);
    assert.deepEqual(empty.body.documents, { style: { revision: 0, body: null }, "caption-styles": { revision: 0, body: null } });

    assert.deepEqual(await write(accessToken, "style", 0, STYLE), { status: 200, body: { revision: 1 } });
    const stored = await read(accessToken);
    assert.deepEqual(stored.body.documents.style, { revision: 1, body: STYLE });
    const unchanged = await read(accessToken, "?style=1&caption-styles=0");
    assert.deepEqual(unchanged.body.documents, { style: { revision: 1 }, "caption-styles": { revision: 0 } });
  });

  test("writing over a revision someone replaced meanwhile is refused with the document as it is now", async () => {
    const { accessToken } = await signedUp();
    assert.equal((await write(accessToken, "caption-styles", 0, { styles: [] })).status, 200);
    const second = { styles: [{ name: "Mine" }] };
    assert.equal((await write(accessToken, "caption-styles", 1, second)).status, 200);
    const stale = await write(accessToken, "caption-styles", 1, { styles: [{ name: "Lost" }] });
    assert.equal(stale.status, 409);
    assert.deepEqual(stale.body, { code: "CONFLICT", revision: 2, body: second });
    const created = await write(accessToken, "caption-styles", 0, { styles: [] });
    assert.equal(created.status, 409);
  });

  test("an account sees only its own documents", async () => {
    const mine = await signedUp();
    const theirs = await signedUp();
    assert.equal((await write(mine.accessToken, "style", 0, STYLE)).status, 200);
    assert.deepEqual((await read(theirs.accessToken)).body.documents.style, { revision: 0, body: null });
  });

  test("no token, a token without the nuzky scope, a revoked sign-in and an unknown kind are refused", async () => {
    assert.equal((await fetch(`${stack.url}/api/auth/sync`)).status, 401);
    assert.equal((await read("not-a-token")).status, 401);
    const narrow = await signedUp(uniqueEmail("narrow"), "openid email profile offline_access");
    assert.equal((await read(narrow.accessToken)).status, 401);
    assert.equal((await write(narrow.accessToken, "style", 0, STYLE)).status, 401);

    const { accessToken, refreshToken } = await signedUp();
    assert.equal((await write(accessToken, "nonsense", 0, STYLE)).status, 404);
    assert.equal((await write(accessToken, "style", -1, STYLE)).status, 400);
    assert.equal((await write(accessToken, "style", 0, "text")).status, 400);
    const revoked = await form(stack.url, "oauth2/revoke", { token: refreshToken, token_type_hint: "refresh_token", client_id: CLIENT_ID });
    assert.equal(revoked.status, 200);
    assert.equal((await read(accessToken)).status, 401);
  });

  test("a body over 256 KB is refused before it is read whole", async () => {
    const { accessToken } = await signedUp();
    const large = await write(accessToken, "style", 0, { text: "x".repeat(256 * 1024) });
    assert.equal(large.status, 413);
    assert.deepEqual((await read(accessToken)).body.documents.style, { revision: 0, body: null });
  });

  test("deleting the account deletes its documents", async () => {
    const email = uniqueEmail("delete");
    const { browser, accessToken } = await signedUp(email);
    assert.equal((await write(accessToken, "style", 0, STYLE)).status, 200);
    const database = new DatabaseSync(path.join(path.dirname(stack.mail), "account.db"), { readOnly: true });
    const documents = () => {
      const { n } = database
        .prepare('SELECT count(*) AS n FROM sync_document WHERE user_id = (SELECT id FROM "user" WHERE email = ?)')
        .get(email) as { n: number };
      return n;
    };
    const { id } = database.prepare('SELECT id FROM "user" WHERE email = ?').get(email) as { id: string };
    assert.equal(documents(), 1);
    assert.equal((await browser.post("/api/auth/delete-user", {})).status, 200);
    assert.equal((await read(accessToken)).status, 401);
    const { n } = database.prepare("SELECT count(*) AS n FROM sync_document WHERE user_id = ?").get(id) as { n: number };
    database.close();
    assert.equal(n, 0);
  });
});

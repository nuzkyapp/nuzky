import assert from "node:assert/strict";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { after, before, describe, test } from "node:test";
import {
  authorizeUrl,
  Browser,
  CLIENT_ID,
  exchange,
  form,
  loopback,
  mailFor,
  refresh,
  SCOPE,
  startStack,
  uniqueEmail,
  waitForMail,
  type Stack,
} from "./harness.ts";

const PASSWORD = "correct horse battery";
let stack: Stack;

before(async () => {
  stack = await startStack({ refreshReuseSeconds: 2 });
});
after(async () => {
  await stack?.stop();
});

async function json(response: Response) {
  return (await response.json()) as Record<string, any>;
}

async function signInPage(browser: Browser, authorize: string): Promise<string> {
  const response = await browser.get(authorize);
  assert.equal(response.status, 302);
  const signIn = new URL(response.headers.get("location")!, stack.url);
  assert.equal(signIn.pathname, "/sign-in");
  return signIn.search.slice(1);
}

function appCallback(url: string, redirectUri: string, state: string): string {
  const callback = new URL(url);
  assert.equal(`${callback.origin}${callback.pathname}`, redirectUri);
  assert.equal(callback.searchParams.get("state"), state);
  const code = callback.searchParams.get("code");
  assert.ok(code, `no code in ${url}`);
  return code;
}

async function signUp(browser: Browser, email: string, redirectUri = loopback()) {
  const { url, verifier, state } = authorizeUrl(stack.url, redirectUri);
  const oauthQuery = await signInPage(browser, url);

  const created = await browser.post("/api/auth/sign-up/email", { email, password: PASSWORD, name: "" });
  assert.equal(created.status, 200);
  const [mail] = await waitForMail(stack.mail, email);
  assert.match(mail.code ?? "", /^\d{6}$/);

  const verified = await browser.post("/api/auth/email-otp/verify-email", { email, otp: mail.code, oauth_query: oauthQuery });
  assert.equal(verified.status, 200);
  const body = await json(verified);
  assert.equal(body.redirect, true);
  return { code: appCallback(body.url, redirectUri, state), verifier, redirectUri };
}

async function signedUpWithTokens(email = uniqueEmail("tokens")) {
  const browser = new Browser(stack.url);
  const { code, verifier, redirectUri } = await signUp(browser, email);
  const tokens = await exchange(stack.url, code, redirectUri, verifier);
  assert.equal(tokens.status, 200, JSON.stringify(tokens.body));
  return { browser, email, tokens: tokens.body as Record<string, any> };
}

async function userinfo(accessToken: string) {
  return fetch(`${stack.url}/api/auth/oauth2/userinfo`, { headers: { authorization: `Bearer ${accessToken}` } });
}

describe("email and password", () => {
  test("the README sequence signs up with a code and gives the app tokens that rotate and revoke", async () => {
    const email = uniqueEmail("readme");
    const { browser, tokens } = await signedUpWithTokens(email);

    assert.equal(tokens.token_type, "Bearer");
    assert.equal(tokens.expires_in, 3600);
    assert.deepEqual(String(tokens.scope).split(" ").sort(), SCOPE.split(" ").sort());
    assert.ok(tokens.access_token && tokens.refresh_token && tokens.id_token);

    const info = await userinfo(tokens.access_token);
    assert.equal(info.status, 200);
    const claims = await json(info);
    assert.equal(claims.email, email);
    assert.equal(claims.email_verified, true);
    assert.equal(typeof claims.sub, "string");
    assert.equal(claims.name, "");

    const rotated = await refresh(stack.url, tokens.refresh_token);
    assert.equal(rotated.status, 200);
    assert.notEqual(rotated.body.refresh_token, tokens.refresh_token);
    assert.equal((await userinfo(String(rotated.body.access_token))).status, 200);

    const replayed = await refresh(stack.url, tokens.refresh_token);
    assert.equal(replayed.status, 200, "reuse inside the window gets the same answer");
    assert.equal(replayed.body.refresh_token, rotated.body.refresh_token);

    await new Promise((resolve) => setTimeout(resolve, 2500));
    const reused = await refresh(stack.url, tokens.refresh_token);
    assert.equal(reused.status, 400);
    assert.equal(reused.body.error, "invalid_grant");
    const afterReuse = await refresh(stack.url, String(rotated.body.refresh_token));
    assert.equal(afterReuse.body.error, "invalid_grant", "reuse after the window ends the whole chain");

    const again = authorizeUrl(stack.url, loopback());
    const back = await browser.get(again.url);
    assert.equal(back.status, 302, "a browser with a session returns to the app at once");
    const code = appCallback(back.headers.get("location")!, new URL(again.url).searchParams.get("redirect_uri")!, again.state);
    const second = await exchange(stack.url, code, new URL(again.url).searchParams.get("redirect_uri")!, again.verifier);
    assert.equal(second.status, 200);

    const revoked = await form(stack.url, "oauth2/revoke", {
      token: String(second.body.refresh_token),
      token_type_hint: "refresh_token",
      client_id: CLIENT_ID,
    });
    assert.equal(revoked.status, 200);
    const afterRevoke = await refresh(stack.url, String(second.body.refresh_token));
    assert.equal(afterRevoke.status, 400);
    assert.equal(afterRevoke.body.error, "invalid_grant");
  });

  test("an authorization code is used once and lives at most two minutes", async () => {
    const browser = new Browser(stack.url);
    const { code, verifier, redirectUri } = await signUp(browser, uniqueEmail("code"));
    const database = new DatabaseSync(path.join(path.dirname(stack.mail), "account.db"), { readOnly: true });
    const row = database
      .prepare("SELECT createdAt, expiresAt FROM verification WHERE value LIKE '%\"authorization_code\"%' ORDER BY createdAt DESC LIMIT 1")
      .get() as Record<string, string>;
    database.close();
    const lifetime = new Date(row.expiresAt).getTime() - new Date(row.createdAt).getTime();
    assert.ok(lifetime <= 120_000, `the code lives ${lifetime / 1000} s`);

    assert.equal((await exchange(stack.url, code, redirectUri, verifier)).status, 200);
    const twice = await exchange(stack.url, code, redirectUri, verifier);
    assert.equal(twice.body.error, "invalid_grant");
  });

  test("prompt=login shows the sign-in page even with a session, then returns to the app", async () => {
    const email = uniqueEmail("prompt");
    const browser = new Browser(stack.url);
    await signUp(browser, email);

    const redirectUri = loopback();
    const { url, verifier, state } = authorizeUrl(stack.url, redirectUri, { prompt: "login" });
    const oauthQuery = await signInPage(browser, url);
    const signedIn = await browser.post("/api/auth/sign-in/email", { email, password: PASSWORD, oauth_query: oauthQuery });
    assert.equal(signedIn.status, 200);
    const code = appCallback((await json(signedIn)).url, redirectUri, state);
    assert.equal((await exchange(stack.url, code, redirectUri, verifier)).status, 200);
  });

  test("a redirect_uri other than http://127.0.0.1:<port>/auth/callback is refused without a redirect to it", async () => {
    const browser = new Browser(stack.url);
    for (const foreign of [
      "http://evil.example:5555/auth/callback",
      "http://localhost:5555/auth/callback",
      "http://127.0.0.1:5555/other",
      "https://127.0.0.1:5555/auth/callback",
      "http://127.0.0.1:5555/auth/callback?next=1",
      "http://[::1]:5555/auth/callback",
      "http://127.0.0.1:5555/auth/callback/",
    ]) {
      const response = await browser.get(authorizeUrl(stack.url, foreign).url);
      assert.equal(response.status, 302, foreign);
      const location = new URL(response.headers.get("location")!, stack.url);
      assert.equal(`${location.origin}${location.pathname}`, `${stack.url}/error`, foreign);
    }
  });

  test("signing up with an address that has an account reveals nothing and changes nothing", async () => {
    const { email } = await signedUpWithTokens(uniqueEmail("owner"));
    const fresh = uniqueEmail("fresh");
    const stranger = new Browser(stack.url);

    const duplicate = await stranger.post("/api/auth/sign-up/email", { email, password: "another password 1", name: "" });
    const brandNew = await stranger.post("/api/auth/sign-up/email", { email: fresh, password: "another password 1", name: "" });
    assert.equal(duplicate.status, brandNew.status);
    const [a, b] = [await json(duplicate), await json(brandNew)];
    assert.deepEqual(Object.keys(a).sort(), Object.keys(b).sort());
    assert.deepEqual(Object.keys(a.user).sort(), Object.keys(b.user).sort());
    assert.equal(a.token, null);
    assert.equal(a.user.emailVerified, b.user.emailVerified);
    assert.equal(a.user.email, email);

    const mails = await waitForMail(stack.mail, email, 1);
    const notice = mails.at(-1)!;
    assert.equal(notice.code, null, "the owner gets a notice, not a code");
    assert.match(notice.subject, /already have a Nuzky account/);

    const owner = new Browser(stack.url);
    assert.equal((await owner.post("/api/auth/sign-in/email", { email, password: PASSWORD })).status, 200);
    assert.equal((await owner.post("/api/auth/sign-in/email", { email, password: "another password 1" })).status, 401);
  });

  test("reset by code gives the same answer for any address, then signs in and continues to the app", async () => {
    const { email } = await signedUpWithTokens(uniqueEmail("reset"));
    const browser = new Browser(stack.url);
    const nobody = uniqueEmail("nobody");

    const unknown = await browser.post("/api/auth/email-otp/request-password-reset", { email: nobody });
    const known = await browser.post("/api/auth/email-otp/request-password-reset", { email });
    assert.equal(unknown.status, 200);
    assert.deepEqual(await json(unknown), await json(known));
    await new Promise((resolve) => setTimeout(resolve, 300));
    assert.equal(mailFor(stack.mail, nobody).length, 0);

    const resetMail = (await waitForMail(stack.mail, email, 1)).at(-1)!;
    assert.match(resetMail.subject, /reset your Nuzky password/);
    const newPassword = "a brand new password";
    const reset = await browser.post("/api/auth/email-otp/reset-password", { email, otp: resetMail.code, password: newPassword });
    assert.equal(reset.status, 200);

    const redirectUri = loopback();
    const { url, verifier, state } = authorizeUrl(stack.url, redirectUri);
    const oauthQuery = await signInPage(browser, url);
    assert.equal((await browser.post("/api/auth/sign-in/email", { email, password: PASSWORD, oauth_query: oauthQuery })).status, 401);
    const signedIn = await browser.post("/api/auth/sign-in/email", { email, password: newPassword, oauth_query: oauthQuery });
    assert.equal(signedIn.status, 200);
    const code = appCallback((await json(signedIn)).url, redirectUri, state);
    assert.equal((await exchange(stack.url, code, redirectUri, verifier)).status, 200);
  });

  test("wrong reset codes answer the same for any address, also once the code is used up", async () => {
    const { email } = await signedUpWithTokens(uniqueEmail("guess"));
    const nobody = uniqueEmail("nobody");
    for (const address of [email, nobody])
      assert.equal((await new Browser(stack.url).post("/api/auth/email-otp/request-password-reset", { email: address })).status, 200);
    await waitForMail(stack.mail, email, 1);
    const answers = async (address: string) => {
      const all = [];
      for (let i = 0; i < 4; i++) {
        const response = await new Browser(stack.url).post("/api/auth/email-otp/reset-password", {
          email: address,
          otp: "000000",
          password: "a brand new password",
        });
        all.push({ status: response.status, body: await json(response) });
      }
      return all;
    };
    assert.deepEqual(await answers(email), await answers(nobody));
  });

  test("an unverified account's first password sign-in waits for the code, then continues to the app", async () => {
    const email = uniqueEmail("later");
    const browser = new Browser(stack.url);
    assert.equal((await browser.post("/api/auth/sign-up/email", { email, password: PASSWORD, name: "" })).status, 200);
    await waitForMail(stack.mail, email);

    const redirectUri = loopback();
    const { url, verifier, state } = authorizeUrl(stack.url, redirectUri);
    const oauthQuery = await signInPage(browser, url);
    const waiting = await browser.post("/api/auth/sign-in/email", { email, password: PASSWORD, oauth_query: oauthQuery });
    assert.equal(waiting.status, 403);
    assert.equal((await json(waiting)).code, "EMAIL_NOT_VERIFIED");
    const codeMail = (await waitForMail(stack.mail, email, 1)).at(-1)!;

    const verified = await browser.post("/api/auth/email-otp/verify-email", { email, otp: codeMail.code, oauth_query: oauthQuery });
    const code = appCallback((await json(verified)).url, redirectUri, state);
    assert.equal((await exchange(stack.url, code, redirectUri, verifier)).status, 200);
  });

  test("deleting the account makes the app's next refresh fail with invalid_grant", async () => {
    const { browser, tokens } = await signedUpWithTokens(uniqueEmail("delete"));
    const deleted = await browser.post("/api/auth/delete-user", {});
    assert.equal(deleted.status, 200);

    const refreshed = await refresh(stack.url, tokens.refresh_token);
    assert.equal(refreshed.status, 400);
    assert.equal(refreshed.body.error, "invalid_grant");
    assert.equal((await userinfo(tokens.access_token)).status, 401);
  });

  test("a leaked password is refused", async () => {
    const browser = new Browser(stack.url);
    const response = await browser.post("/api/auth/sign-up/email", { email: uniqueEmail("leaked"), password: "password1234", name: "" });
    assert.equal(response.status, 400);
    assert.equal((await json(response)).code, "PASSWORD_COMPROMISED");
  });

  test("too many sign-in attempts get 429", async () => {
    const browser = new Browser(stack.url);
    const statuses: number[] = [];
    for (let i = 0; i < 6; i++) {
      statuses.push(
        (await browser.post("/api/auth/sign-in/email", { email: "nobody@example.com", password: `wrong password ${i}` })).status,
      );
    }
    assert.deepEqual(statuses, [401, 401, 401, 401, 401, 429]);
  });

  test("a browser POST without this server's Origin is refused", async () => {
    const browser = new Browser(stack.url);
    const body = { email: "nobody@example.com", password: "wrong password" };
    assert.equal((await browser.post("/api/auth/sign-in/email", body, "https://evil.example")).status, 403);
    assert.equal((await browser.post("/api/auth/sign-in/email", body, null)).status, 403);
    assert.equal((await browser.post("/api/auth/sign-out", {}, null)).status, 403);
    assert.equal((await browser.post("/api/account/set-password", { newPassword: "x".repeat(12) }, "https://evil.example")).status, 403);
    assert.equal((await browser.post("/api/auth/sign-in/email", body)).status, 401);
  });

  test("endpoints the app and pages don't use answer 404", async () => {
    const browser = new Browser(stack.url);
    assert.equal((await browser.post("/api/auth/oauth2/register", { redirect_uris: [loopback()] })).status, 404);
    assert.equal((await browser.post("/api/auth/sign-in/email-otp", { email: "a@example.com", otp: "000000" })).status, 404);
    assert.equal(
      (await browser.post("/api/auth/email-otp/send-verification-otp", { email: "a@example.com", type: "email-verification" })).status,
      404,
    );
    const credentials = await form(stack.url, "oauth2/token", { grant_type: "client_credentials", client_id: CLIENT_ID });
    assert.equal(credentials.status, 400);
  });
});

import assert from "node:assert/strict";
import { after, before, describe, test } from "node:test";
import { authorizeUrl, Browser, exchange, loopback, startStack, uniqueEmail, waitForMail, type Stack } from "./harness.ts";

let stack: Stack;

before(async () => {
  stack = await startStack();
});
after(async () => {
  await stack?.stop();
});

async function signInWithGoogle(browser: Browser, email: string, extra: Record<string, string> = { provider: "google" }) {
  const redirectUri = loopback();
  const { url, verifier, state } = authorizeUrl(stack.url, redirectUri, extra);
  const toGoogle = await browser.follow(url, (next) => next.origin === stack.google);
  const chooser = new URL(toGoogle.final);
  chooser.searchParams.set("login_hint", email);
  const toApp = await browser.follow(chooser.toString(), (next) => next.port === new URL(redirectUri).port);
  const callback = toApp.final;
  assert.equal(`${callback.origin}${callback.pathname}`, redirectUri);
  assert.equal(callback.searchParams.get("state"), state);
  const tokens = await exchange(stack.url, callback.searchParams.get("code")!, redirectUri, verifier);
  assert.equal(tokens.status, 200, JSON.stringify(tokens.body));
  return { tokens: tokens.body as Record<string, string>, toGoogle, toApp };
}

async function userinfo(accessToken: string) {
  const response = await fetch(`${stack.url}/api/auth/oauth2/userinfo`, { headers: { authorization: `Bearer ${accessToken}` } });
  assert.equal(response.status, 200);
  return (await response.json()) as Record<string, unknown>;
}

describe("Google", () => {
  test("the README sequence signs in through the mock Google from the sign-in page", async () => {
    const browser = new Browser(stack.url);
    const email = uniqueEmail("google");
    const redirectUri = loopback();
    const { url, verifier, state } = authorizeUrl(stack.url, redirectUri);

    const page = await browser.get(url);
    assert.equal(page.status, 302);
    const signIn = new URL(page.headers.get("location")!, stack.url);
    assert.equal(signIn.pathname, "/sign-in");
    const html = await (await browser.get(signIn.toString())).text();
    assert.match(html, /Continue with Google/);

    const start = await browser.get(`/sign-in/google${signIn.search}`);
    assert.equal(start.status, 302);
    const chooser = new URL(start.headers.get("location")!);
    assert.equal(chooser.origin, stack.google);
    assert.equal(chooser.searchParams.get("prompt"), "select_account");
    assert.equal(chooser.searchParams.get("scope"), "openid email profile");
    assert.equal(chooser.searchParams.get("code_challenge_method"), "S256");
    assert.equal(chooser.searchParams.get("access_type"), null);
    chooser.searchParams.set("login_hint", email);

    const fromGoogle = await browser.get(chooser.toString());
    const callback = new URL(fromGoogle.headers.get("location")!);
    assert.equal(callback.pathname, "/api/auth/callback/google");
    const toApp = await browser.get(callback.toString());
    assert.equal(toApp.status, 302);
    const app = new URL(toApp.headers.get("location")!);
    assert.equal(`${app.origin}${app.pathname}`, redirectUri);
    assert.equal(app.searchParams.get("state"), state);

    const tokens = await exchange(stack.url, app.searchParams.get("code")!, redirectUri, verifier);
    assert.equal(tokens.status, 200);
    const claims = await userinfo(String(tokens.body.access_token));
    assert.equal(claims.email, email);
    assert.equal(claims.email_verified, true);
    assert.equal(claims.name, email.split("@")[0]);
  });

  test("provider=google goes from the app straight to Google's chooser without a page of ours", async () => {
    const browser = new Browser(stack.url);
    const email = uniqueEmail("direct");
    const { toGoogle } = await signInWithGoogle(browser, email, { provider: "google" });
    const paths = toGoogle.hops.filter((hop) => hop.origin === stack.url).map((hop) => hop.pathname);
    assert.deepEqual(paths, ["/api/auth/oauth2/authorize", "/sign-in", "/sign-in/google"]);
  });

  test("cancelling at Google comes back to the sign-in page with a message instead of going to Google again", async () => {
    const browser = new Browser(stack.url);
    const { url } = authorizeUrl(stack.url, loopback(), { provider: "google" });
    const { final } = await browser.follow(url, (next) => next.origin === stack.google);
    const cancelled = new URL(`${stack.url}/api/auth/callback/google`);
    cancelled.searchParams.set("error", "access_denied");
    cancelled.searchParams.set("state", final.searchParams.get("state")!);

    const back = await browser.get(cancelled.toString());
    assert.equal(back.status, 302);
    const signIn = new URL(back.headers.get("location")!, stack.url);
    assert.equal(signIn.pathname, "/sign-in");
    assert.equal(signIn.searchParams.get("provider"), "google");

    const page = await browser.get(signIn.toString());
    assert.equal(page.status, 200);
    assert.match(await page.text(), /Signing in with Google didn&#x27;t finish/);
  });

  test("Google signs in the owner of an address that someone signed up with but never confirmed", async () => {
    const email = uniqueEmail("taken");
    const squatter = new Browser(stack.url);
    const squatterPassword = "squatter password 1";
    assert.equal((await squatter.post("/api/auth/sign-up/email", { email, password: squatterPassword, name: "" })).status, 200);
    await waitForMail(stack.mail, email);

    const owner = new Browser(stack.url);
    const { tokens } = await signInWithGoogle(owner, email);
    const claims = await userinfo(tokens.access_token);
    assert.equal(claims.email, email);
    assert.equal(claims.email_verified, true);

    const after = await squatter.post("/api/auth/sign-in/email", { email, password: squatterPassword });
    assert.equal(after.status, 401, "the unconfirmed password no longer opens the account");
  });

  test("a Google account's first password is checked for leaks too", async () => {
    const browser = new Browser(stack.url);
    await signInWithGoogle(browser, uniqueEmail("first-password"));
    const leaked = await browser.post("/api/account/set-password", { newPassword: "password1234" });
    assert.equal(leaked.status, 400);
    assert.equal((await leaked.json()).code, "PASSWORD_COMPROMISED");
    assert.equal((await browser.post("/api/account/set-password", { newPassword: "a password of my own" })).status, 200);
  });

  test("a second Google sign-in reaches the same account", async () => {
    const email = uniqueEmail("again");
    const first = await signInWithGoogle(new Browser(stack.url), email);
    const second = await signInWithGoogle(new Browser(stack.url), email);
    assert.equal((await userinfo(first.tokens.access_token)).sub, (await userinfo(second.tokens.access_token)).sub);
  });
});

describe("unconfirmed sign-ups", () => {
  test("disappear after their time, so the address is free again", async () => {
    const short = await startStack({ unverifiedTtlSeconds: 1 });
    try {
      const email = uniqueEmail("stale");
      const browser = new Browser(short.url);
      const password = "stale password 12";
      assert.equal((await browser.post("/api/auth/sign-up/email", { email, password, name: "" })).status, 200);
      assert.equal((await browser.post("/api/auth/sign-in/email", { email, password })).status, 403);
      await new Promise((resolve) => setTimeout(resolve, 2500));
      assert.equal((await browser.post("/api/auth/sign-in/email", { email, password })).status, 401);
    } finally {
      await short.stop();
    }
  });
});

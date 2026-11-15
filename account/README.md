# Nuzky account server

The optional Nuzky account at `https://account.nuzky.app`: sign up and sign in with email and password or with Google, in the system browser, for one public native client, the Nuzky desktop app. Nothing in Nuzky needs it. Signed in, the app keeps the creator's style and caption styles in it, the same on every computer.

Next.js 16 (App Router, standalone output) with better-auth 1.7 and its OAuth 2.1 provider, SQLite through `node:sqlite`, Node 26. Deployment is in [`deploy/`](deploy/README.md).

## Endpoints for the app

The issuer is `https://account.nuzky.app/api/auth`. Every path below is under `https://account.nuzky.app`.

| | Path |
|---|---|
| Authorize | `GET /api/auth/oauth2/authorize` |
| Token | `POST /api/auth/oauth2/token` (form) |
| Revoke | `POST /api/auth/oauth2/revoke` (form, RFC 7009) |
| Userinfo | `GET /api/auth/oauth2/userinfo` with `Authorization: Bearer` |
| JWKS (ID token keys) | `GET /api/auth/jwks` |
| Discovery | `GET /api/auth/.well-known/openid-configuration` |
| Page after a finished sign-in | `GET /done` |

The client is `nuzky-desktop`, public (no secret), PKCE S256 required, no consent screen, grants `authorization_code` and `refresh_token` only. Its redirect is `http://127.0.0.1:<any port>/auth/callback`; `localhost`, `[::1]`, https, another path, a query or a trailing slash are refused, and the browser lands on `/error` instead of the given address. Scopes: `openid email profile offline_access nuzky`.

Authorize takes `response_type=code, client_id, redirect_uri, scope, state, nonce, code_challenge, code_challenge_method=S256`, and two optional parameters:

- `provider=google` sends a browser straight to Google's account chooser.
- `prompt=login` shows the sign-in page even when the browser has a session.

Without a session and without `provider` the browser lands on `/sign-in`. With a session and without `prompt=login` it returns to the app at once.

The app gets `http://127.0.0.1:<port>/auth/callback?code=…&state=…&iss=https%3A%2F%2Faccount.nuzky.app%2Fapi%2Fauth`. The `iss` parameter (RFC 9207) is added by the library and equals the issuer. Once the app has the code it can redirect the browser to `/done`, which says "You're signed in. Go back to Nuzky. You can close this tab." None of the current flows sends `error=access_denied`: a cancelled Google sign-in comes back to `/sign-in` with a message, and the app's own Cancel ends the wait.

The token response has `access_token` (opaque), `refresh_token`, `expires_in` (3600), `expires_at`, `token_type` (`Bearer`), `scope` and `id_token`. A code lives 2 minutes and works once. A refresh token lives 30 days and rotates on every use. Using a rotated one again within 60 seconds returns the same answer; after that it is refused with `invalid_grant` and the whole chain ends, so the app has to sign in again. A revoked token, a deleted account or an unknown token answers `400 {"error":"invalid_grant",…}`.

Userinfo answers `sub`, `name`, `email`, `email_verified`. `name` is empty for accounts made with email and password, and the Google name for Google accounts.

### Telemetry

Neither endpoint needs an account or a token. Both take only `Content-Type: application/json` (anything else answers `415`) and a body of at most 512 KiB (`413`).

`POST /api/telemetry` with `{"install": "<UUID v4, lowercase>", "version": "<at most 64 characters>", "system": "<at most 128 characters>", "events": [{"kind": "<^[a-z][A-Za-z]{0,31}$>", "at": "<ISO 8601 UTC>", "data": {…}}]}`, 1 to 500 events, each `data` an object of at most 64 KiB as JSON. It answers `200 {"stored": n, "skipped": m}`; an event whose `at` is more than 30 days old or more than a day ahead is skipped. Any kind is stored. A wrong shape answers `400 {"code":"INVALID"}` and stores nothing.

`POST /api/telemetry/delete` with `{"install": "<uuid>"}` deletes every event of that install and answers `200 {"deleted": n}`.

`429` comes with `Retry-After` in seconds: 30 batches an hour per install (deleting is not limited per install), 120 an hour per client address, and at most 5000 stored events per install in 24 hours, so a batch that would go above answers `429`. While the stored events take more than 1 GiB, `POST /api/telemetry` answers `503` with `Retry-After: 3600`. Retry after `429`, `503` and other `5xx`; never retry `400`, `413` or `415`.

## Sync

The app keeps one JSON document per kind in the account: `style` (EDIT.md and what learning knows of its rules) and `caption-styles` (`{"styles": […]}`). Both endpoints take the app's access token as `Authorization: Bearer`, which must be the `nuzky-desktop` client's with the `nuzky` scope, and answer `401` with `WWW-Authenticate` otherwise. They share a limit of 120 requests a minute per address.

| | Path |
|---|---|
| Read | `GET /api/auth/sync?style=<revision>&caption-styles=<revision>` |
| Write | `PUT /api/auth/sync/<kind>` with `{"revision": <the one the app has>, "body": {…}}` |

Read answers `{"documents": {"style": {"revision": 3, "body": {…}}, "caption-styles": {"revision": 0, "body": null}}}`; a kind whose revision the app named comes without its body. Revision 0 without a body is no document yet. Write stores the body as the next revision while the document is still at the given revision (0 creates it) and answers `{"revision": 4}`; otherwise `409 {"code": "CONFLICT", "revision", "body"}` with the document as it is now, which the app merges and writes again. A body is a JSON object of at most 256 KB, larger answers `413`; an unknown kind `404`. A write may say where it comes from, `"origin": {"computer", "computerId", "label"}`, short texts the history shows (80, 80 and 200 characters at most), otherwise `400`.

Every stored revision is kept with its origin for 90 days, at most 500 of each document, and always the one the document is at; an hourly sweep forgets the rest. `GET /api/auth/sync/<kind>/history` answers `{"revisions": [{"revision", "computer", "computerId", "label", "at"}]}` newest first, without the bodies, and `GET /api/auth/sync/<kind>/history/<revision>` one of them with its `body`, or `404` once it is forgotten. Deleting the account deletes its documents and their history.

## Behaviour

- An email and password account confirms its address with a 6-digit code in the same tab. Only the first password sign-in waits for the code, then the pending authorization continues to the app. "Send a new code" signs in again with the password, which sends a fresh code.
- Signing up with an address that already has an account answers exactly like a new sign-up. The owner gets an email that says the account exists, with no code, and nothing about the account changes.
- Password reset by code gives the same answer whether or not the address has an account, then signs in and continues to the app. It signs out the other browsers.
- Google accounts count as confirmed from Google's `email_verified`. When the owner of an address signs in with Google while someone has signed up with that address and never confirmed it, the unconfirmed password and its sessions are removed before Google is linked, so the person who typed the password cannot get in. Accounts that are still unconfirmed after 24 hours and have only a password are deleted.
- Passwords are hashed with argon2id (19 MiB, 2 passes, 1 lane), need at least 10 characters and are checked against Have I Been Pwned (only the first 5 characters of the SHA-1 leave the server).
- Rate limits are stored in SQLite and keyed by the client address that Caddy puts in `X-Forwarded-For`: 5 password sign-ins per minute, 3 code checks per minute, and better-auth's defaults elsewhere.
- Every browser POST must carry this server's `Origin`; only `/oauth2/token` and `/oauth2/revoke` accept requests without one. Every other better-auth endpoint answers 404, including dynamic client registration.
- Telemetry is kept in its own `telemetry.db` beside `account.db` and is not backed up. A row holds the SHA-256 of the install id, never the id, with the kind, the time, the version, the system and the event's data, and nothing ties it to an account. No address, header or cookie is stored, the per-address limit is counted in memory only, and Caddy does not log `/api/telemetry*`. Rows are deleted 90 days after they arrived.
- `/admin` shows the telemetry of the last 7 or 30 days to a browser signed in with an address in `ADMIN_EMAILS`. Everyone else gets a 404.
- `/account` shows who is signed in, changes the password (or sets one for a Google account), signs this browser out and deletes the account. Deleting and setting a password need a sign-in from the last 10 minutes. Deleting removes the sessions, linked accounts and OAuth tokens, so the app's next refresh gets `invalid_grant`.

## Development

```sh
cd account
npm ci
npm run typecheck && npm run lint
npm run build
npm test
```

`npm test` starts the built server through `scripts/e2e-server.mjs` for each test file, so build first.

## The end-to-end server

```sh
node account/scripts/e2e-server.mjs --data <dir> [--port <port>] [--access-token-seconds <s>] [--telemetry-retention-seconds <s>] [--admin-emails <a,b>]
```

It starts the built server (`npm ci && npm run build` in `account/` first) on a free port of 127.0.0.1 with a fresh database in `<dir>`, a mock Google (`oauth2-mock-server`) and the file mail transport, and prints one line on stdout once the server answers:

```json
{"url":"http://127.0.0.1:65047","google":"http://127.0.0.1:65046","mail":"/abs/dir/mail"}
```

Everything else goes to stderr. SIGTERM or SIGINT stops the server and the mock. Each mail is one file in `mail`, `{"to","subject","text","code"}`, where `code` is the 6-digit code or `null`.

The mock Google signs in as the address in `login_hint` on its authorize URL, `google-user@example.com` without one, with `email_verified: true` and the name before the `@`. It also answers the Have I Been Pwned range API, where only `password1234` is leaked.

### Signing up by email without JavaScript

This is what `test/email.test.ts` runs as "the README sequence". Keep one cookie jar for the server's address. Send `Accept: text/html` and no `Sec-Fetch-*` headers on every GET, as a browser navigation would; send `Content-Type: application/json`, `Accept: application/json` and `Origin: <url>` on every POST.

1. `GET <url>/api/auth/oauth2/authorize?response_type=code&client_id=nuzky-desktop&redirect_uri=http%3A%2F%2F127.0.0.1%3A<port>%2Fauth%2Fcallback&scope=openid+email+profile+offline_access+nuzky&state=<state>&nonce=<nonce>&code_challenge=<S256 of verifier>&code_challenge_method=S256` answers `302` to `/sign-in?<signed query>`. Keep that query string, without the `?`, as `oauth_query`.
2. `POST <url>/api/auth/sign-up/email` with `{"email":…, "password":…, "name":""}` answers `200`.
3. Read the newest file in `mail` whose `to` is the address and take its `code`.
4. `POST <url>/api/auth/email-otp/verify-email` with `{"email":…, "otp":<code>, "oauth_query":<oauth_query>}` answers `200 {"redirect":true,"url":"http://127.0.0.1:<port>/auth/callback?code=…&state=<state>&iss=…"}` and sets the session cookie.
5. `POST <url>/api/auth/oauth2/token` as a form with `grant_type=authorization_code, code, redirect_uri, client_id=nuzky-desktop, code_verifier`.

Signing in later is step 1 with `prompt=login`, then `POST <url>/api/auth/sign-in/email` with `{"email","password","oauth_query"}`, which answers `200 {"redirect":true,"url":…}`.

### Signing in with Google without JavaScript

This is "the README sequence" in `test/google.test.ts`. Same headers and cookie jar; do not send the server's cookies to the mock.

1. `GET` the same authorize URL, answers `302` to `/sign-in?<signed query>`. With `&provider=google` on the authorize URL, `/sign-in` answers `307` to `/sign-in/google?…` by itself; follow it.
2. `GET <url>/sign-in/google?<signed query>` answers `302` to the mock's `/authorize?…` and sets a state cookie.
3. Add `&login_hint=<address>` to that URL and `GET` it. The mock answers `302` to `<url>/api/auth/callback/google?code=…&state=…`.
4. `GET` that callback with the cookie jar. It answers `302` to `http://127.0.0.1:<port>/auth/callback?code=…&state=<state>&iss=…`.
5. Exchange the code as above.

## Configuration

| Variable | |
|---|---|
| `BASE_URL` | `https://account.nuzky.app` |
| `AUTH_SECRET` | At least 32 characters. It signs cookies and encrypts Google's tokens and the signing keys. |
| `DATA_DIR` | Where `account.db` and `telemetry.db` live, `/data` by default. |
| `ADMIN_EMAILS` | Comma-separated addresses whose signed-in browser sees `/admin`. Empty by default. |
| `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` | The Google Cloud Web client. |
| `GOOGLE_DISCOVERY_URL` | Google's by default; tests point it at the mock. |
| `MAIL_TRANSPORT` | `smtp` (default) or `file`. |
| `SMTP_URL` | For example `smtps://user:password@smtp.example.com:465`. |
| `MAIL_FROM` | `Nuzky <account@nuzky.app>` by default. |
| `MAIL_DIR` | For the file transport. |
| `ACCOUNT_MODE` | `production` unless it is `test`. |

Unless `ACCOUNT_MODE=test`, the server refuses to start with an `http` base URL, a secret shorter than 32 characters, another Google discovery URL, the file mail transport, or any of the test-only variables `PWNED_PASSWORDS_URL`, `ACCESS_TOKEN_SECONDS`, `REFRESH_REUSE_SECONDS`, `UNVERIFIED_TTL_SECONDS` and `TELEMETRY_RETENTION_SECONDS`.

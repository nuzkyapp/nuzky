import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { OAuth2Issuer, OAuth2Service } from "oauth2-mock-server";

export const DEFAULT_GOOGLE_EMAIL = "google-user@example.com";
export const LEAKED_PASSWORD = "password1234";

function identity(email) {
  const address = email.toLowerCase();
  return {
    sub: `mock-${createHash("sha256").update(address).digest("hex").slice(0, 21)}`,
    email: address,
    email_verified: true,
    name: address.split("@")[0],
  };
}

function pwnedRange(prefix) {
  const leaked = createHash("sha1").update(LEAKED_PASSWORD).digest("hex").toUpperCase();
  const lines = ["0018A45C4D1DEF81644B54AB7F969B88D65:0", "00D4F6E8FA6EECAD2A3AA415EEC418D38EC:0"];
  if (leaked.startsWith(prefix.toUpperCase())) lines.push(`${leaked.slice(5)}:52579`);
  return lines.join("\r\n");
}

export async function startMockGoogle({ clientId }) {
  const issuer = new OAuth2Issuer();
  await issuer.keys.generate("RS256");
  const service = new OAuth2Service(issuer);
  const identities = new Map();

  service.on("beforeAuthorizeRedirect", ({ url }, req) => {
    const hint = typeof req.query.login_hint === "string" ? req.query.login_hint : DEFAULT_GOOGLE_EMAIL;
    identities.set(url.searchParams.get("code"), identity(hint));
  });
  service.on("beforeTokenSigning", (token, req) => {
    const found = identities.get(req.body?.code);
    if (found) Object.assign(token.payload, found, { aud: clientId });
  });

  const server = createServer((req, res) => {
    const range = /^\/range\/([0-9A-Fa-f]{5})$/.exec(req.url ?? "");
    if (!range) return service.requestHandler(req, res);
    res.writeHead(200, { "content-type": "text/plain" });
    res.end(pwnedRange(range[1]));
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const url = `http://127.0.0.1:${server.address().port}`;
  issuer.url = url;

  return {
    url,
    discoveryUrl: `${url}/.well-known/openid-configuration`,
    stop: () => new Promise((resolve) => server.close(resolve)),
  };
}

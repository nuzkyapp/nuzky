import { getAuth } from "@/lib/server";
import { forbidden, isSameOrigin } from "@/lib/origin";

const BROWSER_POSTS = new Set([
  "/sign-up/email",
  "/sign-in/email",
  "/sign-out",
  "/email-otp/verify-email",
  "/email-otp/request-password-reset",
  "/email-otp/reset-password",
  "/change-password",
  "/delete-user",
]);
const APP_POSTS = new Set(["/oauth2/token", "/oauth2/revoke"]);
const GETS = new Set([
  "/oauth2/authorize",
  "/oauth2/userinfo",
  "/jwks",
  "/.well-known/openid-configuration",
  "/.well-known/oauth-authorization-server",
  "/callback/google",
]);

function endpoint(request: Request): string {
  return new URL(request.url).pathname.slice("/api/auth".length);
}

function notFound(): Response {
  return Response.json({ code: "NOT_FOUND" }, { status: 404 });
}

export async function GET(request: Request) {
  if (!GETS.has(endpoint(request))) return notFound();
  return (await getAuth()).handler(request);
}

export async function POST(request: Request) {
  const path = endpoint(request);
  if (APP_POSTS.has(path)) return (await getAuth()).handler(request);
  if (!BROWSER_POSTS.has(path)) return notFound();
  if (!(await isSameOrigin(request))) return forbidden();
  return (await getAuth()).handler(request);
}

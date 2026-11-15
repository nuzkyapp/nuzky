import { getAuth } from "@/lib/server";
import { forbidden, isSameOrigin } from "@/lib/origin";
import { isKind, MAX_BODY_BYTES } from "@/lib/sync";

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
  "/sync",
]);
const RESET_PASSWORD = "/email-otp/reset-password";
const CODE_FAILURES = new Set(["INVALID_OTP", "OTP_EXPIRED", "TOO_MANY_ATTEMPTS"]);

function endpoint(request: Request): string {
  return new URL(request.url).pathname.slice("/api/auth".length);
}

function notFound(): Response {
  return Response.json({ code: "NOT_FOUND" }, { status: 404 });
}

/** The request with its body read whole, or null when the body is larger than `limit` bytes. */
async function withinLimit(request: Request, limit: number): Promise<Request | null> {
  if (Number(request.headers.get("content-length") ?? 0) > limit) return null;
  const chunks: Uint8Array[] = [];
  let size = 0;
  if (request.body) {
    for await (const chunk of request.body as unknown as AsyncIterable<Uint8Array>) {
      size += chunk.byteLength;
      if (size > limit) return null;
      chunks.push(chunk);
    }
  }
  return new Request(request.url, { method: request.method, headers: request.headers, body: Buffer.concat(chunks) });
}

/** An address without an account has no code, so a used up or expired code answers as a wrong one. */
async function sameForEveryAddress(response: Response): Promise<Response> {
  if (response.ok) return response;
  const body = await response
    .clone()
    .json()
    .catch(() => null);
  if (!CODE_FAILURES.has(body?.code)) return response;
  return Response.json({ code: "INVALID_OTP", message: "Invalid OTP" }, { status: 400 });
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
  const response = await (await getAuth()).handler(request);
  return path === RESET_PASSWORD ? sameForEveryAddress(response) : response;
}

export async function PUT(request: Request) {
  const path = endpoint(request);
  if (!path.startsWith("/sync/") || !isKind(path.slice("/sync/".length))) return notFound();
  const whole = await withinLimit(request, MAX_BODY_BYTES);
  if (!whole) return Response.json({ code: "TOO_LARGE" }, { status: 413 });
  return (await getAuth()).handler(whole);
}

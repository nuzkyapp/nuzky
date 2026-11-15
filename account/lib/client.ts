export type Failure = { ok: false; status: number; code: string };
export type Result = { ok: true; data: Record<string, unknown> | null } | Failure;

export async function post(path: string, body: Record<string, unknown> = {}): Promise<Result> {
  try {
    const response = await fetch(path, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify(body),
    });
    const data = await response.json().catch(() => null);
    if (response.ok) return { ok: true, data };
    return { ok: false, status: response.status, code: String(data?.code ?? data?.error ?? "") };
  } catch {
    return { ok: false, status: 0, code: "NETWORK" };
  }
}

const MESSAGES: Record<string, string> = {
  NETWORK: "Nuzky's account server didn't answer. Check your connection and try again.",
  INVALID_EMAIL: "Enter your whole email address, like name@example.com.",
  INVALID_EMAIL_OR_PASSWORD: "That email and password don't match. Try again, or reset the password.",
  INVALID_PASSWORD: "Your current password is not right. Try again.",
  PASSWORD_TOO_SHORT: "Use at least 10 characters.",
  PASSWORD_TOO_LONG: "Use at most 128 characters.",
  PASSWORD_COMPROMISED: "This password has appeared in a data leak. Choose a different one.",
  INVALID_OTP: "That code is not right or no longer works. Check the email, or ask for a new code.",
  OTP_EXPIRED: "That code has expired. Ask for a new one.",
  TOO_MANY_ATTEMPTS: "Too many wrong codes. Ask for a new one.",
  SESSION_NOT_FRESH: "For your safety, sign in again first.",
  PASSWORD_ALREADY_SET: "Your account already has a password. Reload the page to change it.",
  invalid_signature: "This sign-in page has expired. Go back to Nuzky and start signing in again.",
};

export function explain(failure: Failure): string {
  if (failure.status === 429) return "Too many tries. Wait a minute and try again.";
  return MESSAGES[failure.code] ?? "Something went wrong. Try again in a moment.";
}

export function continueTo(data: Record<string, unknown> | null, fallback: string) {
  window.location.assign(data?.redirect === true && typeof data.url === "string" ? data.url : fallback);
}

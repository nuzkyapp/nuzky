import { isFresh } from "@/lib/fresh";
import { forbidden, isSameOrigin } from "@/lib/origin";
import { getAuth } from "@/lib/server";

export async function POST(request: Request) {
  if (!(await isSameOrigin(request))) return forbidden();
  const auth = await getAuth();
  const session = await auth.api.getSession({ headers: request.headers });
  if (!session) return Response.json({ code: "UNAUTHORIZED" }, { status: 401 });
  if (!isFresh(session.session.createdAt)) return Response.json({ code: "SESSION_NOT_FRESH" }, { status: 403 });
  const body = await request.json().catch(() => null);
  const newPassword = typeof body?.newPassword === "string" ? body.newPassword : "";
  return auth.api.setPassword({ body: { newPassword }, headers: request.headers, asResponse: true });
}

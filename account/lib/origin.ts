import { getServer } from "./server";

export async function isSameOrigin(request: Request): Promise<boolean> {
  const { config } = await getServer();
  return request.headers.get("origin") === new URL(config.baseURL).origin;
}

export function forbidden(): Response {
  return Response.json({ code: "FORBIDDEN", message: "Open this page from account.nuzky.app and try again." }, { status: 403 });
}

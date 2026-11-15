import { signedOAuthQuery, withQuery } from "@/lib/oauth-query";
import { getAuth } from "@/lib/server";

export async function GET(request: Request) {
  const oauthQuery = signedOAuthQuery(new URL(request.url).search);
  const auth = await getAuth();
  const started = await auth.api.signInSocial({
    body: {
      provider: "google",
      callbackURL: "/account",
      errorCallbackURL: withQuery("/sign-in", oauthQuery),
      disableRedirect: true,
      ...(oauthQuery ? { oauth_query: oauthQuery } : {}),
    },
    headers: request.headers,
    asResponse: true,
  });
  if (!started.ok) return new Response(null, { status: 303, headers: { location: "/error?error=google" } });
  const { url } = (await started.json()) as { url: string };
  const response = new Response(null, { status: 302, headers: { location: url } });
  for (const cookie of started.headers.getSetCookie()) response.headers.append("set-cookie", cookie);
  return response;
}

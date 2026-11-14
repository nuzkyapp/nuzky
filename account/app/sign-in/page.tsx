import { redirect } from "next/navigation";
import { SignInForm } from "@/components/auth-forms";
import { Note, Sheet } from "@/components/ui";
import { readSearch, signedOAuthQuery, withQuery, type SearchParams } from "@/lib/oauth-query";

export default async function SignInPage({ searchParams }: { searchParams: SearchParams }) {
  const search = await readSearch(searchParams);
  const oauthQuery = signedOAuthQuery(search);
  const googleFailed = search.has("error");
  if (oauthQuery && search.get("provider") === "google" && !googleFailed) redirect(withQuery("/sign-in/google", oauthQuery));

  return (
    <Sheet title="Sign in to Nuzky">
      {search.has("deleted") ? <Note>Your Nuzky account is deleted.</Note> : null}
      <SignInForm
        oauthQuery={oauthQuery}
        problem={googleFailed ? "Signing in with Google didn't finish. Try again, or use your email and password." : undefined}
      />
    </Sheet>
  );
}

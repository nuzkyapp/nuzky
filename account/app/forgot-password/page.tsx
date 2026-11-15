import { ResetForm } from "@/components/auth-forms";
import { Sheet } from "@/components/ui";
import { readSearch, signedOAuthQuery, type SearchParams } from "@/lib/oauth-query";

export default async function ForgotPasswordPage({ searchParams }: { searchParams: SearchParams }) {
  const oauthQuery = signedOAuthQuery(await readSearch(searchParams));
  return (
    <Sheet title="Reset your password">
      <ResetForm oauthQuery={oauthQuery} />
    </Sheet>
  );
}

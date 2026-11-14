import { SignUpForm } from "@/components/auth-forms";
import { Sheet } from "@/components/ui";
import { readSearch, signedOAuthQuery, type SearchParams } from "@/lib/oauth-query";

export default async function SignUpPage({ searchParams }: { searchParams: SearchParams }) {
  const oauthQuery = signedOAuthQuery(await readSearch(searchParams));
  return (
    <Sheet title="Create a Nuzky account">
      <SignUpForm oauthQuery={oauthQuery} />
    </Sheet>
  );
}

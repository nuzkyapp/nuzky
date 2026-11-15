import { Note, Sheet, TextLink } from "@/components/ui";
import { readSearch, type SearchParams } from "@/lib/oauth-query";

const NOT_FROM_NUZKY = new Set(["invalid_client", "invalid_redirect", "unauthorized_client", "client_disabled", "invalid_request"]);

export default async function ErrorPage({ searchParams }: { searchParams: SearchParams }) {
  const error = (await readSearch(searchParams)).get("error") ?? "";
  return (
    <Sheet title="Signing in didn't work">
      <Note>
        {NOT_FROM_NUZKY.has(error)
          ? "This sign-in link didn't come from Nuzky. Start signing in from the Nuzky app."
          : "Go back to Nuzky and start signing in again."}
      </Note>
      <p className="text-center text-xs">
        <TextLink href="/account">Open your account</TextLink>
      </p>
    </Sheet>
  );
}

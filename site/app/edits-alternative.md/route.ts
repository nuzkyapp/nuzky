import { comparison } from "@/lib/comparisons";
import { comparisonMarkdown, markdownResponse } from "@/lib/markdown";
import { getRelease } from "@/lib/site";

export const dynamic = "force-static";

export async function GET() {
  return markdownResponse(comparisonMarkdown(comparison("/edits-alternative"), await getRelease()), { canonical: "/edits-alternative" });
}

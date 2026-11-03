import { guide } from "@/lib/guides";
import { guideMarkdown, markdownResponse } from "@/lib/markdown";
import { getRelease } from "@/lib/site";

export const dynamic = "force-static";

export async function GET() {
  return markdownResponse(guideMarkdown(guide("/remove-filler-words"), await getRelease()), { canonical: "/remove-filler-words" });
}

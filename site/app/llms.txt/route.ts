import { llmsTxt, markdownResponse } from "@/lib/markdown";
import { getRelease } from "@/lib/site";

export const dynamic = "force-static";

export async function GET() {
  return markdownResponse(llmsTxt(await getRelease()), { plain: true });
}

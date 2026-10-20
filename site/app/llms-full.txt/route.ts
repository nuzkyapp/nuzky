import { llmsFullTxt, markdownResponse } from "@/lib/markdown";
import { getRelease } from "@/lib/site";

export const dynamic = "force-static";

export async function GET() {
  return markdownResponse(llmsFullTxt(await getRelease()), { plain: true, noindex: true });
}

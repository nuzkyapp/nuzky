import { forget } from "@/lib/telemetry";

export async function POST(request: Request) {
  return forget(request);
}

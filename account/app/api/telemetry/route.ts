import { receive } from "@/lib/telemetry";

export async function POST(request: Request) {
  return receive(request);
}

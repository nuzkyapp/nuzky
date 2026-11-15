export async function register() {
  if (process.env.NEXT_RUNTIME !== "nodejs") return;
  const { startOrExit } = await import("./lib/server");
  await startOrExit();
  const { getTelemetry } = await import("./lib/telemetry");
  await getTelemetry();
}

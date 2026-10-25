// Silences are timeline ranges already, so they cut straight out of the timeline with the titles and captions over
// them.
function run(ctx) {
  const shortest = Math.round(ctx.inputs.minSeconds * 1000000);
  const keep = Math.round(ctx.inputs.keepSeconds * 1000000);
  const ranges = ctx.silences
    .filter((s) => s.track_id === "main" && s.end_us - s.start_us >= shortest)
    .map((s) => ({ startUs: s.start_us + keep, endUs: s.end_us - keep }))
    .filter((r) => r.endUs > r.startUs);
  nuzky.log("pauses longer than", ctx.inputs.minSeconds, "s:", ranges);
  if (ranges.length === 0) return { edits: [], message: "No pause is that long." };
  const seconds = ranges.reduce((sum, r) => sum + r.endUs - r.startUs, 0) / 1000000;
  return {
    edits: [{ type: "rippleDeleteRanges", ranges }],
    message: `Removed ${ranges.length} pause${ranges.length === 1 ? "" : "s"}, ${seconds.toFixed(1)} s in all.`,
  };
}

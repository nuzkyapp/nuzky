// Silences are timeline ranges already, so they cut straight out of the timeline with the titles and captions over
// them.
function run(ctx) {
  const shortest = Math.round(ctx.inputs.minSeconds * 1000000);
  const keep = Math.round(ctx.inputs.keepSeconds * 1000000);
  // A cut inside a pause gives one silence per clip; they are one pause.
  const pauses = [];
  for (const s of ctx.silences.filter((s) => s.track_id === "main")) {
    const last = pauses[pauses.length - 1];
    if (last && s.start_us <= last.end_us) last.end_us = Math.max(last.end_us, s.end_us);
    else pauses.push({ start_us: s.start_us, end_us: s.end_us });
  }
  const ranges = pauses
    .filter((s) => s.end_us - s.start_us >= shortest)
    .map((s) => ({ startUs: s.start_us + keep, endUs: s.end_us - keep }))
    .filter((r) => r.endUs > r.startUs);
  nuzky.log("pauses longer than", ctx.inputs.minSeconds, "s:", ranges);
  if (ranges.length === 0) return { edits: [], message: "No pause is that long." };
  const seconds = ranges.reduce((sum, r) => sum + r.endUs - r.startUs, 0) / 1000000;
  return {
    edits: [{ type: "rippleDeleteRanges", ranges }],
    message: `Removed ${ranges.length} pause${ranges.length === 1 ? "" : "s"}, ${seconds.toFixed(1)}\u00a0s in all.`,
  };
}

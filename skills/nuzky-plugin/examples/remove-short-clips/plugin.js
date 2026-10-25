// The main track closes the gap a deleted clip leaves, and what lies over it on other tracks moves with it.
function run(ctx) {
  const shortest = Math.round(ctx.inputs.minSeconds * 1000000);
  const ids = [];
  for (const track of ctx.project.tracks) {
    if (ctx.inputs.tracks === "main" && track.id !== "main") continue;
    for (const clip of track.clips) {
      if (clip.durationUs < shortest) ids.push(clip.id);
    }
  }
  nuzky.log("clips shorter than", ctx.inputs.minSeconds, "s:", ids);
  if (ids.length === 0) return { edits: [], message: "No clip is that short." };
  return {
    edits: [{ type: "deleteClips", clipIds: ids }],
    message: `Removed ${ids.length} clip${ids.length === 1 ? "" : "s"}.`,
  };
}

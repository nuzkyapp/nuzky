// addText always lasts 3 seconds and an edit cannot name the clip another edit of the same batch makes, so the card
// is pasted as a whole clip with its own length; pasting gives it a new id on a free text track.
function run(ctx) {
  const end = ctx.project.duration_us;
  if (end === 0) return { edits: [], message: "The timeline is empty." };
  const length = Math.min(Math.round(ctx.inputs.seconds * 1000000), end);
  const card = {
    id: "end-card",
    startUs: 0,
    durationUs: length,
    content: {
      type: "text",
      text: ctx.inputs.text,
      style: { fontSize: 96, color: ctx.inputs.color, bold: ctx.inputs.bold },
      transform: { x: 0, y: 0.3, scale: 1, rotation: 0, opacity: 1 },
    },
  };
  return {
    edits: [{ type: "pasteClips", clips: [{ clip: card, trackId: "end-card", trackKind: "text" }], atUs: end - length }],
    message: `Added "${ctx.inputs.text}" over the last ${length / 1000000} s.`,
  };
}

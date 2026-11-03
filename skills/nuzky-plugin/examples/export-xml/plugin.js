// Final Cut Pro 7 XML counts in frames of the timeline's rate.
function frames(us, fps) {
  return Math.round((us * fps) / 1000000);
}

function escape(text) {
  return String(text).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

// file://localhost/ followed by the path with forward slashes, each part percent-encoded.
function pathUrl(path) {
  const parts = path.replace(/\\/g, "/").split("/").map((part, i) => (i === 0 && /^[A-Za-z]:$/.test(part) ? part : encodeURIComponent(part)));
  const joined = parts.join("/");
  return "file://localhost" + (joined.startsWith("/") ? "" : "/") + joined;
}

function rate(fps) {
  return `<rate><timebase>${fps}</timebase><ntsc>FALSE</ntsc></rate>`;
}

function run(ctx) {
  // The preview reaches no file, so it says nothing; the button writes the file.
  if (ctx.preview) return [];
  const project = ctx.project;
  const fps = project.canvas.fps;
  const assets = new Map(project.assets.map((asset) => [asset.id, asset]));
  const written = new Set();
  let skipped = 0;
  let item = 0;

  // The first clip of a file describes it; later ones name it by its id.
  function file(asset) {
    const id = `file-${asset.id}`;
    if (written.has(id)) return `<file id="${escape(id)}"/>`;
    written.add(id);
    const length = asset.kind === "image" ? frames(project.duration_us, fps) : frames(asset.durationUs, fps);
    const video = asset.kind === "audio" ? "" : `<video><samplecharacteristics><width>${asset.width}</width><height>${asset.height}</height></samplecharacteristics></video>`;
    const audio = asset.hasAudio ? "<audio><channelcount>2</channelcount></audio>" : "";
    return `<file id="${escape(id)}"><name>${escape(asset.name)}</name><pathurl>${escape(pathUrl(ctx.paths.media[asset.id]))}</pathurl>${rate(fps)}<duration>${length}</duration><media>${video}${audio}</media></file>`;
  }

  function clipItem(clip, asset) {
    const content = clip.content;
    item += 1;
    const start = frames(clip.startUs, fps);
    const end = start + Math.max(1, frames(clip.durationUs, fps));
    const from = asset.kind === "image" ? 0 : frames(content.sourceInUs, fps);
    return `<clipitem id="clipitem-${item}"><name>${escape(asset.name)}</name>${rate(fps)}<start>${start}</start><end>${end}</end><in>${from}</in><out>${from + (end - start)}</out>${file(asset)}</clipitem>`;
  }

  const video = [];
  const audio = [];
  for (const track of project.tracks) {
    const pictures = [];
    const sounds = [];
    for (const clip of track.clips) {
      const asset = clip.content.type === "media" ? assets.get(clip.content.assetId) : undefined;
      if (!asset) {
        skipped += 1;
        continue;
      }
      if (clip.content.speed !== 1 || (clip.effects ?? []).length) skipped += 1;
      if (asset.kind !== "audio") pictures.push(clipItem(clip, asset));
      if (asset.kind !== "image" && asset.hasAudio && !track.muted) sounds.push(clipItem(clip, asset));
    }
    if (pictures.length) video.push(`<track>${pictures.join("")}</track>`);
    if (sounds.length) audio.push(`<track>${sounds.join("")}</track>`);
  }

  const duration = Math.max(0, ...project.tracks.flatMap((t) => t.clips.map((c) => frames(c.startUs + c.durationUs, fps))));
  const format = `<format><samplecharacteristics>${rate(fps)}<width>${project.canvas.width}</width><height>${project.canvas.height}</height></samplecharacteristics></format>`;
  const xml = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE xmeml>
<xmeml version="4"><sequence id="sequence-1"><name>${escape(project.name || "Nuzky")}</name><duration>${duration}</duration>${rate(fps)}<media><video>${format}${video.join("")}</video><audio>${audio.join("")}</audio></media></sequence></xmeml>
`;
  nuzky.fs.write(ctx.file, xml);
  const name = ctx.file.replace(/\\/g, "/").split("/").pop();
  const left = skipped ? ` Texts, effects and speed changes stay only in Nuzky (${skipped} clip${skipped === 1 ? "" : "s"}).` : "";
  return { edits: [], message: `Saved ${name}.${left}` };
}

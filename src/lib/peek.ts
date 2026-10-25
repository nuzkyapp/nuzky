import { isCaptionTrack, projectDuration } from "./store";
import { formatDuration, formatTime } from "./time";
import type { Clip, ClipContent, Project, Transform } from "./types";

/** How a clip changes when the step is taken: it comes back, goes away, moves, gets another length, or something else changes. */
export type PeekMark = "back" | "gone" | "moved" | "trimmed" | "changed";

/** The word on a marked clip, so the mark never rests on colour alone. */
export const PEEK_WORDS: Record<PeekMark, string> = { back: "Back", gone: "Removed", moved: "Moved", trimmed: "Trimmed", changed: "Changed" };

type Placed = { clip: Clip; caption: boolean; trackId: string };

function placed(project: Project) {
  const all = new Map<string, Placed>();
  for (const t of project.tracks) for (const clip of t.clips) all.set(clip.id, { clip, caption: isCaptionTrack(t), trackId: t.id });
  return all;
}

function clipName(project: Project, clip: Clip) {
  const c = clip.content;
  if (c.type === "text") return `“${c.text.length > 24 ? `${c.text.slice(0, 23)}…` : c.text}”`;
  return project.assets.find((a) => a.id === c.assetId)?.name ?? "a clip";
}

/** One clip by name, else how many, as captions when they all are. */
function some(project: Project, items: Placed[]) {
  if (items.length === 1) return clipName(project, items[0].clip);
  return `${items.length} ${items.every((p) => p.caption) ? "captions" : "clips"}`;
}

const percent = (v: number) => `${Math.round(v * 100)} %`;
/** Transform values with the inspector's names; one that changes alone is said with its values. */
const TRANSFORM: [keyof Transform, string, ((v: number) => string) | null][] = [
  ["opacity", "Opacity", percent],
  ["scale", "Scale", percent],
  ["rotation", "Rotation", (v) => `${Math.round(v)}°`],
  ["x", "Position", null],
  ["y", "Position", null],
  ["crop", "Crop and shape", null],
];
/** Names for the other fields that change, as the inspector calls them. */
const FIELDS: Record<string, string> = {
  text: "Text",
  style: "Style",
  adjust: "Adjust",
  volume: "Volume",
  speed: "Speed",
  keepPitch: "Speed",
  fadeInUs: "Fade in",
  fadeOutUs: "Fade out",
  shape: "Crop and shape",
  background: "Background",
  sourceInUs: "Start in the source",
  cleanVoice: "Voice",
  dereverb: "Voice",
  normalize: "Voice",
  leveler: "Voice",
  duckDb: "Lower under speech",
};
const CLIP_FIELDS: [keyof Clip, string][] = [
  ["keyframes", "Keyframes"],
  ["animIn", "Animation"],
  ["animOut", "Animation"],
  ["transitionIn", "Transition"],
];

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/** What changes on one clip, as "Opacity 40 % → 100 %" when one value says it, else the names of what changes. */
function changeText(name: string, now: Clip, then: Clip) {
  const a = now.content as ClipContent & Record<string, unknown>;
  const b = then.content as ClipContent & Record<string, unknown>;
  const names = new Set<string>();
  let value: string | null = null;
  for (const [k, label, format] of TRANSFORM)
    if (!same(a.transform[k], b.transform[k])) {
      names.add(label);
      value = format ? `${label} ${format(a.transform[k] as number)} → ${format(b.transform[k] as number)}` : null;
    }
  if (a.type === "media" && b.type === "media" && a.volume !== b.volume) value = `Volume ${percent(a.volume)} → ${percent(b.volume)}`;
  for (const k of new Set([...Object.keys(a), ...Object.keys(b)])) if (FIELDS[k] && !same(a[k], b[k])) names.add(FIELDS[k]);
  for (const [k, label] of CLIP_FIELDS) if (!same(now[k], then[k])) names.add(label);
  if (names.size === 1 && value) return `${name}: ${value}`;
  return names.size ? `${name}: ${[...names].join(", ")}` : `${name} changes`;
}

/**
 * How the timeline changes from `now` to `then`, the project undo or redo would restore. When the step takes
 * clips away and the others keep their lengths and only shift, the lanes draw `now` with the removed clips marked,
 * so they are seen where they are; otherwise they draw `then`. `marks` are for the drawn project's clips; `lines` say it in words. Clips that only shift with a cut are
 * marked but not described, since the cut says it. `ahead` is for a step not taken yet, such as a plugin's, whose new
 * clips and media are added rather than brought back.
 */
export function describePeek(now: Project, then: Project, ahead = false): { draw: "now" | "then"; marks: Map<string, PeekMark>; lines: string[] } {
  const before = placed(now);
  const after = placed(then);
  const marks = new Map<string, PeekMark>();
  const back: Placed[] = [];
  const gone: Placed[] = [];
  const trimmed: [Clip, Clip][] = [];
  const moved: Clip[] = [];
  const shifted: Placed[] = [];
  const changed: [Clip, Clip][] = [];
  for (const [id, p] of after) {
    const old = before.get(id);
    const { clip } = p;
    if (!old) {
      marks.set(id, "back");
      back.push(p);
    } else if (old.clip.durationUs !== clip.durationUs) {
      marks.set(id, "trimmed");
      trimmed.push([old.clip, clip]);
    } else if (old.trackId !== p.trackId || old.clip.startUs !== clip.startUs) {
      marks.set(id, "moved");
      if (old.trackId === p.trackId && same({ ...old.clip, startUs: 0 }, { ...clip, startUs: 0 })) shifted.push(p);
      else moved.push(clip);
    } else if (!same(old.clip, clip)) {
      marks.set(id, "changed");
      changed.push([old.clip, clip]);
    }
  }
  for (const [id, p] of before)
    if (!after.has(id)) {
      marks.set(id, "gone");
      gone.push(p);
    }

  const lines: string[] = [];
  const media = then.assets.filter((a) => !now.assets.some((b) => b.id === a.id));
  const files = media.length === 1 ? media[0].name : `${media.length} media files`;
  const uses = (p: Placed, id: string) => p.clip.content.type === "media" && p.clip.content.assetId === id;
  // Only a plugin's own files join ahead of its step; they lead, so a list of changed clips never hides them.
  const theirs = ahead && media.length > 0 && back.length > 0 && back.every((p) => media.some((a) => uses(p, a.id))) && media.every((a) => back.some((p) => uses(p, a.id)));
  if (ahead && media.length) lines.push(theirs ? `Adds the plugin's ${files} to the video and Media` : `Adds the plugin's ${files} to Media`);
  if (back.length && !theirs) lines.push(`${ahead ? "Adds" : "Brings back"} ${some(then, back)}`);
  if (gone.length) lines.push(`Removes ${some(now, gone)}`);
  for (const [a, b] of trimmed) lines.push(`${clipName(then, b)} length ${formatDuration(a.durationUs)} → ${formatDuration(b.durationUs)}`);
  for (const c of moved) lines.push(`${clipName(then, c)} moves to ${formatTime(c.startUs)}`);
  if (shifted.length && !back.length && !gone.length && !trimmed.length && !moved.length) lines.push(`Moves ${some(then, shifted)}`);
  for (const [a, b] of changed) lines.push(changeText(clipName(then, b), a, b));
  if (now.name !== then.name) lines.push(`Name “${now.name}” → “${then.name}”`);
  if (now.canvas.width !== then.canvas.width || now.canvas.height !== then.canvas.height)
    lines.push(`Format ${now.canvas.width} × ${now.canvas.height} → ${then.canvas.width} × ${then.canvas.height}`);
  if (media.length && !ahead) lines.push(`Brings back ${files} in Media`);
  const dropped = now.assets.filter((a) => !then.assets.some((b) => b.id === a.id));
  if (dropped.length) lines.push(`Removes ${dropped.length === 1 ? dropped[0].name : `${dropped.length} media files`} from Media`);
  const tracks = (p: Project) => p.tracks.map(({ clips: _, ...t }) => t);
  if (!same(tracks(now), tracks(then)) && !lines.length) lines.push("Changes a track's settings");
  const shown = lines.length > 5 ? [...lines.slice(0, 4), `and ${lines.length - 4} more changes`] : lines;
  const [from, to] = [projectDuration(now), projectDuration(then)];
  if (from !== to) shown.push(`Video length ${formatDuration(from)} → ${formatDuration(to)}`);
  // Clips that only shift or change their look sit where they are now too; a new length or place needs `then`.
  return { draw: gone.length && !back.length && !trimmed.length && !moved.length ? "now" : "then", marks, lines: shown };
}

import { projectDuration } from "./store";
import { formatDuration, formatTime } from "./time";
import type { Clip, ClipContent, Project, Transform } from "./types";

/** How a clip on the timeline changes when the step is taken back: it returns, it only moves or trims, or something else changes. */
export type PeekMark = "back" | "moved" | "changed";

type Placed = { clip: Clip; trackId: string };

function placed(project: Project) {
  const all = new Map<string, Placed>();
  for (const t of project.tracks) for (const clip of t.clips) all.set(clip.id, { clip, trackId: t.id });
  return all;
}

function clipName(project: Project, clip: Clip) {
  const c = clip.content;
  if (c.type === "text") return `“${c.text.length > 24 ? `${c.text.slice(0, 23)}…` : c.text}”`;
  return project.assets.find((a) => a.id === c.assetId)?.name ?? "a clip";
}

const percent = (v: number) => `${Math.round(v * 100)} %`;
const TRANSFORM: [keyof Transform, string, (v: number) => string][] = [
  ["opacity", "Opacity", percent],
  ["scale", "Scale", percent],
  ["rotation", "Rotation", (v) => `${Math.round(v)}°`],
];
/** Names for the fields that change, in the order a person reads them. */
const FIELDS: Record<string, string> = {
  text: "text",
  style: "style",
  transform: "position",
  adjust: "colour",
  volume: "volume",
  speed: "speed",
  fadeInUs: "fade in",
  fadeOutUs: "fade out",
  shape: "shape",
  background: "background",
  sourceInUs: "start in the source",
  keyframes: "keyframes",
  animIn: "animation",
  animOut: "animation",
  transitionIn: "transition",
};

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/** What changes on one clip, as "Opacity 40 % → 100 %" when one value says it, else the names of the fields. */
function changeText(name: string, now: Clip, then: Clip) {
  const a = now.content as ClipContent & Record<string, unknown>;
  const b = then.content as ClipContent & Record<string, unknown>;
  if (same({ ...a, transform: null }, { ...b, transform: null }) && same({ ...now, content: null }, { ...then, content: null })) {
    const moved = TRANSFORM.filter(([k]) => a.transform[k] !== b.transform[k]);
    const rest = (Object.keys(a.transform) as (keyof Transform)[]).some((k) => !TRANSFORM.some(([t]) => t === k) && !same(a.transform[k], b.transform[k]));
    if (moved.length === 1 && !rest) {
      const [k, label, format] = moved[0];
      return `${name}: ${label} ${format(a.transform[k] as number)} → ${format(b.transform[k] as number)}`;
    }
  }
  if (a.type === "media" && b.type === "media" && same({ ...a, volume: 0 }, { ...b, volume: 0 }) && same({ ...now, content: null }, { ...then, content: null }))
    return `${name}: Volume ${percent(a.volume)} → ${percent(b.volume)}`;
  const fields = new Set<string>();
  for (const k of new Set([...Object.keys(a), ...Object.keys(b)])) if (FIELDS[k] && !same(a[k], b[k])) fields.add(FIELDS[k]);
  for (const k of ["keyframes", "animIn", "animOut", "transitionIn"] as const) if (!same(now[k], then[k])) fields.add(FIELDS[k]);
  return fields.size ? `${name}: ${[...fields].join(", ")}` : `Changes ${name}`;
}

/**
 * How the timeline changes from `now` to `then`, the project undo or redo would restore: a mark for each clip of
 * `then` that changes, and up to four lines that say it in words, then how many more. Clips that only shift with
 * a cut on the main track are marked but not described, since the cut says it.
 */
export function describePeek(now: Project, then: Project): { marks: Map<string, PeekMark>; lines: string[] } {
  const before = placed(now);
  const after = placed(then);
  const marks = new Map<string, PeekMark>();
  const back: Clip[] = [];
  const gone: Clip[] = [];
  const moved: [Clip, Clip][] = [];
  const shifted: Clip[] = [];
  const changed: [Clip, Clip][] = [];
  for (const [id, { clip, trackId }] of after) {
    const old = before.get(id);
    if (!old) {
      marks.set(id, "back");
      back.push(clip);
    } else if (old.trackId !== trackId || old.clip.startUs !== clip.startUs || old.clip.durationUs !== clip.durationUs) {
      marks.set(id, "moved");
      if (old.trackId === trackId && old.clip.durationUs === clip.durationUs && same({ ...old.clip, startUs: 0 }, { ...clip, startUs: 0 })) shifted.push(clip);
      else moved.push([old.clip, clip]);
    } else if (!same(old.clip, clip)) {
      marks.set(id, "changed");
      changed.push([old.clip, clip]);
    }
  }
  for (const [id, { clip }] of before) if (!after.has(id)) gone.push(clip);

  const lines: string[] = [];
  const many = (n: number) => `${n} clips`;
  if (back.length) lines.push(`Brings back ${back.length === 1 ? clipName(then, back[0]) : many(back.length)}`);
  if (gone.length) lines.push(`Removes ${gone.length === 1 ? clipName(now, gone[0]) : many(gone.length)}`);
  for (const [a, b] of moved) {
    const name = clipName(then, b);
    lines.push(a.durationUs !== b.durationUs ? `${name}: ${formatDuration(a.durationUs)} → ${formatDuration(b.durationUs)}` : `${name} moves to ${formatTime(b.startUs)}`);
  }
  if (shifted.length && !back.length && !gone.length && !moved.length) lines.push(`Moves ${shifted.length === 1 ? clipName(then, shifted[0]) : many(shifted.length)}`);
  for (const [a, b] of changed) lines.push(changeText(clipName(then, b), a, b));
  if (now.name !== then.name) lines.push(`Name “${now.name}” → “${then.name}”`);
  if (now.canvas.width !== then.canvas.width || now.canvas.height !== then.canvas.height)
    lines.push(`Format ${now.canvas.width} × ${now.canvas.height} → ${then.canvas.width} × ${then.canvas.height}`);
  const media = then.assets.filter((a) => !now.assets.some((b) => b.id === a.id));
  if (media.length) lines.push(`Brings back ${media.length === 1 ? media[0].name : `${media.length} media files`} in Media`);
  const dropped = now.assets.filter((a) => !then.assets.some((b) => b.id === a.id));
  if (dropped.length) lines.push(`Removes ${dropped.length === 1 ? dropped[0].name : `${dropped.length} media files`} from Media`);
  if (!lines.length) lines.push("Changes project settings");
  const shown = lines.length > 5 ? [...lines.slice(0, 4), `and ${lines.length - 4} more changes`] : lines;
  const [from, to] = [projectDuration(now), projectDuration(then)];
  if (from !== to) shown.push(`Length ${formatTime(from)} → ${formatTime(to)}`);
  return { marks, lines: shown };
}

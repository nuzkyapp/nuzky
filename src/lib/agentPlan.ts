import { MAIN_TRACK, isCaptionTrack } from "./store";
import type { Clip, Project, Track } from "./types";

/** What an agent's edit did on the timeline, which its pointer acts out. Times are the project's before the edit. */
export type Action =
  /** A piece taken out; on the main track the clips after it close the gap. */
  | { kind: "cut"; trackId: string; fromUs: number; toUs: number; whole: boolean }
  | { kind: "move"; clipId: string; trackId: string; fromUs: number; toUs: number; durationUs: number }
  /** The clip's start or end moved in or out; `fromUs` is where that edge was. */
  | { kind: "trim"; clipId: string; trackId: string; edge: "start" | "end"; fromUs: number }
  | { kind: "split"; trackId: string; atUs: number }
  /** In the project after the edit. */
  | { kind: "add"; clipIds: string[]; trackId: string; atUs: number }
  | { kind: "change"; clipId: string; trackId: string; atUs: number; durationUs: number };

export interface Plan {
  /** The actions of the edit's main kind, in time order; the rest of the edit lands with them. */
  actions: Action[];
  /** What the pointer's label says it does. */
  step: string;
}

/** Times within this many µs are the same, as speeds round source times. */
const SAME = 2_000;
/** A clip that went this far off where the main track's ripple took it was moved. */
const MOVED = 20_000;

interface Span {
  clip: Clip;
  track: Track;
  assetId: string;
  inUs: number;
  outUs: number;
  speed: number;
}

/** Clips that play their file forward at one speed, whose frames can be followed from one project to the other. */
function spans(project: Project): Span[] {
  return project.tracks.flatMap((track) =>
    track.clips.flatMap((clip) => {
      const c = clip.content;
      if (c.type !== "media" || c.speed <= 0 || c.reverse || c.ramp?.length) return [];
      return [{ clip, track, assetId: c.assetId, inUs: c.sourceInUs, outUs: c.sourceInUs + clip.durationUs * c.speed, speed: c.speed }];
    }),
  );
}

/** `[from, to]` less the parts of `taken` over it. */
function subtract(from: number, to: number, taken: [number, number][]): [number, number][] {
  let left: [number, number][] = [[from, to]];
  for (const [a, b] of taken) left = left.flatMap(([x, y]) => (b <= x || a >= y ? [[x, y]] : ([[x, a], [b, y]] as [number, number][]).filter(([p, q]) => q - p > SAME)));
  return left;
}

const nameOf = (project: Project, clip: Clip) => {
  const c = clip.content;
  const name = c.type === "text" ? `“${c.text.trim()}”` : c.type === "adjustment" ? "Adjustment" : (project.assets.find((a) => a.id === c.assetId)?.name ?? "a clip");
  return name.length > 30 ? `${name.slice(0, 28)}…` : name;
};

const count = (n: number, one: string, many: string) => (n === 1 ? one : many.replace("#", String(n)));

export function plan(before: Project, after: Project): Plan | null {
  const was = new Map(before.tracks.flatMap((t) => t.clips.map((c) => [c.id, { clip: c, track: t }] as const)));
  const now = new Map(after.tracks.flatMap((t) => t.clips.map((c) => [c.id, { clip: c, track: t }] as const)));
  const beforeSpans = spans(before);
  const afterSpans = spans(after);
  const sameFile = (list: Span[], s: Span) => list.filter((o) => o.assetId === s.assetId && o.track.id === s.track.id);
  const added = new Set(after.tracks.flatMap((t) => t.clips.filter((c) => !was.has(c.id)).map(() => t.id)));

  const cuts: Action[] = [];
  const trims: Action[] = [];
  /** Clips that lost frames to a cut or a trim. */
  const cutFrom = new Set<string>();
  for (const s of beforeSpans) {
    const survives = now.get(s.clip.id);
    // A clip that now shows another file, or plays its own backwards or on a ramp, changed; nothing of it was cut.
    if (survives && !afterSpans.some((o) => o.clip.id === s.clip.id && o.assetId === s.assetId)) continue;
    // A clip moved to another track keeps its frames.
    const kept = afterSpans.filter((o) => o.assetId === s.assetId && (o.track.id === s.track.id || o.clip.id === s.clip.id));
    for (const [u0, u1] of subtract(s.inUs, s.outUs, kept.map((k) => [k.inUs, k.outUs]))) {
      const fromUs = s.clip.startUs + (u0 - s.inUs) / s.speed;
      const toUs = s.clip.startUs + (u1 - s.inUs) / s.speed;
      const atStart = Math.abs(u0 - s.inUs) < SAME;
      const atEnd = Math.abs(u1 - s.outUs) < SAME;
      // A remainder of the file going on after the gap makes it a cut; the clip's own edge moving in is a trim.
      const goesOn = kept.some((k) => k.clip.id !== s.clip.id && Math.abs(k.inUs - u1) < SAME);
      if (survives && atStart && !atEnd) trims.push({ kind: "trim", clipId: s.clip.id, trackId: s.track.id, edge: "start", fromUs });
      else if (survives && atEnd && !atStart && !goesOn) trims.push({ kind: "trim", clipId: s.clip.id, trackId: s.track.id, edge: "end", fromUs: toUs });
      else cuts.push({ kind: "cut", trackId: s.track.id, fromUs, toUs, whole: atStart && atEnd });
      cutFrom.add(s.clip.id);
    }
  }
  for (const [id, { clip, track }] of was) {
    if (now.has(id) || beforeSpans.some((s) => s.clip.id === id)) continue;
    // A track whose clips were made again, as captions are, was replaced rather than cut.
    if (!added.has(track.id)) cuts.push({ kind: "cut", trackId: track.id, fromUs: clip.startUs, toUs: clip.startUs + clip.durationUs, whole: true });
  }

  // Where the main track's pictures went, so clips the ripple carried along are not taken for moved ones.
  const mainBefore = beforeSpans.filter((s) => s.track.id === MAIN_TRACK);
  const mainAfter = afterSpans.filter((s) => s.track.id === MAIN_TRACK);
  const ripple = (t: number) => {
    for (const s of mainBefore) {
      if (t < s.clip.startUs - SAME || t > s.clip.startUs + s.clip.durationUs + SAME) continue;
      const u = s.inUs + (t - s.clip.startUs) * s.speed;
      const there = mainAfter.find((o) => o.assetId === s.assetId && u >= o.inUs - SAME && u <= o.outUs + SAME);
      if (there) return there.clip.startUs + (u - there.inUs) / there.speed - t;
    }
    return 0;
  };

  const splits: Action[] = [];
  /** Clips a split shortened, which were neither trimmed nor cut. */
  const continued = new Set<string>();
  const adds = new Map<string, Clip[]>();
  for (const [id, { clip, track }] of now) {
    if (was.has(id)) continue;
    const s = afterSpans.find((x) => x.clip.id === id);
    const from = s && sameFile(beforeSpans, s).find((o) => s.inUs >= o.inUs - SAME && s.inUs < o.outUs - SAME);
    // A copy of frames another clip still shows is new, as a duplicate is.
    const copy = s && sameFile(afterSpans, s).some((o) => o.clip.id !== id && o.inUs < s.outUs - SAME && o.outUs > s.inUs + SAME);
    if (!s || !from || copy) {
      adds.set(track.id, [...(adds.get(track.id) ?? []), clip]);
      continue;
    }
    // A piece that starts where the clip it came from now ends was split off; one after a gap is a cut's remainder.
    const rest = sameFile(afterSpans, s).find((o) => o.clip.id !== id && Math.abs(o.outUs - s.inUs) < SAME);
    if (rest) {
      splits.push({ kind: "split", trackId: track.id, atUs: from.clip.startUs + (s.inUs - from.inUs) / from.speed });
      continued.add(rest.clip.id);
    }
  }

  const reordered = reorderedOnMain(before, after);
  // Everything about a clip but where it sits, as the history's preview compares it.
  const rest = (c: Clip) => JSON.stringify({ ...c, startUs: 0, durationUs: 0 });
  const moves: Action[] = [];
  const changes: Action[] = [];
  for (const [id, { clip: a, track: ta }] of was) {
    const found = now.get(id);
    if (!found) continue;
    const { clip: b, track: tb } = found;
    if (continued.has(id) || trims.some((t) => t.kind === "trim" && t.clipId === id)) continue;
    const sameLength = Math.abs(a.durationUs - b.durationUs) < SAME;
    const shift = b.startUs - a.startUs;
    const carried = ta.id === MAIN_TRACK || tb.keepInPlace ? 0 : ripple(a.startUs);
    // A move shows as a move whatever else changed with it; the rest lands as the clip is dropped.
    if (sameLength && (ta.id !== tb.id || reordered.has(id) || (ta.id !== MAIN_TRACK && Math.abs(shift - carried) > MOVED))) {
      moves.push({ kind: "move", clipId: id, trackId: ta.id, fromUs: a.startUs, toUs: b.startUs, durationUs: a.durationUs });
      continue;
    }
    const ca = a.content;
    const cb = b.content;
    // Another file, or the same one played backwards, at another speed or newly on a ramp (or off it), is a change of
    // the clip, not a trim; a trim of a clip on a ramp works the ramp out again, which stays a trim.
    const swapped =
      ca.type !== cb.type ||
      (ca.type === "media" && cb.type === "media" && (ca.assetId !== cb.assetId || !!ca.reverse !== !!cb.reverse || ca.speed !== cb.speed || !!ca.ramp?.length !== !!cb.ramp?.length));
    if (swapped && ta.id === tb.id) {
      changes.push({ kind: "change", clipId: id, trackId: ta.id, atUs: a.startUs, durationUs: a.durationUs });
      continue;
    }
    // Made longer, or a text or a clip without frames to follow made shorter.
    if (!sameLength && ta.id === tb.id && !cutFrom.has(id)) {
      const edge = Math.abs(b.startUs - a.startUs - (ta.id === MAIN_TRACK ? 0 : ripple(a.startUs))) < SAME ? "end" : "start";
      trims.push({ kind: "trim", clipId: id, trackId: ta.id, edge, fromUs: edge === "end" ? a.startUs + a.durationUs : a.startUs });
      continue;
    }
    if (sameLength && ta.id === tb.id && rest(a) !== rest(b)) {
      changes.push({ kind: "change", clipId: id, trackId: ta.id, atUs: a.startUs, durationUs: a.durationUs });
    }
  }

  const name = (id: string) => {
    const found = was.get(id);
    return found ? nameOf(before, found.clip) : "a clip";
  };
  const byTime = (list: Action[]) => list.sort((x, y) => start(x) - start(y));
  if (cuts.length) {
    const merged = mergeCuts(cuts);
    const whole = merged.every((c) => c.kind === "cut" && c.whole);
    const removed = [...was.values()].filter(({ clip }) => !now.has(clip.id));
    const step = whole && removed.length === 1 ? `Removing ${nameOf(before, removed[0].clip)}` : whole ? `Removing ${removed.length} clips` : count(merged.length, "Cutting out a piece", "Cutting out # pieces");
    return { actions: merged, step };
  }
  if (moves.length) return { actions: byTime(moves), step: count(moves.length, `Moving ${name((moves[0] as { clipId: string }).clipId)}`, "Moving # clips") };
  if (trims.length) return { actions: byTime(trims), step: count(trims.length, `Trimming ${name((trims[0] as { clipId: string }).clipId)}`, "Trimming # clips") };
  if (splits.length) {
    const first = splits[0] as { trackId: string; atUs: number };
    const clip = before.tracks.find((t) => t.id === first.trackId)?.clips.find((c) => c.startUs <= first.atUs && c.startUs + c.durationUs >= first.atUs);
    return { actions: byTime(splits), step: count(splits.length, `Splitting ${clip ? nameOf(before, clip) : "a clip"}`, "Splitting # clips") };
  }
  if (adds.size) {
    const actions: Action[] = [...adds].map(([trackId, clips]) => {
      const sorted = [...clips].sort((x, y) => x.startUs - y.startUs);
      return { kind: "add", trackId, clipIds: sorted.map((c) => c.id), atUs: sorted[0].startUs };
    });
    const all = [...adds.values()].flat();
    const captions = [...adds.keys()].every((id) => {
      const track = after.tracks.find((t) => t.id === id);
      return !!track && isCaptionTrack(track);
    });
    const step = captions ? count(all.length, "Adding a caption", "Adding # captions") : count(all.length, `Adding ${nameOf(after, all[0])}`, "Adding # clips");
    return { actions: byTime(actions), step };
  }
  if (changes.length) return { actions: byTime(changes), step: count(changes.length, `Changing ${name((changes[0] as { clipId: string }).clipId)}`, "Changing # clips") };
  return null;
}

/**
 * Main-track clips that changed places with others: those off the longest run that kept its order. Two runs as long
 * are told apart by how far their clips went, so of two swapped clips the one that went further moved: a short clip
 * put before a long one, as a hook is.
 */
function reorderedOnMain(before: Project, after: Project): Set<string> {
  const main = (p: Project) => (p.tracks.find((t) => t.id === MAIN_TRACK)?.clips ?? []).toSorted((x, y) => x.startUs - y.startUs);
  const then = main(before);
  const ids = then.map((c) => c.id);
  const now = main(after).filter((c) => ids.includes(c.id));
  const at = now.map((c) => ids.indexOf(c.id));
  const far = now.map((c, i) => Math.abs(c.startUs - then[at[i]].startUs));
  type Run = { ids: number[]; far: number };
  const better = (x: Run, y: Run) => x.ids.length > y.ids.length || (x.ids.length === y.ids.length && x.far < y.far);
  const best: Run[] = [];
  for (let i = 0; i < at.length; i++) {
    let longest: Run = { ids: [], far: 0 };
    for (let j = 0; j < i; j++) if (at[j] < at[i] && better(best[j], longest)) longest = best[j];
    best.push({ ids: [...longest.ids, i], far: longest.far + far[i] });
  }
  const kept = new Set(best.reduce<Run>((x, y) => (better(y, x) ? y : x), { ids: [], far: 0 }).ids.map((i) => now[i].id));
  return new Set(now.filter((c) => !kept.has(c.id)).map((c) => c.id));
}

function start(a: Action) {
  return a.kind === "cut" || a.kind === "trim" || a.kind === "move" ? a.fromUs : a.atUs;
}

/** Pieces cut at the same time from several tracks, as captions over a cut, are one cut on the lowest track named. */
function mergeCuts(cuts: Action[]): Action[] {
  const sorted = (cuts as Extract<Action, { kind: "cut" }>[]).toSorted((a, b) => a.fromUs - b.fromUs || (a.trackId === MAIN_TRACK ? -1 : 1));
  const out: Extract<Action, { kind: "cut" }>[] = [];
  for (const c of sorted) {
    const last = out.at(-1);
    if (last && c.fromUs <= last.toUs + SAME && (last.trackId === c.trackId || last.trackId === MAIN_TRACK)) {
      last.toUs = Math.max(last.toUs, c.toUs);
      last.whole &&= c.whole;
    } else out.push({ ...c });
  }
  return out;
}

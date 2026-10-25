import { create } from "zustand";
import { api, errorText, plainError } from "./api";
import { aiLocked, enqueue, undoAction, useEditor } from "./store";
import type { BeatCutDone, BeatPlans, Project, Rhythm, SongBeats } from "./types";

export const RHYTHMS: { id: Rhythm; label: string; does: string }[] = [
  { id: "calm", label: "Calm", does: "A cut on every bar after the intro." },
  { id: "builds", label: "Builds to the drop", does: "Every bar until the drop, then every beat." },
  { id: "fast", label: "Fast", does: "A cut on every beat after the intro." },
];

/** What the last cut of a music clip did, for the section after it. */
export type MadeCut = { rhythm: Rhythm; clips: number; repeated: number; captionsRemoved: number; keepSound: boolean };

type Beats = {
  /** By music clip: what each rhythm would cut, or why that cannot be known. */
  plans: Record<string, BeatPlans>;
  errors: Record<string, string>;
  /** By the song's file path, for the ticks on its clips; only songs with a steady beat. */
  songs: Record<string, SongBeats>;
  chosen: Record<string, Rhythm>;
  made: Record<string, MadeCut>;
  keepSound: boolean;
  cutting: boolean;
  /** The clip whose section Cut to beat in Project status asked to show. */
  reveal: string | null;
};

export const useBeats = create<Beats>(() => ({ plans: {}, errors: {}, songs: {}, chosen: {}, made: {}, keepSound: false, cutting: false, reveal: null }));

// Another project can reuse clip ids and asset paths for other files.
useEditor.subscribe((s, prev) => {
  if (s.snap?.sessionEpoch !== prev.snap?.sessionEpoch) useBeats.setState({ plans: {}, errors: {}, songs: {}, chosen: {}, made: {} });
});

/** The song a beat cut goes to: a sound on a music track, the one an agent's cut_to_beats would take when it is alone. */
export function musicClipOf(project: Project): { clipId: string; name: string } | null {
  for (const track of project.tracks) {
    if (track.kind !== "audio" || !track.keepInPlace) continue;
    for (const clip of track.clips) {
      const c = clip.content;
      const asset = c.type === "media" ? project.assets.find((a) => a.id === c.assetId) : undefined;
      if (asset?.hasAudio) return { clipId: clip.id, name: asset.name };
    }
  }
  return null;
}

type Load = { revision?: number; find: boolean; again: boolean; done: Promise<void> };
const loading = new Map<string, Load>();

/**
 * What each rhythm would cut for this music clip, against the project as it is. When the song's beat is not known
 * yet and `find` is set, it starts finding it; the job's end loads the plans again. One request per clip at a time:
 * a call while the project moved on asks again once that one ends, and only the newest answer is kept.
 */
export function loadBeats(clipId: string, find: boolean): Promise<void> {
  const running = loading.get(clipId);
  if (running) {
    if (running.revision !== useEditor.getState().snap?.revision || (find && !running.find)) {
      running.again = true;
      running.find ||= find;
    }
    return running.done;
  }
  const load: Load = { find, again: false, done: Promise.resolve() };
  load.done = (async () => {
    try {
      do {
        load.again = false;
        await loadOnce(clipId, load);
      } while (load.again);
    } finally {
      loading.delete(clipId);
    }
  })();
  loading.set(clipId, load);
  return load.done;
}

async function loadOnce(clipId: string, load: Load) {
  const snap = useEditor.getState().snap;
  if (!snap) return;
  load.revision = snap.revision;
  const epoch = snap.sessionEpoch;
  const content = snap.project.tracks.flatMap((t) => t.clips).find((c) => c.id === clipId)?.content;
  const path = content?.type === "media" ? snap.project.assets.find((a) => a.id === content.assetId)?.path : undefined;
  // An answer for a project that changed since is dropped; the next request brings the newest.
  const current = () => !load.again && useEditor.getState().snap?.sessionEpoch === epoch;
  try {
    const plans = await api.beatPlans(clipId, epoch);
    if (!current()) return;
    useBeats.setState((s) => {
      const errors = { ...s.errors };
      delete errors[clipId];
      const all = { ...s.plans };
      if (plans) all[clipId] = plans;
      else delete all[clipId];
      const songs = { ...s.songs };
      if (path && plans?.steady) songs[path] = plans.song;
      else if (path) delete songs[path];
      return { plans: all, errors, songs };
    });
    if (!plans && load.find) await api.findBeats(clipId, epoch);
  } catch (e) {
    const text = errorText(e);
    if (!current() || text.startsWith("EPOCH_CHANGED")) return;
    // The plans were for the project before; the latest error is what holds now.
    useBeats.setState((s) => {
      const plans = { ...s.plans };
      delete plans[clipId];
      return { plans, errors: { ...s.errors, [clipId]: plainError(text) } };
    });
  }
}

/** Finding a song's beat ended: the plans of its clips on screen come again. */
export function onBeatsFound(assetId: string) {
  const project = useEditor.getState().snap?.project;
  if (!project) return;
  for (const clip of project.tracks.flatMap((t) => t.clips)) {
    if (clip.content.type === "media" && clip.content.assetId === assetId) void loadBeats(clip.id, false);
  }
}

/** Cuts the main track to the song in `rhythm` as one undo step and offers Undo. */
export async function cutToBeat(clipId: string, songName: string, rhythm: Rhythm) {
  if (aiLocked() || useBeats.getState().cutting) return;
  const keepSound = useBeats.getState().keepSound;
  useBeats.setState({ cutting: true });
  const out: { done?: BeatCutDone } = {};
  try {
    await enqueue(async (epoch) => (out.done = await api.cutToBeat(clipId, rhythm, keepSound, epoch)).snapshot);
  } finally {
    useBeats.setState({ cutting: false });
  }
  const done = out.done;
  if (!done) return;
  const made: MadeCut = { rhythm, clips: done.clips, repeated: done.repeated, captionsRemoved: done.captionsRemoved, keepSound };
  useBeats.setState((s) => ({ made: { ...s.made, [clipId]: made }, chosen: { ...s.chosen, [clipId]: rhythm } }));
  const text = `Cut ${done.clips} clips to the beat of ${songName}`;
  useEditor.getState().toast(
    done.repeated > 0
      ? { kind: "warning", text: `${text}. Your videos ran out, so ${done.repeated === 1 ? "1 clip shows" : `${done.repeated} clips show`} footage again.`, action: undoAction(done.snapshot) }
      : { kind: "info", text, action: undoAction(done.snapshot) },
  );
  await loadBeats(clipId, false);
}

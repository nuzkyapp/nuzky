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
  /** By song file, for the ticks on its clips; only songs with a steady beat. */
  songs: Record<string, SongBeats>;
  chosen: Record<string, Rhythm>;
  made: Record<string, MadeCut>;
  keepSound: boolean;
  cutting: boolean;
  /** The clip whose section Cut to beat in Project status asked to show. */
  reveal: string | null;
};

export const useBeats = create<Beats>(() => ({ plans: {}, errors: {}, songs: {}, chosen: {}, made: {}, keepSound: false, cutting: false, reveal: null }));

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

const loading = new Map<string, Promise<void>>();

/**
 * What each rhythm would cut for this music clip, against the project as it is. When the song's beat is not known
 * yet and `find` is set, it starts finding it; the job's end loads the plans again.
 */
export function loadBeats(clipId: string, assetId: string, find: boolean): Promise<void> {
  const key = `${clipId}:${find}`;
  const running = loading.get(key);
  if (running) return running;
  const epoch = useEditor.getState().snap?.sessionEpoch;
  const done = (async () => {
    try {
      const plans = await api.beatPlans(clipId, epoch);
      if (plans) {
        useBeats.setState((s) => {
          const errors = { ...s.errors };
          delete errors[clipId];
          const songs = { ...s.songs };
          if (plans.steady) songs[assetId] = plans.song;
          else delete songs[assetId];
          return { plans: { ...s.plans, [clipId]: plans }, errors, songs };
        });
      } else if (find) await api.findBeats(clipId, epoch);
    } catch (e) {
      const text = errorText(e);
      if (!text.startsWith("EPOCH_CHANGED")) useBeats.setState((s) => ({ errors: { ...s.errors, [clipId]: plainError(text) } }));
    } finally {
      loading.delete(key);
    }
  })();
  loading.set(key, done);
  return done;
}

/** Finding a song's beat ended: the plans of its clips on screen come again. */
export function onBeatsFound(assetId: string) {
  const project = useEditor.getState().snap?.project;
  if (!project) return;
  for (const clip of project.tracks.flatMap((t) => t.clips)) {
    if (clip.content.type === "media" && clip.content.assetId === assetId) void loadBeats(clip.id, assetId, false);
  }
}

/** Cuts the main track to the song in `rhythm` as one undo step and offers Undo. */
export async function cutToBeat(clipId: string, assetId: string, songName: string, rhythm: Rhythm) {
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
  await loadBeats(clipId, assetId, false);
}

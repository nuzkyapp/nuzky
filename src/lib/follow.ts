import { create } from "zustand";
import { api, errorText, plainError } from "./api";
import { NO_CROP } from "./presets";
import { useReframe } from "./reframe";
import { aiLocked, currentEpoch, findClip, useEditor, whenIdle } from "./store";
import type { Asset, Canvas, JobEvent, Snapshot, Transform } from "./types";

/** Left, top, width and height. */
export type Box = [number, number, number, number];

interface FollowState {
  /**
   * The video clip whose picture is being boxed in the preview; once the picture shows, the box in canvas pixels and
   * the corners of the whole picture on the canvas at the playhead.
   */
  picking: { clipId: string; box: Box | null; frame: [number, number][] | null; hidden?: boolean } | null;
  job: string | null;
  /** What the last follow did, offered for Undo while it is the newest step. */
  done: { snap: Snapshot; text: string } | null;
}

export const useFollow = create<FollowState>(() => ({ picking: null, job: null, done: null }));

/**
 * Whether the picture, filling the frame at its zoom beyond filling it, can move to keep something in the frame, as
 * the engine places it (`Placement::has_room`).
 */
export function roomToFollow(asset: Asset, canvas: Canvas, t: Transform): boolean {
  const c = t.crop ?? NO_CROP;
  const shown = [1 - c.left - c.right, 1 - c.top - c.bottom];
  const fit = Math.min(canvas.width / Math.max(1, asset.width), canvas.height / Math.max(1, asset.height));
  const [w, h] = [asset.width * fit, asset.height * fit];
  const filling = Math.max(canvas.width / Math.max(w * shown[0], 1e-3), canvas.height / Math.max(h * shown[1], 1e-3));
  const scale = filling * Math.max(1, t.scale / filling);
  return w * scale * shown[0] - canvas.width > 1 || h * scale * shown[1] - canvas.height > 1;
}

/**
 * Shows the box in the preview over the clip's picture, moving the playhead onto the clip when it is elsewhere. While
 * the video plays it waits until the pause is confirmed, as playing ends choosing.
 */
export function startPicking(clipId: string) {
  if (aiLocked() || useFollow.getState().job) return;
  const open = () => {
    const { snap, timeUs, seek } = useEditor.getState();
    const clip = snap && findClip(snap.project, clipId)?.clip;
    if (!clip) return;
    if (timeUs < clip.startUs || timeUs >= clip.startUs + clip.durationUs) seek(clip.startUs);
    useFollow.setState({ picking: { clipId, box: null, frame: null }, done: null });
    useReframe.setState({ done: null });
  };
  if (!useEditor.getState().playing) return open();
  const paused = useEditor.subscribe((s) => {
    if (s.playing) return;
    paused();
    open();
  });
  useEditor.getState().togglePlay();
}

export function stopPicking() {
  if (useFollow.getState().picking) useFollow.setState({ picking: null });
}

/**
 * Follows `region` (fractions of the clip's upright picture) from the playhead through the clip as a job, which
 * applies it as one undo step; the preview then offers Undo.
 */
export async function follow(clipId: string, region: Box) {
  if (aiLocked() || useFollow.getState().job) return;
  const { timeUs, toast } = useEditor.getState();
  useFollow.setState({ picking: null, done: null });
  useReframe.setState({ done: null });
  // Edits made just before are part of what is followed, not a change that would make it fail.
  await whenIdle();
  try {
    const id = await api.startFollow(clipId, Math.round(timeUs), region, currentEpoch());
    useFollow.setState({ job: id });
    // A job that failed at once may have ended before its id came back.
    const ended = useEditor.getState().jobs[id];
    if (ended && ended.status !== "running") onFollowJob(ended);
  } catch (e) {
    toast({ kind: "error", text: plainError(errorText(e)) });
  }
}

export function onFollowJob(job: JobEvent) {
  if (job.id !== useFollow.getState().job || job.status === "running") return;
  useFollow.setState({ job: null });
  const { snap, toast } = useEditor.getState();
  if (job.status === "done" && job.output) {
    const out = JSON.parse(job.output) as { lostUs: number; revision: number | null; sessionEpoch: string | null };
    if (out.revision === null) {
      toast({ kind: "info", text: "Nothing to change: the picture already keeps it in the frame." });
      return;
    }
    const text = out.lostUs >= 500_000 ? `Followed it · lost it for ${(out.lostUs / 1e6).toFixed(1)} s, held still there` : "Followed it through the clip";
    // The project-changed event of the edit came before the job's end; a later edit already moved past it.
    if (snap && snap.revision === out.revision && snap.sessionEpoch === out.sessionEpoch) useFollow.setState({ done: { snap, text } });
  }
  if (job.status === "failed") toast({ kind: "error", text: `Following failed: ${plainError(job.message ?? "")}` });
}

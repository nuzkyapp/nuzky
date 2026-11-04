import { create } from "zustand";
import { api, errorText, plainError } from "./api";
import { aiLocked, currentEpoch, useEditor, whenIdle } from "./store";
import type { EditCmd, JobEvent, Snapshot } from "./types";

/** A caption, title or sticker over a face, as `nuzky_vision::covered::Covered` reports it. */
export interface Covered {
  clip_id: string;
  kind: "caption" | "text" | "sticker";
  label: string;
  start_us: number;
  end_us: number;
  suggestion: { edits: EditCmd[] } | null;
}

interface FacesState {
  job: string | null;
  /** What the last check found, for the revision it looked at, and the one shown. */
  found: { covered: Covered[]; revision: number; sessionEpoch: string; at: number } | null;
  /** The step that moved them, offered for Undo while it is the newest. */
  moved: { snap: Snapshot; text: string } | null;
}

export const useFaces = create<FacesState>(() => ({ job: null, found: null, moved: null }));

export async function checkFaces() {
  if (aiLocked() || useFaces.getState().job) return;
  await whenIdle();
  useFaces.setState({ found: null, moved: null });
  try {
    const id = await api.startFaceCheck(currentEpoch());
    useFaces.setState({ job: id });
    const ended = useEditor.getState().jobs[id];
    if (ended && ended.status !== "running") onFacesJob(ended);
  } catch (e) {
    useEditor.getState().toast({ kind: "error", text: plainError(errorText(e)) });
  }
}

export function onFacesJob(job: JobEvent) {
  if (job.id !== useFaces.getState().job || job.status === "running") return;
  useFaces.setState({ job: null });
  const { toast } = useEditor.getState();
  if (job.status === "done" && job.output) {
    const out = JSON.parse(job.output) as { covered: Covered[]; revision: number; sessionEpoch: string };
    if (out.covered.length === 0) toast({ kind: "success", text: "No caption or sticker covers a face." });
    else {
      useFaces.setState({ found: { ...out, at: 0 } });
      show(0);
    }
  }
  if (job.status === "failed") {
    const message = job.message ?? "";
    toast({
      kind: "error",
      text: message.includes("VISION_UNAVAILABLE") ? "Faces can't be found on this computer, so the check did not run." : `The face check failed: ${plainError(message)}`,
    });
  }
}

/** Selects the finding `at` and puts the playhead in the middle of where it covers the face. */
export function show(at: number) {
  const found = useFaces.getState().found;
  const item = found?.covered[at];
  if (!found || !item) return;
  useFaces.setState({ found: { ...found, at } });
  const { select, seek } = useEditor.getState();
  select([item.clip_id]);
  seek(Math.round((item.start_us + item.end_us) / 2));
}

/** Whether the findings still describe the project: any edit after the check may have moved what they name. */
export function current(found: FacesState["found"], snap: Snapshot | null) {
  return !!found && !!snap && snap.revision === found.revision && snap.sessionEpoch === found.sessionEpoch;
}

/** Moves every finding that has a free place there, as one undo step. */
export async function moveAll() {
  const { found } = useFaces.getState();
  if (!found || !current(found, useEditor.getState().snap)) return;
  const movable = found.covered.filter((c) => c.suggestion);
  const snap = await useEditor.getState().edit(movable.flatMap((c) => c.suggestion!.edits));
  if (!snap) return;
  const n = movable.length;
  const left = found.covered.length - n;
  const text = `Moved ${n} ${n === 1 ? "layer" : "layers"} off the face` + (left > 0 ? `, ${left} had no free place` : "");
  useFaces.setState({ found: null, moved: { snap, text } });
}

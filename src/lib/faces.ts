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
  time_us: number;
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

// Another project opened: the face check of the one before stopped with it (`jobs::stop_project_work`).
useEditor.subscribe((s, prev) => {
  if (prev.snap && s.snap?.sessionEpoch !== prev.snap.sessionEpoch) useFaces.setState({ job: null });
});

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
  const { toast, snap } = useEditor.getState();
  if (job.status === "done" && job.output) {
    const out = JSON.parse(job.output) as { covered: Covered[]; revision: number; sessionEpoch: string };
    // Another project was opened meanwhile: the result names clips of a project no longer open.
    if (snap?.sessionEpoch !== out.sessionEpoch) return;
    if (out.covered.length === 0) toast({ kind: "success", text: "No caption, text or sticker covers a face." });
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

/** Selects the finding `at` and puts the playhead where it was seen over the face, the time the chip shows. */
export function show(at: number) {
  const found = useFaces.getState().found;
  const item = found?.covered[at];
  if (!found || !item) return;
  useFaces.setState({ found: { ...found, at } });
  const { select, seek } = useEditor.getState();
  select([item.clip_id]);
  seek(item.time_us);
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
  const counted = (["caption", "text", "sticker"] as const).flatMap((kind) => {
    const n = movable.filter((c) => c.kind === kind).length;
    return n ? [`${n} ${kind}${n === 1 ? "" : "s"}`] : [];
  });
  const left = found.covered.length - movable.length;
  const text = `Moved ${counted.join(" and ")} off the face` + (left > 0 ? `, ${left} had no free place` : "");
  useFaces.setState({ found: null, moved: { snap, text } });
}

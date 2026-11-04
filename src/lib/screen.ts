import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api, errorText } from "./api";
import { currentEpoch, undoAction, useEditor } from "./store";
import type { ScreenRecording, ScreenWindow } from "./types";

/** The screen recording running, whose Stop is in the top bar. */
export const useScreen = create<{ recording: ScreenRecording | null; stopping: boolean }>(() => ({ recording: null, stopping: false }));

let listening = false;

/** The recording that runs after the window reloaded, and the end of one the system stopped. */
export function watchScreenRecording() {
  void api.screenRecording().then((recording) => useScreen.setState({ recording }));
  if (listening) return;
  listening = true;
  void listen("screen-recording-ended", () => void stopRecording(true));
}

export async function startRecording(window: ScreenWindow | null) {
  watchScreenRecording();
  try {
    useScreen.setState({ recording: await api.screenRecordStart(window) });
  } catch (e) {
    useEditor.getState().toast({ kind: "error", text: errorText(e) });
  }
}

/** `keep` adds what was recorded to Media; otherwise it is deleted. */
export async function stopRecording(keep: boolean) {
  if (!useScreen.getState().recording || useScreen.getState().stopping) return;
  useScreen.setState({ stopping: true });
  const { toast, setSnap } = useEditor.getState();
  try {
    const snap = await api.screenRecordStop(keep, currentEpoch());
    if (snap) {
      setSnap(snap);
      const added = snap.project.assets[snap.project.assets.length - 1];
      toast({ kind: "success", text: `${added?.name ?? "The recording"} is in Media`, action: undoAction(snap) });
    } else if (!keep) toast({ kind: "info", text: "Recording deleted" });
    useScreen.setState({ recording: null });
  } catch (e) {
    toast({ kind: "error", text: errorText(e) });
    // A recording the project refused stays, to be added again or deleted.
    useScreen.setState({ recording: await api.screenRecording().catch(() => null) });
  } finally {
    useScreen.setState({ stopping: false });
  }
}

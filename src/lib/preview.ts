import { create } from "zustand";
import { getCurrentWindow } from "@tauri-apps/api/window";

const MUTED_KEY = "nuzky.previewMuted";

/** How this computer plays the preview; none of it is part of the project or the export. */
export const usePreview = create(() => ({
  muted: localStorage.getItem(MUTED_KEY) === "1",
  full: false,
  /** The share of the frame's width, from the left, shown before the clips' adjustments; `null` when not comparing. */
  compare: null as number | null,
}));

export function toggleCompare() {
  usePreview.setState({ compare: usePreview.getState().compare === null ? 0.5 : null });
}

export function toggleMuted() {
  const muted = !usePreview.getState().muted;
  localStorage.setItem(MUTED_KEY, muted ? "1" : "0");
  usePreview.setState({ muted });
}

/** The UI flows replace its full screen calls, so a run never takes over the screen. */
export const appWindow = getCurrentWindow();
/** A window already in full screen stays in it when the preview leaves. */
let windowWasFull = false;
// One window change at a time: a quick second press must know what the first one found.
let windowChange = Promise.resolve();

export function toggleFullPreview() {
  const full = !usePreview.getState().full;
  usePreview.setState({ full });
  const change = async () => {
    if (full) windowWasFull = await appWindow.isFullscreen();
    if (!windowWasFull) await appWindow.setFullscreen(full);
  };
  windowChange = windowChange.then(change, change);
  return windowChange;
}

import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import { runAction } from "./actions";
import { useCover } from "./cover";
import { FROM_PREVIEW_WINDOW, type ActionId } from "./keys";
import { useReels } from "./reels";
import { closeSource } from "./source";
import { useEditor } from "./store";

/** Whether the preview shows in a window of its own as well, to move to another screen. */
export const usePreviewWindow = create(() => ({ open: false }));

export function togglePreviewWindow() {
  return usePreviewWindow.getState().open ? api.closePreviewWindow() : api.openPreviewWindow();
}

/** A key pressed in the preview window. It shows the timeline, so a media item shown in the editor's preview goes. */
function runFromWindow(action: string) {
  const s = useEditor.getState();
  if (!FROM_PREVIEW_WINDOW.has(action as ActionId) || s.view !== "editor" || !s.snap || s.snap.recovery || useCover.getState().open || useReels.getState().open) return;
  if (action !== "mute") closeSource();
  runAction(action as ActionId);
}

export function listenPreviewWindow() {
  const offs = [listen<boolean>("preview-window", (e) => usePreviewWindow.setState({ open: e.payload })), listen<string>("preview-window-action", (e) => runFromWindow(e.payload))];
  return () => offs.forEach((off) => void off.then((f) => f()));
}

// Without a project the window has nothing to show.
useEditor.subscribe((s, prev) => {
  if (s.view !== prev.view && s.view !== "editor" && usePreviewWindow.getState().open) void api.closePreviewWindow();
});

import { exportCover, useCover } from "./cover";
import type { ActionId } from "./keys";
import { toggleFullPreview, toggleMuted } from "./preview";
import { togglePreviewWindow } from "./previewWindow";
import { seekSource, sourceDuration, useSource } from "./source";
import { copySelection, cutSelection, deleteSelection, deleteSide, duplicateSelection, edgeBeside, freezeAtPlayhead, openExport, pasteAttributes, pasteClips, projectDuration, splitAtPlayhead, useEditor } from "./store";
import { US, lastFrame, nearestFrame, stepFrame } from "./time";
import { togglePanel } from "./dock";
import { pickAndImport } from "../components/panel/assets";

/** What the cover editor holds back: nothing plays there, and the clips are not shown. */
export const RESTS_IN_COVER = new Set<ActionId>(["playPause", "split", "deleteLeft", "deleteRight", "freeze", "duplicate", "copy", "cut", "paste", "pasteAttributes", "delete"]);
/** What choosing reels lets through to the editor. */
export const WORKS_IN_REELS = new Set<ActionId>(["undo", "redo", "framePrev", "frameNext", "secondBack", "secondForward", "previewWindow"]);
/** What would change the hidden timeline unseen in the full screen preview. */
export const RESTS_IN_FULL = new Set<ActionId>(["delete", "split", "deleteLeft", "deleteRight", "freeze", "duplicate", "import", "undo", "redo", "cut", "paste", "pasteAttributes"]);
/** What a media item shown in the preview takes from the timeline. */
export const MOVES_SOURCE = new Set<ActionId>(["framePrev", "frameNext", "secondBack", "secondForward", "goStart", "goEnd"]);

const CLIPBOARD = { copy: copySelection, cut: cutSelection, paste: pasteClips, pasteAttributes } as const;
export const isClipboard = (action: ActionId): action is keyof typeof CLIPBOARD => action in CLIPBOARD;

/** Runs an editor action, from its key or the launcher. `repeat` is a held key, which plays, freezes or toggles once. */
export function runAction(action: ActionId, repeat = false) {
  const s = useEditor.getState();
  const source = useSource.getState();
  if (source.asset && MOVES_SOURCE.has(action)) {
    const step = action === "framePrev" || action === "secondBack" ? -1 : action === "frameNext" || action === "secondForward" ? 1 : 0;
    const second = action === "secondBack" || action === "secondForward";
    if (step) seekSource(second ? source.timeUs + step * US : stepFrame(source.timeUs, step, source.preview?.fps || 30));
    else seekSource(action === "goStart" ? 0 : sourceDuration(source));
    return;
  }
  const fps = s.snap?.project.canvas.fps ?? 30;
  switch (action) {
    case "playPause":
      return repeat ? undefined : s.togglePlay();
    case "undo":
      return s.undo();
    case "redo":
      return s.redo();
    case "import":
      return pickAndImport();
    case "export":
      return useCover.getState().open ? void exportCover() : openExport();
    case "copy":
    case "cut":
    case "paste":
    case "pasteAttributes":
      return void CLIPBOARD[action]();
    case "duplicate":
      return duplicateSelection();
    case "split":
      return splitAtPlayhead();
    case "freeze":
      return repeat ? undefined : void freezeAtPlayhead();
    case "deleteLeft":
    case "deleteRight":
      return deleteSide(action === "deleteLeft" ? "left" : "right");
    case "delete":
      return deleteSelection();
    case "framePrev":
    case "frameNext":
    case "secondBack":
    case "secondForward": {
      const dir = action === "framePrev" || action === "secondBack" ? -1 : 1;
      const second = action === "secondBack" || action === "secondForward";
      const end = s.snap ? projectDuration(s.snap.project) : 0;
      const to = Math.min(second ? nearestFrame(s.timeUs + dir * US, fps) : stepFrame(s.timeUs, dir, fps), lastFrame(end, fps));
      if (dir > 0 ? to > s.timeUs : to < s.timeUs) s.seek(to);
      return;
    }
    case "cutPrev":
    case "cutNext": {
      const edge = s.snap ? edgeBeside(s.snap.project, s.timeUs, action === "cutPrev" ? -1 : 1) : null;
      if (edge !== null) s.seek(edge);
      return;
    }
    case "goStart":
      return s.seek(0);
    case "goEnd":
      return s.snap ? s.seek(projectDuration(s.snap.project)) : undefined;
    case "mute":
      return repeat ? undefined : toggleMuted();
    case "fullScreen":
      return repeat ? undefined : void toggleFullPreview();
    case "previewWindow":
      return repeat ? undefined : void togglePreviewWindow();
    case "zoomIn":
      return s.setZoom(s.zoom * 1.3);
    case "zoomOut":
      return s.setZoom(s.zoom / 1.3);
    case "aiPanel":
      return togglePanel();
  }
}

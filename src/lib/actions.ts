import { exportCover, useCover } from "./cover";
import type { ActionId } from "./keys";
import { SPEEDS, rememberSpeed, toggleFullPreview, toggleMuted, usePreview } from "./preview";
import { api } from "./api";
import { togglePreviewWindow } from "./previewWindow";
import { seekSource, sourceDuration, toggleSourcePlay, useSource } from "./source";
import { copySelection, cutSelection, deleteSelection, deleteSide, duplicateSelection, edgeBeside, endPreview, freezeAtPlayhead, openExport, pasteAttributes, pasteClips, projectDuration, splitAtPlayhead, useEditor } from "./store";
import { US, lastFrame, nearestFrame, stepFrame } from "./time";
import { togglePanel } from "./dock";
import { pickAndImport } from "../components/panel/assets";
import { useVoiceover } from "./voiceover";

/** What the cover editor holds back: nothing plays there, and the clips are not shown. */
export const RESTS_IN_COVER = new Set<ActionId>(["playPause", "playBackward", "stop", "playForward", "split", "deleteLeft", "deleteRight", "freeze", "duplicate", "copy", "cut", "paste", "pasteAttributes", "delete"]);
/** What choosing reels lets through to the editor. */
export const WORKS_IN_REELS = new Set<ActionId>(["undo", "redo", "framePrev", "frameNext", "secondBack", "secondForward", "previewWindow"]);
/** What would change the hidden timeline unseen in the full screen preview. */
export const RESTS_IN_FULL = new Set<ActionId>(["delete", "split", "deleteLeft", "deleteRight", "freeze", "duplicate", "import", "undo", "redo", "cut", "paste", "pasteAttributes"]);
/** What a media item shown in the preview takes from the timeline. */
export const MOVES_SOURCE = new Set<ActionId>(["framePrev", "frameNext", "secondBack", "secondForward", "goStart", "goEnd"]);

/** A voiceover is recorded against the picture at 1x, so the speed waits until it ends. */
export const recordingVoiceover = (phase = useVoiceover.getState().phase) => phase === "countdown" || phase === "recording";

/** The next of SPEEDS above `rate`, or the fastest. */
const faster = (rate: number) => SPEEDS.find((s) => s > rate + 1e-6) ?? SPEEDS[SPEEDS.length - 1];

/**
 * J and L play backwards and forwards, each press while already going that way one speed faster; K stops. Forwards
 * starts at the speed Play uses and becomes it; backwards starts at 1x. A media item in the preview only plays and stops.
 */
export function shuttle(direction: -1 | 0 | 1) {
  const source = useSource.getState();
  if (source.asset) {
    if (direction > 0 ? !source.playing : source.playing) toggleSourcePlay();
    return;
  }
  const s = useEditor.getState();
  if (direction === 0) return void (s.playing && api.pause());
  if (recordingVoiceover() || !s.snap || projectDuration(s.snap.project) === 0) return;
  const { rate, speed } = usePreview.getState();
  const along = s.playing && Math.sign(rate) === direction;
  const next = along ? faster(Math.abs(rate)) : direction > 0 ? speed : 1;
  if (direction > 0) rememberSpeed(next);
  endPreview(true);
  void api.playAt(direction * next);
}

/** Chooses the speed Play plays at; playing, the preview goes on at it, the same way round. */
export function chooseSpeed(speed: number) {
  if (recordingVoiceover()) return;
  rememberSpeed(speed);
  const { rate } = usePreview.getState();
  if (useEditor.getState().playing && Math.abs(rate) !== speed) void api.playAt(rate < 0 ? -speed : speed);
}

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
    case "playBackward":
    case "stop":
    case "playForward":
      return repeat ? undefined : shuttle(action === "playBackward" ? -1 : action === "stop" ? 0 : 1);
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

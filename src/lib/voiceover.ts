import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api, errorText } from "./api";
import { usePreview } from "./preview";
import { aiLocked, currentEpoch, projectDuration, undoAction, useEditor } from "./store";
import { US, formatTime } from "./time";
import { closeSource } from "./source";
import type { Asset, MicLevel, Project } from "./types";

/**
 * Record voiceover: `ready` listens to the microphone for the meter, `countdown` counts 3, 2, 1, and `recording`
 * writes the take while the timeline plays from where it started, without its sound.
 */
export type VoiceoverPhase = "closed" | "opening" | "ready" | "countdown" | "recording" | "stopping";

export const useVoiceover = create<{
  phase: VoiceoverPhase;
  /** The microphone listened to. */
  microphone: string | null;
  /** Why the microphone did not open, with what to do. */
  error: string | null;
  /** 3, 2 or 1 during the countdown. */
  count: number;
  /** Where on the timeline the take starts. */
  startUs: number;
  /** A take the project refused, as while the AI edits, waiting for Add or Discard. */
  unsaved: boolean;
  /** macOS asks the user whether Nuzky may use the microphone. */
  asking: boolean;
}>(() => ({ phase: "closed", microphone: null, error: null, count: 3, startUs: 0, unsaved: false, asking: false }));

/** The meter's reading, 20 times a second: a store of its own, so only the meter draws again. */
export const useMicLevel = create<{ level: MicLevel | null }>(() => ({ level: null }));

const COUNT_FROM = 3;
let timer = 0;
let listening = false;
/** The session the take was started in, so a take never lands in another project opened meanwhile. */
let takeEpoch: string | undefined;

function listenOnce() {
  if (listening) return;
  listening = true;
  void listen<MicLevel>("voiceover-level", (e) => {
    if (useVoiceover.getState().phase !== "closed") useMicLevel.setState({ level: e.payload });
  });
  void listen("voiceover-asking", () => {
    if (useVoiceover.getState().phase === "opening") useVoiceover.setState({ asking: true });
  });
  void listen("voiceover-microphone-lost", () => {
    const { phase } = useVoiceover.getState();
    if (phase === "recording") void stopVoiceover(true, "The microphone was disconnected, so the voiceover stopped.");
    else if (phase !== "closed") {
      cancelCountdown();
      useVoiceover.setState({ phase: "ready", error: "The microphone was disconnected. Connect it again, then reopen the voiceover." });
    }
  });
}

/** Opening and closing the microphone, one after the other, so a quick close never overtakes an open. */
let microphoneCalls: Promise<unknown> = Promise.resolve();
const inTurn = <T>(call: () => Promise<T>) => {
  const next = microphoneCalls.then(call, call);
  microphoneCalls = next.catch(() => {});
  return next;
};

/** A take Nuzky recorded, which is a voice, never a song to cut to. */
export const isRecordedVoiceover = (asset: Asset | undefined) => !!asset && /[\\/]recordings[\\/]voiceover-[^\\/]+\.wav$/.test(asset.path);

/** Shows the voiceover in the library's place and opens the microphone for its meter. */
export async function openVoiceover() {
  if (useVoiceover.getState().phase !== "closed") return;
  listenOnce();
  // The countdown shows over the timeline's picture, which an item shown from the library would hide.
  closeSource();
  useMicLevel.setState({ level: null });
  useVoiceover.setState({ phase: "opening", error: null, asking: false });
  const still = () => useVoiceover.getState().phase === "opening";
  try {
    const microphone = await inTurn(api.voiceoverOpen);
    const { unsaved } = await api.voiceoverState();
    if (still()) useVoiceover.setState({ phase: "ready", microphone, unsaved, asking: false });
  } catch (e) {
    const { unsaved } = await api.voiceoverState().catch(() => ({ unsaved: false }));
    if (still()) useVoiceover.setState({ phase: "ready", microphone: null, unsaved, asking: false, error: errorText(e) });
  }
}

/** Closes the microphone and gives the library back; a take still recording goes on the timeline first. */
export async function closeVoiceover() {
  const { phase } = useVoiceover.getState();
  if (phase === "closed") return;
  cancelCountdown();
  if (phase === "recording") await stopVoiceover(true);
  if (useVoiceover.getState().unsaved)
    useEditor.getState().toast({ kind: "warning", text: "Your last voiceover is not on the timeline yet. Open the voiceover to add it or discard it." });
  useVoiceover.setState({ phase: "closed", microphone: null, error: null });
  useMicLevel.setState({ level: null });
  await inTurn(api.voiceoverClose).catch(() => {});
}

/** Closes and opens the microphone again, as after the user fixed what kept it from opening. */
export async function reopenVoiceover() {
  await closeVoiceover();
  await openVoiceover();
}

/**
 * Why the take cannot start at the playhead: at the end of a video with picture, as sound after the picture is not
 * exported. A timeline of sound alone grows with the take.
 */
export function videoEndReason(project: Project | undefined, timeUs: number) {
  return project && atPictureEnd(project, timeUs) ? "The video ends here. Move the playhead to where the voiceover starts." : null;
}

/** In the last frame of the picture; a timeline of sound alone has no picture to end. */
function atPictureEnd(project: Project, timeUs: number) {
  const picture = project.tracks.filter((t) => t.kind !== "audio").reduce((m, t) => t.clips.reduce((n, c) => Math.max(n, c.startUs + c.durationUs), m), 0);
  return picture > 0 && timeUs >= picture - US / project.canvas.fps;
}

/** Counts 3, 2, 1, then records from the playhead. */
export function startCountdown() {
  const s = useVoiceover.getState();
  const editor = useEditor.getState();
  if (s.phase !== "ready" || !s.microphone || s.unsaved || aiLocked() || videoEndReason(editor.snap?.project, editor.timeUs)) return;
  if (editor.playing) void api.pause();
  useVoiceover.setState({ phase: "countdown", count: COUNT_FROM, startUs: editor.timeUs });
  // Routine news would cover the script while the creator reads it.
  useEditor.setState((st) => ({ toasts: st.toasts.filter((t) => t.kind === "error" || t.kind === "warning" || t.sticky) }));
  const tick = (n: number) => {
    timer = window.setTimeout(() => {
      if (useVoiceover.getState().phase !== "countdown") return;
      if (n > 1) {
        useVoiceover.setState({ count: n - 1 });
        tick(n - 1);
      } else void begin();
    }, 1000);
  };
  tick(COUNT_FROM);
}

export function cancelCountdown() {
  window.clearTimeout(timer);
  if (useVoiceover.getState().phase === "countdown") useVoiceover.setState({ phase: "ready" });
}

/** Resolves whether `ok` came true within `ms`. */
function until(ok: () => boolean, ms: number) {
  return new Promise<boolean>((resolve) => {
    if (ok()) return resolve(true);
    const done = (result: boolean) => {
      stop();
      window.clearTimeout(timeout);
      resolve(result);
    };
    const stop = useEditor.subscribe(() => ok() && done(true));
    const timeout = window.setTimeout(() => done(ok()), ms);
  });
}

async function begin() {
  const { startUs } = useVoiceover.getState();
  // The microphone would hear the video from the speakers.
  void api.setPreviewMuted(true);
  // Play in the last frame would start the video again from its beginning.
  const project = useEditor.getState().snap?.project;
  if (project && startUs < projectDuration(project) - US / project.canvas.fps) void api.play();
  // The picture waits for the sound card before it moves, so the take starts with the first moving frame and goes
  // where that frame is; on an empty timeline or at its end, nothing moves and it starts where the playhead stands.
  const moving = await until(() => useEditor.getState().playing && useEditor.getState().timeUs > startUs, 600);
  const unmute = () => void api.setPreviewMuted(usePreview.getState().muted);
  if (useVoiceover.getState().phase !== "countdown") {
    void api.pause();
    unmute();
    return;
  }
  const from = moving ? useEditor.getState().timeUs : startUs;
  try {
    takeEpoch = currentEpoch();
    await api.voiceoverRecordStart(from);
  } catch (e) {
    void api.pause();
    unmute();
    if (useVoiceover.getState().phase !== "countdown") return;
    useVoiceover.setState({ phase: "ready" });
    useEditor.getState().toast({ kind: "error", text: errorText(e) });
    return;
  }
  // Closed meanwhile, as when another project opened: the take never began for the user.
  if (useVoiceover.getState().phase !== "countdown") {
    void api.pause();
    unmute();
    void api.voiceoverRecordStop(false, takeEpoch).catch(() => {});
    return;
  }
  useVoiceover.setState({ phase: "recording", startUs: from });
  // The take stays in time with the picture only while the picture plays on: when it stops, at the video's end or by
  // Pause, or the playhead is moved, the take ends there and goes on the timeline.
  const watch = useEditor.subscribe((st, prev) => {
    if (useVoiceover.getState().phase !== "recording") return watch();
    const stopped = prev.playing && !st.playing;
    const moved = st.seeks !== prev.seeks;
    if (!stopped && !moved) return;
    const ended = !!st.snap && atPictureEnd(st.snap.project, st.timeUs);
    // Sound alone ends where its last clip does; the take goes on past it.
    if (!moved && !ended && st.snap && st.timeUs >= projectDuration(st.snap.project) - US / st.snap.project.canvas.fps) return;
    watch();
    const why = moved ? "The playhead moved, so the voiceover stopped." : ended ? "The video ended, so the voiceover stopped there." : "Playback stopped, so the voiceover stopped there.";
    void stopVoiceover(true, why);
  });
}

/** `keep` puts the take on the timeline where it started, as one undo step; otherwise it is deleted. */
export async function stopVoiceover(keep: boolean, why?: string) {
  const { phase, unsaved } = useVoiceover.getState();
  if (phase !== "recording" && !(unsaved && phase === "ready")) return;
  useVoiceover.setState({ phase: "stopping" });
  const recorded = phase === "recording";
  if (recorded) {
    void api.pause();
    void api.setPreviewMuted(usePreview.getState().muted);
  }
  const { toast, setSnap } = useEditor.getState();
  try {
    const snap = await api.voiceoverRecordStop(keep, recorded ? takeEpoch : currentEpoch());
    if (snap) {
      setSnap(snap);
      const at = formatTime(useVoiceover.getState().startUs);
      toast({ kind: why ? "warning" : "success", text: why ? `${why} It is on the timeline at ${at}.` : `Voiceover added at ${at}`, action: undoAction(snap) });
    } else if (!keep) {
      toast({ kind: "info", text: "Voiceover deleted" });
      // The next take goes where this one started.
      if (recorded) useEditor.getState().seek(useVoiceover.getState().startUs);
    }
    useVoiceover.setState({ unsaved: false });
  } catch (e) {
    const state = await api.voiceoverState().catch(() => null);
    // A new take the project refused waits in the panel with Add, which says why it cannot be added yet; a refused Add
    // says why, such as the take belonging to another project.
    if (!state?.unsaved || !recorded) toast({ kind: "error", text: errorText(e) });
    useVoiceover.setState({ unsaved: state?.unsaved ?? false });
  } finally {
    if (useVoiceover.getState().phase === "stopping") useVoiceover.setState({ phase: "ready" });
  }
}

const SCROLL: Record<string, number> = { ArrowDown: 0.15, ArrowUp: -0.15, PageDown: 0.8, PageUp: -0.8 };

/**
 * While counting down or recording, Space stops the take (or the countdown, as does Esc) wherever the keyboard is,
 * the arrows and Page keys scroll the script, and the editor's other keys wait, with Command or Ctrl too, so nothing
 * on the timeline changes under the take. Tab and Enter on a button still work; the system's menus keep their keys.
 */
export function voiceoverKeys(e: KeyboardEvent) {
  const { phase } = useVoiceover.getState();
  if (phase !== "recording" && phase !== "countdown") return;
  const onButton = e.target instanceof HTMLElement && e.target.tagName === "BUTTON";
  if (e.key === "Tab" || (e.key === "Enter" && onButton)) return;
  e.preventDefault();
  e.stopPropagation();
  if (e.repeat) return;
  if (phase === "countdown" && (e.key === "Escape" || e.key === " ")) cancelCountdown();
  else if (phase === "recording" && e.key === " ") void stopVoiceover(true);
  else if (phase === "recording" && e.key in SCROLL) {
    const view = document.querySelector<HTMLElement>("[data-voiceover-reading]");
    const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
    view?.scrollBy({ top: SCROLL[e.key] * view.clientHeight, behavior: still ? "auto" : "smooth" });
  }
}

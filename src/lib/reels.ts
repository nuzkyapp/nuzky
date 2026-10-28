import { create } from "zustand";
import { api, errorText, plainError } from "./api";
import { anyAgent, send, sendBlocked, useAgent } from "./agent";
import { useDock } from "./dock";
import { openProject, refreshLibrary } from "./library";
import { MAIN_TRACK, aiLocked, currentEpoch, projectDuration, useEditor, whenIdle } from "./store";
import { US } from "./time";
import type { JobEvent, MadeReel, Project, ReelCandidate, ReelFinish, ReelFraming, TranscriptWord } from "./types";

/** From this length on the Transcript tab offers Reels from a long video. */
export const LONG_VIDEO_US = 180 * US;
/** The longest reel propose_reels suggests; a longer one gets a warning but can still be made. */
export const REEL_LIMIT_US = 60 * US;

/** What Find reels asks the AI panel's agent; the guide it reads tells it how. */
export const FIND_REELS =
  "Find the moments in this video that work as reels on their own and propose them for me to pick. Transcribe it first if it has no transcript. Don't change the video itself.";

const FRAMING_KEY = "nuzky.reelFraming";
const FINISH_KEY = "nuzky.reelFinish";

/** What each reel gets once made, every step unless the user turned it off. */
function storedFinish(): ReelFinish {
  const all = { clean: true, captions: true, zooms: true };
  try {
    return { ...all, ...JSON.parse(localStorage.getItem(FINISH_KEY) ?? "{}") };
  } catch {
    return all;
  }
}

interface ReelsState {
  open: boolean;
  /** The candidate in the preview and the trim pane. */
  current: string | null;
  framing: ReelFraming;
  finish: ReelFinish;
  /** The playhead and clip selection from before, given back on close. */
  before: { timeUs: number; selection: string[] } | null;
  /** Bounds asked for and not confirmed yet, so the trim pane shows them in the same frame. */
  trim: { id: string; from: number; to: number } | null;
  /** Find reels sent its message and the agent is answering it. */
  finding: boolean;
  /** Find reels has finished at least once in this session, so an empty list means nothing was found. */
  searched: boolean;
  /** The make job this mode started. */
  job: string | null;
  /** Why the last make failed, until another starts. */
  failed: string | null;
  /** The video's words as they play now, read once for the list and the trim pane. */
  transcript: { words: TranscriptWord[] | null; error: string | null };
}

export const useReels = create<ReelsState>(() => ({
  open: false,
  current: null,
  framing: localStorage.getItem(FRAMING_KEY) === "blur" ? "blur" : "crop",
  finish: storedFinish(),
  before: null,
  trim: null,
  finding: false,
  searched: false,
  job: null,
  failed: null,
  transcript: { words: null, error: null },
}));

/** Best first, the rest in the order they are said. */
export function candidates(project: Project | undefined): ReelCandidate[] {
  return [...(project?.reelCandidates ?? [])].sort((a, b) => b.score - a.score || a.startUs - b.startUs);
}

export const candidateOf = (project: Project | undefined, id: string | null) => project?.reelCandidates?.find((c) => c.id === id);

/** Width over height of the picture the reel starts on, which Center crop cuts to 9:16; 0 when there is none. */
export function pictureAspect(project: Project | undefined, id: string | null): number {
  const c = candidateOf(project, id);
  const clip = c && project?.tracks.find((t) => t.id === MAIN_TRACK)?.clips.find((k) => k.startUs <= c.startUs && c.startUs < k.startUs + k.durationUs);
  const asset = clip?.content.type === "media" ? project?.assets.find((a) => clip.content.type === "media" && a.id === clip.content.assetId) : undefined;
  return asset && asset.width > 0 && asset.height > 0 ? asset.width / asset.height : 0;
}

/** A cut made since the reel was found moved its words, so making it would cut others. */
export const moved = (c: ReelCandidate, words: TranscriptWord[] | null) =>
  !!words && c.status !== "made" && !(words[c.from]?.startUs === c.startUs && words[c.to]?.endUs === c.endUs);

/** Opens the mode on the best candidate; the playhead and selection come back on close. */
export function openReels() {
  const { snap, playing, timeUs, selection } = useEditor.getState();
  if (!snap) return;
  if (playing) void api.pause();
  const before = useReels.getState().before ?? { timeUs, selection };
  useReels.setState({ open: true, before, failed: null });
  useEditor.getState().select([]);
  showBest(snap.project);
  // Says whether media files of the video are missing.
  void refreshLibrary();
}

/** Shows the best reel not rejected, when the one shown is gone or there was none. */
function showBest(project: Project) {
  const list = candidates(project);
  const best = list.find((c) => c.status !== "rejected") ?? list[0];
  if (best) choose(best.id);
}

export function closeReels() {
  const before = useReels.getState().before;
  if (useEditor.getState().playing) void api.pause();
  useReels.setState({ open: false, before: null, trim: null });
  if (before) {
    useEditor.getState().seek(before.timeUs);
    useEditor.getState().select(before.selection);
  }
}

/** Shows a candidate in the preview, from its start. */
export function choose(id: string) {
  const c = candidateOf(useEditor.getState().snap?.project, id);
  useReels.setState({ current: id, trim: null });
  if (c) useEditor.getState().seek(c.startUs);
}

/** Plays the current reel to its end, from its start when it stopped there. */
export function toggleReelPlay() {
  const { playing, snap, timeUs, seek, togglePlay } = useEditor.getState();
  const c = candidateOf(snap?.project, useReels.getState().current);
  if (!playing && c && timeUs === c.endUs) seek(c.startUs);
  togglePlay();
}

/** The last seconds of the current reel, to hear how it ends. */
export function playEnding() {
  const { playing, snap, seek } = useEditor.getState();
  const c = candidateOf(snap?.project, useReels.getState().current);
  if (!c) return;
  seek(Math.max(c.startUs, c.endUs - 3 * US));
  if (!playing) void api.play();
}

// Playback stops where the reel ends, once it gets there from inside it.
useEditor.subscribe((s, prev) => {
  if (!useReels.getState().open || !s.playing) return;
  const c = candidateOf(s.snap?.project, useReels.getState().current);
  if (c && prev.timeUs < c.endUs && s.timeUs >= c.endUs) {
    void api.pause();
    useEditor.getState().seek(c.endUs);
  }
});

// The agent's first reels show at once, and so does another one when the reel shown goes.
useEditor.subscribe((s) => {
  const { open, current } = useReels.getState();
  if (open && s.snap && !candidateOf(s.snap.project, current)) showBest(s.snap.project);
});

// Another project opened: its editor starts on the video, and reels still being made could not be marked made.
useEditor.subscribe((s, prev) => {
  if (!prev.snap || !s.snap || s.snap.sessionEpoch === prev.snap.sessionEpoch) return;
  const { job } = useReels.getState();
  if (job) void api.cancelJob(job);
  useReels.setState({ open: false, before: null, trim: null, current: null, searched: false, failed: null, job: null });
});

/** Changes one candidate as one undo step, built on the latest project; a made one stays as it is. */
function update(id: string, change: (c: ReelCandidate) => ReelCandidate, coalesce?: string) {
  return useEditor.getState().edit((project) => {
    const c = candidateOf(project, id);
    return c && c.status !== "made" ? { type: "updateReelCandidate", candidate: change(c) } : null;
  }, coalesce);
}

export const setPicked = (id: string, picked: boolean) => update(id, (c) => ({ ...c, status: picked ? "picked" : "proposed" }));
export const setRejected = (id: string, rejected: boolean) => update(id, (c) => ({ ...c, status: rejected ? "rejected" : "proposed" }));

/** Checks every proposed candidate as one undo step. */
export function pickAll() {
  return useEditor.getState().edit((project) => {
    const open = (project.reelCandidates ?? []).filter((c) => c.status === "proposed");
    return open.map((c) => ({ type: "updateReelCandidate", candidate: { ...c, status: "picked" } }));
  });
}

let boundsAsked = 0;
/** The trim being measured, which Make waits for. */
let measuring: Promise<void> = Promise.resolve();

/**
 * Moves the reel to start at word `from` and end at word `to`. The engine measures how long it is once cut; a burst
 * of keys is one undo step.
 */
export function setBounds(id: string, from: number, to: number) {
  if (aiLocked()) return;
  measuring = measure(id, from, to, currentEpoch());
}

async function measure(id: string, from: number, to: number, epoch: string | undefined) {
  const asked = ++boundsAsked;
  useReels.setState({ trim: { id, from, to } });
  let bounds: ReelCandidate;
  try {
    bounds = await api.reelBounds(id, from, to);
  } catch (e) {
    if (asked === boundsAsked) useReels.setState({ trim: null });
    useEditor.getState().toast({ kind: "error", text: plainError(errorText(e)) });
    return;
  }
  // A newer change is on its way and measures again; another project has other words.
  if (asked !== boundsAsked || currentEpoch() !== epoch) return;
  const { from: f, to: t, startUs, endUs, hook, durationUs } = bounds;
  await update(id, (c) => ({ ...c, from: f, to: t, startUs, endUs, hook, durationUs }), `reel-bounds:${id}`);
  if (asked === boundsAsked) useReels.setState({ trim: null });
}

/** Why Find reels cannot ask the AI panel's agent now, or null. */
export function findBlocked(): string | null {
  if (useAgent.getState().agents && !anyAgent()) return "Sign in with ChatGPT first, or install Claude Code or Codex";
  return sendBlocked(false);
}

/** Asks the AI panel's agent for reels; they show up in the list as it proposes them. */
export function findReels() {
  if (aiLocked()) return;
  // Make a thumbnail waiting in the field would turn the message into a thumbnail request.
  useAgent.setState({ thumbnail: false });
  if (findBlocked()) return;
  useDock.setState({ open: true });
  useReels.setState({ finding: true });
  void send(FIND_REELS, "project");
}

// Finding ends with the agent's answer; an empty list from then on means nothing was found.
useAgent.subscribe((s, prev) => {
  if (prev.status !== "idle" && s.status === "idle" && useReels.getState().finding) useReels.setState({ finding: false, searched: true });
});

export function setFraming(framing: ReelFraming) {
  localStorage.setItem(FRAMING_KEY, framing);
  useReels.setState({ framing });
}

export function setFinish(finish: ReelFinish) {
  localStorage.setItem(FINISH_KEY, JSON.stringify(finish));
  useReels.setState({ finish });
}

/** Makes a project of each checked candidate as a job with progress and Stop. */
export async function makeReels() {
  const { snap } = useEditor.getState();
  const { words } = useReels.getState().transcript;
  const ids = (snap?.project.reelCandidates ?? []).filter((c) => c.status === "picked" && !moved(c, words)).map((c) => c.id);
  if (ids.length === 0 || aiLocked() || useReels.getState().job) return;
  const epoch = currentEpoch();
  // Picks and trims made just before are part of what is made, also one made while waiting.
  let waited: Promise<void>;
  do {
    waited = measuring;
    await waited;
  } while (waited !== measuring);
  await whenIdle();
  // Another project opened meanwhile; its reels were not asked for.
  if (currentEpoch() !== epoch) return;
  useReels.setState({ failed: null });
  try {
    const { framing, finish } = useReels.getState();
    const id = await api.startMakeReels(ids, framing, finish, epoch);
    useReels.setState({ job: id });
    // A job can end before its id comes back; its last event is already in.
    const ended = useEditor.getState().jobs[id];
    if (ended) onReelsJob(ended);
  } catch (e) {
    useReels.setState({ failed: makeError(errorText(e)) });
  }
}

/** What went wrong in words that say what to do next. */
function makeError(text: string): string {
  if (text.includes("SPEECH_CHANGED")) return "The video changed since these reels were found. Use Find again.";
  if (text.includes("RUN_ACTIVE")) return "The AI started editing, so no reels were made. Try again when it's done.";
  if (text.includes("os error")) return "Couldn't save the reels beside the long video. Free some space, or check that its folder can be written to.";
  return `Couldn't make the reels: ${plainError(text)}`;
}

export function onReelsJob(job: JobEvent) {
  if (job.id !== useReels.getState().job || job.status === "running") return;
  useReels.setState({ job: null, failed: job.status === "failed" ? makeError(job.message ?? "") : null });
  if (job.status !== "done") return;
  const { reels, unlisted } = JSON.parse(job.output ?? "{}") as { reels: MadeReel[]; unlisted: string | null };
  const made = reels.length === 1 ? "Made 1 reel" : `Made ${reels.length} reels`;
  const skipped = [...new Set(reels.flatMap((reel) => reel.skipped.map((step) => step.replace(/\.$/, ""))))];
  const left = skipped.length ? ` Left out: ${skipped.join("; ")}.` : "";
  if (unlisted)
    useEditor.getState().toast({ kind: "warning", text: `${made} beside the long video, but your projects list could not be updated (${unlisted}). Open them with Open file… on the home screen.${left}` });
  else useEditor.getState().toast(left ? { kind: "warning", text: `${made}.${left}` } : { kind: "success", text: made });
}

/**
 * Opens a made reel's project, with the Export dialog on Reels & TikTok when `exporting`. The mode closes once it is
 * open; a toast leads back to the long video's reels.
 */
export function openReel(c: ReelCandidate, exporting: boolean) {
  const source = useEditor.getState().snap;
  if (!c.projectPath || !source) return;
  const back = { path: source.path, name: source.project.name };
  openProject({ path: c.projectPath, name: c.title }, undefined, () => {
    if (exporting) useEditor.setState({ exportOpen: true, exportPreset: "reels" });
    useEditor.getState().toast({ kind: "info", text: `Opened the reel “${c.title}”`, action: { label: "Back to reels", run: () => openProject(back, undefined, openReels) } });
  });
}

/** Opens the mode on a project just made of a long video, with the keyboard on Find reels. */
export function openReelsToFind() {
  openReels();
  window.setTimeout(() => document.querySelector<HTMLElement>("[data-find-reels]")?.focus(), 0);
}

/** Long enough to look for reels in, or already holding some. */
export const offersReels = (project: Project) => projectDuration(project) >= LONG_VIDEO_US || (project.reelCandidates?.length ?? 0) > 0;

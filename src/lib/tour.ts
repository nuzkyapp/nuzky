import { create } from "zustand";
import { useAgent, type ChatItem } from "./agent";
import { api } from "./api";
import { SAMPLE_REQUEST, lastDemoOutcome } from "./demo";
import { dockMax, useDock } from "./dock";
import { usedBefore } from "./launch";
import { newSampleProject, onlyPristine, pickVideosForNewProject, useLibrary } from "./library";
import { useSetup } from "./setup";
import { useSounds } from "./sounds";
import { useSpeech, useWordPick } from "./speech";
import { allClips, isCaptionTrack, projectDuration, useEditor } from "./store";
import type { Project, Snapshot } from "./types";

/**
 * The first-run tour on the sample video: eight steps, each pointing at one part of the editor while the rest
 * waits under a dim (src/components/Tour.tsx). It starts with a scripted AI edit (src/lib/demo.ts).
 */
export const TOUR_STEPS = ["ask", "watch", "undo", "play", "transcript", "song", "volume", "export"] as const;
export type TourStep = (typeof TOUR_STEPS)[number];

/** The song built into Nuzky that the tour adds. */
export const SONG = "nuzky:dancing";

/**
 * What each step says and points at. `target` is the last element that matches; `extra` is shown too; `focus` takes the
 * keyboard when the step's action is on the target, otherwise the card has it; `side` is where the card goes when it fits.
 */
export const STEP: Record<TourStep, { title: string; text: string; target: string; extra?: string; focus?: string; next: boolean; side: "left" | "right" | "above" | "below" }> = {
  ask: {
    title: "Ask the AI to edit",
    text: "Tell the AI what you want in your own words. Try this one: press Enter.",
    target: "[data-tour=ai-field]",
    focus: "[data-tour=ai-field] textarea",
    next: false,
    side: "left",
  },
  watch: {
    title: "Watch it edit",
    text: "Watch the timeline change. Stop and edit, at the top, lets you take over.",
    target: "section[aria-label=Timeline]",
    extra: "[data-tour=run-bar]",
    next: false,
    side: "above",
  },
  undo: {
    title: "One Undo takes it all back",
    text: "This card lists what the AI changed. Undo here takes back the whole edit in one step.",
    target: "[data-run-card]",
    next: true,
    side: "left",
  },
  play: { title: "Play the result", text: "Press Space to play. What you see here is exactly what you export.", target: "section[aria-label=Preview]", next: true, side: "right" },
  transcript: {
    title: "Fix anything by hand",
    text: "Every word is here. Select words and press Delete to cut them.",
    target: "[data-tour=library]",
    focus: "[data-tour=library] [role=listbox][aria-label=Transcript]",
    next: true,
    side: "right",
  },
  song: {
    title: "Add a song",
    text: "Press + to put this free song under her voice. You can also paste a YouTube link above.",
    target: `[data-sound="${SONG}"]`,
    focus: `[data-sound="${SONG}"] button[aria-label^="Add"]`,
    next: false,
    side: "right",
  },
  volume: {
    title: "Fine-tune what's selected",
    text: "The song is selected. Turn its volume down so her voice stays clear.",
    target: "[data-tour=volume]",
    focus: "[data-tour=volume] input[type=range]",
    next: true,
    side: "left",
  },
  export: { title: "Export when it's ready", text: "Export makes a video you can post anywhere. Ctrl+E opens it too.", target: "[data-tour=export]", next: false, side: "below" },
};

interface TourState {
  step: TourStep | null;
  /** The sample project the tour runs in; another project ends it. */
  epoch: string | null;
  /** The AI's turn was stopped, so Watch it edit waits for Next. */
  stopped: boolean;
  /** One Undo's edit is undone now, so the card offers Redo. */
  undone: boolean;
  /** The transcript's first word, which Fix anything by hand selects for Delete. */
  word: string | null;
  /** Whether this launch opened Get set up by itself; null until the projects are listed. */
  firstLaunch: boolean | null;
}

export const useTour = create<TourState>(() => ({ step: null, epoch: null, stopped: false, undone: false, word: null, firstLaunch: null }));

/** The tour closed the AI panel to show the inspector, and opens it again after. */
let closedPanel = false;
/** What was in the AI field before the tour wrote its request there. */
let keptDraft = "";
/** Watch it edit stays a moment after the edit ends, so its card can be read to the end. */
const WATCH_AFTER_MS = 1500;
let leaveWatch = 0;
/** The project as the AI's edit left it, while One Undo shows; Undo and Redo are told apart from other edits by it. */
let edited: string | null = null;

const runCards = (items: ChatItem[]) => items.some((i) => i.kind === "run");

/** The song's clip, once it is on the timeline. */
function songClip(project: Project) {
  const asset = project.assets.find((a) => a.credit && `${a.credit.source}:${a.credit.id}` === SONG);
  return asset ? allClips(project).find((c) => c.content.type === "media" && c.content.assetId === asset.id) : undefined;
}

/** The AI panel shows in the inspector's place: chosen so, or a column that does not fit this window. */
function panelInInspector() {
  const { open, mode } = useDock.getState();
  return open && (mode === "inspector" || ((mode === "left" || mode === "right") && dockMax(window.innerWidth) === null));
}

function show(step: TourStep) {
  window.clearTimeout(leaveWatch);
  // Only Next leaves Play the result, and the video stops there, so no later card stands over it playing.
  if (useTour.getState().step === "play" && useEditor.getState().playing) void api.pause();
  if (step !== "volume" && closedPanel) {
    closedPanel = false;
    useDock.setState({ open: true });
  }
  const snap = useEditor.getState().snap;
  edited = step === "undo" && snap ? JSON.stringify(snap.project) : null;
  // The first caption's frame, so the still preview already shows the edit.
  const caption = step === "play" && snap ? Math.min(...snap.project.tracks.filter(isCaptionTrack).flatMap((t) => t.clips.map((c) => c.startUs))) : Infinity;
  if (caption !== Infinity) useEditor.getState().seek(caption);
  useWordPick.setState({ startUs: null });
  if (step === "transcript") {
    useEditor.setState({ panelTab: "transcript" });
    pickFirstWord();
  }
  if (step === "song") {
    useEditor.setState({ panelTab: "audio" });
    // The built-in songs show while nothing is searched.
    useSounds.setState((s) => ({ tab: "music", search: { ...s.search, music: { ...s.search.music, query: "" } } }));
  }
  if (step === "volume") {
    const project = useEditor.getState().snap?.project;
    const clip = project && songClip(project);
    if (clip) useEditor.getState().select([clip.id]);
    // Below about 1330 px the panel takes the inspector's place, where the volume is.
    if (panelInInspector()) {
      closedPanel = true;
      useDock.setState({ open: false });
    }
  }
  useTour.setState({ step, stopped: false, undone: false, word: null });
}

/** Selects the transcript's first word, so Delete cuts it, and names it on the card. */
function pickFirstWord() {
  const { epoch } = useTour.getState();
  api.transcriptView(useSpeech.getState().pauseUs).then(
    ({ words: [first] }) => {
      const t = useTour.getState();
      if (!first || t.step !== "transcript" || t.epoch !== epoch) return;
      useTour.setState({ word: first.text });
      useWordPick.setState({ startUs: first.startUs });
    },
    // Without it the card asks to select words by hand.
    () => {},
  );
}

/**
 * One Undo follows the project: Undo on the run's card, Ctrl+Z or the timeline's Undo undo the edit, and Redo brings it
 * back, so the tour's card and the run's card both say which it is. Another edit leaves them as they are.
 */
function followUndo(snap: Snapshot) {
  const back = JSON.stringify(snap.project) === edited;
  if (!back && !snap.canRedo) return;
  useTour.setState({ undone: !back });
  const { items } = useAgent.getState();
  const index = items.findLastIndex((it) => it.kind === "run");
  const run = items[index];
  // With the edit back, the run's card offers Undo again, from the project as it is now.
  if (run?.kind === "run" && !!run.undone === back) useAgent.setState({ items: items.map((it, i) => (i === index ? { ...run, undone: !back, ...(back && { snap }) } : it)) });
}

/** Next on the card: the following step; One Undo is left out when the AI changed nothing. */
export function nextStep() {
  const { step } = useTour.getState();
  if (!step) return;
  const i = TOUR_STEPS.indexOf(step);
  if (i === TOUR_STEPS.length - 1) return endTour();
  const next = TOUR_STEPS[i + 1];
  show(next === "undo" && !runCards(useAgent.getState().items) ? "play" : next);
}

export function endTour() {
  if (!useTour.getState().step) return;
  window.clearTimeout(leaveWatch);
  useWordPick.setState({ startUs: null });
  useTour.setState({ step: null, epoch: null, stopped: false });
  if (closedPanel) {
    closedPanel = false;
    useDock.setState({ open: true });
  }
  const agent = useAgent.getState();
  // A request the tour wrote and nobody sent goes; what was typed before comes back.
  if (agent.draft === SAMPLE_REQUEST) useAgent.setState({ draft: keptDraft });
  // The panel stays a demo until its turn ends.
  if (agent.status === "idle") useAgent.setState({ demo: false });
}

/** Opens the sample as a new project and starts the tour on it, with the request in the AI panel's field. */
export function trySample() {
  useSetup.getState().closeSetup();
  newSampleProject(() => {
    const snap = useEditor.getState().snap;
    if (!snap) return;
    const { draft } = useAgent.getState();
    if (draft !== SAMPLE_REQUEST) keptDraft = draft;
    useAgent.setState({ demo: true, draft: SAMPLE_REQUEST, thumbnail: false });
    useDock.setState({ open: true });
    useTour.setState({ epoch: snap.sessionEpoch });
    show("ask");
  });
}

/** Done's way on: the person's own video, as Open a talking head video… on the first start. */
export function openOwnVideo() {
  endTour();
  void pickVideosForNewProject();
}

useAgent.subscribe((s, prev) => {
  const { step } = useTour.getState();
  if (step === "ask" && s.demo && s.status === "working" && prev.status === "idle") show("watch");
  if (step === "watch" && s.status === "idle" && prev.status !== "idle") {
    const outcome = lastDemoOutcome();
    if (outcome === "stopped") useTour.setState({ stopped: true });
    // The request is back in the field, to send again.
    else if (outcome === "failed") show("ask");
    else leaveWatch = window.setTimeout(() => show(runCards(useAgent.getState().items) ? "undo" : "play"), WATCH_AFTER_MS);
  }
  // A turn that outlived the tour ends the demo with it.
  if (!step && s.demo && s.status === "idle" && prev.status !== "idle") useAgent.setState({ demo: false });
});

useEditor.subscribe((s, prev) => {
  const { step, epoch } = useTour.getState();
  if (!step) return;
  // Another project or the home screen ends it.
  if (s.view === "home" || s.snap?.sessionEpoch !== epoch) return endTour();
  if (step === "undo" && s.snap && s.snap !== prev.snap) followUndo(s.snap);
  // Cut from the transcript: the video got shorter.
  if (step === "transcript" && s.snap && prev.snap && s.snap !== prev.snap && projectDuration(s.snap.project) < projectDuration(prev.snap.project)) show("song");
  if (step === "song" && s.snap !== prev.snap && s.snap && songClip(s.snap.project)) show("volume");
});

/**
 * The first launch opens Get set up by itself, once: Nuzky remembered nothing here when it started, and the projects,
 * once listed, are only the empty one Nuzky made. Someone who updates, even with no projects left, never sees it.
 */
useLibrary.subscribe((s) => {
  if (useTour.getState().firstLaunch !== null || s.projects === null) return;
  const open = !usedBefore && onlyPristine(s.projects, useEditor.getState().snap);
  useTour.setState({ firstLaunch: open });
  if (open) useSetup.getState().openSetup();
});

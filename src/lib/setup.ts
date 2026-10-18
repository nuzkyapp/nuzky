import { create } from "zustand";
import { api } from "./api";
import { useSounds } from "./sounds";

export const SETUP_STEPS = ["captions", "vision", "music", "ai"] as const;
export type SetupStep = (typeof SETUP_STEPS)[number];
type StepState = "done" | "skipped";
type Saved = { states: Partial<Record<SetupStep, StepState>>; current: SetupStep };

type Setup = Saved & {
  open: boolean;
  openSetup: () => void;
  closeSetup: () => void;
  showStep: (step: SetupStep) => void;
  finishStep: (step: SetupStep) => void;
  /** Marks it skipped and shows the next step; the last one stays shown. */
  skipStep: (step: SetupStep) => void;
};

const KEY = "nuzky.setup";
const isStep = (v: unknown): v is SetupStep => SETUP_STEPS.includes(v as SetupStep);

function load(): Saved {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "null");
    const states = Object.fromEntries(
      Object.entries(v?.states ?? {}).filter(([step, state]) => isStep(step) && (state === "done" || state === "skipped")),
    ) as Saved["states"];
    return { states, current: isStep(v?.current) ? v.current : "captions" };
  } catch {
    return { states: {}, current: "captions" };
  }
}

export const useSetup = create<Setup>((set) => ({
  ...load(),
  open: false,
  openSetup: () => set({ open: true }),
  closeSetup: () => set({ open: false }),
  showStep: (current) => set({ current }),
  finishStep: (step) => set((s) => ({ states: { ...s.states, [step]: "done" } })),
  skipStep: (step) =>
    set((s) => ({ states: { ...s.states, [step]: "skipped" }, current: SETUP_STEPS[Math.min(SETUP_STEPS.indexOf(step) + 1, SETUP_STEPS.length - 1)] })),
}));
useSetup.subscribe((s) => localStorage.setItem(KEY, JSON.stringify({ states: s.states, current: s.current })));

export const stepsLeft = (states: Saved["states"]) => SETUP_STEPS.filter((step) => !states[step]).length;

/** Music from YouTube is done once Nuzky has its own yt-dlp, from wherever it was downloaded. */
export async function checkYtdlp() {
  if ((await api.ytdlpDownloaded()) && useSetup.getState().states.music !== "done") useSetup.getState().finishStep("music");
}
// At once when it downloaded, so the step never shows Download yt-dlp again on its way to done.
useSounds.subscribe((s, prev) => {
  if (!prev.installing || s.installing) return;
  if (!s.installError) useSetup.getState().finishStep("music");
  else void checkYtdlp();
});

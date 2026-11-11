import { create } from "zustand";
import { PRESETS, choosePreset, keysOf, useKeys, type PresetId } from "./keys";
import { usedBefore } from "./launch";
import { useSetup } from "./setup";
import { useEditor } from "./store";
import { useTour } from "./tour";

const KEY = "nuzky.keysQuestion";

/**
 * Which editor's keys to use, asked by itself once, at the first project of a profile new at this launch or an earlier
 * one; a profile that used Nuzky before the question existed is never asked by itself. Settings › General asks again.
 */
export const useKeysQuestion = create<{ open: boolean; due: boolean }>(() => ({
  open: false,
  due: localStorage.getItem(KEY) === "due" || (!usedBefore && localStorage.getItem(KEY) === null),
}));
// Written at once, so a launch that ends before the first project still asks at the next one.
localStorage.setItem(KEY, useKeysQuestion.getState().due ? "due" : "asked");
useKeysQuestion.subscribe((s, prev) => s.due !== prev.due && localStorage.setItem(KEY, s.due ? "due" : "asked"));

export const askKeysQuestion = () => useKeysQuestion.setState({ open: true });

/** A set's keys, or null to keep the keys as they are; either way it is not asked again by itself. */
export function answerKeysQuestion(preset: PresetId | null) {
  useKeysQuestion.setState({ open: false, due: false });
  if (!preset) return;
  const undo = useKeys.getState();
  choosePreset(preset);
  const dropped = Object.keys(undo.changed).length;
  const split = keysOf("split")[0];
  useEditor.getState().toast({
    kind: "success",
    text: `Keys like ${PRESETS[preset].name}.${split ? ` ${split} splits.` : ""}${dropped ? ` ${dropped === 1 ? "Your 1 changed key is" : `Your ${dropped} changed keys are`} gone.` : ""}`,
    ...(dropped > 0 && { action: { label: "Undo", run: () => useKeys.setState(undo) } }),
  });
}

/** Asks once the project shows by itself, not over the tour, Get set up or a recovered project. */
function askIfDue() {
  const q = useKeysQuestion.getState();
  const { view, snap } = useEditor.getState();
  if (!q.due || q.open || view !== "editor" || !snap || snap.recovery || useTour.getState().step || useSetup.getState().open) return;
  askKeysQuestion();
}

useEditor.subscribe((s, prev) => {
  // After the switch has run its own next step, such as the tour's start.
  if (s.view === "editor" && s.snap && (prev.view !== "editor" || prev.snap?.sessionEpoch !== s.snap.sessionEpoch)) window.setTimeout(askIfDue);
});

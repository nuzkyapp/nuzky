import { create } from "zustand";
import { api, errorText, plainError } from "./api";
import { reportBug } from "./diagnostics";
import { useKeysQuestion } from "./keysQuestion";
import { usedBefore } from "./launch";
import { useSetup } from "./setup";
import { useEditor } from "./store";
import { useTour } from "./tour";
import type { TelemetryStatus } from "./types";

/**
 * Help improve Nuzky: what Nuzky may send its author (src-tauri/src/telemetry.rs), and the question that asks once.
 * A profile that used Nuzky before is asked at the first launch that knows the question, a new one at its second
 * launch, after its first one showed the tour and Get set up.
 */
export const useTelemetry = create<{
  status: TelemetryStatus | null;
  open: boolean;
  due: boolean;
  error: string | null;
  /** The crash notice still offers its report. */
  crashReport: boolean;
}>(() => ({
  status: null,
  open: false,
  due: false,
  error: null,
  crashReport: false,
}));

export const CRASHED = "Nuzky closed unexpectedly. Your project is as it was last saved.";

export function startTelemetry(status: TelemetryStatus) {
  useTelemetry.setState({ status, due: !status.asked && usedBefore });
  askIfDue();
}

/** Asks once nothing else is in the way: a recovered project, the tour, Get set up or the keys question. */
function askIfDue() {
  const t = useTelemetry.getState();
  const { snap } = useEditor.getState();
  if (!t.due || t.open || !snap || snap.recovery || useTour.getState().step || useSetup.getState().open || useKeysQuestion.getState().open) return;
  useTelemetry.setState({ open: true, due: false });
}
useEditor.subscribe((s, prev) => s.snap?.recovery !== prev.snap?.recovery && window.setTimeout(askIfDue));
useTour.subscribe((s, prev) => s.step !== prev.step && window.setTimeout(askIfDue));
useSetup.subscribe((s, prev) => s.open !== prev.open && window.setTimeout(askIfDue));
useKeysQuestion.subscribe((s, prev) => s.open !== prev.open && window.setTimeout(askIfDue));

async function act(run: () => Promise<TelemetryStatus>) {
  useTelemetry.setState({ error: null });
  try {
    useTelemetry.setState({ status: await run() });
    return true;
  } catch (e) {
    useTelemetry.setState({ error: plainError(errorText(e)) });
    return false;
  }
}

export const setTelemetry = (problems: boolean, conversations: boolean) => act(() => api.setTelemetry(problems, conversations));
export const deleteTelemetry = () => act(api.deleteTelemetry);

/** The question's answer: Share with what was ticked, or Not now. */
export async function answerTelemetry(choice: { problems: boolean; conversations: boolean } | null) {
  useTelemetry.setState({ open: false });
  if (!choice) {
    useTelemetry.setState((s) => ({ status: s.status && { ...s.status, asked: true } }));
    return void api.telemetryNotNow().catch(() => {});
  }
  if (!(await setTelemetry(choice.problems, choice.conversations)))
    return void useEditor.getState().toast({ kind: "error", text: useTelemetry.getState().error ?? "Nuzky couldn't save your choice." });
  if (choice.problems || choice.conversations) useEditor.getState().toast({ kind: "success", text: "Thank you! Change it any time in Settings › Privacy." });
  // Problems and usage covers the report the crash notice offers.
  if (choice.problems && useTelemetry.getState().crashReport) void sendReport();
}

/** Send crash report: the report goes at once, whether or not Problems and usage is on, and the notice goes. */
export async function sendReport() {
  const { toast, toasts, dismissToast } = useEditor.getState();
  useTelemetry.setState({ crashReport: false });
  const notice = toasts.find((t) => t.text === CRASHED);
  if (notice) dismissToast(notice.id);
  try {
    await api.sendReport("");
    toast({ kind: "success", text: "Crash report sent. Thank you!" });
  } catch (e) {
    toast({ kind: "error", text: plainError(errorText(e)), action: { label: "Report a bug…", run: reportBug } });
  }
}

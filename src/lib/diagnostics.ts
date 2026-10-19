import { create } from "zustand";
import { api, errorText } from "./api";
import { copyText } from "./clipboard";
import { useEditor } from "./store";

/** Tells the person how an action went: a toast, or a line of its own where toasts do not show. */
export type Say = (kind: "success" | "error", text: string) => void;

const toast: Say = (kind, text) => useEditor.getState().toast({ kind, text });

export const useBugReport = create<{ open: boolean }>(() => ({ open: false }));

/** Opens the Report a bug dialog. */
export const reportBug = () => useBugReport.setState({ open: true });

/** Where the window cannot draw the dialog: saves the report and opens the form without a description. */
export async function reportBugNow(say: Say) {
  try {
    const { name, openError } = await api.reportBug("");
    say("success", openError ? `Saved ${name} in the logs folder. Couldn't open the browser: attach it to a new issue at github.com/nuzkyapp/nuzky.` : `GitHub opened. Drag ${name} from the logs folder into the issue.`);
  } catch (e) {
    say("error", `Couldn't save the report: ${errorText(e)}`);
  }
}

export function showBugReport() {
  api.showBugReport().catch((e) => toast("error", `Couldn't show the report: ${errorText(e)}`));
}

/** Puts the whole bug report on the clipboard. */
export async function copyDiagnostics(say = toast) {
  try {
    await copyText(await api.diagnostics(), "The clipboard is not available. Open the logs folder and attach nuzky.log instead.");
    say("success", "Diagnostics copied. Paste them into your bug report.");
  } catch (e) {
    say("error", `Couldn't copy the diagnostics: ${errorText(e)}`);
  }
}

export function openLogs() {
  api.openLogs().catch((e) => toast("error", `Couldn't open the logs folder: ${errorText(e)}`));
}

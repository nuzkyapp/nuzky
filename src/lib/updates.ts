import { Channel } from "@tauri-apps/api/core";
import { create } from "zustand";
import { useAgent } from "./agent";
import { api, errorText } from "./api";
import { showEditor } from "./library";
import { useEditor, whenIdle } from "./store";
import type { JobEvent } from "./types";

/**
 * Whether a newer Nuzky was released, and installing it. A check asks the backend, which reads only
 * the version on the latest GitHub release. Automatic checks run 10 s after start and then at most
 * once a day, also across launches, and say nothing when they fail; Check for updates always answers.
 * Only Install and restart downloads anything: the backend keeps the package once its signature
 * matches the key built into Nuzky, and installs it and restarts when the user is ready.
 */
type Updates = {
  /** This Nuzky's version. */
  version: string;
  /** False when update checks are turned off for this installation; nothing is asked then. */
  enabled: boolean;
  /** Check by itself, as the user chose; on unless turned off. */
  auto: boolean;
  /** A newer released version, or null. */
  latest: string | null;
  /** This copy can install `latest` itself; otherwise the release page has its installers. */
  canInstall: boolean;
  /** A check in flight; one the user asked for shows until it answers. */
  checking: "auto" | "manual" | null;
  /** Installing `latest`: downloading (the percentage, null until the size is known), downloaded and waiting for a restart, being installed, or installed and waiting for a restart. */
  install: { stage: "downloading"; percent: number | null } | { stage: "ready" | "installing" | "installed" } | null;
};

type Saved = { auto: boolean; checkedAt: number; latest: string | null; canInstall: boolean; /** The version that checked; another one checks anew. */ by: string };

/** What the restarted Nuzky gets from the one that installed the update (`take_note` in updates.rs). */
type Note = { from: string; view: "home" | "editor"; draft: string };

const KEY = "nuzky.updates";
const FIRST_CHECK_MS = 10_000;
const DAY_MS = 24 * 60 * 60 * 1000;
const HOUR_MS = 60 * 60 * 1000;
const RESTART_IN_S = 5;

function load(): Partial<Saved> {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "null");
    return v && typeof v === "object" ? v : {};
  } catch {
    return {};
  }
}

let checkedAt = 0;
let started = false;

export const useUpdates = create<Updates>(() => ({ version: "", enabled: false, auto: true, latest: null, canInstall: false, checking: null, install: null }));

function save() {
  const { version, auto, latest, canInstall } = useUpdates.getState();
  localStorage.setItem(KEY, JSON.stringify({ auto, checkedAt, latest, canInstall, by: version } satisfies Saved));
}

/** Once per launch, after boot. `note` comes from the Nuzky that restarted to update; only one boot gets it. */
export function startUpdates(version: string, enabled: boolean, note: string | null) {
  if (note) restarted(version, note);
  if (started) return;
  started = true;
  const saved = load();
  const same = saved.by === version;
  checkedAt = same && Number.isFinite(saved.checkedAt) ? Number(saved.checkedAt) : 0;
  useUpdates.setState({ version, enabled, auto: saved.auto !== false, latest: same && typeof saved.latest === "string" ? saved.latest : null, canInstall: same && saved.canInstall === true });
  if (!enabled) return;
  window.setTimeout(checkIfDue, FIRST_CHECK_MS);
  window.setInterval(checkIfDue, HOUR_MS);
}

/** Back where the update was installed from, with what was typed for the AI. */
function restarted(version: string, text: string) {
  let note: Partial<Note>;
  try {
    note = JSON.parse(text);
  } catch {
    return;
  }
  if (note.from && note.from !== version) useEditor.getState().toast({ kind: "success", text: `Updated to Nuzky ${version}.` });
  if (note.view === "editor") showEditor();
  if (note.draft && !useAgent.getState().draft) useAgent.setState({ draft: note.draft });
}

function checkIfDue() {
  if (useUpdates.getState().auto && Date.now() - checkedAt >= DAY_MS) void checkForUpdates(false);
}

/** `manual` answers with a toast, also when this is the latest version or the check failed, and also when it joins a check that was running. */
export async function checkForUpdates(manual: boolean) {
  const { enabled, checking, version, install } = useUpdates.getState();
  // While an update is installed, the version it installs stays the one offered.
  if (!enabled || install) return;
  // A check already on its way answers this one too.
  if (checking) {
    if (manual) useUpdates.setState({ checking: "manual" });
    return;
  }
  useUpdates.setState({ checking: manual ? "manual" : "auto" });
  const asked = () => useUpdates.getState().checking === "manual";
  // A failed check counts too, so an unreachable server is asked at most once a day.
  checkedAt = Date.now();
  try {
    const found = await api.checkForUpdate();
    const latest = found?.version ?? null;
    useUpdates.setState({ latest, canInstall: !!found?.canInstall });
    if (asked()) {
      const toast = useEditor.getState().toast;
      if (latest)
        toast({ kind: "info", text: `Nuzky ${latest} is available.`, action: found?.canInstall ? { label: "Install and restart", run: () => void installUpdate() } : { label: "Download from GitHub…", run: downloadUpdate } });
      else toast({ kind: "success", text: `Nuzky ${version} is the latest version.` });
    }
  } catch (e) {
    console.warn("Update check failed", errorText(e));
    if (asked()) useEditor.getState().toast({ kind: "error", text: "Couldn't check for updates. Check your internet connection or try again later." });
  } finally {
    useUpdates.setState({ checking: null });
    save();
  }
}

export function setAutoCheck(auto: boolean) {
  useUpdates.setState({ auto });
  save();
  if (auto) checkIfDue();
}

/** The work restarting would cut short, in order, as the backend names it too (`work_of` in updates.rs). Preparing sound, a preview copy or a person outline starts again after the restart. */
const WORK: [string, (j: JobEvent) => boolean][] = [
  ["the export", (j) => j.kind === "export" || j.id.startsWith("cover-export:") || (j.kind === "agent" && j.label === "Exporting")],
  ["speech recognition", (j) => j.kind === "captions" || j.kind === "transcript" || (j.kind === "agent" && j.label === "Transcribing")],
  ["the model download", (j) => j.kind === "speech-model" || j.kind === "vision-models"],
  ["learning your style", (j) => j.kind === "style"],
  ["the analysis", (j) => j.kind !== "audio" && j.kind !== "proxy" && j.kind !== "matte"],
];

function workOf(aiRun: string | null, jobs: Record<string, JobEvent>): string | null {
  if (aiRun) return "an AI edit";
  const running = Object.values(jobs).filter((j) => j.status === "running");
  return WORK.find(([, is]) => running.some(is))?.[0] ?? null;
}

/** What restarting now would cut short, or null. */
export function restartBlocker(): string | null {
  if (useAgent.getState().status !== "idle") return "an AI edit";
  const { aiRun, jobs } = useEditor.getState();
  return workOf(aiRun, jobs);
}

/** `restartBlocker`, rendering again only when it changes. */
export function useRestartBlocker(): string | null {
  const agent = useAgent((s) => s.status !== "idle");
  const work = useEditor((s) => workOf(s.aiRun, s.jobs));
  return agent ? "an AI edit" : work;
}

export const waitFor = (work: string) => `Restart when ${work} finishes`;

/** Errors that trying again cannot fix; Download from GitHub can. */
const FINAL = ["UPDATE_NO_PACKAGE", "UPDATE_SIGNATURE", "UPDATE_SOURCE", "UPDATE_FOLDER"];

/** Downloads the newer version and checks its signature; then, with nothing running, restarts into it in 5 s unless the user says Not now. */
export async function installUpdate() {
  const { latest, install, canInstall } = useUpdates.getState();
  if (!latest || install) return;
  if (!canInstall) return downloadUpdate();
  const progress = new Channel<number>();
  progress.onmessage = (percent) => useUpdates.getState().install?.stage === "downloading" && useUpdates.setState({ install: { stage: "downloading", percent } });
  useUpdates.setState({ install: { stage: "downloading", percent: null } });
  try {
    await api.downloadUpdate(progress);
  } catch (e) {
    useUpdates.setState({ install: null });
    return failed(e, () => void installUpdate());
  }
  useUpdates.setState({ install: { stage: "ready" } });
  if (restartBlocker()) return;
  const text = (left: number) => `Nuzky ${latest} is ready. Restarting in ${left} s.`;
  useEditor.getState().toast({
    kind: "info",
    text: text(RESTART_IN_S),
    countdown: { seconds: RESTART_IN_S, text },
    // Work that starts meanwhile takes the toast away, and Restart to update waits for it.
    action: { label: "Restart now", run: () => void restartToUpdate(), valid: () => !restartBlocker() },
    secondary: { label: "Not now", run: focusUpdateButton },
  });
}

/** Saves the project, installs the downloaded update and restarts; work that started meanwhile postpones the restart. */
export async function restartToUpdate() {
  const { install, latest, version } = useUpdates.getState();
  if (install?.stage !== "ready" && install?.stage !== "installed") return;
  const toast = useEditor.getState().toast;
  const work = restartBlocker();
  if (work) return toast({ kind: "warning", text: `${waitFor(work)}.` });
  // A field keeps what was typed until Enter or leaving it; leaving it now saves it with the project.
  if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  useUpdates.setState({ install: { stage: "installing" } });
  await whenIdle();
  const note: Note = { from: version, view: useEditor.getState().view, draft: useAgent.getState().draft };
  try {
    // Unless work started while it installed, Nuzky restarts and this page ends here.
    const busy = await api.restartToUpdate(JSON.stringify(note));
    if (!busy) return;
    useUpdates.setState({ install: { stage: "installed" } });
    toast({ kind: "info", sticky: true, text: `Nuzky ${latest} is installed. ${busy}`, action: { label: "Restart", run: () => void restartToUpdate() } });
  } catch (e) {
    useUpdates.setState({ install: { stage: install.stage } });
    failed(e, () => void restartToUpdate());
  }
}

/** Says what went wrong and offers what can help: trying again, or the installers on GitHub. */
function failed(e: unknown, retry: () => void) {
  focusUpdateButton();
  const error = errorText(e);
  const code = error.match(/^[A-Z_]+(?=:)/)?.[0] ?? "";
  if (code === "CANCELLED") return;
  if (!code) console.warn("Update failed", error);
  const toast = useEditor.getState().toast;
  const text = code ? error.slice(code.length + 1).trim() : "Couldn't install the update. Try again.";
  if (code === "UPDATE_LATEST") {
    useUpdates.setState({ latest: null });
    save();
    return toast({ kind: "info", text });
  }
  if (code === "UPDATE_BUSY") return toast({ kind: "warning", text });
  const download = { label: "Download from GitHub…", run: downloadUpdate };
  if (!FINAL.includes(code)) return toast({ kind: "error", text, action: { label: "Try again", run: retry }, secondary: download });
  // This copy can't install this version; the menus offer the release page instead.
  useUpdates.setState({ install: null, canInstall: false });
  save();
  toast({ kind: "error", text, action: download });
}

export function cancelUpdate() {
  void api.cancelUpdate();
}

/** Where the keyboard goes on after the download's Stop, a failure or Not now took the focused control away. */
function focusUpdateButton() {
  requestAnimationFrame(() => {
    if (document.activeElement && document.activeElement !== document.body) return;
    document.querySelector<HTMLElement>(useEditor.getState().view === "home" ? "[data-version]" : "[data-update-button]")?.focus();
  });
}

/** Opens the latest release's page in the browser: what's new, and the installers. */
export function downloadUpdate() {
  api.openReleasePage().catch((e) => useEditor.getState().toast({ kind: "error", text: `Couldn't open the download page: ${errorText(e)}` }));
}

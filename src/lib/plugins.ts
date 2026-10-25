import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { send, sendBlocked, useAgent } from "./agent";
import { api, errorText, plainError } from "./api";
import { useDock } from "./dock";
import { PLUGIN_RUN, aiLocked, enqueue, stopAiRun, undoAction, useEditor, type Toast } from "./store";
import type { Project, Snapshot } from "./types";

export type PluginInput = { id: string; label: string } & (
  | { type: "number"; min: number; max: number; step: number; default: number; unit?: string }
  | { type: "toggle"; default: boolean }
  | { type: "choice"; options: { value: string; label: string }[]; default: string }
  | { type: "text"; default: string }
  | { type: "color"; default: string }
);

export type PluginValue = number | boolean | string;
export type PluginValues = Record<string, PluginValue>;

/** A plugin in Nuzky's plugins folder; one that cannot load has only its id, a name and `error`. */
export interface Plugin {
  id: string;
  name: string;
  description: string;
  /** The verb on its button, "Run" unless the plugin names one. */
  action: string;
  needs: string[];
  inputs: PluginInput[];
  error: string | null;
}

/** What the plugin would do to the project as it is: `project` after its edits, at `revision`. */
export interface PluginPreview {
  key: string;
  revision: number;
  project: Project | null;
  error: string | null;
  /** The app is still preparing the sound the plugin needs; a run prepares it itself. */
  waiting: boolean;
}

/** When a plugin the user turned on runs by itself: after an import they make in the app, or before an export. */
export type Trigger = "import" | "export";

const WHEN: Record<Trigger, string> = { import: "after import", export: "before export" };

interface PluginsState {
  list: Plugin[] | null;
  /** The plugin shown in the inspector. */
  open: string | null;
  /** The values last set for each plugin, which its automatic runs use too. */
  values: Record<string, PluginValues>;
  /** The plugins the user turned on for each trigger. Only the user turns them on, here; never a plugin or the project. */
  auto: Record<Trigger, string[]>;
  preview: PluginPreview | null;
  running: string | null;
  /** The automatic runs going on now; an export waits for its own. */
  pass: Trigger | null;
  /** Which of the pass's plugins runs now, from 1. */
  step: { index: number; total: number } | null;
  /** The pass before the export, which the export dialog shows until it closes. */
  report: PassReport | null;
}

const SETTINGS_KEY = "nuzky.plugins";

function savedSettings(): Pick<PluginsState, "values" | "auto"> {
  const ids = (v: unknown) => (Array.isArray(v) ? v.filter((id): id is string => typeof id === "string") : []);
  try {
    const v = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? "null") as { values?: unknown; auto?: Record<string, unknown> } | null;
    const values = v?.values && typeof v.values === "object" ? (v.values as Record<string, PluginValues>) : {};
    return { values, auto: { import: ids(v?.auto?.import), export: ids(v?.auto?.export) } };
  } catch {
    return { values: {}, auto: { import: [], export: [] } };
  }
}

export const usePlugins = create<PluginsState>(() => ({ list: null, open: null, ...savedSettings(), preview: null, running: null, pass: null, step: null, report: null }));

usePlugins.subscribe((s, prev) => {
  if (s.values !== prev.values || s.auto !== prev.auto) localStorage.setItem(SETTINGS_KEY, JSON.stringify({ values: s.values, auto: s.auto }));
});

export function setAuto(id: string, trigger: Trigger, on: boolean) {
  usePlugins.setState((s) => ({ auto: { ...s.auto, [trigger]: [...s.auto[trigger].filter((i) => i !== id), ...(on ? [id] : [])] } }));
}

/** The values last set that the plugin still takes; a setting the AI removed or retyped since goes back to its default. */
export function valuesFor(p: Plugin, set: PluginValues | undefined): PluginValues {
  const fits = (input: PluginInput, v: PluginValue | undefined) =>
    typeof v === typeof input.default &&
    (input.type !== "choice" || input.options.some((o) => o.value === v)) &&
    (input.type !== "number" || ((v as number) >= input.min && (v as number) <= input.max));
  return Object.fromEntries(p.inputs.map((input) => [input.id, fits(input, set?.[input.id]) ? set![input.id] : input.default]));
}

export const valuesOf = (p: Plugin): PluginValues => valuesFor(p, usePlugins.getState().values[p.id]);

export function setValue(id: string, input: string, value: PluginValue) {
  usePlugins.setState((s) => ({ values: { ...s.values, [id]: { ...s.values[id], [input]: value } } }));
}

/** A plugin as the backend lists it: the manifest of one that loads, else why it does not. */
export interface ListedPlugin {
  id: string;
  error: string | null;
  manifest: { name: string; description: string; action: string; needs: string[]; inputs: PluginInput[] } | null;
}

const listed = ({ id, error, manifest: m }: ListedPlugin): Plugin => ({
  id,
  name: m?.name ?? id.charAt(0).toUpperCase() + id.slice(1).replaceAll("-", " "),
  description: m?.description ?? "",
  action: m?.action ?? "Run",
  needs: m?.needs ?? [],
  inputs: m?.inputs ?? [],
  error: m ? null : (error ?? "Can't load."),
});

export async function loadPlugins() {
  try {
    const list = (await api.listPlugins()).map(listed);
    usePlugins.setState((s) => ({ list, open: s.open && list.some((p) => p.id === s.open) ? s.open : null }));
  } catch (e) {
    usePlugins.setState({ list: [] });
    useEditor.getState().toast({ kind: "error", text: `Couldn't read your plugins: ${errorText(e)}` });
  }
}

export function listenPlugins() {
  return listen("plugins-changed", () => void loadPlugins());
}

// A preview of a plugin no longer shown would only keep a stuck script running.
usePlugins.subscribe((s, prev) => {
  if (prev.open && s.open !== prev.open) void api.cancelPluginPreview().catch(() => {});
});

// The inspector shows a plugin only while the Plugins tab is open, and never one of another project.
useEditor.subscribe((s, prev) => {
  if (usePlugins.getState().open && (s.panelTab !== "plugins" || s.snap?.sessionEpoch !== prev.snap?.sessionEpoch)) usePlugins.setState({ open: null, preview: null });
});

/** Shows the plugin in the inspector, from the Plugins tab or the launcher. */
export function openPlugin(id: string | null) {
  usePlugins.setState({ open: id, preview: null });
  if (id) useEditor.setState({ panelTab: "plugins", cut: null });
}

/** Why the plugin cannot run on this project now, or null. */
export function pluginBlocked(p: Plugin, hasWords: boolean): string | null {
  if (p.error) return "This plugin can't load. Ask the AI to fix it.";
  if (p.needs.includes("words") && !hasWords) return "Needs to know what is said. Transcribe first.";
  return null;
}

/** The text after "CODE: ", for showing an error the plugin caused. */
export const plainDetail = (text: string) => text.replace(/^[A-Z][A-Z_]+: ?/, "");

/** Whether the AI can fix the error by changing the plugin; a plugin made for a newer Nuzky needs an update instead. */
export const fixable = (error: string) => !error.startsWith("PLUGIN_API_NEWER");

let previews = 0;

export async function previewPlugin(p: Plugin, values: PluginValues, key: string) {
  const { snap, selection, timeUs } = useEditor.getState();
  if (!snap) return;
  const mine = ++previews;
  let preview: PluginPreview;
  try {
    const project = await api.previewPlugin(p.id, values, selection, timeUs);
    preview = { key, revision: snap.revision, project, error: null, waiting: false };
  } catch (e) {
    const text = errorText(e);
    // A newer request cancelled it.
    if (text.startsWith("CANCELLED")) return;
    preview = { key, revision: snap.revision, project: null, error: plainDetail(text), waiting: text.startsWith("AUDIO_NOT_READY") };
  }
  // An older answer arriving late, a closed plugin or a run that started meanwhile keeps what is shown.
  if (mine === previews && usePlugins.getState().open === p.id && !usePlugins.getState().running) usePlugins.setState({ preview });
}

const sentence = (text: string) => (/[.!?]$/.test(text) ? text : `${text}.`);

/** "Removed 6 pauses." read on after "ran after import: ". */
const continued = (text: string) => (/^[A-Z][a-z]/.test(text) ? text.charAt(0).toLowerCase() + text.slice(1) : text);

/** "A", "A and B", "A, B and C". */
const names = (list: string[]) => (list.length < 2 ? list.join("") : `${list.slice(0, -1).join(", ")} and ${list.at(-1)}`);

/** What the plugin's script threw, as a person reads it: without the code, the place in plugin.js and a bare "Error: ". */
export const readableError = (text: string) => plainDetail(text).replace(/^plugin\.js(?::\d+)*: /, "").replace(/^Error: /, "");

/** How one plugin's run went. `from` is the revision it started on, so a pass knows its steps follow one another. */
export type Ran = { plugin: Plugin } & (
  | { did: "changed"; message: string | null; snap: Snapshot; from: number | undefined }
  | { did: "nothing"; message: string | null }
  | { did: "failed"; error: string; loaded: boolean }
  | { did: "stopped" }
);

/** One pass of the plugins turned on for a trigger: how each run went, and those that did not get to run. */
export interface PassReport {
  trigger: Trigger;
  results: Ran[];
  skipped: { names: string[]; all: boolean; reason: string } | null;
  /** Cancel export ended it; the export does not start. */
  cancelled: boolean;
  done: boolean;
}

/** Runs it in a run of its own: the AI lock with Stop while it runs, and one undo step. */
async function execute(p: Plugin, selection: string[], timeUs: number): Promise<Ran> {
  const values = valuesOf(p);
  // The last preview stays, dimmed, so the panel keeps its height while the run goes on.
  usePlugins.setState({ running: p.id });
  const out: { failure?: string; message?: string | null; from?: number } = {};
  const snap = await enqueue(async (epoch) => {
    out.from = useEditor.getState().snap?.revision;
    try {
      const done = await api.runPlugin(p.id, values, selection, timeUs, epoch);
      out.message = done.message;
      return done.snapshot;
    } catch (e) {
      out.failure = errorText(e);
      return null;
    }
  });
  usePlugins.setState({ running: null });
  if (out.failure !== undefined)
    return out.failure.startsWith("CANCELLED") || out.failure.startsWith("RUN_STOPPED") ? { plugin: p, did: "stopped" } : { plugin: p, did: "failed", error: out.failure, loaded: true };
  const message = out.message ?? null;
  return snap ? { plugin: p, did: "changed", message, snap, from: out.from } : { plugin: p, did: "nothing", message };
}

/** The run a person started from the inspector, on the selection and at the playhead, and a toast that says what it did. */
export async function runPlugin(p: Plugin) {
  if (aiLocked() || usePlugins.getState().running) return;
  const { selection, timeUs } = useEditor.getState();
  const ran = await execute(p, selection, timeUs);
  const toast = useEditor.getState().toast;
  if (ran.did === "stopped") return toast({ kind: "info", text: `${p.name} stopped. Nothing changed.` });
  if (ran.did === "failed") return toast(failureToast(ran, null));
  if (ran.did === "changed") return toast({ kind: "success", text: ran.message ?? `${p.name} is done.`, action: undoAction(ran.snap) });
  toast({ kind: "info", text: ran.message ?? `${p.name} found nothing to change.` });
}

/** What a run did, in a sentence: `when` is "after import" or "before export", or empty inside the export dialog. */
export function ranText(r: Exclude<Ran, { did: "failed" }>, when: string) {
  const at = when && ` ${when}`;
  if (r.did === "stopped") return `${r.plugin.name} was stopped${at}. Nothing changed.`;
  if (r.message) return `${r.plugin.name} ran${at}: ${continued(sentence(r.message))}`;
  return r.did === "changed" ? `${r.plugin.name} ran${at}.` : `${r.plugin.name} ran${at} and found nothing to change.`;
}

/** Why it failed, in a sentence, without the place in its script, which Copy error and Fix with AI keep. */
export function failedText(r: Extract<Ran, { did: "failed" }>, when: string) {
  const at = when && ` ${when}`;
  if (!r.loaded) return `${r.plugin.name} didn't run${at}: it can't load. ${sentence(plainDetail(r.error))}`;
  if (busy(r.error)) return `${r.plugin.name} didn't run${at}: ${continued(sentence(plainError(r.error)))}`;
  return `${r.plugin.name} failed${at}: ${sentence(readableError(r.error))} Nothing changed.`;
}

export const skippedText = (skipped: NonNullable<PassReport["skipped"]>, when: string) =>
  `${skipped.all && skipped.names.length > 1 ? "Plugins" : names(skipped.names)} didn't run${when && ` ${when}`}: ${skipped.reason}.`;

export const copyError = (error: string) => void navigator.clipboard.writeText(error);

const busy = (error: string) => error.startsWith("RUN_ACTIVE") || error.startsWith("EPOCH_CHANGED");

function failureToast(r: Extract<Ran, { did: "failed" }>, when: string | null): Omit<Toast, "id"> {
  const { plugin: p, error } = r;
  if (busy(error)) return { kind: "error", text: when === null ? plainError(error) : failedText(r, when) };
  return {
    kind: "warning",
    text: when === null && r.loaded ? `${p.name} failed: ${sentence(readableError(error))} Nothing changed.` : failedText(r, when ?? ""),
    action: fixable(error) ? { label: "Fix with AI", run: () => fixWithAi(p, error) } : undefined,
    secondary: { label: "Copy error", run: () => copyError(error) },
  };
}

/** One toast for what the pass's plugins did, with one Undo for all their steps; one per plugin that failed. */
function announce({ trigger, results, skipped }: PassReport) {
  const toast = useEditor.getState().toast;
  const when = WHEN[trigger];
  const ran = results.filter((r): r is Exclude<Ran, { did: "failed" }> => r.did !== "failed");
  const changed = ran.filter((r): r is Extract<Ran, { did: "changed" }> => r.did === "changed");
  if (ran.length) {
    const item = (r: (typeof ran)[number]) =>
      r.did === "stopped" ? `${r.plugin.name}: stopped, nothing changed.` : `${r.plugin.name}: ${r.message ? continued(sentence(r.message)) : r.did === "changed" ? "changed the project." : "nothing to change."}`;
    // Undo takes back the whole pass only while its steps follow one another, with nobody's edit between them.
    const together = changed.every((r, i) => i === 0 || (r.from === changed[i - 1].snap.revision && r.snap.sessionEpoch === changed[i - 1].snap.sessionEpoch));
    const last = changed.at(-1);
    toast({
      kind: last ? "success" : "info",
      text: ran.length === 1 ? ranText(ran[0], when) : `Plugins ran ${when}. ${ran.map(item).join(" ")}`,
      action: last && together ? undoAction(last.snap, changed.length) : undefined,
    });
  }
  for (const r of results) if (r.did === "failed") toast(failureToast(r, when));
  if (skipped) toast({ kind: "info", text: skippedText(skipped, when) });
}

let cancelRequested = false;

/** Cancel export: stops the plugin running before the export, skips the rest, and the export never starts. */
export function cancelExport() {
  if (usePlugins.getState().pass !== "export") return;
  cancelRequested = true;
  if (usePlugins.getState().running) void stopAiRun();
}

/**
 * Runs the plugins the user turned on for `trigger`, one after another by name, each its own undo step, on the whole
 * project from its start with the values last set. None of them blocks the import or export. Before an export the
 * dialog shows how each went while it is open, and the toasts wait until it closes.
 */
export async function runAutomatically(trigger: Trigger): Promise<PassReport | null> {
  if (usePlugins.getState().auto[trigger].length === 0) return null;
  cancelRequested = false;
  let report: PassReport = { trigger, results: [], skipped: null, cancelled: false, done: false };
  const publish = (patch: Partial<PassReport>) => {
    report = { ...report, ...patch };
    if (trigger === "export") usePlugins.setState({ report });
  };
  usePlugins.setState({ pass: trigger, step: null });
  publish({});
  try {
    await loadPlugins();
    const { list, auto } = usePlugins.getState();
    const chosen = (list ?? []).filter((p) => auto[trigger].includes(p.id)).sort((a, b) => a.name.localeCompare(b.name, "en") || a.id.localeCompare(b.id, "en"));
    for (const [i, p] of chosen.entries()) {
      if (cancelRequested) break;
      const { aiRun } = useEditor.getState();
      // A plugin run that has ended may not have said so yet; the backend refuses a run while another is open.
      const agent = !!aiRun && !aiRun.startsWith(PLUGIN_RUN);
      if (agent || usePlugins.getState().running) {
        publish({ skipped: { names: chosen.slice(i).map((c) => c.name), all: i === 0, reason: agent ? "the AI is editing" : "another plugin is running" } });
        break;
      }
      usePlugins.setState({ step: { index: i + 1, total: chosen.length } });
      const ran: Ran = p.error ? { plugin: p, did: "failed", error: p.error, loaded: false } : await execute(p, [], 0);
      publish({ results: [...report.results, ran] });
    }
  } finally {
    usePlugins.setState({ pass: null, step: null });
  }
  publish({ cancelled: cancelRequested, done: true });
  if (trigger === "import" || !useEditor.getState().exportOpen) {
    if (trigger === "export") usePlugins.setState({ report: null });
    announce(report);
  }
  return report;
}

// The export dialog showed how the plugins before the export went; once it closes, the toasts say it.
useEditor.subscribe((s, prev) => {
  const { report } = usePlugins.getState();
  if (prev.exportOpen && !s.exportOpen && report?.done) {
    usePlugins.setState({ report: null });
    announce(report);
  }
});

/** The project as the plugins for export last left it; exporting it again, as Replace and Retry do, runs none. */
let exportReady: { epoch: string; revision: number } | null = null;

/** Runs the plugins the user turned on for export, and resolves whether the export may start. */
export async function runBeforeExport() {
  const snap = () => useEditor.getState().snap;
  const now = snap();
  if (!now || (exportReady?.epoch === now.sessionEpoch && exportReady.revision === now.revision)) return true;
  const report = await runAutomatically("export");
  if (report?.cancelled) return false;
  const after = snap();
  // Plugins the AI kept from running get their turn at the next export.
  exportReady = report?.skipped ? null : after && { epoch: after.sessionEpoch, revision: after.revision };
  return true;
}

function openAi(draft?: string) {
  if (draft !== undefined) useAgent.setState({ draft });
  useDock.setState({ open: true });
  window.setTimeout(() => {
    const field = document.querySelector<HTMLTextAreaElement>('aside[aria-label="AI"] textarea');
    field?.focus();
    field?.setSelectionRange(field.value.length, field.value.length);
  }, 0);
}

/** Make a plugin: the AI panel with the request started, for the person to finish. */
export const makeWithAi = () => openAi("Make a Nuzky plugin that ");

/** One of the suggestions in the empty Plugins tab, sent as it is. */
export function askAi(text: string) {
  if (sendBlocked()) return openAi(text);
  openAi();
  void send(text, "project");
}

export const changeWithAi = (p: Plugin) => openAi(`Change the Nuzky plugin “${p.name}” (${p.id}) so that `);

export const fixWithAi = (p: Plugin, error: string) => askAi(`Fix the Nuzky plugin “${p.name}” (${p.id}). Nuzky says: ${error}`);

export async function removePlugin(p: Plugin) {
  try {
    await api.removePlugin(p.id);
    if (usePlugins.getState().open === p.id) openPlugin(null);
    // A plugin saved later under the same folder name must not run by itself until the user turns it on.
    setAuto(p.id, "import", false);
    setAuto(p.id, "export", false);
    useEditor.getState().toast({ kind: "info", text: `Moved “${p.name}” to the Trash.` });
    await loadPlugins();
  } catch (e) {
    useEditor.getState().toast({ kind: "error", text: `Couldn't move “${p.name}” to the Trash: ${errorText(e)}` });
  }
}

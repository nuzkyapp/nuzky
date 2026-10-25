import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { send, sendBlocked, useAgent } from "./agent";
import { api, errorText, plainError } from "./api";
import { useDock } from "./dock";
import { PLUGIN_RUN, aiLocked, enqueue, undoAction, useEditor } from "./store";
import type { Project } from "./types";

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

export const usePlugins = create<PluginsState>(() => ({ list: null, open: null, ...savedSettings(), preview: null, running: null, pass: null }));

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

/**
 * Runs the plugin with the values last set for it, in a run of its own: the AI lock with Stop while it runs, one
 * undo step, and a toast that says what it did. With `trigger` it runs by itself on the whole project, from its
 * start, and the toast says when it ran; when it cannot run, a toast says why, and the import or export goes on.
 */
export async function runPlugin(p: Plugin, trigger: Trigger | null = null) {
  const toast = useEditor.getState().toast;
  const when = trigger && WHEN[trigger];
  const after = trigger === "export" ? " The export goes on without it." : "";
  if (trigger) {
    const { aiRun } = useEditor.getState();
    // A plugin run that has ended may not have said so yet; the backend refuses a run while another is open.
    const agent = !!aiRun && !aiRun.startsWith(PLUGIN_RUN);
    if (agent || usePlugins.getState().running)
      return toast({ kind: "info", text: `${p.name} didn't run ${when}: ${agent ? "the AI is editing" : "another plugin is running"}.${after}` });
    if (p.error) {
      const error = p.error;
      return toast({
        kind: "warning",
        text: `${p.name} didn't run ${when}: it can't load. ${sentence(plainDetail(error))}${after}`,
        action: fixable(error) ? { label: "Fix with AI", run: () => fixWithAi(p, error) } : undefined,
      });
    }
  } else if (aiLocked() || usePlugins.getState().running) return;
  const values = valuesOf(p);
  const { selection, timeUs } = trigger ? { selection: [], timeUs: 0 } : useEditor.getState();
  usePlugins.setState({ running: p.id, preview: null });
  let failure: string | null = null;
  let message: string | null = null;
  const snap = await enqueue(async (epoch) => {
    try {
      const done = await api.runPlugin(p.id, values, selection, timeUs, epoch);
      message = done.message;
      return done.snapshot;
    } catch (e) {
      failure = errorText(e);
      return null;
    }
  });
  usePlugins.setState({ running: null });
  if (failure) {
    const text: string = failure;
    if (text.startsWith("CANCELLED") || text.startsWith("RUN_STOPPED"))
      return toast({ kind: "info", text: when ? `${p.name} was stopped ${when}. Nothing changed.${after}` : `${p.name} stopped. Nothing changed.` });
    if (text.startsWith("RUN_ACTIVE") || text.startsWith("EPOCH_CHANGED"))
      return toast({ kind: "error", text: when ? `${p.name} didn't run ${when}: ${continued(plainError(text))}` : plainError(text) });
    return toast({
      kind: "warning",
      text: `${when ? `${p.name} didn't run ${when}` : `${p.name} stopped`}: ${sentence(plainDetail(text))} Nothing changed.${after}`,
      action: { label: "Fix with AI", run: () => fixWithAi(p, text) },
      secondary: { label: "Copy error", run: () => void navigator.clipboard.writeText(text) },
    });
  }
  const said = message as string | null;
  const ran = when && `${p.name} ran ${when}`;
  if (snap) toast({ kind: "success", text: ran ? (said ? `${ran}: ${continued(said)}` : `${ran}.`) : (said ?? `${p.name} is done.`), action: undoAction(snap) });
  else toast({ kind: "info", text: ran ? (said ? `${ran}: ${continued(said)}` : `${ran} and found nothing to change.`) : (said ?? `${p.name} found nothing to change.`) });
}

/** Runs the plugins the user turned on for `trigger`, one after another by name, each its own undo step. */
export async function runAutomatically(trigger: Trigger) {
  if (usePlugins.getState().auto[trigger].length === 0) return;
  usePlugins.setState({ pass: trigger });
  try {
    await loadPlugins();
    const { list, auto } = usePlugins.getState();
    const chosen = (list ?? []).filter((p) => auto[trigger].includes(p.id)).sort((a, b) => a.name.localeCompare(b.name, "en") || a.id.localeCompare(b.id, "en"));
    for (const p of chosen) await runPlugin(p, trigger);
  } finally {
    usePlugins.setState({ pass: null });
  }
}

/** The project as the plugins for export last left it; exporting it again, as Replace and Retry do, runs none. */
let exportReady: { epoch: string; revision: number } | null = null;

/** Runs the plugins the user turned on for export, and resolves when the export may start. */
export async function runBeforeExport() {
  const snap = () => useEditor.getState().snap;
  const now = snap();
  if (!now || (exportReady?.epoch === now.sessionEpoch && exportReady.revision === now.revision)) return;
  await runAutomatically("export");
  const after = snap();
  exportReady = after && { epoch: after.sessionEpoch, revision: after.revision };
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

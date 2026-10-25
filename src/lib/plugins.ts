import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { send, sendBlocked, useAgent } from "./agent";
import { api, errorText, plainError } from "./api";
import { useDock } from "./dock";
import { aiLocked, enqueue, undoAction, useEditor } from "./store";
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

interface PluginsState {
  list: Plugin[] | null;
  /** The plugin shown in the inspector. */
  open: string | null;
  /** The values set for each plugin this session. */
  values: Record<string, PluginValues>;
  preview: PluginPreview | null;
  running: string | null;
}

export const usePlugins = create<PluginsState>(() => ({ list: null, open: null, values: {}, preview: null, running: null }));

/** The values set this session that the plugin still takes; a setting the AI removed or retyped since goes back to its default. */
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

export async function runPlugin(p: Plugin) {
  if (aiLocked() || usePlugins.getState().running) return;
  const values = valuesOf(p);
  const { selection, timeUs } = useEditor.getState();
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
  const toast = useEditor.getState().toast;
  if (failure) {
    const text: string = failure;
    if (text.startsWith("CANCELLED") || text.startsWith("RUN_STOPPED")) return toast({ kind: "info", text: `${p.name} stopped. Nothing changed.` });
    if (text.startsWith("RUN_ACTIVE") || text.startsWith("EPOCH_CHANGED")) return toast({ kind: "error", text: plainError(text) });
    return toast({
      kind: "warning",
      text: `${p.name} stopped: ${plainDetail(text)} Nothing changed.`,
      action: { label: "Fix with AI", run: () => fixWithAi(p, text) },
      secondary: { label: "Copy error", run: () => void navigator.clipboard.writeText(text) },
    });
  }
  if (snap) toast({ kind: "success", text: message ?? `${p.name} is done.`, action: undoAction(snap) });
  else toast({ kind: "info", text: message ?? `${p.name} found nothing to change.` });
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
    useEditor.getState().toast({ kind: "info", text: `Moved “${p.name}” to the Trash.` });
    await loadPlugins();
  } catch (e) {
    useEditor.getState().toast({ kind: "error", text: `Couldn't move “${p.name}” to the Trash: ${errorText(e)}` });
  }
}

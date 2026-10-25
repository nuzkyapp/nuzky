import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { send, sendBlocked, useAgent } from "./agent";
import { api, errorText, plainError } from "./api";
import { useDock } from "./dock";
import { PLUGIN_RUN, aiLocked, enqueue, stopAiRun, undoAction, useEditor, type Toast } from "./store";
import type { Project, Snapshot } from "./types";

/** A choice's option, or a row the plugin found in the project; plain text, shown as it is. */
export type PluginRow = { value: string; label: string };

/** Options listed in the manifest, or "dynamic": the rows the plugin's preview returns. */
type Options = PluginRow[] | "dynamic";

export type PluginInput = { id: string; label: string; when?: { input: string; equals: string | boolean | (string | boolean)[] } } & (
  | { type: "number"; min: number; max: number; step: number; default: number; unit?: string }
  | { type: "toggle"; default: boolean }
  | { type: "choice"; options: Options; default: string }
  | { type: "multi"; options: Options; default: string[] }
  | { type: "text"; default: string }
  | { type: "color"; default: string }
);

export type PluginValue = number | boolean | string | string[];
export type PluginValues = Record<string, PluginValue>;

export interface PluginAction {
  id: string;
  label: string;
}

/** A setting of an effect: a number, a toggle or a colour, as plugin inputs. */
export type EffectParam = Extract<PluginInput, { type: "number" | "toggle" | "color" }>;

/** A look a plugin adds, drawn over a clip's picture; `error` says why it does not draw on this computer. */
export interface PluginEffect {
  plugin: string;
  pluginName: string;
  version: string;
  id: string;
  name: string;
  params: EffectParam[];
  error: string | null;
}

/** A plugin in Nuzky's plugins folder; one that cannot load has only its id, a name and `error`. */
export interface Plugin {
  id: string;
  name: string;
  description: string;
  /** Its buttons, the first the primary one: "Run" unless the plugin names its own. */
  actions: PluginAction[];
  needs: string[];
  inputs: PluginInput[];
  /** Headed groups of inputs; the inputs in none come first. */
  sections: { label: string; inputs: string[] }[];
  error: string | null;
  /** Whether it has a script to run; a plugin may only add effects. */
  runs: boolean;
  effects: PluginEffect[];
}

/** What the plugin would do to the project as it is: `project` after its edits, at `revision`. */
export interface PluginPreview {
  key: string;
  revision: number;
  project: Project | null;
  /** Nuzky's tools the plugin would run after its edits, which the preview names without running. */
  tools: string[];
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
  /** The rows each plugin's last preview found for its dynamic inputs, by plugin and input. */
  rows: Record<string, Record<string, PluginRow[]>>;
  running: string | null;
  /** The action of the run going on. */
  pressed: string | null;
  /** The automatic runs going on now; an export waits for its own. */
  pass: Trigger | null;
  /** An export waits for plugins: its own, or a pass still running before it. */
  exportWaits: boolean;
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

export const usePlugins = create<PluginsState>(() => ({
  list: null,
  open: null,
  ...savedSettings(),
  preview: null,
  rows: {},
  running: null,
  pressed: null,
  pass: null,
  exportWaits: false,
  step: null,
  report: null,
}));

usePlugins.subscribe((s, prev) => {
  if (s.values !== prev.values || s.auto !== prev.auto) localStorage.setItem(SETTINGS_KEY, JSON.stringify({ values: s.values, auto: s.auto }));
});

export function setAuto(id: string, trigger: Trigger, on: boolean) {
  usePlugins.setState((s) => ({ auto: { ...s.auto, [trigger]: [...s.auto[trigger].filter((i) => i !== id), ...(on ? [id] : [])] } }));
}

/** `table[key]` when the table has it itself: a plugin or input may be called `constructor`, which every object has. */
export const own = <T>(table: Record<string, T> | null | undefined, key: string): T | undefined =>
  table && Object.hasOwn(table, key) ? table[key] : undefined;

/**
 * The values last set that the plugin still takes; a setting the AI removed or retyped since goes back to its default,
 * and a row of a dynamic input that its last preview on this project did not find, or before any, is no longer chosen.
 */
export function valuesFor(p: Plugin, set: PluginValues | undefined, rows: Record<string, PluginRow[]> | undefined): PluginValues {
  const offered = (input: PluginInput & { options: Options }, v: string) => (input.options === "dynamic" ? !!rows && (own(rows, input.id) ?? []).some((r) => r.value === v) : input.options.some((o) => o.value === v));
  const value = (input: PluginInput, v: PluginValue | undefined): PluginValue => {
    if (input.type === "multi") return Array.isArray(v) ? v.filter((one) => typeof one === "string" && offered(input, one)) : input.default;
    const fits =
      typeof v === typeof input.default &&
      (input.type !== "choice" || (input.options === "dynamic" && v === "") || offered(input, v as string)) &&
      (input.type !== "number" || ((v as number) >= input.min && (v as number) <= input.max));
    return fits ? v! : input.default;
  };
  return Object.fromEntries(p.inputs.map((input) => [input.id, value(input, own(set, input.id))]));
}

export const valuesOf = (p: Plugin): PluginValues => valuesFor(p, own(usePlugins.getState().values, p.id), own(usePlugins.getState().rows, p.id));

const dynamic = (input: PluginInput) => "options" in input && input.options === "dynamic";

/** What an automatic run uses: the values last set, with every list the plugin fills at its default, as nobody chose from it in this project. */
const autoValues = (p: Plugin): PluginValues => ({ ...valuesOf(p), ...Object.fromEntries(p.inputs.filter(dynamic).map((input) => [input.id, input.default])) });

/** The inputs shown for these values: those without `when`, and those whose earlier input is shown with that value. */
export function shownInputs(p: Plugin, values: PluginValues): Set<string> {
  const shown = new Set<string>();
  for (const input of p.inputs) {
    const w = input.when;
    if (!w || (shown.has(w.input) && [w.equals].flat().includes(values[w.input] as string | boolean))) shown.add(input.id);
  }
  return shown;
}

export function setValue(id: string, input: string, value: PluginValue) {
  usePlugins.setState((s) => ({ values: { ...s.values, [id]: { ...own(s.values, id), [input]: value } } }));
}

/** A plugin as the backend lists it: the manifest of one that loads, else why it does not. */
export interface ListedPlugin {
  id: string;
  error: string | null;
  manifest: {
    name: string;
    description: string;
    version: string;
    action: string;
    actions?: PluginAction[];
    needs: string[];
    inputs: PluginInput[];
    sections?: Plugin["sections"];
    effects?: { id: string; name: string; params?: EffectParam[] }[];
  } | null;
  runs: boolean;
  effects: { id: string; status: string; error: string | null }[];
}

const listed = ({ id, error, manifest: m, runs, effects }: ListedPlugin): Plugin => {
  const name = m?.name ?? id.charAt(0).toUpperCase() + id.slice(1).replaceAll("-", " ");
  return {
    id,
    name,
    description: m?.description ?? "",
    actions: m?.actions?.length ? m.actions : [{ id: "run", label: m?.action ?? "Run" }],
    needs: m?.needs ?? [],
    inputs: m?.inputs ?? [],
    sections: m?.sections ?? [],
    error: m ? null : (error ?? "Can't load."),
    runs,
    effects: (m?.effects ?? []).map((e) => ({
      plugin: id,
      pluginName: name,
      version: m?.version ?? "",
      id: e.id,
      name: e.name,
      params: e.params ?? [],
      error: effects.find((s) => s.id === e.id)?.error ?? null,
    })),
  };
};

export async function loadPlugins() {
  try {
    const list = (await api.listPlugins()).map(listed);
    // A plugin gone from the folder loses its switches, so another saved later under its name does not run by itself;
    // nor does one that only adds effects, as it has nothing to run.
    const kept = (ids: string[]) => ids.filter((id) => list.some((p) => p.id === id && (p.runs || !!p.error)));
    usePlugins.setState((s) => {
      const auto = { import: kept(s.auto.import), export: kept(s.auto.export) };
      const same = auto.import.length === s.auto.import.length && auto.export.length === s.auto.export.length;
      return { list, open: s.open && list.some((p) => p.id === s.open) ? s.open : null, auto: same ? s.auto : auto };
    });
  } catch (e) {
    usePlugins.setState({ list: [] });
    useEditor.getState().toast({ kind: "error", text: `Couldn't read your plugins: ${errorText(e)}` });
  }
}

/** How many times the plugins' files changed while the app ran. */
const usePluginFiles = create(() => ({ changes: 0 }));

export function listenPlugins() {
  return listen("plugins-changed", () => {
    usePluginFiles.setState((s) => ({ changes: s.changes + 1 }));
    void loadPlugins();
    void loadEffectIssues();
  });
}

/** An effect on the project's clips that this computer draws without: not installed, or broken. */
export interface EffectIssue {
  plugin: string;
  effect: string;
  name: string;
  broken: string | null;
  clipIds: string[];
  message: string;
}

/** `toasted`: the toast that names each effect, by `issueKey`, so each is said once per project. */
export const useEffectIssues = create<{ issues: EffectIssue[]; epoch: string | null; toasted: Record<string, number> }>(() => ({ issues: [], epoch: null, toasted: {} }));

const issueKey = (i: EffectIssue) => `${i.broken ? "broken" : "missing"}:${i.plugin}/${i.effect}`;

/**
 * Changes when a plugin's files change or an effect of the project stops or starts drawing, so a picture drawn with
 * effects outside the preview, such as the cover, is asked for again.
 */
export function useEffectsVersion() {
  const changes = usePluginFiles((s) => s.changes);
  const issues = useEffectIssues((s) => s.issues.map(issueKey).join(","));
  return `${changes}:${issues}`;
}

/** What an effect's row in the clip inspector says about it. */
export const issueRow = (i: EffectIssue) =>
  i.broken ? `${i.name} has an error, so the clip is drawn without it.` : `${i.name} isn't installed on this computer, so the clip is drawn without it.`;

/** The warning about effects the video plays without, all missing or all broken. */
export function drawnWithout(issues: EffectIssue[]) {
  const one = issues.length === 1;
  const who = names(issues.map((i) => i.name));
  return issues[0].broken
    ? `${who} ${one ? "has an error" : "have errors"}, so the video plays without ${one ? "it" : "them"}.`
    : `${who} ${one ? "isn't" : "aren't"} installed on this computer, so the video plays and exports without ${one ? "it" : "them"}.`;
}

/** The AI panel asked to fix broken effects, with what Nuzky says about each file and line. */
export function fixEffectsWithAi(broken: EffectIssue[]) {
  const named = (id: string) => usePlugins.getState().list?.find((p) => p.id === id)?.name ?? id;
  askAi(broken.map((i) => `Fix the effect “${i.name}” of the Nuzky plugin “${named(i.plugin)}” (${i.plugin}). Nuzky says: ${i.broken}`).join("\n"));
}

function announceIssues(issues: EffectIssue[]) {
  const { toasted } = useEffectIssues.getState();
  const fresh = issues.filter((i) => !(issueKey(i) in toasted));
  if (fresh.length === 0) return;
  const said: Record<string, number> = {};
  const warn = (about: EffectIssue[], action?: Toast["action"]) => {
    if (about.length === 0) return;
    useEditor.getState().toast({ kind: "warning", text: drawnWithout(about), action });
    const id = useEditor.getState().toasts.at(-1)?.id ?? 0;
    for (const i of about) said[issueKey(i)] = id;
  };
  const broken = fresh.filter((i) => i.broken);
  warn(fresh.filter((i) => !i.broken));
  warn(broken, { label: "Fix with AI", run: () => fixEffectsWithAi(broken) });
  useEffectIssues.setState((s) => ({ toasted: { ...s.toasted, ...said } }));
}

/** An export drawn without effects says so in a toast, unless an open toast already names each of them. */
export function warnExported(text: string) {
  const { toasts, toast } = useEditor.getState();
  const { issues, toasted } = useEffectIssues.getState();
  if (issues.length > 0 && issues.every((i) => toasts.some((t) => t.id === toasted[issueKey(i)]))) return;
  toast({ kind: "warning", text });
}

let issueRequests = 0;
/** What the next answer warns about, every issue or those named, kept when a newer request overtakes the one asked. */
let toWarn: true | string[] = [];

/**
 * Asks which effects of the open project this computer draws without. `warn`: true to warn about each one not named
 * yet, or the names of those the preview now draws without.
 */
export async function loadEffectIssues(warn: boolean | string[] = false) {
  if (warn === true) toWarn = true;
  else if (warn && toWarn !== true) toWarn = [...toWarn, ...warn];
  const epoch = useEditor.getState().snap?.sessionEpoch ?? null;
  const mine = ++issueRequests;
  try {
    const issues = await api.effectIssues();
    if (mine !== issueRequests || useEditor.getState().snap?.sessionEpoch !== epoch) return;
    const wanted = toWarn;
    toWarn = [];
    useEffectIssues.setState((s) => (s.epoch === epoch ? { issues } : { issues, epoch, toasted: {} }));
    announceIssues(wanted === true ? issues : issues.filter((i) => wanted.includes(i.name)));
  } catch {
    // Without an answer the inspector shows no warnings; the preview still says what it draws without.
  }
}

/** The preview says which effects its frame was drawn without whenever that changes. */
export function listenEffects() {
  return listen<string[]>("effects-skipped", (e) => {
    if (e.payload.length > 0) void loadEffectIssues(e.payload);
  });
}

const effectKey = (project: Project | undefined) =>
  JSON.stringify(project?.tracks.flatMap((t) => t.clips.flatMap((c) => (c.effects ?? []).map((e) => [c.id, e.plugin, e.effect]))) ?? []);

// A project opened or its effects changed: what it draws without is asked again, and said once when it opens.
useEditor.subscribe((s, prev) => {
  if (!s.snap) return;
  const opened = s.snap.sessionEpoch !== prev.snap?.sessionEpoch;
  if (opened || effectKey(s.snap.project) !== effectKey(prev.snap?.project)) void loadEffectIssues(opened);
});

/** The plugin's effects that have an error. */
export const brokenEffects = (p: Plugin) => p.effects.filter((e) => e.error);

/** The installed effects, from every plugin that loads. */
export const installedEffects = (list: Plugin[] | null) => (list ?? []).flatMap((p) => p.effects);

// A preview of a plugin no longer shown would only keep a stuck script running.
usePlugins.subscribe((s, prev) => {
  if (prev.open && s.open !== prev.open) void api.cancelPluginPreview().catch(() => {});
});

// The inspector shows a plugin only while the Plugins tab is open, and never one of another project.
useEditor.subscribe((s, prev) => {
  if (usePlugins.getState().open && (s.panelTab !== "plugins" || s.snap?.sessionEpoch !== prev.snap?.sessionEpoch)) usePlugins.setState({ open: null, preview: null });
});

// The rows a plugin found, and the one chosen from them, belong to the project it looked at.
useEditor.subscribe((s, prev) => {
  if (s.snap?.sessionEpoch === prev.snap?.sessionEpoch) return;
  usePlugins.setState(({ list, values }) => ({
    rows: {},
    values: Object.fromEntries(
      Object.entries(values).map(([id, set]) => {
        const lists = list?.find((p) => p.id === id)?.inputs.filter(dynamic) ?? [];
        return [id, lists.length ? { ...set, ...Object.fromEntries(lists.map((input) => [input.id, input.default])) } : set];
      }),
    ),
  }));
});

/** Shows the plugin in the inspector, from the Plugins tab or the launcher. */
export function openPlugin(id: string | null) {
  usePlugins.setState({ open: id, preview: null });
  if (id) useEditor.setState({ panelTab: "plugins", cut: null });
}

/** Where a plugin's effects are added. */
export const ADD_EFFECTS = "Select a video or image, then add them in Adjust → Effects.";

/** Why the plugin cannot run on this project now, or null. */
export function pluginBlocked(p: Plugin, hasWords: boolean): string | null {
  if (p.error) return "This plugin can't load. Ask the AI to fix it.";
  if (!p.runs) return `This plugin only adds effects. ${ADD_EFFECTS}`;
  if (p.needs.includes("words") && !hasWords) return "Needs to know what is said. Transcribe first.";
  return null;
}

/** The text after "CODE: ", for showing an error the plugin caused. */
export const plainDetail = (text: string) => text.replace(/^[A-Z][A-Z_]+: ?/, "");

/** Whether the AI can fix the error by changing the plugin; a plugin made for a newer Nuzky needs an update instead. */
export const fixable = (error: string) => !error.startsWith("PLUGIN_API_NEWER");

const TOOL_WORDS: Record<string, string> = { build_captions: "captions", apply_zooms: "zooms", apply_motion: "motion" };

/** What the tools a plugin runs after its edits add, such as "Then adds captions and zooms"; null for none. */
export function toolsLine(tools: string[], after: boolean): string | null {
  const words = [...new Set(tools.map((t) => TOOL_WORDS[t] ?? t))];
  return words.length ? `${after ? "Then adds" : "Adds"} ${names(words)}` : null;
}

let previews = 0;

export async function previewPlugin(p: Plugin, values: PluginValues, key: string) {
  const { snap, selection, timeUs } = useEditor.getState();
  if (!snap) return;
  const mine = ++previews;
  let preview: PluginPreview;
  let rows: Record<string, PluginRow[]> | null = null;
  try {
    const answer = await api.previewPlugin(p.id, values, selection, timeUs);
    rows = answer.items;
    preview = { key, revision: snap.revision, project: answer.project, tools: answer.tools, error: answer.error && plainDetail(answer.error), waiting: false };
  } catch (e) {
    const text = errorText(e);
    // A newer request cancelled it.
    if (text.startsWith("CANCELLED")) return;
    preview = { key, revision: snap.revision, project: null, tools: [], error: plainDetail(text), waiting: text.startsWith("AUDIO_NOT_READY") };
  }
  // An older answer arriving late, a closed plugin or a run that started meanwhile keeps what is shown.
  if (mine === previews && usePlugins.getState().open === p.id && !usePlugins.getState().running)
    usePlugins.setState((s) => ({ preview, rows: rows ? { ...s.rows, [p.id]: rows } : s.rows }));
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

/**
 * Runs its `action`, the primary one when null, in a run of its own: the AI lock with Stop while it runs, and one undo
 * step. `epoch` keeps it to that project.
 */
/** `still`, for an automatic run, says right before the run starts whether its switch is still on; null when it is not. */
async function execute(p: Plugin, values: PluginValues, selection: string[], timeUs: number, action: string | null = null, epoch?: string, still?: () => boolean): Promise<Ran | null> {
  // The last preview stays, dimmed, so the panel keeps its height while the run goes on.
  usePlugins.setState({ running: p.id, pressed: action ?? p.actions[0].id });
  const out: { failure?: string; message?: string | null; from?: number; skipped?: boolean } = {};
  const snap = await enqueue(async (current) => {
    if (still && !still()) {
      out.skipped = true;
      return null;
    }
    out.from = useEditor.getState().snap?.revision;
    try {
      const done = await api.runPlugin(p.id, values, selection, timeUs, epoch ?? current, action);
      out.message = done.message;
      return done.snapshot;
    } catch (e) {
      out.failure = errorText(e);
      return null;
    }
  });
  usePlugins.setState({ running: null, pressed: null });
  if (out.skipped) return null;
  if (out.failure !== undefined)
    return out.failure.startsWith("CANCELLED") || out.failure.startsWith("RUN_STOPPED") ? { plugin: p, did: "stopped" } : { plugin: p, did: "failed", error: out.failure, loaded: true };
  const message = out.message ?? null;
  return snap ? { plugin: p, did: "changed", message, snap, from: out.from } : { plugin: p, did: "nothing", message };
}

/**
 * The run a person started from one of the plugin's buttons in the inspector, `action` or the primary one, on the
 * selection and at the playhead, and a toast that says what it did.
 */
export async function runPlugin(p: Plugin, action: string | null = null) {
  if (aiLocked() || usePlugins.getState().running) return;
  const { selection, timeUs } = useEditor.getState();
  const ran = await execute(p, valuesOf(p), selection, timeUs, action);
  if (!ran) return;
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

interface Pass {
  trigger: Trigger;
  /** The project the pass is for; it ends when another opens. */
  epoch: string | undefined;
  cancelled: boolean;
  /** Ends the export's wait for passes before it at once. */
  stop?: () => void;
}

/** One pass at a time, so an export starts from what the plugins after an import finished, never from half of it. */
let passes: Promise<unknown> = Promise.resolve();
let exporting: Pass | null = null;
/** A pass after imports that has not started; imports finishing meanwhile join it instead of running the plugins again. */
let importWaiting: { epoch: string | undefined; run: Promise<PassReport | null> } | null = null;

/** Cancel export: stops the plugin running before the export, skips the rest, and the export never starts. */
export function cancelExport() {
  if (!exporting) return;
  exporting.cancelled = true;
  exporting.stop?.();
  if (usePlugins.getState().pass === "export" && usePlugins.getState().running) void stopAiRun();
}

/** Resolves once no import is adding media, as a plugin's run would keep an import that is still probing from adding. */
const importsDone = () =>
  new Promise<void>((resolve) => {
    if (useEditor.getState().importing.length === 0) return resolve();
    const stop = useEditor.subscribe((s) => {
      if (s.importing.length > 0) return;
      stop();
      resolve();
    });
  });

/**
 * Runs the plugins the user turned on for `trigger`, one after another by name, each its own undo step, on the whole
 * project from its start with the values last set. None of them blocks the import or export. Before an export the
 * dialog shows how each went while it is open, and the toasts wait until it closes.
 */
export function runAutomatically(trigger: Trigger, pass: Pass = { trigger, epoch: useEditor.getState().snap?.sessionEpoch, cancelled: false }): Promise<PassReport | null> {
  if (usePlugins.getState().auto[trigger].length === 0) return Promise.resolve(null);
  const waiting = importWaiting;
  if (trigger === "import" && waiting && waiting.epoch === pass.epoch) return waiting.run;
  const run: Promise<PassReport | null> = passes.then(async () => {
    if (trigger === "import") {
      await importsDone();
      if (importWaiting?.run === run) importWaiting = null;
    }
    return runPass(pass);
  });
  if (trigger === "import") importWaiting = { epoch: pass.epoch, run };
  passes = run.catch(() => null);
  return run;
}

async function runPass(pass: Pass): Promise<PassReport> {
  const { trigger } = pass;
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
      if (pass.cancelled) break;
      // Turned off while an earlier one ran, as installing a new version from the gallery does.
      const on = () => usePlugins.getState().auto[trigger].includes(p.id);
      if (!on()) continue;
      const { aiRun, snap } = useEditor.getState();
      // A plugin run that has ended may not have said so yet; the backend refuses a run while another is open.
      const agent = !!aiRun && !aiRun.startsWith(PLUGIN_RUN);
      const elsewhere = snap?.sessionEpoch !== pass.epoch;
      if (agent || elsewhere || usePlugins.getState().running) {
        const reason = elsewhere ? "another project is open" : agent ? "the AI is editing" : "another plugin is running";
        publish({ skipped: { names: chosen.slice(i).map((c) => c.name), all: i === 0, reason } });
        break;
      }
      usePlugins.setState({ step: { index: i + 1, total: chosen.length } });
      const ran = p.error ? { plugin: p, did: "failed" as const, error: p.error, loaded: false } : await execute(p, autoValues(p), [], 0, null, pass.epoch, on);
      if (!ran) continue;
      publish({ results: [...report.results, ran] });
    }
  } finally {
    usePlugins.setState({ pass: null, step: null });
  }
  publish({ cancelled: pass.cancelled, done: true });
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

/**
 * The project as the plugins for export last left it, with which plugins were on and their settings; exporting it again
 * unchanged, as Replace and Retry do, runs none.
 */
let exportReady: string | null = null;

/** Which plugins are on for export, with their settings. */
const exportConfig = () => {
  const { auto, list } = usePlugins.getState();
  return auto.export.map((id) => {
    const p = list?.find((p) => p.id === id);
    return [id, p && valuesOf(p)];
  });
};

const exportKey = (config: unknown) => {
  const { snap } = useEditor.getState();
  return snap && JSON.stringify([snap.sessionEpoch, snap.revision, config]);
};

/** Runs the plugins the user turned on for export, and resolves whether the export may start. */
export async function runBeforeExport() {
  const pass: Pass = { trigger: "export", epoch: useEditor.getState().snap?.sessionEpoch, cancelled: false };
  const stopped = new Promise<null>((resolve) => (pass.stop = () => resolve(null)));
  exporting = pass;
  usePlugins.setState({ exportWaits: true });
  const cancelled = () => {
    if (!usePlugins.getState().report?.cancelled) usePlugins.setState({ report: { trigger: "export", results: [], skipped: null, cancelled: true, done: true } });
    return false;
  };
  try {
    // A pass after an import finishes first, so the export has all its changes.
    await Promise.race([passes, stopped]);
    if (pass.cancelled) return cancelled();
    const config = exportConfig();
    if (exportReady !== null && exportReady === exportKey(config)) return true;
    const report = await Promise.race([runAutomatically("export", pass), stopped]);
    if (pass.cancelled) return cancelled();
    // Plugins the AI kept from running get their turn at the next export; one turned on meanwhile, at the next.
    exportReady = report?.skipped ? null : exportKey(config);
    return true;
  } finally {
    if (exporting === pass) {
      exporting = null;
      usePlugins.setState({ exportWaits: false });
    }
  }
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

/** Make an effect: the AI panel with the request started, for the person to finish. */
export const makeEffectWithAi = () => openAi("Make a Nuzky plugin with an effect that ");

/** One of the suggestions in the empty Plugins tab, sent as it is. */
export function askAi(text: string) {
  if (sendBlocked()) return openAi(text);
  openAi();
  void send(text, "project");
}

export const changeWithAi = (p: Plugin) => openAi(`Change the Nuzky plugin “${p.name}” (${p.id}) so that `);

export const fixWithAi = (p: Pick<Plugin, "id" | "name">, error: string) => askAi(`Fix the Nuzky plugin “${p.name}” (${p.id}). Nuzky says: ${error}`);

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

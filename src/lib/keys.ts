import { create } from "zustand";

/**
 * The editor's keys the user can change. A key is stored as it is shown ("Ctrl+Shift+Z", "Shift+←", "Space"), so
 * tooltips draw it as a key cap. Ctrl also means Cmd, as everywhere in Nuzky.
 */
export const ACTIONS = [
  { id: "playPause", group: "Playback", label: "Play or pause" },
  { id: "framePrev", group: "Playback", label: "Previous frame" },
  { id: "frameNext", group: "Playback", label: "Next frame" },
  { id: "secondBack", group: "Playback", label: "Back 1 second" },
  { id: "secondForward", group: "Playback", label: "Forward 1 second" },
  { id: "cutPrev", group: "Playback", label: "Previous cut" },
  { id: "cutNext", group: "Playback", label: "Next cut" },
  { id: "goStart", group: "Playback", label: "Go to start" },
  { id: "goEnd", group: "Playback", label: "Go to end" },
  { id: "mute", group: "Playback", label: "Mute the preview" },
  { id: "fullScreen", group: "Playback", label: "Full screen preview" },
  { id: "split", group: "Edit", label: "Split at the playhead" },
  { id: "deleteLeft", group: "Edit", label: "Delete left of the playhead" },
  { id: "deleteRight", group: "Edit", label: "Delete right of the playhead" },
  { id: "freeze", group: "Edit", label: "Freeze frame" },
  { id: "delete", group: "Edit", label: "Delete selection" },
  { id: "duplicate", group: "Edit", label: "Duplicate" },
  { id: "copy", group: "Edit", label: "Copy" },
  { id: "cut", group: "Edit", label: "Cut" },
  { id: "paste", group: "Edit", label: "Paste" },
  { id: "pasteAttributes", group: "Edit", label: "Paste attributes" },
  { id: "undo", group: "Edit", label: "Undo" },
  { id: "redo", group: "Edit", label: "Redo" },
  { id: "zoomIn", group: "Timeline", label: "Zoom in" },
  { id: "zoomOut", group: "Timeline", label: "Zoom out" },
  { id: "import", group: "App", label: "Import media" },
  { id: "export", group: "App", label: "Export" },
  { id: "aiPanel", group: "App", label: "Open or close the AI panel" },
  { id: "projects", group: "App", label: "Projects" },
] as const;

export type ActionId = (typeof ACTIONS)[number]["id"];
export type Keymap = Record<ActionId, string[]>;

const NUZKY: Keymap = {
  playPause: ["Space"],
  framePrev: ["←"],
  frameNext: ["→"],
  secondBack: ["Shift+←"],
  secondForward: ["Shift+→"],
  cutPrev: ["↑"],
  cutNext: ["↓"],
  goStart: ["Home"],
  goEnd: ["End"],
  mute: ["M"],
  fullScreen: ["F"],
  split: ["S"],
  deleteLeft: ["Q"],
  deleteRight: ["W"],
  freeze: ["Shift+R"],
  delete: ["Delete", "Backspace"],
  duplicate: ["Ctrl+D"],
  copy: ["Ctrl+C"],
  cut: ["Ctrl+X"],
  paste: ["Ctrl+V"],
  pasteAttributes: ["Ctrl+Alt+V"],
  undo: ["Ctrl+Z"],
  redo: ["Ctrl+Shift+Z", "Ctrl+Y"],
  zoomIn: ["+", "="],
  zoomOut: ["-"],
  import: ["Ctrl+I"],
  export: ["Ctrl+E"],
  aiPanel: ["Ctrl+J"],
  projects: ["Ctrl+K"],
};

const COMMON = {
  playPause: ["Space"],
  framePrev: ["←"],
  frameNext: ["→"],
  secondBack: ["Shift+←"],
  secondForward: ["Shift+→"],
  cutPrev: ["↑"],
  cutNext: ["↓"],
  goStart: ["Home"],
  goEnd: ["End"],
  // None of the other editors has a key for these; M is a marker there, which Nuzky does not have.
  mute: ["M"],
  aiPanel: ["Ctrl+J"],
  delete: ["Delete", "Backspace"],
  copy: ["Ctrl+C"],
  cut: ["Ctrl+X"],
  paste: ["Ctrl+V"],
  undo: ["Ctrl+Z"],
  redo: ["Ctrl+Shift+Z"],
  import: ["Ctrl+I"],
};

/**
 * Each program's default keys for what Nuzky does. An action the program has no key for gets none where Nuzky's key
 * does another edit there (Shift+R, Q, W, Ctrl+D), so a habit never changes the video unexpectedly.
 */
export const PRESETS = {
  nuzky: { name: "Nuzky", keys: NUZKY },
  // CapCut's A, B and T switch its timeline tools; Nuzky's timeline always selects and trims, so they stay free.
  capcut: {
    name: "CapCut",
    keys: { ...COMMON, fullScreen: ["Ctrl+Shift+F"], split: ["Ctrl+B"], deleteLeft: ["Q"], deleteRight: ["W"], freeze: [], duplicate: ["Ctrl+D"], pasteAttributes: [], zoomIn: ["Ctrl+=", "Ctrl++"], zoomOut: ["Ctrl+-"], export: ["Ctrl+E"], projects: ["Ctrl+K"] },
  },
  finalCut: {
    name: "Final Cut Pro",
    keys: { ...COMMON, cutPrev: ["↑", ";"], cutNext: ["↓", "'"], fullScreen: ["Ctrl+Shift+F"], split: ["Ctrl+B"], deleteLeft: ["Alt+["], deleteRight: ["Alt+]"], freeze: ["Alt+F"], duplicate: ["Ctrl+D"], pasteAttributes: ["Ctrl+Shift+V"], zoomIn: ["Ctrl+=", "Ctrl++"], zoomOut: ["Ctrl+-"], export: ["Ctrl+E"], projects: ["Ctrl+K"] },
  },
  resolve: {
    name: "DaVinci Resolve",
    keys: { ...COMMON, fullScreen: ["Ctrl+F"], split: ["Ctrl+\\", "Ctrl+B"], deleteLeft: ["Ctrl+Shift+["], deleteRight: ["Ctrl+Shift+]"], freeze: ["Shift+R"], duplicate: [], pasteAttributes: ["Alt+V"], zoomIn: ["Ctrl+=", "Ctrl++"], zoomOut: ["Ctrl+-"], export: ["Ctrl+E"], projects: ["Shift+1"] },
  },
  premiere: {
    name: "Premiere Pro",
    keys: { ...COMMON, fullScreen: ["Ctrl+`"], split: ["Ctrl+K"], deleteLeft: ["Q"], deleteRight: ["W"], freeze: [], duplicate: [], pasteAttributes: ["Ctrl+Alt+V"], zoomIn: ["=", "+"], zoomOut: ["-"], export: ["Ctrl+M"], projects: ["Ctrl+O"] },
  },
} satisfies Record<string, { name: string; keys: Keymap }>;

export type PresetId = keyof typeof PRESETS;

/** Keys the editor or the system already gives a job that does not move: dialogs, focus, clips and windows. */
const RESERVED = /^(Esc|Tab|Shift\+Tab|Enter|Shift\+Enter|F2|F8|Shift\+F10|Ctrl\+,|Ctrl\+[WQ]|Ctrl\+(Shift\+)?[←→↑↓]|Alt\+(Shift\+)?[←→↑↓])$/;

export const reserved = (chord: string) => RESERVED.test(chord);

const NAMED: Record<string, string> = {
  " ": "Space",
  ArrowLeft: "←",
  ArrowRight: "→",
  ArrowUp: "↑",
  ArrowDown: "↓",
  Home: "Home",
  End: "End",
  PageUp: "PgUp",
  PageDown: "PgDn",
  Delete: "Delete",
  Backspace: "Backspace",
  Enter: "Enter",
  Escape: "Esc",
  Tab: "Tab",
};

type Press = Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">;

/** The punctuation keys by their place, named as on a US keyboard. */
const PLACES: Record<string, string> = { BracketLeft: "[", BracketRight: "]", Semicolon: ";", Quote: "'", Comma: ",", Period: ".", Slash: "/", Backslash: "\\", Minus: "-", Equal: "=", Backquote: "`" };
const ASCII = /^[!-~]$/;
const placeOf = (code: string) => /^Digit(\d)$/.exec(code)?.[1] ?? PLACES[code] ?? null;
const join = (e: Press, key: string, shift: boolean) => [(e.ctrlKey || e.metaKey) && "Ctrl", e.altKey && "Alt", shift && "Shift", key].filter(Boolean).join("+");

/** The keys pressed as they are stored when recorded, or null for a modifier alone or a key Nuzky cannot name. */
export function chordOf(e: Press): string | null {
  const named = NAMED[e.key] ?? (/^F\d{1,2}$/.test(e.key) ? e.key : null);
  if (named) return join(e, named, e.shiftKey);
  if (e.key.length !== 1) return null;
  if (/^[a-z0-9]$/i.test(e.key)) return join(e, e.key.toUpperCase(), e.shiftKey);
  const letter = /^Key([A-Z])$/.exec(e.code)?.[1];
  const place = letter ?? placeOf(e.code);
  // Option types another character on macOS, and a Czech keyboard types ě on the 2; there the key's place names it.
  if (place && (e.altKey || !ASCII.test(e.key))) return join(e, place, e.shiftKey);
  // Shift is how a symbol such as + is typed on many keyboards, so it is not part of the chord.
  return ASCII.test(e.key) ? join(e, e.key, false) : null;
}

/**
 * The chord as typed, then a digit or punctuation key by its place with Shift, as the other editors' sets name keys
 * such as Ctrl+Shift+[ and Shift+1. Letters are left out, as their place differs between QWERTY and QWERTZ.
 */
export function chordsOf(e: Press): string[] {
  const place = placeOf(e.code);
  const chords = [chordOf(e), place && join(e, place, e.shiftKey)];
  return chords.filter((c, i): c is string => !!c && chords.indexOf(c) === i);
}

export type Saved = { preset: PresetId; changed: Partial<Keymap> };
const KEY = "nuzky.keys";

function load(): Saved {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "null");
    const preset: PresetId = v?.preset in PRESETS ? v.preset : "nuzky";
    const changed: Partial<Keymap> = {};
    for (const { id } of ACTIONS) {
      const keys = v?.changed?.[id];
      if (Array.isArray(keys) && keys.every((k) => typeof k === "string" && k.length < 40)) changed[id] = keys;
    }
    return { preset, changed };
  } catch {
    return { preset: "nuzky", changed: {} };
  }
}

export const useKeys = create<Saved>(() => load());
useKeys.subscribe((s) => localStorage.setItem(KEY, JSON.stringify(s)));

export const keysOf = (id: ActionId, s: Saved = useKeys.getState()) => s.changed[id] ?? PRESETS[s.preset].keys[id];

/** The action the keys pressed run now. */
export function actionOf(chords: string[]): ActionId | null {
  const s = useKeys.getState();
  for (const chord of chords) {
    const action = ACTIONS.find((a) => keysOf(a.id, s).includes(chord));
    if (action) return action.id;
  }
  return null;
}

/** "Split at playhead (S)" with the action's first key, or the label alone when it has none. */
export function withKey(label: string, id: ActionId, s: Saved = useKeys.getState()) {
  const key = keysOf(id, s)[0];
  return key ? `${label} (${key})` : label;
}

/** Re-renders on a change of keys and returns withKey and the first key of an action. */
export function useKeyLabels() {
  const s = useKeys();
  return { withKey: (label: string, id: ActionId) => withKey(label, id, s), key: (id: ActionId) => keysOf(id, s)[0] };
}

const ARIA: Record<string, string> = { Ctrl: "Control", "←": "ArrowLeft", "→": "ArrowRight", "↑": "ArrowUp", "↓": "ArrowDown", PgUp: "PageUp", PgDn: "PageDown", Esc: "Escape", "+": "Plus" };

/** A chord as `aria-keyshortcuts` names it ("Control+I"). */
export const ariaKeys = (chord: string | undefined) => chord?.split(/\+(?!$)/).map((k) => ARIA[k] ?? k).join("+");

/** Gives `chord` to `id` alone; any other action that had it loses it. */
export function setKey(id: ActionId, chord: string) {
  const s = useKeys.getState();
  const changed: Partial<Keymap> = { ...s.changed, [id]: [chord] };
  for (const a of ACTIONS) {
    const keys = keysOf(a.id, s);
    if (a.id !== id && keys.includes(chord)) changed[a.id] = keys.filter((k) => k !== chord);
  }
  useKeys.setState({ changed: tidy(s.preset, changed) });
}

/** Back to the preset's keys; another action that took one of them meanwhile keeps it. */
export function resetKey(id: ActionId) {
  const s = useKeys.getState();
  const taken = new Set(ACTIONS.filter((a) => a.id !== id).flatMap((a) => keysOf(a.id, s)));
  const { [id]: _, ...rest } = s.changed;
  const keys = PRESETS[s.preset].keys[id].filter((k) => !taken.has(k));
  useKeys.setState({ changed: tidy(s.preset, keys.length === PRESETS[s.preset].keys[id].length ? rest : { ...rest, [id]: keys }) });
}

export const choosePreset = (preset: PresetId) => useKeys.setState({ preset, changed: {} });

/** Drops the changes that are the preset's own keys again. */
function tidy(preset: PresetId, changed: Partial<Keymap>): Partial<Keymap> {
  const same = (a: string[], b: string[]) => a.length === b.length && a.every((k, i) => k === b[i]);
  return Object.fromEntries(Object.entries(changed).filter(([id, keys]) => !same(keys, PRESETS[preset].keys[id as ActionId]))) as Partial<Keymap>;
}


/**
 * Takes the keys a file from another editor gives Nuzky's actions, on top of that editor's set. A key the file gives
 * two actions stays with the first; the set's own key for an action the file leaves out yields to the file's keys.
 * Returns how many actions got keys and the actions left without one.
 */
export function takeImported(preset: PresetId, imported: Partial<Record<string, string[]>>) {
  const taken = new Set<string>();
  const changed: Partial<Keymap> = {};
  for (const { id } of ACTIONS) {
    const keys = imported[id];
    if (!keys) continue;
    changed[id] = keys.filter((k) => !reserved(k) && !taken.has(k));
    changed[id].forEach((k) => taken.add(k));
  }
  for (const { id } of ACTIONS) {
    const own = PRESETS[preset].keys[id];
    if (!(id in changed) && own.some((k) => taken.has(k))) changed[id] = own.filter((k) => !taken.has(k));
  }
  useKeys.setState({ preset, changed: tidy(preset, changed) });
  return {
    count: ACTIONS.filter((a) => changed[a.id]?.length && a.id in imported).length,
    lost: ACTIONS.filter((a) => !(a.id in imported) && PRESETS[preset].keys[a.id].length > 0 && keysOf(a.id).length === 0).map((a) => a.label),
  };
}

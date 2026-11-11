import {
  ArrowLeftToLine,
  ArrowRightToLine,
  AudioLines,
  Blend,
  Captions,
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
  ClipboardCopy,
  ClipboardPaste,
  Copy,
  Download,
  Film,
  History,
  Image,
  Keyboard,
  Link2,
  Maximize,
  Mic,
  Music,
  Paintbrush,
  Palette,
  PanelLeftClose,
  PanelRightClose,
  Play,
  Puzzle,
  RectangleHorizontal,
  Redo2,
  RotateCwSquare,
  Scissors,
  ScissorsLineDashed,
  ScrollText,
  Settings,
  SkipBack,
  SkipForward,
  SlidersHorizontal,
  Snowflake,
  Sparkles,
  SquareSplitHorizontal,
  Trash2,
  Type,
  Undo2,
  Unlink,
  Upload,
  VolumeX,
  ZoomIn,
  ZoomOut,
  type LucideIcon,
} from "lucide-react";
import { RESTS_IN_COVER, RESTS_IN_FULL, WORKS_IN_REELS, runAction } from "./actions";
import { openCover, useCover } from "./cover";
import { ACTIONS, type ActionId } from "./keys";
import { fold } from "./library";
import { usePreview } from "./preview";
import { useReels } from "./reels";
import { useSource } from "./source";
import { openSettings } from "./settings";
import { useSounds } from "./sounds";
import { useStickers } from "./stickers";
import { AI_EDITING, freezeBlocker, nothingCopied, pasteAttributesBlocker, projectDuration, splitBlocker, useEditor, type PanelTab } from "./store";
import { useVoiceover } from "./voiceover";
import { openYoutube } from "./youtube";

/** An editor action the launcher finds by name: it runs, or the keyboard goes to the control that does it. */
export type Command = {
  id: string;
  label: string;
  /** Its group or where it lives, shown under the name. */
  group: string;
  icon: LucideIcon;
  /** More words it is found by, in English and Czech. */
  terms: string;
  /** The editor's key action, whose keys the row shows. */
  key?: ActionId;
  /** Why it cannot run now, or null. */
  why: () => string | null;
  run: () => void;
};

const FULL = "Leave the full screen preview first.";
const sentence = (text: string) => `${text[0].toUpperCase()}${text.slice(1)}.`;
/** Changes the project, so the AI's lock holds it back. */
const EDITS = new Set<ActionId>(["split", "deleteLeft", "deleteRight", "freeze", "delete", "duplicate", "cut", "paste", "pasteAttributes", "undo", "redo", "import"]);

/** Why a key's action does nothing here and now, as what to do next; null when it would act. */
function whyNot(action: ActionId): string | null {
  const s = useEditor.getState();
  if (!s.snap) return "Open a project first.";
  const { project } = s.snap;
  if (useCover.getState().open && RESTS_IN_COVER.has(action)) return "Close the cover editor first.";
  if (useReels.getState().open && !WORKS_IN_REELS.has(action)) return "Leave reels first.";
  if (usePreview.getState().full && RESTS_IN_FULL.has(action)) return FULL;
  if (EDITS.has(action) && s.aiRun && s.lockShown) return AI_EDITING;
  const t = Math.round(s.timeUs);
  switch (action) {
    case "split":
    case "deleteLeft":
    case "deleteRight": {
      const blocker = splitBlocker(project, s.selection, t);
      return blocker && sentence(blocker);
    }
    case "freeze": {
      const blocker = freezeBlocker(project, s.selection, t);
      return blocker && sentence(blocker);
    }
    case "delete":
      return s.selection.length || s.cut ? null : "Select a clip first.";
    case "duplicate":
    case "copy":
    case "cut":
      return s.selection.length ? null : "Select a clip first.";
    case "paste":
      return s.clipboard ? null : nothingCopied();
    case "pasteAttributes":
      return pasteAttributesBlocker(project, s.selection, s.clipboard);
    case "export": {
      const running = s.exportJobId && s.jobs[s.exportJobId]?.status === "running";
      return running || projectDuration(project) > 0 ? null : "Add a video or picture first.";
    }
    case "undo":
      return s.snap.canUndo ? null : "Nothing to undo.";
    case "redo":
      return s.snap.canRedo ? null : "Nothing to redo.";
  }
  return null;
}

/** Each key's action with where its button lives, its icon, the words it is found by, and a name of its own where the key's is too short to tell. */
const KEYED: Partial<Record<ActionId, { group: string; icon: LucideIcon; terms: string; label?: string }>> = {
  playPause: { group: "Preview", icon: Play, terms: "start stop přehrát pustit zastavit pauza" },
  framePrev: { group: "Preview", icon: ChevronLeft, terms: "step back předchozí snímek políčko zpět" },
  frameNext: { group: "Preview", icon: ChevronRight, terms: "step forward další snímek políčko dopředu" },
  secondBack: { group: "Preview", icon: ChevronsLeft, terms: "rewind zpět sekunda přetočit" },
  secondForward: { group: "Preview", icon: ChevronsRight, terms: "dopředu sekunda přetočit" },
  cutPrev: { group: "Timeline", icon: SkipBack, terms: "edge clip předchozí střih hrana klip" },
  cutNext: { group: "Timeline", icon: SkipForward, terms: "edge clip další střih hrana klip" },
  goStart: { group: "Preview", icon: ArrowLeftToLine, terms: "beginning playhead na začátek" },
  goEnd: { group: "Preview", icon: ArrowRightToLine, terms: "playhead na konec" },
  mute: { group: "Preview", icon: VolumeX, terms: "sound volume unmute ztlumit zvuk hlasitost" },
  fullScreen: { group: "Preview", icon: Maximize, terms: "fullscreen celá obrazovka přes celou obrazovku" },
  split: { group: "Timeline", icon: Scissors, terms: "cut razor blade rozdělit rozstřihnout střih nůžky" },
  deleteLeft: { group: "Timeline", icon: PanelLeftClose, terms: "trim start smazat vlevo oříznout začátek ustřihnout" },
  deleteRight: { group: "Timeline", icon: PanelRightClose, terms: "trim end smazat vpravo oříznout konec ustřihnout" },
  freeze: { group: "Timeline", icon: Snowflake, terms: "still hold zmrazit snímek zastavit obraz" },
  delete: { group: "Timeline", icon: Trash2, terms: "remove clip smazat odstranit vymazat klip" },
  duplicate: { group: "Timeline", icon: Copy, terms: "copy clip duplikovat zdvojit klip" },
  copy: { group: "Clip menu", icon: ClipboardCopy, terms: "kopírovat zkopírovat klipy", label: "Copy clips" },
  cut: { group: "Clip menu", icon: ScissorsLineDashed, terms: "vyjmout klipy", label: "Cut clips" },
  paste: { group: "Clip menu", icon: ClipboardPaste, terms: "vložit klipy", label: "Paste clips" },
  pasteAttributes: { group: "Clip menu", icon: Paintbrush, terms: "style look vložit vlastnosti styl vzhled" },
  undo: { group: "Timeline", icon: Undo2, terms: "back zpět vrátit krok" },
  redo: { group: "Timeline", icon: Redo2, terms: "znovu opakovat krok" },
  zoomIn: { group: "Timeline", icon: ZoomIn, terms: "přiblížit časovou osu zvětšit" },
  zoomOut: { group: "Timeline", icon: ZoomOut, terms: "oddálit časovou osu zmenšit" },
  import: { group: "Media tab", icon: Upload, terms: "add files video photo importovat přidat soubory média video fotky" },
  export: { group: "Top bar", icon: Download, terms: "render save video exportovat uložit vyrenderovat" },
  aiPanel: { group: "Top bar", icon: Sparkles, terms: "agent chat claude codex chatgpt umělá inteligence asistent" },
};

/** Waits for what `selector` names to show, as a tab draws it, then moves the keyboard to it. */
function focusWhenShown(selector: string, fallback?: string, frames = 30) {
  const el = document.querySelector<HTMLElement>(selector);
  if (el) return el.focus();
  if (frames > 0) requestAnimationFrame(() => focusWhenShown(selector, fallback, frames - 1));
  else if (fallback) focusWhenShown(fallback);
}

/** Why the library's tabs are out of reach: the cover editor, reels and the voiceover take its place. */
function libraryWhy(): string | null {
  if (usePreview.getState().full) return FULL;
  if (useCover.getState().open) return "Close the cover editor first.";
  if (useReels.getState().open) return "Leave reels first.";
  if (useVoiceover.getState().phase !== "closed") return "Close the voiceover first.";
  return null;
}

/** A tab of the library, with the keyboard on the control that does the action, else on the tab. */
function tab(id: string, label: string, panelTab: PanelTab, icon: LucideIcon, terms: string, control?: string, open?: () => void): Command {
  const tabButton = `[role=tab][data-tab="${panelTab}"]`;
  return {
    id,
    label,
    group: `${panelTab[0].toUpperCase()}${panelTab.slice(1)} tab`,
    icon,
    terms,
    why: libraryWhy,
    run: () => {
      useEditor.setState({ panelTab });
      open?.();
      focusWhenShown(control ?? tabButton, tabButton);
    },
  };
}

const locked = () => {
  const s = useEditor.getState();
  return s.aiRun && s.lockShown ? AI_EDITING : null;
};

/** Why the control `selector` names cannot be pressed: what it says itself while disabled, as a sentence. */
function controlWhy(selector: string): string | null {
  const el = document.querySelector<HTMLButtonElement>(selector);
  if (!el) {
    if (useCover.getState().open) return "Close the cover editor first.";
    if (useReels.getState().open) return "Leave reels first.";
    return useSource.getState().asset ? "Go back to the timeline first." : "Not available here.";
  }
  // The full screen preview covers everything but its own controls.
  if (usePreview.getState().full && !el.closest("section[aria-label=Preview]")) return FULL;
  if (!el.disabled && el.getAttribute("aria-disabled") !== "true") return null;
  if (locked()) return AI_EDITING;
  // "Rotate: select a clip first" names the button the row already names.
  const said = (el.title || el.getAttribute("aria-label") || "").replace(/^[^:]+:\s*/, "").replace(/\.$/, "");
  return said ? sentence(said) : null;
}

/** A button on screen, pressed as a click would, with the keyboard left on it. */
function button(id: string, label: string, group: string, icon: LucideIcon, terms: string, selector: string): Command {
  return {
    id,
    label,
    group,
    icon,
    terms,
    why: () => controlWhy(selector),
    run: () => {
      const el = document.querySelector<HTMLElement>(selector);
      el?.focus();
      el?.click();
    },
  };
}


/** Every action the launcher offers, the most used first, as ties keep this order. */
export const COMMANDS: Command[] = [
  ...ACTIONS.filter((a) => KEYED[a.id]).map(
    (a): Command => ({ id: a.id, label: a.label, ...KEYED[a.id]!, key: a.id, why: () => whyNot(a.id), run: () => runAction(a.id) }),
  ),
  tab("captions", "Add captions", "captions", Captions, "generate subtitles auto přidat titulky vygenerovat automatické", "[data-generate-captions]"),
  tab("transcribe", "Transcribe speech", "transcript", ScrollText, "transcript text přepis přepsat řeč text mluvené slovo", "[data-transcribe]"),
  tab("text", "Add text", "text", Type, "title přidat text titulek nadpis popisek", "[data-text-preset]", () => useStickers.setState({ tab: "text" })),
  tab("music", "Add music", "audio", Music, "song audio hudba písnička skladba", "#library-panel-audio input", () => useSounds.setState({ tab: "music" })),
  tab("effects", "Add a sound effect", "audio", AudioLines, "sfx whoosh audio zvuk zvukový efekt", "#library-panel-audio input", () => useSounds.setState({ tab: "effects" })),
  tab("transitions", "Add a transition", "transitions", Blend, "dissolve fade přechod prolínačka", undefined),
  tab("filters", "Filters and LUTs", "filters", Palette, "color colour log lut grade filtr barvy barevné korekce", undefined),
  tab("plugins", "Plugins", "plugins", Puzzle, "plugin rozšíření doplňky", undefined),
  tab("media", "Media", "media", Film, "library files média knihovna soubory", undefined),
  button("detach", "Detach audio", "Timeline", Unlink, "separate sound oddělit zvuk odpojit", "[data-detach-audio]"),
  button("rotate", "Rotate 90°", "Timeline", RotateCwSquare, "turn otočit natočit", "[data-rotate]"),
  {
    ...button("voiceover", "Record voiceover", "Timeline", Mic, "microphone narration nahrát komentář mikrofon hlas", "[data-voiceover-button]"),
    // Its button closes the voiceover while it is open.
    why: () => (useVoiceover.getState().phase !== "closed" ? "The voiceover is open." : controlWhy("[data-voiceover-button]")),
  },
  button("compare", "Compare before and after", "Preview", SquareSplitHorizontal, "original colour porovnat před po originál barvy", "[data-compare]"),
  button("ratio", "Ratio", "Preview", RectangleHorizontal, "format aspect vertical 9:16 16:9 formát poměr stran na výšku na šířku", "[data-ratio]"),
  button("quality", "Resolution and frame rate", "Top bar", SlidersHorizontal, "quality fps 1080p 4k kvalita rozlišení snímková frekvence", "[data-quality]"),
  button("versions", "Versions", "Top bar", History, "history restore verze historie obnovit", "[data-versions]"),
  {
    id: "cover",
    label: "Make a cover",
    group: "Main track",
    icon: Image,
    terms: "thumbnail obálka náhled titulní obrázek",
    why: () => {
      const s = useEditor.getState();
      if (useCover.getState().open) return "The cover editor is open.";
      if (useReels.getState().open) return "Leave reels first.";
      if (usePreview.getState().full) return FULL;
      return s.snap && projectDuration(s.snap.project) > 0 ? null : "Add a video or picture first.";
    },
    run: () => openCover(),
  },
  { id: "youtube", label: "Video from YouTube", group: "Media tab", icon: Link2, terms: "download link odkaz stáhnout", why: locked, run: () => openYoutube() },
  { id: "settings", label: "Settings", group: "Top bar", icon: Settings, terms: "preferences options nastavení předvolby", why: () => null, run: () => openSettings() },
  { id: "keys", label: "Keyboard shortcuts", group: "Settings", icon: Keyboard, terms: "keys hotkeys klávesové zkratky klávesy", why: () => null, run: () => openSettings("keyboard") },
];

const wordsOf = (text: string) => fold(text).split(/[^\p{L}\p{N}]+/u).filter(Boolean);

/** Edits that turn `a` into `b`, a swap of two neighbouring letters counting as one. */
export function distance(a: string, b: string): number {
  const d = Array.from({ length: a.length + 1 }, (_, i) => Array.from({ length: b.length + 1 }, (_, j) => (i === 0 ? j : j === 0 ? i : 0)));
  for (let i = 1; i <= a.length; i++)
    for (let j = 1; j <= b.length; j++) {
      d[i][j] = Math.min(d[i - 1][j] + 1, d[i][j - 1] + 1, d[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
      if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) d[i][j] = Math.min(d[i][j], d[i - 2][j - 2] + 1);
    }
  return d[a.length][b.length];
}

/**
 * How well a typed word matches a word of the command: 4 the word, 3 its start, 2 inside it, 1 its start with a
 * typo (one from four letters, two from eight), 0 not at all.
 */
function wordScore(typed: string, word: string): number {
  if (word === typed) return 4;
  if (word.startsWith(typed)) return 3;
  if (typed.length >= 3 && word.includes(typed)) return 2;
  if (typed.length < 4) return 0;
  const allowed = typed.length >= 8 ? 2 : 1;
  for (let n = typed.length - 1; n <= typed.length + 1; n++) if (n <= word.length && distance(typed, word.slice(0, n)) <= allowed) return 1;
  return 0;
}

export type Found = { command: Command; score: number; /** Every typed word matched without a typo. */ exact: boolean };

/**
 * The commands every typed word matches, best first: by name, group or the words they are found by, ignoring case
 * and accents, a part of a word or a word with a typo.
 */
export function findCommands(query: string, commands = COMMANDS): Found[] {
  const typed = wordsOf(query);
  if (typed.length === 0) return [];
  const whole = fold(query.trim());
  return commands
    .flatMap((command) => {
      const words = wordsOf(`${command.label} ${command.group} ${command.terms}`);
      const scores = typed.map((t) => Math.max(0, ...words.map((w) => wordScore(t, w))));
      if (scores.some((s) => s === 0)) return [];
      const named = fold(command.label).startsWith(whole) ? 3 : 0;
      return [{ command, score: scores.reduce((a, b) => a + b, 0) + named, exact: scores.every((s) => s >= 2) }];
    })
    .sort((a, b) => b.score - a.score);
}

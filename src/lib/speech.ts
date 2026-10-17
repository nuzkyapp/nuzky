import { useEffect, useState } from "react";
import { create } from "zustand";
import { api, errorText, plainError } from "./api";
import { aiLocked, currentEpoch, enqueue, undoAction, useEditor, whenIdle } from "./store";
import { CAPTION_STYLES } from "./presets";
import type { Animation, Project, TextStyle, TranscriptCut, TranscriptView, WordsCorrected } from "./types";

/** Characters per caption when captions show 1–3 words, like reels. */
export const SHORT_CAPTION_CHARS = 15;
/** Longest corrected word, as the engine's MAX_CORRECTION_CHARS. */
export const MAX_WORD_CHARS = 100;
/** A gap this long between two words starts a new paragraph in the transcript. */
const PARAGRAPH_GAP_US = 1_000_000;

/** Languages offered for recognition, as Whisper codes. */
export const SPEECH_LANGUAGES: [string, string][] = [
  ["auto", "Detect automatically"],
  ["cs", "Czech"],
  ["sk", "Slovak"],
  ["en", "English"],
  ["de", "German"],
  ["pl", "Polish"],
  ["es", "Spanish"],
  ["fr", "French"],
  ["it", "Italian"],
  ["uk", "Ukrainian"],
];

const LANGUAGE_KEY = "nuzky.speechLanguage";

/** The language picked last, else the computer's language when it is offered, else detection. */
function initialLanguage(): string {
  const offered = (id: string | null | undefined) => !!id && SPEECH_LANGUAGES.some(([code]) => code === id);
  const saved = localStorage.getItem(LANGUAGE_KEY);
  if (offered(saved)) return saved!;
  const system = (navigator.languages?.[0] ?? navigator.language ?? "").slice(0, 2).toLowerCase();
  return offered(system) ? system : "auto";
}

/** Picks the spoken language for Captions and Transcript and remembers it for next time. */
export function setSpeechLanguage(language: string) {
  localStorage.setItem(LANGUAGE_KEY, language);
  useSpeech.setState({ language });
}

const MODEL_KEY = "nuzky.speechModel";
/** The Whisper models the app downloads, as `MODELS` in src-tauri/src/jobs.rs. */
const SPEECH_MODEL_IDS = ["base", "small", "large-v3-turbo-q5_0"];

/** Whether a model was ever picked, in Get set up, Captions or Transcript. */
export const speechModelPicked = () => SPEECH_MODEL_IDS.includes(localStorage.getItem(MODEL_KEY) ?? "");

/** Picks the model for Captions and Transcript and remembers it for next time. */
export function setSpeechModel(model: string) {
  localStorage.setItem(MODEL_KEY, model);
  useSpeech.setState({ model });
}

interface SpeechState {
  /** Shared by Captions and Transcript, so both use the same recognition. */
  model: string;
  language: string;
  /** 1–3 words per caption, or 0 for whole phrases. */
  captionWords: number;
  /** Look of new captions while none are on the timeline, font included. */
  captionLook: CaptionLook;
  /** Pauses longer than this are shown in the transcript and can be removed. */
  pauseUs: number;
  /** Counts changes to the stored transcripts and prepared sound, so the transcript is fetched again. */
  stored: number;
}

export const useSpeech = create<SpeechState>(() => ({
  model: speechModelPicked() ? localStorage.getItem(MODEL_KEY)! : "large-v3-turbo-q5_0",
  language: initialLanguage(),
  captionWords: 2,
  captionLook: { style: { ...CAPTION_STYLES[0].style, fontFamily: null }, animIn: null, animOut: null },
  pauseUs: 500_000,
  stored: 0,
}));

/** Why the timeline cannot be transcribed, or null. Same rule as the engine's `is_heard`. */
export function speechBlocker(project: Project): string | null {
  const heard = project.tracks.some(
    (t) =>
      !t.muted &&
      t.clips.some((c) => {
        const m = c.content;
        if (m.type !== "media" || m.volume <= 0) return false;
        const asset = project.assets.find((a) => a.id === m.assetId);
        return asset?.kind === "video" && asset.hasAudio;
      }),
  );
  return heard ? null : "Add a video with sound to the timeline first";
}

const sameView = (a: TranscriptView, b: TranscriptView) =>
  a.key === b.key && JSON.stringify([a.pauses, a.hesitations, a.untranscribed]) === JSON.stringify([b.pauses, b.hesitations, b.untranscribed]);

/**
 * The timeline's speech as it is now, fetched again after every edit, recognition or change of the
 * pause length. An edit that leaves speech alone keeps the same view, so the selection survives it.
 */
export function useTranscriptView(): { view: TranscriptView | null; error: string | null } {
  const revision = useEditor((s) => s.snap?.revision);
  const epoch = useEditor((s) => s.snap?.sessionEpoch);
  const pauseUs = useSpeech((s) => s.pauseUs);
  const stored = useSpeech((s) => s.stored);
  const [state, setState] = useState<{ epoch?: string; view: TranscriptView | null; error: string | null }>({ view: null, error: null });
  useEffect(() => {
    let live = true;
    api.transcriptView(pauseUs).then(
      (view) => live && setState((s) => ({ epoch, view: s.epoch === epoch && s.view && sameView(s.view, view) ? s.view : view, error: null })),
      (e) => live && setState({ epoch, view: null, error: plainError(errorText(e)) }),
    );
    return () => {
      live = false;
    };
  }, [revision, epoch, pauseUs, stored]);
  // Another project's transcript never shows, not even until the new one arrives.
  return state.epoch === epoch ? state : { view: null, error: null };
}

/** How captions look: their style with its font, and the entry and exit animation of every caption. */
export type CaptionLook = { style: TextStyle; animIn: Animation | null; animOut: Animation | null };

/** What each transcript or captions job was asked for and in which project, so Try again repeats that job there. */
const asked = new Map<string, { captions: CaptionLook | null; refresh: boolean; epoch: string | undefined }>();

/** Starts recognition for the transcript, of every heard file with `refresh`, or for captions in `captions`. */
export async function startSpeech(captions: CaptionLook | null, refresh = false) {
  // Captions change the project; a transcript only reads it.
  if (captions && aiLocked()) return;
  const { model, language, captionWords } = useSpeech.getState();
  // Taken before waiting, so a project opened meanwhile refuses the request instead of running it.
  const epoch = currentEpoch();
  // The backend transcribes the project as it is when the job starts; wait for queued edits.
  await whenIdle();
  try {
    const words = captionWords || null;
    const job = captions
      ? await api.startCaptions(model, language, captions, words, words ? SHORT_CAPTION_CHARS : null, epoch)
      : await api.startTranscript(model, language, refresh, epoch);
    asked.set(job, { captions, refresh, epoch });
  } catch (e) {
    useEditor.getState().toast({ kind: "error", text: errorText(e) });
  }
}

/** Whether `job` can run again: its project is still open. A job whose start has not answered yet counts as open. */
export const canRetrySpeech = (job: string) => (asked.get(job)?.epoch ?? currentEpoch()) === currentEpoch();

/** Starts a failed transcript or captions job again as it was asked for, in the same project. */
export function retrySpeech(job: string) {
  const request = asked.get(job);
  if (request && canRetrySpeech(job)) void startSpeech(request.captions, request.refresh);
}

/**
 * Runs a transcript cut after the edits queued before it, as one undo step, and offers Undo.
 * `what` describes it from the time removed. Resolves to null when nothing was cut.
 */
async function cut(run: (epoch: string | undefined) => Promise<TranscriptCut>, what: (removedUs: number) => string): Promise<TranscriptCut | null> {
  if (aiLocked()) return null;
  const out: { done?: TranscriptCut } = {};
  await enqueue(async (epoch) => (out.done = await run(epoch)).snapshot);
  if (!out.done) return null;
  useEditor.getState().toast({ kind: "info", text: `Removed ${what(out.done.removedUs)}`, action: undoAction(out.done.snapshot) });
  return out.done;
}

export const cutWords = (key: string, ranges: [number, number][], hesitations: number[], what: (removedUs: number) => string) =>
  cut((epoch) => api.cutWords(key, ranges, hesitations, epoch), what);

export const removePauses = (key: string, pauseUs: number, only: number[] | null, hesitations: number[], what: (removedUs: number) => string) =>
  cut((epoch) => api.removePauses(key, pauseUs, only, hesitations, epoch), what);

export const removeRetakes = (key: string, pauseUs: number, what: (removedUs: number) => string) => cut((epoch) => api.removeRetakes(key, pauseUs, epoch), what);

/**
 * Corrects how a word reads, in the transcript and in the captions that show it, after the edits
 * queued before it, as one undo step, and offers Undo. Resolves to whether it was corrected.
 */
export async function correctWord(key: string, i: number, text: string, shown: string): Promise<boolean> {
  if (aiLocked()) return false;
  const out: { done?: WordsCorrected } = {};
  await enqueue(async (epoch) => (out.done = await api.correctWords(key, [{ i, text }], epoch)).snapshot);
  if (!out.done) return false;
  const n = out.done.captions;
  const captions = n > 0 ? `, also in ${n === 1 ? "its caption" : `${n} captions`}` : "";
  useEditor.getState().toast({ kind: "info", text: `Corrected “${shown}” to “${text}”${captions}`, action: undoAction(out.done.snapshot) });
  return true;
}

export type Token =
  /** `original` is what recognition wrote, when the word was corrected. */
  | { kind: "word"; i: number; startUs: number; endUs: number; text: string; original?: string }
  /** The part of a pause that removing it cuts; `i` is its place in the view's pauses. */
  | { kind: "pause"; i: number; startUs: number; endUs: number; gapUs: number }
  /** Voice between words that recognition did not write; `i` is the word before it. */
  | { kind: "hesitation"; i: number; startUs: number; endUs: number };

/** "3 words", or the pauses and hesitations when no word is included: "1 pause, 1 hesitation". */
export function tokenCounts(tokens: Token[]): string {
  const count = (kind: Token["kind"]) => {
    const n = tokens.filter((t) => t.kind === kind).length;
    return n > 0 ? `${n} ${kind}${n === 1 ? "" : "s"}` : null;
  };
  const parts = count("word") ? [count("word")] : [count("pause"), count("hesitation")];
  return parts.filter((p) => p !== null).join(", ");
}

/** Words, pauses and hesitations in timeline order. */
export function tokenize(view: TranscriptView): Token[] {
  const words = view.words.map((w): Token => ({ kind: "word", i: w.i, startUs: w.startUs, endUs: w.endUs, text: w.text, original: w.original }));
  const pauses = view.pauses.map((p, i): Token => ({ kind: "pause", i, ...p }));
  const hesitations = view.hesitations.map((h): Token => ({ kind: "hesitation", i: h.afterWord, startUs: h.startUs, endUs: h.endUs }));
  return [...words, ...pauses, ...hesitations].sort((a, b) => a.startUs - b.startUs);
}

/** Token index ranges [from, to) of paragraphs, split where speech pauses for a second or more. */
export function paragraphs(tokens: Token[]): [number, number][] {
  const out: [number, number][] = [];
  let start = 0;
  let lastWordEnd: number | null = null;
  tokens.forEach((t, i) => {
    if (t.kind !== "word") return;
    if (lastWordEnd !== null && t.startUs - lastWordEnd >= PARAGRAPH_GAP_US && i > start) {
      out.push([start, i]);
      start = i;
    }
    lastWordEnd = t.endUs;
  });
  if (start < tokens.length) out.push([start, tokens.length]);
  return out;
}

/** Index of the token playing at `t`, or -1. A word stays current through a short gap after it. */
export function tokenAt(tokens: Token[], t: number): number {
  let lo = 0;
  let hi = tokens.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (tokens[mid].startUs <= t) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  if (found < 0) return -1;
  const next = tokens[found + 1];
  return t < tokens[found].endUs || (tokens[found].kind === "word" && next?.kind === "word") ? found : -1;
}

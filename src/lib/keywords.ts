import { KEYWORD_COLOR } from "./presets";
import { useEditor } from "./store";
import type { CaptionWord, Clip, EditCmd, KeywordPick, TextStyle, Track } from "./types";

const letters = (w: CaptionWord) => [...w.text].filter((c) => /[\p{Alphabetic}\p{N}]/u.test(c)).length;

/** The engine colours a caption's words only while its text is them joined by single spaces. */
export const wordsShown = (text: string, words: CaptionWord[]) => words.length > 0 && words.map((w) => w.text).join(" ") === text;

/** Which words of a caption the pick alone makes key, as the engine does (`key_words` in crates/engine/src/model.rs). */
function picks(words: CaptionWord[], pick: KeywordPick | null | undefined): boolean[] {
  const said = words.flatMap((w, i) => (letters(w) > 0 ? [i] : []));
  let picked: number[] = [];
  if (pick === "emphasis") picked = said.filter((i) => words[i].key);
  else if (pick === "first") picked = said.slice(0, 1);
  else if (pick === "last") picked = said.slice(-1);
  else if (pick === "longest") picked = said.length ? [said.reduce((a, b) => (letters(words[b]) > letters(words[a]) ? b : a))] : [];
  return words.map((_, i) => picked.includes(i));
}

/** A word of a caption: whether it is key, whether the pick alone would make it so, and the key word colour. */
export type CaptionWordRef = { clipId: string; word: number; key: boolean; picked: boolean; color: string };

/** The words of a caption, or null when it colours none of them. */
export function captionKeys(clip: Clip): { words: CaptionWord[]; refs: CaptionWordRef[]; color: string } | null {
  const c = clip.content;
  if (c.type !== "text" || !c.words || !wordsShown(c.text, c.words)) return null;
  const picked = picks(c.words, c.style.keywords?.pick);
  const color = keyColor(c.style);
  const refs = c.words.map((w, i) => ({ clipId: clip.id, word: i, key: w.marked ?? picked[i], picked: picked[i], color }));
  return { words: c.words, refs, color };
}

export const keyColor = (style: TextStyle) => style.keywords?.color ?? KEYWORD_COLOR;

/**
 * For each span of the timeline, the caption words said in it, each where its middle is. Only the caption a word is
 * heard in counts, as a split leaves every word in both halves; the engine keeps marks the same way.
 */
export function captionWordsIn(tracks: Track[], spans: { startUs: number; endUs: number }[]): CaptionWordRef[][] {
  const all = tracks
    .flatMap((t) => (t.kind === "text" ? t.clips : []))
    .flatMap((clip) => {
      const keys = captionKeys(clip);
      if (!keys) return [];
      return keys.words.flatMap((w, i) => {
        const middle = (w.startUs + w.endUs) / 2;
        return middle >= 0 && middle < clip.durationUs ? [{ ref: keys.refs[i], atUs: clip.startUs + middle }] : [];
      });
    })
    .sort((a, b) => a.atUs - b.atUs);
  return spans.map(({ startUs, endUs }) => {
    let lo = 0;
    let hi = all.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (all[mid].atUs < startUs) lo = mid + 1;
      else hi = mid;
    }
    const out: CaptionWordRef[] = [];
    for (let i = lo; i < all.length && all[i].atUs < endUs; i++) out.push(all[i].ref);
    return out;
  });
}

/**
 * Makes the words key, or plain, as one undo step. A word set to what its captions' pick gives anyway follows the pick
 * again, so trying a word and taking it back leaves nothing behind; otherwise the mark stays whatever the pick.
 */
export function markKeyWords(words: CaptionWordRef[], key: boolean) {
  const cmds: EditCmd[] = words.map((w) => ({ type: "markKeyWord", clipId: w.clipId, word: w.word, marked: key === w.picked ? null : key }));
  if (cmds.length) void useEditor.getState().edit(cmds);
}

/** Relative luminance of a `#rgb`, `#rrggbb` or `#rrggbbaa` colour, its alpha left out. */
function luminance(color: string) {
  const hex = color.slice(1);
  const full = hex.length === 3 ? [...hex].map((c) => c + c).join("") : hex.slice(0, 6);
  const [r, g, b] = [0, 2, 4].map((i) => {
    const c = parseInt(full.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** `panel` (#18181b), behind the transcript and the inspector. */
const PANEL = luminance("#18181b");

/**
 * Whether a key word colour can carry text on the panes: 4.5:1 on `panel` and apart from the white of plain text. A key
 * word in white, as Neon has, or in a dark colour then shows by a line under it instead.
 */
export function keyAsText(color: string) {
  const l = luminance(color);
  return (l + 0.05) / (PANEL + 0.05) >= 4.5 && l < 0.8;
}

/** The key word colour for a line on the panes, or the text's own colour when it is too dark to see there. */
export const keyLine = (color: string) => ((luminance(color) + 0.05) / (PANEL + 0.05) >= 3 ? color : "currentColor");

/** Black or white, whichever reads on the key word colour. */
export const onKey = (color: string) => (luminance(color) > 0.18 ? "#000" : "#fff");

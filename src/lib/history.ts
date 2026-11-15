import { create } from "zustand";

/** Which history the dialog shows: Your style, My caption styles, or both. */
export type HistoryKind = "all" | "style" | "caption-styles";

/** The History dialog: every revision of Your style and My caption styles the account keeps. */
export const useHistory = create<{ open: boolean; kind: HistoryKind }>(() => ({ open: false, kind: "all" }));

export const openHistory = (kind: HistoryKind = "all") => useHistory.setState({ open: true, kind });

export type DiffLine = { kind: "same" | "back" | "goes"; text: string };

/** Longer texts are shown whole: comparing them line by line would take too long. */
const MAX_CELLS = 4_000_000;

/**
 * EDIT.md as restoring `then` over `now` changes it, line by line: `back` lines are only in `then` and come back, `goes`
 * lines are only in `now` and go away.
 */
export function lineDiff(then: string, now: string): DiffLine[] {
  const a = then.split("\n");
  const b = now.split("\n");
  if (a.length * b.length > MAX_CELLS) return a.map((text) => ({ kind: "same", text }));
  // The longest common lines from each pair of positions to the end.
  const width = b.length + 1;
  const common = new Uint32Array((a.length + 1) * width);
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--)
      common[i * width + j] = a[i] === b[j] ? common[(i + 1) * width + j + 1] + 1 : Math.max(common[(i + 1) * width + j], common[i * width + j + 1]);
  const lines: DiffLine[] = [];
  let [i, j] = [0, 0];
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) {
      lines.push({ kind: "same", text: a[i] });
      i++;
      j++;
    } else if (j < b.length && (i === a.length || common[i * width + j + 1] >= common[(i + 1) * width + j])) {
      lines.push({ kind: "goes", text: b[j++] });
    } else {
      lines.push({ kind: "back", text: a[i++] });
    }
  }
  return lines;
}

const DAY = 24 * 60 * 60 * 1000;
const startOfDay = (ms: number) => new Date(ms).setHours(0, 0, 0, 0);
const DATE = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric" });
const DATE_YEAR = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric", year: "numeric" });
const CLOCK = new Intl.DateTimeFormat("en-US", { hour: "numeric", minute: "2-digit" });

/** The day a revision is listed under: Today, Yesterday, Nov 12. */
export function dayLabel(ms: number, now = Date.now()): string {
  const days = Math.round((startOfDay(now) - startOfDay(ms)) / DAY);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  return new Date(ms).getFullYear() === new Date(now).getFullYear() ? DATE.format(ms) : DATE_YEAR.format(ms);
}

export const clockLabel = (ms: number) => CLOCK.format(ms);

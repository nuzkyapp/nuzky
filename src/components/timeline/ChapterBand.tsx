import type { KeyboardEvent } from "react";
import { ListVideo } from "lucide-react";
import { currentChapter, openChapters, useChapterList, useChapters } from "../../lib/chapters";
import { useEditor } from "../../lib/store";
import { US, formatDuration } from "../../lib/time";

export const BAND_H = 24;
/** A segment's padding, half a left-out mark and the space kept between a mark and a title, in px. */
const PAD = 6;
const HALF_MARK = 7;

/** Where a segment `width` px wide draws its title so it runs under none of the marks at `marks` px into it: the
 * widest stretch between the marks and the segment's padding. */
function titleRoom(marks: number[], width: number) {
  if (marks.length === 0) return undefined;
  const edges = [PAD, ...[...marks].sort((a, b) => a - b).flatMap((m) => [m - HALF_MARK - PAD, m + HALF_MARK + PAD]), width - PAD];
  let best = { from: PAD, room: -1 };
  for (let i = 0; i < edges.length; i += 2) {
    const [from, to] = [Math.max(PAD, edges[i]), Math.min(width - PAD, edges[i + 1])];
    if (to - from > best.room) best = { from, room: to - from };
  }
  return { marginLeft: best.from - PAD, maxWidth: Math.max(0, best.room) };
}

/** The band shows while the project has chapters or the AI is finding them. */
export function useChapterBandShown() {
  const has = useEditor((s) => (s.snap?.project.chapters?.length ?? 0) > 0);
  const finding = useChapters((s) => s.finding);
  return has || finding;
}

/**
 * The video's chapters under the ruler, as YouTube's player bar splits a video: one segment per chapter in the list,
 * from its start to the next, with its title. Clicking one moves the playhead to its start, a double click or F2
 * opens its title in the Transcript tab. A chapter the list leaves out is a dashed mark at its start.
 */
export function ChapterBand({ zoom, header, top, endUs }: { zoom: number; header: number; top: number; endUs: number }) {
  const { list } = useChapterList();
  const finding = useChapters((s) => s.finding);
  const chapters = (list?.chapters ?? []).map((c, i) => ({ ...c, i }));
  const listed = chapters.filter((c) => !c.leftOut && c.startUs !== null);
  const starts = listed.map((c) => c.startUs!);
  const marks = chapters.filter((c) => c.leftOut && c.startUs !== null).map((c) => c.startUs!);
  const current = useEditor((s) => currentChapter(starts, s.timeUs));
  const x = (us: number) => (us / US) * zoom;
  const onKey = (i: number) => (e: KeyboardEvent) => {
    if (e.key !== "F2") return;
    e.preventDefault();
    openChapters(i);
  };

  return (
    <>
      <div
        className="sticky left-0 z-[48] flex shrink-0 items-center gap-2 border-b border-r border-white/[.07] bg-panel pl-[7px] text-[12px] text-muted"
        style={{ width: header, height: BAND_H, top, marginBottom: -BAND_H }}
      >
        <span className="flex w-6 justify-center" aria-hidden>
          <ListVideo size={14} />
        </span>
        Chapters
      </div>
      <div className="sticky z-40 flex shrink-0" style={{ top, height: BAND_H }}>
        <div className="shrink-0" style={{ width: header }} />
        <div data-chapter-band role="list" aria-label="Chapters for YouTube" className="relative flex-1 overflow-x-clip border-b border-white/[.07] bg-well">
          {finding && listed.length === 0 && <div className="skeleton absolute top-1 h-[15px] rounded-[4px]" style={{ left: 0, width: Math.max(24, x(endUs) - 2) }} aria-label="Finding chapters" />}
          {listed.map((c, k) => {
            const next = starts[k + 1] ?? endUs;
            const width = Math.max(4, x(next) - x(c.startUs!) - 2);
            const clear = titleRoom(
              marks.filter((m) => m > c.startUs! && m < next).map((m) => x(m) - x(c.startUs!)),
              width,
            );
            return (
              <button
                key={c.i}
                type="button"
                role="listitem"
                data-chapter-segment
                className={`absolute top-1 h-[15px] overflow-hidden rounded-[4px] px-1.5 text-left text-[11px] leading-[15px] whitespace-nowrap transition-colors duration-[120ms] ease-out ${
                  k === current ? "bg-accent/[.22] text-fg" : "bg-white/[.08] text-fg/80 hover:bg-white/[.14] hover:text-fg"
                }`}
                style={{ left: x(c.startUs!), width }}
                title={`${c.time} · ${c.title} · ${formatDuration(next - c.startUs!)}`}
                onClick={() => useEditor.getState().seek(c.startUs!)}
                onDoubleClick={() => openChapters(c.i)}
                onKeyDown={onKey(c.i)}
              >
                <span className="block overflow-hidden text-ellipsis" style={clear}>
                  {c.title}
                </span>
              </button>
            );
          })}
          {chapters
            .filter((c) => c.leftOut && c.startUs !== null)
            .map((c) => (
              <button
                key={c.i}
                type="button"
                role="listitem"
                data-chapter-left-out
                className="absolute top-[5px] h-3.5 w-3.5 -translate-x-1/2 rounded-[4px] border border-dashed border-warn/70 text-center text-[11px] leading-3 font-semibold text-warn"
                style={{ left: x(c.startUs!) }}
                title={`${c.title} · Not on YouTube: ${c.leftOut}`}
                onClick={() => openChapters(c.i)}
                onKeyDown={onKey(c.i)}
              >
                !
              </button>
            ))}
        </div>
      </div>
    </>
  );
}

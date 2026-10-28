import { useEffect } from "react";
import { CircleAlert, Film, Lock, Sparkles, TriangleAlert } from "lucide-react";
import { loadPoster, useLibrary } from "../../lib/library";
import { formatLength } from "../../lib/time";
import type { LibraryProject, ThumbnailFormat } from "../../lib/types";
import { FadeImg } from "../ui";

const COVER_RATIO: Record<ThumbnailFormat, number> = { cover_9x16: 9 / 16, youtube_16x9: 16 / 9 };

/**
 * The picture's size inside a box of `box` width over height, in the `shown` shape of the poster it shows, or until one
 * arrives, of its cover or else its canvas.
 */
function frameStyle(p: LibraryProject, box: number, shown: number | null | undefined) {
  const canvas = p.width > 0 && p.height > 0 ? p.width / p.height : 9 / 16;
  const ratio = shown ?? (p.cover ? COVER_RATIO[p.cover] : canvas);
  return ratio < box ? { height: "100%", aspectRatio: `${ratio}` } : { width: "100%", aspectRatio: `${ratio}` };
}

const chip = "flex items-center gap-1 rounded-[5px] bg-black/60 px-1.5 py-[3px] text-[11px] font-medium leading-none text-fg";

/**
 * The project's picture, its cover or else a frame, in its own shape, centred in a dark box like the preview
 * stage: rendered by the same renderer as the export. While it loads, the frame is an empty shape of the right size.
 */
export function Poster({ p, box, ai = false, compact = false }: { p: LibraryProject; /** Width over height of the box. */ box: number; ai?: boolean; compact?: boolean }) {
  const poster = useLibrary((s) => s.posters[p.path]);
  useEffect(() => loadPoster(p), [p]);

  if (p.state === "broken")
    return (
      <div className="flex h-full w-full flex-col items-center justify-center gap-1.5 text-muted">
        <CircleAlert size={compact ? 15 : 20} className="text-danger" />
        {!compact && <span className="text-[12px]">Can't open</span>}
      </div>
    );
  const frame = frameStyle(p, box, poster?.ratio);
  if (p.state === "empty")
    return (
      <div className="flex h-full w-full items-center justify-center">
        <div className="flex flex-col items-center justify-center gap-1.5 rounded-[3px] border border-dashed border-white/15 text-muted" style={frame}>
          <Film size={compact ? 13 : 17} />
          {!compact && <span className="text-[11px]">Empty project</span>}
        </div>
      </div>
    );
  return (
    <div className="flex h-full w-full items-center justify-center">
      <div className={`relative overflow-hidden bg-white/[.04] ${p.state === "busy" ? "opacity-35" : ""}`} style={frame}>
        {poster?.url ? (
          <FadeImg src={poster.url} alt="" draggable={false} className="h-full w-full object-cover" />
        ) : (
          poster && (
            <div className="flex h-full w-full items-center justify-center text-subtle">
              <Film size={compact ? 13 : 17} />
            </div>
          )
        )}
        {!compact && (
          // The length keeps the bottom right; on a frame too narrow for both, the other chip sits above it.
          <div className="absolute inset-x-1.5 bottom-1.5 flex flex-row-reverse flex-wrap-reverse justify-between gap-1">
            {p.state !== "busy" && <span className={`tabular ${chip}`}>{formatLength(p.durationUs)}</span>}
            {(ai || p.state === "missing") && (
              <span className={`whitespace-nowrap ${chip}`}>
                {ai ? <Sparkles size={11} className="text-accent" /> : <TriangleAlert size={11} className="text-warn" />}
                {ai ? "AI" : `${p.missing} missing`}
              </span>
            )}
          </div>
        )}
      </div>
      {!compact && p.state === "busy" && (
        <span className={`absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 whitespace-nowrap ${chip}`}>
          <Lock size={11} /> In use
        </span>
      )}
    </div>
  );
}

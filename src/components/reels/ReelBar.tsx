import { Pause, Play, SkipBack, SkipForward } from "lucide-react";
import { candidateOf, closeReels, playEnding, toggleReelPlay, useReels } from "../../lib/reels";
import { useEditor } from "../../lib/store";
import { formatTime } from "../../lib/time";
import { Button, IconButton } from "../ui";
import { useKeyLabels } from "../../lib/keys";

/** The playhead in the recording; the reel's span stands over its words. */
function Where() {
  const timeUs = useEditor((s) => s.timeUs);
  return <span className="tabular truncate text-[13px] font-medium text-fg">{formatTime(timeUs)}</span>;
}

/** The transport while choosing reels: the reel from its start, its ending, and Done. */
export function ReelBar() {
  const { withKey } = useKeyLabels();
  const playing = useEditor((s) => s.playing);
  const current = useReels((s) => s.current);
  const c = useEditor((s) => candidateOf(s.snap?.project, current));
  const none = !c;
  return (
    <div className="bar grid h-11 w-full max-w-[620px] grid-cols-[1fr_auto_1fr] items-center gap-2 rounded-full pl-[18px] pr-1.5">
      <Where />
      <div className="flex items-center gap-2.5">
        <IconButton round label={withKey("Reel start", "goStart")} disabled={none} onClick={() => c && useEditor.getState().seek(c.startUs)}>
          <SkipBack size={16} />
        </IconButton>
        <button
          type="button"
          aria-label={withKey(playing ? "Pause" : "Play the reel", "playPause")}
          title={withKey(playing ? "Pause" : "Play the reel", "playPause")}
          disabled={none}
          onClick={toggleReelPlay}
          className="flex h-9 w-9 items-center justify-center rounded-full bg-fg text-black shadow-[0_1px_3px_rgb(0_0_0/.4)] transition-transform duration-[120ms] ease-out active:scale-95 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {playing ? <Pause size={16} fill="currentColor" /> : <Play size={16} fill="currentColor" className="translate-x-px" />}
        </button>
        <IconButton round label={withKey("Hear the ending", "goEnd")} disabled={none} onClick={playEnding}>
          <SkipForward size={16} />
        </IconButton>
      </div>
      <div className="flex justify-end">
        <Button pill className="h-8 px-3.5" title="Back to editing (Esc)" onClick={closeReels}>
          Done
        </Button>
      </div>
    </div>
  );
}

/**
 * What Center crop keeps of the picture, fitted into the `width`×`height` canvas as a clip is: its middle 9:16, the rest
 * dimmed as the safe zone dims. `picture` is its width over height.
 */
export function CropGuide({ width, height, picture }: { width: number; height: number; picture: number }) {
  const canvas = width / height;
  const [pw, ph] = picture >= canvas ? [1, canvas / picture] : [picture / canvas, 1];
  const kw = Math.min(pw, (ph * 9) / 16 / canvas);
  const kh = Math.min(ph, (pw * canvas * 16) / 9);
  if (kw > pw - 0.001 && kh > ph - 0.001) return null;
  const pct = (v: number) => `${v * 100}%`;
  const [left, top] = [(1 - kw) / 2, (1 - kh) / 2];
  const band = "pointer-events-none absolute bg-black/45";
  return (
    <>
      <div className={`${band} inset-x-0 top-0`} style={{ height: pct(top) }} />
      <div className={`${band} inset-x-0 bottom-0`} style={{ height: pct(top) }} />
      <div className={`${band} left-0`} style={{ top: pct(top), bottom: pct(top), width: pct(left) }} />
      <div className={`${band} right-0`} style={{ top: pct(top), bottom: pct(top), width: pct(left) }} />
      <div className="pointer-events-none absolute border border-dashed border-fg/50" style={{ left: pct(left), top: pct(top), width: pct(kw), height: pct(kh) }} />
    </>
  );
}

import { memo } from "react";
import { Flag } from "lucide-react";
import { useBeats } from "../../lib/beats";
import { US } from "../../lib/time";

/** Closer than this the beats would blur into a band, so only bar starts show. */
const MIN_GAP_PX = 4;

/**
 * The song's beat over its clip, once found: a short tick per beat, a taller one where a bar starts, and a flag at each
 * drop. Source times go through the clip's source in and speed, as the waveform's do. Not subscribed to the playhead.
 */
export const BeatTicks = memo(function BeatTicks({
  assetId,
  sourceInUs,
  speed,
  zoom,
  width,
  height,
  visible,
}: {
  assetId: string;
  sourceInUs: number;
  speed: number;
  zoom: number;
  width: number;
  height: number;
  /** The part of the clip on screen, in clip pixels. */
  visible: [number, number];
}) {
  const song = useBeats((s) => s.songs[assetId]);
  if (!song || song.beatsUs.length < 2) return null;
  const x = (us: number) => ((us - sourceInUs) / speed / US) * zoom;
  const gap = x(song.beatsUs[1]) - x(song.beatsUs[0]);
  const beats = gap >= MIN_GAP_PX;
  if (!beats && gap * 4 < MIN_GAP_PX) return null;
  const from = Math.max(0, visible[0]);
  const to = Math.min(width, visible[1]);
  let beat = "";
  let bar = "";
  song.beatsUs.forEach((us, i) => {
    const at = x(us);
    if (at < from || at > to) return;
    const isBar = i >= song.firstBar && (i - song.firstBar) % 4 === 0;
    const px = Math.round(at) + 0.5;
    if (isBar) bar += `M${px} 0V10`;
    else if (beats) beat += `M${px} 0V5`;
  });
  const drops = song.drops.map((d) => ({ at: x(d.atUs), label: d.beatStarts ? "Drop · beat starts" : "Drop" })).filter((d) => d.at >= 0 && d.at <= width);
  return (
    <>
      <svg className="pointer-events-none absolute left-0 top-0" width={width} height={height} aria-hidden data-beat-ticks>
        {beat && <path d={beat} strokeWidth={1} className="stroke-white/60" />}
        {bar && <path d={bar} strokeWidth={1.5} className="stroke-white/90" />}
        {drops.map((d) => (
          <path key={d.at} d={`M${Math.round(d.at) + 0.5} 0V${height}`} strokeWidth={1} className="stroke-white/90" />
        ))}
      </svg>
      {drops.map(
        (d) =>
          d.at + 4 < width && (
            <span
              key={d.at}
              className="pointer-events-none absolute top-[3px] flex h-[17px] max-w-[calc(100%-8px)] items-center gap-1 rounded-[5px] bg-black/80 pl-[5px] pr-1.5 text-[11px] font-medium whitespace-nowrap text-fg"
              style={{ left: d.at + 3 }}
              data-beat-drop
            >
              <Flag size={11} className="shrink-0" />
              <span className="truncate">{d.label}</span>
            </span>
          ),
      )}
    </>
  );
});

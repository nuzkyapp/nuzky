import { useEffect, useRef, useState } from "react";
import { PEAKS_PER_BLOCK, PEAKS_PER_SECOND, useEditor, type WaveBlock } from "../../lib/store";
import { US } from "../../lib/time";

/** The widest canvas drawn; the timeline draws only its window of a clip. */
const MAX_WIDTH = 8192;
/** Bars are 2 px wide every 3 px of the clip, counted from its start, so they stay put while the timeline scrolls. */
const BAR = 2;
const PITCH = 3;
/** The dB under full scale a bar spans; quieter sound draws the 1 px floor, so pauses read as pauses. */
const RANGE_DB = 42;

/** A peak, or undefined while it is not loaded. Past the end of a finished block the sound has ended. */
function peakAt(blocks: Record<number, WaveBlock> | undefined, i: number): number | undefined {
  const block = blocks?.[Math.floor(i / PEAKS_PER_BLOCK)];
  if (!block) return undefined;
  const j = i % PEAKS_PER_BLOCK;
  return j < block.peaks.length ? block.peaks[j] : block.complete ? 0 : undefined;
}

/**
 * The peak a bar over peaks `a..b` stands for, or undefined while one of them is not loaded. Blending keeps the first
 * peak alone while the next one is missing, so a bar at the end of the sound still draws.
 */
function barPeak(blocks: Record<number, WaveBlock> | undefined, a: number, b: number): number | undefined {
  if (b - a < 1) {
    // Zoomed in past a peak per bar: blend the two peaks around the bar's middle, so bars do not repeat one height in steps.
    const x = Math.max(0, (a + b) / 2 - 0.5);
    const i = Math.floor(x);
    const p = peakAt(blocks, i);
    if (p === undefined) return undefined;
    const q = peakAt(blocks, i + 1) ?? p;
    return p + (q - p) * (x - i);
  }
  let m = 0;
  for (let i = Math.floor(a); i < Math.floor(b); i++) {
    const peak = peakAt(blocks, i);
    if (peak === undefined) return undefined;
    m = Math.max(m, peak);
  }
  return m;
}

/**
 * Peak bars rising from the bottom edge, for the source range a clip plays; `speed` stretches it like the renderer and
 * `gain`, the clip's volume, raises or lowers the bars by as many dB as it changes the sound, up to full scale.
 * Only `visible` (px of the clip) is drawn, at the screen's pixel density, from the blocks of peaks under it and one
 * block around; bars whose peaks are not loaded yet stay empty. Without `visible` it fits its container.
 */
export function Waveform({
  assetId,
  sourceInUs,
  durationUs,
  speed,
  gain = 1,
  width,
  visible,
  color,
  className = "absolute top-px h-[calc(100%-2px)]",
}: {
  assetId: string;
  sourceInUs: number;
  durationUs: number;
  speed: number;
  gain?: number;
  width: number;
  visible?: [number, number];
  color: string;
  className?: string;
}) {
  const blocks = useEditor((s) => s.waveforms[assetId]);
  const ref = useRef<HTMLCanvasElement>(null);
  const epoch = useEditor((s) => s.snap?.sessionEpoch);
  const fits = !visible;
  const [fitted, setFitted] = useState(0);
  const [dpr, setDpr] = useState(window.devicePixelRatio || 1);
  const across = fits ? fitted || width : width;
  const first = (sourceInUs / US) * PEAKS_PER_SECOND;
  const span = ((durationUs * speed) / US) * PEAKS_PER_SECOND;
  const from = Math.max(0, Math.floor(visible?.[0] ?? 0));
  const to = Math.min(Math.ceil(across), Math.ceil(visible?.[1] ?? across), from + MAX_WIDTH);
  const shown = to > from && across > 0;
  const firstBlock = Math.max(0, Math.floor((first + (from / across) * span) / PEAKS_PER_BLOCK) - 1);
  const lastBlock = Math.floor((first + (to / across) * span) / PEAKS_PER_BLOCK) + 1;
  useEffect(() => (shown ? useEditor.getState().showWaveform(assetId, firstBlock, lastBlock) : undefined), [assetId, firstBlock, lastBlock, shown, epoch]);
  useEffect(() => {
    const el = ref.current;
    if (!fits || !el) return;
    const observer = new ResizeObserver(() => setFitted(el.clientWidth));
    observer.observe(el);
    return () => observer.disconnect();
  }, [fits, shown]);
  // Moving the window to a screen of another pixel density draws the bars again, sharp there.
  useEffect(() => {
    const density = matchMedia(`(resolution: ${dpr}dppx)`);
    const changed = () => setDpr(window.devicePixelRatio || 1);
    density.addEventListener("change", changed);
    return () => density.removeEventListener("change", changed);
  }, [dpr]);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.width = Math.round((to - from) * dpr);
    el.height = Math.round((el.clientHeight || 24) * dpr);
    const h = el.height;
    const ctx = el.getContext("2d")!;
    ctx.fillStyle = color;
    // Bars never overlap, so alpha here looks the same as CSS opacity and spares a transparency layer per canvas.
    ctx.globalAlpha = 0.95;
    for (let x = Math.ceil(from / PITCH) * PITCH; x < to; x += PITCH) {
      const peak = barPeak(blocks, first + (x / across) * span, first + (Math.min(x + PITCH, across) / across) * span);
      if (peak === undefined) continue;
      const level = peak > 0 && gain > 0 ? Math.min(1, Math.max(0, 1 + (20 * Math.log10((peak * gain) / 255)) / RANGE_DB)) : 0;
      const bar = Math.max(Math.round(dpr), Math.round(level * h));
      const left = Math.round((x - from) * dpr);
      ctx.fillRect(left, h - bar, Math.round((x + BAR - from) * dpr) - left, bar);
    }
  }, [blocks, from, to, first, span, gain, across, color, dpr]);
  if (!shown) return null;
  return <canvas ref={ref} className={`pointer-events-none ${fits ? "w-full" : ""} ${className}`} style={visible && { left: from, width: to - from }} />;
}

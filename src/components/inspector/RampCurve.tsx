import { useRef, type KeyboardEvent, type PointerEvent } from "react";
import { followPointer } from "../../lib/drag";
import { LIMITS } from "../../lib/limits";
import { rampSpeed } from "../../lib/retime";
import type { SpeedPoint } from "../../lib/types";
import { useLockReason } from "../ui";

const H = 112;
/** Room above 10x and below 0.1x, so a point there stays whole. */
const PAD = 8;
/** The closest two points may come, as a share of the clip. */
const GAP = 0.02;
const MAX_POINTS = 16;
const LOG_MIN = Math.log10(LIMITS.minSpeed);
const LOG_SPAN = Math.log10(LIMITS.maxSpeed) - LOG_MIN;

const toY = (speed: number) => PAD + (1 - (Math.log10(speed) - LOG_MIN) / LOG_SPAN) * (H - 2 * PAD);
const fromY = (y: number) => 10 ** (LOG_MIN + (1 - (y - PAD) / (H - 2 * PAD)) * LOG_SPAN);
export const roundSpeed = (s: number) => Math.round(Math.min(LIMITS.maxSpeed, Math.max(LIMITS.minSpeed, s)) * 100) / 100;
const roundAt = (at: number) => Math.round(at * 1000) / 1000;
const speedLabel = (s: number) => `${Number(s.toFixed(2))}x`;

/** Point `i` moved to `at` and `speed`; the first and last stay at the clip's edges, the others between their neighbours. */
export function movePoint(points: SpeedPoint[], i: number, at: number, speed: number): SpeedPoint[] {
  const last = points.length - 1;
  const lo = i === 0 ? 0 : points[i - 1].at + GAP;
  const hi = i === last ? 1 : points[i + 1].at - GAP;
  const placed = i === 0 ? 0 : i === last ? 1 : roundAt(Math.min(hi, Math.max(lo, at)));
  return points.map((p, j) => (j === i ? { at: placed, speed: roundSpeed(speed) } : p));
}

/**
 * The speed of a ramp along the clip on a logarithmic scale from 0.1x to 10x, its points draggable: double-click adds
 * one, Delete removes the chosen one. By keyboard Left and Right choose a point, Up and Down change its speed, Shift
 * with Left or Right moves it along the clip and Enter adds one after it.
 */
export function RampCurve({
  points,
  selected,
  onSelect,
  onChange,
}: {
  points: SpeedPoint[];
  selected: number;
  onSelect: (i: number) => void;
  /** `key` joins the changes of one drag into one undo step. */
  onChange: (points: SpeedPoint[], key: string) => void;
}) {
  const lock = useLockReason();
  const box = useRef<HTMLDivElement>(null);
  const plot = useRef<HTMLDivElement>(null);
  const chosen = Math.min(selected, points.length - 1);
  const add = (at: number) => {
    if (points.length >= MAX_POINTS || points.some((p) => Math.abs(p.at - at) < GAP)) return;
    const next = [...points, { at: roundAt(at), speed: roundSpeed(rampSpeed(points, at)) }].sort((a, b) => a.at - b.at);
    onSelect(next.findIndex((p) => p.at === roundAt(at)));
    onChange(next, `ramp-add:${Date.now()}`);
  };
  const remove = (i: number) => {
    if (i === 0 || i === points.length - 1) return;
    onSelect(i - 1);
    onChange(points.filter((_, j) => j !== i), `ramp-remove:${Date.now()}`);
  };
  const startDrag = (e: PointerEvent, i: number) => {
    if (lock || e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    box.current?.focus();
    onSelect(i);
    const rect = plot.current!.getBoundingClientRect();
    const start = points;
    const key = `ramp-drag:${Date.now()}`;
    // Once per display frame: each change retimes the clip in the engine.
    let frame = 0;
    let latest: globalThis.PointerEvent | null = null;
    const apply = () => {
      frame = 0;
      if (latest) onChange(movePoint(start, i, (latest.clientX - rect.left) / rect.width, fromY(latest.clientY - rect.top)), key);
    };
    followPointer({
      move: (m) => {
        latest = m;
        frame ||= requestAnimationFrame(apply);
      },
      up: (m) => {
        cancelAnimationFrame(frame);
        if (latest) {
          latest = m;
          apply();
        }
      },
      cancel: () => cancelAnimationFrame(frame),
    });
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (lock) return;
    const p = points[chosen];
    const key = `ramp-key:${chosen}:${e.key}`;
    if ((e.key === "ArrowLeft" || e.key === "ArrowRight") && !e.shiftKey) {
      onSelect(Math.max(0, Math.min(points.length - 1, chosen + (e.key === "ArrowLeft" ? -1 : 1))));
    } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      onChange(movePoint(points, chosen, p.at + (e.key === "ArrowLeft" ? -0.01 : 0.01), p.speed), key);
    } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      onChange(movePoint(points, chosen, p.at, p.speed * (e.key === "ArrowUp" ? 1.05 : 1 / 1.05)), key);
    } else if (e.key === "Delete" || e.key === "Backspace") {
      remove(chosen);
    } else if (e.key === "Enter") {
      const next = points[chosen + 1] ?? points[chosen - 1];
      add((p.at + next.at) / 2);
    } else {
      return;
    }
    e.preventDefault();
    e.stopPropagation();
  };
  const line = Array.from({ length: 201 }, (_, k) => k / 200)
    .concat(points.map((p) => p.at))
    .sort((a, b) => a - b)
    .map((x) => `${(x * 1000).toFixed(1)},${toY(rampSpeed(points, x)).toFixed(1)}`)
    .join(" ");
  const left = (at: number) => `${at * 100}%`;
  const sel = points[chosen];
  const position = chosen === 0 ? "start" : chosen === points.length - 1 ? "end" : `${Math.round(sel.at * 100)} %`;
  return (
    <div
      ref={box}
      role="group"
      aria-label={`Speed ramp, ${points.length} points; point ${chosen + 1} at ${position}, ${speedLabel(sel.speed)}`}
      aria-disabled={lock ? true : undefined}
      tabIndex={lock ? -1 : 0}
      title={lock ?? "Drag a point; double-click adds one, Delete removes it"}
      onKeyDown={onKeyDown}
      onDoubleClick={(e) => {
        if (lock || (e.target as HTMLElement).dataset.rampPoint !== undefined || !plot.current) return;
        const rect = plot.current.getBoundingClientRect();
        add(Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width)));
      }}
      data-ramp-curve
      className={`relative h-[112px] touch-none select-none rounded-[8px] bg-white/[.035] outline-none focus-visible:outline-2 focus-visible:outline-accent ${lock ? "opacity-40" : ""}`}
    >
      {[LIMITS.maxSpeed, 1, LIMITS.minSpeed].map((s) => (
        <span key={s} aria-hidden className="tabular pointer-events-none absolute left-1.5 -translate-y-1/2 text-[11px] leading-none text-muted" style={{ top: toY(s) }}>
          {speedLabel(s)}
        </span>
      ))}
      {/* The plot leaves the scale its own column, so the first point never covers a label. */}
      <div ref={plot} data-ramp-plot className="absolute inset-y-0 left-8 right-2">
        <svg aria-hidden className="pointer-events-none absolute inset-0 h-full w-full" viewBox={`0 0 1000 ${H}`} preserveAspectRatio="none">
          {[LIMITS.maxSpeed, 1, LIMITS.minSpeed].map((s) => (
            <line key={s} x1={0} x2={1000} y1={toY(s)} y2={toY(s)} vectorEffect="non-scaling-stroke" strokeDasharray={s === 1 ? undefined : "2 3"} className={s === 1 ? "stroke-white/20" : "stroke-white/[.08]"} />
          ))}
          <polyline points={line} fill="none" vectorEffect="non-scaling-stroke" strokeWidth={2} strokeLinejoin="round" className="stroke-accent" />
        </svg>
        {points.map((p, i) => (
          // A 24 px target around a 10 px dot, so a point is easy to grab.
          <span
            key={i}
            aria-hidden
            data-ramp-point={i}
            onPointerDown={(e) => startDrag(e, i)}
            className={`absolute flex size-6 -translate-x-1/2 -translate-y-1/2 items-center justify-center ${i === chosen ? "z-10" : ""} ${lock ? "" : "cursor-grab active:cursor-grabbing"}`}
            style={{ left: left(p.at), top: toY(p.speed) }}
          >
            <span className={`pointer-events-none rounded-full ${i === chosen ? "size-3 border-2 border-white bg-accent" : "size-2.5 border-[1.5px] border-black/60 bg-fg"}`} />
          </span>
        ))}
        <span
          aria-hidden
          className="tabular pointer-events-none absolute z-10 -translate-x-1/2 rounded-[4px] bg-black/70 px-1 text-[11px] font-semibold leading-[17px] text-white"
          style={{ left: `clamp(22px, ${sel.at * 100}%, calc(100% - 22px))`, top: toY(sel.speed) + 26 > H ? toY(sel.speed) - 27 : toY(sel.speed) + 9 }}
        >
          {speedLabel(sel.speed)}
        </span>
      </div>
    </div>
  );
}

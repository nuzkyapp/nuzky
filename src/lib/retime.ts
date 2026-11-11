import type { ClipContent, SpeedPoint } from "./types";

/** What of a media clip says where it plays its file. */
export type Timing = Pick<Extract<ClipContent, { type: "media" }>, "sourceInUs" | "speed" | "reverse" | "ramp">;

/**
 * Where in its file a media clip plays at each offset from its start, as `crates/engine/src/retime.rs` works it out:
 * its speed, a speed ramp (the speed moves evenly between points, held before the first and after the last, scaled so
 * its average is `speed`) and playing backwards. Offsets and times are microseconds; offsets may lie outside the clip.
 */
export interface Retime {
  /** Source time at `offset`. */
  sourceAt(offset: number): number;
  /** The offset at which source time `source` plays. */
  offsetOf(source: number): number;
  reverse: boolean;
  retimed: boolean;
}

export function isRetimed(c: Timing): boolean {
  return !!c.reverse || (c.ramp?.length ?? 0) > 0;
}

export function retime(c: Timing, durationUs: number): Retime {
  const ramp = c.ramp ?? [];
  const reverse = !!c.reverse;
  const speed = c.speed;
  const d = Math.max(1, durationUs);
  const end = c.sourceInUs + d * speed;
  const mean = ramp.length ? area(ramp, 1) - area(ramp, 0) : 1;
  const travel = (u: number) => (ramp.length ? (d * speed * (area(ramp, u / d) - area(ramp, 0))) / mean : u * speed);
  const offsetFor = (dist: number) => (ramp.length ? areaInverse(ramp, (dist * mean) / (d * speed) + area(ramp, 0)) * d : dist / speed);
  return {
    reverse,
    retimed: reverse || ramp.length > 0,
    sourceAt: (u) => (reverse ? end - travel(u) : c.sourceInUs + travel(u)),
    offsetOf: (s) => offsetFor(reverse ? end - s : s - c.sourceInUs),
  };
}

/** The ramp's speed at share `x` of the clip. */
export function rampSpeed(points: SpeedPoint[], x: number): number {
  const first = points[0];
  const last = points[points.length - 1];
  if (x <= first.at) return first.speed;
  if (x >= last.at) return last.speed;
  let i = 0;
  while (points[i + 1].at <= x) i++;
  const p = points[i];
  const q = points[i + 1];
  return p.speed + ((q.speed - p.speed) * (x - p.at)) / (q.at - p.at);
}

/** The average speed of a ramp from 0 to 1: the clip's speed with it. */
export function rampMean(points: SpeedPoint[]): number {
  return area(points, 1) - area(points, 0);
}

function area(points: SpeedPoint[], x: number): number {
  const first = points[0];
  if (x <= first.at) return first.speed * (x - first.at);
  let sum = 0;
  for (let i = 0; i + 1 < points.length; i++) {
    const p = points[i];
    const q = points[i + 1];
    if (x <= q.at) {
      const w = x - p.at;
      return sum + p.speed * w + (((q.speed - p.speed) / (q.at - p.at)) * w * w) / 2;
    }
    sum += ((p.speed + q.speed) / 2) * (q.at - p.at);
  }
  const last = points[points.length - 1];
  return sum + last.speed * (x - last.at);
}

function areaInverse(points: SpeedPoint[], target: number): number {
  const first = points[0];
  if (target <= 0) return first.at + target / first.speed;
  let sum = 0;
  for (let i = 0; i + 1 < points.length; i++) {
    const p = points[i];
    const q = points[i + 1];
    const piece = ((p.speed + q.speed) / 2) * (q.at - p.at);
    if (target <= sum + piece) {
      const rest = target - sum;
      const slope = (q.speed - p.speed) / (q.at - p.at);
      return p.at + (2 * rest) / (p.speed + Math.sqrt(Math.max(0, p.speed * p.speed + 2 * slope * rest)));
    }
    sum += piece;
  }
  const last = points[points.length - 1];
  return last.at + (target - sum) / last.speed;
}

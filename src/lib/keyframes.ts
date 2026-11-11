import { LIMITS } from "./limits";
import { NO_CROP } from "./presets";
import type { Canvas, Clip, Ease, Keyframe, Mask, MotionKind, Transform } from "./types";
import { US } from "./time";

const UNMOVED: Transform = { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1 };

/** The clip's own transform, without keyframes; an adjustment layer covers the whole picture unmoved. */
export function ownTransform(clip: Clip): Transform {
  return clip.content.type === "adjustment" ? UNMOVED : clip.content.transform;
}

/** Keyframes closer than half a frame to the playhead count as "at the playhead". */
export function keyframeTolerance(fps: number) {
  return US / fps / 2;
}

/** Share of the way to the next keyframe at `p` of the time between them, as the engine's `Ease::at`. */
export function eased(ease: Ease, p: number) {
  switch (ease) {
    case "smooth":
      return p * p * (3 - 2 * p);
    case "easeIn":
      return p * p * p;
    case "easeOut":
      return 1 - (1 - p) ** 3;
    default:
      return p;
  }
}

/**
 * The keyframe whose curve leads from the playhead's place onwards: the one at the playhead, else the one before it,
 * when another keyframe follows; -1 when the playhead is past the last one or before the first.
 */
export function curveIndex(clip: Clip, offsetUs: number, tol: number) {
  const at = keyframeIndexAt(clip, offsetUs, tol);
  const i = at >= 0 ? at : clip.keyframes.findLastIndex((k) => k.tUs < offsetUs);
  return i >= 0 && i < clip.keyframes.length - 1 ? i : -1;
}

const MASK_VALUES = ["x", "y", "width", "height", "rotation", "feather"] as const;

/** The mask between two keyframes, as the engine's `keyframed`: the shape and its inversion hold until the next one. */
function lerpMask(a: Mask | null | undefined, b: Mask | null | undefined, m: (x: number, y: number) => number): Mask | null | undefined {
  if (!a || !b || a.kind !== b.kind || !a.invert !== !b.invert) return a;
  return { ...a, ...Object.fromEntries(MASK_VALUES.map((k) => [k, m(a[k], b[k])])) };
}

function lerp(a: Transform, b: Transform, f: number): Transform {
  const m = (x: number, y: number) => x + (y - x) * f;
  const t: Transform = { x: m(a.x, b.x), y: m(a.y, b.y), scale: m(a.scale, b.scale), rotation: m(a.rotation, b.rotation), opacity: m(a.opacity, b.opacity) };
  const mask = lerpMask(a.mask, b.mask, m);
  if (mask) t.mask = mask;
  // A keyframe without a crop shows the whole layer.
  if (!a.crop && !b.crop) return t;
  const [ca, cb] = [a.crop ?? NO_CROP, b.crop ?? NO_CROP];
  return { ...t, crop: { left: m(ca.left, cb.left), top: m(ca.top, cb.top), right: m(ca.right, cb.right), bottom: m(ca.bottom, cb.bottom) } };
}

/** Same rule as the renderer: along each keyframe's ease to the next, nearest one holds outside them. */
export function transformAt(clip: Clip, base: Transform, offsetUs: number): Transform {
  const ks = clip.keyframes;
  if (ks.length === 0) return base;
  if (offsetUs <= ks[0].tUs) return ks[0].transform;
  const last = ks[ks.length - 1];
  if (offsetUs >= last.tUs) return last.transform;
  const i = ks.findIndex((k) => k.tUs > offsetUs);
  const a = ks[i - 1];
  const b = ks[i];
  return lerp(a.transform, b.transform, eased(a.ease, (offsetUs - a.tUs) / (b.tUs - a.tUs)));
}

/** Playhead position inside the clip, clamped to its length. */
export function clipOffset(clip: Clip, timeUs: number) {
  return Math.round(Math.max(0, Math.min(clip.durationUs, timeUs - clip.startUs)));
}

export function keyframeIndexAt(clip: Clip, offsetUs: number, tol: number) {
  return clip.keyframes.findIndex((k) => Math.abs(k.tUs - offsetUs) <= tol);
}

/**
 * Replaces the keyframe at `offsetUs` or inserts one, keeping the list sorted. A new keyframe
 * takes the curve of the one before it, so a smooth move stays smooth.
 */
export function upsertKeyframe(clip: Clip, offsetUs: number, transform: Transform, tol: number): Keyframe[] {
  const i = keyframeIndexAt(clip, offsetUs, tol);
  if (i >= 0) return clip.keyframes.map((k, j) => (j === i ? { ...k, transform } : k));
  const before = clip.keyframes.findLast((k) => k.tUs < offsetUs) ?? clip.keyframes[0];
  return [...clip.keyframes, { tUs: offsetUs, transform, ease: before?.ease ?? "linear" }].sort((a, b) => a.tUs - b.tUs);
}

/** The canvas point a motion preset zooms about, as the engine's `motion_centre`. */
function motionCentre(canvas: Canvas, kind: MotionKind): [number, number] {
  const { width: w, height: h } = canvas;
  const { left, top, right, bottom } = canvas.safeArea ?? { left: 0, top: 0, right: w, bottom: h };
  return [(kind === "kenBurns" ? right : (left + right) / 2) / w - 0.5, (top + bottom) / 2 / h - 0.5];
}

const near = (a: number, b: number) => Math.abs(a - b) < 1e-4;
const sameMask = (a: Mask | null | undefined, b: Mask | null | undefined) =>
  !a || !b ? !a === !b : a.kind === b.kind && !a.invert === !b.invert && MASK_VALUES.every((k) => near(a[k], b[k]));
/** The same transform, crop and mask included, so a keyframe edited by hand never passes for a preset. */
const close = (a: Transform, b: Transform) =>
  (["x", "y", "scale", "rotation", "opacity"] as const).every((k) => near(a[k], b[k])) &&
  !a.crop === !b.crop &&
  (!a.crop || !b.crop || (["left", "top", "right", "bottom"] as const).every((k) => near(a.crop![k], b.crop![k]))) &&
  sameMask(a.mask, b.mask);

/**
 * The motion preset the clip's keyframes are, with its strength, or null for none or keyframes of
 * the user's own: two smooth keyframes from the clip's transform, zoomed about the preset's centre
 * by a strength the engine accepts.
 */
export function motionOf(clip: Clip, canvas: Canvas): { kind: MotionKind; strength: number } | null {
  const ks = clip.keyframes;
  const base = ownTransform(clip);
  if (ks.length !== 2 || ks.some((k) => k.ease !== "smooth")) return null;
  for (const kind of ["pushIn", "pullOut", "kenBurns"] as const) {
    const [still, zoomed] = kind === "pullOut" ? [ks[1], ks[0]] : [ks[0], ks[1]];
    const s = zoomed.transform.scale / base.scale;
    const [fx, fy] = motionCentre(canvas, kind);
    const expected = { ...base, x: base.x + (s - 1) * (base.x - fx), y: base.y + (s - 1) * (base.y - fy), scale: base.scale * s };
    const strength = Math.round((s - 1) * 1000) / 1000;
    const valid = strength >= LIMITS.minMotionStrength && strength <= LIMITS.maxMotionStrength;
    if (valid && close(still.transform, base) && close(zoomed.transform, expected)) return { kind, strength };
  }
  return null;
}

/** A transform's crop and mask, which have sections of their own, so Reset transform keeps them. */
export const cropAndMask = ({ crop, mask }: Transform): Pick<Transform, "crop" | "mask"> => ({ crop, mask });

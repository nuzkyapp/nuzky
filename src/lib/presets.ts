import captionStyles from "../../assets/presets/captions.json";
import textStyles from "../../assets/presets/texts.json";
import type { Adjust, Animation, AnimationKind, CaptionPreset, Crop, KeywordPick, Shape, SplitLayout, TextPreset, TextShadow, TextStyle, Transform, TransitionKind } from "./types";

/** Canvas formats at 1080p; `canvasSize` gives them at another resolution. */
export const FORMATS = [
  { label: "9:16", hint: "Reels, TikTok, Shorts", width: 1080, height: 1920 },
  { label: "16:9", hint: "YouTube", width: 1920, height: 1080 },
  { label: "1:1", hint: "Square", width: 1080, height: 1080 },
  { label: "4:5", hint: "Instagram feed", width: 1080, height: 1350 },
];

/** The shorter side of the canvas presets are made for. */
const REFERENCE_SIDE = 1080;

/** Canvas pixels per pixel of a preset, as the engine's `Canvas::unit`: 1 on 1080p, 2 on 4K. */
export const canvasUnit = (canvas: { width: number; height: number }) => Math.min(canvas.width, canvas.height) / REFERENCE_SIDE;

/** A canvas or format of the same shape at the resolution with this shorter side, in even pixels as the engine keeps it. */
export function canvasSize(size: { width: number; height: number }, short: number) {
  const even = (v: number) => 2 * Math.round((v * short) / Math.min(size.width, size.height) / 2);
  return { width: even(size.width), height: even(size.height) };
}

export const sameRatio = (a: { width: number; height: number }, b: { width: number; height: number }) => a.width * b.height === a.height * b.width;

export function formatLabel(width: number, height: number) {
  return FORMATS.find((f) => sameRatio(f, { width, height }))?.label ?? `${width}×${height}`;
}

/** A style's sizes in canvas pixels times `k`: a preset's on a canvas `k` times 1080p, or back with 1 / `k`. */
export function scaleStyle(style: TextStyle, k: number): TextStyle {
  if (k === 1) return style;
  return { ...style, fontSize: style.fontSize * k, strokeWidth: style.strokeWidth * k, maxWidth: style.maxWidth == null ? style.maxWidth : style.maxWidth * k };
}

export const DEFAULT_TRANSFORM: Transform = { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1 };
export const NO_CROP: Crop = { left: 0, top: 0, right: 0, bottom: 0 };
export const NO_SHAPE: Shape = { radius: 0, borderWidth: 0, borderColor: "#ffffff", shadow: 0 };
/** A split screen's layouts as the engine's `SplitLayout::cells` lays them out: each cell's left, top, right and bottom edge in 0..1 of the frame. */
export const SPLIT_LAYOUTS: { layout: SplitLayout; label: string; cells: [number, number, number, number][] }[] = [
  { layout: "sideBySide", label: "Side by side", cells: [[0, 0, 0.5, 1], [0.5, 0, 1, 1]] },
  { layout: "topAndBottom", label: "Stacked", cells: [[0, 0, 1, 0.5], [0, 0.5, 1, 1]] },
  { layout: "threeColumns", label: "Columns", cells: [[0, 0, 1 / 3, 1], [1 / 3, 0, 2 / 3, 1], [2 / 3, 0, 1, 1]] },
  { layout: "threeRows", label: "Rows", cells: [[0, 0, 1, 1 / 3], [0, 1 / 3, 1, 2 / 3], [0, 2 / 3, 1, 1]] },
  { layout: "oneOverTwo", label: "One over two", cells: [[0, 0, 1, 0.5], [0, 0.5, 0.5, 1], [0.5, 0.5, 1, 1]] },
  { layout: "grid", label: "Grid", cells: [[0, 0, 0.5, 0.5], [0.5, 0, 1, 0.5], [0, 0.5, 0.5, 1], [0.5, 0.5, 1, 1]] },
];

export const NO_ADJUST: Adjust = { exposure: 0, tint: 0, highlights: 0, shadows: 0, fade: 0, brightness: 0, contrast: 0, saturation: 0, temperature: 0, vignette: 0 };

export const ADJUST_ROWS: { key: keyof Adjust; label: string; min: number }[] = [
  { key: "exposure", label: "Exposure", min: -100 },
  { key: "brightness", label: "Brightness", min: -100 },
  { key: "contrast", label: "Contrast", min: -100 },
  { key: "highlights", label: "Highlights", min: -100 },
  { key: "shadows", label: "Shadows", min: -100 },
  { key: "saturation", label: "Saturation", min: -100 },
  { key: "temperature", label: "Temperature", min: -100 },
  { key: "tint", label: "Tint", min: -100 },
  { key: "fade", label: "Fade", min: 0 },
  { key: "vignette", label: "Vignette", min: 0 },
];

/** Styles for titles and other text, shared with the engine and MCP (assets/presets/texts.json), Classic first. */
export const TEXT_PRESETS = textStyles as TextPreset[];

/** The Text tab's groups of text styles, in the order the file lists them. */
export const TEXT_GROUPS = [...new Set(TEXT_PRESETS.map((p) => p.group))].map((name) => ({ name, presets: TEXT_PRESETS.filter((p) => p.group === name) }));

/**
 * Caption styles, shared with the engine and MCP (assets/presets/captions.json); Reel comes first. A style
 * without a font keeps the font the captions have, so a font picked separately survives it.
 */
export const CAPTION_STYLES = captionStyles as CaptionPreset[];

const lower = (c: string | null | undefined) => (c ?? "").toLowerCase();

/** The weight a style draws when its family has every weight. */
export const weightOf = (style: TextStyle) => style.fontWeight ?? (style.bold ? 700 : 400);

/** The background box's corner radius, as a share of the font size, of a text that never changed it. */
export const BOX_RADIUS = 0.25;

/**
 * Whether `b` has the look of preset `a`, whatever font it has and, when `a` sets none, its case; a karaoke
 * highlight and key words count, so Karaoke is not Reel. `b` is on a canvas `unit` times 1080p.
 */
export function sameStyle(a: TextStyle, b: TextStyle, unit = 1) {
  const near = (preset: number, value: number) => Math.abs(preset * unit - value) < 0.01;
  return (
    near(a.fontSize, b.fontSize) &&
    lower(a.color) === lower(b.color) &&
    weightOf(a) === weightOf(b) &&
    near(a.strokeWidth, b.strokeWidth) &&
    lower(a.strokeColor) === lower(b.strokeColor) &&
    lower(a.background) === lower(b.background) &&
    lower(a.highlight) === lower(b.highlight) &&
    lower(a.keywords?.color) === lower(b.keywords?.color) &&
    (a.keywords?.pick ?? null) === (b.keywords?.pick ?? null) &&
    (!a.letterCase || a.letterCase === (b.letterCase ?? "none")) &&
    (a.align ?? "center") === (b.align ?? "center") &&
    (a.boxRadius ?? BOX_RADIUS) === (b.boxRadius ?? BOX_RADIUS) &&
    (a.letterSpacing ?? 0) === (b.letterSpacing ?? 0) &&
    (a.lineSpacing ?? 0) === (b.lineSpacing ?? 0) &&
    sameShadow(a.shadow, b.shadow)
  );
}

function sameShadow(a: TextShadow | null | undefined, b: TextShadow | null | undefined) {
  if (!a || !b) return !a && !b;
  return lower(a.color) === lower(b.color) && a.opacity === b.opacity && a.blur === b.blur && a.distance === b.distance && a.angle === b.angle;
}

// By kind only: the engine shortens an animation to a short caption.
const sameAnimation = (a: Animation | null | undefined, b: Animation | null | undefined) => (a?.kind ?? null) === (b?.kind ?? null);

/** Whether captions in `style` with these animations look like `preset`: its font counts only when it brings one. */
export function isLook(preset: CaptionPreset, style: TextStyle, animIn: Animation | null | undefined, animOut: Animation | null | undefined, unit = 1) {
  return (
    sameStyle(preset.style, style, unit) &&
    (!preset.style.fontFamily || preset.style.fontFamily === style.fontFamily) &&
    sameAnimation(preset.animIn, animIn) &&
    sameAnimation(preset.animOut, animOut)
  );
}

/** Which words of a caption the key word colour marks. */
export const KEYWORD_PICKS: { id: KeywordPick | "off"; label: string; title: string }[] = [
  { id: "off", label: "Manual", title: "Only the words you mark" },
  { id: "emphasis", label: "Emphasis", title: "Numbers and words said louder than the rest" },
  { id: "first", label: "First", title: "The first word of every caption" },
  { id: "last", label: "Last", title: "The last word of every caption" },
  { id: "longest", label: "Longest", title: "The longest word of every caption" },
];
export const KEYWORD_COLOR = "#ffd400";

/** Typewriter only makes sense for text; the engine treats it as Fade on media. */
export const ANIMATIONS: { kind: AnimationKind; label: string; textOnly?: boolean }[] = [
  { kind: "fade", label: "Fade" },
  { kind: "zoomIn", label: "Zoom in" },
  { kind: "zoomOut", label: "Zoom out" },
  { kind: "slideUp", label: "Slide up" },
  { kind: "slideDown", label: "Slide down" },
  { kind: "slideLeft", label: "Slide left" },
  { kind: "slideRight", label: "Slide right" },
  { kind: "pop", label: "Pop" },
  { kind: "typewriter", label: "Typewriter", textOnly: true },
];

export const TRANSITIONS: { kind: TransitionKind; label: string }[] = [
  { kind: "dissolve", label: "Dissolve" },
  { kind: "fadeBlack", label: "Fade to black" },
  { kind: "fadeWhite", label: "Flash white" },
  { kind: "slideLeft", label: "Slide left" },
  { kind: "slideRight", label: "Slide right" },
  { kind: "slideUp", label: "Slide up" },
  { kind: "slideDown", label: "Slide down" },
  { kind: "wipeLeft", label: "Wipe left" },
  { kind: "wipeRight", label: "Wipe right" },
  { kind: "wipeUp", label: "Wipe up" },
  { kind: "wipeDown", label: "Wipe down" },
  { kind: "zoomIn", label: "Zoom in" },
  { kind: "zoomOut", label: "Zoom out" },
  { kind: "spin", label: "Spin" },
  { kind: "blur", label: "Blur" },
];

export const DEFAULT_TRANSITION_US = 500_000;

export const FILTERS: { id: string; label: string; adjust: Adjust }[] = [
  { id: "none", label: "None", adjust: NO_ADJUST },
  { id: "vivid", label: "Vivid", adjust: { ...NO_ADJUST, brightness: 0.03, contrast: 0.15, saturation: 0.35 } },
  { id: "warm", label: "Warm", adjust: { ...NO_ADJUST, saturation: 0.1, temperature: 0.35 } },
  { id: "cool", label: "Cool", adjust: { ...NO_ADJUST, brightness: 0.02, temperature: -0.35 } },
  { id: "mono", label: "Mono", adjust: { ...NO_ADJUST, contrast: 0.2, saturation: -1 } },
  { id: "fade", label: "Fade", adjust: { ...NO_ADJUST, fade: 0.6, contrast: -0.1, saturation: -0.2 } },
  { id: "moody", label: "Moody", adjust: { ...NO_ADJUST, shadows: -0.5, brightness: -0.05, contrast: 0.3, saturation: -0.2, temperature: -0.1, vignette: 0.45 } },
  { id: "punch", label: "Punch", adjust: { ...NO_ADJUST, contrast: 0.5, saturation: 0.1, vignette: 0.25 } },
];

export function sameAdjust(a: Adjust, b: Adjust) {
  return (Object.keys(NO_ADJUST) as (keyof Adjust)[]).every((k) => Math.abs(a[k] - b[k]) < 0.005);
}

/** Rough CSS approximation of an Adjust, for preset tiles only. The engine does the real grading. */
export function adjustCss(a: Adjust): { filter: string; tint: string | null; vignette: number } {
  const wb = [1.15 * a.temperature + 0.9 * a.tint, 0.15 * a.temperature - 0.9 * a.tint, -1.15 * a.temperature + 0.9 * a.tint].map((v) => 2 ** v);
  const y = 0.2126 * wb[0] + 0.7152 * wb[1] + 0.0722 * wb[2];
  const gain = wb.map((v) => (v / y) ** (1 / 2.4));
  const top = Math.max(...gain);
  const light = 2 ** (0.83 * a.exposure + 0.9 * a.brightness) * (1 + a.fade * 0.2) + a.highlights * 0.1 + a.shadows * 0.1;
  const contrast = ((1 + 0.55 * a.contrast + a.highlights * 0.1 - a.shadows * 0.1) * (1 - a.fade * 0.3)) / (1 + a.fade * 0.2);
  // Contrast before brightness: brightness above 1 would clip a bright sky before fade's contrast could bring it back.
  const filter = `contrast(${Math.max(0, contrast)}) brightness(${Math.max(0, light * top)}) saturate(${1 + a.saturation})`;
  const tint = a.temperature === 0 && a.tint === 0 ? null : `rgb(${gain.map((v) => Math.round((255 * v) / top)).join(",")})`;
  return { filter, tint, vignette: a.vignette };
}

export const SPEED_PRESETS = [0.5, 1, 1.5, 2, 3];

/** Shapes of a speed ramp, at the speeds they start from; the points' `at` runs from 0, the clip's start, to 1, its end. */
export const RAMP_PRESETS: { name: string; points: { at: number; speed: number }[] }[] = [
  { name: "Slow-mo", points: [{ at: 0, speed: 1 }, { at: 0.4, speed: 0.25 }, { at: 0.6, speed: 0.25 }, { at: 1, speed: 1 }] },
  { name: "Slow down", points: [{ at: 0, speed: 1.5 }, { at: 1, speed: 0.3 }] },
  { name: "Speed up", points: [{ at: 0, speed: 0.3 }, { at: 1, speed: 2 }] },
  { name: "Burst in", points: [{ at: 0, speed: 5 }, { at: 0.3, speed: 1 }, { at: 1, speed: 1 }] },
  { name: "Burst out", points: [{ at: 0, speed: 1 }, { at: 0.7, speed: 1 }, { at: 1, speed: 5 }] },
  { name: "Pulse", points: [{ at: 0, speed: 2 }, { at: 0.3, speed: 0.5 }, { at: 0.5, speed: 2 }, { at: 0.8, speed: 0.5 }, { at: 1, speed: 2 }] },
];

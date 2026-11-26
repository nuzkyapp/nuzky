import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import { usePreview } from "../../lib/preview";
import { useEditor } from "../../lib/store";
import type { Clip } from "../../lib/types";
import { useLockReason } from "../ui";

/** How long the pair stays after the last change. */
const LINGER_MS = 1000;
const GAP = 8;
const PAD = 8;
const MAX_TILE_H = 120;
const STEP_KEYS = new Set(["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End"]);

/** The preview's canvas, which shows the frame the engine drew last, with the time of that frame. */
const previewCanvas = () => document.querySelector<HTMLCanvasElement>("canvas[data-preview-frame]");

/** The last frame the canvas showed split by Compare; frames after it show the whole picture. */
let splitUntil = 0;
usePreview.subscribe((s, prev) => {
  if (prev.compare !== null && s.compare === null) splitUntil = Number(previewCanvas()?.dataset.n ?? 0);
});

/**
 * Whether the preview shows the paused frame at the playhead, as Before needs: not while playing, comparing (the
 * canvas is split then, and until the next frame after it), or before the frame of a new playhead position arrives.
 */
function still(source: HTMLCanvasElement) {
  const { playing, timeUs } = useEditor.getState();
  return !playing && usePreview.getState().compare === null && Number(source.dataset.n) > splitUntil && Number(source.dataset.us) === timeUs;
}

/**
 * While a slider among `children` is dragged or stepped with the keys, shows over it the preview as it was when the
 * change began (Before) beside the preview now (After), so the two can be told apart without looking away. Both are
 * copies of the preview's own frame, so the engine draws nothing more. It shows only while the playhead is over `clip`.
 */
export function PeekBeforeAfter({ clip, children }: { clip: Clip; children: ReactNode }) {
  const lock = useLockReason();
  const [peek, setPeek] = useState<{ row: HTMLElement; shot: HTMLCanvasElement; us: string } | null>(null);
  const before = useRef<HTMLCanvasElement>(null);
  const after = useRef<HTMLCanvasElement>(null);
  const hide = useRef(0);

  const linger = useCallback(() => {
    window.clearTimeout(hide.current);
    hide.current = window.setTimeout(() => setPeek(null), LINGER_MS);
  }, []);
  /** `fresh`: a new drag, which compares with the frame as it is now, while key steps in a row keep their Before. */
  const begin = (target: EventTarget, fresh: boolean) => {
    const el = target as HTMLElement;
    if (lock || !(el instanceof HTMLInputElement) || el.disabled) return;
    const row = el.closest<HTMLElement>("[data-peek-row]");
    const source = previewCanvas();
    const t = useEditor.getState().timeUs;
    if (!row || !source || source.width === 0 || t < clip.startUs || t >= clip.startUs + clip.durationUs || !still(source)) return;
    window.clearTimeout(hide.current);
    if (peek?.row === row && !fresh) return;
    // The frame before this change, at the preview's size, until the pair is drawn.
    const shot = document.createElement("canvas");
    [shot.width, shot.height] = [source.width, source.height];
    shot.getContext("2d")?.drawImage(source, 0, 0);
    setPeek({ row, shot, us: source.dataset.us ?? "" });
  };

  useEffect(() => () => window.clearTimeout(hide.current), []);
  useEffect(() => {
    if (!peek) return;
    window.addEventListener("pointerup", linger);
    const shown = before.current?.getContext("2d");
    shown?.drawImage(peek.shot, 0, 0, shown.canvas.width, shown.canvas.height);
    // After follows the preview, frame by frame, only while the pair shows.
    let raf = 0;
    const draw = () => {
      const source = previewCanvas();
      // Playing, comparing or another frame would set two different pictures side by side.
      if (!source || source.dataset.us !== peek.us || !still(source)) return setPeek(null);
      const ctx = after.current?.getContext("2d");
      ctx?.drawImage(source, 0, 0, ctx.canvas.width, ctx.canvas.height);
      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => {
      window.removeEventListener("pointerup", linger);
      cancelAnimationFrame(raf);
    };
  }, [peek, linger]);

  const onPointerDown = (e: PointerEvent) => e.button === 0 && begin(e.target, true);
  const onKeyDown = (e: KeyboardEvent) => {
    if (!STEP_KEYS.has(e.key)) return;
    begin(e.target, false);
    linger();
  };

  return (
    <div className="contents" onPointerDownCapture={onPointerDown} onKeyDownCapture={onKeyDown}>
      {children}
      {peek && !lock && createPortal(<Pair row={peek.row} aspect={peek.shot.width / peek.shot.height} before={before} after={after} />, document.body)}
    </div>
  );
}

function Pair({ row, aspect, before, after }: { row: HTMLElement; aspect: number; before: RefObject<HTMLCanvasElement | null>; after: RefObject<HTMLCanvasElement | null> }) {
  const r = row.getBoundingClientRect();
  // Room above is counted from the top of the inspector's scrolling part, so the pair never covers its tabs.
  const pane = (row.closest("[role=tabpanel]") ?? row.closest("aside"))?.getBoundingClientRect();
  // Over the slider and its value, clear of the label, as wide as they are.
  const left = r.left + 88;
  const width = Math.max(160, r.right - left);
  let w = (width - 2 * PAD - GAP) / 2;
  let h = w / aspect;
  if (h > MAX_TILE_H) [h, w] = [MAX_TILE_H, MAX_TILE_H * aspect];
  const height = h + 2 * PAD + 18;
  const above = r.top - height - 6 >= (pane?.top ?? 0) + 8;
  const top = above ? r.top - height - 6 : r.bottom + 6;
  const scale = window.devicePixelRatio || 1;
  const tile = (ref: RefObject<HTMLCanvasElement | null>, name: string) => (
    <figure className="m-0 flex flex-col items-center gap-1">
      <canvas ref={ref} width={Math.round(w * scale)} height={Math.round(h * scale)} style={{ width: w, height: h }} className="rounded-md" />
      <figcaption className="text-[11px] text-muted">{name}</figcaption>
    </figure>
  );
  return (
    <div aria-hidden data-peek className="overlay overlay-solid pointer-events-none fixed z-50 flex justify-center gap-2 rounded-xl" style={{ left, top, width, padding: PAD }}>
      {tile(before, "Before")}
      {tile(after, "After")}
    </div>
  );
}

import { useEffect, useRef, useState } from "react";
import { api } from "../../lib/api";
import { followPointer } from "../../lib/drag";
import { type Box, follow, stopPicking, useFollow } from "../../lib/follow";
import { findClip, useEditor } from "../../lib/store";
import { alongQuad } from "./LayerOverlay";
import { CHIP, ChipButton, JobChip } from "./ReframeStatus";

type Pt = [number, number];
const HANDLE = 10;
/** A press that moves less than this many screen pixels places the box there instead of drawing a new one. */
const CLICK = 8;
/** The smallest box side in screen pixels. */
const MIN_SIDE = 16;

/** The box in fractions of the clip's whole picture, whose quad on the canvas is `frame`, as the engine reads it. */
export function regionOf([x, y, w, h]: Box, frame: Pt[]): Box {
  const uv = ([[x, y], [x + w, y], [x + w, y + h], [x, y + h]] as Pt[]).map((p) => alongQuad(frame, p));
  const [u0, v0] = [Math.max(0, Math.min(...uv.map((p) => p[0]))), Math.max(0, Math.min(...uv.map((p) => p[1])))];
  const [u1, v1] = [Math.min(1, Math.max(...uv.map((p) => p[0]))), Math.min(1, Math.max(...uv.map((p) => p[1])))];
  return [u0, v0, u1 - u0, v1 - v0];
}

/** The box kept inside `area`, each side at least `min` (or the area's). */
function inside([x, y, w, h]: Box, [ax, ay, aw, ah]: Box, min: number): Box {
  w = Math.min(Math.max(w, min), aw);
  h = Math.min(Math.max(h, min), ah);
  return [Math.min(Math.max(x, ax), ax + aw - w), Math.min(Math.max(y, ay), ay + ah - h), w, h];
}

const spanning = (a: Pt, b: Pt): Box => [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.abs(a[0] - b[0]), Math.abs(a[1] - b[1])];

/** Follows what the box holds now, from the playhead. */
function followPicked() {
  const p = useFollow.getState().picking;
  if (p?.box && p.frame) void follow(p.clipId, regionOf(p.box, p.frame));
}

/**
 * While choosing what to follow, over the preview frame in place of the selection box: the box over the clip's
 * picture with the rest dimmed. Drag outside it to draw a new one, inside it to move it, a corner to resize it; a
 * click places it there. Arrow keys move it (Shift ×10), + and − resize it, Enter follows and Esc cancels.
 */
export function FollowPicker({ width, height }: { width: number; height: number }) {
  const picking = useFollow((s) => s.picking)!;
  const canvas = useEditor((s) => s.snap!.project.canvas);
  const timeUs = useEditor((s) => s.timeUs);
  const revision = useEditor((s) => s.snap!.revision);
  const ref = useRef<HTMLDivElement>(null);
  const k = width / canvas.width;
  // The part of the canvas the clip's picture shows at the playhead, where the box can go.
  const [area, setArea] = useState<Box | null>(null);
  const { clipId, box } = picking;

  useEffect(() => {
    let alive = true;
    api
      .layerBounds(timeUs)
      .then((all) => {
        if (!alive) return;
        const mine = all.find((b) => b.clipId === clipId);
        if (!mine) {
          setArea(null);
          useFollow.setState((s) => (s.picking ? { picking: { ...s.picking, frame: null } } : s));
          return;
        }
        const xs = mine.corners.map((c) => c[0]);
        const ys = mine.corners.map((c) => c[1]);
        const [left, top] = [Math.max(0, Math.min(...xs)), Math.max(0, Math.min(...ys))];
        const [right, bottom] = [Math.min(canvas.width, Math.max(...xs)), Math.min(canvas.height, Math.max(...ys))];
        const shown: Box = [left, top, Math.max(0, right - left), Math.max(0, bottom - top)];
        setArea(shown);
        useFollow.setState((s) => {
          if (!s.picking) return s;
          // A third of the shown picture's shorter side, in its middle, until the user draws one.
          const side = Math.min(shown[2], shown[3]) / 3;
          const start: Box = [shown[0] + (shown[2] - side) / 2, shown[1] + (shown[3] - side) / 2, side, side];
          return { picking: { ...s.picking, frame: mine.frame as Pt[], box: inside(s.picking.box ?? start, shown, 1) } };
        });
      })
      .catch(() => alive && setArea(null));
    return () => {
      alive = false;
    };
  }, [timeUs, revision, clipId, canvas.width, canvas.height]);

  const setBox = (next: Box) => useFollow.setState((s) => (s.picking ? { picking: { ...s.picking, box: next } } : s));

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (document.querySelector("dialog[open], [aria-modal='true']")) return;
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        stopPicking();
        document.querySelector<HTMLElement>("[data-follow]")?.focus();
        return;
      }
      // Keys typed into a field or meant for another control stay there.
      const here = !target || target === document.body || !!target.closest("section[aria-label=Preview]");
      if (!here || target?.closest("input, textarea, select, [contenteditable]")) return;
      const { picking } = useFollow.getState();
      if (!picking?.box || !area) return;
      const [x, y, w, h] = picking.box;
      const step = (e.shiftKey ? 10 : 1) / k;
      const moves: Record<string, Pt> = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] };
      const grow = e.key === "+" || e.key === "=" ? 1.1 : e.key === "-" ? 1 / 1.1 : null;
      if (e.key === "Enter" && !(target instanceof HTMLButtonElement)) followPicked();
      else if (moves[e.key]) setBox(inside([x + moves[e.key][0], y + moves[e.key][1], w, h], area, MIN_SIDE / k));
      else if (grow) setBox(inside([x + (w - w * grow) / 2, y + (h - h * grow) / 2, w * grow, h * grow], area, MIN_SIDE / k));
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    // Before the editor's keys, which would step frames or deselect the clip.
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [area, k]);

  const toCanvas = (e: { clientX: number; clientY: number }): Pt => {
    const r = ref.current!.getBoundingClientRect();
    return [(e.clientX - r.left) / k, (e.clientY - r.top) / k];
  };

  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0 || !area || !box) return;
    e.preventDefault();
    const start = toCanvas(e);
    const corner = (e.target as HTMLElement).dataset.corner;
    const [x, y, w, h] = box;
    const corners: Pt[] = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
    const within = start[0] >= x && start[0] <= x + w && start[1] >= y && start[1] <= y + h;
    const min = MIN_SIDE / k;
    let moved = false;
    followPointer({
      move: (ev) => {
        const p = toCanvas(ev);
        moved ||= Math.hypot(p[0] - start[0], p[1] - start[1]) * k >= CLICK;
        if (!moved) return;
        if (corner !== undefined) setBox(inside(spanning(corners[(Number(corner) + 2) % 4], p), area, min));
        else if (within) setBox(inside([x + p[0] - start[0], y + p[1] - start[1], w, h], area, min));
        else setBox(inside(spanning(start, p), area, min));
      },
      up: () => {
        if (!moved && corner === undefined && !within) setBox(inside([start[0] - w / 2, start[1] - h / 2, w, h], area, min));
      },
    });
  };

  const s = box && box.map((v) => v * k);
  return (
    <div ref={ref} className="absolute inset-0 cursor-crosshair touch-none" onPointerDown={onPointerDown} data-testid="follow-picker">
      {s && (
        <>
          <svg className="pointer-events-none absolute inset-0" width={width} height={height}>
            <path d={`M0 0H${width}V${height}H0Z M${s[0]} ${s[1]}V${s[1] + s[3]}H${s[0] + s[2]}V${s[1]}Z`} fill="rgb(0 0 0 / .5)" fillRule="evenodd" />
            <rect x={s[0]} y={s[1]} width={s[2]} height={s[3]} fill="none" stroke="var(--color-fg)" strokeWidth={1.5} />
          </svg>
          <div className="absolute cursor-move" style={{ left: s[0], top: s[1], width: s[2], height: s[3] }} data-follow-box />
          {[
            [s[0], s[1]],
            [s[0] + s[2], s[1]],
            [s[0] + s[2], s[1] + s[3]],
            [s[0], s[1] + s[3]],
          ].map(([cx, cy], i) => (
            <div
              key={i}
              role="presentation"
              data-corner={i}
              className="absolute rounded-[2px] border border-black/60 bg-fg"
              style={{ left: cx - HANDLE / 2, top: cy - HANDLE / 2, width: HANDLE, height: HANDLE, cursor: i % 2 === 0 ? "nwse-resize" : "nesw-resize" }}
            />
          ))}
        </>
      )}
    </div>
  );
}

/**
 * The follow's place in the status chip: while choosing, what to do with Follow and Cancel; then its progress with
 * Stop; then what it did with Undo and Done. Choosing ends when the clip is no longer the one selected, when the video
 * plays or when an agent starts editing.
 */
export function FollowStatus() {
  const picking = useFollow((s) => s.picking);
  const id = useFollow((s) => s.job);
  const done = useFollow((s) => s.done);
  const clipId = picking?.clipId ?? null;
  const ready = !!picking?.box && !!picking.frame;
  const alone = useEditor((s) => s.selection.length === 1 && s.selection[0] === clipId);
  const over = useEditor((s) => {
    const clip = clipId && s.snap ? findClip(s.snap.project, clipId)?.clip : undefined;
    return !!clip && s.timeUs >= clip.startUs && s.timeUs < clip.startUs + clip.durationUs;
  });
  const stopped = useEditor((s) => s.playing || !!s.aiRun);
  useEffect(() => {
    if (clipId && (!alone || stopped)) stopPicking();
  }, [clipId, alone, stopped]);
  // The keyboard goes to Follow, so Enter follows and Tab reaches Cancel.
  useEffect(() => {
    if (clipId) document.querySelector<HTMLElement>("[data-follow-go]")?.focus();
  }, [clipId]);
  if (picking) {
    const why = !over ? "Move the playhead back over the clip" : !ready ? "The clip isn't shown at the playhead" : undefined;
    return (
      <div className={CHIP} role="status">
        <span className="truncate">Drag a box around what to follow</span>
        <ChipButton data-follow-go disabled={!!why} disabledReason={why} title="Enter · arrow keys move the box, + and − resize it" onClick={followPicked}>
          Follow
        </ChipButton>
        <ChipButton
          onClick={() => {
            stopPicking();
            document.querySelector<HTMLElement>("[data-follow]")?.focus();
          }}
        >
          Cancel
        </ChipButton>
      </div>
    );
  }
  return <JobChip id={id} working="Following" done={done} close={() => useFollow.setState({ done: null })} back="[data-follow]" />;
}

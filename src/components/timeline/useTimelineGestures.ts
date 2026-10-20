import { useCallback, useEffect, useRef, useState, type PointerEvent, type RefObject } from "react";
import { followPointer } from "../../lib/drag";
import { FADE_STEP_US, allClips, clipEdges, maxFadeUs, projectDuration, useEditor } from "../../lib/store";
import { US, lastFrame, nearestFrame } from "../../lib/time";
import type { Clip, Project, Track } from "../../lib/types";
import { EDGE, clipAsset } from "./ClipView";

const SNAP_PX = 8;
// How near the line between two tracks the pointer drops a clip on a new track there.
const GAP_PX = 6;
// Cmd on a Mac, where Ctrl-click is a right click.
const MAC = navigator.userAgent.includes("Mac");

type Mode = "move" | "trimL" | "trimR" | "fadeIn" | "fadeOut";

export const isFade = (mode: Mode) => mode === "fadeIn" || mode === "fadeOut";

export interface Drag {
  clip: Clip;
  trackId: string;
  mode: Mode;
  startX: number;
  startY: number;
  moved: boolean;
  toggle: boolean;
  dxUs: number;
  /** undefined: same track, null: a new track, string: another track, `below`: a new track right below that one. */
  target: string | null | { below: string } | undefined;
  snapUs: number | null;
  candidates: number[];
  maxDurUs: number | null;
  sourceInUs: number | null;
  speed: number;
  /** A fade drag: the sound fade at that end when it started, and the longest it may get. */
  fadeUs: number;
  maxFadeUs: number;
}

/**
 * Clip timing while a drag is in progress. Every value is whole microseconds (the engine takes
 * integers), rounded towards the inside of the source the same way the engine clamps a trim.
 */
export function dragResult(d: Drag, minUs: number): { startUs: number; durationUs: number; sourceInUs?: number } {
  const { clip } = d;
  if (d.mode === "move") return { startUs: Math.max(0, clip.startUs + d.dxUs), durationUs: clip.durationUs };
  if (d.mode === "trimL") {
    // The left edge stops where the source starts, at this clip's speed.
    const lower = d.sourceInUs !== null ? Math.ceil(-d.sourceInUs / d.speed) : -clip.startUs;
    const delta = Math.min(Math.max(d.dxUs, lower, -clip.startUs), clip.durationUs - minUs);
    const sourceInUs = d.sourceInUs !== null ? Math.max(0, d.sourceInUs + Math.round(delta * d.speed)) : undefined;
    return { startUs: clip.startUs + delta, durationUs: clip.durationUs - delta, sourceInUs };
  }
  const dur = Math.max(minUs, Math.min(clip.durationUs + d.dxUs, d.maxDurUs ?? Infinity));
  return { startUs: clip.startUs, durationUs: dur };
}

/** The fade a fade drag sets: the fade-in handle lengthens it to the right, the fade-out handle to the left. */
export function dragFade(d: Drag): number {
  const us = d.fadeUs + (d.mode === "fadeIn" ? d.dxUs : -d.dxUs);
  return Math.max(0, Math.min(d.maxFadeUs, Math.round(us / FADE_STEP_US) * FADE_STEP_US));
}

/** The box a drag over empty lane space draws, in the timeline scroller's content pixels. */
export type Box = { left: number; top: number; width: number; height: number };

/**
 * The video or text track whose lower edge the pointer is at, so a clip dropped there goes on a new track
 * right below it, over the track under the edge. Not beside a track the clip has to itself, where it would
 * stay on the same layer.
 */
function trackAboveGap(project: Project, rects: (readonly [string, DOMRect])[], y: number, d: Drag): string | undefined {
  const visual = rects.filter(([id]) => project.tracks.find((t) => t.id === id)?.kind !== "audio").sort((a, b) => a[1].top - b[1].top);
  const alone = project.tracks.find((t) => t.id === d.trackId)?.clips.length === 1;
  for (let i = 0; i + 1 < visual.length; i++) {
    const [[upper, u], [lower, l]] = [visual[i], visual[i + 1]];
    if (y < u.bottom - GAP_PX || y > l.top + GAP_PX) continue;
    return alone && (upper === d.trackId || lower === d.trackId) ? undefined : upper;
  }
}

function snap(value: number, candidates: number[], thr: number): number | null {
  let best: number | null = null;
  for (const c of candidates) if (Math.abs(c - value) <= thr && (best === null || Math.abs(c - value) < Math.abs(best - value))) best = c;
  return best;
}

/**
 * Moving and trimming clips, dragging their sound fades, scrubbing the ruler, and clicking or box
 * selecting in empty lane space; all follow the pointer on the window. A drag binds its listeners once when it starts and reads
 * the latest zoom and snapping from a ref, so pointer moves only update the drag state.
 */
export function useTimelineGestures({
  project,
  zoom,
  snapping,
  minUs,
  rows,
  scroller,
  lanesAt,
  timeAt,
}: {
  project: Project | undefined;
  zoom: number;
  snapping: boolean;
  minUs: number;
  rows: RefObject<Map<string, HTMLDivElement>>;
  scroller: RefObject<HTMLDivElement | null>;
  /** Where the lanes start in the scroller, past the track headers and the ruler. */
  lanesAt: { left: number; top: number };
  timeAt: (clientX: number) => number;
}) {
  const [drag, setDrag] = useState<Drag | null>(null);
  const live = useRef({ project, zoom, snapping, minUs, timeAt });
  live.current = { project, zoom, snapping, minUs, timeAt };
  const stopDrag = useRef<(() => void) | null>(null);
  useEffect(() => () => stopDrag.current?.(), []);

  /** The drag after the pointer moved to `e`, or null while it has not moved far enough to count. */
  const step = useCallback(
    (d: Drag, e: globalThis.PointerEvent, kind: Track["kind"]): Drag | null => {
      const { project, zoom, snapping } = live.current;
      const moved = d.moved || Math.hypot(e.clientX - d.startX, e.clientY - d.startY) > 3;
      if (!moved || !project) return null;
      let dxUs = ((e.clientX - d.startX) / zoom) * US;
      if (isFade(d.mode)) return { ...d, moved: true, dxUs: Math.round(dxUs) };
      let snapUs: number | null = null;
      const thr = (SNAP_PX / zoom) * US;
      if (snapping) {
        if (d.mode === "move") {
          const s = d.clip.startUs + dxUs;
          const a = snap(s, d.candidates, thr);
          const b = snap(s + d.clip.durationUs, d.candidates, thr);
          const da = a === null ? Infinity : Math.abs(a - s);
          const db = b === null ? Infinity : Math.abs(b - (s + d.clip.durationUs));
          if (a !== null && da <= db) {
            dxUs = a - d.clip.startUs;
            snapUs = a;
          } else if (b !== null) {
            dxUs = b - d.clip.durationUs - d.clip.startUs;
            snapUs = b;
          }
        } else {
          const edge = d.mode === "trimL" ? d.clip.startUs + dxUs : d.clip.startUs + d.clip.durationUs + dxUs;
          const s = snap(edge, d.candidates, thr);
          if (s !== null) {
            dxUs += s - edge;
            snapUs = s;
          }
        }
      }
      let target: Drag["target"] = undefined;
      if (d.mode === "move") {
        const rects = [...rows.current.entries()].map(([id, el]) => [id, el.getBoundingClientRect()] as const);
        const hit = rects.find(([, r]) => e.clientY >= r.top && e.clientY <= r.bottom);
        const above = kind !== "audio" ? trackAboveGap(project, rects, e.clientY, d) : undefined;
        if (above) {
          target = { below: above };
        } else if (hit) {
          const t = project.tracks.find((tr) => tr.id === hit[0]);
          if (t && t.kind === kind && t.id !== d.trackId) target = t.id;
        } else if (rects.length > 0) {
          const top = Math.min(...rects.map(([, r]) => r.top));
          const bottom = Math.max(...rects.map(([, r]) => r.bottom));
          if ((kind !== "audio" && e.clientY < top) || (kind === "audio" && e.clientY > bottom)) target = null;
        }
      }
      return { ...d, moved: true, dxUs: Math.round(dxUs), snapUs, target };
    },
    [rows],
  );

  const finish = (d: Drag) => {
    const { select, edit } = useEditor.getState();
    if (!d.moved) {
      const sel = useEditor.getState().selection;
      if (d.toggle) select(sel.includes(d.clip.id) ? sel.filter((id) => id !== d.clip.id) : [...sel, d.clip.id]);
      else select([d.clip.id]);
      return;
    }
    if (isFade(d.mode)) {
      const fadeUs = dragFade(d);
      if (fadeUs !== d.fadeUs) edit({ type: "updateClip", clipId: d.clip.id, ...(d.mode === "fadeIn" ? { fadeInUs: fadeUs } : { fadeOutUs: fadeUs }) });
      select([d.clip.id]);
      return;
    }
    const r = dragResult(d, live.current.minUs);
    const { target } = d;
    if (d.mode === "move") {
      if (target && typeof target === "object") {
        edit({ type: "moveClip", clipId: d.clip.id, trackId: target.below, startUs: r.startUs, newTrack: "below" });
      } else {
        const trackId = target === undefined ? d.trackId : target;
        if (trackId === d.trackId && r.startUs === d.clip.startUs) return;
        edit({ type: "moveClip", clipId: d.clip.id, trackId, startUs: r.startUs });
      }
    } else {
      edit({ type: "trimClip", clipId: d.clip.id, startUs: r.startUs, durationUs: r.durationUs, sourceInUs: r.sourceInUs ?? null });
    }
    select([d.clip.id]);
  };
  const finishRef = useRef(finish);
  finishRef.current = finish;

  const startClipDrag = useCallback(
    (e: PointerEvent, clip: Clip, track: Track) => {
      const { project } = live.current;
      if (e.button !== 0 || !project) return;
      e.stopPropagation();
      stopDrag.current?.();
      const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
      const x = e.clientX - rect.left;
      // The fade handles sit over the top of the trim zones.
      const fade = (e.target as HTMLElement).closest<HTMLElement>("[data-fade]")?.dataset.fade;
      const mode: Mode = fade === "in" ? "fadeIn" : fade === "out" ? "fadeOut" : x <= EDGE ? "trimL" : x >= rect.width - EDGE ? "trimR" : "move";
      const asset = clipAsset(project, clip);
      const media = clip.content.type === "media" ? clip.content : null;
      const sourceInUs = media && asset?.kind !== "image" ? media.sourceInUs : null;
      // The right edge stops where the source ends, at this clip's speed (floored like the engine).
      const maxDurUs = sourceInUs !== null && asset && media ? Math.floor((asset.durationUs - sourceInUs) / media.speed) : null;
      const candidates = [0, useEditor.getState().timeUs];
      for (const c of allClips(project)) if (c.id !== clip.id) candidates.push(c.startUs, c.startUs + c.durationUs);
      // While the AI edits, pressing a clip only selects it.
      const locked = !!useEditor.getState().aiRun;
      let d: Drag = { clip, trackId: track.id, mode, startX: e.clientX, startY: e.clientY, moved: false, toggle: e.shiftKey || (MAC ? e.metaKey : e.ctrlKey), dxUs: 0, target: undefined, snapUs: null, candidates, maxDurUs, sourceInUs, speed: media?.speed ?? 1, fadeUs: (mode === "fadeIn" ? media?.fadeInUs : media?.fadeOutUs) ?? 0, maxFadeUs: maxFadeUs(clip.durationUs) };
      setDrag(d);

      // Pointer moves are handled once per display frame: a fast mouse sends several per frame, and each
      // one would re-render the timeline. The last one is applied before a release commits the drag.
      let frame = 0;
      let latest: globalThis.PointerEvent | null = null;
      const apply = () => {
        frame = 0;
        const ev = latest;
        latest = null;
        const next = ev && step(d, ev, track.kind);
        if (next) setDrag((d = next));
      };
      const end = (commit: boolean) => {
        unfollow();
        window.removeEventListener("keydown", onKey, true);
        cancelAnimationFrame(frame);
        if (commit) apply();
        stopDrag.current = null;
        setDrag(null);
        if (commit) finishRef.current(d);
      };
      const onKey = (ev: KeyboardEvent) => {
        if (ev.key === "Escape") {
          ev.stopPropagation();
          end(false);
        }
      };
      // A cancelled pointer or a lost window focus ends the drag like Esc: nothing changes.
      const unfollow = followPointer({
        move: (ev) => {
          if (locked) return;
          latest = ev;
          frame ||= requestAnimationFrame(apply);
        },
        up: () => end(true),
        cancel: () => end(false),
      });
      window.addEventListener("keydown", onKey, true);
      stopDrag.current = () => end(false);
    },
    [step],
  );

  /** Where the playhead lands for a pointer at `x`: on a frame, or on a clip edge within 8 px while snapping. */
  const playheadAt = useCallback((x: number) => {
    const { project, zoom, snapping, timeAt } = live.current;
    const t = timeAt(x);
    if (!project) return t;
    const { fps } = project.canvas;
    return (snapping ? snap(t, clipEdges(project), (SNAP_PX / zoom) * US) : null) ?? Math.min(nearestFrame(t, fps), lastFrame(projectDuration(project), fps));
  }, []);

  const startScrub = useCallback(
    (e: PointerEvent) => {
      if (e.button !== 0) return;
      const { seek } = useEditor.getState();
      seek(playheadAt(e.clientX));
      let raf = 0;
      let lastX = e.clientX;
      followPointer({
        move: (ev) => {
          lastX = ev.clientX;
          if (!raf)
            raf = requestAnimationFrame(() => {
              raf = 0;
              seek(playheadAt(lastX));
            });
        },
      });
    },
    [playheadAt],
  );

  const [box, setBox] = useState<Box | null>(null);

  /**
   * A press on empty lane space: a click moves the playhead there and clears the selection; a drag
   * draws a box and selects every clip it touches, added to the selection with Shift or Cmd/Ctrl.
   */
  const startBoxSelect = useCallback(
    (e: PointerEvent) => {
      const el = scroller.current;
      if (e.button !== 0 || !el) return;
      stopDrag.current?.();
      const { select, selectCut, seek, selection: before, cut } = useEditor.getState();
      const add = e.shiftKey || (MAC ? e.metaKey : e.ctrlKey);
      const base = add ? before : [];
      // In the scroller's content, so the box stays on the clips when the timeline scrolls.
      const at = (x: number, y: number) => {
        const r = el.getBoundingClientRect();
        return { x: x - r.left + el.scrollLeft, y: y - r.top + el.scrollTop };
      };
      const start = at(e.clientX, e.clientY);
      let b: Box | null = null;
      let frame = 0;
      let pointer = { x: e.clientX, y: e.clientY };
      const apply = (autoScroll: boolean) => {
        frame = 0;
        if (!b && Math.hypot(pointer.x - e.clientX, pointer.y - e.clientY) <= 3) return;
        const r = el.getBoundingClientRect();
        if (autoScroll) {
          // Within 24 px of an edge of the lanes the timeline scrolls on, faster nearer the edge, so the box reaches clips out of view.
          const speed = (d: number) => (d < 24 ? Math.ceil((24 - Math.max(0, d)) / 2) : 0);
          const [left, top] = [el.scrollLeft, el.scrollTop];
          el.scrollLeft += speed(r.left + el.clientWidth - pointer.x) - speed(pointer.x - r.left - lanesAt.left);
          el.scrollTop += speed(r.top + el.clientHeight - pointer.y) - speed(pointer.y - r.top - lanesAt.top);
          if (el.scrollLeft !== left || el.scrollTop !== top) frame = requestAnimationFrame(() => apply(true));
        }
        const p = at(pointer.x, pointer.y);
        b = { left: Math.min(start.x, p.x), top: Math.min(start.y, p.y), width: Math.abs(p.x - start.x), height: Math.abs(p.y - start.y) };
        setBox(b);
        const [x0, y0] = [b.left + r.left - el.scrollLeft, b.top + r.top - el.scrollTop];
        const hit: string[] = [];
        for (const c of el.querySelectorAll<HTMLElement>("[data-clip-id]")) {
          const cr = c.getBoundingClientRect();
          if (cr.right > x0 && cr.left < x0 + b.width && cr.bottom > y0 && cr.top < y0 + b.height) hit.push(c.dataset.clipId!);
        }
        const ids = [...base, ...hit.filter((id) => !base.includes(id))];
        const { selection: sel, cut } = useEditor.getState();
        if (cut || ids.length !== sel.length || ids.some((id, i) => id !== sel[i])) select(ids);
      };
      const end = (commit: boolean) => {
        unfollow();
        window.removeEventListener("keydown", onKey, true);
        el.removeEventListener("scroll", follow);
        cancelAnimationFrame(frame);
        stopDrag.current = null;
        if (commit) apply(false);
        setBox(null);
        // A transition is selected apart from the clips, so it comes back on its own.
        if (!commit) (cut ? selectCut(cut) : select(before));
        else if (!b) {
          if (!add) select([]);
          seek(playheadAt(e.clientX));
        }
      };
      const onKey = (ev: KeyboardEvent) => {
        if (ev.key === "Escape") {
          ev.stopPropagation();
          end(false);
        }
      };
      const follow = () => (frame ||= requestAnimationFrame(() => apply(true)));
      const unfollow = followPointer({
        move: (ev) => {
          pointer = { x: ev.clientX, y: ev.clientY };
          follow();
        },
        up: (ev) => {
          pointer = { x: ev.clientX, y: ev.clientY };
          end(true);
        },
        cancel: () => end(false),
      });
      window.addEventListener("keydown", onKey, true);
      // Scrolling with the wheel mid-drag stretches the box too.
      el.addEventListener("scroll", follow);
      stopDrag.current = () => end(false);
    },
    [scroller, lanesAt, playheadAt],
  );

  return { drag, box, startClipDrag, startScrub, startBoxSelect };
}

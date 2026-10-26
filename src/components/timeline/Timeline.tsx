import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type RefObject } from "react";
import { Copy, Magnet, Maximize2, PanelLeftClose, PanelRightClose, RotateCwSquare, Scissors, Snowflake, Trash2, Unlink, ZoomIn, ZoomOut } from "lucide-react";
import { useShallow } from "zustand/react/shallow";
import {
  MAIN_TRACK,
  allClips,
  contentEnd,
  deleteSelection,
  deleteSide,
  detachAudio,
  detachBlocker,
  displayTracks,
  duplicateSelection,
  editClip,
  findClip,
  mainClips,
  projectDuration,
  rotateSelection,
  splitAtPlayhead,
  splitBlocker,
  freezeAtPlayhead,
  freezeBlocker,
  AI_EDITING,
  useAiLocked,
  useEditor,
} from "../../lib/store";
import { US, formatDuration, formatTime } from "../../lib/time";
import type { Clip, EditCmd, Project, Track } from "../../lib/types";
import { candidateReason, thumbnailOf, useCandidates, useCover } from "../../lib/cover";
import { describePeek } from "../../lib/peek";
import { usePlugins } from "../../lib/plugins";
import { setDropResolver, type DropMedia } from "../panel/assets";
import { IconButton, RangeInput } from "../ui";
import { ClipMenu, type MenuAt } from "./ClipMenu";
import { ClipView } from "./ClipView";
import { CutMarkers } from "./CutMarkers";
import { HistoryButton, type Peek } from "./HistoryButton";
import { TrackHeader } from "./TrackHeader";
import { SNAP_PX, dragFade, dragResult, isFade, snap, useTimelineGestures, type Drag } from "./useTimelineGestures";

const HEADER_W = 152;
const RULER_H = 28;
/** Space between lanes. It belongs to the rows, so the track header column runs unbroken. */
const ROW_GAP = 4;
const LANES_AT = { left: HEADER_W, top: RULER_H };
/** How far into each end of a clip dropped media still goes in beside it instead of replacing it, in px. */
const REPLACE_END = 24;
const OVERLAY_LANE_H = 28;
const END_TIP = "The video ends here. Sound after this point is not exported.";

function rowHeight(track: Track) {
  if (track.id === MAIN_TRACK) return 64;
  if (track.kind === "video") return 50;
  if (track.kind === "audio") return 58;
  return 34;
}

type Timing = { startUs: number; durationUs: number };

/** Where the engine puts a clip moved or added to the magnetic main track: before the first other clip whose middle is later. */
function mainInsertIndex(others: Clip[], startUs: number) {
  const i = others.findIndex((c) => c.startUs + c.durationUs / 2 > startUs);
  return i < 0 ? others.length : i;
}

/**
 * The main track as the drag in progress would leave it, so neighbours move live: a moved clip
 * opens a slot where it would go in (`slotUs`, null when it leaves the track) and the gap it left
 * closes; a trim of the end moves everything after it with the edge. A trim of the start leaves the
 * track still, so the edge stays under the pointer as in CapCut; the gap closes when it lands.
 */
function mainLayout(project: Project, d: Drag | null, minUs: number): { timing: Map<string, Timing>; slotUs: number | null } | null {
  if (!d?.moved) return null;
  const main = mainClips(project);
  const timing = new Map<string, Timing>();
  const r = dragResult(d, minUs);
  if (d.mode === "move") {
    const dest = d.target === undefined ? d.trackId : d.target;
    if (d.trackId !== MAIN_TRACK && dest !== MAIN_TRACK) return null;
    const others = main.filter((c) => c.id !== d.clip.id);
    const at = dest === MAIN_TRACK ? mainInsertIndex(others, r.startUs) : -1;
    let t = 0;
    let slotUs: number | null = null;
    others.forEach((c, i) => {
      if (i === at) {
        slotUs = t;
        t += d.clip.durationUs;
      }
      timing.set(c.id, { startUs: t, durationUs: c.durationUs });
      t += c.durationUs;
    });
    if (at === others.length) slotUs = t;
    return { timing, slotUs };
  }
  if (d.trackId !== MAIN_TRACK || d.mode === "trimL") return null;
  const shift = r.durationUs - d.clip.durationUs;
  for (const c of main)
    timing.set(c.id, c.id === d.clip.id ? { startUs: c.startUs, durationUs: r.durationUs } : { startUs: c.startUs > d.clip.startUs ? c.startUs + shift : c.startUs, durationUs: c.durationUs });
  return { timing, slotUs: null };
}

/** How far a time is from a clip: 0 inside it. */
const distance = (c: Clip, t: number) => (t < c.startUs ? c.startUs - t : Math.max(0, t - c.startUs - c.durationUs));

/**
 * The clip an arrow key moves keyboard focus to: ←/→ the previous or next clip on the same track,
 * ↑/↓ the clip nearest in time on the track above or below that has clips. Null at an edge.
 */
function neighbour(tracks: Track[], track: Track, from: Clip, key: string): Clip | null {
  if (key === "ArrowLeft" || key === "ArrowRight") {
    const clips = [...track.clips].sort((a, b) => a.startUs - b.startUs);
    return clips[clips.findIndex((c) => c.id === from.id) + (key === "ArrowLeft" ? -1 : 1)] ?? null;
  }
  const rows = tracks.filter((t) => t.clips.length > 0);
  const next = rows[rows.findIndex((t) => t.id === track.id) + (key === "ArrowUp" ? -1 : 1)];
  const mid = from.startUs + from.durationUs / 2;
  return next ? next.clips.reduce((best, c) => (distance(c, mid) < distance(best, mid) ? c : best)) : null;
}

/**
 * Alt+←/→ moves a clip by a frame (Shift: a second), stopping at its neighbours so it never jumps to
 * a new track; on the magnetic main track it trades places with the clip beside it. A selected clip
 * takes the rest of the selection off the main track along, as a drag does. A burst of presses is
 * one undo step.
 */
function nudge(clipId: string, dir: -1 | 1, second: boolean) {
  editClip(
    clipId,
    (c, track, project): EditCmd | null => {
      if (track.id === MAIN_TRACK) {
        const main = [...track.clips].sort((a, b) => a.startUs - b.startUs);
        const other = main[main.findIndex((m) => m.id === c.id) + dir];
        if (!other) return null;
        // The engine puts a moved clip before the first other clip whose middle is later.
        return { type: "moveClip", clipId: c.id, trackId: MAIN_TRACK, startUs: dir < 0 ? other.startUs : Math.round(other.startUs + other.durationUs / 2) };
      }
      const selection = useEditor.getState().selection;
      const group = selection.length > 1 && selection.includes(c.id);
      const moving = group ? project.tracks.filter((t) => t.id !== MAIN_TRACK).flatMap((t) => t.clips.filter((o) => selection.includes(o.id)).map((o) => [o, t] as const)) : [[c, track] as const];
      const ids = new Set(moving.map(([m]) => m.id));
      let [lo, hi] = [-Number.MAX_SAFE_INTEGER, Number.MAX_SAFE_INTEGER];
      for (const t of new Set(moving.map(([, t]) => t))) {
        // Each moved clip stops at the nearest clip of its track that stays, so walk the track in time order.
        const clips = [...t.clips].sort((a, b) => a.startUs - b.startUs);
        let end = 0;
        for (const o of clips) {
          if (ids.has(o.id)) lo = Math.max(lo, end - o.startUs);
          else end = Math.max(end, o.startUs + o.durationUs);
        }
        let start = Number.MAX_SAFE_INTEGER;
        for (const o of clips.reverse()) {
          if (ids.has(o.id)) hi = Math.min(hi, start - o.startUs - o.durationUs);
          else start = Math.min(start, o.startUs);
        }
      }
      const step = second ? US : US / project.canvas.fps;
      const offsetUs = Math.round(Math.max(lo, Math.min(hi, dir * step)));
      if (offsetUs === 0) return null;
      return group ? { type: "moveClips", clipIds: moving.map(([m]) => m.id), offsetUs } : { type: "moveClip", clipId: c.id, trackId: track.id, startUs: c.startUs + offsetUs };
    },
    `nudge:${clipId}`,
  );
}

/**
 * Alt+↑/↓ moves a video or text clip a layer up or down: onto a new track right past the next video or
 * text track above or below its own. Main-track clips stay, and nothing goes below the main track.
 */
function restack(clipId: string, up: boolean, moving: () => void) {
  editClip(clipId, (c, track, project): EditCmd | null => {
    if (track.kind === "audio" || track.id === MAIN_TRACK) return null;
    const visual = project.tracks.filter((t) => t.kind !== "audio");
    const past = visual[visual.findIndex((t) => t.id === track.id) + (up ? 1 : -1)];
    if (!past || (!up && past.id === MAIN_TRACK)) return null;
    moving();
    return { type: "moveClip", clipId: c.id, trackId: past.id, startUs: c.startUs, newTrack: up ? "above" : "below" };
  });
}

/** Focuses a clip and scrolls it into view, clear of the sticky ruler and track headers. */
function focusClipIn(box: HTMLDivElement | null, id: string) {
  const el = box?.querySelector<HTMLElement>(`[data-clip-id="${id}"]`);
  if (!box || !el) return;
  el.focus({ preventScroll: true });
  const view = box.getBoundingClientRect();
  const r = el.getBoundingClientRect();
  const [left, right] = [view.left + HEADER_W, view.left + box.clientWidth];
  const [top, bottom] = [view.top + RULER_H, view.top + box.clientHeight];
  if (r.left < left) box.scrollLeft -= left - r.left + 24;
  else if (r.right > right) box.scrollLeft += Math.min(r.left - left - 24, r.right - right + 24);
  if (r.top < top) box.scrollTop -= top - r.top;
  else if (r.bottom > bottom) box.scrollTop += r.bottom - bottom;
}

function tickStep(zoom: number): number {
  for (const s of [0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600]) if (s * zoom >= 72) return s;
  return 1200;
}

const VISIBLE_STEP = 400;

/**
 * The playhead line, the one part of the timeline that follows playback frame by frame; it keeps itself in view while playing,
 * except while `follow` is off: a box select scrolls the timeline itself.
 */
function Playhead({ zoom, scroller, follow }: { zoom: number; scroller: RefObject<HTMLDivElement | null>; follow: boolean }) {
  const timeUs = useEditor((s) => s.timeUs);
  const playing = useEditor((s) => s.playing);
  useEffect(() => {
    const el = scroller.current;
    if (!el || !playing || !follow) return;
    const x = (timeUs / US) * zoom;
    const lane = el.clientWidth - HEADER_W;
    if (x > el.scrollLeft + lane - 24 || x < el.scrollLeft) el.scrollLeft = Math.max(0, x - 48);
  }, [timeUs, playing, zoom, scroller, follow]);
  // Above the ruler, below the track headers. The dark edges keep it visible on bright footage; no blur, it moves every frame.
  // It moves by transform on its own layer, so playback never repaints the clips under it.
  return (
    <div className="pointer-events-none absolute bottom-0 top-0 z-[45] will-change-transform" style={{ left: HEADER_W, transform: `translateX(${(timeUs / US) * zoom}px)` }}>
      <div className="absolute -left-[0.75px] bottom-0 top-2 w-[1.5px] bg-white shadow-[0_0_0_0.5px_rgb(0_0_0/.4)]" />
      <div className="absolute -left-1.5 top-0.5 h-[15px] w-3 rounded-t-[2px] bg-white [clip-path:polygon(0_0,100%_0,100%_60%,50%_100%,0_60%)]" />
    </div>
  );
}

export function Timeline({ height }: { height: number }) {
  const snap0 = useEditor((s) => s.snap);
  const project = snap0?.project;
  const selection = useEditor((s) => s.selection);
  const cut = useEditor((s) => s.cut);
  // A string or null, so playback re-renders the timeline only when it changes.
  const splitBlocked = useEditor((s) => (s.snap ? splitBlocker(s.snap.project, s.selection, s.timeUs) : "move the playhead over a clip"));
  const freezeBlocked = useEditor((s) => (s.snap ? freezeBlocker(s.snap.project, s.selection, Math.round(s.timeUs)) : "move the playhead over a video clip to freeze its frame"));
  const zoom = useEditor((s) => s.zoom);
  // Media dragged from the panel, or files dragged in from the desktop.
  const assetDrag = useEditor((s) => s.assetDrag ?? s.fileDrag);
  const dragMedia = useEditor(
    useShallow((s): DropMedia | null => {
      const asset = s.assetDrag && s.snap?.project.assets.find((a) => a.id === s.assetDrag!.assetId);
      return asset ? { kind: asset.kind, hasAudio: asset.hasAudio, durationUs: asset.durationUs } : s.fileDrag?.kind ? { kind: s.fileDrag.kind, several: s.fileDrag.several } : null;
    }),
  );
  const locked = useAiLocked();
  // In the cover editor the timeline chooses the cover's frame: clicks and drags move the playhead, which is the frame.
  const cover = useCover((s) => s.open);
  const coverFromImage = useEditor((s) => !!(cover && thumbnailOf(s.snap?.project, cover)?.image));
  const candidates = useCandidates();
  const { select, setZoom } = useEditor.getState();
  const scroller = useRef<HTMLDivElement>(null);
  const rows = useRef(new Map<string, HTMLDivElement>());
  /** The empty lane above the main track that puts dropped media over the video. */
  const overlayLane = useRef<HTMLDivElement>(null);
  const [snapping, setSnapping] = useState(true);
  const [view, setView] = useState({ left: 0, width: 1000 });
  const [menu, setMenu] = useState<MenuAt | null>(null);
  /** The clip keyboard focus was on last; it stays the timeline's tab stop. */
  const [focusId, setFocusId] = useState<string | null>(null);
  /** The focused clip, so focus can move on when it is deleted; null once focus left the clips. */
  const focusPlace = useRef<string | null>(null);
  /** A clip Alt+↑/↓ moved a layer, scrolled into view when it shows on its new track. */
  const restacked = useRef<string | null>(null);
  /** The project as last shown, where a deleted clip's neighbours are found. */
  const shown = useRef(project);
  const closeMenu = useCallback(() => setMenu(null), []);
  const fps = project?.canvas.fps ?? 30;
  const minUs = Math.ceil(US / fps);
  const timeAt = useCallback(
    (clientX: number) => {
      const el = scroller.current!;
      const rect = el.getBoundingClientRect();
      return Math.max(0, ((clientX - rect.left + el.scrollLeft - HEADER_W) / zoom) * US);
    },
    [zoom],
  );

  const { drag: gesture, box, startClipDrag, startScrub, startBoxSelect } = useTimelineGestures({ project, zoom, snapping, minUs, rows, lane: overlayLane, scroller, lanesAt: LANES_AT, timeAt });
  // While Undo or Redo is hovered, the lanes draw the project that step would restore, with what changes marked.
  // Only the drawing: selection, focus and edits keep working on the project as it is. A box select passing over
  // the button never shows it, since the box selects the clips drawn.
  const [peek, setPeek] = useState<Peek | null>(null);
  const peekShown = peek && !locked && !box && project && peek.revision === snap0?.revision ? peek : null;
  const described = useMemo(() => (peekShown && project ? describePeek(project, peekShown.project) : null), [peekShown, project]);
  const drawn = peekShown && described?.draw === "then" ? peekShown.project : project;
  // While a plugin is open, the clips its run would change are marked where they are now; editing goes on.
  const pluginPreview = usePlugins((s) => s.preview);
  const pluginMarks = useMemo(
    () => (!peekShown && !locked && project && pluginPreview?.project && pluginPreview.revision === snap0?.revision ? describePeek(project, pluginPreview.project).marks : null),
    [peekShown, locked, project, pluginPreview, snap0?.revision],
  );

  const tracks = useMemo(() => (drawn ? displayTracks(drawn) : []), [drawn]);
  const duration = drawn ? contentEnd(drawn) : 0;
  // A shorter preview never narrows the lanes, so the scroll position survives it. Nor does anything narrow them
  // under what is on screen: zooming out at the end keeps its anchor, and a clip dragged past the end has the ruler above it.
  const lanesShown = view.width - HEADER_W;
  const laneWidth = Math.max(view.left + lanesShown, (Math.max(duration, project ? contentEnd(project) : 0) / US) * zoom + lanesShown / 2);
  // Clips get the window they draw filmstrip tiles in. It moves in steps, with at least 200 px of margin on
  // each side, so scrolling re-renders the clips once per step instead of on every scroll event.
  const windowFrom = Math.floor(view.left / VISIBLE_STEP) * VISIBLE_STEP;
  const visible = useMemo<[number, number]>(() => [windowFrom - 200, windowFrom + view.width + VISIBLE_STEP + 200], [windowFrom, view.width]);

  // Media dropped on the middle of a clip that can show or play it goes in that clip; its ends insert beside it.
  // While Undo or Redo is hovered the clips are drawn where they were, so nothing is offered.
  const peeking = !!peekShown;
  const replaceTarget = useCallback(
    (x: number, y: number, media: DropMedia | null) => {
      const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-clip-id]");
      if (peeking || !media || media.several || !project || !el || !scroller.current?.contains(el)) return null;
      const r = el.getBoundingClientRect();
      const end = Math.min(r.width / 4, REPLACE_END);
      const found = findClip(project, el.dataset.clipId!);
      const c = found?.clip.content;
      if (!found || c?.type !== "media" || c.speed === 0 || x < r.left + end || x > r.right - end) return null;
      const fits = found.track.kind === "video" ? media.kind !== "audio" : found.track.kind === "audio" && media.kind !== "image" && media.hasAudio !== false;
      // The engine's tolerance: one frame of the clip's source.
      const short = media.kind !== "image" && media.durationUs !== undefined && media.durationUs + minUs * c.speed < found.clip.durationUs * c.speed;
      return fits ? { clipId: found.clip.id, short } : null;
    },
    [project, peeking, minUs],
  );

  const playing = useEditor((s) => s.playing);
  // Where media dropped at a point lands. Over a track beside the main one, which takes video at a cut, it snaps
  // like a moved clip, but not to a playing playhead, which moves on without redrawing the drop line.
  const dropAt = useCallback(
    (x: number, y: number) => {
      const el = scroller.current;
      if (!el || !project) return null;
      const r = el.getBoundingClientRect();
      if (x < r.left || x > r.right || y < r.top || y > r.bottom) return null;
      let trackId: string | null = null;
      for (const [id, row] of rows.current) {
        const rr = row.getBoundingClientRect();
        if (y >= rr.top && y <= rr.bottom) trackId = id;
      }
      const lane = overlayLane.current?.getBoundingClientRect();
      const over = lane && y >= lane.top && y <= lane.bottom ? ("overlay" as const) : undefined;
      const t = timeAt(x);
      const edges = [0, ...(playing ? [] : [useEditor.getState().timeUs]), ...allClips(project).flatMap((c) => [c.startUs, c.startUs + c.durationUs])];
      const snapped = snapping && (over || (trackId && trackId !== MAIN_TRACK)) ? snap(t, edges, (SNAP_PX / zoom) * US) : null;
      return { trackId, startUs: Math.round(snapped ?? t), over };
    },
    [project, playing, snapping, timeAt, zoom],
  );

  // Media dragged from the media panel lands here.
  useEffect(() => {
    setDropResolver((x, y, media) => {
      const place = dropAt(x, y);
      return place && { ...place, replaceId: replaceTarget(x, y, media)?.clipId ?? null };
    });
    return () => setDropResolver(null);
  }, [dropAt, replaceTarget]);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const update = () => setView({ left: el.scrollLeft, width: el.clientWidth });
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Zooming keeps one moment where it is on screen: the one under the pointer for Ctrl + wheel,
  // otherwise the playhead, which comes to the middle when it was out of view.
  const zoomAnchor = useRef<{ t: number; px: number } | null>(null);
  const shownZoom = useRef(zoom);
  useLayoutEffect(() => {
    const el = scroller.current;
    const before = shownZoom.current;
    shownZoom.current = zoom;
    if (!el || before === zoom) return;
    const lane = el.clientWidth - HEADER_W;
    let anchor = zoomAnchor.current;
    zoomAnchor.current = null;
    if (!anchor) {
      const t = useEditor.getState().timeUs / US;
      const px = t * before - el.scrollLeft;
      anchor = { t, px: px >= 0 && px <= lane ? px : lane / 2 };
    }
    el.scrollLeft = Math.max(0, anchor.t * zoom - anchor.px);
  }, [zoom]);

  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      const z = useEditor.getState().zoom;
      const px = e.clientX - el.getBoundingClientRect().left - HEADER_W;
      zoomAnchor.current = { t: (el.scrollLeft + px) / z, px };
      // A touchpad sends many small deltas per gesture, a mouse wheel one large delta per notch.
      const dy = e.deltaMode === WheelEvent.DOM_DELTA_PIXEL ? e.deltaY : e.deltaY * 40;
      setZoom(z * Math.min(1.15, Math.max(1 / 1.15, Math.exp(-dy * 0.005))));
      // Already at the limit: nothing moves, and the next zoom anchors on the playhead again.
      if (useEditor.getState().zoom === z) zoomAnchor.current = null;
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [setZoom]);

  // A fade drag changes only the clip's fades; the ghost, the slot and the snap guide belong to moves and trims.
  const drag = gesture && !isFade(gesture.mode) ? gesture : null;
  const groupIds = useMemo(() => new Set(drag?.group?.map((c) => c.id)), [drag?.group]);
  const fading = gesture?.moved && isFade(gesture.mode) ? gesture : null;

  // When the focused clip is deleted, focus and select the clip that followed it on its track, else
  // the one before it, so the keyboard keeps its place. Neighbours come from the order shown before
  // the deletion, so a clip moved meanwhile counts where it was.
  useLayoutEffect(() => {
    const before = shown.current;
    shown.current = project;
    const id = focusPlace.current;
    if (!project || !before || !id) return;
    if (findClip(project, id)) {
      // A clip moved to another track is drawn anew there and keeps the focus.
      if (!document.activeElement || document.activeElement === document.body) {
        if (restacked.current === id) focusClipIn(scroller.current, id);
        else scroller.current?.querySelector<HTMLElement>(`[data-clip-id="${id}"]`)?.focus({ preventScroll: true });
      }
      restacked.current = null;
      return;
    }
    focusPlace.current = null;
    if (document.activeElement && document.activeElement !== document.body) return;
    const clips = [...(findClip(before, id)?.track.clips ?? [])].sort((a, b) => a.startUs - b.startUs);
    const at = clips.findIndex((c) => c.id === id);
    const next = [...clips.slice(at + 1), ...clips.slice(0, at).reverse()].find((c) => findClip(project, c.id));
    if (!next) return;
    select([next.id]);
    scroller.current?.querySelector<HTMLElement>(`[data-clip-id="${next.id}"]`)?.focus({ preventScroll: true });
  }, [project, select]);

  // A preview can take the focused clip off screen for a moment; when it ends, the clip gets focus back.
  useEffect(() => {
    const id = focusPlace.current;
    if (peeking || !id || (document.activeElement && document.activeElement !== document.body)) return;
    scroller.current?.querySelector<HTMLElement>(`[data-clip-id="${id}"]`)?.focus({ preventScroll: true });
  }, [peeking]);

  const openMenu = useCallback((e: React.MouseEvent, clip: Clip) => {
    e.preventDefault();
    if (!useEditor.getState().selection.includes(clip.id)) useEditor.getState().select([clip.id]);
    setMenu({ clipId: clip.id, x: e.clientX, y: e.clientY });
  }, []);

  if (!project || !drawn) return <section className="pane shrink-0 bg-well" style={{ height }} />;

  // The video ends at its last picture or text; music running past it is cut on export.
  const videoEnd = projectDuration(drawn);
  const endX = (videoEnd / US) * zoom;
  const pastEnd = videoEnd > 0 && duration > videoEnd;
  const step = tickStep(zoom);
  const firstTick = Math.floor(Math.max(0, view.left - 100) / zoom / step) * step;
  const lastTick = (view.left + view.width + 100) / zoom;
  const ticks: number[] = [];
  for (let t = firstTick; t <= lastTick; t += step) ticks.push(Math.round(t * 1000) / 1000);

  const layout = mainLayout(project, drag, minUs);
  const fadeUs = fading ? dragFade(fading) : 0;
  const fadeContent = fading?.clip.content.type === "media" ? fading.clip.content : null;
  const fades = fading && fadeContent ? { inUs: fading.mode === "fadeIn" ? fadeUs : fadeContent.fadeInUs, outUs: fading.mode === "fadeOut" ? fadeUs : fadeContent.fadeOutUs } : undefined;
  // A main-track clip trimmed at its end shows where it ends up; elsewhere the ghost follows the pointer.
  const ghostTiming = drag?.moved ? ((drag.mode !== "move" && layout?.timing.get(drag.clip.id)) || dragResult(drag, minUs)) : null;
  const gapBelow = drag?.moved && drag.target && typeof drag.target === "object" ? drag.target.below : null;
  const ghostTrackId = drag?.moved && !gapBelow ? (drag.target === undefined ? drag.trackId : (drag.target as string | null)) : null;
  const replacing = assetDrag ? replaceTarget(assetDrag.x, assetDrag.y, dragMedia) : null;
  // Media dropped on the magnetic main track goes in at a cut, so the line shows that cut.
  const place = assetDrag && !replacing ? dropAt(assetDrag.x, assetDrag.y) : null;
  const dropTime = !place
    ? null
    : place.trackId !== MAIN_TRACK
      ? place.startUs
      : (() => {
          const main = mainClips(project);
          const i = mainInsertIndex(main, place.startUs);
          return i < main.length ? main[i].startUs : main.reduce((end, c) => Math.max(end, c.startUs + c.durationUs), 0);
        })();
  const emptyTimeline = allClips(drawn).length === 0;
  // Until something sits over the video, an empty lane above it shows where a picture over it goes.
  const overlayLaneShown = !!project && !emptyTimeline && !cover && !peekShown && !project.tracks.some((t) => t.kind === "video" && t.id !== MAIN_TRACK);
  const overLane = !locked && ((!!place?.over && dragMedia?.kind !== "audio") || !!drag?.onLane);
  const laneRow = overlayLaneShown && (
    <div key="overlay-lane" data-lane className="relative flex shrink-0" style={{ height: OVERLAY_LANE_H + ROW_GAP }}>
      <div className="sticky left-0 z-[47] shrink-0 border-r border-white/[.07] bg-panel" style={{ width: HEADER_W }} />
      <div ref={overlayLane} data-lane data-testid="overlay-lane" className="relative flex-1" style={{ marginBlock: ROW_GAP / 2 }}>
        <div
          className={`pointer-events-none absolute inset-y-0 right-1 left-1 flex items-center rounded-[7px] border text-[12px] transition-colors duration-[120ms] ease-out ${
            overLane ? "border-accent bg-accent/15 text-fg" : "border-dashed border-white/15 text-muted"
          }`}
        >
          <span className="sticky px-3" style={{ left: HEADER_W }}>
            {locked ? AI_EDITING : overLane ? "Drop to put it over the video" : "Drag a video or picture here to put it over the video"}
          </span>
        </div>
        {place?.over && dropTime !== null && (
          <div data-testid="drop-line" className="pointer-events-none absolute inset-y-0 z-30 w-0.5 bg-accent" style={{ left: (dropTime / US) * zoom }} />
        )}
      </div>
    </div>
  );
  // One tab stop for all clips: the last focused, else the first selected, else the first main-track clip.
  const known = (id: string | null | undefined) => !!id && allClips(project).some((c) => c.id === id);
  const tabStop = known(focusId) ? focusId : known(selection[0]) ? selection[0] : (mainClips(project)[0]?.id ?? tracks.find((t) => t.clips.length > 0)?.clips[0]?.id ?? null);

  const focusClip = (id: string) => focusClipIn(scroller.current, id);

  // Arrows move between clips and select the one they reach, so Delete, S, Q, W and Ctrl+D act on
  // the clip with focus. Ctrl+arrows move focus only, to add clips with Shift+Enter; Alt+←/→
  // nudge and Alt+↑/↓ move it a layer; the Menu key or Shift+F10 opens the clip menu. The keys stop here, so the playhead does
  // not also step.
  const onClipKey = (e: React.KeyboardEvent) => {
    if (cover) return;
    const id = (e.target as HTMLElement).getAttribute?.("data-clip-id");
    const found = id ? tracks.flatMap((t) => t.clips.map((c) => [t, c] as const)).find(([, c]) => c.id === id) : undefined;
    if (!found || e.metaKey) return;
    const [track, clip] = found;
    const arrow = e.key === "ArrowLeft" || e.key === "ArrowRight" || e.key === "ArrowUp" || e.key === "ArrowDown";
    if ((e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      e.stopPropagation();
      if (!useEditor.getState().selection.includes(clip.id)) select([clip.id]);
      const r = (e.target as HTMLElement).getBoundingClientRect();
      setMenu({ clipId: clip.id, x: r.left + 8, y: r.bottom - 4 });
    } else if (e.ctrlKey) {
      if (!arrow || e.altKey || e.shiftKey) return;
      e.preventDefault();
      e.stopPropagation();
      const next = neighbour(tracks, track, clip, e.key);
      if (next) focusClip(next.id);
    } else if (arrow && e.altKey) {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "ArrowUp" || e.key === "ArrowDown") {
        if (!e.shiftKey) restack(clip.id, e.key === "ArrowUp", () => (restacked.current = clip.id));
      } else nudge(clip.id, e.key === "ArrowLeft" ? -1 : 1, e.shiftKey);
    } else if (arrow && !e.shiftKey) {
      e.preventDefault();
      e.stopPropagation();
      const next = neighbour(tracks, track, clip, e.key);
      if (next) {
        focusClip(next.id);
        select([next.id]);
      }
    } else if (e.key === "Enter") {
      e.preventDefault();
      const sel = useEditor.getState().selection;
      if (e.shiftKey) select(sel.includes(clip.id) ? sel.filter((s) => s !== clip.id) : [...sel, clip.id]);
      else select([clip.id]);
    }
  };
  // Changes the lanes draw out of view are named, with the side to scroll to; the timeline itself stays where it is.
  const peekLines = (() => {
    if (!described || !peekShown) return null;
    const lane: [number, number] = [view.left, view.left + view.width - HEADER_W];
    let before = 0;
    let after = 0;
    for (const t of drawn.tracks)
      for (const c of t.clips) {
        if (!described.marks.has(c.id)) continue;
        if (((c.startUs + c.durationUs) / US) * zoom < lane[0]) before++;
        else if ((c.startUs / US) * zoom > lane[1]) after++;
      }
    const off = (n: number, side: string) => `${n} changed clip${n === 1 ? "" : "s"} off screen, to the ${side}`;
    return [...described.lines, ...(before ? [off(before, "left")] : []), ...(after ? [off(after, "right")] : [])];
  })();
  const dragTrack = drag ? project.tracks.find((t) => t.id === drag.trackId) : undefined;
  const dragKind = dragTrack?.kind ?? null;
  // A sound clip's name has its own row, so its ghost needs an audio row's height to keep its waveform.
  const ghostHeight = dragKind === "audio" ? rowHeight(dragTrack!) : 44;
  const picked = selection.flatMap((id) => {
    const found = findClip(project, id);
    return found ? [found] : [];
  });
  const detachable = picked.filter((f) => !detachBlocker(project, f.clip)).map((f) => f.clip.id);
  const detachLabel = detachable.length
    ? detachable.length === 1
      ? "Detach audio"
      : `Detach audio of ${detachable.length} clips`
    : picked.length === 1
      ? `Detach audio: ${detachBlocker(project, picked[0].clip)?.toLowerCase()}`
      : picked.length
        ? "Detach audio: none of the selected clips has sound to detach"
        : "Detach audio: select a video first";
  const rotatable = picked.some((f) => f.track.kind !== "audio");
  const rotateLabel = rotatable ? "Rotate 90°" : picked.length ? "Rotate: audio has no picture to turn" : "Rotate: select a clip first";
  const deleteLabel = cut ? "Delete transition (Delete)" : selection.length ? "Delete selection (Delete)" : "Delete: select a clip first";
  // While the AI edits, the editing buttons say so instead of what they would need.
  const lockedLabel = (action: string) => `${action}: the AI is editing`;

  return (
    <section className="pane flex shrink-0 flex-col overflow-hidden bg-well" style={{ height }} aria-label="Timeline">
      <div className="flex h-11 shrink-0 items-center gap-1.5 px-2.5">
        {cover ? (
          <p className="truncate pl-1.5 text-[12px] text-muted">
            {coverFromImage ? "Moving the playhead keeps the image; Use frame takes this frame." : "Click or drag on the timeline to choose the cover's frame."}
          </p>
        ) : (
          <div className="bar flex items-center rounded-full">
            <HistoryButton redo={false} locked={locked} lines={peekShown && !peekShown.redo ? peekLines : null} onPeek={setPeek} />
            <HistoryButton redo locked={locked} lines={peekShown?.redo ? peekLines : null} onPeek={setPeek} />
            <span aria-hidden className="mx-1 h-4 w-px bg-white/[.12]" />
            <IconButton round label={locked ? lockedLabel("Split") : splitBlocked ? `Split: ${splitBlocked}` : "Split at playhead (S)"} disabled={locked || !!splitBlocked} onClick={splitAtPlayhead}>
              <Scissors size={16} />
            </IconButton>
            <IconButton round label={locked ? lockedLabel("Freeze frame") : freezeBlocked ? `Freeze frame: ${freezeBlocked}` : "Freeze frame (Shift+R)"} disabled={locked || !!freezeBlocked} onClick={() => void freezeAtPlayhead()}>
              <Snowflake size={16} />
            </IconButton>
            <IconButton
              round
              label={locked ? lockedLabel("Delete left") : splitBlocked ? `Delete left: ${splitBlocked}` : "Delete left of playhead (Q)"}
              disabled={locked || !!splitBlocked}
              onClick={() => deleteSide("left")}
            >
              <PanelLeftClose size={16} />
            </IconButton>
            <IconButton
              round
              label={locked ? lockedLabel("Delete right") : splitBlocked ? `Delete right: ${splitBlocked}` : "Delete right of playhead (W)"}
              disabled={locked || !!splitBlocked}
              onClick={() => deleteSide("right")}
            >
              <PanelRightClose size={16} />
            </IconButton>
            <IconButton round label={locked ? lockedLabel("Delete") : deleteLabel} disabled={locked || (selection.length === 0 && !cut)} onClick={deleteSelection}>
              <Trash2 size={16} />
            </IconButton>
            <span aria-hidden className="mx-1 h-4 w-px bg-white/[.12]" />
            <IconButton
              round
              label={locked ? lockedLabel("Duplicate") : selection.length ? "Duplicate (Ctrl+D)" : "Duplicate: select a clip first"}
              disabled={locked || selection.length === 0}
              onClick={duplicateSelection}
            >
              <Copy size={15} />
            </IconButton>
            <IconButton round label={locked ? lockedLabel("Detach audio") : detachLabel} disabled={locked || detachable.length === 0} onClick={() => void detachAudio(detachable)}>
              <Unlink size={15} />
            </IconButton>
            <IconButton round label={locked ? lockedLabel("Rotate") : rotateLabel} disabled={locked || !rotatable} onClick={rotateSelection}>
              <RotateCwSquare size={15} />
            </IconButton>
            <span aria-hidden className="mx-1 h-4 w-px bg-white/[.12]" />
            <IconButton round label={snapping ? "Snapping on" : "Snapping off"} active={snapping} onClick={() => setSnapping(!snapping)}>
              <Magnet size={16} />
            </IconButton>
          </div>
        )}
        <div className="flex-1" />
        <IconButton round label="Zoom out (-)" onClick={() => setZoom(zoom / 1.3)}>
          <ZoomOut size={16} />
        </IconButton>
        <RangeInput
          label="Timeline zoom"
          min={Math.log(4)}
          max={Math.log(600)}
          step={0.01}
          value={Math.log(zoom)}
          valueText={`${Math.round(zoom)} pixels per second`}
          onChange={(v) => setZoom(Math.exp(v))}
          className="w-24"
        />
        <IconButton round label="Zoom in (+)" onClick={() => setZoom(zoom * 1.3)}>
          <ZoomIn size={16} />
        </IconButton>
        <IconButton
          round
          label="Fit timeline to view"
          disabled={duration === 0}
          onClick={() => {
            const w = (scroller.current?.clientWidth ?? 800) - HEADER_W - 40;
            zoomAnchor.current = { t: 0, px: 0 };
            setZoom(w / Math.max(1, duration / US));
            if (useEditor.getState().zoom === zoom && scroller.current) {
              zoomAnchor.current = null;
              scroller.current.scrollLeft = 0;
            }
          }}
        >
          <Maximize2 size={15} />
        </IconButton>
      </div>

      {/* Inset by 1 px, so the pane's hairline stays visible beside the opaque track headers. */}
      <div
        ref={scroller}
        className="relative mx-px mb-px min-h-0 flex-1 overflow-auto"
        onScroll={(e) => setView({ left: e.currentTarget.scrollLeft, width: e.currentTarget.clientWidth })}
      >
        {/*
          Layers, bottom to top: lanes and clips, the ruler (40), the playhead (45), the track header
          column (47) and the corner above it (48). The corner is not part of the ruler: the ruler is
          its own stacking context, and the playhead has to pass over the ruler but under the corner.
        */}
        <div className="relative flex flex-col" style={{ width: HEADER_W + laneWidth, minHeight: "100%" }}>
          <div className="sticky left-0 top-0 z-[48] shrink-0 border-b border-r border-white/[.07] bg-panel" style={{ width: HEADER_W, height: RULER_H, marginBottom: -RULER_H }} />
          {/* Ruler */}
          <div className="sticky top-0 z-40 flex shrink-0" style={{ height: RULER_H }}>
            <div className="shrink-0" style={{ width: HEADER_W }} />
            {/* Clipped, or labels past the lanes' end would widen the scroll, which draws more labels, without end. */}
            <div data-ruler className="relative flex-1 cursor-pointer overflow-x-clip border-b border-white/[.07] bg-well" onPointerDown={startScrub}>
              {ticks.map((t) => (
                <div key={t} className="absolute top-0 h-full" style={{ left: t * zoom }}>
                  <span className="tabular absolute left-1 top-[5px] text-[11px] leading-[13px] text-muted">{formatTime(t * US, step < 1)}</span>
                  <div className="absolute bottom-0 h-2 w-px bg-subtle" />
                  {[1, 2, 3, 4].map((i) => (
                    <div key={i} className="absolute bottom-0 h-1 w-px bg-subtle/60" style={{ left: (i * step * zoom) / 5 }} />
                  ))}
                </div>
              ))}
              {pastEnd && (
                <div className="absolute inset-y-0 w-2 -translate-x-1/2" style={{ left: endX }} title={END_TIP}>
                  <div className="mx-auto h-full w-0 border-l border-dashed border-muted" />
                </div>
              )}
              {/* The frames Pick for me found, as dots along the bottom of the ruler. */}
              {cover &&
                candidates.map((c) => (
                  <span
                    key={c.timeUs}
                    className="absolute bottom-0.5 h-1.5 w-1.5 -translate-x-1/2 rounded-full bg-fg/60 shadow-[0_0_0_1px_rgb(0_0_0/.5)]"
                    style={{ left: (c.timeUs / US) * zoom }}
                    title={`${formatTime(c.timeUs)} · ${Math.round(c.score * 100)} · ${candidateReason(c, cover)}`}
                  />
                ))}
            </div>
          </div>

          {/* Tracks */}
          <div
            className="relative flex flex-1 flex-col"
            onPointerDown={(e) => (e.target as HTMLElement).hasAttribute("data-lane") && startBoxSelect(e)}
            onKeyDown={onClipKey}
            onFocus={(e) => {
              const id = (e.target as HTMLElement).getAttribute?.("data-clip-id");
              if (!id) return;
              setFocusId(id);
              focusPlace.current = id;
            }}
            // A removed element fires no blur, so a deleted clip keeps its place for the effect below.
            onBlur={(e) => e.target.isConnected && (focusPlace.current = null)}
          >
            {drag?.moved && drag.target === null && !drag.onLane && dragKind !== "audio" && <div className="ml-[152px] h-0.5 rounded-full bg-accent" title="Drop to create a new track" />}
            {tracks.map((track) => {
              const h = rowHeight(track);
              const isMain = track.id === MAIN_TRACK;
              const row = (
                <div key={track.id} data-lane className="relative flex shrink-0" style={{ height: h + ROW_GAP }}>
                  <TrackHeader track={track} width={HEADER_W} locked={locked} />
                  <div
                    ref={(el) => {
                      if (el) rows.current.set(track.id, el);
                      else rows.current.delete(track.id);
                    }}
                    data-lane
                    role="listbox"
                    aria-label={`${track.name || track.kind} track`}
                    aria-multiselectable
                    className={`relative flex-1 ${isMain ? "bg-white/[0.035]" : "bg-white/[0.015]"} ${assetDrag && dropTime !== null && !place?.over ? "outline-1 outline-dashed outline-white/20" : ""}`}
                    style={{ marginBlock: ROW_GAP / 2 }}
                  >
                    {isMain && emptyTimeline && !peekShown && (
                      <div className="pointer-events-none absolute inset-1 flex items-center justify-center rounded-[7px] border border-dashed border-white/15 text-[12px] text-muted">
                        Drag media here, or press + on a media item
                      </div>
                    )}
                    {track.clips.map((clip) => {
                      if (drag?.moved && (drag.clip.id === clip.id || groupIds.has(clip.id))) {
                        // Where the main track's layout places the clip, its old spot goes; otherwise it stays outlined.
                        if (isMain && layout) return null;
                        return (
                          <div
                            key={clip.id}
                            className="absolute top-1 bottom-1 rounded-[7px] border border-dashed border-muted/50"
                            style={{ left: (clip.startUs / US) * zoom + 1, width: (clip.durationUs / US) * zoom - 2 }}
                          />
                        );
                      }
                      return (
                        <ClipView
                          key={clip.id}
                          clip={clip}
                          track={track}
                          project={drawn}
                          zoom={zoom}
                          height={h}
                          visible={visible}
                          selected={!peekShown && selection.includes(clip.id)}
                          locked={locked || !!peekShown}
                          mark={described?.marks.get(clip.id) ?? pluginMarks?.get(clip.id)}
                          replacing={clip.id === replacing?.clipId ? (replacing.short ? "short" : "replace") : undefined}
                          dimmed={!!peekShown && !described?.marks.has(clip.id)}
                          tabbable={!cover && clip.id === tabStop}
                          timing={isMain ? layout?.timing.get(clip.id) : undefined}
                          fades={fading?.clip.id === clip.id ? fades : undefined}
                          onPointerDown={peekShown ? undefined : startClipDrag}
                          onContextMenu={peekShown ? undefined : openMenu}
                        />
                      );
                    })}
                    {/* Where a moved clip would go in on the main track: its slot and an insertion bar. */}
                    {isMain && drag?.moved && layout?.slotUs != null && (
                      <>
                        <div
                          className="pointer-events-none absolute top-1 bottom-1 rounded-[7px] border border-dashed border-muted/50"
                          style={{ left: (layout.slotUs / US) * zoom + 1, width: (drag.clip.durationUs / US) * zoom - 2 }}
                        />
                        <div className="pointer-events-none absolute -top-0.5 -bottom-0.5 z-20 w-0.5 -translate-x-1/2 rounded-full bg-accent" style={{ left: (layout.slotUs / US) * zoom }} />
                      </>
                    )}
                    {isMain && !drag?.moved && !peekShown && <CutMarkers project={project} zoom={zoom} locked={locked} />}
                    {drag?.moved && ghostTiming && ghostTrackId === track.id && (
                      <ClipView clip={drag.clip} track={track} project={project} zoom={zoom} height={h} visible={visible} selected ghost timing={ghostTiming} />
                    )}
                    {/* The group's ghosts keep their own times and shift together, so they do not re-render as the pointer moves.
                        The layer has no width of its own, so shifting it does not widen the lanes. */}
                    {drag?.moved && ghostTiming && drag.group && (
                      <div className="pointer-events-none absolute inset-y-0 left-0 z-30" style={{ transform: `translateX(${((ghostTiming.startUs - drag.clip.startUs) / US) * zoom}px)` }}>
                        {track.clips
                          .filter((clip) => groupIds.has(clip.id))
                          .map((clip) => (
                            <ClipView key={clip.id} clip={clip} track={track} project={project} zoom={zoom} height={h} visible={visible} selected ghost />
                          ))}
                      </div>
                    )}
                  </div>
                  {gapBelow === track.id && ghostTiming && (
                    <div
                      className="pointer-events-none absolute -bottom-0.5 z-20 h-1 rounded-full bg-accent"
                      title="Drop to create a new track here"
                      data-testid="track-gap"
                      style={{ left: HEADER_W + (ghostTiming.startUs / US) * zoom, width: Math.max(4, (ghostTiming.durationUs / US) * zoom) }}
                    />
                  )}
                </div>
              );
              return isMain && laneRow ? [laneRow, row] : row;
            })}
            {drag?.moved && drag.target === null && dragKind === "audio" && <div className="ml-[152px] h-0.5 rounded-full bg-accent" />}
            {/* The header column runs on below the last track, so nothing scrolled under it shows through. */}
            <div data-lane className="flex flex-1">
              <div className="sticky left-0 z-[47] shrink-0 border-r border-white/[.07] bg-panel" style={{ width: HEADER_W }} />
            </div>
          </div>

          {/* In the cover editor the lanes only choose the frame; clips cannot be selected, moved or trimmed. */}
          {cover && <div className="absolute bottom-0 right-0 z-[41] cursor-pointer" style={{ left: HEADER_W, top: RULER_H }} onPointerDown={startScrub} data-testid="cover-scrub" />}

          {/* Ghost for a drag onto a new track follows the pointer row-less. */}
          {drag?.moved && drag.target === null && ghostTiming && (
            <div className="pointer-events-none absolute z-30" style={{ left: HEADER_W, top: RULER_H, height: ghostHeight, width: laneWidth }}>
              <ClipView
                clip={drag.clip}
                track={dragTrack!}
                project={project}
                zoom={zoom}
                height={ghostHeight}
                visible={visible}
                selected
                ghost
                timing={ghostTiming}
              />
            </div>
          )}

          {/* Past the video's end: dimmed above the clips, below the track headers. */}
          {pastEnd && (
            <div
              className="pointer-events-none absolute bottom-0 z-[15] border-l border-dashed border-muted bg-black/30"
              style={{ left: HEADER_W + endX, top: RULER_H, width: laneWidth - endX }}
            />
          )}

          {box && <div data-box className="pointer-events-none absolute z-[35] rounded-[3px] border border-accent bg-accent/15 shadow-[0_0_0_1px_rgb(0_0_0/.45)]" style={box} />}

          {/* Snap guide */}
          {drag?.moved && drag.snapUs !== null && (
            <div className="pointer-events-none absolute bottom-0 top-0 z-30 w-px bg-warn shadow-[0_0_0_1px_rgba(0,0,0,0.55)]" style={{ left: HEADER_W + (drag.snapUs / US) * zoom }} />
          )}

          {/* Drop position for media dragged from the panel */}
          {dropTime !== null && !place?.over && <div data-testid="drop-line" className="pointer-events-none absolute bottom-0 top-0 z-30 w-0.5 bg-accent" style={{ left: HEADER_W + (dropTime / US) * zoom }} />}

          <Playhead zoom={zoom} scroller={scroller} follow={!box} />

          {drag?.moved && ghostTiming && (
            // Pinned to the edge being dragged: the start for a move or left trim, the end for a right trim.
            <div
              className={`tabular pointer-events-none absolute z-50 rounded-md bg-black/80 px-1.5 py-0.5 text-[11px] font-medium text-fg ${drag.mode === "trimR" ? "-translate-x-full" : ""}`}
              style={{ left: HEADER_W + ((drag.mode === "trimR" ? ghostTiming.startUs + ghostTiming.durationUs : ghostTiming.startUs) / US) * zoom, top: 2 }}
            >
              {drag.mode === "move" ? formatTime(layout?.slotUs ?? ghostTiming.startUs) : formatDuration(ghostTiming.durationUs)}
            </div>
          )}
          {fading && (
            // Pinned to the end of the fade, like a trim's tooltip to its edge.
            <div
              className={`tabular pointer-events-none absolute z-50 rounded-md bg-black/80 px-1.5 py-0.5 text-[11px] font-medium text-fg ${fading.mode === "fadeOut" ? "-translate-x-full" : ""}`}
              style={{ left: HEADER_W + ((fading.mode === "fadeIn" ? fading.clip.startUs + fadeUs : fading.clip.startUs + fading.clip.durationUs - fadeUs) / US) * zoom, top: 2 }}
            >
              {fading.mode === "fadeIn" ? "Fade in" : "Fade out"} {formatDuration(fadeUs)}
            </div>
          )}
        </div>
      </div>
      {menu && <ClipMenu at={menu} onClose={closeMenu} />}
    </section>
  );
}

import { useEffect, useRef, useState } from "react";
import { AlertCircle, Loader2 } from "lucide-react";
import { api, errorText, plainError } from "../../lib/api";
import { COVER_FORMATS, frameAt, chooseFrame, coverFormat, downloadModels, draftCover, editCover, editOverlay, editText, mask, selectOverlay, selectText, switchFormat, thumbnailOf, useCover } from "../../lib/cover";
import { useEffectsVersion } from "../../lib/plugins";
import { AI_EDITING, projectDuration, useAiLocked, useEditor } from "../../lib/store";
import { formatTime } from "../../lib/time";
import type { CoverView, LayerBounds, Snapshot, Thumbnail, ThumbnailFormat, Transform } from "../../lib/types";
import { LayerBox, type LayerSource } from "../preview/LayerOverlay";
import { Button, Segmented } from "../ui";

/** A text the person covers more of than this gets a warning: the hook should still read at a glance. */
export const COVERED_TOO_MUCH = 0.3;

/** The cover being edited: the project's, or the one the editor offers at the playhead before there is any. */
export function useShownCover(format: ThumbnailFormat): { cover: Thumbnail; draft: boolean } | null {
  const project = useEditor((s) => s.snap?.project);
  const timeUs = useEditor((s) => s.timeUs);
  const models = useCover((s) => s.models);
  if (!project || projectDuration(project) <= 0) return null;
  const cover = thumbnailOf(project, format);
  if (cover) return { cover, draft: false };
  // Recomputed per render: `models` decides whether a YouTube thumbnail of a vertical video starts blurred.
  void models;
  return { cover: draftCover(format, project, frameAt(project, timeUs)), draft: true };
}

/** Why the cover cannot be drawn; a missing image on it can also be taken off. */
function coverError(text: string, format: ThumbnailFormat) {
  const name = text.startsWith("MEDIA_MISSING: ") ? text.slice("MEDIA_MISSING: ".length).split(/[\\/]/).pop() : null;
  const project = useEditor.getState().snap?.project;
  const onCover = thumbnailOf(project, format)?.overlays?.some((o) => project?.assets.find((a) => a.id === o.assetId)?.name === name);
  return name && onCover ? `${name} is missing. Put the file back where it was, or take the image off the cover.` : plainError(text);
}

/** Asks the engine for the cover one request at a time; a change while one renders asks again once it is done. */
function useCoverView(format: ThumbnailFormat, width: number, draft: Thumbnail | null) {
  const revision = useEditor((s) => s.snap?.revision ?? 0);
  const masks = useCover((s) => s.masks);
  const effects = useEffectsVersion();
  const canvas = useRef<HTMLCanvasElement>(null);
  const [view, setView] = useState<CoverView | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** The project revision the cover on screen was drawn from. */
  const [drawnFrom, setDrawnFrom] = useState(-1);
  const state = useRef({ busy: false, again: false, masks: -1, shown: -1, waiters: [] as { revision: number; done: () => void }[] });
  const args = useRef({ format, width, draft, revision, masks });
  args.current = { format, width, draft, revision, masks };
  const draftKey = draft ? JSON.stringify(draft) : "";

  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    const s = state.current;
    // One request at a time; the newest arguments are read when it starts, and a change meanwhile asks again.
    const request = () => {
      if (s.busy) return void (s.again = true);
      s.busy = true;
      const a = args.current;
      const refresh = a.masks !== s.masks;
      api
        .coverView(a.format, a.width, refresh, a.draft)
        .then(({ view, rgba }) => {
          if (!mounted.current) return;
          s.masks = a.masks;
          const el = canvas.current;
          if (el) {
            if (el.width !== view.width || el.height !== view.height) {
              el.width = view.width;
              el.height = view.height;
            }
            el.getContext("2d")?.putImageData(new ImageData(rgba, view.width, view.height), 0, 0);
          }
          setView(view);
          setDrawnFrom(a.revision);
          setError(null);
          useCover.setState({ hidden: view.maskMissing ? [] : view.hidden });
        })
        .catch((e) => mounted.current && setError(coverError(errorText(e), a.format)))
        .finally(() => {
          s.busy = false;
          s.shown = a.revision;
          s.waiters = s.waiters.filter((w) => (w.revision <= a.revision ? (w.done(), false) : true));
          if (mounted.current && s.again) {
            s.again = false;
            request();
          }
        });
    };
    request();
  }, [format, width, revision, masks, draftKey, effects]);

  /** Resolves once a cover drawn from `revision` or later is on screen. */
  const shownFrom = (revision: number) => (state.current.shown >= revision ? Promise.resolve() : new Promise<void>((done) => state.current.waiters.push({ revision, done })));
  return { canvas, view, error, shownFrom, drawnFrom };
}

const OVERLAY = "overlay:";

/** The cover in place of the video: drawn by the engine as the export draws it, with its frame, images and texts to drag. */
export function CoverStage({ format, box }: { format: ThumbnailFormat; box: { w: number; h: number } }) {
  const { width: tw, height: th } = coverFormat(format);
  const aspect = tw / th;
  const fit = box.w / box.h > aspect ? { h: box.h, w: box.h * aspect } : { w: box.w, h: box.w / aspect };
  const shown = useShownCover(format);
  const dpr = window.devicePixelRatio || 1;
  // In steps, so resizing the window does not ask for a new cover on every pixel.
  const width = Math.max(16, Math.min(tw, Math.ceil((fit.w * dpr) / 32) * 32));
  const { canvas, view, error, shownFrom, drawnFrom } = useCoverView(format, width, shown?.draft ? shown.cover : null);
  const selected = useCover((s) => s.selected);

  // In the order the cover is drawn, so a click takes what is on top: texts behind the person, then images, then the rest.
  const behind = (i: number) => !view?.maskMissing && !!shown?.cover.texts[i]?.behind;
  const texts = (under: boolean) => view?.bounds.flatMap((b, i) => (b && behind(i) === under ? [{ clipId: String(i), corners: b, frame: b }] : [])) ?? [];
  const layers: LayerBounds[] = view
    ? [
        ...(shown && !shown.draft ? [{ clipId: "frame", corners: view.frame[0], frame: view.frame[1] }] : []),
        ...texts(true),
        ...view.overlays.flatMap((b, i) => (b ? [{ clipId: `${OVERLAY}${i}`, corners: b[0], frame: b[1] }] : [])),
        ...texts(false),
      ]
    : [];
  const latest = useRef(layers);
  latest.current = layers;
  // A drag can start on a draft text while selecting it is still making the cover.
  const cover = () => thumbnailOf(useEditor.getState().snap?.project, format) ?? shown?.cover;
  const source: LayerSource = {
    canvas: { width: tw, height: th },
    bounds: layers,
    current: () => Promise.resolve(latest.current),
    settled: (last) =>
      (last as Promise<Snapshot | null>)
        .then((snap) => (snap ? shownFrom(snap.revision) : undefined))
        .then(() => latest.current),
    onBounds: () => {},
    selected: selected === null ? null : String(selected),
    isOnlySelected: (id) => String(useCover.getState().selected) === id,
    select: (id) =>
      id === null || id === "frame"
        ? useCover.setState({ selected: id })
        : id.startsWith(OVERLAY)
          ? void selectOverlay(format, Number(id.slice(OVERLAY.length)))
          : void selectText(format, Number(id)),
    transform: (id) => {
      const c = cover();
      if (id.startsWith(OVERLAY)) return c?.overlays?.[Number(id.slice(OVERLAY.length))]?.transform ?? null;
      return (id === "frame" ? c?.frame : c?.texts[Number(id)]?.transform) ?? null;
    },
    change: (id, patch: Partial<Transform>, key) =>
      id === "frame"
        ? editCover(format, (c) => (c ? { ...c, frame: { ...c.frame, ...patch } } : null), key)
        : id.startsWith(OVERLAY)
          ? editOverlay(format, Number(id.slice(OVERLAY.length)), (o) => ({ ...o, transform: { ...o.transform, ...patch } }), key)
          : editText(format, Number(id), (t) => ({ ...t, transform: { ...t.transform, ...patch } }), key),
    croppable: () => false,
    radius: () => 0,
    locked: () => !!useEditor.getState().aiRun,
    hidden: false,
  };

  if (!shown)
    return (
      <div className="absolute inset-0 flex items-center justify-center p-6 text-center text-[13px] text-muted">Add a video to the timeline to make a cover.</div>
    );
  return (
    <div className="absolute inset-0 flex items-center justify-center">
      <div
        className="relative"
        style={{ width: fit.w, height: fit.h }}
        data-testid="cover-stage"
        data-time={view?.timeUs}
        data-revision={drawnFrom}
        data-mask={view?.maskMissing ? "missing" : "ready"}
      >
        {/* Square corners and only a hairline, as the video's frame. */}
        <div className="absolute inset-0 overflow-hidden bg-black shadow-[0_0_0_1px_rgb(255_255_255/.07)]">
          <canvas ref={canvas} className="h-full w-full" aria-label={`${coverFormat(format).label} as it exports`} data-testid="cover-canvas" />
          {!view && !error && <div className="skeleton absolute inset-0 opacity-60" aria-label="Drawing the cover" />}
          {error && (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-black/80 p-6 text-center" role="alert">
              <AlertCircle size={18} className="text-danger" />
              <p className="text-[13px] text-fg">{error}</p>
            </div>
          )}
        </div>
        {view && <LayerBox width={fit.w} height={fit.h} bleed={{ x: (box.w - fit.w) / 2, y: (box.h - fit.h) / 2 }} source={source} />}
        <StatusChip view={view} />
      </div>
    </div>
  );
}

/** One chip at the top left of the cover for what it is waiting for, with one way on. */
function StatusChip({ view }: { view: CoverView | null }) {
  const models = useCover((s) => s.models);
  const job = useEditor((s) => {
    const { maskJob, modelsJob } = useCover.getState();
    const id = [maskJob, modelsJob].find((j) => j && s.jobs[j]?.status === "running");
    return id ? s.jobs[id] : null;
  });
  const failed = useCover((s) => s.failed);
  const damaged = useCover((s) => s.damaged);
  const locked = useEditor((s) => !!s.aiRun);
  const missing = !!view?.maskMissing;
  const tried = useRef(new Set<string>());

  // The person is cut out a moment after the picture stops changing, when the models are there.
  useEffect(() => {
    if (!view || !missing || !models?.downloaded || job || locked) return;
    const t = view.timeUs;
    if (tried.current.has(view.source)) return;
    const timer = window.setTimeout(() => {
      tried.current.add(view.source);
      void mask(t);
    }, 500);
    return () => window.clearTimeout(timer);
  }, [view, missing, models, job, locked]);

  const chip = "absolute left-2 top-2 z-10 flex h-7 max-w-[calc(100%-16px)] items-center gap-1.5 rounded-lg bg-black/70 pl-2 text-[12px] text-fg";
  if (job)
    return (
      <div className={`${chip} pr-1`} role="status">
        <Loader2 size={13} className="shrink-0 animate-spin text-accent" />
        <span className="tabular truncate">
          {job.phase ?? job.label}
          {job.progress > 0 ? ` ${Math.round(job.progress * 100)}%` : "…"}
        </span>
        <Button pill className="h-5 shrink-0 px-2 text-[11px]" onClick={() => void api.cancelJob(job.id)}>
          Stop
        </Button>
      </div>
    );
  if (!missing) return null;
  if (models?.unavailable)
    return (
      <div className={`${chip} pr-2`} title={models.unavailable}>
        <span className="truncate text-muted">Cutting out the person doesn't work on this computer</span>
      </div>
    );
  if (failed)
    return (
      <div className={`${chip} pr-1`} role="alert">
        <AlertCircle size={13} className="shrink-0 text-danger" />
        <span className="truncate" title={failed}>
          {failed}
        </span>
        <Button pill className="h-5 shrink-0 px-2 text-[11px]" onClick={() => (models?.downloaded && !damaged ? view && mask(view.timeUs) : downloadModels())}>
          {damaged ? "Download again" : "Try again"}
        </Button>
      </div>
    );
  if (models && !models.downloaded)
    return (
      <div className={`${chip} pr-1`}>
        <span className="truncate">Cutting out the person needs the cover models</span>
        <Button pill className="h-5 shrink-0 px-2 text-[11px]" onClick={() => void downloadModels()}>
          Download {models.sizeMb} MB
        </Button>
      </div>
    );
  return (
    <div className={`${chip} pr-1`}>
      <span className="truncate">Not cut out yet</span>
      <Button pill className="h-5 shrink-0 px-2 text-[11px]" onClick={() => view && void mask(view.timeUs)}>
        Cut out
      </Button>
    </div>
  );
}

/** In place of the transport: the frame's time, or the image the cover is made from, and the format. */
export function CoverBar({ format }: { format: ThumbnailFormat }) {
  const time = useEditor((s) => thumbnailOf(s.snap?.project, format)?.timeUs ?? s.timeUs);
  const image = useEditor((s) => {
    const id = thumbnailOf(s.snap?.project, format)?.image;
    return id ? (s.snap?.project.assets.find((a) => a.id === id)?.name ?? null) : null;
  });
  const locked = useAiLocked();
  return (
    <div className="bar grid h-11 w-full max-w-[620px] grid-cols-[1fr_auto_1fr] items-center gap-2 rounded-full pl-[18px] pr-1.5" data-testid="cover-bar">
      {image ? (
        <span className="flex min-w-0 items-center gap-1.5 text-[13px]">
          <span className="font-medium text-fg @max-[460px]:hidden">Image</span>
          <Button
            pill
            className="h-6 shrink-0 px-2.5 text-[12px]"
            title={`Replace ${image} with the frame at the playhead`}
            disabled={locked}
            disabledReason={AI_EDITING}
            onClick={() => void chooseFrame(format, useEditor.getState().timeUs)}
          >
            Use frame
          </Button>
        </span>
      ) : (
        <span className="tabular truncate text-[13px]">
          <span className="text-muted @max-[460px]:hidden">Frame </span>
          <span className="font-medium text-fg">{formatTime(time)}</span>
        </span>
      )}
      <div className="w-[200px] @max-[460px]:w-[112px]">
        <Segmented
          label="Cover format"
          value={format}
          onChange={switchFormat}
          options={COVER_FORMATS.map((f) => ({ id: f.id, label: <><span className="@max-[460px]:hidden">{f.label}</span><span className="hidden @max-[460px]:inline">{f.short}</span></>, title: `${f.width}×${f.height}` }))}
        />
      </div>
    </div>
  );
}

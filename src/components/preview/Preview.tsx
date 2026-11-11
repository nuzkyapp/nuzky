import { useEffect, useRef, useState } from "react";
import { AudioLines, Maximize, Minimize, Pause, Play, SkipBack, SkipForward, Smartphone, SquareSplitHorizontal, Volume2, VolumeX, X } from "lucide-react";
import { api } from "../../lib/api";
import { useCover } from "../../lib/cover";
import { toggleCompare, toggleFullPreview, toggleMuted, usePreview } from "../../lib/preview";
import { NO_ADJUST, sameAdjust } from "../../lib/presets";
import { pictureAspect, useReels } from "../../lib/reels";
import { closeSource, seekSource, sourceDuration, toggleSourcePlay, useSource } from "../../lib/source";
import { projectDuration, useEditor } from "../../lib/store";
import { formatTime } from "../../lib/time";
import type { SafeArea } from "../../lib/types";
import { CoverBar, CoverStage } from "../cover/CoverStage";
import { CropGuide, ReelBar } from "../reels/ReelBar";
import { AiRunBar } from "../TopBar";
import { CompareDivider } from "./CompareDivider";
import { IconButton, LockReason, RangeInput, trapTab } from "../ui";
import { KindIcon } from "../panel/assets";
import { LayerOverlay } from "./LayerOverlay";
import { PreviewDrop } from "./PreviewDrop";
import { RatioMenu } from "./RatioMenu";
import { FacesStatus } from "./FacesStatus";
import { ReframeStatus } from "./ReframeStatus";
import { useKeyLabels } from "../../lib/keys";

const HEADER = 24;
const SAFE_ZONE_KEY = "nuzky.safeZone";

/** The parts of the frame Reels and TikTok cover with their interface, dimmed, around the free area. */
function SafeZone({ area, width, height }: { area: SafeArea; width: number; height: number }) {
  const pct = (v: number, of: number) => `${(v / of) * 100}%`;
  const band = "pointer-events-none absolute bg-black/45";
  return (
    <>
      <div className={`${band} inset-x-0 top-0`} style={{ height: pct(area.top, height) }} />
      <div className={`${band} inset-x-0 bottom-0`} style={{ height: pct(height - area.bottom, height) }} />
      <div className={`${band} left-0`} style={{ top: pct(area.top, height), bottom: pct(height - area.bottom, height), width: pct(area.left, width) }} />
      <div className={`${band} right-0`} style={{ top: pct(area.top, height), bottom: pct(height - area.bottom, height), width: pct(width - area.right, width) }} />
      <div
        className="pointer-events-none absolute border border-dashed border-fg/50"
        style={{ left: pct(area.left, width), top: pct(area.top, height), width: pct(area.right - area.left, width), height: pct(area.bottom - area.top, height) }}
      />
    </>
  );
}

/** The project's playhead follows the frames while playing. */
const followPlayhead = (t: number, playing: boolean) => {
  // A frame rendered just before a pause can arrive after it; the pause already set the time.
  if (playing && useEditor.getState().playing) useEditor.setState({ timeUs: t });
};

/** Receives rendered frames from an engine and paints the newest one per display frame; `onFrame` gets its time. */
export function useFrameStream(url: string, canvas: React.RefObject<HTMLCanvasElement | null>, onFrame: (t: number, playing: boolean) => void) {
  const frame = useRef(onFrame);
  frame.current = onFrame;
  const [connected, setConnected] = useState(false);
  useEffect(() => {
    if (!url) return;
    let ws: WebSocket | null = null;
    let closed = false;
    let latest: ArrayBuffer | null = null;
    let raf = 0;
    let retry = 0;

    const draw = () => {
      raf = 0;
      const buf = latest;
      latest = null;
      const el = canvas.current;
      if (!buf || !el) return;
      const view = new DataView(buf);
      if (view.getUint32(0, true) !== 0x31465043) return; // "CPF1"
      const w = view.getUint32(4, true);
      const h = view.getUint32(8, true);
      const playing = (view.getUint32(12, true) & 1) === 1;
      const t = Number(view.getBigInt64(16, true));
      if (el.width !== w || el.height !== h) {
        el.width = w;
        el.height = h;
      }
      // Opaque: frames have no transparency, and an opaque canvas needs no blending with what is behind it.
      el.getContext("2d", { alpha: false })?.putImageData(new ImageData(new Uint8ClampedArray(buf, HEADER, w * h * 4), w, h), 0, 0);
      // The time of the frame on screen. After a seek the previous frame stays a moment; the UI flows wait for this.
      el.dataset.us = String(t);
      frame.current(t, playing);
    };

    const connect = () => {
      ws = new WebSocket(url);
      ws.binaryType = "arraybuffer";
      ws.onopen = () => {
        retry = 0;
        setConnected(true);
      };
      ws.onmessage = (e) => {
        latest = e.data as ArrayBuffer;
        if (!raf) raf = requestAnimationFrame(draw);
      };
      ws.onclose = () => {
        setConnected(false);
        if (!closed) window.setTimeout(connect, Math.min(2000, 200 * ++retry));
      };
    };
    connect();
    return () => {
      closed = true;
      ws?.close();
      cancelAnimationFrame(raf);
    };
  }, [url, canvas]);
  return connected;
}

/** Playhead time and length; the only part of the preview that follows playback frame by frame. */
function Timecode({ duration }: { duration: number }) {
  const now = formatTime(useEditor((s) => s.timeUs));
  return (
    <span className="tabular truncate pl-3 text-[13px]">
      <span className="text-[15px] font-semibold text-fg @max-[560px]:text-[13px]">
        {now.slice(0, -3)}
        <span className="font-medium text-muted">{now.slice(-3)}</span>
      </span>
      <span className="text-muted @max-[560px]:hidden"> / {formatTime(duration)}</span>
    </span>
  );
}

/** The media item's transport, laid out as the timeline's so Play stays where the hand expects it. */
function SourceTransport({ children }: { children: React.ReactNode }) {
  const { withKey } = useKeyLabels();
  const playing = useSource((s) => s.playing);
  const timeUs = useSource((s) => s.timeUs);
  const duration = useSource(sourceDuration);
  const picture = useSource((s) => s.asset?.kind === "image");
  const now = formatTime(timeUs);
  const idle = duration === 0;
  return (
    <div className="bar grid h-11 w-full max-w-[620px] grid-cols-[1fr_auto_1fr] items-center gap-2 rounded-full px-1.5">
      <span className="tabular truncate pl-3 text-[13px]">
        {!picture && (
          <>
            <span className="text-[15px] font-semibold text-fg @max-[560px]:text-[13px]">
              {now.slice(0, -3)}
              <span className="font-medium text-muted">{now.slice(-3)}</span>
            </span>
            <span className="text-muted @max-[560px]:hidden"> / {formatTime(duration)}</span>
          </>
        )}
      </span>
      <div className="flex items-center gap-2.5">
        <IconButton round label={withKey("Go to start", "goStart")} onClick={() => seekSource(0)} disabled={idle} className="@max-[440px]:hidden">
          <SkipBack size={16} />
        </IconButton>
        <button
          type="button"
          aria-label={withKey(playing ? "Pause" : "Play", "playPause")}
          title={picture ? "A picture does not play" : withKey(playing ? "Pause" : "Play", "playPause")}
          disabled={idle}
          onClick={toggleSourcePlay}
          className="flex h-[38px] w-[38px] items-center justify-center rounded-full bg-fg text-black shadow-[0_1px_3px_rgb(0_0_0/.4)] transition-transform duration-[120ms] ease-out active:scale-95 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {playing ? <Pause size={16} fill="currentColor" /> : <Play size={16} fill="currentColor" className="translate-x-px" />}
        </button>
        <IconButton round label={withKey("Go to end", "goEnd")} onClick={() => seekSource(duration)} disabled={idle} className="@max-[440px]:hidden">
          <SkipForward size={16} />
        </IconButton>
      </div>
      <div className="flex items-center justify-end gap-1">{children}</div>
    </div>
  );
}

const SCRUB = 28;

/** The slider over the media item's length, under its frame, as the timeline is the timeline's. */
function SourceScrub({ width }: { width: number }) {
  const timeUs = useSource((s) => s.timeUs);
  const duration = useSource(sourceDuration);
  const fps = useSource((s) => s.preview?.fps || 30);
  const name = useSource((s) => s.asset?.name ?? "");
  return (
    // Seeking a file the user only looks at changes nothing, so the AI lock does not hold it.
    <div className="mt-3 shrink-0" style={{ width }}>
      <LockReason.Provider value={null}>
        <RangeInput label={`Position in ${name}`} value={timeUs} min={0} max={Math.max(duration, 1)} step={1e6 / fps} onChange={seekSource} disabled={duration === 0} valueText={formatTime(timeUs)} />
      </LockReason.Provider>
    </div>
  );
}

/** Which media item the preview shows, with the way back to the timeline. */
function SourceChip() {
  const asset = useSource((s) => s.asset);
  if (!asset) return null;
  return (
    <div className="absolute top-0 left-0 z-10 flex h-7 max-w-[min(60%,320px)] items-center gap-1.5 rounded-lg bg-black/70 pr-1 pl-2.5 text-[12px] text-fg">
      <span className="shrink-0 text-muted">
        <KindIcon kind={asset.kind} size={13} />
      </span>
      <span className="truncate">{asset.name}</span>
      <button
        type="button"
        aria-label="Back to the timeline (Esc)"
        title="Back to the timeline (Esc)"
        onClick={closeSource}
        className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-muted transition-colors duration-[120ms] ease-out hover:bg-white/15 hover:text-fg"
      >
        <X size={13} />
      </button>
    </div>
  );
}

export function Preview() {
  const { withKey } = useKeyLabels();
  const url = useEditor((s) => s.previewUrl);
  const canvas = useEditor((s) => s.snap?.project.canvas);
  const duration = useEditor((s) => (s.snap ? projectDuration(s.snap.project) : 0));
  const playing = useEditor((s) => s.playing);
  const engineError = useEditor((s) => s.engineError);
  // The cover editor shows the cover here in place of the video.
  const cover = useCover((s) => s.open);
  // Choosing reels shows what Center crop keeps, and the clips cannot be moved.
  const reels = useReels((s) => s.open);
  const crop = useReels((s) => s.framing === "crop");
  const reel = useReels((s) => s.current);
  const picture = useEditor((s) => (reels ? pictureAspect(s.snap?.project, reel) : 0));
  const home = useEditor((s) => s.view === "home");
  const adjusted = useEditor(
    (s) =>
      !!s.snap?.project.tracks.some((t) =>
        t.clips.some((c) => c.content.type === "media" && (!sameAdjust(c.content.adjust, NO_ADJUST) || (c.content.log ?? "none") !== "none" || c.content.lut?.type === "file")),
      ),
  );
  const { muted, full, compare } = usePreview();
  const { togglePlay, seek } = useEditor.getState();
  const sectionRef = useRef<HTMLElement>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [box, setBox] = useState({ w: 0, h: 0 });
  const connected = useFrameStream(url, canvasRef, followPlayhead);
  // A media item from the library shows in place of the timeline; the cover editor and reels keep the preview.
  const sourceAsset = useSource((s) => s.asset);
  const sourcePreview = useSource((s) => s.preview);
  const sourceError = useSource((s) => s.error);
  const source = sourceAsset && !cover && !reels ? sourceAsset : null;
  const sourceCanvasRef = useRef<HTMLCanvasElement>(null);
  const sourceConnected = useFrameStream(sourcePreview?.url ?? "", sourceCanvasRef, (t, framePlaying) => {
    if (framePlaying && useSource.getState().playing) useSource.setState({ timeUs: t });
  });
  const assetGone = useEditor((s) => !!sourceAsset && !s.snap?.project.assets.some((a) => a.id === sourceAsset.id && a.path === sourceAsset.path));
  useEffect(() => {
    if (cover || reels || assetGone) closeSource();
  }, [cover, reels, assetGone]);
  // Another view or project takes the comparison away rather than bringing it back on return.
  useEffect(() => {
    if (cover || reels || source || home) usePreview.setState({ compare: null });
  }, [cover, reels, source, home]);
  const deviceBox = useRef({ w: 0, h: 0 });
  useEffect(() => {
    if (sourcePreview && deviceBox.current.w > 0) void api.sourcePreviewResize(deviceBox.current.w, deviceBox.current.h);
  }, [sourcePreview]);

  useEffect(() => void api.setPreviewMuted(muted), [muted]);
  // The home screen sits under the preview's full screen; leaving the editor leaves full screen first.
  useEffect(() => {
    if (home && usePreview.getState().full) void toggleFullPreview();
  }, [home]);
  // Keys and Tab stay in the full screen preview instead of reaching the editor hidden behind it.
  useEffect(() => {
    if (!full) return;
    const back = document.activeElement as HTMLElement | null;
    sectionRef.current?.focus();
    return () => back?.focus();
  }, [full]);

  useEffect(() => {
    const el = boxRef.current;
    if (!el) return;
    let timer = 0;
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setBox({ w: width, h: height });
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        const dpr = window.devicePixelRatio || 1;
        deviceBox.current = { w: Math.round(width * dpr), h: Math.round(height * dpr) };
        api.setPreviewBox(deviceBox.current.w, deviceBox.current.h);
        if (useSource.getState().preview) void api.sourcePreviewResize(deviceBox.current.w, deviceBox.current.h);
      }, 80);
    });
    ro.observe(el);
    return () => {
      ro.disconnect();
      window.clearTimeout(timer);
    };
  }, []);

  const aspect = canvas ? canvas.width / canvas.height : 9 / 16;
  const area = canvas?.safeArea ?? null;
  const [showSafe, setShowSafe] = useState(() => localStorage.getItem(SAFE_ZONE_KEY) === "1");
  const toggleSafe = () => {
    localStorage.setItem(SAFE_ZONE_KEY, showSafe ? "0" : "1");
    setShowSafe(!showSafe);
  };
  const fit = box.w / box.h > aspect ? { h: box.h, w: box.h * aspect } : { w: box.w, h: box.w / aspect };
  const empty = duration === 0;
  const sourceAspect = sourcePreview ? sourcePreview.width / sourcePreview.height : source && source.width > 0 && source.height > 0 ? source.width / source.height : 16 / 9;
  const scrub = source && source.kind !== "image" ? SCRUB : 0;
  const sourceBox = { w: box.w, h: Math.max(0, box.h - scrub) };
  const sourceFit = sourceBox.w / sourceBox.h > sourceAspect ? { h: sourceBox.h, w: sourceBox.h * sourceAspect } : { w: sourceBox.w, h: sourceBox.w / sourceAspect };
  const muteButton = (
    <IconButton round label={withKey(muted ? "Unmute the preview" : "Mute the preview", "mute")} active={muted} onClick={toggleMuted}>
      {muted ? <VolumeX size={16} /> : <Volume2 size={16} />}
    </IconButton>
  );
  const fullButton = (
    <IconButton round label={withKey(full ? "Exit full screen" : "Full screen", "fullScreen")} onClick={() => void toggleFullPreview()}>
      {full ? <Minimize size={16} /> : <Maximize size={16} />}
    </IconButton>
  );

  return (
    <section
      ref={sectionRef}
      tabIndex={full ? -1 : undefined}
      onKeyDown={full ? trapTab : undefined}
      className={`@container flex min-w-0 flex-col outline-none ${full ? "fixed inset-0 z-[70] bg-black" : "flex-1 rounded-xl bg-stage shadow-[inset_0_0_0_1px_rgb(255_255_255/.05)]"}`}
      aria-label="Preview"
    >
      {/* The top bar is hidden; what the AI is doing and its Stop stay in view. */}
      {full && (
        <div className="flex justify-center pt-3 empty:hidden">
          <AiRunBar />
        </div>
      )}
      {/* Clips the selection box of a layer scaled or rotated past the frame to the preview area. */}
      <div ref={boxRef} className="relative mx-4 mt-3 min-h-0 flex-1 overflow-hidden">
        {cover && box.w > 0 && <CoverStage format={cover} box={box} />}
        {!cover && <ReframeStatus />}
        {!cover && <FacesStatus />}
        {source && (
          <>
            <div className="absolute inset-0 flex flex-col items-center justify-center">
              <div className="relative shrink-0 overflow-hidden bg-black shadow-[0_0_0_1px_rgb(255_255_255/.07)]" style={{ width: sourceFit.w, height: sourceFit.h }}>
                <canvas ref={sourceCanvasRef} className="h-full w-full" aria-label={`Preview of ${source.name}`} />
                {source.kind === "audio" && sourcePreview && (
                  <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-2 p-6 text-center text-muted">
                    <AudioLines size={32} />
                    <p className="max-w-full truncate text-[13px] text-fg">{source.name}</p>
                  </div>
                )}
                {!sourceError && (!sourcePreview || !sourceConnected) && <div className="skeleton absolute inset-0 opacity-60" aria-label={`Opening ${source.name}`} />}
                {sourceError && (
                  <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-black/80 p-6 text-center" role="alert">
                    <p className="text-[13px] font-medium text-danger">{source.name} can't be shown</p>
                    <p className="text-[12px] text-muted">{sourceError}</p>
                  </div>
                )}
              </div>
              {scrub > 0 && <SourceScrub width={Math.min(box.w, Math.max(sourceFit.w, 240))} />}
            </div>
            <SourceChip />
          </>
        )}
        <div className="absolute inset-0 flex items-center justify-center" hidden={!!cover || !!source}>
          <div className="relative" style={{ width: fit.w, height: fit.h }}>
            {/* Square corners and no blurred shadow: a rounded clip would cost a mask on every frame, and in
                WebKitGTK a blurred shadow on this box left the canvas black. */}
            <div className="absolute inset-0 overflow-hidden bg-black shadow-[0_0_0_1px_rgb(255_255_255/.07)]">
              <canvas ref={canvasRef} className="h-full w-full" style={{ imageRendering: "auto" }} />
              {showSafe && area && canvas && !reels && <SafeZone area={area} width={canvas.width} height={canvas.height} />}
              {reels && crop && canvas && picture > 0 && <CropGuide width={canvas.width} height={canvas.height} picture={picture} />}
              {empty && !engineError && (
                <div className="absolute inset-0 flex flex-col items-center justify-center gap-1 p-6 text-center">
                  <p className="text-[13px] text-fg">Your video appears here</p>
                  <p className="text-[12px] text-muted">Add media to the timeline to start editing.</p>
                </div>
              )}
              {!connected && !engineError && <div className="skeleton absolute inset-0 opacity-60" aria-label="Connecting to preview" />}
              {engineError && (
                <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-black/80 p-6 text-center" role="alert">
                  <p className="text-[13px] font-medium text-danger">Preview is unavailable</p>
                  <p className="text-[12px] text-muted">{engineError}</p>
                </div>
              )}
            </div>
            {/* Outside the clipped frame so handles of a layer larger than the canvas stay reachable. */}
            {!empty && !engineError && canvas && fit.w > 0 && !reels && compare === null && <LayerOverlay width={fit.w} height={fit.h} bleed={{ x: (box.w - fit.w) / 2, y: (box.h - fit.h) / 2 }} />}
            {!empty && !engineError && canvas && !reels && !cover && <PreviewDrop canvas={canvas} />}
            {compare !== null && !empty && !engineError && !reels && !cover && !source && fit.w > 0 && <CompareDivider width={fit.w} />}
          </div>
        </div>
      </div>
      <div className="flex shrink-0 justify-center px-5 py-2.5 @max-[460px]:px-2">
        {cover ? <CoverBar format={cover} /> : reels ? <ReelBar /> : source ? (
          <SourceTransport>
            {muteButton}
            {fullButton}
          </SourceTransport>
        ) : <div className="bar grid h-11 w-full max-w-[700px] grid-cols-[1fr_auto_1fr] items-center gap-2 rounded-full px-1.5">
          <Timecode duration={duration} />
          <div className="flex items-center gap-2.5">
            <IconButton round label={withKey("Go to start", "goStart")} onClick={() => seek(0)} disabled={empty} className="@max-[440px]:hidden">
              <SkipBack size={16} />
            </IconButton>
            <button
              type="button"
              aria-label={withKey(playing ? "Pause" : "Play", "playPause")}
              title={withKey(playing ? "Pause" : "Play", "playPause")}
              disabled={empty}
              onClick={togglePlay}
              className="flex h-[38px] w-[38px] items-center justify-center rounded-full bg-fg text-black shadow-[0_1px_3px_rgb(0_0_0/.4)] transition-transform duration-[120ms] ease-out active:scale-95 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {playing ? <Pause size={16} fill="currentColor" /> : <Play size={16} fill="currentColor" className="translate-x-px" />}
            </button>
            <IconButton round label={withKey("Go to end", "goEnd")} onClick={() => seek(duration)} disabled={empty} className="@max-[440px]:hidden">
              <SkipForward size={16} />
            </IconButton>
          </div>
          <div className="flex items-center justify-end gap-1">
            {muteButton}
            {/* Comparing changes nothing in the project, so it stays open to the user while an agent edits. */}
            <LockReason.Provider value={null}>
              <IconButton
                round
                label={compare !== null ? "Stop comparing" : adjusted ? "Compare before and after the colour" : "Nothing to compare: no clip has Log, a LUT, a filter or adjustments"}
                active={compare !== null}
                disabled={compare === null && !adjusted}
                onClick={toggleCompare}
              >
                <SquareSplitHorizontal size={16} />
              </IconButton>
            </LockReason.Provider>
            <IconButton
              round
              label={area ? (showSafe ? "Hide the Reels and TikTok safe zone" : "Show the Reels and TikTok safe zone") : "The safe zone applies to vertical videos"}
              active={showSafe && !!area}
              disabled={!area}
              onClick={toggleSafe}
            >
              <Smartphone size={16} />
            </IconButton>
            {canvas && <RatioMenu />}
            {fullButton}
          </div>
        </div>}
      </div>
    </section>
  );
}

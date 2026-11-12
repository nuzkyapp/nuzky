import { useEffect, useRef, useState } from "react";
import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Maximize, Minimize, Pause, Play, SkipBack, SkipForward } from "lucide-react";
import { api } from "../../lib/api";
import { useFrameStream } from "../../lib/frameStream";
import { FROM_PREVIEW_WINDOW, actionOf, chordsOf, suppressSpaceRelease, useKeyLabels, type ActionId } from "../../lib/keys";
import { formatTime } from "../../lib/time";
import type { PreviewProject, Transport } from "../../lib/types";
import { IconButton } from "../ui";

/** What the window shows, as the editor's engine reports it. The time changes with every frame, so only the timecode reads it. */
const useWindow = create(() => ({
  url: "",
  playing: false,
  timeUs: 0,
  project: { durationUs: 0, width: 1080, height: 1920 } as PreviewProject,
  engineError: null as string | null,
  full: false,
}));

const win = getCurrentWindow();
// WebKit fills what the page does not cover, such as the area under macOS's title bar, with the root's colour.
for (const root of [document.documentElement, document.body, document.getElementById("root")]) if (root) root.style.background = "#000";
const send = (action: ActionId) => void api.previewWindowAction(action);

async function toggleFull() {
  await win.setFullscreen(!(await win.isFullscreen()));
}

/** The playing frames carry their time; a frame drawn just before a pause can arrive after it. */
const followFrames = (t: number, playing: boolean) => {
  if (playing && useWindow.getState().playing) useWindow.setState({ timeUs: t });
};

function Timecode() {
  const now = formatTime(useWindow((s) => s.timeUs));
  const duration = useWindow((s) => s.project.durationUs);
  return (
    <span className="tabular truncate pl-3 text-[13px]">
      <span className="text-[15px] font-semibold text-fg">
        {now.slice(0, -3)}
        <span className="font-medium text-muted">{now.slice(-3)}</span>
      </span>
      <span className="text-muted @max-[560px]:hidden"> / {formatTime(duration)}</span>
    </span>
  );
}

/** The window's keys: its own full screen, closing it, and the editor's playback keys, which the editor runs. */
function useKeys() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const action = actionOf(chordsOf(e));
      if (e.key === "Escape") {
        if (useWindow.getState().full) void win.setFullscreen(false);
        return;
      }
      if (!action) return;
      e.preventDefault();
      if (e.key === " ") {
        if (e.repeat) return;
        suppressSpaceRelease();
      }
      if (action === "fullScreen") {
        if (!e.repeat) void toggleFull();
      } else if (action === "previewWindow") void api.closePreviewWindow();
      else if (FROM_PREVIEW_WINDOW.has(action) && !(e.repeat && (action === "playPause" || action === "mute"))) send(action);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

/** The preview in a window of its own: the editor's frames over black, with play, the time and full screen under them. */
export default function PreviewWindow() {
  const { withKey } = useKeyLabels();
  const { url, playing, engineError, full } = useWindow();
  const { durationUs, width, height } = useWindow((s) => s.project);
  const boxRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [box, setBox] = useState({ w: 0, h: 0 });
  const connected = useFrameStream(url, canvasRef, followFrames);
  useKeys();

  useEffect(() => {
    void api.previewWindowBoot().then((boot) =>
      useWindow.setState({ url: boot.previewUrl, playing: boot.transport.playing, timeUs: boot.transport.tUs, project: boot.project, engineError: boot.engineError }),
    );
    const offs = [
      listen<Transport>("transport", (e) => useWindow.setState({ playing: e.payload.playing, timeUs: e.payload.tUs })),
      listen<string | null>("engine-error", (e) => useWindow.setState({ engineError: e.payload })),
      listen<PreviewProject>("preview-window-project", (e) => useWindow.setState({ project: e.payload })),
      win.onResized(async () => useWindow.setState({ full: await win.isFullscreen() })),
    ];
    return () => offs.forEach((off) => void off.then((f) => f()));
  }, []);

  useEffect(() => {
    const el = boxRef.current;
    if (!el) return;
    let timer = 0;
    const ro = new ResizeObserver(([entry]) => {
      const { width: w, height: h } = entry.contentRect;
      setBox({ w, h });
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        const dpr = window.devicePixelRatio || 1;
        void api.setPreviewWindowBox(Math.round(w * dpr), Math.round(h * dpr));
      }, 80);
    });
    ro.observe(el);
    return () => {
      ro.disconnect();
      window.clearTimeout(timer);
    };
  }, []);

  const aspect = width / height;
  const fit = box.w / box.h > aspect ? { h: box.h, w: box.h * aspect } : { w: box.w, h: box.w / aspect };
  const empty = durationUs === 0;
  return (
    <main className="@container fixed inset-0 flex flex-col bg-black" aria-label="Preview window">
      <div ref={boxRef} className="relative mx-4 mt-3 min-h-0 flex-1 overflow-hidden">
        <div className="absolute inset-0 flex items-center justify-center">
          {/* Square corners and a hairline without a blurred shadow, as the editor's frame (DESIGN.md, Platform notes). */}
          <div className="relative overflow-hidden bg-black shadow-[0_0_0_1px_rgb(255_255_255/.07)]" style={{ width: fit.w, height: fit.h }} onDoubleClick={() => void toggleFull()}>
            <canvas ref={canvasRef} className="h-full w-full" />
            {empty && !engineError && (
              <div className="absolute inset-0 flex flex-col items-center justify-center gap-1 p-6 text-center">
                <p className="text-[13px] text-fg">Your video appears here</p>
                <p className="text-[12px] text-muted">Add media to the timeline in the editor.</p>
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
        </div>
      </div>
      <div className="flex shrink-0 justify-center px-5 py-2.5">
        <div className="bar grid h-11 w-full max-w-[620px] grid-cols-[1fr_auto_1fr] items-center gap-2 rounded-full px-1.5">
          <Timecode />
          <div className="flex items-center gap-2.5">
            <IconButton round label={withKey("Go to start", "goStart")} onClick={() => send("goStart")} disabled={empty} className="@max-[440px]:hidden">
              <SkipBack size={16} />
            </IconButton>
            <button
              type="button"
              aria-label={withKey(playing ? "Pause" : "Play", "playPause")}
              title={withKey(playing ? "Pause" : "Play", "playPause")}
              disabled={empty}
              onClick={() => send("playPause")}
              className="flex h-[38px] w-[38px] items-center justify-center rounded-full bg-fg text-black shadow-[0_1px_3px_rgb(0_0_0/.4)] transition-transform duration-[120ms] ease-out active:scale-95 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {playing ? <Pause size={16} fill="currentColor" /> : <Play size={16} fill="currentColor" className="translate-x-px" />}
            </button>
            <IconButton round label={withKey("Go to end", "goEnd")} onClick={() => send("goEnd")} disabled={empty} className="@max-[440px]:hidden">
              <SkipForward size={16} />
            </IconButton>
          </div>
          <div className="flex items-center justify-end gap-1">
            <IconButton round label={withKey(full ? "Exit full screen" : "Full screen", "fullScreen")} onClick={() => void toggleFull()}>
              {full ? <Minimize size={16} /> : <Maximize size={16} />}
            </IconButton>
          </div>
        </div>
      </div>
    </main>
  );
}

// Test hook for the UI flows, which replace the window's full screen so a run never takes over the screen.
if (import.meta.env.DEV) Object.assign(window, { __nuzkyWindow: { win, state: useWindow } });

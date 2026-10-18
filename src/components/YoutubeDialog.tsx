import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { AlertCircle, Link2, Pause, Play, WifiOff, X, ZoomIn, ZoomOut } from "lucide-react";
import { api, errorText, plainError } from "../lib/api";
import { useEditor } from "../lib/store";
import { formatLength, US } from "../lib/time";
import type { Filmstrip, Part, VideoPreview, YoutubeVideo } from "../lib/types";
import { addYoutubeVideo, useYoutube } from "../lib/youtube";
import { useFrameStream } from "./preview/Preview";
import { Button, IconButton, NumberInput, ProgressBar, trapTab } from "./ui";

/** Footage kept hidden around the part is the backend's business; the dialog only ever shows the part. */
type Step =
  | { kind: "link" }
  | { kind: "rights"; url: string }
  | { kind: "lookup"; url: string }
  | { kind: "download"; url: string; video: YoutubeVideo; progress: number }
  | { kind: "stopped"; url: string; video: YoutubeVideo }
  | { kind: "ready"; url: string; video: YoutubeVideo; preview: VideoPreview }
  | { kind: "error"; url: string; error: string };

/** Videos longer than this get the overview and the zoom; shorter ones show whole in the strip. */
const LONG_US = 60 * US;
const VIEW_US = 30 * US;
const MIN_VIEW_US = 4 * US;
const STRIP_HEIGHT = 52;

/** 6:21.4, or 1:02:03.5 past an hour. */
function clock(us: number) {
  const tenths = Math.round(Math.max(0, us) / 100_000);
  const h = Math.floor(tenths / 36_000);
  const m = Math.floor((tenths % 36_000) / 600);
  const s = ((tenths % 600) / 10).toFixed(1).padStart(4, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/** Reads 6:21.4, 381.4 or 1:02:03 as microseconds; anything else is NaN. */
function parseClock(text: string) {
  const parts = text.trim().split(":");
  if (parts.length > 3 || parts.some((p) => !/^\d*\.?\d+$/.test(p))) return NaN;
  return parts.reduce((total, p) => total * 60 + Number(p), 0) * US;
}

/** What a failed lookup or download means here, where the link is to a video rather than a song. */
function problem(error: string) {
  if (error.startsWith("INVALID_LINK")) return "That isn't a link to a YouTube video. Copy the link from the video's Share button.";
  if (error.startsWith("PLAYLIST")) return "That's a link to a playlist. Open one video and copy its link instead.";
  const text = plainError(error);
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** 4.0 s under a minute, 1:30 from there. */
const lengthText = (us: number) => (us < 60 * US ? `${(us / US).toFixed(1)} s` : formatLength(us));

const isTyping = (el: EventTarget | null) => el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement;

export function YoutubeDialog() {
  const open = useYoutube((s) => s.open);
  return open ? <Dialog /> : null;
}

function Dialog() {
  const initial = useYoutube.getState().url;
  const [text, setText] = useState(initial);
  const [step, setStep] = useState<Step>({ kind: "link" });
  const [installing, setInstalling] = useState<number | null>(null);
  const runs = useRef({ n: 0 });
  const dialog = useRef<HTMLDivElement>(null);
  const field = useRef<HTMLInputElement>(null);

  const close = () => useYoutube.setState({ open: false, url: "" });

  /** Looks the link up and downloads the video; a newer link or Stop drops what this one was doing. */
  const go = async (url: string) => {
    url = url.trim();
    const run = ++runs.current.n;
    api.youtubeVideoCancel();
    api.youtubePreviewClose();
    if (!url) return setStep({ kind: "link" });
    setStep({ kind: "lookup", url });
    try {
      if (!(await api.soundSettings()).youtubeRights) return run === runs.current.n && setStep({ kind: "rights", url });
      const video = await api.youtubeVideoLink(url);
      if (run !== runs.current.n) return;
      setStep({ kind: "download", url, video, progress: 0 });
      try {
        const preview = await api.youtubeVideoFetch(video.id);
        if (run === runs.current.n) setStep({ kind: "ready", url, video, preview });
      } catch (e) {
        if (run !== runs.current.n) return;
        const error = errorText(e);
        setStep(error.startsWith("CANCELLED") ? { kind: "stopped", url, video } : { kind: "error", url, error });
      }
    } catch (e) {
      if (run === runs.current.n) setStep({ kind: "error", url, error: errorText(e) });
    }
  };

  const stop = () => api.youtubeVideoCancel();

  const answerRights = async (agree: boolean, url: string) => {
    if (!agree) return setStep({ kind: "link" });
    try {
      await api.confirmYoutubeRights();
      await go(url);
    } catch (e) {
      setStep({ kind: "error", url, error: errorText(e) });
    }
  };

  const installYtdlp = async (url: string) => {
    setInstalling(0);
    try {
      await api.installYtdlp();
      setInstalling(null);
      await go(url);
    } catch (e) {
      setInstalling(null);
      if (!errorText(e).startsWith("CANCELLED")) setStep({ kind: "error", url, error: errorText(e) });
    }
  };

  useEffect(() => {
    // The editor pauses: two things playing at once is never wanted.
    if (useEditor.getState().playing) api.pause();
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const counter = runs.current;
    field.current?.focus();
    if (initial) go(initial);
    const offs = [
      listen<{ id: string; progress: number }>("youtube-video-progress", (e) =>
        setStep((s) => (s.kind === "download" && s.video.id === e.payload.id ? { ...s, progress: e.payload.progress } : s)),
      ),
      listen<{ id: string; progress: number }>("sound-progress", (e) => {
        if (e.payload.id === "yt-dlp") setInstalling((i) => (i === null ? i : e.payload.progress));
      }),
    ];
    return () => {
      counter.n++;
      api.youtubeVideoCancel();
      api.youtubePreviewClose();
      for (const off of offs) off.then((f) => f());
      opener?.focus();
    };
    // Runs once per opening.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const video = "video" in step ? step.video : null;
  return (
    <div data-scrim className="fixed inset-0 z-[100] flex items-center justify-center scrim-in bg-black/45" onPointerDown={(e) => e.target === e.currentTarget && close()}>
      <div
        ref={dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby="youtube-title"
        onKeyDown={(e) => {
          // Fields that use Escape themselves, such as a time being typed, stop it first.
          if (e.key === "Escape") {
            e.stopPropagation();
            close();
          }
          trapTab(e);
        }}
        className="overlay flex max-h-[calc(100vh-48px)] w-[720px] max-w-[calc(100vw-48px)] flex-col overflow-y-auto rounded-[20px] p-6 dialog-in"
      >
        <div className="mb-4 flex items-center justify-between gap-3">
          <h2 id="youtube-title" className="text-[17px] font-semibold leading-[22px] tracking-[-0.025em]">
            Video from YouTube
          </h2>
          <IconButton label="Close (Esc)" round className="-mr-2" onClick={close}>
            <X size={16} />
          </IconButton>
        </div>

        <form
          className="relative flex h-8 items-center"
          onSubmit={(e) => {
            e.preventDefault();
            go(text);
          }}
        >
          <Link2 size={14} className="pointer-events-none absolute left-2.5 text-muted" />
          <input
            ref={field}
            type="text"
            inputMode="url"
            value={text}
            placeholder="Paste a link to a YouTube video"
            aria-label="Link to a YouTube video"
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => setText(e.target.value)}
            onPaste={(e) => {
              const pasted = e.clipboardData.getData("text").trim();
              if (!pasted || e.currentTarget.value) return;
              e.preventDefault();
              setText(pasted);
              go(pasted);
            }}
            className={`h-8 w-full text-ellipsis rounded-lg border border-white/[.08] bg-white/[.055] pl-8 text-[13px] text-fg outline-none placeholder:text-muted focus:border-accent ${text ? "pr-7" : "pr-2"}`}
          />
          {text && (
            <button
              type="button"
              aria-label="Clear link"
              onClick={() => {
                setText("");
                go("");
                field.current?.focus();
              }}
              className="absolute right-1.5 flex h-5 w-5 items-center justify-center rounded-full text-muted hover:text-fg"
            >
              <X size={13} />
            </button>
          )}
        </form>

        {step.kind === "lookup" && (
          <div role="status" aria-label="Looking up the video" className="mt-4 flex items-center justify-between gap-6">
            <div className="skeleton h-3.5 w-1/2 rounded" />
            <div className="skeleton h-3 w-1/5 rounded" />
          </div>
        )}
        {video && (
          <div className="mt-4 flex items-baseline justify-between gap-6">
            <span className="min-w-0 truncate text-[13px] font-semibold" title={video.title}>
              {video.title}
            </span>
            <span className="tabular shrink-0 truncate text-[12px] text-muted">
              {video.channel} · {formatLength(video.durationUs)}
            </span>
          </div>
        )}

        {step.kind === "rights" && <Rights onAnswer={(agree) => answerRights(agree, step.url)} />}

        {step.kind === "error" &&
          (step.error.startsWith("YTDLP_MISSING") || step.error.startsWith("YTDLP_OUTDATED") ? (
            <div role="alert" className="mt-3 flex items-center gap-2 text-[12px] text-muted">
              <span className="min-w-0 flex-1">
                {installing !== null ? (
                  <>
                    Downloading <span className="whitespace-nowrap">yt-dlp</span>… <span className="tabular">{Math.round(installing * 100)}%</span>
                  </>
                ) : step.error.startsWith("YTDLP_OUTDATED") ? (
                  <>
                    The <span className="whitespace-nowrap">yt-dlp</span> on this computer couldn't get this video, probably because it's out of date. Nuzky can download its own and keep it
                    updated.
                  </>
                ) : (
                  <>
                    Videos from YouTube come through <span className="whitespace-nowrap">yt-dlp</span>, a free tool. Nuzky can download it for you.
                  </>
                )}
              </span>
              {installing !== null ? (
                <Button className="h-7 shrink-0 text-[12px]" onClick={() => api.soundCancel("yt-dlp")}>
                  Stop
                </Button>
              ) : (
                <Button variant="primary" className="h-7 shrink-0 text-[12px]" onClick={() => installYtdlp(step.url)}>
                  Download yt-dlp
                </Button>
              )}
            </div>
          ) : (
            <div role="alert" className="mt-3 flex items-start gap-2 text-[12px] text-muted">
              {step.error.startsWith("OFFLINE") ? <WifiOff size={14} className="mt-px shrink-0" /> : <AlertCircle size={14} className="mt-px shrink-0 text-warn" />}
              <span className="min-w-0 flex-1 select-text">{problem(step.error)}</span>
              {!["INVALID_LINK", "PLAYLIST", "SOUND_UNAVAILABLE"].some((code) => step.error.startsWith(code)) && (
                <Button className="h-7 shrink-0 text-[12px]" onClick={() => go(step.url)}>
                  Try again
                </Button>
              )}
            </div>
          ))}

        {(step.kind === "download" || step.kind === "stopped") && (
          <div className="mt-3 flex h-[clamp(160px,calc(100vh-440px),288px)] flex-col items-center justify-center gap-3 bg-black shadow-[0_0_0_1px_rgb(255_255_255/.07)]">
            {step.kind === "stopped" ? (
              <>
                <p className="text-[13px] text-muted">The download stopped.</p>
                <Button pill onClick={() => go(step.url)}>
                  Download again
                </Button>
              </>
            ) : step.progress < 1 ? (
              <>
                <p className="tabular text-[13px]">
                  Downloading video <span className="text-muted">{Math.round(step.progress * 100)}%</span>
                </p>
                <ProgressBar value={step.progress} label="Downloading the video" className="w-60" />
                <Button pill className="mt-1" onClick={stop}>
                  Stop
                </Button>
              </>
            ) : (
              <>
                <p className="text-[13px]">Preparing the preview…</p>
                <ProgressBar value={0} label="Preparing the preview" className="w-60" />
              </>
            )}
          </div>
        )}

        {step.kind === "ready" ? (
          <Chooser key={step.video.id} url={step.url} video={step.video} preview={step.preview} onImport={close} onCancel={close} />
        ) : (
          step.kind !== "rights" && <Footer onCancel={close} />
        )}
      </div>
    </div>
  );
}

/** Asked once before the first link: the rights to a video are the user's to check. */
function Rights({ onAnswer }: { onAnswer: (agree: boolean) => void }) {
  const card = useRef<HTMLDivElement>(null);
  useEffect(() => card.current?.querySelector<HTMLElement>("[data-agree]")?.focus(), []);
  return (
    <div ref={card} className="mt-4 rounded-xl bg-white/[.055] p-4 shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
      <p className="text-[13px] font-semibold">YouTube videos are yours to clear</p>
      <div className="mt-1.5 flex flex-col gap-2 text-[13px] text-fg/85">
        <p>
          Nuzky downloads the video you link with <span className="whitespace-nowrap">yt-dlp</span> on this computer.
        </p>
        <p>
          You're responsible for having the rights to use it in your video and for following YouTube's Terms of Service. A video that uses someone else's footage or music can be muted,
          blocked or demonetized.
        </p>
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <Button pill onClick={() => onAnswer(false)}>
          Cancel
        </Button>
        <Button data-agree pill variant="primary" onClick={() => onAnswer(true)}>
          I understand
        </Button>
      </div>
    </div>
  );
}

function Footer({ onCancel }: { onCancel: () => void }) {
  return (
    <div className="mt-5 flex justify-end gap-2 border-t border-white/[.07] pt-4">
      <Button pill onClick={onCancel}>
        Cancel
      </Button>
      <Button pill variant="primary" disabled disabledReason="The video is not downloaded yet">
        Import
      </Button>
    </div>
  );
}

const Key = ({ children }: { children: string }) => (
  <kbd className="rounded-[5px] bg-white/[.055] px-[5px] py-0.5 font-sans text-[11px] leading-[13px] font-medium text-muted">{children}</kbd>
);

/** The downloaded video with its own preview, where the user chooses the part to import. */
function Chooser({ url, video, preview, onImport, onCancel }: { url: string; video: YoutubeVideo; preview: VideoPreview; onImport: () => void; onCancel: () => void }) {
  const duration = preview.durationUs;
  const frame = Math.round(US / (preview.fps > 0 ? preview.fps : 30));
  const long = duration > LONG_US;
  const [part, setPart] = useState<Part>({ startUs: 0, endUs: duration });
  const [time, setTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [view, setView] = useState({ startUs: 0, lengthUs: long ? VIEW_US : duration });
  const canvas = useRef<HTMLCanvasElement>(null);
  const stage = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ w: 0, h: 0 });
  // Playback that starts inside a chosen part stops at its end, so the user hears exactly the cut.
  const stopAt = useRef<number | null>(null);
  const whole = part.startUs === 0 && part.endUs === duration;

  const seek = (t: number) => {
    t = Math.max(0, Math.min(duration, Math.round(t)));
    stopAt.current = null;
    setTime(t);
    api.youtubePreviewSeek(t);
  };
  const play = () => {
    if (playing) return api.youtubePreviewPause();
    let from = time;
    if (!whole && from >= part.endUs - frame && from <= part.endUs + frame) {
      from = part.startUs;
      seek(from);
    }
    stopAt.current = !whole && from >= part.startUs && from < part.endUs ? part.endUs : null;
    setPlaying(true);
    api.youtubePreviewPlay();
  };
  const setStart = (t: number) => setPart((p) => ({ ...p, startUs: Math.max(0, Math.min(p.endUs - frame, Math.round(t))) }));
  const setEnd = (t: number) => setPart((p) => ({ ...p, endUs: Math.min(duration, Math.max(p.startUs + frame, Math.round(t))) }));

  useFrameStream(preview.url, canvas, (t, framePlaying) => {
    if (!framePlaying) return;
    if (stopAt.current !== null && t >= stopAt.current) {
      const end = stopAt.current;
      api.youtubePreviewPause();
      seek(end);
      return;
    }
    setTime(t);
  });

  useEffect(() => {
    const off = listen<{ playing: boolean; tUs: number }>("youtube-transport", (e) => {
      setPlaying(e.payload.playing);
      if (!e.payload.playing && stopAt.current === null) setTime(e.payload.tUs);
    });
    return () => void off.then((f) => f());
  }, []);

  useEffect(() => {
    const el = stage.current;
    if (!el) return;
    let timer = 0;
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setBox({ w: width, h: height });
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        const dpr = window.devicePixelRatio || 1;
        api.youtubePreviewResize(Math.round(width * dpr), Math.round(height * dpr));
      }, 60);
    });
    ro.observe(el);
    return () => {
      ro.disconnect();
      window.clearTimeout(timer);
    };
  }, []);

  // The strip keeps the playhead in view, as playing or a typed time can move it out.
  useEffect(() => {
    setView((v) => (time >= v.startUs && time <= v.startUs + v.lengthUs ? v : { ...v, startUs: clampView(time - v.lengthUs * 0.1, v.lengthUs, duration) }));
  }, [time, duration]);

  const zoom = (by: number) =>
    setView((v) => {
      const lengthUs = Math.max(MIN_VIEW_US, Math.min(duration, v.lengthUs * by));
      return { lengthUs, startUs: clampView(time - (time - v.startUs) * (lengthUs / v.lengthUs), lengthUs, duration) };
    });

  const add = () => {
    onImport();
    addYoutubeVideo(video, whole ? null : part, url);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (isTyping(e.target) || e.ctrlKey || e.metaKey || e.altKey) return;
    const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    const handled = (() => {
      switch (key) {
        case " ":
          if (e.target instanceof HTMLButtonElement) return false;
          play();
          return true;
        case "i":
          setStart(time);
          return true;
        case "o":
          setEnd(time);
          return true;
        case "ArrowLeft":
        case "ArrowRight":
          if ((e.target as HTMLElement).getAttribute?.("role") === "slider") return false;
          seek(time + (key === "ArrowLeft" ? -1 : 1) * (e.shiftKey ? US : frame));
          return true;
        case "Home":
          seek(0);
          return true;
        case "End":
          seek(duration);
          return true;
        case "Enter":
          if (e.target instanceof HTMLButtonElement) return false;
          add();
          return true;
        default:
          return false;
      }
    })();
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  };

  const aspect = preview.width / preview.height;
  const fit = box.w / box.h > aspect ? { h: box.h, w: box.h * aspect } : { w: box.w, h: box.w / aspect };
  const length = part.endUs - part.startUs;

  return (
    <div onKeyDown={onKeyDown} className="flex min-h-0 flex-col">
      <div ref={stage} className="relative mt-3 flex h-[clamp(160px,calc(100vh-440px),288px)] items-center justify-center bg-black shadow-[0_0_0_1px_rgb(255_255_255/.07)]">
        {/* Square corners and no blurred shadow, as the editor's preview: WebKitGTK would otherwise mask or blank the canvas. */}
        <canvas ref={canvas} style={{ width: fit.w, height: fit.h }} aria-label={`Preview of ${video.title}`} />
      </div>

      <div className="mt-3 flex h-8 items-center gap-2.5">
        <button
          type="button"
          aria-label={playing ? "Pause (Space)" : "Play (Space)"}
          title={playing ? "Pause (Space)" : "Play (Space)"}
          onClick={play}
          className="flex h-7 w-7 items-center justify-center rounded-full bg-fg text-black transition-transform duration-[120ms] ease-out active:scale-95"
        >
          {playing ? <Pause size={13} fill="currentColor" /> : <Play size={13} fill="currentColor" className="translate-x-px" />}
        </button>
        <span className="tabular text-[12px]">
          <span className="font-medium text-fg">{clock(time)}</span>
          <span className="text-muted"> / {clock(duration)}</span>
        </span>
        {long && (
          <div className="ml-auto flex items-center gap-0.5">
            <IconButton label="Zoom out" round disabled={view.lengthUs >= duration} onClick={() => zoom(2)}>
              <ZoomOut size={15} />
            </IconButton>
            <IconButton label="Zoom in" round disabled={view.lengthUs <= MIN_VIEW_US} onClick={() => zoom(0.5)}>
              <ZoomIn size={15} />
            </IconButton>
          </div>
        )}
      </div>

      {long && <Overview duration={duration} view={view} part={whole ? null : part} time={time} onView={(startUs) => setView((v) => ({ ...v, startUs: clampView(startUs, v.lengthUs, duration) }))} />}
      <Strip duration={duration} frame={frame} view={view} part={part} time={time} onSeek={seek} onStart={setStart} onEnd={setEnd} />

      <div className="mt-3 flex min-h-7 flex-wrap items-center gap-x-2.5 gap-y-2 text-[12px] text-muted">
        <label className="flex items-center gap-1.5">
          Start
          <NumberInput label="Start of the part" value={part.startUs} min={0} max={part.endUs - frame} step={100_000} format={clock} parse={parseClock} onChange={(v) => { setStart(v); seek(v); }} className="w-[76px]" />
        </label>
        <label className="flex items-center gap-1.5">
          End
          <NumberInput label="End of the part" value={part.endUs} min={part.startUs + frame} max={duration} step={100_000} format={clock} parse={parseClock} onChange={(v) => { setEnd(v); seek(v); }} className="w-[76px]" />
        </label>
        <span className="tabular text-fg">{lengthText(length)}</span>
        {!whole && (
          <Button pill className="ml-auto h-7 text-[12px]" onClick={() => setPart({ startUs: 0, endUs: duration })}>
            Use whole video
          </Button>
        )}
      </div>

      <div className="mt-5 flex items-center gap-2 border-t border-white/[.07] pt-4">
        <div className="mr-auto flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted" aria-hidden>
          <span className="flex items-center gap-1">
            <Key>Space</Key> Play
          </span>
          <span className="flex items-center gap-1">
            <Key>I</Key>
            <Key>O</Key> Start and end
          </span>
          <span className="flex items-center gap-1">
            <Key>←</Key>
            <Key>→</Key> Step
          </span>
        </div>
        <Button pill onClick={onCancel}>
          Cancel
        </Button>
        <Button pill variant="primary" onClick={add}>
          {whole ? "Import whole video" : `Import ${lengthText(length)}`}
        </Button>
      </div>
    </div>
  );
}

const clampView = (startUs: number, lengthUs: number, duration: number) => Math.max(0, Math.min(duration - lengthUs, startUs));

/** The whole of a long video as a thin track: the zoomed stretch as a box, the part in accent and the playhead. */
function Overview({ duration, view, part, time, onView }: { duration: number; view: { startUs: number; lengthUs: number }; part: Part | null; time: number; onView: (startUs: number) => void }) {
  const at = (us: number) => `${(us / duration) * 100}%`;
  const drag = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const move = (x: number) => onView(((x - rect.left) / rect.width) * duration - view.lengthUs / 2);
    move(e.clientX);
    const onMove = (ev: PointerEvent) => move(ev.clientX);
    const up = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", up);
  };
  return (
    <div className="relative mt-1.5 mb-0.5 h-3.5 cursor-pointer" onPointerDown={drag} title="Drag to move the zoomed stretch">
      <div className="absolute inset-x-0 top-[5px] h-1 rounded-full bg-white/[.13]" />
      {part && <div className="absolute top-1 h-1.5 min-w-[3px] rounded-[2px] bg-accent" style={{ left: at(part.startUs), width: at(part.endUs - part.startUs) }} />}
      <div className="absolute top-0 h-3.5 min-w-2.5 rounded-[4px] bg-white/[.08] shadow-[inset_0_0_0_1px_rgb(255_255_255/.38)]" style={{ left: at(view.startUs), width: at(view.lengthUs) }} />
      <div className="absolute top-px h-3 w-[1.5px] -translate-x-[.75px] rounded-[1px] bg-fg" style={{ left: at(time) }} />
    </div>
  );
}

/** Ruler steps that leave room for a label: 1, 2, 5, 10, 15, 30 s, then minutes. */
const STEPS = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800].map((s) => s * US);

/**
 * The zoomed stretch as a filmstrip with a ruler, the part between two handles and the playhead.
 * Dragging the strip scrubs; dragging a handle moves that edge, showing the frame there.
 */
function Strip({
  duration,
  frame,
  view,
  part,
  time,
  onSeek,
  onStart,
  onEnd,
}: {
  duration: number;
  frame: number;
  view: { startUs: number; lengthUs: number };
  part: Part;
  time: number;
  onSeek: (t: number) => void;
  onStart: (t: number) => void;
  onEnd: (t: number) => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [strip, setStrip] = useState<{ fromUs: number; film: Filmstrip } | null>(null);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Frames for the stretch in view; the old ones stay, placed by their own times, until the new ones arrive.
  useEffect(() => {
    if (width === 0) return;
    let stale = false;
    const timer = window.setTimeout(async () => {
      const frameWidth = (STRIP_HEIGHT * 16) / 9;
      const count = Math.max(1, Math.ceil(width / frameWidth));
      try {
        const film = await api.youtubeFilmstrip(view.startUs, view.lengthUs / count, count);
        if (!stale) setStrip({ fromUs: view.startUs, film });
      } catch {
        // The preview closed meanwhile; nothing to show.
      }
    }, 120);
    return () => {
      stale = true;
      window.clearTimeout(timer);
    };
  }, [view.startUs, view.lengthUs, width]);

  const x = (us: number) => ((us - view.startUs) / view.lengthUs) * width;
  const timeAt = (clientX: number) => {
    const rect = box.current!.getBoundingClientRect();
    return view.startUs + ((clientX - rect.left) / rect.width) * view.lengthUs;
  };
  const follow = (e: React.PointerEvent, to: (t: number) => void) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    let raf = 0;
    let last = e.clientX;
    const apply = () => {
      raf = 0;
      const t = Math.max(0, Math.min(duration, timeAt(last)));
      to(t);
    };
    apply();
    const onMove = (ev: PointerEvent) => {
      last = ev.clientX;
      if (!raf) raf = requestAnimationFrame(apply);
    };
    const up = () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", up);
  };
  const edge = (to: (t: number) => void) => (t: number) => {
    to(t);
    onSeek(t);
  };
  const handleKeys = (to: (t: number) => void, value: number) => (e: React.KeyboardEvent) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    e.stopPropagation();
    edge(to)(value + (e.key === "ArrowLeft" ? -1 : 1) * (e.shiftKey ? US : frame));
  };

  const step = STEPS.find((s) => (s / view.lengthUs) * width >= 64) ?? STEPS[STEPS.length - 1];
  const ticks: number[] = [];
  for (let t = Math.ceil(view.startUs / step) * step; t <= view.startUs + view.lengthUs; t += step) ticks.push(t);
  const film = strip?.film;
  const from = strip?.fromUs ?? 0;
  const shown = film ? { w: (film.frameWidth * STRIP_HEIGHT) / film.frameHeight, slot: (film.intervalUs / view.lengthUs) * width } : null;
  const left = Math.max(0, x(part.startUs));
  const right = Math.min(width, x(part.endUs));
  const handle = "absolute top-[-2px] bottom-[-2px] z-10 flex w-2 cursor-ew-resize items-center justify-center bg-accent outline-offset-2 after:h-3.5 after:w-0.5 after:rounded-[1px] after:bg-black/55 focus-visible:outline-2 focus-visible:outline-accent";

  return (
    <div ref={box} className="relative mt-1 select-none">
      <div className="relative h-[18px]" aria-hidden>
        {ticks.filter((t) => x(t) >= 14 && x(t) <= width - 14).map((t) => (
          <span key={t} className="tabular absolute bottom-1.5 -translate-x-1/2 text-[11px] leading-[11px] text-muted" style={{ left: x(t) }}>
            {formatLength(t)}
          </span>
        ))}
      </div>
      <div
        data-strip={film ? "ready" : "loading"}
        className={`relative h-[52px] cursor-pointer overflow-hidden rounded-md bg-white/[.04] shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)] ${film ? "" : "skeleton"}`}
        onPointerDown={(e) => follow(e, onSeek)}
      >
        {film &&
          shown &&
          Array.from({ length: film.count }, (_, i) => (
            <div
              key={i}
              className="absolute top-0 h-full"
              style={{
                left: x(from + i * film.intervalUs),
                width: shown.slot + 1,
                backgroundImage: `url(${film.url})`,
                backgroundSize: `${shown.w * film.count}px ${STRIP_HEIGHT}px`,
                backgroundPosition: `${-(i * shown.w) + (shown.slot - shown.w) / 2}px 0`,
              }}
            />
          ))}
        <div className="absolute inset-y-0 left-0 bg-black/60" style={{ width: left }} />
        <div className="absolute inset-y-0 right-0 bg-black/60" style={{ width: Math.max(0, width - right) }} />
      </div>
      {right > 0 && left < width && (
        <div className="pointer-events-none absolute top-[18px] h-[52px] border-y-2 border-accent" style={{ left, width: Math.max(0, right - left) }} />
      )}
      {x(part.startUs) >= -4 && x(part.startUs) <= width && (
        <div
          role="slider"
          tabIndex={0}
          aria-label="Start of the part"
          aria-valuemin={0}
          aria-valuemax={part.endUs}
          aria-valuenow={part.startUs}
          aria-valuetext={clock(part.startUs)}
          className={`${handle} rounded-l-[5px]`}
          style={{ left: x(part.startUs) - 8, top: 16, bottom: "auto", height: 56 }}
          onPointerDown={(e) => follow(e, edge(onStart))}
          onKeyDown={handleKeys(onStart, part.startUs)}
        />
      )}
      {x(part.endUs) >= 0 && x(part.endUs) <= width + 4 && (
        <div
          role="slider"
          tabIndex={0}
          aria-label="End of the part"
          aria-valuemin={part.startUs}
          aria-valuemax={duration}
          aria-valuenow={part.endUs}
          aria-valuetext={clock(part.endUs)}
          className={`${handle} rounded-r-[5px]`}
          style={{ left: x(part.endUs), top: 16, bottom: "auto", height: 56 }}
          onPointerDown={(e) => follow(e, edge(onEnd))}
          onKeyDown={handleKeys(onEnd, part.endUs)}
        />
      )}
      {time >= view.startUs && time <= view.startUs + view.lengthUs && (
        <div className="pointer-events-none absolute top-1 z-20 h-[66px] w-[1.5px] -translate-x-[.75px] bg-fg" style={{ left: x(time) }}>
          <div className="absolute -top-0.5 -left-[4.25px] h-3 w-2.5 rounded-[3px_3px_5px_5px] bg-fg" />
        </div>
      )}
    </div>
  );
}

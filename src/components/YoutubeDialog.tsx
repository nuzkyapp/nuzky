import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { AlertCircle, Link2, Pause, Play, WifiOff, X } from "lucide-react";
import { api, errorText, plainError } from "../lib/api";
import { AI_EDITING, useAiLocked, useEditor } from "../lib/store";
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

/** Where a link's t= points, as YouTube's Share writes it (83, 83s, 1m23s); 0 without one. */
function linkTime(url: string) {
  const t = /[?&#]t=(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s?)?(?:[&#]|$)/.exec(url);
  return t ? ((Number(t[1] ?? 0) * 60 + Number(t[2] ?? 0)) * 60 + Number(t[3] ?? 0)) * US : 0;
}

/** What a failed lookup or download means here, where the link is to a video rather than a song. */
function problem(error: string) {
  if (error.startsWith("INVALID_LINK")) return "That isn't a link to a YouTube video. Copy the link from the video's Share button.";
  if (error.startsWith("PLAYLIST")) return "That's a link to a playlist. Open one video and copy its link instead.";
  const text = plainError(error);
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** 0.03 s under a second, 4.0 s under a minute, 1:30 from there. */
const lengthText = (us: number) => (us < 60 * US ? `${(us / US).toFixed(us < US ? 2 : 1)} s` : formatLength(us));

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
    const run = runs.current.n;
    setInstalling(0);
    try {
      await api.installYtdlp();
      setInstalling(null);
      // Closed, or another link, meanwhile: nothing more to download for this one.
      if (run === runs.current.n) await go(url);
    } catch (e) {
      setInstalling(null);
      if (run === runs.current.n && !errorText(e).startsWith("CANCELLED")) setStep({ kind: "error", url, error: errorText(e) });
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

  // A step that removes the focused button, such as Stop or I understand, leaves focus in the dialog.
  useEffect(() => {
    if (!dialog.current?.contains(document.activeElement)) (field.current ?? dialog.current)?.focus();
  }, [step.kind]);

  const video = "video" in step ? step.video : null;
  return (
    // Once a video downloads, a press that misses the dialog would throw away the download and the chosen part.
    <div data-scrim className="fixed inset-0 z-[100] flex items-center justify-center scrim-in bg-black/45" onPointerDown={(e) => e.target === e.currentTarget && !video && close()}>
      <div
        ref={dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby="youtube-title"
        tabIndex={-1}
        onKeyDown={(e) => {
          // Fields that use Escape themselves, such as a time being typed, stop it first.
          if (e.key === "Escape") {
            e.stopPropagation();
            close();
          }
          trapTab(e);
        }}
        className="overlay overlay-solid flex max-h-[calc(100vh-48px)] outline-none w-[720px] max-w-[calc(100vw-48px)] flex-col overflow-y-auto rounded-[20px] p-6 dialog-in"
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
              const input = e.currentTarget;
              if (!pasted || input.selectionStart !== 0 || input.selectionEnd !== input.value.length) return;
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
                <Button className="h-7 shrink-0 text-[12px]" onClick={() => go(text)}>
                  Try again
                </Button>
              )}
            </div>
          ))}

        {(step.kind === "download" || step.kind === "stopped") && (
          <div className="mt-3 flex h-[clamp(160px,calc(100vh-458px),360px)] flex-col items-center justify-center gap-3 bg-black shadow-[0_0_0_1px_rgb(255_255_255/.07)]">
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
                <Button pill className="mt-1" onClick={stop}>
                  Stop
                </Button>
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
  const [part, setPart] = useState<Part>({ startUs: 0, endUs: duration });
  const [time, setTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const canvas = useRef<HTMLCanvasElement>(null);
  const stage = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ w: 0, h: 0 });
  // Playback that starts inside a chosen part stops at its end, so the user hears exactly the cut.
  const stopAt = useRef<number | null>(null);
  const whole = part.startUs === 0 && part.endUs === duration;
  const locked = useAiLocked();
  const root = useRef<HTMLDivElement>(null);
  const [nudging, setNudging] = useState<string | null>(null);
  const focusRoot = () => root.current?.focus({ preventScroll: true });

  const seek = (t: number) => {
    t = Math.max(0, Math.min(duration, Math.round(t)));
    stopAt.current = null;
    setTime(t);
    api.youtubePreviewSeek(t);
  };
  const play = () => {
    // The arrows step the playhead again, rather than moving a handle grabbed before.
    if (document.activeElement?.getAttribute("role") === "slider") focusRoot();
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
  // I, O and typed times past the other edge begin a new part there, as in Final Cut, rather than squeezing it to a frame.
  const markStart = (t: number) =>
    setPart((p) => (t <= p.endUs - frame ? { ...p, startUs: Math.max(0, Math.round(t)) } : { startUs: Math.min(duration - frame, Math.round(t)), endUs: duration }));
  const markEnd = (t: number) => setPart((p) => (t >= p.startUs + frame ? { ...p, endUs: Math.min(duration, Math.round(t)) } : { startUs: 0, endUs: Math.max(frame, Math.round(t)) }));

  // Space, I, O and Enter work from the start, rather than typing into the link field, and a link's t= opens
  // where it points, as on YouTube.
  useEffect(() => {
    focusRoot();
    const at = Math.min(duration, linkTime(url));
    if (at > 0) seek(at);
    // Runs once per video; the chooser is keyed by it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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

  const [previewError, setPreviewError] = useState<string | null>(null);
  useEffect(() => {
    const offs = [
      listen<{ playing: boolean; tUs: number }>("youtube-transport", (e) => {
        setPlaying(e.payload.playing);
        if (!e.payload.playing && stopAt.current === null) setTime(e.payload.tUs);
      }),
      listen<string | null>("youtube-preview-error", (e) => setPreviewError(e.payload)),
    ];
    // It may have failed before this listened.
    api.youtubePreviewError().then((error) => error && setPreviewError(error));
    return () => void offs.forEach((off) => off.then((f) => f()));
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

  const add = () => {
    if (locked) return;
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
          markStart(time);
          return true;
        case "o":
          markEnd(time);
          return true;
        case "ArrowLeft":
        case "ArrowRight":
          if ((e.target as HTMLElement).getAttribute?.("role") === "slider") return false;
          seek(time + (key === "ArrowLeft" ? -1 : 1) * (e.shiftKey ? US : frame));
          return true;
        case "Home":
          if ((e.target as HTMLElement).getAttribute?.("role") === "slider") return false;
          seek(0);
          return true;
        case "End":
          if ((e.target as HTMLElement).getAttribute?.("role") === "slider") return false;
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
    <div
      ref={root}
      tabIndex={-1}
      onKeyDown={onKeyDown}
      onFocus={(e) => setNudging((e.target as HTMLElement).dataset.edge ?? null)}
      onBlur={(e) => {
        setNudging(null);
        // Enter or Esc in a time field, or a click on plain text, drops focus to the page, where these keys do nothing.
        if (!e.relatedTarget) requestAnimationFrame(() => document.activeElement === document.body && focusRoot());
      }}
      className="flex min-h-0 flex-col outline-none"
    >
      <div ref={stage} className="relative mt-3 flex h-[clamp(160px,calc(100vh-458px),360px)] items-center justify-center bg-black shadow-[0_0_0_1px_rgb(255_255_255/.07)]">
        {/* Square corners and no blurred shadow, as the editor's preview: WebKitGTK would otherwise mask or blank the canvas. */}
        <canvas ref={canvas} style={{ width: fit.w, height: fit.h }} aria-label={`Preview of ${video.title}`} />
        {previewError && (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-black/80 p-6 text-center" role="alert">
            <p className="text-[13px] font-medium text-danger">The preview is unavailable</p>
            <p className="text-[12px] text-muted">{previewError} You can still choose the part by its times and import it.</p>
          </div>
        )}
      </div>

      <div className="mt-3 flex h-8 items-center gap-2.5">
        <button
          type="button"
          aria-label={playing ? "Pause (Space)" : "Play (Space)"}
          title={playing ? "Pause (Space)" : "Play (Space)"}
          // A click hands the keys to the chooser, so Enter still imports, and leaving a time field takes what was
          // typed there before it plays; Tab and Enter press it like any button.
          onMouseDown={(e) => {
            e.preventDefault();
            focusRoot();
          }}
          onClick={play}
          className="flex h-7 w-7 items-center justify-center rounded-full bg-fg text-black transition-transform duration-[120ms] ease-out active:scale-95"
        >
          {playing ? <Pause size={13} fill="currentColor" /> : <Play size={13} fill="currentColor" className="translate-x-px" />}
        </button>
        <span className="tabular text-[12px]">
          <span className="font-medium text-fg">{clock(time)}</span>
          <span className="text-muted"> / {clock(duration)}</span>
        </span>
      </div>

      <Strip duration={duration} frame={frame} aspect={aspect} part={part} time={time} onSeek={seek} onStart={setStart} onEnd={setEnd} onGrab={focusRoot} />

      <div className="mt-3 flex min-h-7 flex-wrap items-center gap-x-2.5 gap-y-2 text-[12px] text-muted">
        <label className="flex items-center gap-1.5">
          Start
          <NumberInput label="Start of the part" value={part.startUs} min={0} max={duration - frame} step={100_000} format={clock} parse={parseClock} onChange={(v) => { markStart(v); seek(v); }} className="w-[76px]" />
        </label>
        <label className="flex items-center gap-1.5">
          End
          <NumberInput label="End of the part" value={part.endUs} min={frame} max={duration} step={100_000} format={clock} parse={parseClock} onChange={(v) => { markEnd(v); seek(v); }} className="w-[76px]" />
        </label>
        <span className="tabular text-fg">{lengthText(length)}</span>
        {!whole && (
          <Button
            pill
            className="ml-auto h-7 text-[12px]"
            onClick={() => {
              setPart({ startUs: 0, endUs: duration });
              // The button goes with the part; the keys stay with the chooser.
              focusRoot();
            }}
          >
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
            <Key>→</Key> {nudging === "start" ? "Move start" : nudging === "end" ? "Move end" : "Step"}
          </span>
        </div>
        <Button pill onClick={onCancel}>
          Cancel
        </Button>
        <Button pill variant="primary" disabled={locked} disabledReason={AI_EDITING} onClick={add}>
          {whole ? "Import whole video" : `Import ${lengthText(length)}`}
        </Button>
      </div>
    </div>
  );
}

/** Ruler steps that leave room for a label: 1, 2, 5, 10, 15, 30 s, then minutes. */
const STEPS = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800].map((s) => s * US);

/** A pressed handle that rests this long zooms the strip in around it, as on iPhone. */
const HOLD_MS = 400;

type View = { startUs: number; lengthUs: number };

/**
 * The whole video as a filmstrip with a ruler, the part between two handles and the playhead.
 * Dragging the strip scrubs; dragging a handle moves that edge, showing the frame there, and leaves it focused
 * so the arrow keys fine-tune it. Holding a handle still zooms in to about a frame a pixel until it is let go.
 */
function Strip({
  duration,
  frame,
  aspect,
  part,
  time,
  onSeek,
  onStart,
  onEnd,
  onGrab,
}: {
  duration: number;
  frame: number;
  aspect: number;
  part: Part;
  time: number;
  onSeek: (t: number) => void;
  onStart: (t: number) => void;
  onEnd: (t: number) => void;
  onGrab: () => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [zoom, setZoom] = useState<View | null>(null);
  const view = zoom ?? { startUs: 0, lengthUs: duration };
  const [whole, setWhole] = useState<Filmstrip | null>(null);
  const [zoomed, setZoomed] = useState<{ view: View; film: Filmstrip } | null>(null);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // As many frames as fit side by side at the video's own shape, but no more than thumbs::filmstrip_size gives
  // (one each half second, 40 in all), which would leave the rest of the strip without frames.
  const count = Math.max(1, Math.min(Math.ceil(width / (STRIP_HEIGHT * aspect)), Math.floor(duration / 500_000), 40));
  useEffect(() => {
    if (width === 0) return;
    let stale = false;
    const timer = window.setTimeout(async () => {
      try {
        const film = await api.youtubeFilmstrip(0, duration / count, count);
        if (!stale) setWhole(film);
      } catch {
        // The preview closed meanwhile; nothing to show.
      }
    }, 120);
    return () => {
      stale = true;
      window.clearTimeout(timer);
    };
  }, [duration, width, count]);
  // The whole video's frames stay, placed by their own times, until the zoom's arrive.
  useEffect(() => {
    if (!zoom) return;
    let stale = false;
    api.youtubeFilmstrip(zoom.startUs, zoom.lengthUs / count, count).then(
      (film) => !stale && setZoomed({ view: zoom, film }),
      () => {},
    );
    return () => {
      stale = true;
    };
  }, [zoom, count]);

  const x = (us: number) => ((us - view.startUs) / view.lengthUs) * width;
  const follow = (e: React.PointerEvent, to: (t: number) => void, edgeUs?: number) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const rect = box.current!.getBoundingClientRect();
    let v = view;
    const timeAt = (px: number) => v.startUs + (px / rect.width) * v.lengthUs;
    const pxAt = (us: number) => ((us - v.startUs) / v.lengthUs) * rect.width;
    let px = e.clientX - rect.left;
    // A handle moves as far as the pointer does, wherever on it the pointer caught it.
    let grip = edgeUs === undefined ? 0 : pxAt(edgeUs) - px;
    let t = 0;
    let raf = 0;
    let hold = 0;
    let restAt = px;
    const apply = () => {
      raf = 0;
      t = Math.max(v.startUs, Math.min(v.startUs + v.lengthUs, timeAt(px + grip)));
      to(t);
    };
    const zoomIn = () => {
      const lengthUs = rect.width * frame;
      if (v.lengthUs < duration || duration < lengthUs * 2) return;
      v = { lengthUs, startUs: Math.max(0, Math.min(duration - lengthUs, t - ((px + grip) / rect.width) * lengthUs)) };
      grip = pxAt(t) - px;
      setZoom(v);
    };
    const rest = () => {
      window.clearTimeout(hold);
      if (edgeUs !== undefined) hold = window.setTimeout(zoomIn, HOLD_MS);
    };
    apply();
    rest();
    const onMove = (ev: PointerEvent) => {
      px = ev.clientX - rect.left;
      if (Math.abs(px - restAt) > 3) {
        restAt = px;
        rest();
      }
      if (!raf) raf = requestAnimationFrame(apply);
    };
    const up = () => {
      window.clearTimeout(hold);
      // Where the pointer let go, also when that was within the frame it last moved in.
      if (raf) {
        cancelAnimationFrame(raf);
        apply();
      }
      setZoom(null);
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
  const grabEdge = (to: (t: number) => void, value: number) => (e: React.PointerEvent<HTMLDivElement>) => {
    e.currentTarget.focus({ preventScroll: true });
    follow(e, edge(to), value);
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
  const strip = zoom && zoomed?.view === zoom ? zoomed : whole && { view: { startUs: 0, lengthUs: duration }, film: whole };
  const film = strip?.film;
  // Frames at their own width side by side, each the one at its time, so a short video repeats some in order.
  const tileWidth = strip ? (strip.film.frameWidth * STRIP_HEIGHT) / strip.film.frameHeight : 0;
  const tiles = strip
    ? Array.from({ length: Math.ceil(width / tileWidth) }, (_, tile) => {
        const t = view.startUs + (((tile + 0.5) * tileWidth) / width) * view.lengthUs;
        return Math.max(0, Math.min(strip.film.count - 1, Math.floor((t - strip.view.startUs) / strip.film.intervalUs)));
      })
    : [];
  const inView = (us: number) => us >= view.startUs && us <= view.startUs + view.lengthUs;
  const left = Math.max(0, Math.min(width, x(part.startUs)));
  const right = Math.max(0, Math.min(width, x(part.endUs)));
  // The visible handle is the timeline's 8 px; its ::before widens what the pointer can catch outwards. One out of
  // the zoomed stretch hides but stays, so it keeps the focus.
  const handle =
    "absolute top-4 z-10 flex h-14 w-2 cursor-ew-resize items-center justify-center bg-accent outline-offset-2 before:absolute before:inset-y-0 after:h-3.5 after:w-0.5 after:rounded-[1px] after:bg-black/55 focus:outline-2 focus:outline-accent";
  const hidden = (us: number) => (inView(us) ? undefined : { opacity: 0, pointerEvents: "none" as const });

  return (
    <div ref={box} data-zoomed={zoom ? "" : undefined} className="relative mt-1 select-none">
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
        onPointerDown={(e) => {
          onGrab();
          follow(e, onSeek);
        }}
      >
        {strip &&
          tiles.map((frame, tile) => (
            <div
              key={tile}
              className="absolute top-0 h-full"
              style={{
                left: tile * tileWidth,
                width: tileWidth,
                backgroundImage: `url(${strip.film.url})`,
                backgroundSize: `${tileWidth * strip.film.count}px ${STRIP_HEIGHT}px`,
                backgroundPosition: `${-frame * tileWidth}px 0`,
              }}
            />
          ))}
        <div className="absolute inset-y-0 left-0 bg-black/60" style={{ width: left }} />
        <div className="absolute inset-y-0 right-0 bg-black/60" style={{ width: width - right }} />
      </div>
      <div className="pointer-events-none absolute top-[18px] h-[52px] border-y-2 border-accent" style={{ left, width: right - left }} />
      <div
        role="slider"
        tabIndex={0}
        data-edge="start"
        aria-label="Start of the part"
        aria-valuemin={0}
        aria-valuemax={part.endUs}
        aria-valuenow={part.startUs}
        aria-valuetext={clock(part.startUs)}
        title="Drag to move the start; hold still to zoom in"
        className={`${handle} rounded-l-[5px] before:-left-2.5 before:right-0`}
        style={{ left: left - 8, ...hidden(part.startUs) }}
        onPointerDown={grabEdge(onStart, part.startUs)}
        onKeyDown={handleKeys(onStart, part.startUs)}
      />
      <div
        role="slider"
        tabIndex={0}
        data-edge="end"
        aria-label="End of the part"
        aria-valuemin={part.startUs}
        aria-valuemax={duration}
        aria-valuenow={part.endUs}
        aria-valuetext={clock(part.endUs)}
        title="Drag to move the end; hold still to zoom in"
        className={`${handle} rounded-r-[5px] before:left-0 before:-right-2.5`}
        style={{ left: right, ...hidden(part.endUs) }}
        onPointerDown={grabEdge(onEnd, part.endUs)}
        onKeyDown={handleKeys(onEnd, part.endUs)}
      />
      {inView(time) && (
        // Whole pixels and no transform: WebKitGTK left slivers of the knob behind where it moved from.
        <div className="pointer-events-none absolute top-1 z-20 h-[66px] w-0.5 bg-fg" style={{ left: Math.round(x(time)) - 1 }}>
          <div className="absolute -top-0.5 -left-1 h-3 w-2.5 rounded-[3px_3px_5px_5px] bg-fg" />
        </div>
      )}
    </div>
  );
}

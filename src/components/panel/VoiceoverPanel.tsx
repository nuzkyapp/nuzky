import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Mic, Square, TriangleAlert, X } from "lucide-react";
import { useEditor } from "../../lib/store";
import { formatDuration, formatTime } from "../../lib/time";
import type { MicWarning } from "../../lib/types";
import {
  cancelCountdown,
  closeVoiceover,
  reopenVoiceover,
  startCountdown,
  stopVoiceover,
  useMicLevel,
  useVoiceover,
  videoEndReason,
  voiceoverKeys,
} from "../../lib/voiceover";
import { Button, IconButton, LockReason, useLockReason } from "../ui";

const MAC = navigator.userAgent.includes("Mac");

/** While the creator reads, one line of fixed height, so the script never moves; the whole advice is its tooltip. */
const SHORT: Record<MicWarning, string> = {
  loud: "Too loud, move back a little",
  quiet: "Too quiet, move closer",
  silent: "Nuzky hears nothing",
};

const WARNINGS: Record<MicWarning, string> = {
  loud: "Too loud, your voice may distort. Move back from the microphone or turn its input level down.",
  quiet: "Too quiet. Move closer to the microphone or turn its input level up.",
  silent: MAC
    ? "Nuzky hears nothing. Check that the microphone is on and that Nuzky may use it in System Settings › Privacy & Security › Microphone."
    : "Nuzky hears nothing. Check that the microphone is on and that Nuzky may use it in the system's privacy settings.",
};

/** The meter spans -60 to 0 dBFS; the mark at -12 dBFS is where a voice's loudest moments should land. */
const share = (db: number) => Math.max(0, Math.min(1, (db + 60) / 60));
const MARK = share(-12);

/** The microphone's level, drawn on its own as it changes 20 times a second. */
function Meter({ reading }: { reading: boolean }) {
  const level = useMicLevel((s) => s.level);
  const loud = level?.warning === "loud";
  const low = level?.warning === "quiet" || level?.warning === "silent";
  return (
    <>
      <div
        role="meter"
        aria-label="Microphone level"
        aria-valuemin={-60}
        aria-valuemax={0}
        aria-valuenow={Math.round(level?.peakDb ?? -60)}
        aria-valuetext={level ? `${Math.round(level.peakDb)} dB` : "No sound yet"}
        title="Your voice's loudest moments should reach about the mark"
        className="relative mt-2 h-1.5 overflow-hidden rounded-full bg-white/[.08]"
      >
        <div
          data-meter-fill
          className={`absolute inset-y-0 left-0 rounded-full transition-[width] duration-75 ease-out ${loud ? "bg-danger" : low ? "bg-warn" : "bg-ok"}`}
          style={{ width: `${share(level?.peakDb ?? -60) * 100}%` }}
        />
        <div aria-hidden className="absolute -inset-y-px w-px bg-white/35" style={{ left: `${MARK * 100}%` }} />
      </div>
      {reading ? (
        <p data-mic-warning={level?.warning ?? undefined} role="status" title={level?.warning ? WARNINGS[level.warning] : undefined} className="mt-2 flex h-4 items-center gap-1.5 text-[12px] leading-4 text-fg">
          {level?.warning && (
            <>
              <TriangleAlert size={14} className={`shrink-0 ${loud ? "text-danger" : "text-warn"}`} />
              <span className="truncate">{SHORT[level.warning]}</span>
            </>
          )}
        </p>
      ) : (
        level?.warning && (
          <p data-mic-warning={level.warning} role="status" className="mt-2 flex items-start gap-1.5 text-[12px] leading-4 text-fg">
            <TriangleAlert size={14} className={`mt-px shrink-0 ${loud ? "text-danger" : "text-warn"}`} />
            <span>{WARNINGS[level.warning]}</span>
          </p>
        )
      )}
    </>
  );
}

/** How long the take is, as its clip on the timeline says it. */
function TakeLength() {
  const length = useMicLevel((s) => formatDuration(s.level?.recordedUs ?? 0));
  return <span className="tabular text-[13px] text-muted">{length}</span>;
}

/**
 * The project's script. While it has focus the field shows what was typed, not the confirmed project, which
 * arrives a moment later and would move the caret; each focus is one undo step.
 */
function ScriptField({ disabled }: { disabled: string | null }) {
  const script = useEditor((s) => s.snap?.project.script ?? "");
  const edit = useEditor((s) => s.edit);
  const [draft, setDraft] = useState<string | null>(null);
  return (
    <>
      <label htmlFor="voiceover-script" className="mt-4 mb-1.5 text-[12px] leading-4 text-muted">
        Script
      </label>
      <textarea
        id="voiceover-script"
        data-voiceover-script
        value={draft ?? script}
        disabled={!!disabled}
        title={disabled ?? undefined}
        placeholder="Paste or write what you'll say"
        spellCheck
        onChange={(e) => {
          setDraft(e.target.value);
          void edit({ type: "setScript", text: e.target.value }, "script");
        }}
        onBlur={() => setDraft(null)}
        className="scroll-shown min-h-0 flex-1 resize-none rounded-lg border border-white/[.08] bg-white/[.055] px-3 py-2.5 text-[15px] leading-[22px] text-fg placeholder:text-muted focus:border-accent disabled:opacity-40"
      />
    </>
  );
}

/**
 * The script as the creator reads it while recording: larger, not editable, scrolled with the wheel or the arrow
 * keys, which have the keyboard from the start.
 */
function ReadingView() {
  const script = useEditor((s) => s.snap?.project.script ?? "");
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => ref.current?.focus({ preventScroll: true }), []);
  return (
    <div
      ref={ref}
      tabIndex={0}
      data-voiceover-reading
      aria-label="Script"
      className="scroll-shown mt-4 min-h-0 flex-1 overflow-y-auto rounded-lg text-[19px] leading-[30px] whitespace-pre-wrap text-fg outline-none focus-visible:outline-2 focus-visible:outline-accent"
    >
      {script || <span className="text-muted">No script. Say what you like while the video plays.</span>}
    </div>
  );
}

/**
 * Record voiceover, in the library's place: the microphone and its meter, the script, and Record, which counts down
 * 3, 2, 1 and records from the playhead while the video plays without its sound. Stop puts the take on the
 * timeline where it started, as one undo step.
 */
export function VoiceoverPanel() {
  const { phase, microphone, error, startUs, unsaved, asking } = useVoiceover();
  const playhead = useEditor((s) => s.timeUs);
  const atVideoEnd = useEditor((s) => videoEndReason(s.snap?.project, s.timeUs));
  const lock = useLockReason();
  const recording = phase === "recording" || phase === "stopping";
  const counting = phase === "countdown";

  useEffect(() => {
    if (phase !== "countdown" && phase !== "recording") return;
    window.addEventListener("keydown", voiceoverKeys, true);
    return () => window.removeEventListener("keydown", voiceoverKeys, true);
  }, [phase]);

  // Toasts move above Record, Stop or Add while the voiceover shows, so an Undo toast never covers them.
  const footer = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    useEditor.setState({ toastLift: footer.current?.offsetHeight ?? 0 });
  }, [phase, unsaved]);
  useEffect(() => () => useEditor.setState({ toastLift: 0 }), []);

  const recordReason =
    lock ??
    (phase === "opening"
      ? "Opening the microphone…"
      : !microphone
        ? "Nuzky could not open the microphone"
        : unsaved
          ? "Add or discard the last voiceover first"
          : atVideoEnd);
  const close = () => void closeVoiceover();
  return (
    <div
      className="flex min-h-0 flex-1 flex-col px-3.5"
      data-voiceover
      onKeyDown={(e) => {
        if (e.key === "Escape" && phase === "ready") {
          e.stopPropagation();
          close();
        }
      }}
    >
      <div className="flex h-12 shrink-0 items-center gap-2">
        {recording ? (
          <>
            <span aria-hidden className="h-2 w-2 shrink-0 animate-pulse rounded-full bg-danger motion-reduce:animate-none" />
            <h2 className="text-[13px] font-semibold text-fg">Recording</h2>
            <TakeLength />
          </>
        ) : (
          <>
            <h2 className="text-[13px] font-semibold text-fg">Voiceover</h2>
            <div className="flex-1" />
            <LockReason.Provider value={null}>
              <IconButton round label="Close voiceover" disabled={counting} onClick={close}>
                <X size={16} />
              </IconButton>
            </LockReason.Provider>
          </>
        )}
      </div>

      <div className="flex items-start gap-1.5 text-[12px] leading-4 text-muted">
        <Mic size={14} className="mt-px shrink-0" />
        <span className={asking ? "" : "truncate"} data-microphone>
          {microphone ?? (asking ? "Allow Nuzky to use the microphone in the window macOS shows" : phase === "opening" ? "Opening the microphone…" : "No microphone")}
        </span>
      </div>
      {error ? (
        <div className="mt-2 flex flex-col items-start gap-2">
          <p role="alert" data-mic-error className="flex items-start gap-1.5 text-[12px] leading-4 text-fg">
            <TriangleAlert size={14} className="mt-px shrink-0 text-danger" />
            <span>{error}</span>
          </p>
          <LockReason.Provider value={null}>
            <Button
              variant="plain"
              className="h-7 px-3 text-[12px]"
              onClick={() => void reopenVoiceover()}
            >
              Try again
            </Button>
          </LockReason.Provider>
        </div>
      ) : (
        <Meter reading={recording || counting} />
      )}

      {recording || counting ? <ReadingView /> : <ScriptField disabled={lock} />}

      <div ref={footer} className="shrink-0 pt-3 pb-3.5">
        {unsaved && !recording ? (
          <>
            <p className="mb-2 text-[12px] leading-4 text-muted">
              {lock ? "Your take is not on the timeline yet. Add it once the AI is done editing." : "Your take is not on the timeline yet."}
            </p>
            <LockReason.Provider value={null}>
              <div className="flex gap-2">
                <Button variant="primary" pill className="h-9 flex-1" disabled={!!lock} disabledReason={lock ?? undefined} onClick={() => void stopVoiceover(true)}>
                  Add to timeline
                </Button>
                <Button variant="ghost" pill className="h-9 px-3" onClick={() => void stopVoiceover(false)}>
                  Discard
                </Button>
              </div>
            </LockReason.Provider>
          </>
        ) : recording ? (
          <LockReason.Provider value={null}>
            <div className="flex gap-2">
              <Button variant="primary" pill className="h-9 flex-1 gap-2" disabled={phase === "stopping"} disabledReason="Adding the voiceover…" onClick={() => void stopVoiceover(true)}>
                <Square size={12} fill="currentColor" />
                Stop
                <kbd aria-hidden className="rounded-[5px] px-[5px] py-0.5 font-sans text-[11px] leading-[13px] font-medium text-white/85 shadow-[inset_0_0_0_1px_rgb(255_255_255/.35)]">
                  Space
                </kbd>
              </Button>
              <Button variant="ghost" pill className="h-9 px-3" disabled={phase === "stopping"} onClick={() => void stopVoiceover(false)}>
                Discard
              </Button>
            </div>
          </LockReason.Provider>
        ) : (
          <>
            <p className="mb-2 text-[12px] leading-4 text-muted">
              Starts at <span className="tabular">{formatTime(counting ? startUs : playhead)}</span>. The video plays without sound while you record.
            </p>
            {counting ? (
              <LockReason.Provider value={null}>
                <Button pill className="h-9 w-full" onClick={cancelCountdown}>
                  Cancel
                </Button>
              </LockReason.Provider>
            ) : (
              <Button variant="primary" pill className="h-9 w-full gap-2" disabled={!!recordReason} disabledReason={recordReason ?? undefined} onClick={startCountdown} data-record-voiceover>
                <span aria-hidden className="h-2.5 w-2.5 rounded-full bg-danger shadow-[0_0_0_1.5px_rgb(255_255_255/.9)]" />
                Record
              </Button>
            )}
          </>
        )}
      </div>
    </div>
  );
}

/** 3, 2, 1 over the preview's frame before the take starts. */
export function VoiceoverCountdown() {
  const counting = useVoiceover((s) => s.phase === "countdown");
  const count = useVoiceover((s) => s.count);
  if (!counting) return null;
  return (
    <div data-voiceover-countdown role="status" aria-live="assertive" className="pointer-events-none absolute inset-0 z-20 flex flex-col items-center justify-center gap-3.5 bg-black/35">
      <div className="tabular flex h-[140px] w-[140px] items-center justify-center rounded-full bg-black/60 text-[76px] font-semibold text-fg">{count}</div>
      <p className="rounded-lg bg-black/60 px-2.5 py-1 text-[12px] text-fg">
        Recording in {count} s ·{" "}
        <kbd className="rounded-[5px] px-[5px] py-0.5 font-sans text-[11px] leading-[13px] font-medium text-fg/85 shadow-[inset_0_0_0_1px_rgb(255_255_255/.35)]">Esc</kbd> cancels
      </p>
    </div>
  );
}

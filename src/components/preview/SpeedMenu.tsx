import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { Check, Rewind } from "lucide-react";
import { chooseSpeed, recordingVoiceover } from "../../lib/actions";
import { useVoiceover } from "../../lib/voiceover";
import { chose } from "../../lib/exit";
import { SPEEDS, speedLabel, usePreview } from "../../lib/preview";
import { useEditor } from "../../lib/store";
import { useKeyLabels } from "../../lib/keys";

/** The preview's playback speed beside the time: what it plays at now, and a menu to choose the speed Play uses. */
export function SpeedMenu({ disabled }: { disabled: boolean }) {
  const { key } = useKeyLabels();
  const playing = useEditor((s) => s.playing);
  const rate = usePreview((s) => s.rate);
  const speed = usePreview((s) => s.speed);
  const recording = useVoiceover((s) => recordingVoiceover(s.phase));
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const shown = playing ? Math.abs(rate) : speed;
  const backward = playing && rate < 0;
  const keys = [key("playBackward"), key("stop"), key("playForward")].filter(Boolean).join(", ");
  const title = recording ? "The voiceover records at 1x" : `Preview speed${keys ? ` (${keys})` : ""}`;

  const closeToTrigger = () => {
    setOpen(false);
    trigger.current?.focus();
  };

  useEffect(() => {
    if (!open) return;
    ref.current?.querySelector<HTMLElement>("[aria-checked=true]")?.focus();
    const close = (e: PointerEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const escape = (e: globalThis.KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      closeToTrigger();
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", escape, true);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", escape, true);
    };
  }, [open]);

  const onMenuKey = (e: KeyboardEvent<HTMLDivElement>) => {
    e.stopPropagation();
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = [...e.currentTarget.querySelectorAll<HTMLElement>("[role=menuitemradio]")];
    const i = items.indexOf(document.activeElement as HTMLElement);
    items[(i + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length]?.focus();
  };

  return (
    <div className="relative" ref={ref}>
      <button
        ref={trigger}
        type="button"
        data-speed
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`${title}: ${backward ? "backwards at " : ""}${speedLabel(shown)}`}
        title={title}
        disabled={disabled || recording}
        onClick={() => setOpen(!open)}
        className={`tabular flex h-8 min-w-[44px] items-center justify-center gap-1 rounded-full px-2 text-[12px] font-medium transition-colors duration-[120ms] ease-out disabled:cursor-not-allowed disabled:opacity-40 ${
          open ? "bg-white/[.14]" : "enabled:hover:bg-white/[.08]"
        } ${shown !== 1 || backward ? "text-accent" : "text-fg"}`}
      >
        {backward && <Rewind size={12} fill="currentColor" aria-hidden />}
        {speedLabel(shown)}
      </button>
      {open && (
        <div data-exit role="menu" aria-label="Preview speed" onKeyDown={onMenuKey} className="overlay menu-in absolute right-0 bottom-11 z-50 w-[7.5rem] origin-bottom-right rounded-xl p-1">
          {SPEEDS.map((s) => (
            <button
              key={s}
              type="button"
              role="menuitemradio"
              aria-checked={s === speed}
              onClick={(e) => {
                chose(e.currentTarget);
                chooseSpeed(s);
                closeToTrigger();
              }}
              className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left hover:bg-white/[.08] focus-visible:bg-white/[.08] focus-visible:outline-offset-[-2px]"
            >
              <span className="tabular flex-1 text-[13px] font-medium text-fg">{speedLabel(s)}</span>
              {s === speed && <Check size={14} className="shrink-0 text-accent" />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

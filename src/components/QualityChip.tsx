import { useEffect, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";
import { canvasSize } from "../lib/presets";
import { AI_EDITING, projectDuration, useAiLocked, useEditor } from "../lib/store";
import { formatLength } from "../lib/time";
import { FRAME_RATES, QUALITIES, RESOLUTIONS, estimateBytes, formatBytes, loadOptions, outputSize } from "./ExportDialog";
import { AiLock, Button, Segmented } from "./ui";

const NAMES: Record<number, string> = { 720: "HD", 1080: "FHD", 1440: "2K", 2160: "4K" };
export const resolutionName = (short: number) => NAMES[short] ?? `${short}p`;

/**
 * The video's quality, as the export starts from it: its resolution and frame rate ("FHD 30"). Its popover changes the
 * project's canvas and frame rate in one undo step, which the preview plays and the export writes from then on, and
 * estimates the file of the whole timeline.
 */
export function QualityChip() {
  const canvas = useEditor((s) => s.snap?.project.canvas);
  const duration = useEditor((s) => (s.snap ? projectDuration(s.snap.project) : 0));
  // The export dialog remembers its resolution, preset and quality; the chip shows them again once it closes.
  useEditor((s) => s.exportOpen);
  const edit = useEditor((s) => s.edit);
  const locked = useAiLocked();
  /** Open, and whether from the keyboard, which then moves focus into it. */
  const [open, setOpen] = useState<{ keyboard: boolean } | null>(null);
  const box = useRef<HTMLDivElement>(null);

  const closeToTrigger = () => {
    setOpen(null);
    box.current?.querySelector<HTMLElement>("[data-quality]")?.focus();
  };

  useEffect(() => {
    if (!open) return;
    if (open.keyboard) box.current?.querySelector<HTMLElement>("[role=dialog] [aria-pressed=true]")?.focus();
    const outside = (e: PointerEvent) => !box.current?.contains(e.target as Node) && setOpen(null);
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      closeToTrigger();
    };
    window.addEventListener("pointerdown", outside);
    window.addEventListener("keydown", key, true);
    return () => {
      window.removeEventListener("pointerdown", outside);
      window.removeEventListener("keydown", key, true);
    };
  }, [open]);

  if (!canvas) return null;
  const options = loadOptions(canvas, null);
  const size = outputSize(canvas, options.resolution);
  const name = resolutionName(options.resolution);
  const bitrate = QUALITIES.find((q) => q.id === options.quality)!.label;
  const choose = (resolution: number, fps: number) => {
    if (resolution === options.resolution && fps === options.fps) return;
    // The size comes from the latest confirmed canvas, so a Ratio change made a moment ago is kept.
    edit((p) => ({ type: "setCanvas", ...canvasSize(p.canvas, resolution), fps }));
  };

  return (
    <div className="relative" ref={box}>
      <Button
        variant="bar"
        pill
        data-quality
        aria-haspopup="dialog"
        aria-expanded={!!open}
        aria-label={`Quality: ${name}, ${options.fps} fps`}
        title={`Quality: ${size.w}×${size.h}, ${options.fps} fps`}
        disabled={locked}
        disabledReason={AI_EDITING}
        // A click has a pointer position; Enter and Space have none.
        onClick={(e) => setOpen(open ? null : { keyboard: e.detail === 0 })}
        className={`tabular ${open ? "bg-white/[.13]" : ""}`}
      >
        {name} {options.fps}
        <ChevronDown size={13} className="text-muted" />
      </Button>
      {open && (
        // Opaque like a dialog: its controls lie over the inspector's text.
        <div
          data-exit
          role="dialog"
          aria-label="Quality"
          // Keys stay here: Space or an arrow would otherwise play or move the playhead.
          onKeyDown={(e) => e.stopPropagation()}
          onBlur={(e) => !e.currentTarget.contains(e.relatedTarget as Node | null) && e.relatedTarget && setOpen(null)}
          className="overlay overlay-solid menu-in absolute right-0 top-10 z-50 flex w-[18rem] origin-top-right flex-col gap-2 rounded-xl p-3"
        >
          <AiLock>
            <div className="flex items-baseline justify-between text-[12px] text-muted">
              <span>Resolution</span>
              <span className="tabular">
                {size.w}×{size.h}
              </span>
            </div>
            <Segmented
              label="Resolution"
              value={options.resolution}
              onChange={(resolution) => choose(resolution, options.fps)}
              options={RESOLUTIONS.map((r) => ({ id: r.id, label: resolutionName(r.id), title: r.label }))}
            />
            <span className="mt-1 text-[12px] text-muted">Frame rate</span>
            <Segmented
              label="Frame rate"
              compact
              value={options.fps}
              onChange={(fps) => choose(options.resolution, fps)}
              options={FRAME_RATES.map((f) => ({ id: f, label: `${f}` }))}
            />
          </AiLock>
          <div
            className="tabular mt-1 flex items-baseline justify-between gap-3 border-t border-white/[.08] pt-2.5 text-[12px] text-muted"
            aria-live="polite"
            title={duration > 0 ? `Estimate at ${bitrate} quality; scenes with a lot of motion come out larger` : undefined}
          >
            {duration > 0 ? (
              <>
                <span>File for {formatLength(duration)}</span>
                <span data-estimate>
                  about <span className="text-[13px] font-semibold text-fg">{formatBytes(estimateBytes(size.w, size.h, options.fps, options.quality, duration))}</span>
                </span>
              </>
            ) : (
              <span>Add a clip to the timeline to see the file size</span>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

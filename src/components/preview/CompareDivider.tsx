import { useEffect, useRef, type KeyboardEvent, type PointerEvent } from "react";
import { ChevronsLeftRight } from "lucide-react";
import { api } from "../../lib/api";
import { followPointer } from "../../lib/drag";
import { usePreview } from "../../lib/preview";

const STEP = 0.05;
const CHIP = "absolute top-2 rounded-md bg-black/60 px-1.5 py-0.5 text-[11px] font-medium text-fg";

/**
 * The line between the picture before and after the clips' adjustments, over a frame `width` px wide. The engine draws
 * both sides of the frame itself, so each is what the export would show without and with them.
 */
export function CompareDivider({ width }: { width: number }) {
  const split = usePreview((s) => s.compare) ?? 0.5;
  const ref = useRef<HTMLDivElement>(null);
  // At most one frame a display frame however fast the pointer moves, the last position always included.
  useEffect(() => {
    const raf = requestAnimationFrame(() => void api.setPreviewCompare(split));
    return () => cancelAnimationFrame(raf);
  }, [split]);
  useEffect(() => () => void api.setPreviewCompare(null), []);

  const set = (value: number) => usePreview.setState({ compare: Math.min(1, Math.max(0, value)) });
  const at = (clientX: number) => {
    const r = ref.current!.getBoundingClientRect();
    return (clientX - r.left) / Math.max(r.width, 1);
  };
  const grab = (e: PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    (e.currentTarget as HTMLElement).focus({ preventScroll: true });
    // The line moves with the pointer from where it was grabbed, without jumping under it.
    const offset = at(e.clientX) - split;
    followPointer({ move: (m) => set(at(m.clientX) - offset) });
  };
  const keys = (e: KeyboardEvent) => {
    const next = e.key === "ArrowLeft" ? split - STEP : e.key === "ArrowRight" ? split + STEP : e.key === "Home" ? 0 : e.key === "End" ? 1 : null;
    if (next === null) return;
    e.preventDefault();
    e.stopPropagation();
    set(next);
  };
  // Whole pixels and no transform: WebKitGTK left slivers of a moved handle behind.
  const x = Math.round(split * width);
  const percent = Math.round(split * 100);
  return (
    <div ref={ref} className="pointer-events-none absolute inset-0" data-testid="compare">
      {split > 0.2 && <span className={`${CHIP} left-2`}>Before</span>}
      {split < 0.8 && <span className={`${CHIP} right-2`}>After</span>}
      <div className="absolute inset-y-0 w-0.5 bg-fg shadow-[0_0_0_1px_rgb(0_0_0/.25)]" style={{ left: x - 1 }} />
      {/* Only the knob takes the pointer, so the selected layer's handles under the line stay in reach. */}
      <div
        role="slider"
        tabIndex={0}
        aria-label="Before and after divider"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        aria-valuetext={`${percent} % before`}
        title="Drag to compare the picture before and after its adjustments"
        className="pointer-events-auto absolute top-[calc(50%-16px)] flex h-8 w-8 cursor-ew-resize touch-none items-center justify-center rounded-full bg-fg text-black shadow-[0_1px_3px_rgb(0_0_0/.4)] focus-visible:outline-offset-2"
        style={{ left: x - 16 }}
        onPointerDown={grab}
        onKeyDown={keys}
      >
        <ChevronsLeftRight size={16} />
      </div>
    </div>
  );
}

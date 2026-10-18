import { memo, useEffect, useRef, useState } from "react";
import { Plus } from "lucide-react";
import { DEFAULT_TRANSITION_US, TRANSITIONS } from "../../lib/presets";
import { mainCuts, useEditor } from "../../lib/store";
import { US, formatDuration } from "../../lib/time";
import type { Project } from "../../lib/types";
import { TRANSITION_ICONS } from "../panel/TransitionsTab";

const NEAR_PX = 12;

/** A square on every main-track cut with a transition; an empty cut shows "+" (adds a Dissolve) only near the pointer or focus. */
export const CutMarkers = memo(function CutMarkers({ project, zoom, locked }: { project: Project; zoom: number; locked: boolean }) {
  const selected = useEditor((s) => s.cut);
  const { edit, selectCut } = useEditor.getState();
  const cuts = mainCuts(project);
  const anchor = useRef<HTMLSpanElement>(null);
  const [near, setNear] = useState<string | null>(null);

  useEffect(() => {
    const lane = anchor.current?.parentElement;
    if (!lane) return;
    const move = (e: PointerEvent) => {
      // A press keeps what shows, so a click on "+" that jitters still lands on it.
      if (e.buttons) return;
      const x = e.clientX - lane.getBoundingClientRect().left;
      let found: string | null = null;
      let best = NEAR_PX;
      for (const cut of cuts) {
        const d = Math.abs((cut.atUs / US) * zoom - x);
        if (d <= best) [best, found] = [d, cut.clipId];
      }
      setNear(found);
    };
    const leave = () => setNear(null);
    lane.addEventListener("pointermove", move);
    lane.addEventListener("pointerleave", leave);
    return () => {
      lane.removeEventListener("pointermove", move);
      lane.removeEventListener("pointerleave", leave);
    };
  }, [cuts, zoom]);

  return (
    <>
      <span ref={anchor} hidden />
      {cuts.map((cut) => {
        const x = (cut.atUs / US) * zoom;
        const t = cut.transition;
        const Icon = t ? TRANSITION_ICONS[t.kind] : Plus;
        const name = t ? TRANSITIONS.find((k) => k.kind === t.kind)?.label : null;
        const on = selected === cut.clipId;
        return (
          <div key={cut.clipId}>
            {t && (
              <div
                className={`pointer-events-none absolute inset-y-1 z-[15] rounded-[7px] bg-black/45 ${on ? "border-2 border-accent" : "border border-white/40"}`}
                style={{ left: x - ((t.durationUs / US) * zoom) / 2, width: (t.durationUs / US) * zoom }}
              />
            )}
            <button
              type="button"
              data-cut={cut.clipId}
              aria-label={t ? `${name} transition, ${formatDuration(t.durationUs)}` : "Add transition"}
              title={t ? `${name} · ${formatDuration(t.durationUs)}` : locked ? "Add transition: the AI is editing" : "Add transition (Dissolve)"}
              aria-pressed={on}
              aria-disabled={(locked && !t) || undefined}
              onPointerDown={(e) => e.stopPropagation()}
              onClick={async () => {
                if (locked && !t) return;
                if (!t && !(await edit({ type: "setTransition", clipId: cut.clipId, transition: { kind: "dissolve", durationUs: DEFAULT_TRANSITION_US } }))) return;
                selectCut(cut.clipId);
              }}
              // On the clips' top edge, covering only the top corner of the trim edges; 2 px lower than centred, so the
              // sticky ruler never hides its top on the first track.
              // A crisp dark edge instead of a soft shadow: there is one per cut.
              className={`absolute -top-0.5 z-20 flex h-4 w-4 -translate-x-1/2 items-center justify-center rounded-[5px] border transition-[background-color,border-color,opacity] duration-[120ms] ease-out aria-disabled:cursor-not-allowed ${
                t
                  ? on
                    ? "border-accent bg-accent text-black"
                    : "border-black/40 bg-fg text-black hover:bg-white"
                  : `border-white/50 bg-[rgb(20_20_24/.9)] text-fg focus-visible:opacity-100 ${near === cut.clipId ? "" : "pointer-events-none opacity-0"}`
              }`}
              style={{ left: x }}
            >
              <Icon size={10} />
            </button>
          </div>
        );
      })}
    </>
  );
});

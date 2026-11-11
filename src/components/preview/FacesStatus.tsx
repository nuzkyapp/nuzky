import { useRef } from "react";
import { Loader2 } from "lucide-react";
import { api } from "../../lib/api";
import { current, moveAll, show, useFaces } from "../../lib/faces";
import { useFollow } from "../../lib/follow";
import { useReframe } from "../../lib/reframe";
import { AI_EDITING, undoAction, useAiLocked, useEditor } from "../../lib/store";
import { formatTime } from "../../lib/time";
import { Button } from "../ui";

const KIND = { caption: "Caption", text: "Text", sticker: "Sticker" };

/**
 * The face check in the cover's status chip, bottom middle of the preview area: its progress with Stop, then each
 * caption, text or sticker over a face with Next, Move all and Done, then what Move all did with Undo. Above Reframe's or Follow's
 * chip while that one shows. Stop, Next and Done change nothing, so an agent's run leaves them usable.
 */
export function FacesStatus() {
  const id = useFaces((s) => s.job);
  const found = useFaces((s) => s.found);
  const moved = useFaces((s) => s.moved);
  const job = useEditor((s) => (id ? s.jobs[id] : undefined));
  const snap = useEditor((s) => s.snap);
  const locked = useAiLocked();
  // As ReframeStatus and FollowStatus decide whether their chip shows.
  const reframeJob = useReframe((s) => s.job);
  const reframeDone = useReframe((s) => s.done);
  const followChip = useFollow((s) => !!s.picking || !!s.job || (!!s.done && !!undoAction(s.done.snap).valid?.()));
  const reframing = !!reframeJob || (!!reframeDone && !!undoAction(reframeDone.snap).valid?.()) || followChip;
  const ref = useRef<HTMLDivElement>(null);
  // Keyboard focus goes back to the button that started the check, not to the page, when the chip's button goes.
  const leave = () => {
    if (ref.current?.contains(document.activeElement)) document.querySelector<HTMLElement>("[data-check-faces]")?.focus();
  };
  const close = () => {
    leave();
    useFaces.setState({ found: null, moved: null });
  };
  const chip = `absolute ${reframing ? "bottom-11" : "bottom-2"} left-1/2 z-20 flex h-7 max-w-[calc(100%-16px)] -translate-x-1/2 items-center gap-1.5 whitespace-nowrap rounded-lg bg-black/70 pl-2 pr-1 text-[12px] text-fg`;
  const small = "h-5 shrink-0 px-2 text-[11px]";
  const done = (
    <Button pill className={small} onClick={close}>
      Done
    </Button>
  );
  if (id)
    return (
      <div ref={ref} className={chip} role="status">
        <Loader2 size={13} className="shrink-0 animate-spin text-accent" />
        <span className="tabular truncate">
          {job?.phase ?? "Looking for faces"}
          {job && job.progress > 0 ? ` · ${Math.round(job.progress * 100)}%` : "…"}
        </span>
        <Button
          pill
          className={small}
          onClick={() => {
            leave();
            void api.cancelJob(id);
          }}
        >
          Stop
        </Button>
      </div>
    );
  if (moved) {
    const undo = undoAction(moved.snap);
    if (!undo.valid?.()) return null;
    return (
      <div ref={ref} className={chip} role="status">
        <span className="truncate" title={moved.text}>
          {moved.text}
        </span>
        <Button
          pill
          className={small}
          disabled={locked}
          disabledReason={AI_EDITING}
          onClick={() => {
            close();
            undo.run();
          }}
        >
          Undo
        </Button>
        {done}
      </div>
    );
  }
  // Findings of another project are not shown in this one.
  if (!found || found.sessionEpoch !== snap?.sessionEpoch) return null;
  const item = found.covered[found.at];
  const count = found.covered.length;
  // The time and kind first: in a narrow preview the label is what gets cut.
  const text = `${formatTime(item.time_us)} · ${KIND[item.kind]} over a face · “${item.label}”`;
  const stale = !current(found, snap);
  const movable = found.covered.some((c) => c.suggestion);
  return (
    <div ref={ref} className={chip} role="status">
      <span className="truncate" title={text}>
        {text}
      </span>
      {count > 1 && (
        <>
          <span className="tabular shrink-0 text-muted">
            {found.at + 1} of {count}
          </span>
          <Button pill className={small} onClick={() => show((found.at + 1) % count)}>
            Next
          </Button>
        </>
      )}
      <Button
        pill
        className={small}
        disabled={locked || stale || !movable}
        disabledReason={locked ? AI_EDITING : stale ? "The project changed since the check. Find text over faces again to move them." : "No free place nearby keeps clear of the face"}
        onClick={() => {
          leave();
          void moveAll();
        }}
      >
        {count > 1 ? "Move all" : "Move"}
      </Button>
      {done}
    </div>
  );
}

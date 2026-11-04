import { useRef } from "react";
import { Loader2 } from "lucide-react";
import { api } from "../../lib/api";
import { checkFaces, current, moveAll, show, useFaces } from "../../lib/faces";
import { useReframe } from "../../lib/reframe";
import { undoAction, useEditor } from "../../lib/store";
import { formatTime } from "../../lib/time";
import { AiLock, Button } from "../ui";

const KIND = { caption: "Caption", text: "Text", sticker: "Sticker" };

/**
 * The face check in the cover's status chip, bottom middle of the preview area: its progress with Stop, then each
 * caption, text or sticker over a face with Next, Move all and Done, then what Move all did with Undo.
 */
export function FacesStatus() {
  return (
    <AiLock>
      <Chip />
    </AiLock>
  );
}

function Chip() {
  const id = useFaces((s) => s.job);
  const found = useFaces((s) => s.found);
  const moved = useFaces((s) => s.moved);
  const job = useEditor((s) => (id ? s.jobs[id] : undefined));
  const snap = useEditor((s) => s.snap);
  // Reframe's chip sits in the same place; a reframe started after a check shows over it.
  const reframing = useReframe((s) => !!(s.job || s.done));
  const ref = useRef<HTMLDivElement>(null);
  const leave = () => {
    if (ref.current?.contains(document.activeElement)) document.querySelector<HTMLElement>("[data-check-faces]")?.focus();
  };
  const close = () => {
    leave();
    useFaces.setState({ found: null, moved: null });
  };
  const chip =
    "absolute bottom-2 left-1/2 z-20 flex h-7 max-w-[calc(100%-16px)] -translate-x-1/2 items-center gap-1.5 whitespace-nowrap rounded-lg bg-black/70 pl-2 pr-1 text-[12px] text-fg";
  const small = "h-5 shrink-0 px-2 text-[11px]";
  const done = (
    <Button pill className={small} onClick={close}>
      Done
    </Button>
  );
  if (reframing) return null;
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
  if (!found) return null;
  if (!current(found, snap))
    return (
      <div ref={ref} className={chip} role="status">
        <span className="truncate">The project changed since the face check</span>
        <Button pill className={small} onClick={() => void checkFaces()}>
          Check again
        </Button>
        {done}
      </div>
    );
  const item = found.covered[found.at];
  const count = found.covered.length;
  // The time and kind first: in a narrow preview the label is what gets cut.
  const text = `${formatTime(item.start_us)} · ${KIND[item.kind]} over a face · “${item.label}”`;
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
        disabled={!movable}
        disabledReason="No place nearby keeps clear of the face and inside the safe area"
        onClick={() => void moveAll()}
      >
        {count > 1 ? "Move all" : "Move"}
      </Button>
      {done}
    </div>
  );
}

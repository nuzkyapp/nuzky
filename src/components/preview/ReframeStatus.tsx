import { useRef, type ReactNode } from "react";
import { Loader2 } from "lucide-react";
import { api } from "../../lib/api";
import { useReframe } from "../../lib/reframe";
import { undoAction, useEditor } from "../../lib/store";
import type { Snapshot } from "../../lib/types";
import { Button } from "../ui";

export const CHIP =
  "absolute bottom-2 left-1/2 z-20 flex h-7 max-w-[calc(100%-16px)] -translate-x-1/2 items-center gap-1.5 whitespace-nowrap rounded-lg bg-black/70 pl-2 pr-1 text-[12px] text-fg";

/** A small capsule of the status chip. */
export function ChipButton({ children, ...rest }: { onClick: () => void; children: ReactNode; title?: string; disabled?: boolean; disabledReason?: string }) {
  return (
    <Button pill className="h-5 shrink-0 px-2 text-[11px]" {...rest}>
      {children}
    </Button>
  );
}

/**
 * Bottom middle of the preview area, above the transport, in the cover's status chip: a job's progress with Stop,
 * then what it did with Undo while it is still the newest step, and Done. The chip goes away under the keyboard's
 * focus; `back` names the control that started the job, which gets the focus instead.
 */
export function JobChip({ id, working, done, close, back }: { id: string | null; working: string; done: { snap: Snapshot; text: string } | null; close: () => void; back: string }) {
  const job = useEditor((s) => (id ? s.jobs[id] : undefined));
  // Undo applies only while the job's step is the newest; any later edit hides it.
  useEditor((s) => s.snap);
  const ref = useRef<HTMLDivElement>(null);
  const leave = () => {
    if (ref.current?.contains(document.activeElement)) document.querySelector<HTMLElement>(back)?.focus();
  };
  if (id)
    return (
      <div ref={ref} className={CHIP} role="status">
        <Loader2 size={13} className="shrink-0 animate-spin text-accent" />
        <span className="tabular truncate">
          {job?.phase ?? working}
          {job && job.progress > 0 ? ` · ${Math.round(job.progress * 100)}%` : "…"}
        </span>
        <ChipButton
          onClick={() => {
            leave();
            void api.cancelJob(id);
          }}
        >
          Stop
        </ChipButton>
      </div>
    );
  if (!done) return null;
  const undo = undoAction(done.snap);
  if (!undo.valid?.()) return null;
  const finish = () => {
    leave();
    close();
  };
  return (
    <div ref={ref} className={CHIP} role="status">
      <span className="truncate" title={done.text}>
        {done.text}
      </span>
      <ChipButton
        onClick={() => {
          finish();
          undo.run();
        }}
      >
        Undo
      </ChipButton>
      <ChipButton onClick={finish}>Done</ChipButton>
    </div>
  );
}

/** The reframe's chip; closing it by keyboard returns focus to Ratio, where Reframe was chosen. */
export function ReframeStatus() {
  const id = useReframe((s) => s.job);
  const done = useReframe((s) => s.done);
  return <JobChip id={id} working="Following the face" done={done} close={() => useReframe.setState({ done: null })} back="[data-ratio]" />;
}

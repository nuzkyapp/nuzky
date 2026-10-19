import { useRef } from "react";
import { AlertCircle, Check, ScanFace, Sparkles } from "lucide-react";
import { coverAiBlocked, makeCoverWithAi, useAgent } from "../../lib/agent";
import { api } from "../../lib/api";
import { candidateReason, chooseFrame, downloadModels, pick, useCandidates, useCover } from "../../lib/cover";
import { useEditor } from "../../lib/store";
import { formatTime } from "../../lib/time";
import type { CoverCandidate, ThumbnailFormat } from "../../lib/types";
import { useShownCover } from "./CoverStage";
import { AiLock, Button, ProgressBar, lockedProps, useLockReason } from "../ui";

/**
 * The library's place in the cover editor. Until there is a cover, Make it with AI leads: the agent picks the frame
 * and writes the hook. Then Pick for me and the frames it found.
 */
export function CoverFrames({ format }: { format: ThumbnailFormat }) {
  // Either cover: the AI makes both, so it no longer leads once one is made by hand.
  const made = useEditor((s) => !!s.snap?.project.thumbnails?.length);
  return (
    <div className="flex min-h-0 flex-1 flex-col" data-testid="cover-frames">
      <div className="flex h-12 shrink-0 items-center justify-between gap-2 border-b border-white/[.07] pl-4 pr-2">
        <h2 className="text-[13px] font-semibold text-fg">Frames</h2>
        {made && (
          <AiLock>
            <MakeWithAi variant="ghost" />
          </AiLock>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4 pt-3">
        <AiLock>
          <Frames format={format} made={made} />
        </AiLock>
      </div>
    </div>
  );
}

/** Why Make it with AI cannot start, read again whenever the agent or the cover models change. */
function useAiBlocked() {
  useAgent((s) => s.agents);
  useAgent((s) => s.agent);
  useAgent((s) => s.status);
  useCover((s) => s.models);
  return coverAiBlocked();
}

function MakeWithAi({ variant }: { variant: "primary" | "ghost" }) {
  const blocked = useAiBlocked();
  return (
    <Button
      variant={variant}
      data-make-cover-ai
      className={variant === "ghost" ? "h-7 gap-1 px-2 text-[12px] text-accent! hover:text-accent!" : "w-full"}
      disabled={!!blocked}
      disabledReason={blocked ?? undefined}
      title="Your AI agent picks the frame, writes the hook and places it"
      onClick={makeCoverWithAi}
    >
      <Sparkles size={variant === "ghost" ? 13 : 15} /> Make it with AI
    </Button>
  );
}

function Frames({ format, made }: { format: ThumbnailFormat; made: boolean }) {
  const candidates = useCandidates();
  const fromImage = useEditor((s) => !!s.snap?.project.thumbnails?.find((t) => t.format === format)?.image);
  const aiBlocked = useAiBlocked();
  // The AI waits for the cover models it needs; their download shows here like a pick.
  const aiWaits = useCover((s) => !!s.afterModels);
  const job = useEditor((s) => {
    const { pickJob, modelsJob, afterModels } = useCover.getState();
    const id = [pickJob, afterModels ? modelsJob : null].find((j) => j && s.jobs[j]?.status === "running");
    return id ? s.jobs[id] : null;
  });
  const failed = useCover((s) => s.failed);
  const damaged = useCover((s) => s.damaged);
  const kept = useCover((s) => s.kept);
  const models = useCover((s) => s.models);
  const time = useShownCover(format)?.cover.timeUs ?? null;
  const fps = useEditor((s) => s.snap?.project.canvas.fps ?? 30);
  const lock = useLockReason();
  const grid = useRef<HTMLDivElement>(null);
  const current = candidates.findIndex((c) => time !== null && Math.abs(c.timeUs - time) < 1e6 / fps / 2);

  const choose = (c: CoverCandidate) => {
    if (lock) return;
    void chooseFrame(format, c.timeUs);
    useEditor.getState().seek(c.timeUs);
  };
  // The grid is one tab stop: arrows move between the frames, Enter or Space chooses one.
  const onKey = (e: React.KeyboardEvent) => {
    const tiles = [...(grid.current?.querySelectorAll<HTMLButtonElement>("[data-candidate]") ?? [])];
    const at = tiles.indexOf(document.activeElement as HTMLButtonElement);
    const columns = Math.max(1, Math.round((grid.current?.clientWidth ?? 1) / (tiles[0]?.offsetWidth || 1)));
    const step = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -columns, ArrowDown: columns }[e.key];
    const to = e.key === "Home" ? 0 : e.key === "End" ? tiles.length - 1 : step !== undefined ? at + step : -1;
    if (to < 0 || to >= tiles.length || at < 0) return;
    e.preventDefault();
    e.stopPropagation();
    tiles[to].focus();
  };

  return (
    <div className="flex flex-col gap-3">
      {job ? (
        <div className="flex flex-col gap-2" role="status">
          <div className="flex items-center justify-between gap-2 text-[12px]">
            <span className="tabular truncate text-fg">
              {job.phase ?? job.label}
              {job.progress > 0 && ` · ${Math.round(job.progress * 100)}%`}
            </span>
            <Button pill className="h-6 shrink-0 px-2.5 text-[12px]" onClick={() => void api.cancelJob(job.id)}>
              Stop
            </Button>
          </div>
          <ProgressBar value={job.progress} label={job.label} />
          {aiWaits && <p className="text-[12px] text-muted">Then the AI makes the cover.</p>}
        </div>
      ) : (
        <div className="flex flex-col gap-1.5">
          {!made && (
            <>
              <MakeWithAi variant="primary" />
              <p className="mb-1.5 text-[12px] text-muted">{aiBlocked ?? "Picks the frame and writes the hook from what is said in the video."}</p>
            </>
          )}
          {/* Prominent once there is a cover, until it has found frames; then picking again is the rarer step. */}
          <Button
            variant={!made || candidates.length > 0 || fromImage ? "secondary" : "primary"}
            className="w-full"
            disabled={!!models?.unavailable}
            disabledReason={models?.unavailable ? "Finding faces doesn't work on this computer. Click the timeline to choose a frame." : undefined}
            onClick={() => void pick(format)}
          >
            <ScanFace size={15} /> {candidates.length > 0 ? "Pick again" : "Pick for me"}
          </Button>
          {fromImage ? (
            <p className="text-[12px] text-muted">The cover is an image. Choosing a frame replaces it.</p>
          ) : (
            candidates.length === 0 && <p className="text-[12px] text-muted">Finds frames with open eyes and a sharp face.</p>
          )}
          {models && !models.downloaded && !models.unavailable && <p className="text-[12px] text-muted">First use downloads the cover models ({models.sizeMb} MB).</p>}
        </div>
      )}
      {failed && !job && (
        <div className="flex flex-col items-start gap-2" role="alert">
          <p className="flex items-start gap-1.5 text-[12px] text-danger">
            <AlertCircle size={14} className="mt-px shrink-0" />
            {failed}
          </p>
          {damaged && (
            <Button pill className="h-6 px-2.5 text-[12px]" onClick={() => void downloadModels()}>
              Download again
            </Button>
          )}
        </div>
      )}
      {candidates.length > 0 && (
        <>
          {candidates.every((c) => !c.face) && <p className="text-[12px] text-muted">No face found in this video. These are the sharpest, best-lit frames.</p>}
          {kept === format && <p className="text-[12px] text-muted">Kept the frame you chose. The best one is first.</p>}
          <div ref={grid} role="listbox" aria-label="Frames for the cover" className="grid grid-cols-[repeat(auto-fill,minmax(96px,1fr))] gap-x-2 gap-y-3" onKeyDown={onKey}>
            {candidates.map((c, i) => {
              const reason = candidateReason(c, format);
              const chosen = i === current;
              return (
                <button
                  key={c.timeUs}
                  type="button"
                  role="option"
                  aria-selected={chosen}
                  data-candidate={c.timeUs}
                  tabIndex={i === Math.max(0, current) ? 0 : -1}
                  title={lock ?? `${formatTime(c.timeUs)} · ${Math.round(c.score * 100)} · ${reason}`}
                  {...lockedProps(lock)}
                  onClick={lock ? undefined : () => choose(c)}
                  className="group flex min-w-0 flex-col gap-1 rounded-[10px] text-left aria-disabled:cursor-not-allowed"
                >
                  <span
                    className={`relative block w-full overflow-hidden rounded-[10px] border bg-bg ${chosen ? "border-accent shadow-[0_0_0_1px_var(--color-accent)]" : "border-white/[.08] group-hover:border-white/30"}`}
                  >
                    <img src={c.still} alt="" className="block w-full" draggable={false} />
                    <span className="tabular absolute bottom-1 right-1 rounded bg-black/60 px-1 text-[11px] leading-4 text-fg">{formatTime(c.timeUs).slice(0, -1)}</span>
                    {chosen && (
                      <span className="absolute right-1 top-1 flex h-4 w-4 items-center justify-center rounded-full bg-accent-strong text-white shadow-[0_1px_2px_rgb(0_0_0/.5)]">
                        <Check size={11} strokeWidth={3} />
                      </span>
                    )}
                  </span>
                  <span className={`truncate text-[11px] leading-[14px] ${chosen ? "text-fg" : "text-muted"}`}>{reason}</span>
                </button>
              );
            })}
          </div>
        </>
      )}
    </div>
  );
}

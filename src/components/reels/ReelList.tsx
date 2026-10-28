import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { AlertCircle, AlertTriangle, Check, ChevronDown, Film, GalleryVerticalEnd, Loader2, Plug, Sparkles } from "lucide-react";
import { api } from "../../lib/api";
import { anyAgent, loadAgents, useAgent } from "../../lib/agent";
import { openChatGpt, useChatGpt } from "../../lib/chatgpt";
import { useModels } from "../../lib/models";
import { useDock } from "../../lib/dock";
import { useLibrary } from "../../lib/library";
import { useTranscriptView } from "../../lib/speech";
import { REEL_LIMIT_US, candidates, choose, findBlocked, findReels, makeReels, moved, openReel, pickAll, setFinish, setFraming, setPicked, setRejected, toggleReelPlay, useReels } from "../../lib/reels";
import { useEditor } from "../../lib/store";
import { formatDuration } from "../../lib/time";
import type { ReelCandidate, ReelFinish } from "../../lib/types";
import { AiLock, Bars, Button, Checkbox, LockReason, Menu, ProgressBar, Segmented } from "../ui";

const plural = (n: number, what: string) => `${n} ${what}${n === 1 ? "" : "s"}`;

function Empty({ icon, title, children, actions }: { icon: React.ReactNode; title: string; children: React.ReactNode; actions?: React.ReactNode }) {
  return (
    <div className="m-auto flex max-w-[440px] flex-col items-center gap-1 px-4 py-6 text-center">
      <span className="text-muted">{icon}</span>
      <p className="mt-1 text-[15px] font-semibold text-fg">{title}</p>
      <p className="text-[13px] text-muted">{children}</p>
      {actions && <div className="mt-3 flex items-center gap-2">{actions}</div>}
    </div>
  );
}

/** The reel's cover once something makes one; until then the empty shape of a vertical video. */
function Thumb() {
  return (
    <span aria-hidden className="flex h-[100px] w-14 shrink-0 items-center justify-center rounded-[7px] border border-dashed border-white/15 text-muted">
      <Film size={14} />
    </span>
  );
}

function Card({ c, current, tabbable, stale }: { c: ReelCandidate; current: boolean; tabbable: boolean; stale: boolean }) {
  const made = c.status === "made";
  const rejected = c.status === "rejected";
  const long = c.durationUs > REEL_LIMIT_US;
  const score = Math.round(c.score * 100);
  const inner = current ? 0 : -1;
  return (
    <div
      role="option"
      aria-selected={current}
      aria-label={`${c.title}, ${formatDuration(c.durationUs)}${made ? ", made" : rejected ? ", rejected" : c.status === "picked" ? ", checked" : ""}`}
      data-reel={c.id}
      tabIndex={tabbable ? 0 : -1}
      onFocus={(e) => e.target === e.currentTarget && !current && choose(c.id)}
      onClick={() => !current && choose(c.id)}
      className={`flex cursor-pointer gap-3 rounded-xl p-2.5 transition-colors duration-[120ms] ease-out focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent ${
        current ? "bg-raised shadow-[inset_0_0_0_2px_var(--color-accent)]" : "bg-white/[.055] shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)] hover:shadow-[inset_0_0_0_1px_rgb(255_255_255/.16)]"
      }`}
    >
      <Thumb />
      <div className={`flex min-w-0 flex-1 flex-col gap-1 ${rejected ? "opacity-45" : ""}`}>
        <div className="flex items-center gap-2">
          {made ? (
            <Check size={16} className="shrink-0 text-ok" aria-label="Made" />
          ) : (
            !rejected && (
              <span onClick={(e) => e.stopPropagation()} className="flex">
                <Checkbox
                  hideLabel
                  tabIndex={inner}
                  label={`Make “${c.title}”`}
                  checked={c.status === "picked" && !stale}
                  disabled={stale}
                  title={stale ? "Its words moved since it was found" : undefined}
                  onChange={(v) => void setPicked(c.id, v)}
                />
              </span>
            )
          )}
          <span className="min-w-0 flex-1 truncate text-[13px] font-semibold text-fg" title={c.title}>
            {c.title}
          </span>
          <span className={`tabular flex shrink-0 items-center gap-1 text-[12px] ${long ? "text-warn" : "text-muted"}`} title={long ? "Longer than 60 s once cut. Trim it, or make it anyway." : "How long the reel is once cut"}>
            {long && <AlertTriangle size={12} />}
            {formatDuration(c.durationUs)}
          </span>
        </div>
        <p className="line-clamp-2 text-[13px] leading-[18px] text-fg/90">“{c.hook}”</p>
        {stale ? (
          <p className="flex items-start gap-1.5 text-[12px] leading-4 text-fg">
            <AlertTriangle size={12} className="mt-0.5 shrink-0 text-warn" />
            Its words moved since it was found. Find again proposes it anew.
          </p>
        ) : (
          c.why && <p className="line-clamp-2 text-[12px] leading-4 text-muted">{c.why}</p>
        )}
        <div className="mt-auto flex items-center gap-2 pt-1">
          {made ? (
            <span className="flex-1 text-[12px] text-muted" title={c.projectPath ?? undefined}>
              Made
            </span>
          ) : (
            <span className="flex flex-1 items-center gap-1.5 text-[12px] text-muted" title={`Score ${score} of 100: how well your AI thinks it works on its own`}>
              <Bars lit={c.score >= 0.75 ? 3 : c.score >= 0.5 ? 2 : 1} />
              <span className="tabular">{score}</span>
              {rejected && <span>· Rejected</span>}
            </span>
          )}
          {made ? (
            // A made reel opens even while the AI edits; switching asks about its changes first.
            <LockReason.Provider value={null}>
              <Button pill tabIndex={inner} className="h-6 px-2.5 text-[12px]" onClick={() => openReel(c, false)}>
                Open
              </Button>
              <Button pill tabIndex={inner} className="h-6 px-2.5 text-[12px]" title="Open it with Export on Reels & TikTok" onClick={() => openReel(c, true)}>
                Export…
              </Button>
            </LockReason.Provider>
          ) : (
            <Button variant="ghost" tabIndex={inner} className="h-6 px-2 text-[12px]" onClick={(e) => (e.stopPropagation(), void setRejected(c.id, !rejected))}>
              {rejected ? "Restore" : "Reject"}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

/** The cards are one tab stop: arrows move between them and show each in the preview. */
function Cards({ list, current }: { list: ReelCandidate[]; current: string | null }) {
  const grid = useRef<HTMLDivElement>(null);
  const words = useReels((s) => s.transcript.words);
  const at = Math.max(0, list.findIndex((c) => c.id === current));

  // A card chosen elsewhere, such as the first one on opening, scrolls into view.
  useEffect(() => {
    grid.current?.querySelector(`[data-reel="${current}"]`)?.scrollIntoView({ block: "nearest" });
  }, [current]);

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const c = list[at];
    if (!c || !(e.target instanceof HTMLElement) || !e.target.dataset.reel) return;
    const mod = e.ctrlKey || e.metaKey;
    const cards = [...(grid.current?.querySelectorAll<HTMLElement>("[data-reel]") ?? [])];
    const columns = Math.max(1, Math.round((grid.current?.clientWidth ?? 1) / (cards[0]?.offsetWidth || 1)));
    const step = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -columns, ArrowDown: columns }[e.key];
    const to = e.key === "Home" ? 0 : e.key === "End" ? list.length - 1 : step !== undefined ? Math.max(0, Math.min(list.length - 1, at + step)) : -1;
    if (to >= 0) {
      e.preventDefault();
      e.stopPropagation();
      choose(list[to].id);
      cards[to]?.focus();
      return;
    }
    const handled = () => (e.preventDefault(), e.stopPropagation());
    // The global key would leave a focused option alone.
    if (e.key === " ") {
      handled();
      toggleReelPlay();
    } else if (e.key === "Enter") {
      handled();
      if (c.status === "made") openReel(c, false);
      else if (c.status === "rejected") void setRejected(c.id, false);
      else void setPicked(c.id, c.status !== "picked");
    } else if (e.key === "Delete" || e.key === "Backspace") {
      handled();
      if (c.status !== "made") void setRejected(c.id, c.status !== "rejected");
    } else if (mod && e.key.toLowerCase() === "a") {
      handled();
      void pickAll();
    } else if (mod && e.key.toLowerCase() === "e" && c.status === "made") {
      handled();
      openReel(c, true);
    }
  };

  return (
    <div ref={grid} role="listbox" aria-label="Reels" onKeyDown={onKey} className="grid min-h-0 flex-1 auto-rows-min grid-cols-[repeat(auto-fill,minmax(320px,1fr))] gap-2 overflow-y-auto p-2 pt-0">
      {list.map((c, i) => (
        <Card key={c.id} c={c} current={c.id === current} tabbable={i === at} stale={moved(c, words)} />
      ))}
    </div>
  );
}

/** What to do while there are no reels: find them, wait for the agent, or why it cannot look. */
function NoReels() {
  const finding = useReels((s) => s.finding);
  const searched = useReels((s) => s.searched);
  const agents = useAgent((s) => s.agents);
  // Read again whenever the ChatGPT sign-in or its models change.
  useChatGpt((s) => s.status);
  useModels((s) => s.lists.chatgpt);
  const failed = useAgent((s) => s.items[s.items.length - 1]?.kind === "error");
  const blocked = useAgent(() => findBlocked());
  useEffect(() => {
    if (agents === null) void loadAgents();
  }, [agents]);

  if (finding)
    return (
      <Empty icon={<Loader2 size={20} className="animate-spin text-accent" />} title="Finding reels">
        Your AI is reading the transcript. Reels show up here as soon as it proposes them.
      </Empty>
    );
  if (agents && !anyAgent())
    return (
      <Empty
        icon={<Sparkles size={20} />}
        title="Sign in to find reels"
        actions={
          <>
            <Button variant="primary" pill className="h-7 px-3 text-[12px]" onClick={openChatGpt}>
              ChatGPT account…
            </Button>
            <Button pill className="h-7 px-3 text-[12px]" onClick={() => void loadAgents()}>
              Check again
            </Button>
            <Button variant="ghost" className="h-7 px-2 text-[12px] text-accent! hover:text-accent!" onClick={() => useEditor.setState({ connectOpen: true })}>
              <Plug size={14} /> Connect an agent in a terminal…
            </Button>
          </>
        }
      >
        Finding reels needs an AI. Sign in with ChatGPT, or install Claude Code or Codex and sign in. You can also connect the agent you use in a terminal and ask it to find reels.
      </Empty>
    );
  const find = (
    <Button variant="primary" pill disabled={!!blocked} disabledReason={blocked ?? undefined} onClick={findReels} data-find-reels>
      <Sparkles size={15} /> Find reels
    </Button>
  );
  if (searched && failed)
    return (
      <Empty icon={<AlertCircle size={20} className="text-danger" />} title="The AI stopped before finding reels" actions={find}>
        The AI panel says why and what to do next.
      </Empty>
    );
  if (searched)
    return (
      <Empty
        icon={<GalleryVerticalEnd size={20} />}
        title="No reels found"
        actions={
          <Button pill className="h-7 px-3 text-[12px]" onClick={() => useDock.setState({ open: true })}>
            Open the AI panel
          </Button>
        }
      >
        Your AI found no moment that works on its own. Tell it in the AI panel what to look for, such as “3 tips, 30 s each”.
      </Empty>
    );
  return (
    <Empty icon={<GalleryVerticalEnd size={20} />} title="No reels yet" actions={find}>
      Your AI picks the moments of this video that work on their own, up to a minute each, and you choose which to make.
    </Empty>
  );
}

const FINISH_STEPS: [keyof ReelFinish, string][] = [
  ["clean", "Remove retakes and fillers"],
  ["captions", "Captions in your style"],
  ["zooms", "Zooms on what is said with emphasis"],
];

/** What each reel gets once made, as a menu of steps to turn on and off. */
function FinishMenu() {
  const finish = useReels((s) => s.finish);
  const [menu, setMenu] = useState<{ at: { x: number; y: number; align: "end" }; keyboard: boolean; back: HTMLElement } | null>(null);
  const on = FINISH_STEPS.filter(([key]) => finish[key]).length;
  return (
    <>
      <Button
        pill
        className="h-7 px-3 text-[12px]"
        aria-haspopup="menu"
        aria-expanded={!!menu}
        title="What each reel gets once made"
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setMenu({ at: { x: r.right, y: r.bottom + 6, align: "end" }, keyboard: e.detail === 0, back: e.currentTarget });
        }}
      >
        Each reel gets <span className="tabular text-muted">{on === FINISH_STEPS.length ? "all" : `${on} of ${FINISH_STEPS.length}`}</span>
        <ChevronDown size={14} className="text-muted" />
      </Button>
      {menu && (
        <Menu
          label="Each reel gets"
          minWidth={260}
          at={menu.at}
          keyboard={menu.keyboard}
          items={FINISH_STEPS.map(([key, label]) => ({ label, checked: finish[key], run: () => setFinish({ ...finish, [key]: !finish[key] }) }))}
          onClose={(chose) => {
            const back = menu.back;
            setMenu(null);
            if (!chose) back.focus();
          }}
        />
      )}
    </>
  );
}

/** Make N reels, or the job making them with its progress and Stop. */
function MakeButton({ picked, blocker }: { picked: number; blocker: string | null }) {
  const id = useReels((s) => s.job);
  const job = useEditor((s) => (id ? s.jobs[id] : undefined));
  if (job?.status === "running")
    return (
      <div className="flex w-[260px] items-center gap-2" role="status">
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="tabular truncate text-[12px] text-fg">
            {job.phase ?? "Making reels"}
            {job.progress > 0 && ` · ${Math.round(job.progress * 100)}%`}
          </span>
          <ProgressBar value={job.progress} label="Making reels" />
        </div>
        <Button pill className="h-6 shrink-0 px-2.5 text-[12px]" title="Stop. Reels made so far are removed." onClick={() => void api.cancelJob(job.id)}>
          Stop
        </Button>
      </div>
    );
  return (
    <Button variant="primary" pill disabled={picked === 0 || !!blocker} disabledReason={blocker ?? "Check the reels to make"} onClick={() => void makeReels()}>
      {picked > 0 ? `Make ${plural(picked, "reel")}` : "Make reels"}
    </Button>
  );
}

/** Reels from a long video, in the timeline's place: the moments an agent found, to check, reject and make. */
export function ReelList({ height }: { height: number }) {
  const project = useEditor((s) => s.snap?.project);
  const path = useEditor((s) => s.snap?.path);
  const current = useReels((s) => s.current);
  const framing = useReels((s) => s.framing);
  const finding = useReels((s) => s.finding);
  const failed = useReels((s) => s.failed);
  const blocked = useAgent(() => findBlocked());
  const missing = useLibrary((s) => s.projects?.find((p) => p.path === path)?.missing ?? 0);
  const list = candidates(project);
  const { view, error } = useTranscriptView();
  // Read once here for the cards and the trim pane.
  useEffect(() => useReels.setState({ transcript: { words: view?.words ?? null, error } }), [view, error]);
  const picked = list.filter((c) => c.status === "picked" && !moved(c, view?.words ?? null)).length;
  const missingText = missing > 0 ? `${plural(missing, "media file")} ${missing === 1 ? "is" : "are"} missing. Put ${missing === 1 ? "it" : "them"} back where ${missing === 1 ? "it was" : "they were"} to play and make reels.` : null;

  return (
    <section className="pane flex flex-col overflow-hidden" style={{ height }} aria-label="Reels from a long video">
      <AiLock>
        <div className="flex h-11 shrink-0 items-center gap-2 px-3">
          <h2 className="text-[13px] font-semibold text-fg">Reels</h2>
          {list.length > 0 && (
            <span className="tabular text-[13px] text-muted">
              {list.length}
              {picked > 0 && ` · ${picked} checked`}
            </span>
          )}
          {list.length > 0 &&
            (finding ? (
              <span className="flex items-center gap-1.5 text-[12px] text-muted" role="status">
                <Loader2 size={14} className="animate-spin text-accent" /> Finding reels…
              </span>
            ) : (
              <Button variant="ghost" className="h-7 px-2 text-[12px] text-accent! hover:text-accent!" disabled={!!blocked} disabledReason={blocked ?? undefined} title="Ask your AI again. Reels not made yet are replaced." onClick={findReels}>
                <Sparkles size={14} /> Find again
              </Button>
            ))}
          <span className="flex-1" />
          {list.length > 0 && (
            <>
              <FinishMenu />
              <span className="text-[12px] text-muted">9:16</span>
              <div className="w-[248px]">
                <Segmented
                  label="How the reels fill 9:16"
                  value={framing}
                  onChange={setFraming}
                  options={[
                    { id: "crop", label: "Center crop", title: "The middle of the picture fills the frame; its sides are cut off" },
                    { id: "blur", label: "Blurred background", title: "The whole picture, over a blurred copy of it" },
                  ]}
                />
              </div>
              <MakeButton picked={picked} blocker={missingText && "Put the missing media back first"} />
            </>
          )}
        </div>
        {missingText && (
          <p className="mx-3 mb-2 flex items-start gap-1.5 text-[12px] text-fg" role="status">
            <AlertTriangle size={14} className="mt-px shrink-0 text-warn" />
            {missingText}
          </p>
        )}
        {failed && (
          <p className="mx-3 mb-2 flex items-start gap-1.5 text-[12px] text-danger" role="alert">
            <AlertCircle size={14} className="mt-px shrink-0" />
            {failed}
          </p>
        )}
        {list.length > 0 ? <Cards list={list} current={current} /> : <NoReels />}
      </AiLock>
    </section>
  );
}

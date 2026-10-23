import { Fragment, createContext, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import {
  AlertCircle,
  AlertTriangle,
  ArrowLeft,
  ArrowUp,
  Captions,
  Check,
  CheckCircle2,
  ChevronDown,
  Camera,
  ChevronRight,
  CircleStop,
  Clapperboard,
  Copy,
  Crosshair,
  Download,
  ExternalLink,
  Film,
  GripVertical,
  Image as ImageIcon,
  Info,
  Loader2,
  Plug,
  MessagesSquare,
  MessageSquareText,
  Plus,
  RefreshCw,
  ScanFace,
  Scissors,
  Sparkles,
  SquarePen,
  Undo2,
  X,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { open as openFiles } from "@tauri-apps/plugin-dialog";
import {
  agentName,
  attach,
  detach,
  choose,
  chooseAgent,
  jobText,
  markUndone,
  loadAgents,
  loadChats,
  makeThumbnail,
  newChat,
  openChat,
  SEARCHING,
  send,
  sendBlocked,
  steer,
  steerBlocked,
  stop,
  useAgent,
  type AgentErrorCode,
  type AiAbout,
  type Attachment,
  type ChatItem,
  type CliAgentId,
  type RunChange,
  type Step,
} from "../../lib/agent";
import { demoBlocked, sendDemo, stopDemo } from "../../lib/demo";
import { DOCK_LABELS, TOO_NARROW, useDock, useDockLayout, type DockMode } from "../../lib/dock";
import { api } from "../../lib/api";
import { copyText } from "../../lib/clipboard";
import { FINISH_DRAFT } from "../../lib/capcut";
import { downloadModels, useCover } from "../../lib/cover";
import { openRecap, undoAction, undoStep, useEditor } from "../../lib/store";
import { formatTime } from "../../lib/time";
import { whenLabel } from "../../lib/library";
import { Button, IconButton, LockReason, ProgressBar, Segmented } from "../ui";
import { MODEL_QUESTION, answerModelQuestion } from "../ModelQuestionDialog";
import { Markdown } from "./Markdown";
import { IMAGE_EXTENSIONS, VIDEO_EXTENSIONS } from "../panel/assets";
import { ChatGptLogo } from "../ChatGptLogo";
import { ContinueButton, USAGE_PAGE } from "../ChatGptDialog";
import { continueWithChatGpt, openChatGpt, planReady, useChatGpt } from "../../lib/chatgpt";
import { EFFORTS, chooseEffort, chooseModel, effortLabel, loadModels, resolved, useModels } from "../../lib/models";
import { moveFloatBy, startMove } from "./AiDock";
import { GeminiGuide } from "./GeminiGuide";

const INSTALL: Record<CliAgentId, string> = {
  claude: "https://docs.claude.com/en/docs/claude-code/setup",
  codex: "https://developers.openai.com/codex/cli",
};
const COMMAND: Record<CliAgentId, string> = { claude: "claude", codex: "codex" };

/** A small menu under a trigger (`up`: above it): ↑/↓ move, Esc closes back to the trigger, no key reaches the editor. */
function Menu({
  label,
  up = false,
  width = "w-72",
  place = "left-0",
  shrink = false,
  trigger,
  children,
}: {
  label: string;
  up?: boolean;
  width?: string;
  /** Where the menu starts against its button, e.g. at the field's edge instead. */
  place?: string;
  /** The button may get narrower than its label, which it truncates. */
  shrink?: boolean;
  trigger: (p: { open: boolean; toggle: () => void; ref: React.RefObject<HTMLButtonElement | null> }) => ReactNode;
  children: (close: () => void) => ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const close = () => {
    setOpen(false);
    triggerRef.current?.focus();
  };
  useEffect(() => {
    if (!open) return;
    const items = [...(ref.current?.querySelectorAll<HTMLElement>("[role^=menuitem]") ?? [])];
    (items.find((el) => el.getAttribute("aria-checked") === "true") ?? items[0])?.focus();
    const outside = (e: PointerEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: globalThis.KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      close();
    };
    window.addEventListener("pointerdown", outside);
    window.addEventListener("keydown", esc, true);
    return () => {
      window.removeEventListener("pointerdown", outside);
      window.removeEventListener("keydown", esc, true);
    };
  }, [open]);
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    e.stopPropagation();
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = [...e.currentTarget.querySelectorAll<HTMLElement>("[role^=menuitem]")];
    const i = items.indexOf(document.activeElement as HTMLElement);
    items[(i + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length]?.focus();
  };
  return (
    <div
      className={shrink ? "relative min-w-0" : "relative"}
      ref={ref}
      onBlur={(e) => e.relatedTarget && !e.currentTarget.contains(e.relatedTarget) && setOpen(false)}
    >
      {trigger({ open, toggle: () => setOpen(!open), ref: triggerRef })}
      {open && (
        <div role="menu" aria-label={label} onKeyDown={onKey} className={`overlay absolute ${place} z-50 rounded-xl p-1 ${width} ${up ? "bottom-9" : "top-9"}`}>
          {children(close)}
        </div>
      )}
    </div>
  );
}

function MenuItem({ checked, disabled, reason, onClick, children }: { checked?: boolean; disabled?: boolean; reason?: string; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      role={checked === undefined ? "menuitem" : "menuitemradio"}
      aria-checked={checked}
      aria-disabled={disabled || undefined}
      title={disabled ? reason : undefined}
      onClick={disabled ? undefined : onClick}
      className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] text-fg hover:bg-white/[.08] aria-disabled:cursor-not-allowed aria-disabled:text-muted aria-disabled:hover:bg-transparent"
    >
      <span className="flex w-4 shrink-0 justify-center">{checked && <Check size={14} className="text-accent" />}</span>
      {children}
    </button>
  );
}

/** The grip: drag it (or the header) to move the panel, click it to pick a place; arrows move a floating panel. */
function PlaceMenu({ panel }: { panel: React.RefObject<HTMLElement | null> }) {
  const { mode, sideFits } = useDockLayout();
  return (
    <Menu
      label="Panel place"
      trigger={({ open, toggle, ref }) => (
        <button
          ref={ref}
          type="button"
          aria-label="Move panel"
          title={mode === "float" ? "Move panel: drag it, use the arrow keys, or click to dock it" : "Move panel: drag it, or click to pick a place"}
          aria-haspopup="menu"
          aria-expanded={open}
          onPointerDown={(e) => panel.current && startMove(e, panel.current, toggle)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              toggle();
              return;
            }
            if (mode !== "float" || !e.key.startsWith("Arrow")) return;
            e.preventDefault();
            e.stopPropagation();
            const step = e.shiftKey ? 96 : 24;
            moveFloatBy(e.key === "ArrowLeft" ? -step : e.key === "ArrowRight" ? step : 0, e.key === "ArrowUp" ? -step : e.key === "ArrowDown" ? step : 0);
          }}
          className={`flex h-7 w-6 shrink-0 cursor-grab items-center justify-center rounded-md text-muted transition-colors duration-[120ms] hover:bg-white/[.08] hover:text-fg active:cursor-grabbing ${open ? "bg-white/[.08] text-fg" : ""}`}
        >
          <GripVertical size={15} />
        </button>
      )}
    >
      {(close) =>
        (["right", "left", "inspector", "float"] as DockMode[]).map((m) => (
          <MenuItem
            key={m}
            checked={mode === m}
            disabled={(m === "left" || m === "right") && !sideFits}
            reason={TOO_NARROW}
            onClick={() => {
              useDock.setState({ mode: m, open: true });
              close();
            }}
          >
            {DOCK_LABELS[m]}
          </MenuItem>
        ))
      }
    </Menu>
  );
}

function AgentMenu() {
  const agents = useAgent((s) => s.agents);
  const agent = useAgent((s) => s.agent);
  const busy = useAgent((s) => s.status !== "idle");
  const chatgpt = useChatGpt((s) => s.status);
  const gemini = useAgent((s) => s.gemini);
  const windows = navigator.userAgent.includes("Windows");
  const name = agentName(agent, agents);
  return (
    <Menu
      label="Agent"
      trigger={({ open, toggle, ref }) => (
        <button
          ref={ref}
          type="button"
          aria-haspopup="menu"
          aria-expanded={open}
          title="Choose the agent"
          onClick={toggle}
          className={`flex h-7 min-w-0 items-center gap-1 rounded-md px-1.5 text-[13px] font-semibold text-fg hover:bg-white/[.08] ${open ? "bg-white/[.08]" : ""}`}
        >
          <span className="truncate">{name}</span>
          <ChevronDown size={14} className="shrink-0 text-muted" />
        </button>
      )}
    >
      {(close) => (
        <>
          {(agents ?? []).map((a) => (
            <MenuItem
              key={a.id}
              checked={a.id === agent}
              disabled={!a.path || busy}
              reason={!a.path ? `${a.name} is not installed` : "Wait for the answer, or stop it"}
              onClick={() => {
                chooseAgent(a.id);
                close();
              }}
            >
              <span className="flex-1">{a.name}</span>
              <span className="text-[12px] text-muted">{a.path ? (a.version ?? "Installed") : "Not installed"}</span>
            </MenuItem>
          ))}
          <MenuItem
            checked={agent === "chatgpt"}
            disabled={busy}
            reason="Wait for the answer, or stop it"
            onClick={() => {
              chooseAgent("chatgpt");
              close();
              // Signing in is how ChatGPT gets ready, so it is offered at once.
              if (!planReady(chatgpt)) openChatGpt();
            }}
          >
            <span className="flex-1">ChatGPT</span>
            <span className="text-[12px] text-muted">{planReady(chatgpt) ? "Signed in" : chatgpt?.state === "signedIn" ? "Plan not allowed" : chatgpt ? "Not signed in" : ""}</span>
          </MenuItem>
          <MenuItem
            checked={agent === "antigravity"}
            disabled={windows || busy}
            reason={windows ? "AI editing isn't available on Windows yet" : "Wait for the answer, or stop it"}
            onClick={() => {
              chooseAgent("antigravity");
              close();
            }}
          >
            <span className="flex-1">Gemini</span>
            <span className="text-[12px] text-muted">{windows ? "Not on Windows" : !gemini ? "" : !gemini.installed ? "Not installed" : gemini.connection.state === "connected" ? "Connected" : "Not connected"}</span>
          </MenuItem>
          <div className="mx-1.5 my-1 h-px bg-white/[.08]" />
          <MenuItem
            onClick={() => {
              close();
              openChatGpt();
            }}
          >
            <ChatGptLogo size={14} className="text-muted" />
            ChatGPT account…
          </MenuItem>
          <MenuItem
            onClick={() => {
              close();
              useEditor.setState({ connectOpen: true });
            }}
          >
            <Plug size={14} className="text-muted" />
            Connect an agent outside Nuzky…
          </MenuItem>
        </>
      )}
    </Menu>
  );
}

const STEP_ICON: Record<Step["status"], ReactNode> = {
  running: <Loader2 size={14} className="animate-spin text-accent" />,
  done: <Check size={14} className="text-ok" />,
  failed: <AlertCircle size={14} className="text-danger" />,
  stopped: <CircleStop size={14} className="text-muted" />,
  interrupted: <CircleStop size={14} className="text-muted" />,
};

function grouped(steps: Step[]): (Step & { times: number })[] {
  const rows: (Step & { times: number })[] = [];
  for (const s of steps) {
    const last = rows[rows.length - 1];
    // Every search shows what it searched for, so only a repeat of the same search joins its row.
    const searches = s.title === SEARCHING && last?.detail !== s.detail;
    if (last && last.title === s.title && last.status !== "failed" && s.status !== "failed" && !searches) {
      // Calls can run side by side, so the row runs until every one of them is done.
      const status = [last.status, s.status].includes("running") ? "running" : [last.status, s.status].includes("stopped") ? "stopped" : [last.status, s.status].includes("interrupted") ? "interrupted" : s.status;
      rows[rows.length - 1] = { ...s, id: last.id, status, times: last.times + 1 };
    } else rows.push({ ...s, times: 1 });
  }
  return rows;
}

function Steps({ steps, collapsed }: { steps: Step[]; collapsed: boolean }) {
  const [open, setOpen] = useState(!collapsed);
  useEffect(() => setOpen(!collapsed), [collapsed]);
  const rows = grouped(steps);
  if (!open)
    return (
      <button type="button" onClick={() => setOpen(true)} className="-ml-1 flex items-center gap-1 self-start rounded-md px-1 py-0.5 text-[12px] text-muted hover:bg-white/[.06] hover:text-fg">
        <ChevronRight size={14} />
        {rows.length === 1 ? "1 step" : `${rows.length} steps`}
      </button>
    );
  return (
    <ol className="flex flex-col border-y border-white/[.06] py-1.5" aria-label="Steps">
      {rows.map((s) => (
        <li key={s.id} className="grid min-h-[26px] grid-cols-[18px_auto_minmax(0,1fr)] items-center gap-x-1.5 text-[12px]">
          {STEP_ICON[s.status]}
          <span className={s.status === "running" ? "text-fg" : "text-fg/80"}>{s.title}</span>
          <span className="tabular truncate text-right text-muted" title={s.detail ?? undefined}>
            {s.status === "stopped" ? "Stopped" : s.status === "interrupted" ? "Interrupted" : s.times > 1 ? `${s.times} times` : s.detail}
          </span>
        </li>
      ))}
    </ol>
  );
}

function Options({ item, past }: { item: Extract<ChatItem, { kind: "options" }>; past: boolean }) {
  const busy = useAgent((s) => s.status !== "idle");
  return (
    <div className="flex flex-col gap-1.5" role="group" aria-label={item.question ?? "Options"}>
      {item.question && <p className="leading-[20px] text-fg/90">{item.question}</p>}
      {item.options.map((o) => {
        const chosen = item.chosen === o.label;
        const off = item.chosen !== null || busy || past;
        return (
          <button
            key={o.label}
            type="button"
            aria-disabled={off || undefined}
            aria-pressed={chosen}
            title={off && !chosen ? (item.chosen ? "Already answered" : past ? "From an earlier visit. Write your answer below." : "Wait for the answer, or stop it") : undefined}
            onClick={off ? undefined : () => choose(item.id, o.label)}
            className={`flex flex-col items-start gap-0.5 rounded-[10px] px-3 py-2 text-left transition-colors duration-[120ms] ${
              chosen ? "bg-accent/[.18] shadow-[inset_0_0_0_1px_rgb(41_151_255/.6)]" : off ? "cursor-not-allowed bg-raised opacity-50" : "bg-raised hover:bg-[#35353a]"
            }`}
          >
            <span className="flex items-center gap-1.5 text-[13px] font-medium text-fg">
              {chosen && <Check size={13} className="text-accent" />}
              {o.label}
            </span>
            {o.detail && <span className="text-[12px] leading-[17px] text-muted">{o.detail}</span>}
          </button>
        );
      })}
    </div>
  );
}

function ChangeRow({ change, undone, past }: { change: RunChange; undone: boolean; past: boolean }) {
  // Recomputed when the project changes, never with the playhead while playing.
  const project = useEditor((s) => s.snap?.project);
  const ids = useMemo(() => {
    const all = new Set(project?.tracks.flatMap((t) => t.clips.map((c) => c.id)));
    return change.clipIds.filter((id) => all.has(id));
  }, [project, change]);
  // The clips and times of an earlier visit may be gone since.
  const go = !undone && !past && (ids.length > 0 || change.atUs !== null);
  const show = () => {
    const s = useEditor.getState();
    if (ids.length) s.select(ids);
    if (change.atUs !== null) s.seek(change.atUs);
  };
  return (
    <li>
      <button
        type="button"
        disabled={!go}
        onClick={show}
        title={go ? "Show it on the timeline" : undefined}
        className={`flex w-full items-start gap-2 rounded-md py-1 pl-[30px] pr-1.5 text-left text-[12px] leading-[17px] enabled:hover:bg-white/[.06] disabled:cursor-default ${undone ? "text-fg/45" : "text-fg/90"}`}
      >
        <span className="flex-1">{change.text}</span>
        {change.atUs !== null && <span className="tabular shrink-0 text-muted">{formatTime(change.atUs, false)}</span>}
      </button>
    </li>
  );
}

/** `past`: from an earlier visit, which this session's Undo cannot take back; Versions can. */
function RunCard({ item, index, past }: { item: Extract<ChatItem, { kind: "run" }>; index: number; past: boolean }) {
  // Undo is checked each render, so it turns off once later changes are on top of the run.
  useEditor((s) => `${s.snap?.sessionEpoch}:${s.snap?.revision}`);
  const locked = useEditor((s) => !!s.aiRun);
  const undo = item.snap && !item.undone ? undoAction(item.snap) : null;
  const valid = !!undo && (undo.valid?.() ?? true) && !locked;
  const label = item.undone ? `Undone: ${item.label}` : item.stopped ? `Stopped: ${item.label}` : item.label;
  return (
    <div data-run-card className="flex flex-col gap-1.5 rounded-[10px] bg-raised p-3">
      <div className="flex items-center gap-2">
        {item.undone ? (
          <Undo2 size={16} className="shrink-0 text-muted" />
        ) : item.stopped ? (
          <CircleStop size={16} className="shrink-0 text-muted" />
        ) : (
          <CheckCircle2 size={16} className="shrink-0 text-ok" />
        )}
        <span className="min-w-0 flex-1 truncate text-[13px] font-semibold text-fg" title={label}>
          {label}
        </span>
        {undo && (
          <Button
            className="h-7 shrink-0 px-2.5 text-[12px]"
            disabled={!valid}
            disabledReason={locked ? "AI is editing. Wait for it, or stop it." : "Later changes are on top of this run. Undo them first."}
            title="Undo everything this run did"
            onClick={() => void undoStep(item.snap!).then((done) => done && markUndone(index))}
          >
            <Undo2 size={13} /> Undo
          </Button>
        )}
      </div>
      {item.changes && item.changes.length > 0 ? (
        <ul className="-mx-1.5 flex flex-col" aria-label="What changed">
          {item.changes.map((c, i) => (
            <ChangeRow key={i} change={c} undone={!!item.undone} past={past} />
          ))}
        </ul>
      ) : (
        !item.snap && !past && <p className="text-[12px] text-muted">Nothing in the project changed.</p>
      )}
      {item.runId && item.cut && !item.undone && (
        <Button
          className="ml-6 h-7 self-start px-2.5 text-[12px]"
          title="A short vertical video of how this run cut the video"
          onClick={() => openRecap(item.runId)}
        >
          <Clapperboard size={13} /> Export recap…
        </Button>
      )}
    </div>
  );
}

/** `last`: the conversation ends with it, so its way on takes focus when it has only one. */
function ErrorNotice({ code, message, retryText, retryAttachments, last = false }: { code: AgentErrorCode; message: string; retryText?: string; retryAttachments?: Attachment[]; last?: boolean }) {
  const actions = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (last && code === "SESSION_GONE") actions.current?.querySelector("button")?.focus();
  }, [last, code]);
  const agents = useAgent((s) => s.agents);
  const agent = useAgent((s) => s.agent);
  const chatgptReady = useChatGpt((s) => planReady(s.status));
  const name = agentName(agent, agents);
  const other = agents?.find((a) => a.id !== agent && a.path)?.id ?? (agent !== "chatgpt" && chatgptReady ? "chatgpt" : undefined);
  const retry = (
    <Button className="h-7 px-2.5 text-[12px]" onClick={() => void (retryText !== undefined ? send(retryText, undefined, retryAttachments ?? []) : send())}>
      Try again
    </Button>
  );
  const switchTo = other && (
    <Button variant="ghost" className="h-7 px-2.5 text-[12px]" title="Starts a new conversation; your message stays" onClick={() => chooseAgent(other)}>
      Ask {agentName(other, agents)} instead
    </Button>
  );
  const button = (label: ReactNode, onClick: () => void) => (
    <Button variant="primary" className="h-7 px-2.5 text-[12px]" onClick={onClick}>
      {label}
    </Button>
  );
  const term = (text: string) => <code className="rounded bg-white/[.08] px-1">{text}</code>;
  const cmd = agent === "chatgpt" || agent === "antigravity" ? null : term(COMMAND[agent]);
  const signIn = agent === "codex" ? <>run {term("codex login")} and choose Sign in with ChatGPT</> : <>run {cmd} and sign in</>;
  // A message that failed after the agent began answering stays in the conversation, with Try again, not in the field.
  const kept = retryText !== undefined ? message : <>{message} Your message is still here.</>;
  const [icon, title, body, buttons]: [ReactNode, string, ReactNode, ReactNode] =
    code === "SESSION_GONE"
      ? [
          <AlertTriangle size={16} className="text-warn" />,
          `${name} no longer has this chat`,
          "It clears old chats after a while, so it can't pick this one up. Start a new chat with your message; this one stays here to read.",
          button("New chat with this message", () => {
            newChat();
            document.querySelector<HTMLTextAreaElement>('aside[aria-label="AI"] textarea')?.focus();
          }),
        ]
      : agent === "chatgpt"
      ? code === "NOT_SIGNED_IN"
        ? [<AlertCircle size={16} className="text-danger" />, "ChatGPT isn't signed in.", kept, <>{button("Sign in…", continueWithChatGpt)}{retry}{switchTo}</>]
        : code === "USAGE_LIMIT"
          ? [
              // OpenAI asks to keep ChatGPT's identity on its usage-limit message.
              <ChatGptLogo size={16} className="text-warn" />,
              "ChatGPT can't run on your plan right now.",
              kept,
              <>{button(<>Manage usage <ExternalLink size={12} /></>, () => void openUrl(USAGE_PAGE))}{retry}{switchTo}</>,
            ]
          : code === "PLAN_UNAVAILABLE"
            ? [<AlertTriangle size={16} className="text-warn" />, "Nuzky can't use your ChatGPT plan.", kept, <>{button("ChatGPT account…", openChatGpt)}{switchTo}</>]
            : [<AlertCircle size={16} className="text-danger" />, "ChatGPT stopped with an error.", <Failure message={message} />, retry]
      : code === "NOT_SIGNED_IN"
      ? [<AlertCircle size={16} className="text-danger" />, `${name} isn't signed in.`, <>Open a terminal, {signIn}. Your message is still here.</>, <>{retry}{switchTo}</>]
      : code === "USAGE_LIMIT"
        ? [<AlertTriangle size={16} className="text-warn" />, `${name} usage limit reached.`, <>{message} The limit comes from your plan, not from Nuzky. Your message is still here.</>, switchTo]
        : code === "NOT_INSTALLED"
          ? [
              <AlertCircle size={16} className="text-danger" />,
              `${name} isn't installed.`,
              <>Install it and sign in from a terminal. Your message is still here.</>,
              <Button className="h-7 px-2.5 text-[12px]" onClick={() => void openUrl(INSTALL[agent as CliAgentId])}>
                Install <ExternalLink size={12} />
              </Button>,
            ]
          : [<AlertCircle size={16} className="text-danger" />, `${name} stopped with an error.`, <Failure message={message} cmd={cmd} />, retry];
  return (
    <div role="alert" className="grid grid-cols-[20px_1fr] gap-2 rounded-[10px] bg-raised p-3 text-[12px] leading-[18px] text-fg/85">
      <span className="pt-px">{icon}</span>
      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="text-[13px] font-semibold text-fg">{title}</span>
        <span className="flex flex-col items-start break-words">
          <span>{body}</span>
        </span>
        {buttons && (
          <div ref={actions} className="mt-2 flex flex-wrap gap-1.5">
            {buttons}
          </div>
        )}
      </div>
    </div>
  );
}

/** The first line of what went wrong and what to do; the agent's own lines behind Details. `cmd`: the agent to run in a terminal to see why. */
function Failure({ message, cmd }: { message: string; cmd?: ReactNode }) {
  const [open, setOpen] = useState(false);
  const [first, ...rest] = message.trim().split("\n");
  return (
    <>
      <span className="line-clamp-2">{first}</span> {cmd ? <>Try again; if it keeps failing, run {cmd} in a terminal to see why.</> : "Try again."}
      {rest.length > 0 &&
        (open ? (
          <pre className="mt-1.5 max-h-40 overflow-auto whitespace-pre-wrap rounded-md bg-white/[.06] px-2 py-1.5 text-[11px] leading-[16px] text-muted">{rest.join("\n")}</pre>
        ) : (
          <button type="button" onClick={() => setOpen(true)} className="mt-1 self-start text-[12px] text-accent hover:underline">
            Details
          </button>
        ))}
    </>
  );
}

const STARTERS: [ReactNode, string][] = [
  [<Scissors size={14} />, "Cut retakes and long pauses"],
  [<Captions size={14} />, "Add captions"],
  [<Download size={14} />, "Make a reel and export it"],
  // Puts its chip in the field rather than its text: the agent then gets the MCP prompt `thumbnail`.
  [<ImageIcon size={14} />, "Make a thumbnail"],
];
/** First for a project opened from a CapCut draft; it puts the whole request in the field. */
const DRAFT_STARTER: [ReactNode, string] = [<Sparkles size={14} />, "Finish this draft in my style"];
const STYLE_STARTERS: [ReactNode, string][] = [
  [<MessageSquareText size={14} />, "What do you cut in my videos, and why?"],
  [<MessageSquareText size={14} />, "Never cut my sign-off at the end"],
  [<MessageSquareText size={14} />, "Leave my pauses a bit longer"],
];

/** What the panel is about; on the Your style page, the creator's style. */
const About = createContext<AiAbout>("project");

/** ChatGPT chosen but not ready: signing in, or allowing the plan, is the way on. Without Claude Code or Codex, how to get them or Gemini too. */
function ChatGptSignIn() {
  const agents = useAgent((s) => s.agents) ?? [];
  const gemini = useAgent((s) => s.gemini);
  const signedIn = useChatGpt((s) => s.status?.state === "signedIn");
  return (
    <div className="my-auto flex flex-col items-start px-1">
      <h3 className="text-[15px] font-semibold text-fg">{signedIn ? "Allow Nuzky to use your ChatGPT plan" : "Use your ChatGPT plan"}</h3>
      <p className="mb-3 mt-1.5 text-[12px] leading-[18px] text-muted">
        {signedIn
          ? "Continue with ChatGPT and allow it when the browser asks. If a work or school workspace blocks it, sign out in ChatGPT account… and use a personal account."
          : "Sign in with ChatGPT, and ChatGPT edits your video right here. What it uses counts toward your ChatGPT plan's usage limits."}
      </p>
      <ContinueButton onClick={continueWithChatGpt} />
      {!agents.some((a) => a.path) && (
        <div className="mt-6 flex w-full flex-col">
          <p className="mb-1.5 text-[12px] leading-[18px] text-muted">Or use Claude Code or Codex: install one, sign in from your terminal, then check again.</p>
          {agents.map((a) => (
            <div key={a.id} className="flex h-9 items-center justify-between border-t border-white/[.07] text-[13px]">
              {a.name}
              <Button variant="ghost" className="h-7 px-2 text-[12px] text-accent" onClick={() => void openUrl(INSTALL[a.id])}>
                Install <ExternalLink size={12} />
              </Button>
            </div>
          ))}
          <div className="flex h-9 items-center justify-between border-t border-white/[.07] text-[13px]">
            Gemini
            <Button variant="ghost" className="h-7 px-2 text-[12px] text-accent" onClick={() => chooseAgent("antigravity")}>
              {gemini?.connection.state === "connected" ? "Show" : "Set up"} <ChevronRight size={12} />
            </Button>
          </div>
          <Button className="mt-3 h-7 self-start px-2.5 text-[12px]" onClick={() => void loadAgents()}>
            <RefreshCw size={13} /> Check again
          </Button>
        </div>
      )}
    </div>
  );
}

function Empty() {
  const about = useContext(About);
  const agents = useAgent((s) => s.agents);
  const agent = useAgent((s) => s.agent);
  const chatgptReady = useChatGpt((s) => planReady(s.status));
  const error = useAgent((s) => s.agentsError);
  const blocked = useEditor((s) => !!s.aiRun);
  const draft = useEditor((s) => !!s.snap?.project.imported) && about === "project";
  // The tour's request is already in the field, and no agent runs, so nothing here asks for one.
  const demo = useAgent((s) => s.demo) && about === "project";
  if (demo) return <p className="mt-auto mb-2 px-2 text-[12px] text-muted">Edits show up live. One Undo takes back a whole run.</p>;
  if (error) return <div className="mt-auto"><ErrorNotice code="AGENT_FAILED" message={error} /></div>;
  if (agents === null) return <p className="mt-auto px-1 text-[12px] text-muted">Looking for your AI…</p>;
  if (agent === "chatgpt" && !chatgptReady) return <ChatGptSignIn />;
  return (
    <div className="mt-auto flex flex-col gap-0.5">
      <p className="mb-2 px-2 text-[12px] text-muted">
        {about === "style" ? "Tell the AI how you edit. Once you agree, your style changes here at once, with Undo." : "Edits show up live. One Undo takes back a whole run."}
      </p>
      {(about === "style" ? STYLE_STARTERS : draft ? [DRAFT_STARTER, ...STARTERS] : STARTERS).map(([icon, text]) => (
        <button
          key={text}
          type="button"
          aria-disabled={blocked || undefined}
          title={blocked ? "Another agent is editing" : "Put this in the field, to send or change"}
          onClick={
            blocked
              ? undefined
              : (e) => {
                  if (text === "Make a thumbnail") makeThumbnail();
                  else useAgent.setState({ draft: text === DRAFT_STARTER[1] ? FINISH_DRAFT : text });
                  // The field of this panel: the editor's stays mounted behind the home screen.
                  e.currentTarget.closest("aside")?.querySelector("textarea")?.focus();
                }
          }
          className="flex h-8 items-center gap-2.5 rounded-md px-2 text-left text-[13px] text-fg hover:bg-white/[.08] aria-disabled:cursor-not-allowed aria-disabled:opacity-40 aria-disabled:hover:bg-transparent [&>svg]:text-muted"
        >
          {icon}
          {text}
        </button>
      ))}
    </div>
  );
}

/** What went with a message, as small pictures; a video is marked as one. */
function Attached({ attachments }: { attachments: Attachment[] }) {
  return (
    <div className="flex flex-wrap justify-end gap-1.5">
      {attachments.map((a) => (
        <div key={a.path} title={a.name} aria-label={`${a.kind === "video" ? "Video" : "Picture"} ${a.name}`} role="img" className="relative h-16 overflow-hidden rounded-[10px] bg-raised">
          {a.preview ? (
            <img src={a.preview} alt="" className="h-full min-w-16 max-w-[120px] object-cover" />
          ) : (
            <span className="flex h-full w-16 items-center justify-center text-muted">{a.kind === "video" ? <Film size={16} /> : <ImageIcon size={16} />}</span>
          )}
          {a.kind === "video" && a.preview && (
            <span className="absolute bottom-1 left-1 flex h-4 w-4 items-center justify-center rounded bg-black/60 text-white">
              <Film size={10} />
            </span>
          )}
        </div>
      ))}
    </div>
  );
}

/** Hidden unless an ancestor with the `group` class is hovered. */
function CopyButton({ text, label, className = "" }: { text: string; label: string; className?: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const id = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(id);
  }, [copied]);
  const copy = () =>
    copyText(text, "The clipboard is not available here.").then(
      () => setCopied(true),
      (e: Error) => useEditor.getState().toast({ kind: "error", text: e.message }),
    );
  return (
    <IconButton
      label={copied ? "Copied" : label}
      onClick={() => void copy()}
      className={`h-6 w-6 shrink-0 focus-visible:opacity-100 group-hover:opacity-100 ${copied ? "opacity-100" : "opacity-0"} ${className}`}
    >
      {copied ? <Check size={13} className="text-ok" /> : <Copy size={13} />}
    </IconButton>
  );
}

function UserMessage({ text, context, attachments }: { text: string; context: string | null; attachments?: Attachment[] }) {
  return (
    <div className="group flex max-w-[88%] flex-col items-end gap-1 self-end">
      {attachments?.length ? <Attached attachments={attachments} /> : null}
      {text && (
        <div className="flex max-w-full items-end gap-1">
          <CopyButton text={text} label="Copy message" />
          <div className="min-w-0 whitespace-pre-wrap break-words rounded-[14px] rounded-br-[4px] bg-raised px-3 py-2 leading-[19px] text-fg">{text}</div>
        </div>
      )}
      {context && <div className="tabular text-[11px] text-muted">{context}</div>}
    </div>
  );
}

function Item({ item, index, collapsed, past, last }: { item: ChatItem; index: number; collapsed: boolean; past: boolean; last: boolean }) {
  switch (item.kind) {
    case "user":
      return <UserMessage text={item.text} context={item.context} attachments={item.attachments} />;
    case "agent":
      return (
        <div className="group flex flex-col items-start gap-0.5">
          <div className="max-w-full break-words leading-[20px] text-fg/90">
            <Markdown text={item.text} />
          </div>
          <CopyButton text={item.text} label="Copy reply" className="-ml-1" />
        </div>
      );
    case "steps":
      return <Steps steps={item.steps} collapsed={collapsed} />;
    case "options":
      return <Options item={item} past={past} />;
    case "run":
      return <RunCard item={item} index={index} past={past} />;
    case "stopped":
      return (
        <div className="flex items-center gap-2 rounded-[10px] bg-raised p-3 text-[12px]">
          <CircleStop size={16} className="text-muted" />
          <span className="text-[13px] font-semibold text-fg">Stopped</span>
          <span className="text-muted">Nothing changed</span>
        </div>
      );
    case "error":
      return <ErrorNotice code={item.code} message={item.message} retryText={item.retry} retryAttachments={item.retryAttachments} last={last} />;
  }
}

/** An agent's job waits for the face and person models: Download here, and the agent goes on once they are in. */
function VisionModelsQuestion() {
  const question = useEditor((s) => s.visionModelQuestion);
  const job = useEditor((s) => (s.visionModelJob ? s.jobs[s.visionModelJob] : undefined));
  const running = job?.status === "running";
  const card = useRef<HTMLDivElement>(null);
  // Download and Not now go away when pressed; the focus goes to Stop, or back to the field.
  useEffect(() => {
    if (document.activeElement !== document.body) return;
    if (running) card.current?.querySelector("button")?.focus();
    else if (!question) document.querySelector<HTMLElement>("aside[aria-label=AI] textarea")?.focus();
  }, [running, question]);
  if (question)
    return (
      <div role="group" aria-labelledby="vision-question-title" className="grid grid-cols-[20px_1fr] gap-2 rounded-[10px] bg-raised p-3 text-[12px] leading-[18px] text-fg/85">
        <ScanFace size={16} className="mt-px text-accent" />
        <div className="flex min-w-0 flex-col gap-0.5">
          <span id="vision-question-title" className="text-[13px] font-semibold text-fg">
            {MODEL_QUESTION.vision.title}
          </span>
          <span>The AI needs them to find faces and the person in your video. They run on this computer, a one-time download.</span>
          {/* The agent waits for this answer, so the AI lock does not hold it. */}
          <LockReason.Provider value={null}>
            <div className="mt-2 flex flex-wrap gap-1.5">
              <Button variant="primary" className="h-7 px-2.5 text-[12px]" onClick={() => answerModelQuestion("vision", true)}>
                Download {question.sizeMb} MB
              </Button>
              <Button variant="ghost" className="h-7 px-2.5 text-[12px]" onClick={() => answerModelQuestion("vision", false)}>
                Not now
              </Button>
            </div>
          </LockReason.Provider>
        </div>
      </div>
    );
  if (!running) return null;
  return (
    <div ref={card} className="flex flex-col gap-1.5 rounded-[10px] bg-raised p-3 text-[12px]" role="status">
      <div className="flex items-center gap-2">
        <Loader2 size={13} className="shrink-0 animate-spin text-accent" />
        <span className="min-w-0 flex-1 truncate text-fg">Downloading face and person models</span>
        {job.progress > 0 && <span className="tabular text-muted">{Math.round(job.progress * 100)}%</span>}
        <LockReason.Provider value={null}>
          <Button className="h-6 px-2 text-[12px]" title="The AI goes on without them" onClick={() => void api.cancelJob(job.id)}>
            Stop
          </Button>
        </LockReason.Provider>
      </div>
      <ProgressBar value={job.progress} label="Downloading face and person models" />
    </div>
  );
}

/** "Today 10:42", "Yesterday 21:47", "Monday 09:15", "Sep 28 18:02". */
function dayAndTime(ms: number) {
  const time = new Date(ms).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const today = new Date(ms).toDateString() === new Date().toDateString();
  return `${today ? "Today" : whenLabel(ms)} ${time}`;
}

function Separator({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-center gap-3 text-[12px] text-muted" role="separator">
      <span className="h-px flex-1 bg-white/[.06]" />
      {children}
      <span className="h-px flex-1 bg-white/[.06]" />
    </div>
  );
}

function Conversation() {
  const items = useAgent((s) => s.items);
  const visionQuestion = useEditor((s) => s.visionModelQuestion);
  // The editor's conversation asks for the face and person models; Your style's is about the style.
  const project = useContext(About) === "project";
  useEffect(() => {
    if (!project) return;
    useEditor.setState((s) => ({ aiChats: s.aiChats + 1 }));
    return () => useEditor.setState((s) => ({ aiChats: s.aiChats - 1 }));
  }, [project]);
  const pastCount = useAgent((s) => s.pastCount);
  const demoItems = useAgent((s) => s.demoItems);
  const working = useAgent((s) => s.status !== "idle");
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  // Follows new content while the user is at the bottom; scrolling up to read stops that.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [items, visionQuestion]);
  const lastUser = items.map((i) => i.kind).lastIndexOf("user");
  // A reopened chat says when it was, and when today's part of it starts.
  const sentAt = (from: number) => {
    const user = items.slice(from).find((i) => i.kind === "user");
    return user?.kind === "user" ? (user.at ?? null) : null;
  };
  const lines = new Map<number, number>();
  if (pastCount !== null) {
    const first = sentAt(0);
    if (first !== null) lines.set(0, first);
    const later = sentAt(pastCount);
    if (later !== null) lines.set(pastCount, later);
  }
  return (
    <div
      ref={scroller}
      onScroll={(e) => {
        const el = e.currentTarget;
        pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
      }}
      className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-3.5 pb-2 pt-3.5 text-[13px]"
    >
      {items.length === 0 ? <Empty /> : <div className="mt-auto" />}
      {items.map((item, i) => (
        <Fragment key={i}>
          {lines.has(i) && (
            <Separator>
              <span className="tabular">{dayAndTime(lines.get(i)!)}</span>
            </Separator>
          )}
          {/* The tour's scripted turn stays after the tour, under the real agent's name, so it says no AI wrote it. */}
          {demoItems && i === 0 && <Separator>Tour demo · no AI ran</Separator>}
          {/* Steps fold away once the agent has answered after them. */}
          <Item item={item} index={i} last={i === items.length - 1} past={pastCount !== null && i < pastCount} collapsed={item.kind === "steps" && (i < lastUser || (!working && items.slice(i + 1).some((x) => x.kind === "agent")))} />
        </Fragment>
      ))}
      {project && <VisionModelsQuestion />}
    </div>
  );
}

function Chip({ icon, children, onRemove, label }: { icon: ReactNode; children: ReactNode; onRemove: () => void; label: string }) {
  return (
    <span className="flex h-[22px] min-w-0 items-center gap-1.5 whitespace-nowrap rounded-md bg-white/[.08] pl-1.5 text-[12px] text-fg/90 [&>svg]:text-muted">
      {icon}
      {children}
      <button
        type="button"
        aria-label={`Don't send ${label}`}
        title={`Don't send ${label}`}
        onClick={(e) => {
          e.stopPropagation();
          onRemove();
        }}
        className="flex h-[22px] w-5 items-center justify-center rounded-r-md text-muted hover:bg-white/[.08] hover:text-fg"
      >
        <X size={11} />
      </button>
    </span>
  );
}

function Elapsed() {
  // The start lives in the store, so moving the panel during a run does not restart the count.
  const start = useAgent((s) => s.startedAt) ?? Date.now();
  const [, tick] = useState(0);
  useEffect(() => {
    const id = window.setInterval(() => tick((n) => n + 1), 1000);
    return () => window.clearInterval(id);
  }, []);
  const s = Math.floor((Date.now() - start) / 1000);
  return <>{`${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`}</>;
}

/** Pasting holds the whole file in memory on its way; + reads a larger one from its place instead. */
const MAX_PASTE_BYTES = 100 * 1024 * 1024;

async function pickAttachments() {
  const picked = await openFiles({ multiple: true, filters: [{ name: "Pictures and videos", extensions: [...IMAGE_EXTENSIONS, ...VIDEO_EXTENSIONS] }] });
  if (picked) await attach(Array.isArray(picked) ? picked : [picked]);
}

/** An agent outside the panel, such as one in a terminal, is editing: the panel waits, and the top bar stops it. With Gemini, which
 * answers only in a terminal, that is most likely the person's own ask, so it says what is editing rather than to wait. */
function OtherRun({ label, outside = false }: { label: string; outside?: boolean }) {
  return (
    <div role="status" className="mb-2 grid grid-cols-[20px_1fr] gap-2 rounded-[10px] bg-raised p-3 text-[12px] leading-[18px] text-fg/85">
      <Info size={16} className="text-accent" />
      <div>
        <div className="text-[13px] font-semibold text-fg">{outside ? "An agent outside Nuzky is editing." : "Another agent is editing."}</div>
        {label}. {outside ? "Stop and edit in the top bar stops it." : "Wait until it finishes, or stop it with Stop and edit in the top bar."}
      </div>
    </div>
  );
}

function Composer() {
  const about = useContext(About);
  const draft = useAgent((s) => s.draft);
  const attachments = useAgent((s) => s.attachments);
  const attaching = useAgent((s) => s.attaching);
  const status = useAgent((s) => s.status);
  const frame = useAgent((s) => s.frame);
  const thumbnail = useAgent((s) => s.thumbnail && about === "project");
  const models = useCover((s) => s.models);
  const modelsJob = useEditor((s) => {
    const id = useCover.getState().modelsJob;
    return id && s.jobs[id]?.status === "running" ? s.jobs[id] : null;
  });
  const dropSelection = useAgent((s) => s.dropSelection);
  const dropPlayhead = useAgent((s) => s.dropPlayhead);
  const jobs = useAgent((s) => s.jobs);
  const running = jobs.filter((j) => j.status === "running");
  const failed = jobs.find((j) => j.status === "failed");
  const agents = useAgent((s) => s.agents);
  const agent = useAgent((s) => s.agent);
  const hasItems = useAgent((s) => s.items.length > 0);
  const reopened = useAgent((s) => s.pastCount !== null) && about === "project";
  const selected = useEditor((s) => s.selection.length);
  // From the selection and the project only, so playing does not recompute it every frame.
  const selection = useEditor((s) => s.selection);
  const project = useEditor((s) => s.snap?.project);
  const range = useMemo(() => {
    const chosen = new Set(selection);
    const clips = project?.tracks.flatMap((t) => t.clips).filter((c) => chosen.has(c.id)) ?? [];
    if (!clips.length) return null;
    const start = clips.reduce((m, c) => Math.min(m, c.startUs), Infinity);
    const end = clips.reduce((m, c) => Math.max(m, c.startUs + c.durationUs), 0);
    return `${formatTime(start, false)}–${formatTime(end, false)}`;
  }, [selection, project]);
  // Whole seconds, so the chip does not redraw with every frame while playing.
  const playhead = useEditor((s) => formatTime(s.timeUs, false));
  const otherRun = useEditor((s) => (s.aiRun && status === "idle" ? s.aiRun : null));
  const field = useRef<HTMLTextAreaElement>(null);
  const name = agentName(agent, agents);
  const plan = useChatGpt((s) => agent === "chatgpt" && planReady(s.status));
  const installed = agent !== "chatgpt" && !!agents?.find((a) => a.id === agent)?.path;
  const working = status !== "idle";
  // The tour's scripted turn: its request is sent as written, by no agent, so nothing names or configures one.
  const demo = useAgent((s) => s.demo) && about === "project";
  const blocked = demo ? demoBlocked() : working ? steerBlocked() : sendBlocked();

  useLayoutEffect(() => {
    const el = field.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [draft]);
  // The field takes focus when the panel opens, so typing can start at once.
  useEffect(() => field.current?.focus(), []);

  // Enter that cannot send says why for a moment, instead of doing nothing.
  const [why, setWhy] = useState<string | null>(null);
  useEffect(() => {
    if (!why) return;
    const id = window.setTimeout(() => setWhy(null), 3000);
    return () => window.clearTimeout(id);
  }, [why]);
  // A new state makes an old reason wrong.
  useEffect(() => setWhy(null), [status]);
  // Fresh from the store: Enter can come before React has drawn the latest text.
  const submit = () => {
    const now = useAgent.getState();
    if (!now.draft.trim() && !now.thumbnail && !now.attachments.length) return;
    if (demo) {
      const reason = demoBlocked();
      if (reason) setWhy(reason);
      else void sendDemo();
      return;
    }
    const reason = now.attaching ? "Wait until the attachment is added" : now.status === "idle" ? sendBlocked() : steerBlocked();
    if (reason) setWhy(reason);
    else if (useAgent.getState().status === "idle") void send(undefined, about);
    else void steer();
  };
  const sendTitle = !working
    ? "Send (Enter)"
    : agent === "codex"
      ? `Send now (Enter). ${name} stops its current step and goes on with this, or answers it next when it has not started yet`
      : `Send now (Enter). ${name} takes it after its current step`;
  const sendButton = (
    <button
      type="button"
      aria-label={working ? "Send now" : "Send"}
      title={blocked ?? (draft.trim() || attachments.length || (thumbnail && !working) ? sendTitle : "Type a message first")}
      aria-disabled={!!blocked || (!draft.trim() && !thumbnail && !attachments.length) || undefined}
      onClick={submit}
      className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-accent-strong text-white transition-colors duration-[120ms] hover:bg-[#0064c8] aria-disabled:cursor-not-allowed aria-disabled:bg-white/[.08] aria-disabled:text-white/40"
    >
      <ArrowUp size={16} />
    </button>
  );

  return (
    <div className="shrink-0 px-2.5 pb-2.5">
      {otherRun && <OtherRun label={otherRun} />}
      {/* A click on the model menu or its button keeps the focus there, which the menu needs to stay open. */}
      <div data-tour="ai-field" onClick={(e) => !(e.target as Element).closest("[role=menu],[aria-haspopup=menu]") && field.current?.focus()} className="rounded-xl border border-white/[.08] bg-white/[.055] px-2.5 pb-1.5 pt-2 transition-colors duration-[120ms] has-[textarea:focus]:border-accent">
        {about === "style" && (
          <div className="mb-2 flex flex-wrap gap-1.5">
            <span className="flex h-[22px] items-center gap-1.5 rounded-md bg-white/[.08] px-2 text-[12px] text-fg" title="Messages here are about your style, for all your videos">
              <Scissors size={12} className="text-muted" />
              Your style
            </span>
          </div>
        )}
        {!demo && (attachments.length > 0 || attaching > 0 || (about === "project" && ((selected > 0 && !dropSelection) || !dropPlayhead || frame || thumbnail))) && (
          <div className="mb-2 flex max-h-[76px] flex-wrap gap-1.5 overflow-y-auto">
            {attachments.map((a) => (
              <Chip
                key={a.path}
                icon={a.preview ? <img src={a.preview} alt="" className="h-4 w-4 shrink-0 rounded-[3px] object-cover" /> : a.kind === "video" ? <Film size={12} /> : <ImageIcon size={12} />}
                label={a.name}
                onRemove={() => detach(a.path)}
              >
                <span className="max-w-[160px] truncate" title={a.name}>
                  {a.name}
                </span>
              </Chip>
            ))}
            {attaching > 0 && (
              <span className="flex h-[22px] items-center gap-1.5 rounded-md bg-white/[.08] px-2 text-[12px] text-muted" role="status">
                <Loader2 size={12} className="animate-spin" />
                Adding…
              </span>
            )}
            {about === "project" && thumbnail && (
              <Chip icon={<ImageIcon size={12} />} label="Make a thumbnail" onRemove={() => useAgent.setState({ thumbnail: false })}>
                Make a thumbnail
              </Chip>
            )}
            {about === "project" && selected > 0 && !dropSelection && (
              <Chip icon={<Film size={12} />} label="the selected clips" onRemove={() => useAgent.setState({ dropSelection: true })}>
                {selected === 1 ? "1 clip" : `${selected} clips`}
                {range && <span className="tabular text-muted">· {range}</span>}
              </Chip>
            )}
            {about !== "project" ? null : frame ? (
              <Chip icon={<Camera size={12} />} label="the frame" onRemove={() => useAgent.setState({ frame: false })}>
                <span className="tabular">Frame {playhead}</span>
              </Chip>
            ) : (
              !dropPlayhead && (
                <Chip icon={<Crosshair size={12} />} label="the playhead" onRemove={() => useAgent.setState({ dropPlayhead: true })}>
                  <span className="tabular">Playhead {playhead}</span>
                </Chip>
              )
            )}
          </div>
        )}
        {!working && thumbnail && models && !models.downloaded && !models.unavailable && (
          <div className="mb-2 flex items-center gap-2 text-[12px] text-fg/85" role="status">
            {modelsJob ? (
              <>
                <Loader2 size={13} className="shrink-0 animate-spin text-accent" />
                <span className="tabular min-w-0 flex-1 truncate">Downloading cover models {Math.round(modelsJob.progress * 100)}%</span>
                <Button className="h-6 px-2 text-[12px]" onClick={() => void api.cancelJob(modelsJob.id)}>
                  Stop
                </Button>
              </>
            ) : (
              <>
                <span className="min-w-0 flex-1">Needs the cover models first ({models.sizeMb} MB, once).</span>
                <Button className="h-6 px-2 text-[12px]" onClick={() => void downloadModels()}>
                  Download
                </Button>
              </>
            )}
          </div>
        )}
        {!working && running.length > 0 ? (
          <div className="mb-2 flex items-center gap-2 text-[12px] text-fg/85" role="status">
            <Loader2 size={13} className="shrink-0 animate-spin text-accent" />
            <span className="tabular min-w-0 flex-1 truncate">
              {jobText(running[0])}
              {running.length > 1 && <span className="text-muted"> · {running.length - 1} more</span>}
            </span>
            <Button className="h-6 px-2 text-[12px]" disabled={running.every((j) => j.cancel_requested)} title="Stop what the agent started. It can start it again when you ask." onClick={() => void stop()}>
              Stop
            </Button>
          </div>
        ) : (
          !working &&
          failed && (
            <div className="mb-2 flex items-center gap-2 text-[12px] text-fg/85" role="status">
              <AlertCircle size={13} className="shrink-0 text-danger" />
              <span className="min-w-0 flex-1 truncate" title={jobText(failed)}>
                {jobText(failed)}
              </span>
            </div>
          )
        )}
        <textarea
          ref={field}
          rows={2}
          value={draft}
          readOnly={demo}
          aria-label={demo ? "Request to the AI demo" : `Message to ${name}`}
          placeholder={demo ? (working ? "" : "After the tour, ask your own AI here") : working ? `Tell ${name} more while it works…` : blocked === "Start a new chat to send this" ? `${name} can't continue this chat` : reopened ? `Continue with ${name}` : agent === "chatgpt" && !plan ? "Sign in to ChatGPT to start" : thumbnail ? "Add a note, e.g. the hook (optional)" : about === "style" ? "What should change in how you edit?" : hasItems ? "Ask for changes…" : `Ask ${name} to edit…`}
          onChange={(e) => useAgent.setState({ draft: e.target.value })}
          onPaste={(e) => {
            // A screenshot, or a file copied in Finder; text pastes as usual.
            // WebKit names every pasted screenshot image.png; the time tells them apart.
            const files = [...e.clipboardData.files]
              .filter((f) => !f.type || /^(image|video)\//.test(f.type))
              .map((f) => (f.name === "image.png" ? new File([f], `Pasted ${new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })}.png`, { type: f.type }) : f));
            if (!files.length) return;
            e.preventDefault();
            const big = files.find((f) => f.size > MAX_PASTE_BYTES);
            if (big) useEditor.getState().toast({ kind: "error", text: `${big.name} is too big to paste. Attach it with + instead.` });
            void attach(files.filter((f) => f.size <= MAX_PASTE_BYTES));
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              submit();
            }
          }}
          className="block max-h-40 w-full resize-none bg-transparent text-[13px] leading-[19px] text-fg outline-none placeholder:text-muted"
        />
        <div className="mt-1 flex items-center gap-2">
          {!demo && (
            <IconButton label="Attach pictures or videos. You can also paste them into the field." onClick={() => void pickAttachments()} className="h-7 w-7">
              <Plus size={16} />
            </IconButton>
          )}
          {about === "project" && !demo && (
          <IconButton
            label={frame ? "Don't show the agent the frame at the playhead" : "Show the agent the frame at the playhead"}
            active={frame}
            onClick={() => useAgent.setState(frame ? { frame: false } : { frame: true, dropPlayhead: false })}
            className={frame ? "h-7 w-auto gap-1.5 px-2 text-[12px] font-medium" : "h-7 w-7"}
          >
            <Camera size={15} />
            {frame && "Frame"}
          </IconButton>
          )}
          {/* Hidden while the agent answers, so its progress and the reasons have the row. */}
          {(plan || installed) && !working && !demo && <ModelMenu />}
          <span className="flex-1" />
          {!working && why && (
            <span className="min-w-0 truncate text-[12px] text-fg/85" role="status">
              {why}
            </span>
          )}
          {working ? (
            <>
              <span className="tabular flex min-w-0 items-center gap-1.5 text-[12px] text-muted" role="status">
                <Loader2 size={13} className="shrink-0 animate-spin text-accent" />
                {why ? (
                  <span className="truncate text-fg/85">{why}</span>
                ) : status === "stopping" ? (
                  "Stopping…"
                ) : (
                  <>
                    Working · <Elapsed />
                  </>
                )}
              </span>
              <Button className="h-7 px-2.5 text-[12px]" disabled={status === "stopping"} title="Stop the agent. What it did so far stays, and Undo takes it back." onClick={() => (demo ? stopDemo() : void stop())}>
                <CircleStop size={13} /> Stop
              </Button>
              {(draft.trim() || attachments.length > 0) && sendButton}
            </>
          ) : (
            sendButton
          )}
        </div>
      </div>
      {plan && !demo && (
        <div className="mt-1.5 flex items-center gap-1.5 px-1 text-[12px] text-muted">
          <ChatGptLogo size={12} className="shrink-0" />
          Using ChatGPT plan
          <span aria-hidden>·</span>
          <button type="button" title="Review and limit what Nuzky uses, in ChatGPT's settings" onClick={() => void openUrl(USAGE_PAGE)} className="text-accent hover:underline">
            Manage usage
          </button>
        </div>
      )}
    </div>
  );
}

/** The model and effort the panel sends with: one button, and a menu with the models over the effort. */
function ModelMenu() {
  const about = useContext(About);
  const agent = useAgent((s) => s.agent);
  const models = useModels();
  const list = models.lists[agent];
  const error = models.errors[agent];
  const { model, effort, efforts } = resolved(agent, models);
  // Without a model Claude Code uses its own setting, and so does Codex when its list didn't load.
  const name = model?.displayName ?? (agent === "claude" || (agent === "codex" && error) ? "Default" : error ? "No models" : "Loading models…");
  return (
    <Menu
      label="Model"
      up
      // From the field's edge, so it fits the narrowest panel, 300 px.
      width="w-64"
      place={about === "project" ? "-left-[72px]" : "-left-9"}
      shrink
      trigger={({ open, toggle, ref }) => (
        <button
          ref={ref}
          type="button"
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label={`Model: ${name}${effort ? `, effort: ${effortLabel(effort)}` : ""}`}
          title="Choose the model and effort"
          onClick={toggle}
          className={`flex h-7 min-w-0 max-w-full items-center gap-1 rounded-md px-1.5 text-[12px] text-fg/85 hover:bg-white/[.08] ${open ? "bg-white/[.08]" : ""}`}
        >
          <span className="truncate">{name}</span>
          {effort && (
            <>
              <span aria-hidden className="text-subtle">
                ·
              </span>
              <span className="shrink-0">{effortLabel(effort)}</span>
            </>
          )}
          <ChevronDown size={13} className="shrink-0 text-muted" />
        </button>
      )}
    >
      {() => (
        <>
          <p className="px-2 pb-1 pt-1.5 text-[12px] font-semibold text-muted">Model</p>
          {agent === "claude" && (
            <MenuItem checked={!model} onClick={() => chooseModel(agent, null)}>
              Default
              <span className="ml-auto text-[12px] text-muted">Claude Code setting</span>
            </MenuItem>
          )}
          {error ? (
            <>
              <p className="flex gap-1.5 px-2 py-1.5 text-[12px] leading-[17px] text-fg/85">
                <AlertCircle size={13} className="mt-0.5 shrink-0 text-danger" />
                {error}
              </p>
              <MenuItem onClick={() => void loadModels(agent, agent === "chatgpt" ? models.chatgptFor : null)}>Try again</MenuItem>
            </>
          ) : !list ? (
            <p className="flex items-center gap-1.5 px-2 py-1.5 text-[12px] text-muted">
              <Loader2 size={13} className="shrink-0 animate-spin text-accent" />
              Loading models…
            </p>
          ) : (
            list.map((m) => (
              <MenuItem key={m.slug} checked={m.slug === model?.slug} onClick={() => chooseModel(agent, m.slug)}>
                <span className="truncate">{m.displayName}</span>
              </MenuItem>
            ))
          )}
          {efforts.length > 0 && (
            <>
              <div className="mx-1.5 my-1 h-px bg-white/[.08]" />
              <p className="px-2 pb-1 pt-1.5 text-[12px] font-semibold text-muted">Effort</p>
              <div className="px-1.5">
                <Segmented
                  compact
                  label="Effort"
                  value={effort}
                  options={efforts.map((e) => ({ id: e, label: EFFORTS[e]?.short ?? effortLabel(e), title: effortLabel(e) }))}
                  onChange={(e) => chooseEffort(agent, e)}
                />
              </div>
              <p className="px-2 pb-1.5 pt-1.5 text-[12px] leading-[16px] text-muted" aria-live="polite">
                {effort ? (EFFORTS[effort]?.hint ?? `${effortLabel(effort)} effort.`) : "Uses your Claude Code setting."}
              </p>
            </>
          )}
        </>
      )}
    </Menu>
  );
}

/** The AI chat: header, conversation and the field to write in. AiDock places it. */
/** `about: "style"` is the panel beside the Your style page: it stays there, and messages are about the style. */
export function AiPanel({
  panelRef,
  className = "",
  style,
  about = "project",
  onClose,
}: {
  panelRef: React.RefObject<HTMLElement | null>;
  className?: string;
  style?: React.CSSProperties;
  about?: AiAbout;
  onClose?: () => void;
}) {
  const busy = useAgent((s) => s.status !== "idle");
  const empty = useAgent((s) => s.items.length === 0);
  const left = useAgent((s) => s.jobs.some((j) => j.status === "running"));
  const kept = useAgent((s) => (s.chats === null ? null : s.chats.length));
  const projectName = useEditor((s) => s.snap?.project.name);
  // The tour's scripted turn names no agent and keeps no chat.
  const demo = useAgent((s) => s.demo) && about === "project";
  // Gemini answers in a terminal, so the panel shows how to reach it there instead of a conversation.
  const gemini = useAgent((s) => s.agent === "antigravity") && !demo;
  const otherRun = useEditor((s) => s.aiRun);
  const [listing, setListing] = useState(false);
  // An answer starting elsewhere, such as Make it with AI, brings the conversation back.
  useEffect(() => {
    if (busy) setListing(false);
  }, [busy]);
  // Your style is about every project, so a project's reopened chat does not continue there.
  useEffect(() => {
    if (about === "style" && useAgent.getState().pastCount !== null) newChat();
  }, [about]);
  const { mode } = useDockLayout();
  useEffect(() => {
    if (useAgent.getState().agents === null) void loadAgents();
  }, []);
  const move = (e: React.PointerEvent) => panelRef.current && startMove(e, panelRef.current);
  return (
    <About value={about}>
    <aside
      ref={panelRef as React.RefObject<HTMLElement>}
      aria-label="AI"
      data-dock-slot={mode === "inspector" ? "inspector" : undefined}
      className={`pane flex flex-col overflow-hidden ${className}`}
      style={style}
    >
      <header
        // The header's empty space moves the panel too; its buttons keep their clicks.
        onPointerDown={(e) => about === "project" && e.target === e.currentTarget && move(e)}
        className="flex h-12 shrink-0 items-center gap-1 border-b border-white/[.07] pl-1.5 pr-2"
      >
        {about === "project" && <PlaceMenu panel={panelRef} />}
        {listing ? (
          <>
            <IconButton label="Back to the chat (Esc)" onClick={() => setListing(false)} className="h-7 w-7">
              <ArrowLeft size={15} />
            </IconButton>
            <h2 className="min-w-0 truncate px-1 text-[13px] font-semibold text-fg">Chats in {projectName ?? "this project"}</h2>
          </>
        ) : demo ? (
          <h2 className="min-w-0 truncate px-1.5 text-[13px] font-semibold text-fg" title="The tour plays a short scripted edit of the sample. No agent runs.">
            AI demo
          </h2>
        ) : (
          <AgentMenu />
        )}
        <span className={`flex-1 self-stretch ${about === "project" ? "cursor-grab" : ""}`} onPointerDown={about === "project" ? move : undefined} />
        {about === "project" && !listing && !demo && (
          <IconButton
            label={busy ? "Past chats: wait for the answer, or stop it" : kept === null ? "Past chats: loading…" : kept === 0 ? "Past chats: none in this project yet" : "Past chats in this project"}
            disabled={busy || !kept}
            onClick={() => setListing(true)}
            className="h-7 w-7"
          >
            <MessagesSquare size={15} />
          </IconButton>
        )}
        {!listing && !demo && (
        <IconButton
          label={busy ? "New chat: wait for the answer, or stop it" : empty ? "New chat: there is nothing to clear yet" : left ? "New chat: stops what the agent is still doing" : "New chat"}
          disabled={busy || empty}
          onClick={newChat}
          className="h-7 w-7"
        >
          <SquarePen size={15} />
        </IconButton>
        )}
        <IconButton label={about === "style" ? "Close AI" : "Close AI (Ctrl+J)"} onClick={onClose ?? (() => useDock.setState({ open: false }))} className="h-7 w-7">
          <X size={16} />
        </IconButton>
      </header>
      {listing ? (
        <ChatList onBack={() => setListing(false)} />
      ) : gemini ? (
        <>
          <GeminiGuide />
          {otherRun && (
            <div className="shrink-0 px-2.5 pb-0.5">
              <OtherRun label={otherRun} outside />
            </div>
          )}
        </>
      ) : (
        <>
          <Conversation />
          <Composer />
        </>
      )}
    </aside>
    </About>
  );
}

/** The chats this project keeps, newest first: one opens to read and continue. ↑/↓, Home and End move, Enter opens, Esc goes back. */
function ChatList({ onBack }: { onBack: () => void }) {
  const chats = useAgent((s) => s.chats);
  const current = useAgent((s) => s.chat);
  const agents = useAgent((s) => s.agents);
  const left = useAgent((s) => s.jobs.some((j) => j.status === "running"));
  const list = useRef<HTMLUListElement>(null);
  const loaded = chats !== null;
  useEffect(() => void loadChats(), []);
  // Focus starts on the open chat, so Enter goes back to it and ↑/↓ reach the others.
  useEffect(() => {
    const rows = [...(list.current?.querySelectorAll<HTMLElement>("button") ?? [])];
    (rows.find((el) => el.dataset.current) ?? rows[0])?.focus();
  }, [loaded]);
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const rows = [...(list.current?.querySelectorAll<HTMLElement>("button") ?? [])];
    const i = rows.indexOf(document.activeElement as HTMLElement);
    const next = { ArrowDown: i + 1, ArrowUp: i - 1, Home: 0, End: rows.length - 1 }[e.key];
    if (e.key !== "Escape" && next === undefined) return;
    // Only the list's own keys stay here; Ctrl+J and the editor's shortcuts still work.
    e.stopPropagation();
    e.preventDefault();
    if (e.key === "Escape") onBack();
    else rows[Math.max(0, Math.min(rows.length - 1, next!))]?.focus();
  };
  return (
    <div onKeyDown={onKey} className="flex min-h-0 flex-1 flex-col overflow-y-auto px-2 py-2 text-[13px]">
      {left && <p className="mb-1 px-2 text-[12px] leading-[17px] text-muted">Opening another chat stops what the agent is still doing.</p>}
      {chats === null ? (
        <p className="px-2 py-1.5 text-[12px] text-muted">Loading your chats…</p>
      ) : (
        <ul ref={list} aria-label="Past chats" className="flex flex-col gap-0.5">
          {chats.map((c) => {
            const open = c.id === current;
            return (
              <li key={c.id}>
                <button
                  type="button"
                  data-current={open || undefined}
                  aria-current={open || undefined}
                  onClick={() => (open ? onBack() : void openChat(c.id).then(onBack))}
                  className="flex w-full flex-col gap-0.5 rounded-lg px-2.5 py-2 text-left hover:bg-white/[.06]"
                >
                  <span className="flex w-full items-baseline gap-2">
                    <span className="min-w-0 truncate font-medium text-fg" title={c.title || undefined}>
                      {c.title || "Untitled chat"}
                    </span>
                    {open && <span className="shrink-0 text-[12px] text-accent">Open now</span>}
                  </span>
                  <span className="tabular text-[12px] text-muted">
                    {agentName(c.agent, agents)} · {dayAndTime(c.updatedMs)}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

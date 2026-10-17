import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AlertCircle, AlertTriangle, Check, Copy, ExternalLink, Loader2 } from "lucide-react";
import { api, errorText, plainError } from "../../../lib/api";
import { copyText } from "../../../lib/clipboard";
import { pickOneVideo } from "../../../lib/library";
import { useSetup } from "../../../lib/setup";
import { useEditor } from "../../../lib/store";
import type { AgentKind, AgentStatus } from "../../../lib/types";
import { Button, IconButton } from "../../ui";
import { StepActions } from "../StepActions";

const AGENTS: AgentKind[] = ["claudeCode", "codex"];
const NAME: Record<AgentKind, string> = { claudeCode: "Claude Code", codex: "Codex" };
const GUIDE: Record<AgentKind, string> = { claudeCode: "https://code.claude.com/docs/en/setup", codex: "https://learn.chatgpt.com/docs/codex/cli" };
/** The official commands for macOS and Linux, in the parts a narrow column may wrap between. */
const INSTALL: Record<AgentKind, string[]> = {
  claudeCode: ["curl -fsSL", "https://claude.ai/install.sh", "| bash"],
  codex: ["curl -fsSL", "https://chatgpt.com/codex/install.sh | sh"],
};
const SIGN_IN: Record<AgentKind, string[]> = { claudeCode: ["claude"], codex: ["codex login"] };
const MAC = navigator.userAgent.includes("Mac");

/** A Connect (`wrote` once Nuzky is in the agent's settings), or the check of an agent connected before. */
type Run = { phase: "writing" | "checking" | "connected" | "failed"; fresh: boolean; wrote: boolean; backup?: string | null; error?: string; started: number; took?: number };

/** Connect your AI: whether Claude Code and Codex are installed, signed in and reach Nuzky, and how to get each there. */
export function AiStep() {
  const skipped = useSetup((s) => s.states.ai === "skipped");
  const [again, setAgain] = useState(false);
  // Evaluated here, not once, so a test can stand in for Windows.
  const windows = navigator.userAgent.includes("Windows");
  return windows || (skipped && !again) ? <WithoutAi windows={windows} onAgents={() => setAgain(true)} /> : <Agents />;
}

function Agents() {
  const done = useSetup((s) => s.states.ai === "done");
  const [status, setStatus] = useState<Partial<Record<AgentKind, AgentStatus>>>({});
  const [runs, setRuns] = useState<Partial<Record<AgentKind, Run>>>({});
  const [looking, setLooking] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());
  const live = useRef(true);
  const runsNow = useRef(runs);
  runsNow.current = runs;

  const update = (agent: AgentKind, run: Run) => live.current && setRuns((all) => ({ ...all, [agent]: run }));

  const connect = async (agent: AgentKind, write: boolean) => {
    let run: Run = { phase: write ? "writing" : "checking", fresh: write, wrote: false, started: Date.now() };
    update(agent, run);
    if (write) {
      try {
        run = { ...run, phase: "checking", wrote: true, backup: (await api.connectAgent(agent)).backup, started: Date.now() };
      } catch (e) {
        return update(agent, { ...run, phase: "failed", error: errorText(e) });
      }
      update(agent, run);
    }
    try {
      await api.verifyAgent(agent);
      update(agent, { ...run, phase: "connected", took: Date.now() - run.started });
      useSetup.getState().finishStep("ai");
    } catch (e) {
      update(agent, { ...run, phase: "failed", error: errorText(e) });
    }
  };

  // An agent connected before is checked again, so Connected always means it reaches Nuzky now.
  const look = () => {
    setLooking(true);
    setError(null);
    return Promise.all(
      AGENTS.map((agent) =>
        api.agentStatus(agent).then(
          (found) => {
            if (!live.current) return;
            setStatus((all) => ({ ...all, [agent]: found }));
            const run = runsNow.current[agent];
            const connected = found.installed && found.signedIn !== "signedOut" && found.connection.state === "connected";
            if (connected && (!run || run.phase === "failed")) void connect(agent, false);
          },
          (e) => live.current && setError(errorText(e)),
        ),
      ),
    ).finally(() => live.current && setLooking(false));
  };

  const lookNow = useRef(look);
  lookNow.current = look;
  useEffect(() => {
    live.current = true;
    void lookNow.current();
    return () => {
      live.current = false;
    };
  }, []);

  const checking = Object.values(runs).some((r) => r.phase === "checking");
  useEffect(() => {
    if (!checking) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [checking]);

  const busy = AGENTS.find((a) => runs[a]?.phase === "writing" || runs[a]?.phase === "checking");
  const connectable = (a: AgentKind) => {
    const s = status[a];
    return !!s?.installed && s.signedIn !== "signedOut" && s.connection.state !== "unreadable" && runs[a]?.phase !== "connected";
  };
  // One prominent button at a time: Try again, else the first Connect, else Check again; once done, Done.
  const prominent = done ? undefined : (AGENTS.find((a) => runs[a]?.phase === "failed") ?? AGENTS.find((a) => connectable(a) && !runs[a]));
  // An agent still to install, sign in or fix, which Check again looks for.
  const waiting = AGENTS.some((a) => {
    const s = status[a];
    return !s || !s.installed || s.signedIn === "signedOut" || s.connection.state === "unreadable";
  });

  // Connect or Check again going away leaves the focus nowhere; it goes to the prominent button, never to Skip.
  useEffect(() => {
    if (document.activeElement !== document.body) return;
    (document.querySelector<HTMLElement>("#setup-panel .btn-prominent") ?? document.querySelector<HTMLElement>("#setup-panel footer button"))?.focus();
  });

  return (
    <div className="flex flex-col gap-3 pt-3">
      <p className="text-[13px] leading-[18px] text-fg">
        Claude Code or Codex can cut your videos in Nuzky.
        <span className="block text-muted">Set up one of them.</span>
      </p>
      <div className="flex flex-col gap-2">
        {AGENTS.map((agent) => (
          <AgentCard
            key={agent}
            agent={agent}
            status={status[agent]}
            run={runs[agent]}
            now={now}
            prominent={prominent === agent}
            wait={busy && busy !== agent ? `Wait for the check of ${NAME[busy]}` : null}
            onConnect={() => void connect(agent, true)}
          />
        ))}
      </div>
      {!Object.keys(runs).length && AGENTS.some(connectable) && (
        <p className="text-[12px] leading-4 text-muted">Connect backs up the agent's settings and adds Nuzky to them, so it can edit your videos now and every time Nuzky starts.</p>
      )}
      {error && (
        <p className="flex gap-1.5 text-[12px] text-danger" role="alert">
          <AlertTriangle size={14} className="mt-px shrink-0" />
          Could not look for the agents: {plainError(error)}
        </p>
      )}
      {waiting && (
        <StepActions>
          <Button pill variant={prominent || done ? "secondary" : "primary"} disabled={looking} disabledReason="Looking for Claude Code and Codex…" onClick={() => void look()}>
            Check again
          </Button>
        </StepActions>
      )}
    </div>
  );
}

function AgentCard({
  agent,
  status,
  run,
  now,
  prominent,
  wait,
  onConnect,
}: {
  agent: AgentKind;
  status?: AgentStatus;
  run?: Run;
  now: number;
  prominent: boolean;
  wait: string | null;
  onConnect: () => void;
}) {
  const name = NAME[agent];
  const phase = run?.phase;
  const unreadable = status?.connection.state === "unreadable" && !run;
  const state = !status
    ? "Looking for it…"
    : !status.installed
      ? "Not installed"
      : status.signedIn === "signedOut"
        ? "Not signed in"
        : unreadable
          ? "Its settings can't be read"
          : phase === "writing" || phase === "checking"
            ? "Connecting…"
            : phase === "connected"
              ? "Connected"
              : phase === "failed"
                ? "Couldn't connect"
                : status.signedIn === "signedIn"
                  ? "Signed in"
                  : "Installed";
  const canConnect = !!status?.installed && status.signedIn !== "signedOut" && !unreadable && phase !== "connected";
  const seconds = (ms: number) => `${Math.max(1, Math.round(ms / 1000))} s`;
  return (
    <div role="group" aria-label={name} className="rounded-xl bg-white/[.055] px-3.5 py-3 shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
      <div className="flex min-h-8 items-center gap-3">
        <div className="flex min-w-0 flex-1 flex-col gap-[3px]">
          <span className="text-[13px] font-medium leading-4 text-fg">
            {name}
            {status?.version && <span className="tabular ml-1.5 text-[12px] font-normal text-muted">{status.version}</span>}
          </span>
          <span aria-live="polite" className={`flex items-center gap-[5px] text-[12px] leading-[15px] ${phase === "connected" || phase === "failed" ? "text-fg" : "text-muted"}`}>
            {phase === "connected" && <Check size={13} strokeWidth={2.5} className="shrink-0 text-ok" />}
            {state}
          </span>
        </div>
        {status && !status.installed && (
          <button type="button" onClick={() => void openUrl(GUIDE[agent])} className="-mr-2 flex h-7 shrink-0 items-center gap-1 rounded-lg px-2 text-[12px] font-medium text-accent hover:bg-white/[.06]">
            Install guide <ExternalLink size={12} />
          </button>
        )}
        {canConnect && (
          <Button
            pill
            variant={prominent ? "primary" : "secondary"}
            disabled={phase === "writing" || phase === "checking" || !!wait}
            disabledReason={wait ?? "Connecting…"}
            onClick={onConnect}
          >
            {phase === "failed" ? "Try again" : "Connect"}
          </Button>
        )}
      </div>
      {status && !status.installed && (
        <>
          <p className="mt-2.5 text-[12px] leading-4 text-muted">
            To install, paste this into {MAC ? "Terminal" : "a terminal"} and press {MAC ? "Return" : "Enter"}.
          </p>
          <Command parts={INSTALL[agent]} />
        </>
      )}
      {status?.installed && status.signedIn === "signedOut" && (
        <>
          <p className="mt-2.5 text-[12px] leading-4 text-muted">To sign in, run this in a new {MAC ? "Terminal" : "terminal"} window and follow its steps.</p>
          <Command parts={SIGN_IN[agent]} />
        </>
      )}
      {unreadable && (
        <p className="mt-2.5 text-[12px] leading-4 text-muted" title={status.connection.problem ?? undefined}>
          {status.connection.path} can't be read, so Nuzky leaves it alone. Fix it, then press Check again.
        </p>
      )}
      {run && (
        <ol aria-label="Steps" className="mt-2.5 flex flex-col border-t border-white/[.06] pt-1.5 text-[12px] leading-4">
          {run.phase === "writing" && <Row icon="running" text={`Adding Nuzky to ${name}'s settings`} />}
          {run.wrote && run.backup && <Row icon="done" text={`Backed up ${status?.connection.path ?? ""}`} title={`Old settings: ${run.backup}`} />}
          {run.wrote && <Row icon="done" text="Added Nuzky and turned on Agent access" />}
          {run.phase === "checking" && <Row icon="running" text={`Checking that ${name} sees Nuzky`} detail={seconds(now - run.started)} />}
          {run.phase === "connected" && <Row icon="done" text={`${name} sees Nuzky`} detail={seconds(run.took ?? 0)} />}
          {run.phase === "failed" && <Row icon="failed" text={run.error ?? ""} />}
        </ol>
      )}
      {run?.phase === "connected" && run.fresh && <p className="mt-1.5 text-[12px] leading-4 text-fg">If {name} is open, restart it so it sees Nuzky.</p>}
    </div>
  );
}

const ROW_ICON = {
  running: <Loader2 size={14} className="animate-spin text-accent" />,
  done: <Check size={14} className="text-ok" />,
  failed: <AlertCircle size={14} className="mt-px text-danger" />,
};

/** A step of a Connect, as the AI panel's steps. */
function Row({ icon, text, detail, title }: { icon: keyof typeof ROW_ICON; text: string; detail?: string; title?: string }) {
  const failed = icon === "failed";
  return (
    <li role={failed ? "alert" : undefined} title={title} className={`grid min-h-[26px] grid-cols-[18px_minmax(0,1fr)_auto] gap-x-1.5 ${failed ? "items-start py-[5px]" : "items-center"}`}>
      {ROW_ICON[icon]}
      <span className={failed || icon === "running" ? "text-fg" : "text-fg/80"}>{text}</span>
      {detail && <span className="tabular text-muted">{detail}</span>}
    </li>
  );
}

/** A command to paste, which wraps only between its parts, and Copy. */
function Command({ parts }: { parts: string[] }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const id = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(id);
  }, [copied]);
  const text = parts.join(" ");
  const copy = () =>
    copyText(text, "The clipboard is not available here. Select the command and copy it.").then(
      () => setCopied(true),
      (e: Error) => useEditor.getState().toast({ kind: "error", text: e.message }),
    );
  return (
    <div className="mt-1.5 flex items-start gap-1 rounded-lg bg-black/25 py-1 pl-2.5 pr-1 shadow-[inset_0_0_0_1px_rgb(255_255_255/.08)]">
      <code className="min-w-0 flex-1 select-all py-1 font-mono text-[11.5px] leading-4 text-fg [overflow-wrap:anywhere]">
        {parts.map((part, i) => (
          <span key={part}>
            {i > 0 && " "}
            <span className="whitespace-nowrap">{part}</span>
          </span>
        ))}
      </code>
      <IconButton label={copied ? "Copied" : "Copy the command"} onClick={() => void copy()} className="h-6 w-6 shrink-0">
        {copied ? <Check size={13} strokeWidth={2.5} className="text-ok" /> : <Copy size={13} />}
      </IconButton>
    </div>
  );
}

/** Skipped, or on Windows: the way to cut by hand in the transcript, and the video to start with. */
function WithoutAi({ windows, onAgents }: { windows: boolean; onAgents: () => void }) {
  // Pressing Skip leaves the focus on the footer's Done that took its place; Open a video… is the way on.
  useEffect(() => {
    const focus = document.activeElement;
    if (focus === document.body || focus?.closest("#setup-panel footer")) document.querySelector<HTMLElement>("#setup-panel footer button")?.focus();
  }, []);
  const openVideo = () =>
    void pickOneVideo(() => {
      useEditor.setState({ panelTab: "transcript" });
      useSetup.getState().closeSetup();
    });
  return (
    <div className="flex flex-col gap-3 pt-3">
      {windows && (
        <p className="text-[13px] leading-[18px] text-fg">
          AI editing isn't available on Windows yet. <span className="text-muted">Claude Code and Codex edit in Nuzky on macOS and Linux.</span>
        </p>
      )}
      <div>
        <p className="text-[13px] font-semibold leading-4 text-fg">Cut without AI</p>
        <p className="mt-1 text-[13px] leading-[18px] text-muted">Nuzky writes out what's said. Cutting words from the text cuts them from the video.</p>
      </div>
      <ol className="flex flex-col gap-2.5 text-[13px] leading-[18px] text-fg">
        {["Open a talking head video", <>In Transcript, press <b className="font-medium">Transcribe timeline</b></>, <>Press <b className="font-medium">Remove retakes and pauses</b></>, <>Select other words and press <b className="font-medium">Delete</b></>].map((line, i) => (
          <li key={i} className="flex items-start gap-2.5">
            <span className="tabular flex size-5 shrink-0 items-center justify-center rounded-full bg-white/[.09] text-[11px] font-semibold">{i + 1}</span>
            <span className="pt-px">{line}</span>
          </li>
        ))}
      </ol>
      {!windows && (
        <button type="button" onClick={onAgents} className="self-start text-[12px] font-medium text-accent hover:underline">
          Set up Claude Code or Codex instead
        </button>
      )}
      <StepActions>
        <Button pill variant="primary" onClick={openVideo}>
          Open a video…
        </Button>
      </StepActions>
    </div>
  );
}

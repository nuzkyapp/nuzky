import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AlertTriangle, Check, ExternalLink } from "lucide-react";
import { api, errorText, plainError } from "../../../lib/api";
import { pickOneVideo } from "../../../lib/library";
import { useSetup } from "../../../lib/setup";
import { useEditor } from "../../../lib/store";
import { useTour } from "../../../lib/tour";
import type { AgentKind, AgentStatus } from "../../../lib/types";
import { Button } from "../../ui";
import { StepActions } from "../StepActions";
import { AskGemini, Command, ConnectRows, GUIDE, GeminiWhy, INSTALL, MAC, PASTE, RETURN, TERMINAL, connectAgent, type Run } from "../agentConnect";

const AGENTS: AgentKind[] = ["claudeCode", "codex", "antigravity"];
/** Gemini runs in Google's Antigravity CLI, `agy`, which is what gets installed and connected. */
const NAME: Record<AgentKind, string> = { claudeCode: "Claude Code", codex: "Codex", antigravity: "Gemini" };
/** Antigravity CLI cannot say whether it is signed in without going online, so it is never shown signed out. */
const SIGN_IN: Partial<Record<AgentKind, string[]>> = { claudeCode: ["claude"], codex: ["codex login"] };

/** Connect your AI: whether Claude Code, Codex and Gemini's Antigravity CLI are installed, signed in and reach Nuzky, and how to get each there. */
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
    if (await connectAgent(agent, write, (run) => update(agent, run))) useSetup.getState().finishStep("ai");
  };

  // A connected agent is checked again, also one checked before, so Connected always means it reaches Nuzky now.
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
            if (run?.phase === "writing" || run?.phase === "checking") return;
            if (found.installed && found.signedIn !== "signedOut" && found.connection.state === "connected") void connect(agent, false);
            else if (run)
              setRuns((all) => {
                const rest = { ...all };
                delete rest[agent];
                return rest;
              });
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
  const waiting = AGENTS.some((a) => {
    const s = status[a];
    return !s || !s.installed || s.signedIn === "signedOut" || s.connection.state === "unreadable";
  });

  // Connect or Check again going away leaves the focus nowhere; it goes to the prominent button, never to Skip.
  useEffect(() => {
    if (document.activeElement !== document.body) return;
    (document.querySelector<HTMLElement>("#setup-panel .btn-prominent") ?? document.querySelector<HTMLElement>("#setup-panel footer button:not([data-sample])"))?.focus();
  });

  return (
    <div className="flex flex-col gap-3 pt-3">
      <p className="text-[13px] leading-[18px] text-fg">
        Claude Code, Codex or Gemini can cut your videos.
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
        <p className="text-[12px] leading-4 text-muted">
          Connect backs up the agent's settings and adds Nuzky to them, so it can edit your videos now and every time Nuzky starts. Gemini may then use Nuzky's tools without asking before
          each step.
        </p>
      )}
      {error && (
        <p className="flex gap-1.5 text-[12px] text-danger" role="alert">
          <AlertTriangle size={14} className="mt-px shrink-0" />
          Could not look for the agents: {plainError(error)}
        </p>
      )}
      {waiting && (
        <StepActions>
          <Button pill variant={prominent || done ? "secondary" : "primary"} disabled={looking} disabledReason="Looking for your agents…" onClick={() => void look()}>
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
  const gemini = agent === "antigravity";
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
  // Gemini answers in a terminal, so once it is connected, how to ask it is the next thing to do, not Done.
  const ask = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!gemini || phase !== "connected" || !run?.fresh) return;
    ask.current?.scrollIntoView({ block: "nearest" });
    ask.current?.querySelector("button")?.focus();
  }, [gemini, phase, run?.fresh]);
  return (
    <div role="group" aria-label={name} className="rounded-xl bg-white/[.055] px-3.5 py-3 shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
      <div className="flex min-h-8 items-center gap-3">
        <div className="flex min-w-0 flex-1 flex-col gap-[3px]">
          <span className="text-[13px] font-medium leading-4 text-fg">
            {name}
            {status?.version && <span className="tabular ml-1.5 text-[12px] font-normal text-muted">{gemini ? `Antigravity CLI ${status.version}` : status.version}</span>}
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
      {gemini && phase !== "connected" && (
        <div className="mt-2">
          <GeminiWhy />
        </div>
      )}
      {status && !status.installed && (
        <>
          <p className="mt-2.5 text-[12px] leading-4 text-muted">
            {gemini && MAC
              ? "To install Google's Antigravity CLI, open Terminal (press ⌘ Space, type Terminal and press Return), paste this and press Return."
              : gemini
                ? `To install Google's Antigravity CLI, open a terminal, ${PASTE} and press Enter.`
                : `To install, paste this into ${TERMINAL} and press ${RETURN}.`}
          </p>
          <Command parts={INSTALL[agent]} />
        </>
      )}
      {status?.installed && status.signedIn === "signedOut" && SIGN_IN[agent] && (
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
      {run && <ConnectRows agent={agent} name={name} run={run} now={now} />}
      {run?.phase === "connected" &&
        (gemini ? (
          <div ref={ask} className="mt-2">
            <AskGemini agy={status?.path ?? "agy"} fresh={run.fresh} />
          </div>
        ) : (
          run.fresh && <p className="mt-1.5 text-[12px] leading-4 text-fg">If {name} is open, restart it so it sees Nuzky.</p>
        ))}
    </div>
  );
}

/** Skipped, or on Windows: the way to cut by hand in the transcript, and the video to start with. */
function WithoutAi({ windows, onAgents }: { windows: boolean; onAgents: () => void }) {
  // Pressing Skip leaves the focus on the footer's Done that took its place; the prominent button is the way on: Open a
  // video…, or on the first launch Try the sample video.
  const firstSetup = useTour((s) => s.firstSetup);
  const ended = useSetup((s) => !!s.states.ai);
  useEffect(() => {
    const focus = document.activeElement;
    if (focus === document.body || focus?.closest("#setup-panel footer")) document.querySelector<HTMLElement>("#setup-panel footer .btn-prominent")?.focus();
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
          AI editing isn't available on Windows yet. <span className="text-muted">Claude Code, Codex and Gemini edit in Nuzky on macOS and Linux.</span>
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
          Set up Claude Code, Codex or Gemini instead
        </button>
      )}
      <StepActions>
        <Button pill variant={firstSetup && ended ? "secondary" : "primary"} onClick={openVideo}>
          Open a video…
        </Button>
      </StepActions>
    </div>
  );
}

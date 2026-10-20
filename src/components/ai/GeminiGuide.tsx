import { useEffect, useRef, useState, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, ExternalLink, RefreshCw } from "lucide-react";
import { loadGemini, useAgent } from "../../lib/agent";
import { api, errorText, plainError } from "../../lib/api";
import { useEditor } from "../../lib/store";
import type { AgentAccess } from "../../lib/types";
import { Button } from "../ui";
import { AskGemini, Command, ConnectRows, GUIDE, GeminiWhy, INSTALL, MAC, TERMINAL, connectAgent, type Run } from "../setup/agentConnect";

/** One numbered step: its number, or a check once done, the title and what it needs now. */
function Step({ n, title, state, done, current, action, children }: { n: number; title: string; state?: string; done: boolean; current: boolean; action?: ReactNode; children?: ReactNode }) {
  return (
    <li className="grid grid-cols-[20px_minmax(0,1fr)] gap-x-2.5">
      <span
        className={`tabular mt-px flex size-5 items-center justify-center rounded-full text-[11px] font-semibold ${done ? "bg-accent-strong text-white" : "bg-white/[.09] text-fg"}`}
        aria-hidden
      >
        {done ? <Check size={12} strokeWidth={3} /> : n}
      </span>
      <div className="min-w-0">
        <div className="flex min-h-5 items-start gap-2">
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className={`text-[13px] leading-5 text-fg ${current || done ? "font-medium" : ""}`}>{title}</span>
            {state && <span className={`text-[12px] leading-4 ${current ? "text-fg" : "text-muted"}`}>{state}</span>}
          </div>
          {action}
        </div>
        {children && <div className="mt-2">{children}</div>}
      </div>
    </li>
  );
}

/**
 * Gemini in the AI panel: it cannot answer here, so the panel says why and leads to it in a terminal: install Google's
 * Antigravity CLI, connect it to Nuzky, ask it there. Each step shows where it stands, read again when the panel shows it
 * and when Nuzky's window comes back to the front, as after installing in a terminal.
 */
export function GeminiGuide() {
  const gemini = useAgent((s) => s.gemini);
  const [access, setAccess] = useState<AgentAccess | null>(null);
  const [run, setRun] = useState<Run | null>(null);
  const [now, setNow] = useState(Date.now());
  const title = useRef<HTMLHeadingElement>(null);
  const ask = useRef<HTMLDivElement>(null);
  const live = useRef(true);
  const update = (next: Run) => live.current && setRun(next);

  const look = () => {
    void loadGemini();
    api.agentAccess().then((a) => live.current && setAccess(a), () => {});
  };
  useEffect(() => {
    live.current = true;
    look();
    // The title, never Connect, so a stray Enter writes nothing.
    title.current?.focus();
    window.addEventListener("focus", look);
    return () => {
      live.current = false;
      window.removeEventListener("focus", look);
    };
  }, []);

  const installed = !!gemini?.installed;
  const state = gemini?.connection.state;
  // An agent connected before is checked again, quietly, so Connected means it reaches Nuzky now.
  useEffect(() => {
    if (!installed || state !== "connected" || !access?.on || run) return;
    api.verifyAgent("antigravity").catch((e) => update({ phase: "failed", fresh: false, wrote: false, error: errorText(e), started: Date.now() }));
  }, [installed, state, access?.on, run]);

  const checking = run?.phase === "writing" || run?.phase === "checking";
  // Connect or Turn on going away leaves the focus nowhere; it goes on to copying agy, the next thing to do.
  useEffect(() => {
    if (run?.phase === "connected" && (!document.activeElement || document.activeElement === document.body)) ask.current?.querySelector("button")?.focus();
  }, [run?.phase]);
  useEffect(() => {
    if (!checking) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [checking]);

  const connect = async () => {
    await connectAgent("antigravity", true, update);
    look();
  };
  const turnOn = async () => {
    try {
      setAccess(await api.setAgentAccess(true, true));
      await connectAgent("antigravity", false, update);
    } catch (e) {
      useEditor.getState().toast({ kind: "error", text: plainError(errorText(e)) });
    }
  };

  const unreadable = state === "unreadable";
  const accessOff = installed && state === "connected" && access !== null && !access.on && !checking;
  const connected = run?.phase === "connected" || (installed && state === "connected" && !!access?.on && !run);
  const connectState = !installed
    ? undefined
    : unreadable
      ? "Its settings can't be read"
      : checking
        ? "Connecting…"
        : run?.phase === "failed"
          ? "Couldn't connect"
          : accessOff
            ? "Connected. Agent access is off"
            : connected
              ? "Connected"
              : "Not connected";
  const small = "h-7 shrink-0 px-3 text-[12px]";

  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-3.5 pb-3 pt-4 text-[13px]">
      <h3 ref={title} tabIndex={-1} className="text-[15px] font-semibold text-fg outline-none">
        Use Gemini from {TERMINAL}
      </h3>
      <div className="mb-4 mt-1.5">
        <GeminiWhy />
      </div>
      {gemini === null ? (
        <p className="text-[12px] text-muted">Looking for Antigravity CLI…</p>
      ) : (
        <ol aria-label="How to use Gemini" className="flex flex-col gap-4">
          <Step
            n={1}
            title="Install Antigravity CLI"
            state={installed ? `Installed · ${gemini.version ?? ""}`.replace(/ · $/, "") : "Not installed"}
            done={installed}
            current={!installed}
            action={
              !installed && (
                <button type="button" onClick={() => void openUrl(GUIDE.antigravity)} className="-mr-2 flex h-7 shrink-0 items-center gap-1 rounded-lg px-2 text-[12px] font-medium text-accent hover:bg-white/[.06]">
                  Install guide <ExternalLink size={12} />
                </button>
              )
            }
          >
            {!installed && (
              <>
                <p className="text-[12px] leading-[18px] text-muted">
                  {MAC ? "Open Terminal: press ⌘ Space, type Terminal and press Return. Paste this there and press Return." : "Open a terminal, paste this and press Enter."}
                </p>
                <Command parts={INSTALL.antigravity} />
                <Button className={`mt-2.5 ${small}`} onClick={look}>
                  <RefreshCw size={13} /> Check again
                </Button>
              </>
            )}
          </Step>
          <Step
            n={2}
            title="Connect it to Nuzky"
            state={connectState}
            done={connected}
            current={installed && !connected}
            action={
              installed &&
              !unreadable &&
              (accessOff ? (
                <Button variant="primary" className={small} aria-label="Turn on Agent access" onClick={() => void turnOn()}>
                  Turn on
                </Button>
              ) : (
                !connected && (
                  <Button variant="primary" className={small} disabled={checking} disabledReason="Connecting…" onClick={() => void connect()}>
                    {run?.phase === "failed" ? "Try again" : "Connect"}
                  </Button>
                )
              ))
            }
          >
            {installed && unreadable && (
              <>
                <p className="text-[12px] leading-[18px] text-muted" title={gemini.connection.problem ?? undefined}>
                  {gemini.connection.path} can't be read, so Nuzky leaves it alone. Fix it, then press Check again.
                </p>
                <Button className={`mt-2.5 ${small}`} onClick={look}>
                  <RefreshCw size={13} /> Check again
                </Button>
              </>
            )}
            {installed && !unreadable && !connected && !run && !accessOff && (
              <p className="text-[12px] leading-[18px] text-muted">
                Connect backs up Gemini's settings and adds Nuzky to them, so Gemini can edit here now and every time Nuzky starts, without asking before each step.
              </p>
            )}
            {accessOff && <p className="text-[12px] leading-[18px] text-muted">Turn on lets Gemini edit here now and every time Nuzky starts.</p>}
            {run && (run.fresh || run.phase === "failed") && <ConnectRows agent="antigravity" name="Gemini" run={run} now={now} />}
          </Step>
          <Step n={3} title={`Ask Gemini in ${TERMINAL}`} done={false} current={connected}>
            {connected && (
              <div ref={ask}>
                <AskGemini agy={gemini.path ?? "agy"} fresh={!!run?.fresh} />
              </div>
            )}
          </Step>
        </ol>
      )}
    </div>
  );
}

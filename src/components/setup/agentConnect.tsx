import { useEffect, useState } from "react";
import { AlertCircle, Check, Copy, Loader2 } from "lucide-react";
import { api, errorText } from "../../lib/api";
import { copyText } from "../../lib/clipboard";
import { useEditor } from "../../lib/store";
import type { AgentKind } from "../../lib/types";
import { IconButton } from "../ui";

export const MAC = navigator.userAgent.includes("Mac");
const WINDOWS = navigator.userAgent.includes("Windows");
/** On Windows the commands below are PowerShell's. */
export const TERMINAL = MAC ? "Terminal" : WINDOWS ? "PowerShell" : "a terminal";
export const RETURN = MAC ? "Return" : "Enter";
/** A terminal on Linux pastes with Ctrl+Shift+V, which someone new to it would not guess. */
export const PASTE = MAC ? "paste this" : WINDOWS ? "paste this (Ctrl+V)" : "paste this (Ctrl+Shift+V)";

export const GUIDE: Record<AgentKind, string> = {
  claudeCode: "https://code.claude.com/docs/en/setup",
  codex: "https://learn.chatgpt.com/docs/codex/cli",
  antigravity: "https://antigravity.google/docs/getting-started",
};
/** The official commands for macOS and Linux, and for PowerShell on Windows, in the parts a narrow column may wrap between. */
export const INSTALL: Record<AgentKind, string[]> = WINDOWS
  ? {
      claudeCode: ["irm", "https://claude.ai/install.ps1", "| iex"],
      codex: ["npm install -g", "@openai/codex"],
      antigravity: ["irm", "https://antigravity.google", "/cli/install.ps1", "| iex"],
    }
  : {
      claudeCode: ["curl -fsSL", "https://claude.ai/install.sh", "| bash"],
      codex: ["curl -fsSL", "https://chatgpt.com/codex/install.sh | sh"],
      antigravity: ["curl -fsSL", "https://antigravity.google", "/cli/install.sh", "| bash"],
    };

/** A Connect (`wrote` once Nuzky is in the agent's settings), or the check of an agent connected before. */
export type Run = { phase: "writing" | "checking" | "connected" | "failed"; fresh: boolean; wrote: boolean; backups?: string[]; error?: string; started: number; took?: number };

/** Connects the agent (`write`) or only checks it, telling `update` each phase; true once the agent reaches Nuzky. */
export async function connectAgent(agent: AgentKind, write: boolean, update: (run: Run) => void): Promise<boolean> {
  let run: Run = { phase: write ? "writing" : "checking", fresh: write, wrote: false, started: Date.now() };
  update(run);
  if (write) {
    try {
      run = { ...run, phase: "checking", wrote: true, backups: (await api.connectAgent(agent)).backups, started: Date.now() };
    } catch (e) {
      update({ ...run, phase: "failed", error: errorText(e) });
      return false;
    }
    update(run);
  }
  try {
    await api.verifyAgent(agent);
    update({ ...run, phase: "connected", took: Date.now() - run.started });
    return true;
  } catch (e) {
    update({ ...run, phase: "failed", error: errorText(e) });
    return false;
  }
}

const ROW_ICON = {
  running: <Loader2 size={14} className="animate-spin text-accent" />,
  done: <Check size={14} className="text-ok" />,
  failed: <AlertCircle size={14} className="mt-px text-danger" />,
};

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

const seconds = (ms: number) => `${Math.max(1, Math.round(ms / 1000))} s`;

/** What a Connect did and what its check does, with the seconds; `now` ticks while it checks. */
export function ConnectRows({ agent, name, run, now }: { agent: AgentKind; name: string; run: Run; now: number }) {
  return (
    <ol aria-label="Steps" className="mt-2.5 flex flex-col border-t border-white/[.06] pt-1.5 text-[12px] leading-4">
      {run.phase === "writing" && <Row icon="running" text={`Adding Nuzky to ${name}'s settings`} />}
      {run.wrote && run.backups?.map((backup) => <Row key={backup} icon="done" text={`Backed up ${backup.replace(/\.nuzky-backup-\d+$/, "")}`} title={`Old settings: ${backup}`} />)}
      {run.wrote && <Row icon="done" text="Added Nuzky and turned on Agent access" />}
      {run.wrote && agent === "antigravity" && <Row icon="done" text="Let Gemini use Nuzky's tools without asking" />}
      {run.phase === "checking" && <Row icon="running" text={`Checking that ${name} sees Nuzky`} detail={seconds(now - run.started)} />}
      {run.phase === "connected" && <Row icon="done" text={`${name} sees Nuzky`} detail={seconds(run.took ?? 0)} />}
      {run.phase === "failed" && <Row icon="failed" text={run.error ?? ""} />}
    </ol>
  );
}

/** A command to paste, which wraps only between its parts, and Copy. A part starting with `/` goes on the one before without a space, so a long URL or path can wrap there. */
export function Command({ parts, label = "Copy the command" }: { parts: string[]; label?: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const id = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(id);
  }, [copied]);
  const joined = (part: string, i: number) => (i > 0 && !part.startsWith("/") ? " " : "");
  const text = parts.map((part, i) => joined(part, i) + part).join("");
  const copy = () =>
    copyText(text, "The clipboard is not available here. Select the command and copy it.").then(
      () => setCopied(true),
      (e: Error) => useEditor.getState().toast({ kind: "error", text: e.message }),
    );
  return (
    <div className="mt-1.5 flex items-start gap-1 rounded-lg bg-black/25 py-1 pl-2.5 pr-1 shadow-[inset_0_0_0_1px_rgb(255_255_255/.08)]">
      <code className="min-w-0 flex-1 select-all py-1 font-mono text-[11.5px] leading-4 text-fg [overflow-wrap:anywhere]">
        {parts.map((part, i) => (
          <span key={i}>
            {joined(part, i) || (i > 0 && <wbr />)}
            <span className="whitespace-nowrap">{part}</span>
          </span>
        ))}
      </code>
      <IconButton label={copied ? "Copied" : label} onClick={() => void copy()} className="h-6 w-6 shrink-0">
        {copied ? <Check size={13} strokeWidth={2.5} className="text-ok" /> : <Copy size={13} />}
      </IconButton>
    </div>
  );
}

/** Why Gemini runs in a terminal and not in the AI panel, with Google's reasons under Details. */
export function GeminiWhy() {
  const [open, setOpen] = useState(false);
  return (
    <div className="text-[12px] leading-[18px] text-muted">
      <p>
        Google doesn't let other apps run Gemini on your Google account, so it can't answer in Nuzky's AI panel. You start it yourself in {TERMINAL}, with Google's Antigravity CLI, and it
        edits the video open in Nuzky, live.
      </p>
      {open ? (
        <p className="mt-1.5">
          Google turned off Gemini CLI for personal Google accounts on 18 June 2026. Antigravity CLI, which replaced it, may not be run by another app: Google's terms call that a breach and
          can suspend the Google account. And unlike ChatGPT, Gemini has no way to let another app use your plan.
        </p>
      ) : (
        <button type="button" onClick={() => setOpen(true)} className="mt-0.5 text-[12px] text-accent hover:underline">
          Details
        </button>
      )}
    </div>
  );
}

const EXAMPLE = "Cut the pauses and retakes in the video open in Nuzky, then add captions";

/** The path as one argument to paste into a shell; `~/` stays outside the quotes, where the shell expands it. PowerShell
 * runs a path in quotes only after `&`. */
function shellPath(path: string) {
  if (WINDOWS) return /^[\w:.\\-]+$/.test(path) ? path : `& '${path.replaceAll("'", "''")}'`;
  if (/^[\w@%+=:,./~-]+$/.test(path)) return path;
  const home = path.startsWith("~/") ? "~/" : "";
  return `${home}'${path.slice(home.length).replaceAll("'", "'\\''")}'`;
}

/** How to ask Gemini once Nuzky is in its settings. `agy` is the path Nuzky found, which works also before a new terminal knows the command. */
export function AskGemini({ agy, fresh }: { agy: string; fresh: boolean }) {
  return (
    <div className="text-[12px] leading-[18px] text-muted">
      <p>
        In {TERMINAL}, {PASTE} and press {RETURN}. The first time, choose Google OAuth and sign in with the Google account that has your Gemini plan; a free
        account works too.
      </p>
      <Command parts={shellPath(agy).split(/(?=\/)/)} />
      <p className="mt-2.5">With a video open in Nuzky, ask it, for example:</p>
      <Command parts={EXAMPLE.split(" ")} label="Copy the example" />
      {fresh && <p className="mt-2.5 text-fg">If Gemini was already running in a terminal, close that window and paste the command in a new one, so it sees Nuzky.</p>}
    </div>
  );
}

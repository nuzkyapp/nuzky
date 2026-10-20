import { useEffect, useRef, useState } from "react";
import { AlertTriangle, Check, X } from "lucide-react";
import { api, errorText } from "../lib/api";
import { copyText } from "../lib/clipboard";
import { useEditor } from "../lib/store";
import type { AgentAccess, AgentConnection, AgentKind, AgentSetup } from "../lib/types";
import { AiLock, Button, Checkbox, IconButton, Switch, trapTab } from "./ui";

const STATE_TEXT: Record<AgentConnection["state"], string> = {
  connected: "Connected",
  other: "Set up for another copy of Nuzky or turned off",
  missing: "Not connected",
  unreadable: "Its settings file could not be read",
};

const COPIES: { setup: AgentSetup; label: string; name: string }[] = [
  { setup: "claude", label: "Copy for Claude", name: "Claude Desktop" },
  { setup: "cursor", label: "Copy for Cursor", name: "Cursor" },
  { setup: "other", label: "Copy MCP config", name: "your agent" },
];

/** Connect your agent: one click writes Nuzky into Claude Code's, Codex's or Antigravity CLI's own settings, a copy serves any other agent, and Agent access decides whether they may edit. */
export function ConnectAgentDialog() {
  const open = useEditor((s) => s.connectOpen);
  const [rows, setRows] = useState<AgentConnection[] | null>(null);
  const [access, setAccess] = useState<AgentAccess | null>(null);
  // Read when the dialog opens, so a copy happens within the click: WebKit copies only from something the user did.
  const [setups, setSetups] = useState<Record<AgentSetup, string> | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<AgentKind | null>(null);
  const dialog = useRef<HTMLDivElement>(null);

  const close = () => useEditor.setState({ connectOpen: false });

  useEffect(() => {
    if (!open) return;
    setError(null);
    setRows(null);
    api.agentConnections().then(setRows, (e) => setError(errorText(e)));
    api.agentAccess().then(setAccess, (e) => setError(errorText(e)));
    Promise.all(COPIES.map((c) => api.agentSetup(c.setup))).then(
      (texts) => setSetups(Object.fromEntries(COPIES.map((c, i) => [c.setup, texts[i]])) as Record<AgentSetup, string>),
      (e) => setError(errorText(e)),
    );
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      close();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open]);

  const ready = open && rows !== null;
  useEffect(() => {
    if (!ready) return;
    // Closing returns focus to the button that opened the dialog.
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.current?.querySelector<HTMLElement>("[data-autofocus]")?.focus();
    return () => opener?.focus();
  }, [ready]);

  // A connected agent's button goes away, so focus moves on to the next Connect, else to Done.
  const refocus = useRef(false);
  useEffect(() => {
    if (!refocus.current) return;
    refocus.current = false;
    dialog.current?.querySelector<HTMLElement>("[data-autofocus]")?.focus();
  }, [rows]);

  if (!open) return null;

  const connect = async (row: AgentConnection) => {
    setBusy(row.agent);
    setError(null);
    try {
      const done = await api.connectAgent(row.agent);
      refocus.current = !!dialog.current?.contains(document.activeElement);
      setRows((all) => all?.map((r) => (r.agent === done.agent ? done : r)) ?? null);
      // Connecting turns Agent access on, now and at every launch.
      setAccess(await api.agentAccess());
      const old = done.backups.length > 1 ? " Your old settings are backed up next to them." : done.backups.length ? ` Your old settings are in ${done.backups[0]}.` : "";
      // agy may not be on a new terminal's PATH yet, so the AI panel's Gemini guide gives the command to paste.
      const next = done.agent === "antigravity" ? "Choose Gemini in the AI panel to see how to ask it in a terminal." : `Restart ${done.name}, then ask it to edit your video.`;
      useEditor.getState().toast({ kind: "success", text: `${done.name} is connected. ${next}${old}` });
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
    }
  };

  const changeAccess = async (on: boolean, atLaunch: boolean) => {
    setError(null);
    try {
      setAccess(await api.setAgentAccess(on, atLaunch));
    } catch (e) {
      setError(errorText(e));
    }
  };

  const copy = async (name: string, text: string) => {
    setError(null);
    try {
      await copyText(text, "The clipboard is not available here.");
      const next = !access?.on ? " Turn on Agent access so it can edit." : !access.atLaunch ? " Check Turn on when Nuzky starts so it can edit after a restart too." : "";
      useEditor.getState().toast({ kind: "success", text: `Copied the setup for ${name}.${next}` });
    } catch (e) {
      setError(errorText(e));
    }
  };

  const first = rows?.find((r) => r.state !== "connected" && r.state !== "unreadable");
  return (
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center scrim-in bg-black/45"
      onPointerDown={(e) => e.target === e.currentTarget && close()}
    >
      <div
        ref={dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby="connect-title"
        onKeyDown={trapTab}
        className="overlay w-[480px] rounded-[20px] p-6 dialog-in"
      >
        <div className="mb-5 flex items-start justify-between gap-3">
          <h2 id="connect-title" className="text-[17px] font-semibold leading-[22px] tracking-[-0.025em]">
            Connect your agent
          </h2>
          <IconButton label="Close" round className="-mr-2 -mt-1" onClick={close}>
            <X size={16} />
          </IconButton>
        </div>
        <div className="flex flex-col gap-3">
          {access && (
            <AiLock>
              <div role="group" aria-label="Agent access" className="flex flex-col gap-2.5 rounded-xl bg-white/[.055] px-3.5 py-3 shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
                <div className="flex items-center gap-3">
                  <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span className="text-[13px] font-medium text-fg">Agent access</span>
                    <span className={`text-[12px] ${access.on ? "text-fg" : "text-muted"}`}>
                      {access.on ? "Agents outside Nuzky can edit the open project" : "Off. Agents outside Nuzky can't edit"}
                    </span>
                  </div>
                  <Switch label="Agent access" checked={access.on} onChange={(on) => void changeAccess(on, access.atLaunch)} />
                </div>
                <Checkbox label="Turn on when Nuzky starts" checked={access.atLaunch} onChange={(atLaunch) => void changeAccess(access.on, atLaunch)} />
              </div>
            </AiLock>
          )}
          {rows === null && !error && <p className="text-[12px] text-muted">Reading the agents' settings…</p>}
          {rows?.map((row) => (
            <div key={row.agent} role="group" aria-label={row.name} className="flex items-center gap-3 rounded-xl bg-white/[.055] px-3.5 py-3 shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="text-[13px] font-medium text-fg">{row.name}</span>
                <span className="truncate text-[11px] text-muted" title={row.path}>
                  {row.path}
                </span>
                <span className={`mt-0.5 flex items-center gap-1 text-[12px] ${row.state === "connected" && access?.on ? "text-fg" : "text-muted"}`} title={row.problem ?? undefined}>
                  {row.state === "connected" && access?.on && <Check size={13} className="shrink-0 text-ok" />}
                  {row.state === "unreadable" && <AlertTriangle size={13} className="shrink-0 text-warn" />}
                  {row.state === "connected" && access && !access.on ? "Connected. Agent access is off" : STATE_TEXT[row.state]}
                </span>
              </div>
              {/* A connected agent needs nothing more here; "Connected" says so. */}
              {row.state !== "connected" && (
                <Button
                  pill
                  variant={row === first ? "primary" : undefined}
                  data-autofocus={row === first || undefined}
                  disabled={row.state === "unreadable" || busy !== null}
                  disabledReason={row.state === "unreadable" ? `Fix ${row.path} first; Nuzky does not change a file it cannot read` : "Connecting…"}
                  onClick={() => void connect(row)}
                >
                  {row.state === "other" ? "Reconnect" : "Connect"}
                </Button>
              )}
            </div>
          ))}
          <div role="group" aria-labelledby="connect-copy" className="mt-1 flex flex-col gap-2">
            <span id="connect-copy" className="text-[12px] text-muted">
              Copy the setup for another agent
            </span>
            <div className="flex flex-wrap gap-2">
              {COPIES.map((c) => (
                <Button key={c.setup} pill disabled={!setups} disabledReason="Reading this app's location…" onClick={() => setups && void copy(c.name, setups[c.setup])}>
                  {c.label}
                </Button>
              ))}
            </div>
          </div>
          {error && (
            <p className="flex items-start gap-1.5 text-[12px] text-fg" role="alert">
              <AlertTriangle size={14} className="mt-px shrink-0 text-danger" />
              {error}
            </p>
          )}
          <p className="text-[12px] text-muted">Connect backs up the agent's settings and adds only Nuzky. For Gemini, it also lets Antigravity CLI use Nuzky's tools without asking.</p>
        </div>
        <div className="mt-6 flex justify-end">
          <Button pill variant={rows && !first ? "primary" : undefined} data-autofocus={!first || undefined} onClick={close}>
            Done
          </Button>
        </div>
      </div>
    </div>
  );
}

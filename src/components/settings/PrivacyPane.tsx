import { useEffect, useRef, useState } from "react";
import { AlertCircle, ChevronRight } from "lucide-react";
import { useAccount } from "../../lib/account";
import { api } from "../../lib/api";
import { useChatGpt } from "../../lib/chatgpt";
import { usePlugins } from "../../lib/plugins";
import { useRemote } from "../../lib/remote";
import { PANE_TITLES, useSettings, type Pane } from "../../lib/settings";
import { deleteTelemetry, setTelemetry, useTelemetry } from "../../lib/telemetry";
import { useUpdates } from "../../lib/updates";
import type { AgentAccess } from "../../lib/types";
import { Button, Switch } from "../ui";

type Row = { title: string; detail: string; state: string; on?: boolean; pane?: Pane; target?: string };

const GROUP = "flex flex-col rounded-xl bg-white/[.055] shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]";
const ROW = "flex min-h-[44px] items-center gap-3 border-t border-white/[.06] px-3.5 py-2.5 first:border-t-0";

/** Help improve Nuzky: the two switches, what was sent, and deleting it from the server. */
function Improve() {
  const status = useTelemetry((s) => s.status);
  const error = useTelemetry((s) => s.error);
  const [confirm, setConfirm] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleted, setDeleted] = useState(false);
  const group = useRef<HTMLDivElement>(null);
  const asked = useRef(false);
  // What was sent meanwhile counts too.
  useEffect(() => void api.telemetryStatus().then((status) => useTelemetry.setState({ status }), () => {}), []);
  // The keyboard goes to Cancel while Delete asks, and back to the button that asked once it is answered.
  useEffect(() => {
    if (confirm) group.current?.querySelector<HTMLElement>("[data-cancel-delete]")?.focus();
    else if (asked.current) group.current?.querySelector<HTMLElement>("[data-delete-sent]")?.focus();
    asked.current = confirm;
  }, [confirm]);
  if (!status) return null;
  const rows = [
    { title: "Problems and usage", detail: "Crashes, errors, which features you use and how fast Nuzky runs. Never your videos, sound, captions or file names.", on: status.problems, set: (on: boolean) => setTelemetry(on, status.conversations) },
    { title: "AI conversations", detail: "What you write in the AI panel, what the AI answers and the tools it calls. Its answers can quote what is said in your videos.", on: status.conversations, set: (on: boolean) => setTelemetry(status.problems, on) },
  ];
  const remove = async () => {
    setDeleting(true);
    if (await deleteTelemetry()) {
      setDeleted(true);
      setConfirm(false);
    }
    setDeleting(false);
  };
  const nothing = status.sent === 0 ? "Nothing was sent yet" : undefined;
  return (
    <div ref={group} data-setting="improve" className="flex flex-col gap-2">
      <h3 className="text-[13px] font-semibold text-fg">Help improve Nuzky</h3>
      <div role="group" aria-label="Help improve Nuzky" className={GROUP}>
        {rows.map((r, i) => (
          <div key={r.title} className={ROW}>
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="text-[13px] font-medium text-fg">{r.title}</span>
              <span id={`improve-row-${i}`} className="text-[12px] text-muted">
                {r.detail}
              </span>
            </span>
            <Switch label={r.title} checked={r.on} describedBy={`improve-row-${i}`} onChange={(on) => void r.set(on).then(() => setDeleted(false))} />
          </div>
        ))}
      </div>
      <p className="text-[12px] text-muted">{deleted ? "Deleted everything this computer sent." : "Sent to account.nuzky.app without your name, email or account."}</p>
      {confirm ? (
        <div role="group" aria-label="Delete what was sent" className="flex items-center gap-2">
          <span className="flex-1 text-[12px] text-fg">Delete everything this computer sent and turn both off?</span>
          <Button pill data-cancel-delete onClick={() => setConfirm(false)}>
            Cancel
          </Button>
          <Button pill variant="danger" disabled={deleting} disabledReason="Deleting…" onClick={() => void remove()}>
            {deleting ? "Deleting…" : "Delete"}
          </Button>
        </div>
      ) : (
        <div className="flex justify-end gap-2">
          <Button pill disabled={!!nothing} disabledReason={nothing} onClick={() => void api.showTelemetry()}>
            Show what was sent…
          </Button>
          <Button pill data-delete-sent disabled={!!nothing} disabledReason={nothing} onClick={() => setConfirm(true)}>
            Delete what was sent…
          </Button>
        </div>
      )}
      {error && (
        <p role="alert" className="flex items-start gap-1.5 text-[12px] leading-[18px] text-fg">
          <AlertCircle size={14} className="mt-0.5 shrink-0 text-danger" />
          {error}
        </p>
      )}
    </div>
  );
}

/**
 * Settings › Privacy: everything that leaves this computer, in one list, and whether it is on now. Each row leads to the
 * pane where it is turned on or off; Help improve Nuzky, which belongs to no other pane, is switched here.
 */
export function PrivacyPane() {
  const updates = useUpdates((s) => s.enabled);
  const auto = useUpdates((s) => s.enabled && s.auto);
  const chatgpt = useChatGpt((s) => s.status);
  const account = useAccount((s) => (s.available ? s.status : undefined));
  const sharing = useRemote((s) => !!s.sharing);
  const full = usePlugins((s) => s.list?.filter((p) => p.fullAccess && p.allowed).length ?? 0);
  const [access, setAccess] = useState<AgentAccess | null>(null);
  useEffect(() => void api.agentAccess().then(setAccess, () => setAccess(null)), []);

  const online: Row[] = [
    updates
      ? { title: "Update check", detail: "GitHub, once a day. Nuzky reads only the newest version number.", state: auto ? "On" : "Off", on: auto, pane: "general", target: "auto-update" }
      : { title: "Update check", detail: "This copy doesn't look for updates.", state: "Not in this copy" },
    { title: "Model downloads", detail: "Only when you click Download. Every file is checked before Nuzky uses it.", state: "When you ask", pane: "models", target: "speech" },
    { title: "Sound library", detail: "Openverse, or Freesound with your key, gets only the words you search for.", state: "When you search", pane: "sounds", target: "freesound" },
    { title: "YouTube", detail: "yt-dlp reaches YouTube only for a link you paste. Nuzky looks for a newer yt-dlp on GitHub once a week.", state: "When you paste a link", pane: "sounds", target: "ytdlp" },
    {
      title: "ChatGPT",
      detail: "OpenAI gets the conversation, Nuzky's guide and what its tools found, while ChatGPT answers in the AI panel.",
      state: chatgpt?.state === "signedIn" ? "Signed in" : "Signed out",
      on: chatgpt?.state === "signedIn",
      pane: "ai",
      target: "chatgpt",
    },
    ...(account === undefined
      ? []
      : [
          {
            title: "Nuzky account",
            detail: "account.nuzky.app, only while you sign in or out and when the sign-in is renewed.",
            state: account?.state === "signedIn" ? "Signed in" : "Signed out",
            on: account?.state === "signedIn",
            pane: "account" as const,
            target: "account",
          },
        ]),
    { title: "Claude Code, Codex and Gemini", detail: "In the AI panel they send the conversation to their own provider, as they do in a terminal.", state: "When you send" },
    { title: "Plugin gallery", detail: "GitHub, only while the Gallery in the Plugins tab is open.", state: "When open" },
    { title: "Edit from your phone", detail: "Only your local network, while sharing is on. Turn it on or off with the QR code button in the top bar.", state: sharing ? "On" : "Off", on: sharing },
  ];
  const local: Row[] = [
    {
      title: "Agent access",
      detail: "Agents outside Nuzky, such as Claude Code or Codex in a terminal, can edit the open project.",
      state: access ? (access.on ? "On" : "Off") : "",
      on: access?.on,
      pane: "ai",
      target: "agent-access",
    },
    { title: "Plugins with full access", detail: "They can read your files, use the internet and run other apps.", state: full ? `${full} allowed` : "None", on: full > 0, pane: "plugins", target: "plugins-access" },
  ];

  return (
    <div data-setting="privacy" className="flex flex-col gap-6">
      <p className="text-[12px] text-muted">Captions, faces, Your style and every edit stay on this computer.</p>
      <Improve />
      <div className="flex flex-col gap-2">
        <h3 className="text-[13px] font-semibold text-fg">Goes online</h3>
        <Rows rows={online} label="Goes online" />
      </div>
      <div className="flex flex-col gap-2">
        <h3 className="text-[13px] font-semibold text-fg">On this computer</h3>
        <Rows rows={local} label="On this computer" />
      </div>
    </div>
  );
}

function Rows({ rows, label }: { rows: Row[]; label: string }) {
  return (
    <ul aria-label={label} className="flex flex-col rounded-xl bg-white/[.055] shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
      {rows.map((r) => {
        const body = (
          <>
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="text-[13px] font-medium text-fg">{r.title}</span>
              <span className="text-[12px] text-muted">{r.detail}</span>
            </span>
            <span className={`shrink-0 text-[12px] ${r.on ? "text-fg" : "text-muted"}`}>{r.state}</span>
            <span className="flex w-4 shrink-0 justify-center">{r.pane && <ChevronRight size={15} className="text-muted" />}</span>
          </>
        );
        const row = "flex min-h-[44px] w-full items-center gap-3 px-3.5 py-2.5 text-left";
        return (
          <li key={r.title} className="border-t border-white/[.06] first:border-t-0 first:[&>button]:rounded-t-xl last:[&>button]:rounded-b-xl">
            {r.pane ? (
              <button
                type="button"
                aria-label={`${r.title}: ${r.state}. Show in ${PANE_TITLES[r.pane]}`}
                onClick={() => useSettings.setState({ pane: r.pane, target: r.target ?? null })}
                className={`${row} transition-colors duration-[120ms] ease-out hover:bg-white/[.04]`}
              >
                {body}
              </button>
            ) : (
              <div className={row}>{body}</div>
            )}
          </li>
        );
      })}
    </ul>
  );
}

import { useEffect, useState } from "react";
import { ChevronRight } from "lucide-react";
import { api } from "../../lib/api";
import { useChatGpt } from "../../lib/chatgpt";
import { usePlugins } from "../../lib/plugins";
import { useRemote } from "../../lib/remote";
import { PANE_TITLES, useSettings, type Pane } from "../../lib/settings";
import { useUpdates } from "../../lib/updates";
import type { AgentAccess } from "../../lib/types";

type Row = { title: string; detail: string; state: string; on?: boolean; pane?: Pane; target?: string };

/**
 * Settings › Privacy: everything that leaves this computer, in one list, and whether it is on now. Each row leads to the
 * pane where it is turned on or off; nothing is switched here, so each setting has one place.
 */
export function PrivacyPane() {
  const auto = useUpdates((s) => s.enabled && s.auto);
  const chatgpt = useChatGpt((s) => s.status);
  const sharing = useRemote((s) => !!s.sharing);
  const full = usePlugins((s) => s.list?.filter((p) => p.fullAccess && p.allowed).length ?? 0);
  const [access, setAccess] = useState<AgentAccess | null>(null);
  useEffect(() => void api.agentAccess().then(setAccess, () => setAccess(null)), []);

  const online: Row[] = [
    { title: "Update check", detail: "GitHub, once a day. Nuzky reads only the newest version number.", state: auto ? "On" : "Off", on: auto, pane: "general", target: "auto-update" },
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
      <p className="text-[13px] text-fg/85">Captions, faces, Your style and every edit stay on this computer. Nuzky goes online only for these:</p>
      <Rows rows={online} label="Online" />
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
          <li key={r.title} className="border-t border-white/[.06] first:border-t-0">
            {r.pane ? (
              <button
                type="button"
                aria-label={`${r.title}: ${r.state}. Show in ${PANE_TITLES[r.pane]}`}
                onClick={() => useSettings.setState({ pane: r.pane, target: r.target ?? null })}
                className={`${row} rounded-xl transition-colors duration-[120ms] ease-out hover:bg-white/[.04]`}
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

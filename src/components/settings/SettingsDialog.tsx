import { useEffect, useRef, useState, type ComponentType, type KeyboardEvent } from "react";
import { HardDrive, Keyboard, Music, Puzzle, Settings, Shield, Sparkles, Subtitles, X } from "lucide-react";
import { ACTIONS, keysOf, useKeys } from "../../lib/keys";
import { PANES, PANE_TITLES, closeSettings, useSettings, type Pane } from "../../lib/settings";
import { useUpdates } from "../../lib/updates";
import { IconButton, SearchField, trapTab } from "../ui";
import { AiPane } from "./AiPane";
import { GeneralPane } from "./GeneralPane";
import { KeysPane } from "./KeysPane";
import { ModelsPane } from "./ModelsPane";
import { PluginsPane } from "./PluginsPane";
import { PrivacyPane } from "./PrivacyPane";
import { SoundsPane } from "./SoundsPane";
import { StoragePane } from "./StoragePane";

const ICONS: Record<Pane, ComponentType<{ size?: number; className?: string }>> = {
  general: Settings,
  ai: Sparkles,
  models: Subtitles,
  sounds: Music,
  plugins: Puzzle,
  keyboard: Keyboard,
  storage: HardDrive,
  privacy: Shield,
};

const BODIES: Record<Pane, ComponentType> = {
  general: GeneralPane,
  ai: AiPane,
  models: ModelsPane,
  sounds: SoundsPane,
  plugins: PluginsPane,
  keyboard: KeysPane,
  storage: StoragePane,
  privacy: PrivacyPane,
};

/** What search finds besides the keys: a setting's name, the pane and row it is on, and other words people use for it. */
const ENTRIES: { label: string; pane: Pane; target: string; words?: string }[] = [
  { label: "Check for updates", pane: "general", target: "updates", words: "version update new" },
  { label: "Check for updates automatically", pane: "general", target: "auto-update", words: "daily" },
  { label: "Get set up", pane: "general", target: "setup", words: "start models download" },
  { label: "Take the tour", pane: "general", target: "tour", words: "sample video help learn" },
  { label: "ChatGPT account", pane: "ai", target: "chatgpt", words: "openai sign in out plan usage" },
  { label: "Agent access", pane: "ai", target: "agent-access", words: "mcp agents outside edit" },
  { label: "Connect Claude Code, Codex or Gemini", pane: "ai", target: "agents", words: "agent cursor opencode claude desktop mcp setup copy" },
  { label: "Learn from my edits", pane: "ai", target: "learn", words: "your style export" },
  { label: "Spoken language", pane: "models", target: "speech", words: "captions transcript whisper language" },
  { label: "Captions model", pane: "models", target: "speech", words: "fast balanced most accurate whisper transcript remove delete" },
  { label: "Face and person models", pane: "models", target: "vision", words: "faces people cover blur reframe" },
  { label: "Freesound key", pane: "sounds", target: "freesound", words: "api key sound effects search" },
  { label: "yt-dlp", pane: "sounds", target: "ytdlp", words: "youtube music download link" },
  { label: "Run plugins automatically", pane: "plugins", target: "plugins-auto", words: "after import before export" },
  { label: "Plugins with full access", pane: "plugins", target: "plugins-access", words: "revoke allow files internet" },
  { label: "Keys like another editor", pane: "keyboard", target: "keys-preset", words: "premiere final cut davinci resolve shortcuts" },
  { label: "Import keys from a file", pane: "keyboard", target: "keys-preset", words: "kys commandset shortcuts" },
  { label: "Storage", pane: "storage", target: "storage", words: "disk space cache clear free size" },
  { label: "Privacy", pane: "privacy", target: "privacy", words: "network internet online data" },
];

const fold = (text: string) =>
  text
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "")
    .toLowerCase();

type Result = { label: string; pane: Pane; target: string; keys?: string[] };

function search(query: string, updates: boolean): Result[] {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  const hit = (text: string) => words.every((w) => fold(text).includes(w));
  // A copy that doesn't update itself has no update settings to find.
  const settings = ENTRIES.filter((e) => (updates || (e.target !== "updates" && e.target !== "auto-update")) && hit(`${e.label} ${PANE_TITLES[e.pane]} ${e.words ?? ""}`));
  const keys = ACTIONS.filter((a) => hit(`${a.label} ${keysOf(a.id).join(" ")} shortcut key`)).map((a) => ({
    label: a.label,
    pane: "keyboard" as const,
    target: `key:${a.id}`,
    keys: keysOf(a.id),
  }));
  return [...settings, ...keys];
}

const selectorOf = (target: string) => (target.startsWith("key:") ? `[data-action="${target.slice(4)}"]` : `[data-setting="${target}"]`);

export function SettingsDialog() {
  const open = useSettings((s) => s.open);
  return open ? <Dialog /> : null;
}

function Dialog() {
  const pane = useSettings((s) => s.pane);
  const target = useSettings((s) => s.target);
  useKeys();
  const [query, setQuery] = useState("");
  const [inField, setInField] = useState(false);
  const updates = useUpdates((s) => s.enabled);
  const dialog = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const results = search(query, updates);
  const Body = BODIES[pane];

  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    // The pane list takes the keyboard, so the arrows move between panes at once; typing there searches.
    dialog.current?.querySelector<HTMLElement>("[role=tab][aria-selected=true]")?.focus();
    // A button that goes away, such as Cancel once a sign-in ends, leaves the keyboard on the page; Esc still closes.
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== "Escape" || document.activeElement !== document.body) return;
      e.stopPropagation();
      closeSettings();
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      opener?.focus();
    };
  }, []);

  // A row asked for, by search or by a button elsewhere, scrolls into view and lights up for a moment once it shows.
  useEffect(() => {
    if (!target) return;
    let tries = 0;
    let timer = 0;
    const find = () => {
      const row = body.current?.querySelector<HTMLElement>(selectorOf(target));
      if (!row) {
        // Some rows show once what they read has loaded.
        if (++tries < 20) timer = window.setTimeout(find, 50);
        return;
      }
      row.scrollIntoView({ block: "nearest" });
      // The keyboard goes to the row's main control; one that shows later takes it from the pane (ChatGptSection).
      const control = row.querySelector<HTMLElement>("[data-autofocus], button:not([aria-disabled=true]), input, [role=switch]");
      (control ?? body.current)?.focus();
      // The same light as a part of Your style that just changed.
      row.classList.remove("changed");
      void row.offsetWidth;
      row.classList.add("changed");
      useSettings.setState({ target: null });
    };
    find();
    return () => window.clearTimeout(timer);
  }, [target, pane]);

  const show = (next: Pane, at: string | null = null) => {
    useSettings.setState({ pane: next, target: at });
    body.current?.scrollTo({ top: 0 });
  };

  const choose = (r: Result) => {
    setQuery("");
    show(r.pane, r.target);
  };

  const railKeys = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key.length === 1 && e.key !== " " && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      setQuery(e.key);
      dialog.current?.querySelector<HTMLElement>("input[type=search]")?.focus();
      return;
    }
    const i = PANES.indexOf(pane);
    const to = { ArrowUp: i - 1, ArrowDown: i + 1, Home: 0, End: PANES.length - 1 }[e.key];
    if (to === undefined) return;
    e.preventDefault();
    const next = PANES[Math.max(0, Math.min(PANES.length - 1, to))];
    show(next);
    e.currentTarget.querySelector<HTMLElement>(`#settings-tab-${next}`)?.focus();
  };

  const resultKeys = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const rows = [...e.currentTarget.querySelectorAll<HTMLElement>("button")];
    const i = rows.indexOf(document.activeElement as HTMLElement);
    if (e.key === "ArrowUp" && i <= 0) return dialog.current?.querySelector<HTMLElement>("input[type=search]")?.focus();
    rows[Math.max(0, Math.min(rows.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))]?.focus();
  };

  return (
    <div
      data-scrim
      data-exit
      className="scrim-in fixed inset-0 z-[100] flex items-center justify-center bg-black/55"
      onPointerDown={(e) => {
        if (e.target !== e.currentTarget) return;
        e.preventDefault();
        closeSettings();
      }}
    >
      <div
        ref={dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-title"
        data-settings
        tabIndex={-1}
        onKeyDown={(e) => {
          // The editor's and the home screen's keys wait while Settings is open; F8 still reaches the toasts.
          if (e.key !== "F8") e.stopPropagation();
          if (e.key === "Escape") {
            e.preventDefault();
            closeSettings();
          } else trapTab(e);
        }}
        className="dialog-in overlay overlay-solid flex h-[640px] max-h-[calc(100vh-48px)] w-[880px] max-w-[calc(100vw-48px)] overflow-hidden rounded-[20px] text-fg outline-none"
      >
        <div className="flex w-[220px] shrink-0 flex-col gap-3 border-r border-white/[.06] bg-black/[.12] p-3 pt-4">
          <div className="flex shrink-0" onFocus={() => setInField(true)} onBlur={() => setInField(false)}>
            <SearchField
              value={query}
              onChange={setQuery}
              placeholder="Search settings"
              onEnter={() => results[0] && choose(results[0])}
              onDown={() => dialog.current?.querySelector<HTMLElement>("[data-results] button")?.focus()}
            />
          </div>
          {query.trim() ? (
            <div data-results role="list" aria-label="Search results" onKeyDown={resultKeys} className="-mx-1 flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-1">
              <p role="status" className="px-2.5 pb-1 text-[11px] text-muted">
                {results.length === 0 ? "" : results.length === 1 ? "1 result" : `${results.length} results`}
              </p>
              {results.length === 0 && (
                <p className="px-2.5 text-[12px] text-muted">
                  No settings match “{query.trim()}”. Try a word from a setting's name, like “captions” or “update”.
                </p>
              )}
              {results.map((r, i) => (
                <button
                  key={`${r.pane}:${r.target}:${r.label}`}
                  type="button"
                  role="listitem"
                  // Enter in the field takes the first result, so it shows as chosen while the field has the keyboard.
                  aria-current={inField && i === 0 ? "true" : undefined}
                  onClick={() => choose(r)}
                  className={`flex flex-col items-start rounded-lg px-2.5 py-1.5 text-left transition-colors duration-[120ms] ease-out hover:bg-white/[.06] focus-visible:bg-white/[.08] ${inField && i === 0 ? "bg-white/[.06]" : ""}`}
                >
                  <span className="flex w-full items-center gap-2 text-[13px] text-fg">
                    <span className="min-w-0 flex-1 truncate">{r.label}</span>
                    {r.keys?.[0] && <kbd className="shrink-0 rounded-[5px] bg-white/[.08] px-1.5 font-sans text-[11px] font-medium leading-[18px] text-fg/90">{r.keys[0]}</kbd>}
                  </span>
                  <span className="text-[11px] text-muted">{PANE_TITLES[r.pane]}</span>
                </button>
              ))}
            </div>
          ) : (
            <div role="tablist" aria-orientation="vertical" aria-label="Settings" onKeyDown={railKeys} className="-mx-1 flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-1">
              {PANES.map((id) => {
                const Icon = ICONS[id];
                const on = id === pane;
                return (
                  <button
                    key={id}
                    type="button"
                    role="tab"
                    id={`settings-tab-${id}`}
                    aria-selected={on}
                    aria-controls="settings-panel"
                    tabIndex={on ? 0 : -1}
                    onClick={(e) => {
                      show(id);
                      // WebKit leaves the keyboard where it was on a click, so the ring would stay on the pane before.
                      e.currentTarget.focus();
                    }}
                    className={`flex h-8 shrink-0 items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] transition-colors duration-[120ms] ease-out ${on ? "bg-white/[.1] font-medium text-fg" : "text-fg hover:bg-white/[.05]"} ${id === "storage" ? "mt-3" : ""}`}
                  >
                    <Icon size={16} className={on ? "text-accent" : "text-muted"} />
                    {PANE_TITLES[id]}
                  </button>
                );
              })}
            </div>
          )}
        </div>
        <div role="tabpanel" id="settings-panel" aria-labelledby={`settings-tab-${pane}`} className="flex min-w-0 flex-1 flex-col">
          <div className="flex items-center justify-between gap-3 px-6 pt-5 pb-4">
            <h2 id="settings-title" className="text-[17px] font-semibold leading-[22px] tracking-[-0.025em]">
              {PANE_TITLES[pane]}
            </h2>
            <IconButton round label="Close" onClick={closeSettings} className="h-7 w-7 bg-white/[.07]">
              <X size={15} />
            </IconButton>
          </div>
          <div ref={body} tabIndex={-1} data-settings-body className="scroll-shown flex min-h-0 flex-1 flex-col overflow-y-auto pb-6 pl-6 pr-3.5 [scrollbar-gutter:stable] outline-none">
            <Body key={pane} />
          </div>
        </div>
      </div>
    </div>
  );
}

import { Ellipsis, Eye, EyeOff, Loader2, Plus } from "lucide-react";
import { useEffect, useId, useState } from "react";
import { agentName, loadAgents, stop, translateBlocked, translateCaptions, useAgent } from "../../lib/agent";
import { api } from "../../lib/api";
import { useChatGpt } from "../../lib/chatgpt";
import { languageName } from "../../lib/speech";
import { undoAction, useEditor } from "../../lib/store";
import type { Track } from "../../lib/types";
import { Button, IconButton, LockReason, Menu, useLockReason } from "../ui";

/** The one language the Captions tab offers to translate into. */
export const TRANSLATE_TO = "en";
/** A translation's size beside the speech captions', as Nuzky first makes it (`TRANSLATION_SIZE` in crates/mcp/src/translate.rs). */
export const TRANSLATION_SIZE = 0.7;

const captions = (n: number) => `${n} ${n === 1 ? "caption" : "captions"}`;

/** The name of the language the speech was recognised in, asked again when its captions change; null before any. */
export function useSpeechLanguage(speech: Track | undefined): string | null {
  const [language, setLanguage] = useState<string | null>(null);
  const clips = speech?.clips.length;
  useEffect(() => {
    if (clips === undefined) return;
    let live = true;
    api.speechLanguage().then(
      (l) => live && setLanguage(l),
      () => live && setLanguage(null),
    );
    return () => {
      live = false;
    };
  }, [speech?.id, clips]);
  return language && languageName(language);
}

/**
 * The captions tracks, one row each: the speech's and its translations. A row's eye shows or hides its track, and
 * picking a row picks the captions the controls below style. The empty row asks the AI to translate.
 */
export function CaptionLanguages({
  speech,
  language,
  translations,
  editing,
  onEdit,
}: {
  speech: Track;
  /** The speech's language by name, null when it is not known. */
  language: string | null;
  translations: Track[];
  editing: string;
  onEdit: (trackId: string) => void;
}) {
  const translating = useAgent((s) => (s.status !== "idle" ? s.translating : null));
  const agent = useAgent((s) => s.agent);
  const missing = !translations.some((t) => t.translation === TRANSLATE_TO);
  const order = [speech.id, ...translations.map((t) => t.id)];
  // ↑ and ↓ pick the row before or after, as in any group of radio buttons, and take the focus with them.
  const step = (from: string, by: number) => {
    const to = order[(order.indexOf(from) + by + order.length) % order.length];
    onEdit(to);
    window.setTimeout(() => document.querySelector<HTMLElement>(`[data-caption-track="${to}"]`)?.focus(), 0);
  };
  const row = { onEdit, step, menus: translations.length > 0 };
  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-[12px] text-muted">Languages</span>
      <div role="radiogroup" aria-label="Captions to style" className="flex flex-col gap-1.5">
        <Row track={speech} name={language ?? "Captions"} detail={language ? `From speech · ${captions(speech.clips.length)}` : captions(speech.clips.length)} on={editing === speech.id} {...row} />
        {translations.map((t) => (
          <Row key={t.id} track={t} name={languageName(t.translation ?? "")} detail={`Translated · ${captions(t.clips.length)}`} on={editing === t.id} {...row} />
        ))}
      </div>
      {translating ? (
        <div className="flex h-11 items-center gap-2.5 rounded-[10px] bg-white/[.05] pr-1.5 pl-3" role="status" data-translating>
          <Loader2 size={14} aria-hidden className="shrink-0 animate-spin text-accent" />
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="truncate text-[13px] text-fg">Translating to {languageName(translating)}</span>
            <span className="truncate text-[11px] text-muted">{agentName(agent)} is working</span>
          </span>
          {/* Stop works while the AI's run locks everything else. */}
          <LockReason.Provider value={null}>
            <Button className="h-7 px-2.5 text-[12px]" onClick={() => void stop()}>
              Stop
            </Button>
          </LockReason.Provider>
        </div>
      ) : (
        missing && <AddTranslation />
      )}
    </div>
  );
}

function Row({
  track,
  name,
  detail,
  on,
  onEdit,
  step,
  menus,
}: {
  track: Track;
  name: string;
  detail: string;
  on: boolean;
  onEdit: (trackId: string) => void;
  step: (from: string, by: number) => void;
  /** A row of the list has a … button, so every row keeps its column and the eyes line up. */
  menus: boolean;
}) {
  const lock = useLockReason();
  const { edit } = useEditor.getState();
  const [menu, setMenu] = useState<{ x: number; y: number; keyboard: boolean } | null>(null);
  const translated = !!track.translation;
  const remove = async () => {
    const snap = await edit({ type: "deleteClips", clipIds: track.clips.map((c) => c.id) });
    if (snap) useEditor.getState().toast({ kind: "info", text: `Deleted the ${name} captions`, action: undoAction(snap) });
  };
  return (
    <div
      role="radio"
      aria-checked={on}
      tabIndex={on ? 0 : -1}
      data-caption-track={track.id}
      aria-label={`${name} captions`}
      title={on ? undefined : `Style the ${name} captions`}
      onClick={() => onEdit(track.id)}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return;
        if (e.key === " " || e.key === "Enter") {
          e.preventDefault();
          onEdit(track.id);
        } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
          e.preventDefault();
          step(track.id, e.key === "ArrowDown" ? 1 : -1);
        }
      }}
      className={`flex h-11 cursor-default items-center gap-1 rounded-[10px] border pr-1 pl-3 transition-colors duration-[120ms] ease-out focus-visible:outline-fg focus-visible:outline-offset-2 ${
        on ? "border-accent bg-accent/[.08] shadow-[0_0_0_1px_var(--color-accent)]" : "border-white/[.08] bg-white/[.03] hover:border-white/30"
      }`}
    >
      <span className="flex min-w-0 flex-1 flex-col">
        <span className={`truncate text-[13px] font-medium ${track.hidden ? "text-muted" : "text-fg"}`}>{name}</span>
        <span className="truncate text-[11px] text-muted">{track.hidden ? `Hidden · ${detail}` : detail}</span>
      </span>
      <IconButton
        label={track.hidden ? `Show the ${name} captions` : `Hide the ${name} captions`}
        className="h-7 w-7"
        onClick={(e) => {
          e.stopPropagation();
          void edit({ type: "updateTrack", trackId: track.id, hidden: !track.hidden });
        }}
      >
        {track.hidden ? <EyeOff size={14} /> : <Eye size={14} />}
      </IconButton>
      {!translated && menus && <span aria-hidden className="w-7 shrink-0" />}
      {translated && (
        <IconButton
          label={`More for the ${name} captions`}
          className="h-7 w-7"
          onClick={(e) => {
            e.stopPropagation();
            const r = e.currentTarget.getBoundingClientRect();
            setMenu({ x: r.left, y: r.bottom + 4, keyboard: e.detail === 0 });
          }}
        >
          <Ellipsis size={14} />
        </IconButton>
      )}
      {menu && !lock && (
        <Menu
          label={`${name} captions`}
          at={menu}
          keyboard={menu.keyboard}
          onClose={() => setMenu(null)}
          items={[
            { label: "Translate again with AI", disabled: translateBlocked(), run: () => translateCaptions(track.translation ?? TRANSLATE_TO) },
            { label: `Delete the ${name} captions`, danger: true, run: () => void remove() },
          ]}
        />
      )}
    </div>
  );
}

function AddTranslation() {
  const lock = useLockReason();
  const reasonId = useId();
  useEffect(() => {
    if (useAgent.getState().agents === null) void loadAgents();
  }, []);
  // Whether an AI can take it changes with the agent picked, its answer and signing in to ChatGPT.
  useAgent((s) => s.status);
  useAgent((s) => s.agent);
  useAgent((s) => s.agents);
  useAgent((s) => s.agentsError);
  useChatGpt((s) => s.status);
  const blocked = lock ?? translateBlocked();
  const name = languageName(TRANSLATE_TO);
  return (
    // As tall as the row that takes its place while the AI translates, so nothing under it moves. Off, it says why.
    <button
      type="button"
      data-translate-captions
      aria-disabled={blocked ? true : undefined}
      aria-describedby={blocked ? reasonId : undefined}
      title={blocked ?? undefined}
      onClick={blocked ? undefined : () => translateCaptions(TRANSLATE_TO)}
      className="flex h-11 min-w-0 flex-col items-center justify-center rounded-[10px] border border-dashed border-white/[.15] px-3 transition-colors duration-[120ms] hover:border-white/30 aria-disabled:cursor-not-allowed aria-disabled:hover:border-white/[.15]"
    >
      <span className={`flex items-center gap-1.5 text-[12px] ${blocked ? "text-muted" : "text-fg"}`}>
        <Plus size={13} aria-hidden /> Add {name} captions
      </span>
      {blocked && (
        <span id={reasonId} className="max-w-full truncate text-[11px] text-muted">
          {blocked}
        </span>
      )}
    </button>
  );
}

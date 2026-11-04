import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { MonitorPlay, X } from "lucide-react";
import { api, errorText } from "../../lib/api";
import { aiLocked, enqueue, undoAction, useEditor } from "../../lib/store";
import { formatDuration } from "../../lib/time";
import type { BrollSuggestions, SuggestedBroll, TranscriptView } from "../../lib/types";
import { Button, IconButton } from "../ui";

export interface BrollSuggestionState {
  /** Suggestions for the current words, or null when none are shown. */
  items: SuggestedBroll[] | null;
  pending: "suggesting" | "adding" | null;
  suggest: () => Promise<void>;
  /** Adds these suggestions as one undo step; resolves to whether they were added. */
  add: (items: SuggestedBroll[]) => Promise<boolean>;
  dismiss: (item: SuggestedBroll | null) => void;
}

/**
 * Screen recordings to show where the speaker names the app they show. Like suggested zooms, they belong to the words
 * they were made for and go when the speech changes.
 */
export function useBrollSuggestions(view: TranscriptView | null): BrollSuggestionState {
  const [found, setFound] = useState<BrollSuggestions | null>(null);
  const [pending, setPending] = useState<BrollSuggestionState["pending"]>(null);
  const key = view?.key;
  useEffect(() => setFound((f) => (f && f.key === key ? f : null)), [key]);

  const suggest = async () => {
    setPending("suggesting");
    try {
      const next = await api.suggestBroll();
      if (next.items.length > 0) setFound(next);
      else {
        setFound(null);
        const text =
          next.recordings === 0
            ? "No screen recording in this project yet. Record your screen in Media, then suggest B-roll again."
            : "No recording here is named after an app or tool you mention.";
        useEditor.getState().toast({ kind: "info", text });
      }
    } catch (e) {
      useEditor.getState().toast({ kind: "error", text: errorText(e) });
    } finally {
      setPending(null);
    }
  };

  const add = async (items: SuggestedBroll[]) => {
    if (!found || aiLocked() || items.length === 0) return false;
    setPending("adding");
    try {
      const choices = items.map(({ from, to, assetId }) => ({ from, to, assetId }));
      const key = found.key;
      const snap = await enqueue((epoch) => api.addBroll(key, choices, epoch));
      if (!snap) return false;
      const text = `Added ${items.length === 1 ? items[0].assetName : `${items.length} recordings`} over the video`;
      useEditor.getState().toast({ kind: "info", text, action: undoAction(snap) });
      setFound((f) => {
        const left = f?.items.filter((i) => !items.includes(i)) ?? [];
        return f && left.length ? { ...f, items: left } : null;
      });
      return true;
    } finally {
      setPending(null);
    }
  };

  const dismiss = (item: SuggestedBroll | null) =>
    setFound((f) => {
      const left = item ? (f?.items.filter((i) => i !== item) ?? []) : [];
      return f && left.length ? { ...f, items: left } : null;
    });

  return { items: found && found.key === key ? found.items : null, pending, suggest, add, dismiss };
}

export function SuggestBrollButton({ state, blocker }: { state: BrollSuggestionState; blocker: string | null }) {
  return (
    <Button
      className="h-7 px-2"
      data-suggest-broll
      disabled={!!blocker || state.pending === "suggesting"}
      disabledReason={blocker ?? "Finding recordings…"}
      title="Show a screen recording over the video where you name the app it shows"
      onClick={state.suggest}
    >
      <MonitorPlay size={14} /> B-roll
    </Button>
  );
}

/**
 * The suggestions above the text: each sentence with the recording to show over it, Add or ×, and Add all. Clicking a
 * sentence moves the playhead there. Esc dismisses them all and focus returns to the B-roll button.
 */
export function BrollBar({ state, blocker }: { state: BrollSuggestionState; blocker: string | null }) {
  const bar = useRef<HTMLDivElement>(null);
  const items = state.items;
  const shown = !!items;
  useEffect(() => {
    if (shown) bar.current?.querySelector<HTMLElement>("[data-broll-add]")?.focus();
  }, [shown]);
  if (!items) return null;
  const back = () => document.querySelector<HTMLElement>("[data-suggest-broll]")?.focus();
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "Escape") return;
    e.stopPropagation();
    state.dismiss(null);
    back();
  };
  const busy = state.pending === "adding";
  return (
    <div ref={bar} role="region" aria-label="Suggested B-roll" data-broll-bar className="flex shrink-0 flex-col gap-1 border-b border-white/[.07] px-3.5 py-2" onKeyDown={onKeyDown}>
      <div className="flex items-center gap-2">
        <MonitorPlay size={14} className="shrink-0 text-accent" aria-hidden />
        <span className="tabular flex-1 truncate text-[12px] text-muted">
          {items.length} place{items.length === 1 ? "" : "s"} to show a recording
        </span>
        <Button
          className="h-7 px-2 text-[12px]"
          onClick={() => {
            state.dismiss(null);
            back();
          }}
        >
          Dismiss
        </Button>
        {items.length > 1 && (
          <Button variant="primary" className="h-7 px-2.5 text-[12px]" disabled={!!blocker || busy} disabledReason={blocker ?? "Adding…"} onClick={() => void state.add(items)}>
            Add all
          </Button>
        )}
      </div>
      <ul className="flex max-h-[180px] flex-col gap-1 overflow-y-auto">
        {items.map((item) => (
          <li key={`${item.from}-${item.assetId}`} className="flex items-center gap-2 rounded-lg bg-white/[.04] py-1 pr-1 pl-2" data-broll-item>
            <button type="button" className="min-w-0 flex-1 text-left" title="Go to this sentence" onClick={() => useEditor.getState().seek(item.startUs)}>
              <span className="block truncate text-[12px] text-fg">“{item.text}”</span>
              <span className="tabular block truncate text-[11px] text-muted">
                {item.assetName} · {formatDuration(item.durationUs)}
                {item.durationUs < item.endUs - item.startUs && ` of ${formatDuration(item.endUs - item.startUs)} said: the recording ends first`}
              </span>
            </button>
            <Button data-broll-add className="h-7 shrink-0 px-2.5 text-[12px]" disabled={!!blocker || busy} disabledReason={blocker ?? "Adding…"} title={`Show ${item.assetName} over the whole video while you say this`} onClick={() => void state.add([item])}>
              Add
            </Button>
            <IconButton label="Dismiss this suggestion" className="h-7 w-7 shrink-0" onClick={() => state.dismiss(item)}>
              <X size={14} />
            </IconButton>
          </li>
        ))}
      </ul>
    </div>
  );
}

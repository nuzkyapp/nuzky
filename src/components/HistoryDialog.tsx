import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { AlertCircle, Loader2, X } from "lucide-react";
import { api, errorText, plainError } from "../lib/api";
import { clockLabel, dayLabel, lineDiff, useHistory, type HistoryKind } from "../lib/history";
import { useEditor } from "../lib/store";
import type { CaptionChange, RevisionView, SyncRevision } from "../lib/types";
import { Button, IconButton, Segmented, trapTab } from "./ui";

type Kind = Exclude<HistoryKind, "all">;
type Entry = { kind: Kind; rev: SyncRevision };

const KINDS: Kind[] = ["style", "caption-styles"];
const NAME: Record<Kind, string> = { style: "Your style", "caption-styles": "Caption styles" };
const TABS = [
  { id: "all" as const, label: "All" },
  { id: "style" as const, label: "Your style" },
  { id: "caption-styles" as const, label: "Caption styles" },
];
const CHANGE: Record<CaptionChange, { text: string; className: string }> = {
  back: { text: "Comes back", className: "text-ok" },
  goes: { text: "Goes away", className: "text-danger" },
  changes: { text: "Changes", className: "text-fg" },
  same: { text: "No change", className: "text-muted" },
};
const EMPTY: Record<HistoryKind, string> = {
  all: "Nothing has synced yet. Changes show here once they sync.",
  style: "Your style has not synced yet.",
  "caption-styles": "No caption styles have synced yet.",
};

const key = (e: Entry) => `${e.kind}:${e.rev.revision}`;
const where = (rev: SyncRevision) => `${rev.computer}${rev.mine ? ", this computer" : ""}`;
const when = (ms: number) => {
  const day = dayLabel(ms);
  return `${day === "Today" || day === "Yesterday" ? day.toLowerCase() : day} ${clockLabel(ms)}`;
};
const backAgain = (kind: Kind, ms: number) =>
  kind === "style" ? `Your style is back as it was ${when(ms)}.` : `Your caption styles are back as they were ${when(ms)}.`;

/** Whether restoring the version would change nothing: it is the same as now. */
function unchanged(view: RevisionView, kind: Kind) {
  if (kind === "style") return view.text === view.nowText;
  return view.captionStyles.every(([, change]) => change === "same");
}

export function HistoryDialog() {
  const open = useHistory((s) => s.open);
  return open ? <Dialog /> : null;
}

/** Every revision of Your style and My caption styles the account keeps, what restoring one would change, and Restore. */
function Dialog() {
  const [tab, setTab] = useState<HistoryKind>(useHistory.getState().kind);
  const [entries, setEntries] = useState<Entry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [preview, setPreview] = useState<{ key: string; view: RevisionView | null; error: string | null } | null>(null);
  const [restoring, setRestoring] = useState(false);
  const [restoreError, setRestoreError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [previewAttempt, setPreviewAttempt] = useState(0);
  const dialog = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const well = useRef<HTMLDivElement>(null);

  const close = () => useHistory.setState({ open: false });

  useEffect(() => {
    let live = true;
    setEntries(null);
    setError(null);
    Promise.all(KINDS.map((kind) => api.syncHistory(kind).then((revs) => revs.map((rev) => ({ kind, rev })))))
      .then((lists) => live && setEntries(lists.flat().sort((a, b) => b.rev.atMs - a.rev.atMs || b.rev.revision - a.rev.revision)))
      .catch((e) => live && setError(plainError(errorText(e))));
    return () => {
      live = false;
    };
  }, [attempt]);

  const shown = (entries ?? []).filter((e) => tab === "all" || e.kind === tab);
  const newest = Object.fromEntries(KINDS.map((kind) => [kind, Math.max(0, ...(entries ?? []).filter((e) => e.kind === kind).map((e) => e.rev.revision))]));
  const isNow = (e: Entry) => e.rev.revision === newest[e.kind];
  const chosen = shown.find((e) => key(e) === selected) ?? shown.find((e) => !isNow(e)) ?? shown[0] ?? null;
  const chosenKey = chosen ? key(chosen) : null;

  // The keyboard starts on the dialog, then on the chosen version once the list shows, so ↑ and ↓ move through it.
  useEffect(() => {
    dialog.current?.focus();
  }, []);
  const listed = entries !== null;
  useEffect(() => {
    if (listed && chosenKey && (document.activeElement === dialog.current || !dialog.current?.contains(document.activeElement)))
      list.current?.querySelector<HTMLElement>(`[data-entry="${chosenKey}"]`)?.focus();
  }, [listed, chosenKey]);

  useEffect(() => {
    if (!chosenKey) return;
    let live = true;
    const [kind, revision] = chosenKey.split(":") as [Kind, string];
    setPreview({ key: chosenKey, view: null, error: null });
    setRestoreError(null);
    api.syncRevision(kind, Number(revision)).then(
      (view) => live && setPreview({ key: chosenKey, view, error: null }),
      (e) => live && setPreview({ key: chosenKey, view: null, error: plainError(errorText(e)) }),
    );
    return () => {
      live = false;
    };
  }, [chosenKey, previewAttempt]);

  const view = preview && preview.key === chosenKey ? preview : null;
  const same = !!(chosen && view?.view && unchanged(view.view, chosen.kind));

  // The first line that changes, in the middle of the well, as a long EDIT.md may change far down.
  useEffect(() => {
    well.current?.querySelector<HTMLElement>("[data-diff=back], [data-diff=goes]")?.scrollIntoView({ block: "center" });
  }, [view?.view]);

  const restore = async () => {
    if (!chosen) return;
    const { kind, rev } = chosen;
    const was = newest[kind];
    setRestoring(true);
    setRestoreError(null);
    try {
      await api.syncRestore(kind, rev.revision);
      useEditor.getState().toast({
        kind: "success",
        text: `${backAgain(kind, rev.atMs)} Your other computers get it too.`,
        action: { label: "Undo", run: () => void api.syncRestore(kind, was).catch((e) => useEditor.getState().toast({ kind: "error", text: errorText(e) })) },
      });
      close();
    } catch (e) {
      const text = errorText(e);
      setRestoreError(plainError(text));
      setRestoring(false);
      if (text.startsWith("RESTORE_CHANGED")) setPreviewAttempt((n) => n + 1);
    }
  };

  /** ↑ and ↓ move through the list, as in a list of files. */
  const onListKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const at = shown.findIndex((entry) => key(entry) === chosenKey);
    const next = shown[Math.max(0, Math.min(shown.length - 1, at + (e.key === "ArrowDown" ? 1 : -1)))];
    if (!next) return;
    setSelected(key(next));
    list.current?.querySelector<HTMLElement>(`[data-entry="${key(next)}"]`)?.focus();
  };

  const reason = !chosen
    ? ""
    : isNow(chosen) || same
      ? "This is how it is now"
      : restoring
        ? "Restoring…"
        : view?.error
          ? "This version could not be loaded"
          : "Loading this version…";
  let lastDay = "";
  return (
    <div data-scrim data-exit className="fixed inset-0 z-[100] flex items-center justify-center scrim-in bg-black/55" onPointerDown={(e) => e.target === e.currentTarget && close()}>
      <div
        ref={dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby="history-title"
        data-history
        tabIndex={-1}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            close();
          }
          trapTab(e);
        }}
        className="overlay overlay-solid flex h-[700px] max-h-[calc(100vh-48px)] w-[1040px] max-w-[calc(100vw-48px)] flex-col overflow-hidden rounded-[20px] outline-none dialog-in"
      >
        <div className="flex h-[60px] shrink-0 items-center justify-between px-[22px] shadow-[inset_0_-1px_0_rgb(255_255_255/.06)]">
          <h2 id="history-title" className="text-[17px] font-semibold leading-[22px] tracking-[-0.025em]">
            History
          </h2>
          <IconButton label="Close (Esc)" round className="-mr-2" onClick={close}>
            <X size={16} />
          </IconButton>
        </div>
        <div className="flex min-h-0 flex-1">
          <div className="flex w-[360px] shrink-0 flex-col gap-2 p-3 shadow-[inset_-1px_0_0_rgb(255_255_255/.06)]">
            <Segmented label="Show" options={TABS} value={tab} onChange={setTab} />
            <div ref={list} role="listbox" aria-label="Versions" onKeyDown={onListKey} className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto">
              {error ? (
                <div role="alert" className="flex flex-col items-start gap-2 p-3 text-[12px] leading-[18px]">
                  <span>{error}</span>
                  <Button pill onClick={() => setAttempt((n) => n + 1)}>
                    Try again
                  </Button>
                </div>
              ) : !entries ? (
                <p role="status" className="flex items-center gap-2 p-3 text-[12px] text-muted">
                  <Loader2 size={14} className="animate-spin" />
                  Loading the history…
                </p>
              ) : shown.length === 0 ? (
                <p className="p-3 text-[12px] text-muted">{EMPTY[tab]}</p>
              ) : (
                shown.map((e) => {
                  const day = dayLabel(e.rev.atMs);
                  const header = day !== lastDay;
                  lastDay = day;
                  const on = key(e) === chosenKey;
                  return (
                    <div key={key(e)} className="flex flex-col">
                      {header && <div className="px-3 pt-2.5 pb-1 text-[12px] font-semibold text-muted">{day}</div>}
                      <button
                        type="button"
                        role="option"
                        aria-selected={on}
                        data-entry={key(e)}
                        tabIndex={on ? 0 : -1}
                        onClick={(ev) => {
                          setSelected(key(e));
                          ev.currentTarget.focus();
                        }}
                        className={`flex flex-col items-start rounded-[10px] px-3 py-2 text-left transition-colors duration-[120ms] ease-out ${
                          on ? "bg-accent/[.18] shadow-[inset_0_0_0_1px_rgb(41_151_255/.5)]" : "hover:bg-white/[.05]"
                        }`}
                      >
                        <span className="line-clamp-2 text-[13px] text-fg">
                          {tab === "all" && <span className="text-muted">{NAME[e.kind]} · </span>}
                          {e.rev.label || `Changed ${e.kind === "style" ? "your style" : "caption styles"}`}
                        </span>
                        <span className="text-[12px] text-muted">
                          {where(e.rev)} · {clockLabel(e.rev.atMs)}
                          {isNow(e) && " · now"}
                        </span>
                      </button>
                    </div>
                  );
                })
              )}
            </div>
          </div>
          <div className="flex min-w-0 flex-1 flex-col px-[22px] pt-[18px] pb-[18px]">
            {chosen && (
              <>
                <div className="text-[13px] font-semibold">
                  {NAME[chosen.kind]}, {when(chosen.rev.atMs)}
                </div>
                <div className="text-[12px] text-muted">
                  {isNow(chosen) ? (
                    "This is how it is now."
                  ) : same ? (
                    "Same as now: restoring it would change nothing."
                  ) : chosen.kind === "style" ? (
                    <>
                      <span className="text-ok">Green</span> comes back on Restore · <span className="text-danger line-through">red</span> is in your style now and goes away
                    </>
                  ) : (
                    "What Restore does to each caption style"
                  )}
                </div>
                <div ref={well} className="mt-3 min-h-0 flex-1 overflow-auto rounded-[10px] bg-black/25 py-3 shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
                  {view?.error ? (
                    <div role="alert" className="flex flex-col items-start gap-2 px-3.5 text-[12px] leading-[18px]">
                      <span>{view.error}</span>
                      <Button pill onClick={() => setPreviewAttempt((n) => n + 1)}>
                        Try again
                      </Button>
                    </div>
                  ) : !view?.view ? (
                    <p role="status" className="flex items-center gap-2 px-3.5 text-[12px] text-muted">
                      <Loader2 size={14} className="animate-spin" />
                      Loading this version…
                    </p>
                  ) : chosen.kind === "style" ? (
                    <StyleDiff then={view.view.text} now={view.view.nowText} />
                  ) : (
                    <CaptionChanges changes={view.view.captionStyles} />
                  )}
                </div>
                {restoreError && (
                  <p role="alert" className="mt-3 flex items-start gap-1.5 text-[12px] leading-[18px] text-fg">
                    <AlertCircle size={14} className="mt-0.5 shrink-0 text-danger" />
                    {restoreError}
                  </p>
                )}
                <div className="mt-3.5 flex justify-end gap-2">
                  <Button pill onClick={close}>
                    Close
                  </Button>
                  <Button pill variant="primary" disabled={isNow(chosen) || same || !view?.view || restoring} disabledReason={reason} onClick={() => void restore()}>
                    {restoring ? "Restoring…" : "Restore this version"}
                  </Button>
                </div>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function StyleDiff({ then, now }: { then: string | null; now: string | null }) {
  const lines = useMemo(() => lineDiff(then ?? "", now ?? ""), [then, now]);
  if (then === null && now === null) return <p className="px-3.5 text-[12px] text-muted">No style, then or now: the guide's defaults.</p>;
  return (
    <div aria-label="EDIT.md" className="font-mono text-[12.5px] leading-5">
      {then === null && <p className="px-3.5 pb-2 font-sans text-[12px] text-muted">No style then: Restore goes back to the guide's defaults.</p>}
      {lines === null ? (
        <>
          <p className="px-3.5 pb-2 font-sans text-[12px] text-muted">Too long to compare line by line. This is the version as it was.</p>
          <div className="px-3.5 whitespace-pre-wrap text-fg/80">{then}</div>
        </>
      ) : (
        lines.map((line, i) => (
          <div
            key={i}
            data-diff={line.kind}
            className={`px-3.5 whitespace-pre-wrap ${
              line.kind === "back" ? "bg-ok/[.14]" : line.kind === "goes" ? "bg-danger/[.14] line-through decoration-white/50" : "text-fg/80"
            }`}
          >
            {line.kind !== "same" && <span className="sr-only">{line.kind === "back" ? "Comes back: " : "Goes away: "}</span>}
            {line.text || " "}
          </div>
        ))
      )}
    </div>
  );
}

/** The caption styles that change first, and how many stay as they are. */
function CaptionChanges({ changes }: { changes: [string, CaptionChange][] }) {
  const changed = changes.filter(([, change]) => change !== "same");
  const kept = changes.length - changed.length;
  if (changes.length === 0) return <p className="px-3.5 text-[12px] text-muted">No caption styles of your own, then or now.</p>;
  return (
    <ul aria-label="Caption styles" className="flex flex-col">
      {changed.map(([name, change]) => (
        <li key={name} className="flex items-center justify-between gap-3 px-3.5 py-1.5 text-[13px]">
          <span className="truncate">{name}</span>
          <span className={`shrink-0 text-[12px] ${CHANGE[change].className}`}>{CHANGE[change].text}</span>
        </li>
      ))}
      {kept > 0 && (
        <li className="px-3.5 py-1.5 text-[12px] text-muted">
          {changed.length === 0 ? (kept === 1 ? "1 style, no change" : `${kept} styles, no change`) : `${kept} more, no change`}
        </li>
      )}
    </ul>
  );
}

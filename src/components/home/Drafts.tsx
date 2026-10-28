import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { CopyPlus, Film, FolderOpen, FolderPlus, Loader2 } from "lucide-react";
import { addDraftFolder, findDrafts, loadCover, openDraft, projectOf, useCapCut } from "../../lib/capcut";
import { openProject, useLibrary, whenLabel } from "../../lib/library";
import { formatLength } from "../../lib/time";
import type { CapCutDraft, LibraryProject } from "../../lib/types";
import { Button, FadeImg, IconButton, Menu, type MenuEntry } from "../ui";

/** Width over height of a card's picture box, as a project card's. */
const CARD_BOX = 152 / 180;
const CARD_MIN = 148;
const GAP = 16;
const chip = "flex items-center gap-1 rounded-[5px] bg-black/60 px-1.5 py-[3px] text-[11px] font-medium leading-none text-fg";

function useColumns() {
  const grid = useRef<HTMLDivElement>(null);
  const [columns, setColumns] = useState(4);
  useLayoutEffect(() => {
    const el = grid.current;
    if (!el) return;
    const measure = () => setColumns(Math.max(1, Math.floor((el.clientWidth + GAP) / (CARD_MIN + GAP))));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return [grid, columns] as const;
}

/** CapCut's drafts on this computer as cards, newest first: one row of them, or all. */
export function DraftsSection({ title, className = "" }: { title: string; className?: string }) {
  const drafts = useCapCut((s) => s.drafts);
  const showAll = useCapCut((s) => s.showAll);
  const projects = useLibrary((s) => s.projects);
  const [grid, columns] = useColumns();
  const [focused, setFocused] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ items: MenuEntry[]; at: { x: number; y: number }; label: string; keyboard: boolean; back: HTMLElement } | null>(null);

  useEffect(() => {
    void findDrafts();
    // CapCut may have saved a draft while Nuzky was in the background.
    const onFocus = () => void findDrafts();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  const shown = showAll ? (drafts ?? []) : (drafts ?? []).slice(0, columns);
  const tabStop = shown.some((d) => d.path === focused) ? focused : shown[0]?.path;

  /** Opening the project made of a draft, or the draft again as a new project once it changed in CapCut. */
  const showMenu = (draft: CapCutDraft, at: { x: number; y: number }, keyboard: boolean, back: HTMLElement) => {
    const made = projectOf(projects, draft);
    const items: MenuEntry[] = [
      ...(made ? [{ label: `Open “${made.name}”`, icon: <FolderOpen size={15} />, run: () => openProject(made) }] : []),
      { label: "Open as a new project", icon: <CopyPlus size={15} />, run: () => openDraft(draft, true) },
    ];
    setMenu({ items, at, label: draft.name, keyboard, back });
  };

  /** One Tab stop; the arrow keys move by where the cards sit, Home and End to the ends, as in the projects grid. */
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const card = (e.target as HTMLElement).closest<HTMLElement>("[data-draft]");
    const index = shown.findIndex((d) => d.path === card?.dataset.draft);
    if (!card || index < 0) return;
    const step: Record<string, number> = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: columns, ArrowUp: -columns };
    const to = e.key === "Home" ? 0 : e.key === "End" ? shown.length - 1 : e.key in step ? index + step[e.key] : null;
    if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
      e.preventDefault();
      const r = card.getBoundingClientRect();
      return showMenu(shown[index], { x: r.left + 12, y: r.top + 24 }, true, card);
    }
    if (to === null || to < 0 || to >= shown.length) return;
    e.preventDefault();
    setFocused(shown[to].path);
    grid.current?.querySelector<HTMLElement>(`[data-draft="${CSS.escape(shown[to].path)}"]`)?.focus();
  };

  return (
    <section aria-label="CapCut drafts" hidden={!drafts?.length} className={className}>
      <div className="mb-3 flex h-7 items-center gap-2">
        <h2 className="text-[13px] font-semibold text-fg">{title}</h2>
        <span className="tabular text-[13px] text-muted">{drafts?.length}</span>
        <div className="flex-1" />
        <IconButton label="Add a draft folder…" onClick={() => void addDraftFolder()} className="h-7 w-7">
          <FolderPlus size={15} />
        </IconButton>
        {(drafts?.length ?? 0) > columns && (
          <Button variant="ghost" onClick={() => useCapCut.setState({ showAll: !showAll })} className="h-7 px-2 text-[12px] text-accent! hover:text-accent!">
            {showAll ? "Show fewer" : "Show all"}
          </Button>
        )}
      </div>
      <div ref={grid} role="group" aria-label="CapCut drafts, newest first" onKeyDown={onKey} className="grid grid-cols-[repeat(auto-fill,minmax(148px,168px))] gap-x-4 gap-y-5">
        {shown.map((d) => (
          <DraftCard
            key={d.path}
            draft={d}
            made={projectOf(projects, d)}
            focusable={d.path === tabStop}
            onFocus={() => setFocused(d.path)}
            onMenu={(at, back) => showMenu(d, at, false, back)}
          />
        ))}
      </div>
      {menu && (
        <Menu
          items={menu.items}
          at={menu.at}
          label={menu.label}
          keyboard={menu.keyboard}
          onClose={(chose) => {
            const back = menu.back;
            setMenu(null);
            if (!chose) back.focus();
          }}
        />
      )}
    </section>
  );
}

/** A draft with its cover; it opens as a new project, or as the project made of it before. */
function DraftCard({
  draft,
  made,
  focusable,
  onFocus,
  onMenu,
}: {
  draft: CapCutDraft;
  made: LibraryProject | undefined;
  focusable: boolean;
  onFocus: () => void;
  onMenu: (at: { x: number; y: number }, back: HTMLElement) => void;
}) {
  const cover = useCapCut((s) => s.covers[draft.path]);
  const opening = useCapCut((s) => s.opening === draft.path);
  const [ratio, setRatio] = useState(9 / 16);
  useEffect(() => loadCover(draft), [draft]);

  const when = whenLabel(draft.modifiedMs);
  // Saved in CapCut after it was opened: opening it again as a new project brings that over.
  const changed = !!made && draft.modifiedMs > (made.draftModifiedMs ?? 0);
  const meta = made ? `${changed ? "Changed since" : "In Nuzky"} · ${when}` : when;
  const frame = ratio < CARD_BOX ? { height: "100%", aspectRatio: `${ratio}` } : { width: "100%", aspectRatio: `${ratio}` };
  return (
    <button
      type="button"
      data-draft={draft.path}
      tabIndex={focusable ? 0 : -1}
      aria-busy={opening || undefined}
      aria-label={`${draft.name}, changed ${when}${made ? `, in Nuzky as ${made.name}${changed ? ", changed in CapCut since" : ""}` : ""}`}
      title={
        made
          ? `Open “${made.name}”, the project made of this draft.${changed ? " It changed in CapCut since; right-click to open it again as a new project." : ""}`
          : "Open as a new project. The draft in CapCut stays as it is."
      }
      onClick={() => !opening && openDraft(draft)}
      onFocus={onFocus}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu({ x: e.clientX, y: e.clientY }, e.currentTarget);
      }}
      className="group/card flex min-w-0 flex-col text-left outline-none"
    >
      <div
        className="relative flex w-full items-center justify-center overflow-hidden rounded-[10px] bg-stage outline outline-1 -outline-offset-1 outline-white/[.06] transition-[outline-color] duration-[120ms] ease-out group-hover/card:outline-white/[.16] group-focus-visible/card:outline-2 group-focus-visible/card:outline-offset-2 group-focus-visible/card:outline-accent"
        style={{ aspectRatio: `${CARD_BOX}` }}
      >
        <div className="relative overflow-hidden bg-white/[.04]" style={frame}>
          {cover?.url ? (
            <FadeImg src={cover.url} alt="" draggable={false} onLoad={(e) => setRatio(e.currentTarget.naturalWidth / Math.max(1, e.currentTarget.naturalHeight))} className="h-full w-full object-cover" />
          ) : (
            cover && (
              <div className="flex h-full w-full items-center justify-center text-subtle">
                <Film size={17} />
              </div>
            )
          )}
          {draft.durationUs !== null && <span className={`tabular absolute bottom-1.5 right-1.5 ${chip}`}>{formatLength(draft.durationUs)}</span>}
        </div>
        {opening && (
          <span className={`absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 whitespace-nowrap ${chip}`}>
            <Loader2 size={11} className="animate-spin" /> Opening…
          </span>
        )}
      </div>
      <span data-name className="mt-2 truncate text-[13px] font-medium text-fg">
        {draft.name}
      </span>
      <span data-meta className="tabular mt-0.5 truncate text-[12px] text-muted">
        {meta}
      </span>
    </button>
  );
}

import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { AlertTriangle, Copy, ListVideo, Loader2, Sparkles, X } from "lucide-react";
import { MAX_CHAPTER_TITLE, MIN_CHAPTERS_VIDEO_US, renameChapters, suggestBlocked, suggestChapters, useChapterList, useChapters } from "../../lib/chapters";
import { copyText } from "../../lib/clipboard";
import { useAgent } from "../../lib/agent";
import { useChatGpt } from "../../lib/chatgpt";
import { useModels } from "../../lib/models";
import { projectDuration, useEditor } from "../../lib/store";
import { errorText } from "../../lib/api";
import { Button, IconButton, useLockReason } from "../ui";

/** Opens the Chapters region; offered once the video is long enough for YouTube to show chapters, or has some. */
export function ChaptersButton() {
  const open = useChapters((s) => s.open);
  const offered = useEditor((s) => !!s.snap && (projectDuration(s.snap.project) >= MIN_CHAPTERS_VIDEO_US || (s.snap.project.chapters?.length ?? 0) > 0));
  if (!offered) return null;
  return (
    <Button
      className={`h-6 px-2 text-[12px] ${open ? "bg-accent/[.18]! text-accent!" : ""}`}
      data-chapters-toggle
      aria-expanded={open}
      title="Chapters for YouTube from what is said"
      onClick={() => useChapters.setState({ open: !open })}
    >
      <ListVideo size={13} /> Chapters
    </Button>
  );
}

const back = () => document.querySelector<HTMLElement>("[data-chapters-toggle]")?.focus();

/**
 * The chapters above the text: each with its time in the video as it plays now, which moves the playhead there, and
 * its title to edit; Copy puts the list for YouTube on the clipboard. Esc closes it and focus returns to Chapters.
 */
export function ChaptersBar() {
  const open = useChapters((s) => s.open);
  const finding = useChapters((s) => s.finding);
  const { list, error } = useChapterList();
  const duration = useEditor((s) => (s.snap ? projectDuration(s.snap.project) : 0));
  const lock = useLockReason();
  useAgent((s) => s.status);
  useAgent((s) => s.agents);
  useChatGpt((s) => s.status);
  useModels((s) => s.lists.chatgpt);
  const blocked = suggestBlocked(duration);
  const [drafts, setDrafts] = useState<string[] | null>(null);
  const bar = useRef<HTMLDivElement>(null);
  const titles = list?.chapters.map((c) => c.title).join("\n");
  useEffect(() => setDrafts(null), [titles]);
  if (!open) return null;

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "Escape" || e.defaultPrevented) return;
    e.stopPropagation();
    useChapters.setState({ open: false });
    back();
  };
  const copy = async () => {
    if (!list?.text) return;
    const listed = list.chapters.flatMap((c, i) => (c.leftOut || c.startUs === null ? [] : [`${c.time} ${drafts?.[i]?.trim() || c.title}`]));
    try {
      await copyText(listed.join("\n"), "Copying doesn't work here. Select the list in the file Export saves and copy it from there.");
      const n = listed.length;
      useEditor.getState().toast({ kind: "success", text: `Copied ${n} chapters. Paste them into the video's description on YouTube.` });
    } catch (e) {
      useEditor.getState().toast({ kind: "error", text: errorText(e) });
    }
  };
  const save = () => drafts && void renameChapters(drafts);
  const chapters = list?.chapters ?? [];
  const suggest = (again: boolean) => (
    <Button
      variant={again ? "secondary" : "primary"}
      className={again ? "h-7 px-2 text-[12px]" : "h-7 px-2.5 text-[12px]"}
      data-suggest-chapters
      disabled={!!blocked || finding}
      disabledReason={blocked ?? "Your AI is finding chapters…"}
      title="Your AI reads the transcript and names the chapters; Nuzky times them on the video"
      onClick={suggestChapters}
    >
      <Sparkles size={14} /> {again ? "Suggest again" : "Suggest chapters"}
    </Button>
  );

  return (
    <div ref={bar} role="region" aria-label="Chapters" data-chapters className="flex max-h-[60%] shrink-0 flex-col border-b border-white/[.07] py-2" onKeyDown={onKeyDown}>
      <div className="flex shrink-0 items-center gap-2 px-3.5">
        <ListVideo size={14} className="shrink-0 text-accent" aria-hidden />
        <span className="tabular flex-1 truncate text-[12px] text-muted">{chapters.length ? `${chapters.length} chapters` : "Chapters for YouTube"}</span>
        {chapters.length > 0 && (
          <Button variant="primary" className="h-7 px-2.5 text-[12px]" data-copy-chapters disabled={!list?.text} disabledReason={list?.problem ?? undefined} onClick={() => void copy()}>
            <Copy size={14} /> Copy
          </Button>
        )}
        <IconButton
          label="Close chapters"
          className="h-7 w-7 shrink-0"
          onClick={() => {
            useChapters.setState({ open: false });
            back();
          }}
        >
          <X size={14} />
        </IconButton>
      </div>
      <div className="scroll-shown mt-1.5 flex min-h-0 flex-col gap-1.5 overflow-y-auto px-3.5">
        {chapters.length > 0 && (
          <ol className="flex flex-col" aria-label="Chapters in the order they play">
            {chapters.map((c, i) => (
              <li key={i} className="flex flex-col py-0.5" data-chapter>
                <div className="flex items-center gap-1.5">
                  {c.startUs === null ? (
                    <span data-chapter-time className="tabular w-[52px] shrink-0 px-1 text-[12px] text-muted">
                      --:--
                    </span>
                  ) : (
                    <button
                      type="button"
                      data-chapter-time
                      className={`tabular w-[52px] shrink-0 rounded-md px-1 py-0.5 text-left text-[12px] hover:bg-white/[.06] hover:text-fg ${c.leftOut ? "text-muted line-through" : "text-fg/80"}`}
                      title={`Go to ${c.time}`}
                      onClick={() => useEditor.getState().seek(c.startUs!)}
                    >
                      {c.time}
                    </button>
                  )}
                  <input
                    className="h-7 min-w-0 flex-1 rounded-md border border-transparent bg-transparent px-1.5 text-[13px] text-fg/90 outline-none hover:bg-white/[.05] focus:border-accent focus:bg-raised disabled:cursor-not-allowed"
                    aria-label={`Title of chapter ${i + 1}`}
                    value={drafts?.[i] ?? c.title}
                    maxLength={MAX_CHAPTER_TITLE}
                    disabled={!!lock}
                    title={lock ?? undefined}
                    spellCheck={false}
                    onChange={(e) => {
                      const next = drafts ?? chapters.map((x) => x.title);
                      setDrafts(next.map((t, j) => (j === i ? e.target.value : t)));
                    }}
                    onBlur={save}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") e.currentTarget.blur();
                      else if (e.key === "Escape") {
                        e.preventDefault();
                        setDrafts(null);
                      }
                    }}
                  />
                </div>
                {c.leftOut && (
                  <span data-left-out className="flex items-center gap-1.5 pl-[58px] text-[11px] text-muted">
                    <AlertTriangle size={12} className="shrink-0 text-warn" aria-hidden />
                    Left out: {c.leftOut}
                  </span>
                )}
              </li>
            ))}
          </ol>
        )}
        {list?.problem && (
          <p className="flex items-start gap-1.5 text-[12px] text-fg" role="status">
            <AlertTriangle size={14} className="mt-px shrink-0 text-warn" />
            {list.problem}
          </p>
        )}
        {error && (
          <p className="text-[12px] text-danger" role="alert">
            {error}
          </p>
        )}
        {finding ? (
          <p className="flex h-7 items-center gap-1.5 text-[12px] text-muted" role="status">
            <Loader2 size={14} className="shrink-0 animate-spin text-accent" /> Your AI is finding chapters…
          </p>
        ) : chapters.length > 0 ? (
          <div className="flex">{suggest(true)}</div>
        ) : (
          <div className="flex items-center gap-2">
            <span className="flex-1 text-[12px] text-muted">Your AI names them, Nuzky times them on the finished video.</span>
            {suggest(false)}
          </div>
        )}
      </div>
    </div>
  );
}

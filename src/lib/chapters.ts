import { useEffect, useState } from "react";
import { create } from "zustand";
import { api, errorText, plainError } from "./api";
import { anyAgent, send, sendBlocked, useAgent } from "./agent";
import { useDock } from "./dock";
import { useSpeech } from "./speech";
import { aiLocked, projectDuration, useEditor } from "./store";
import { US } from "./time";
import type { ChapterList } from "./types";

/** The longest chapter title, as `MAX_CHAPTER_TITLE_CHARS` in model.rs. */
export const MAX_CHAPTER_TITLE = 100;

/** YouTube shows chapters only in a video this long: three of 10 s. */
export const MIN_CHAPTERS_VIDEO_US = 30 * US;

/** What Suggest chapters asks the AI panel's agent; the guide it reads tells it how. */
export const FIND_CHAPTERS =
  "Suggest YouTube chapters for this video from what is said, with short titles in the language of the video. Transcribe it first if it has no transcript. Don't change the video itself.";

interface ChaptersState {
  /** The Chapters region of the Transcript tab is open. */
  open: boolean;
  /** Suggest chapters sent its message and the agent is answering it. */
  finding: boolean;
}

export const useChapters = create<ChaptersState>(() => ({ open: false, finding: false }));

/** Why Suggest chapters cannot ask the AI panel's agent now, or null. */
export function suggestBlocked(durationUs: number): string | null {
  if (durationUs < MIN_CHAPTERS_VIDEO_US) return "YouTube shows chapters only in a video of 30 s or more";
  if (useAgent.getState().agents && !anyAgent()) return "Sign in with ChatGPT first, or install Claude Code or Codex";
  return sendBlocked(false);
}

/** Asks the AI panel's agent for chapters; they show up as it proposes them. */
export function suggestChapters() {
  const snap = useEditor.getState().snap;
  if (!snap || aiLocked()) return;
  useAgent.setState({ thumbnail: false });
  if (suggestBlocked(projectDuration(snap.project))) return;
  useDock.setState({ open: true });
  useChapters.setState({ finding: true });
  void send(FIND_CHAPTERS, "project");
}

useEditor.subscribe(function showSuggestedChapters(s, prev) {
  const count = (snap: typeof s.snap) => snap?.project.chapters?.length ?? 0;
  if (s.snap?.sessionEpoch === prev.snap?.sessionEpoch && count(prev.snap) === 0 && count(s.snap) > 0) useChapters.setState({ open: true });
});

useAgent.subscribe(function endFindingWithTheAnswer(s, prev) {
  if (prev.status !== "idle" && s.status === "idle" && useChapters.getState().finding) useChapters.setState({ finding: false });
});

/** Renames the chapters, as one undo step; an empty title keeps the one before. */
export function renameChapters(titles: string[]) {
  return useEditor.getState().edit((project) => {
    const chapters = project.chapters ?? [];
    const next = chapters.map((c, i) => ({ ...c, title: titles[i]?.trim() || c.title }));
    return next.some((c, i) => c.title !== chapters[i].title) ? { type: "setChapters", chapters: next } : null;
  });
}

/** The project's chapters as the video plays them now, asked again after every edit and recognition. */
export function useChapterList(): { list: ChapterList | null; error: string | null } {
  const revision = useEditor((s) => s.snap?.revision);
  const epoch = useEditor((s) => s.snap?.sessionEpoch);
  const has = useEditor((s) => (s.snap?.project.chapters?.length ?? 0) > 0);
  const recognised = useSpeech((s) => s.stored);
  const [state, setState] = useState<{ at: string; list: ChapterList | null; error: string | null }>({ at: "", list: null, error: null });
  const at = `${epoch} ${revision} ${recognised}`;
  useEffect(() => {
    if (!has) return;
    let live = true;
    api.chapterList().then(
      (list) => live && setState({ at, list, error: null }),
      (e) => live && setState({ at, list: null, error: plainError(errorText(e)) }),
    );
    return () => {
      live = false;
    };
  }, [at, has]);
  if (!has) return { list: null, error: null };
  return state.at.split(" ")[0] === `${epoch}` ? state : { list: null, error: null };
}

import { useEffect, useState } from "react";
import { create } from "zustand";
import { api, errorText, plainError } from "./api";
import { anyAgent, send, sendBlocked, useAgent } from "./agent";
import { useDock } from "./dock";
import { useSpeech } from "./speech";
import { aiLocked, projectDuration, useEditor } from "./store";
import { closeVoiceover, useVoiceover } from "./voiceover";
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
  /** The chapter whose title takes focus once the region shows it. */
  focus: number | null;
}

export const useChapters = create<ChaptersState>(() => ({ open: false, finding: false, focus: null }));

/** Why the chapters cannot show in the Transcript tab now, or null. */
export function chaptersBlocked(): string | null {
  const { phase } = useVoiceover.getState();
  return phase === "countdown" || phase === "recording" || phase === "stopping" ? "Stop recording the voiceover first" : null;
}

/** Shows the chapters in the Transcript tab, with the title of chapter `focus` ready to type in; an idle voiceover gives the pane back. */
export function openChapters(focus: number | null = null) {
  if (chaptersBlocked()) return;
  if (useVoiceover.getState().phase !== "closed") void closeVoiceover();
  useEditor.setState({ panelTab: "transcript" });
  useChapters.setState({ open: true, focus });
}

/** The last of `starts` the playhead has passed, or -1 before the first. */
export function currentChapter(starts: number[], timeUs: number) {
  let current = -1;
  starts.forEach((start, i) => start <= timeUs && (current = i));
  return current;
}

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

/** One request in flight serves every view that asks for the same revision; a settled one is asked again. */
let fetching: { at: string; list: Promise<ChapterList> } | null = null;
/** The last list that arrived, so a view shown again for the same revision starts from it rather than from nothing. */
let arrived: { at: string; list: ChapterList } | null = null;

/** The project's chapters as the video plays them now, asked again after every edit and recognition. */
export function useChapterList(): { list: ChapterList | null; error: string | null } {
  const revision = useEditor((s) => s.snap?.revision);
  const epoch = useEditor((s) => s.snap?.sessionEpoch);
  const has = useEditor((s) => (s.snap?.project.chapters?.length ?? 0) > 0);
  const recognised = useSpeech((s) => s.stored);
  const at = `${epoch} ${revision} ${recognised}`;
  const [state, setState] = useState<{ at: string; list: ChapterList | null; error: string | null }>(() => (arrived?.at === at ? { at, list: arrived.list, error: null } : { at: "", list: null, error: null }));
  useEffect(() => {
    if (!has) return;
    let live = true;
    if (fetching?.at !== at) {
      const request = { at, list: api.chapterList() };
      fetching = request;
      const done = () => {
        if (fetching === request) fetching = null;
      };
      request.list.then(done, done);
    }
    fetching!.list.then(
      (list) => {
        arrived = { at, list };
        if (live) setState({ at, list, error: null });
      },
      (e) => live && setState({ at, list: null, error: plainError(errorText(e)) }),
    );
    return () => {
      live = false;
    };
  }, [at, has]);
  if (!has) return { list: null, error: null };
  return state.at.split(" ")[0] === `${epoch}` ? state : { list: null, error: null };
}

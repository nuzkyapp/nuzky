import { create } from "zustand";
import { api, errorText, plainError } from "./api";
import { aiLocked, currentEpoch, useEditor } from "./store";
import type { Part, YoutubeVideo } from "./types";

/** The Video from YouTube dialog, and the link it opens with. */
export const useYoutube = create<{ open: boolean; url: string }>(() => ({ open: false, url: "" }));

export const openYoutube = (url = "") => useYoutube.setState({ open: true, url });

let adds = 0;

/**
 * Adds the downloaded video, or its part, to the media while a skeleton tile stands in for it.
 * A failure says why and offers to open the link again; the download is still there.
 */
export async function addYoutubeVideo(video: YoutubeVideo, part: Part | null, url: string) {
  if (aiLocked()) return;
  const { setSnap, toast } = useEditor.getState();
  const pending = { key: -++adds, name: video.title, kind: "video" as const };
  useEditor.setState({ importing: [...useEditor.getState().importing, pending] });
  try {
    setSnap(await api.youtubeVideoAdd(video, part, currentEpoch()));
  } catch (e) {
    toast({ kind: "error", text: `Couldn't add ${video.title}. ${plainError(errorText(e))}`, action: { label: "Open again", run: () => openYoutube(url) } });
  } finally {
    useEditor.setState({ importing: useEditor.getState().importing.filter((i) => i !== pending) });
  }
}

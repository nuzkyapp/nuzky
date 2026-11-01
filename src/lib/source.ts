import { create } from "zustand";
import { api, errorText, plainError } from "./api";
import type { Asset, VideoPreview } from "./types";

/** A media item played in the preview in place of the timeline, as CapCut's player does; the project stays as it is. */
export const useSource = create<{ asset: Asset | null; preview: VideoPreview | null; timeUs: number; playing: boolean; error: string | null }>(() => ({
  asset: null,
  preview: null,
  timeUs: 0,
  playing: false,
  error: null,
}));

let opening = 0;

/** Shows the asset in the preview, paused at its start. The timeline pauses and keeps its playhead. */
export async function openSource(asset: Asset) {
  api.pause();
  const id = ++opening;
  // The previous item's frames and length must not show under this one's name.
  useSource.setState({ asset, preview: null, timeUs: 0, playing: false, error: null });
  try {
    const preview = await api.sourcePreviewOpen(asset.id);
    if (id === opening) useSource.setState({ preview });
    // Closed while it opened: the preview it started would run unseen.
    else if (!sourceOpen()) void api.sourcePreviewClose();
  } catch (e) {
    if (id === opening) useSource.setState({ error: plainError(errorText(e)) });
  }
}

/** Back to the timeline in the preview. */
export function closeSource() {
  if (!useSource.getState().asset) return;
  opening++;
  useSource.setState({ asset: null, preview: null, playing: false, timeUs: 0, error: null });
  void api.sourcePreviewClose();
}

export const sourceOpen = () => useSource.getState().asset !== null;

/** A picture has nothing to play. */
export const sourceDuration = (s: { asset: Asset | null; preview: VideoPreview | null }) => (s.asset?.kind === "image" ? 0 : (s.preview?.durationUs ?? 0));

export function seekSource(tUs: number) {
  const s = useSource.getState();
  const t = Math.max(0, Math.min(sourceDuration(s), Math.round(tUs)));
  useSource.setState({ timeUs: t });
  void api.sourcePreviewSeek(t);
}

export function toggleSourcePlay() {
  const s = useSource.getState();
  if (sourceDuration(s) === 0) return;
  void (s.playing ? api.sourcePreviewPause() : api.sourcePreviewPlay());
}

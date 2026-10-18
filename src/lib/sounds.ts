import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { api, errorText } from "./api";
import { copyText } from "./clipboard";
import { aiLocked, currentEpoch, useEditor } from "./store";
import type { Sound, SoundKind } from "./types";

export type AudioTab = "project" | "music" | "effects";

export type Search = {
  query: string;
  sounds: Sound[];
  page: number;
  more: boolean;
  /** "Openverse" or "Freesound", once results came. */
  service: string | null;
  loading: boolean;
  error: string | null;
};

const EMPTY: Search = { query: "", sounds: [], page: 0, more: false, service: null, loading: false, error: null };

/** A pasted YouTube link and the video it points at, once looked up. */
export type Link = { url: string; sound: Sound | null; loading: boolean; error: string | null };

const NO_LINK: Link = { url: "", sound: null, loading: false, error: null };
const waitsForYtdlp = (link: Link) => !!link.error?.startsWith("YTDLP_");

type State = {
  tab: AudioTab;
  builtIn: Sound[];
  search: Record<SoundKind, Search>;
  /** The sound playing, or still loading to play. */
  preview: { id: string; loading: boolean } | null;
  /** Download progress of a sound being fetched, 0 to 1. */
  progress: Record<string, number>;
  /** Sounds being added to the timeline. */
  adding: string[];
  link: Link;
  /** The link waiting for the user to read, once, that the music's rights are theirs. */
  rightsFor: string | null;
  /** yt-dlp is downloading. */
  installing: boolean;
  /** Why the last yt-dlp download ended without it; `CANCELLED: …` when it was stopped. */
  installError: string | null;
};

export const useSounds = create<State>(() => ({
  tab: "project",
  builtIn: [],
  search: { music: EMPTY, effect: EMPTY },
  preview: null,
  progress: {},
  adding: [],
  link: NO_LINK,
  rightsFor: null,
  installing: false,
  installError: null,
}));

let listening = false;
/** Download progress and finished previews come as events; the timeline starting to play stops a preview. */
function listenOnce() {
  if (listening) return;
  listening = true;
  listen<{ id: string; progress: number }>("sound-progress", (e) =>
    useSounds.setState((s) => ({ progress: { ...s.progress, [e.payload.id]: e.payload.progress } })),
  );
  listen<string>("sound-preview-ended", (e) => {
    if (useSounds.getState().preview?.id === e.payload) useSounds.setState({ preview: null });
  });
  useEditor.subscribe((s, prev) => {
    if (s.playing && !prev.playing && useSounds.getState().preview) stopPreview();
  });
}

export async function loadBuiltIn() {
  listenOnce();
  if (useSounds.getState().builtIn.length === 0) useSounds.setState({ builtIn: await api.soundLibrary() });
}

const searches: Record<SoundKind, number> = { music: 0, effect: 0 };
const setSearch = (kind: SoundKind, next: Partial<Search>) => useSounds.setState((s) => ({ search: { ...s.search, [kind]: { ...s.search[kind], ...next } } }));

/** Page 1 replaces the results, a later page adds to them. An answer to an older search is dropped. */
export async function searchSounds(kind: SoundKind, query: string, page = 1) {
  const seq = ++searches[kind];
  const text = query.trim();
  if (!text) {
    setSearch(kind, { ...EMPTY, query });
    return;
  }
  setSearch(kind, { query, loading: true, error: null, ...(page === 1 ? { sounds: [], page: 0, more: false } : {}) });
  try {
    const found = await api.soundSearch(text, kind, page);
    if (seq !== searches[kind]) return;
    const before = page === 1 ? [] : useSounds.getState().search[kind].sounds;
    setSearch(kind, { sounds: [...before, ...found.sounds], page, more: found.more, service: found.service, loading: false });
  } catch (e) {
    if (seq === searches[kind]) setSearch(kind, { loading: false, error: errorText(e) });
  }
}

let links = 0;
let rightsRead = false;
/**
 * What the user asked of the linked song when their own yt-dlp could not get it, done once Nuzky has its own:
 * adding it where and in the project it was asked, else playing it.
 */
let afterYtdlp: { sound: Sound; add: { timeUs: number; epoch: string | undefined } | null } | null = null;

/** Looks up the video of a YouTube link; the first time, the user reads whose the music's rights are. */
export async function openLink(url: string) {
  const text = url.trim();
  const { link, adding, preview } = useSounds.getState();
  if (link.sound && text === link.url) return;
  // Another link takes the row and its Stop away, so its download stops with it; one from Get set up has no row here.
  if (useSounds.getState().installing && waitsForYtdlp(link)) api.soundCancel("yt-dlp");
  if (link.sound) {
    if (adding.includes(link.sound.id)) api.soundCancel(link.sound.id);
    if (preview?.id === link.sound.id) stopPreview();
  }
  const seq = ++links;
  if (!text) return useSounds.setState({ link: NO_LINK, rightsFor: null });
  useSounds.setState({ link: { ...NO_LINK, url: text, loading: true } });
  try {
    rightsRead ||= (await api.soundSettings()).youtubeRights;
    if (seq !== links) return;
    if (!rightsRead) return useSounds.setState({ link: { ...NO_LINK, url: text }, rightsFor: text });
    const sound = await api.soundLink(text);
    if (seq === links) useSounds.setState({ link: { ...NO_LINK, url: text, sound } });
  } catch (e) {
    if (seq === links) useSounds.setState({ link: { ...NO_LINK, url: text, error: errorText(e) } });
  }
}

/** Answers the notice about rights: agreeing looks the link up, Cancel leaves it unlooked. */
export async function answerRights(agree: boolean) {
  const url = useSounds.getState().rightsFor;
  useSounds.setState({ rightsFor: null });
  if (!agree || !url) return;
  try {
    rightsRead = (await api.confirmYoutubeRights()).youtubeRights;
    await openLink(url);
  } catch (e) {
    useSounds.setState({ link: { ...NO_LINK, url, error: errorText(e) } });
  }
}

/**
 * Downloads yt-dlp and looks a link waiting for it up again. Music's own download then plays or adds the song when
 * that is what failed; one from Get set up drops that request, as nobody there asked for the song. Stopped, the link
 * still offers the download.
 */
export async function installYtdlp(finishAsked: boolean) {
  if (useSounds.getState().installing) return;
  useSounds.setState({ installing: true, installError: null });
  try {
    await api.installYtdlp();
    useSounds.setState({ installing: false });
    const next = finishAsked ? afterYtdlp : null;
    afterYtdlp = null;
    const { link } = useSounds.getState();
    if (waitsForYtdlp(link)) await openLink(link.url);
    if (!next || useSounds.getState().link.sound?.id !== next.sound.id) return;
    if (!next.add) await togglePreview(next.sound);
    else if (next.add.epoch === currentEpoch()) await addSound(next.sound, next.add.timeUs);
  } catch (e) {
    const text = errorText(e);
    useSounds.setState((s) => ({ installing: false, installError: text, link: !text.startsWith("CANCELLED") && waitsForYtdlp(s.link) ? { ...s.link, error: text } : s.link }));
  } finally {
    useSounds.setState((s) => ({ progress: { ...s.progress, "yt-dlp": 0 } }));
  }
}

/** When the user's own yt-dlp could not download the linked song, the link offers Nuzky's copy in its place. */
function offerOwnYtdlp(sound: Sound, text: string, add: { timeUs: number; epoch: string | undefined } | null) {
  const { link } = useSounds.getState();
  // A double click plays and adds at once; whichever fails second still counts, and adding wins.
  const offered = afterYtdlp?.sound.id === sound.id && !!link.error?.startsWith("YTDLP_OUTDATED");
  if (!text.startsWith("YTDLP_OUTDATED") || (link.sound?.id !== sound.id && !offered)) return false;
  if (!offered || add) afterYtdlp = { sound, add };
  useSounds.setState({ link: { ...NO_LINK, url: link.url, error: text } });
  return true;
}

export function stopPreview() {
  useSounds.setState({ preview: null });
  api.soundStop();
}

/** Plays the sound, or stops it when it is the one playing. The timeline pauses meanwhile. */
export async function togglePreview(sound: Sound) {
  if (useSounds.getState().preview?.id === sound.id) return stopPreview();
  const { playing, toast } = useEditor.getState();
  if (playing) await api.pause();
  useSounds.setState({ preview: { id: sound.id, loading: true } });
  try {
    await api.soundPreview(sound.id);
    if (useSounds.getState().preview?.id === sound.id) useSounds.setState({ preview: { id: sound.id, loading: false } });
  } catch (e) {
    const text = errorText(e);
    if (useSounds.getState().preview?.id !== sound.id || text.startsWith("CANCELLED")) return;
    useSounds.setState({ preview: null });
    if (!offerOwnYtdlp(sound, text, null)) toast({ kind: "error", text });
  }
}

/** Adds the sound at the playhead, or at `timeUs`, downloading it first when needed. */
export async function addSound(sound: Sound, timeUs = useEditor.getState().timeUs) {
  if (aiLocked() || useSounds.getState().adding.includes(sound.id)) return;
  const { setSnap, toast } = useEditor.getState();
  const epoch = currentEpoch();
  useSounds.setState((s) => ({ adding: [...s.adding, sound.id] }));
  try {
    setSnap(await api.soundAdd(sound.id, timeUs, epoch));
  } catch (e) {
    const text = errorText(e);
    if (!text.includes("CANCELLED") && !offerOwnYtdlp(sound, text, { timeUs, epoch })) toast({ kind: "error", text: `Could not add ${sound.title}: ${text}` });
  } finally {
    useSounds.setState((s) => ({ adding: s.adding.filter((id) => id !== sound.id), progress: { ...s.progress, [sound.id]: 0 } }));
  }
}

export async function copyCredits() {
  const { toast } = useEditor.getState();
  try {
    const text = await api.soundCredits(currentEpoch());
    if (!text) return toast({ kind: "info", text: "No sound in the video needs credit." });
    await copyText(text, "The clipboard is not available. Export the video: its credits are saved beside it.");
    toast({ kind: "success", text: "Credits copied. Paste them into the video's description." });
  } catch (e) {
    toast({ kind: "error", text: errorText(e) });
  }
}

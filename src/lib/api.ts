import { invoke } from "@tauri-apps/api/core";
import type { AgentAccess, AgentConnection, AgentKind, AgentSetup, Animation, Boot, CapCutDraft, CaptionPreset, ChatGptModel, ChatGptStatus, Collection, CoverView, DeletedCollection, SpeechModel, SpeechSetup, EditCmd, ExportRequest, Filmstrip, FontFamilies, LayerBounds, Library, PreviewStarted, ProjectSummary, ProjectVersion, RecapInfo, ReelCandidate, ReelFraming, Said, Snapshot, Sound, SoundKind, SoundPage, SoundSettings, StickerCatalog, StyleAction, StylePair, StyleView, TextStyle, Thumbnail, ThumbnailFormat, TimelinePlan, TranscriptCut, TranscriptView, VisionModels, WordsCorrected, ZoomSuggestions, ZoomsApplied } from "./types";

/**
 * The session a change was made for. Mutating commands carry it, so a change still in flight when
 * another project opens is refused instead of landing in the wrong project.
 */
type Epoch = string | undefined;

export type FeedbackKind = "bug" | "idea" | "message";

export const api = {
  setUiContext: (selection: string[], playheadUs: number) => invoke<void>("set_ui_context", { selection, playheadUs: Math.round(playheadUs) }),
  resolveRecovery: (action: "keep" | "restore", epoch: Epoch) => invoke<Snapshot>("resolve_recovery", { action, expectedEpoch: epoch }),
  /** Ends the agent's run, keeping its changes as one undo step; `discard` takes them back instead. */
  stopRun: (epoch: Epoch, discard = false) => invoke<Snapshot>("stop_run", { expectedEpoch: epoch, discard }),
  boot: () => invoke<Boot>("boot"),
  applyEdit: (cmd: EditCmd, coalesce: string | null, epoch: Epoch) => invoke<Snapshot>("apply_edit", { cmd, coalesce, expectedEpoch: epoch }),
  /** All or nothing, as one undo step. */
  applyEdits: (cmds: EditCmd[], coalesce: string | null, epoch: Epoch) => invoke<Snapshot>("apply_edits", { cmds, coalesce, expectedEpoch: epoch }),
  undo: (epoch: Epoch) => invoke<Snapshot>("undo", { expectedEpoch: epoch }),
  redo: (epoch: Epoch) => invoke<Snapshot>("redo", { expectedEpoch: epoch }),
  /** The newest kept versions of the open project, newest first. */
  listVersions: (limit: number) => invoke<ProjectVersion[]>("list_versions", { limit }),
  /** Restores a kept version as one undo step. */
  restoreVersion: (index: number, epoch: Epoch) => invoke<Snapshot>("restore_version", { index, expectedEpoch: epoch }),
  /** Paths of files whose clips with a background still wait for the person to be found. */
  mattesMissing: () => invoke<string[]>("mattes_missing"),
  importMedia: (paths: string[], epoch: Epoch) =>
    invoke<{ snapshot: Snapshot; added: string[]; failed: { path: string; error: string }[] }>("import_media", { paths, expectedEpoch: epoch }),
  thumbnail: (assetId: string) => invoke<string | null>("thumbnail", { assetId }),
  /** Waveform peaks `from..to`; while the sound is still being prepared, only the part decoded so far and `complete: false`. */
  waveform: (assetId: string, from: number, to: number) => invoke<{ peaks: number[]; complete: boolean } | null>("waveform", { assetId, from, to }),
  /** The gain Match loudness gives the clip in dB, or null until its file's loudness is measured. */
  matchGain: (clipId: string) => invoke<number | null>("match_gain", { clipId }),
  play: () => invoke<void>("play"),
  pause: () => invoke<void>("pause"),
  seek: (tUs: number) => invoke<void>("seek", { tUs: Math.round(tUs) }),
  setPreviewBox: (width: number, height: number) => invoke<void>("set_preview_box", { width, height }),
  setPreviewMuted: (muted: boolean) => invoke<void>("set_preview_muted", { muted }),
  listProjects: () => invoke<ProjectSummary[]>("list_projects"),
  agentConnections: () => invoke<AgentConnection[]>("agent_connections"),
  connectAgent: (agent: AgentKind) => invoke<AgentConnection>("connect_agent", { agent }),
  /** The `nuzky` server and a first message to paste into an agent Nuzky does not connect itself. */
  agentSetup: (setup: AgentSetup) => invoke<string>("agent_setup", { setup }),
  agentAccess: () => invoke<AgentAccess>("agent_access"),
  setAgentAccess: (on: boolean, atLaunch: boolean) => invoke<AgentAccess>("set_agent_access", { on, atLaunch }),
  /** `renew`: renews the sign-in first when it is about to expire. */
  chatgptStatus: (renew: boolean) => invoke<ChatGptStatus>("chatgpt_status", { renew }),
  /** Opens the browser and answers once it came back, was cancelled or waited 5 minutes. */
  chatgptSignIn: (anotherAccount: boolean) => invoke<ChatGptStatus>("chatgpt_sign_in", { anotherAccount }),
  chatgptCancelSignIn: () => invoke<void>("chatgpt_cancel_sign_in"),
  chatgptSignOut: () => invoke<ChatGptStatus>("chatgpt_sign_out"),
  /** The signed-in account's models, as ChatGPT orders them. */
  chatgptModels: () => invoke<ChatGptModel[]>("chatgpt_models"),
  newProject: (width: number, height: number) => invoke<Snapshot>("new_project", { width, height }),
  openProject: (path: string) => invoke<Snapshot>("open_project", { path }),
  /** A new project with the media on its timeline, in the format of the first picture. */
  newProjectFromMedia: (paths: string[]) =>
    invoke<{ snapshot: Snapshot; added: string[]; failed: { path: string; error: string }[] }>("new_project_from_media", { paths }),
  /** The CapCut draft in that folder as a new project, opened at once; the draft stays as it was. */
  openCapCutDraft: (path: string) => invoke<Snapshot>("open_capcut_draft", { path }),
  /** A PNG data URL of the picture CapCut keeps of a draft, or null. */
  capCutDraftCover: (path: string) => invoke<string | null>("capcut_draft_cover", { path }),
  library: () => invoke<Library>("library"),
  /** A PNG data URL of the project's picture, or null when it has none. */
  projectPoster: (path: string) => invoke<string | null>("project_poster", { path }),
  searchSaid: (query: string, collection: string | null = null) => invoke<Said>("search_said", { query, collection }),
  /** For a project that is not open; the open one is renamed with an edit. */
  renameProject: (path: string, name: string) => invoke<void>("rename_project", { path, name }),
  /** Returns the copy's path. */
  duplicateProject: (path: string) => invoke<string>("duplicate_project", { path }),
  trashProjects: (paths: string[]) => invoke<void>("trash_projects", { paths }),
  restoreProjects: (paths: string[]) => invoke<void>("restore_projects", { paths }),
  createCollection: (name: string) => invoke<Collection>("create_collection", { name }),
  renameCollection: (id: string, name: string) => invoke<void>("rename_collection", { id, name }),
  deleteCollection: (id: string) => invoke<DeletedCollection>("delete_collection", { id }),
  restoreCollection: (deleted: DeletedCollection) => invoke<void>("restore_collection", { deleted }),
  /** null takes the projects out of their collection. */
  setCollection: (paths: string[], collection: string | null) => invoke<void>("set_collection", { paths, collection }),
  /** The sound effects built into Nuzky. */
  soundLibrary: () => invoke<Sound[]>("sound_library"),
  /** One page of online results; only the query leaves the computer. */
  soundSearch: (query: string, kind: SoundKind, page: number) => invoke<SoundPage>("sound_search", { query, kind, page }),
  /** The video of a YouTube or YouTube Music link as a sound, looked up through the user's yt-dlp. */
  soundLink: (url: string) => invoke<Sound>("sound_link", { url }),
  /** Downloads yt-dlp from its GitHub releases; progress comes as `sound-progress` for "yt-dlp", `soundCancel("yt-dlp")` stops it. */
  installYtdlp: () => invoke<void>("install_ytdlp"),
  /** The user read that the rights to music from YouTube links are theirs; asked only once. */
  confirmYoutubeRights: () => invoke<SoundSettings>("confirm_youtube_rights"),
  /** Plays a sound, downloading it first when needed; `sound-preview-ended` names it when it is over. */
  soundPreview: (id: string) => invoke<PreviewStarted>("sound_preview", { id }),
  soundStop: () => invoke<void>("sound_stop"),
  /** Stops downloading a sound being added. */
  soundCancel: (id: string) => invoke<void>("sound_cancel", { id }),
  /** Adds the sound at `startUs` on an audio track, as one undo step. */
  soundAdd: (id: string, startUs: number, epoch: Epoch) => invoke<Snapshot>("sound_add", { id, startUs: Math.round(startUs), expectedEpoch: epoch }),
  /** Nuzky's own stickers and the emoji, in the order the Text tab shows them. */
  stickerCatalog: () => invoke<StickerCatalog>("sticker_catalog"),
  /** Keys of the emoji and stickers that match, best first. */
  stickerSearch: (query: string) => invoke<string[]>("sticker_search", { query }),
  /** PNG data URLs of `keys` drawn `px` wide, in the same order; "" for an unknown key. */
  stickerThumbs: (keys: string[], px: number) => invoke<string[]>("sticker_thumbs", { keys, px }),
  stickerAdd: (key: string, startUs: number, epoch: Epoch) => invoke<Snapshot>("sticker_add", { key, startUs: Math.round(startUs), expectedEpoch: epoch }),
  /** The credits the video's description needs; null when no CC BY sound is heard in it. */
  soundCredits: (epoch: Epoch) => invoke<string | null>("sound_credits", { expectedEpoch: epoch }),
  soundSettings: () => invoke<SoundSettings>("sound_settings"),
  /** `key`: a new Freesound key, "" to remove it, undefined to keep it. */
  setSoundSettings: (freesound: boolean, key?: string) => invoke<SoundSettings>("set_sound_settings", { freesound, key: key ?? null }),
  openSoundPage: (url: string) => invoke<void>("open_sound_page", { url }),
  /** Without `replaceExisting` an existing file is kept and the export fails with DESTINATION_EXISTS. */
  /** CC BY sounds bring `<name>.credits.txt`; without `replaceCredits` an existing one fails with CREDITS_EXIST. */
  startExport: (path: string, options: ExportRequest, epoch: Epoch, replaceExisting: boolean, replaceCredits = false) =>
    invoke<string>("start_export", { path, options, expectedEpoch: epoch, replaceExisting, replaceCredits }),
  filmstrip: (assetId: string) => invoke<Filmstrip | null>("filmstrip", { assetId }),
  layerBounds: (tUs: number) => invoke<LayerBounds[]>("layer_bounds", { tUs: Math.round(tUs) }),
  /** What a recap of an AI run, or with null of everything cut since the import, would count and how long it is. */
  recapInfo: (runId: string | null, epoch: Epoch) => invoke<RecapInfo>("recap_info", { runId, expectedEpoch: epoch }),
  startRecap: (out: string, runId: string | null, endCard: boolean, epoch: Epoch, replaceExisting: boolean) =>
    invoke<string>("start_recap", { out, runId, endCard, replaceExisting, expectedEpoch: epoch }),
  cancelJob: (id: string) => invoke<void>("cancel_job", { id }),
  speechModels: () => invoke<SpeechModel[]>("speech_models"),
  /** What Get set up needs to know to offer the models for `language`: the GPU, the word timing model, what is missing and the free space. */
  speechSetup: (language: string) => invoke<SpeechSetup>("speech_setup", { language }),
  /** Downloads everything the first transcript of `language` with `model` needs, as the `speech-model:setup` job; refused with NO_SPACE when it does not fit. */
  startSpeechSetup: (model: string, language: string) => invoke<string>("start_speech_setup", { model, language }),
  /** Download starts the speech model download an agent waits for; false tells the agent the user said not now. */
  answerSpeechModel: (id: string, download: boolean) => invoke<void>("answer_speech_model", { id, download }),
  /** Whether the face and person models covers need, or with "background" the person model of clip backgrounds,
   * can run here, are installed and what is left to download. */
  visionModels: (set?: "background") => invoke<VisionModels>("vision_models", { set }),
  /** Downloads the missing cover models (or the "background" set) as a `vision-models` job; `repair` downloads damaged ones again too. */
  startVisionModels: (repair = false, set?: "background") => invoke<string>("start_vision_models", { repair, set }),
  /** The cover of `format` `width` pixels wide, drawn as the export draws it, or `draft` in its place; `refreshMask` looks for a mask a job just made. */
  coverView: async (format: ThumbnailFormat, width: number, refreshMask: boolean, draft: Thumbnail | null) => {
    const buffer = await invoke<ArrayBuffer>("cover_view", { format, width: Math.round(width), refreshMask, draft });
    const length = new DataView(buffer).getUint32(0, true);
    const view = JSON.parse(new TextDecoder().decode(new Uint8Array(buffer, 4, length))) as CoverView;
    return { view, rgba: new Uint8ClampedArray(buffer, 4 + length, view.width * view.height * 4) };
  },
  /** Frees the cover renderer once the cover editor closes. */
  coverClose: () => invoke<void>("cover_close"),
  /** Pick for me: downloads the models on first use, picks frames for `format` and masks the person in the best, as a `cover` job. */
  startCoverPick: (format: ThumbnailFormat, epoch: Epoch) => invoke<string>("start_cover_pick", { format, expectedEpoch: epoch }),
  startReframe: (width: number, height: number, clipIds: string[] | null, epoch: Epoch) =>
    invoke<string>("start_reframe", { width, height, clipIds, expectedEpoch: epoch }),
  /** Masks the person in the frame at `timeUs`, downloading the mask model on first use. */
  startCoverMask: (timeUs: number, epoch: Epoch) => invoke<string>("start_cover_mask", { timeUs: Math.round(timeUs), expectedEpoch: epoch }),
  /** Without `replaceExisting` an existing file is kept and the export fails with DESTINATION_EXISTS. */
  startCoverExport: (format: ThumbnailFormat, path: string, replaceExisting: boolean, epoch: Epoch) =>
    invoke<string>("start_cover_export", { format, path, replaceExisting, expectedEpoch: epoch }),
  /** `maxWords`/`maxChars` null group by phrase, capped at 12 words and 42 characters. */
  startCaptions: (model: string, language: string, look: { style: TextStyle; animIn: Animation | null; animOut: Animation | null }, maxWords: number | null, maxChars: number | null, epoch: Epoch) =>
    invoke<string>("start_captions", { request: { model, language, ...look, maxWords, maxChars }, expectedEpoch: epoch }),
  /** The user's own caption styles, kept beside the projects; each change returns the list. */
  captionStyles: () => invoke<CaptionPreset[]>("caption_styles"),
  saveCaptionStyle: (style: CaptionPreset) => invoke<CaptionPreset[]>("save_caption_style", { style }),
  renameCaptionStyle: (from: string, to: string) => invoke<CaptionPreset[]>("rename_caption_style", { from, to }),
  deleteCaptionStyle: (name: string) => invoke<CaptionPreset[]>("delete_caption_style", { name }),
  /** Recognises the heard media without a transcript, or all of it with `refresh`. */
  startTranscript: (model: string, language: string, refresh: boolean, epoch: Epoch) => invoke<string>("start_transcript", { model, language, refresh, expectedEpoch: epoch }),
  transcriptView: (pauseUs: number) => invoke<TranscriptView>("transcript_view", { pauseUs: Math.round(pauseUs) }),
  /** Inclusive word index ranges, as one undo step. */
  cutWords: (key: string, ranges: [number, number][], epoch: Epoch) => invoke<TranscriptCut>("cut_words", { key, delete: ranges, expectedEpoch: epoch }),
  /** The view's pauses at `pauseUs`, by index, or all of them. */
  removePauses: (key: string, pauseUs: number, only: number[] | null, epoch: Epoch) =>
    invoke<TranscriptCut>("remove_pauses", { key, pauseUs: Math.round(pauseUs), only, expectedEpoch: epoch }),
  /** What the retake analysis suggests deleting and every pause longer than `pauseUs`, as one undo step. */
  removeRetakes: (key: string, pauseUs: number, epoch: Epoch) => invoke<TranscriptCut>("remove_retakes", { key, pauseUs: Math.round(pauseUs), expectedEpoch: epoch }),
  /** Words by view index and the text they should read, with the captions that show them, as one undo step. */
  correctWords: (key: string, corrections: { i: number; text: string }[], epoch: Epoch) =>
    invoke<WordsCorrected>("correct_words", { key, corrections, expectedEpoch: epoch }),
  listFonts: () => invoke<FontFamilies>("list_fonts"),
  /** Sentences of the timeline said with emphasis, for a punch-in. */
  suggestZooms: () => invoke<ZoomSuggestions>("suggest_zooms"),
  /** Punch-ins on inclusive word ranges of the view with `key`, as one undo step. */
  applyZooms: (key: string, zooms: { from: number; to: number; scale: number }[], epoch: Epoch) =>
    invoke<ZoomsApplied>("apply_zooms", { key, zooms, expectedEpoch: epoch }),
  /** The reel candidate with the words `from` to `to`, measured as making it would cut it; nothing changes. */
  reelBounds: (id: string, from: number, to: number) => invoke<ReelCandidate>("reel_bounds", { id, from, to }),
  /** A project of its own beside this one for each reel candidate, as a `reels` job; marked made here as one undo step. */
  startMakeReels: (ids: string[], canvas: ReelFraming, epoch: Epoch) => invoke<string>("start_make_reels", { ids, canvas, expectedEpoch: epoch }),
  /** A newer released version, or null when this one is the latest. */
  checkForUpdate: () => invoke<string | null>("check_for_update"),
  openReleasePage: () => invoke<void>("open_release_page"),
  /** The issue form for a bug or an idea with this version and system filled in, or the author's X profile, in the browser. */
  openFeedback: (kind: FeedbackKind) => invoke<void>("open_feedback", { kind }),
  /** The version, system, graphics and the end of the log, without folders, typed text or secrets. */
  diagnostics: () => invoke<string>("diagnostics"),
  openLogs: () => invoke<void>("open_logs"),
  /** Writes an error of the interface into the log. */
  logUiError: (message: string) => invoke<void>("log_ui_error", { message }),
  styleView: () => invoke<StyleView>("style_view"),
  styleAct: (action: StyleAction) => invoke<StyleView>("style_act", { action }),
  /** Learns from 1 to 3 recordings, each with the video cut from it, as a `style` job. */
  startStyleLearning: (pairs: StylePair[]) => invoke<string>("start_style_learning", { pairs }),
  /** What each timeline file from another editor offers to learn from, read at once. */
  styleReadTimelines: (paths: string[]) => invoke<TimelinePlan[]>("style_read_timelines", { paths }),
  /** CapCut's drafts, newest first: where CapCut keeps them on this computer, or in `folder`, which may be one draft. */
  styleCapCutDrafts: (folder: string | null) => invoke<CapCutDraft[]>("style_capcut_drafts", { folder }),
  /** Learns from timelines cut in another editor as a `style` job, which ends with EDIT.md as it would be. */
  startTimelineLearning: (paths: string[]) => invoke<string>("start_timeline_learning", { paths }),
  /** Makes what that job learned the style, as it showed it; STYLE_CHANGED when the style changed since. */
  styleUseLearned: (jobId: string) => invoke<StyleView>("style_use_learned", { jobId }),
};

/** The error as the backend sent it, "CODE: detail" included; code checks use this. */
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

const sentence = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** What the backend's error codes mean for someone editing, with what to do next. */
const PLAIN: Record<string, string | ((detail: string) => string)> = {
  READ_ONLY: "This project cannot be changed in this window. Open it again from Projects.",
  RECOVERY_PENDING: "Choose what happens to the unfinished AI edit first.",
  RUN_ACTIVE: "AI is editing. Stop it to edit yourself.",
  RUN_BUSY: "An AI edit is still open. Stop it first.",
  EPOCH_CHANGED: "Another project was opened meanwhile, so this did not happen.",
  EDIT_REJECTED: (detail) => {
    const what = detail.replace(/; project unchanged$/, "").replace(/\.$/, "");
    // A clip, track or media item that went away meanwhile, e.g. through Undo or an AI edit.
    const gone = /^unknown (clip|track|asset|media)\b/i.exec(what);
    if (gone) return `That ${gone[1] === "asset" ? "media item" : gone[1].toLowerCase()} was removed meanwhile, so nothing changed.`;
    return `${sentence(what)}. The project is unchanged.`;
  },
  INVALID_PROJECT: (detail) => `That change would break the project (${detail}), so it was not made.`,
  SPEECH_CHANGED: "The speech on the timeline changed meanwhile. Select the words again and retry.",
  TRANSCRIPT_MISSING: "Transcribe the timeline first, then try again.",
  AUDIO_NOT_READY: "The sound of the clips is still being prepared. Try again in a moment.",
  NO_WORDS: "Transcribe the timeline first, then try again.",
  OUTPUT_EXISTS: "A file with that name appeared while exporting. Export again to replace it or choose another name.",
  DESTINATION_EXISTS: "A file with that name already exists.",
  NOT_WRITABLE: "Nuzky isn't allowed to save in that folder. Export to another folder.",
  UNKNOWN_VERSION: "That version is no longer kept. Open the list of versions again.",
  RUN_UNAVAILABLE: "The project as it was before or after this AI run is no longer kept, so its recap can't be made.",
  HISTORY_CORRUPT: "That version could not be read, so the project is unchanged.",
  CANCELLED: "Cancelled.",
  AGENT_BUSY: (detail) => detail,
  NOT_INSTALLED: (detail) => sentence(detail.replace(/\.$/, "")) + ". Install it and sign in from a terminal.",
  APP_CLOSED: "Nuzky is closing, so this stopped.",
  JOB_FAILED: "The task stopped unexpectedly. Try again.",
  CRASHED: "Nuzky hit a bug and stopped. Try again, and if it happens again, report it with Send feedback, the speech bubble next to Projects.",
  DOWNLOAD_FAILED: (detail) => `The ${detail.split(":")[0]} could not be downloaded. Check the internet connection and free disk space, then try again.`,
  MODEL_INVALID: "The model did not download completely. Try again to download it anew.",
  NO_SPACE: (detail) => `There isn't enough free space for the download. Free up ${detail} more on this computer, then try again.`,
  MEDIA_MISSING: (detail) => `${detail.split(/[\\/]/).pop()} is missing. Put the file back where it was, then try again.`,
  VISION_UNAVAILABLE:
    "Faces and people can't be found on this computer. Covers still let you choose the frame and add text, and a clip's background stays None.",
  MODEL_MISSING: (detail) =>
    detail.startsWith("person outline (selfie-segmenter.onnx) not installed")
      ? "The background needs the person model. Download it in the clip's Background section."
      : "The face and person models are not installed. Pick for me downloads them.",
  THUMBNAIL_MISSING: "This project has no cover in that format yet.",
  INVALID_RANGE: "The cover's frame is past the end of the video now. Choose its frame again.",
  STORE_UNREADABLE: "The saved transcripts could not be read. Transcribe the timeline again.",
  NO_MATCH: (detail) => `${sentence(detail.replace(/, so it was not cut from it$/, ""))}. Skipped.`,
  STYLE_CHANGED: "Your style changed in another window meanwhile. Copy your text, open the style again and redo your change.",
  NOT_SUGGESTED: "That suggestion is gone; the style was learned again meanwhile.",
  NOTHING_TO_REVERT: "That rule has not changed, so there is nothing to put back.",
  UNKNOWN_RULE: "That rule is no longer in your style.",
  TOO_LONG: (detail) => sentence(detail) + ".",
  // The sound library's errors already say what happened; a technical reason in brackets is dropped.
  OFFLINE: (detail) => detail.replace(/ \(.*\)$/, ""),
  RATE_LIMITED: (detail) => detail,
  SOURCE_REFUSED: (detail) => detail,
  SOURCE_FAILED: (detail) => detail.replace(/ \(.*\)$/, ""),
  SOUND_UNAVAILABLE: (detail) => detail.replace(/ \(.*\)$/, ""),
  UNLICENSED: (detail) => sentence(detail) + ".",
  KEY_REJECTED: (detail) => `${detail} Check it under Where to search, the sliders button beside the search field.`,
  KEY_MISSING: "Add your Freesound API key under Where to search, the sliders button beside the search field.",
  INVALID_KEY: "A Freesound API key has only letters and digits. Copy it again from Freesound.",
  INVALID_LINK: "That isn't a link to a YouTube or YouTube Music video. Copy the link from the video's Share button.",
  PLAYLIST: "That's a link to a playlist. Open one song and copy its link instead.",
  YTDLP_MISSING: "Music from YouTube needs yt-dlp. Download it under the link field in the Music tab.",
  YTDLP_UNSUPPORTED: (detail) => detail,
  YTDLP_CORRUPT: (detail) => detail,
  YTDLP_DOWNLOAD_FAILED: (detail) => detail.replace(/ \(.*\)$/, ""),
  YTDLP_FAILED: "yt-dlp is installed but didn't start. Reinstall it, then try again.",
  NOT_LEARNED: "What was learned is gone. Learn from the projects again.",
  SIGN_IN_DENIED: "Signing in was cancelled in the browser. Continue with ChatGPT to try again.",
  SIGN_IN_TIMEOUT: "Nuzky stopped waiting for the browser after 5 minutes. Continue with ChatGPT to try again.",
  SIGN_IN_FAILED: (detail) => sentence(detail.replace(/\.$/, "")) + ".",
  WRONG_ACCOUNT: (detail) => `That's another ChatGPT account. ${detail}`,
  KEYCHAIN_UNAVAILABLE:
    "Nuzky keeps your ChatGPT sign-in in the system keychain and couldn't use it. Allow Nuzky when the system asks. On Linux, start GNOME Keyring or KWallet, then restart Nuzky.",
};

/**
 * Text for people: a known "CODE: detail" becomes plain words with a next step, wherever it sits in
 * the message ("Export failed: CODE: …"); anything else is shown as it came.
 */
export function plainError(text: string): string {
  return text.replace(/\b([A-Z][A-Z_]{3,}): ?([\s\S]*)$/, (all, code: string, detail: string) => {
    const plain = PLAIN[code];
    return plain === undefined ? all : typeof plain === "string" ? plain : plain(detail);
  });
}

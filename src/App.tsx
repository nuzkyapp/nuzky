import { Suspense, lazy, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { AlertCircle } from "lucide-react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { api, errorText, plainError } from "./lib/api";
import { setLimits } from "./lib/limits";
import { onFacesJob } from "./lib/faces";
import { onReframeJob } from "./lib/reframe";
import { canRetrySpeech, retrySpeech, useSpeech } from "./lib/speech";
import { useNames } from "./lib/names";
import { reportBug } from "./lib/diagnostics";
import { currentEpoch, PLUGIN_RUN, runWasDiscarded, undoAction, useEditor } from "./lib/store";
import { formatDuration } from "./lib/time";
import { MOVES_SOURCE, RESTS_IN_COVER, RESTS_IN_FULL, WORKS_IN_REELS, isClipboard, runAction } from "./lib/actions";
import type { Asset, JobEvent, ProjectSummary, Snapshot, ModelQuestion, Transport } from "./lib/types";
import { AGENT_VISION_MODELS } from "./components/ModelQuestionDialog";
import { showsVisionDownload } from "./components/setup/steps/VisionStep";
import { useYoutube } from "./lib/youtube";
import { importLutPaths, useLuts } from "./lib/color";
import { MEDIA_EXTENSIONS, importPaths, mediaKind, resolveDrop } from "./components/panel/assets";
import { Toasts } from "./components/Toasts";
import { Tour } from "./components/Tour";
import { listenAgentEvents, panelRunEnded, useAgent } from "./lib/agent";
import { listenRemote, useRemote } from "./lib/remote";
import { togglePanel, useDock, useDockLayout, useWindowSize } from "./lib/dock";
import { checkYtdlp, useSetup } from "./lib/setup";
import { useTour } from "./lib/tour";
import { paneLayout, usePanes } from "./lib/layout";
import { Button, DisabledHint, Tooltip } from "./components/ui";
import { Home } from "./components/home/Home";
import { openDraft, useCapCut, openDraftFolder } from "./lib/capcut";
import { newProjectFromMedia, openProject, refreshLibrary, showHome, trashProjects, useLibrary } from "./lib/library";
import { checkForUpdates, startUpdates, useUpdates } from "./lib/updates";
import { useSounds } from "./lib/sounds";
import { useStickers } from "./lib/stickers";
import { useGallery } from "./lib/gallery";
import { listenEffects, listenPlugins, runPlugin, usePlugins, warnExported } from "./lib/plugins";
import { exportVideo } from "./components/ExportDialog";
import { addProjects, findDrafts, listenStyle, loadStyle, useStyle } from "./lib/style";
import { closeCover, closeExport, deleteOverlay, deleteText, loadModels as loadVisionModels, onCoverJob, selectedOverlay, useCover } from "./lib/cover";
import { answerNextSave, savesAsked } from "./lib/exportFolder";
import { candidateOf, closeReels, onReelsJob, openReelsToFind, playEnding, toggleReelPlay, useReels } from "./lib/reels";
import { onBeatsFound } from "./lib/beats";
import { closeSource, useSource } from "./lib/source";
import { appWindow, toggleFullPreview, usePreview } from "./lib/preview";
import { titleWindow } from "./lib/window";
import { actionOf, chordsOf, useKeys } from "./lib/keys";
import { useKeysQuestion } from "./lib/keysQuestion";
import { openSettings, useSettings } from "./lib/settings";
import { useMicLevel, useVoiceover } from "./lib/voiceover";

const isMedia = (path: string) => MEDIA_EXTENSIONS.includes(path.split(".").pop()?.toLowerCase() ?? "");

const TEXT_INPUTS = new Set(["text", "search", "email", "number", "password", "url", "tel"]);

function isTyping(el: HTMLElement | null) {
  if (!el) return false;
  if (el.isContentEditable || el.tagName === "TEXTAREA" || el.tagName === "SELECT") return true;
  return el.tagName === "INPUT" && TEXT_INPUTS.has((el as HTMLInputElement).type);
}

/** Controls whose own Space action (toggle, pick a menu item) wins over play and pause. */
function usesSpace(el: HTMLElement | null) {
  if (!(el instanceof Element)) return false;
  if (el instanceof HTMLInputElement && (el.type === "checkbox" || el.type === "radio")) return true;
  return /^(checkbox|radio|switch|menuitem|menuitemradio|menuitemcheckbox|option)$/.test(el.getAttribute("role") ?? "");
}

/** Swallows the next Space release, which would otherwise click the focused button. */
function suppressSpaceRelease() {
  const up = (e: KeyboardEvent) => {
    if (e.key !== " ") return;
    e.preventDefault();
    window.removeEventListener("keyup", up, true);
  };
  window.addEventListener("keyup", up, true);
}

function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      const s = useEditor.getState();
      // Settings open from anywhere, as in other apps.
      if (chordsOf(e).includes("Ctrl+,") && !document.querySelector("dialog[open], [aria-modal='true']")) {
        e.preventDefault();
        openSettings();
        return;
      }
      // The home screen and the launcher have their own keys. While an update installs, a key would change the project it is saving.
      if (s.view === "home" || s.launcherOpen || s.snap?.recovery || useUpdates.getState().install?.stage === "installing") return;
      const action = actionOf(chordsOf(e));
      // The AI panel's and the launcher's keys also work from a text field, but only with a modifier, so typing stays typing.
      const typed = isTyping(target) && !(e.ctrlKey || e.metaKey || e.altKey);
      if (action === "aiPanel" && !typed) {
        e.preventDefault();
        togglePanel();
        return;
      }
      if (action === "projects" && !typed && !document.querySelector("dialog[open]")) {
        e.preventDefault();
        useEditor.setState({ launcherOpen: true });
        return;
      }
      // The editor's keys stay with an open dialog.
      if (isTyping(target) || document.querySelector("dialog[open], [aria-modal='true']")) return;
      const escape = e.key === "Escape";
      // The transcript's words are not clips, and selected text, such as an error message, copies as text.
      if (action && isClipboard(action) && (target?.closest("#library-panel-transcript") || (action === "copy" && window.getSelection()?.toString()))) return;
      // A focused slider keeps its own arrow, Home and End keys.
      if (target instanceof HTMLInputElement && target.type === "range" && /^(Arrow|Home$|End$|Page)/.test(e.key)) return;
      // In the cover editor the playhead is the cover's frame and nothing plays; the clip keys rest.
      const cover = useCover.getState();
      if (cover.open) {
        if (escape) {
          if (cover.selected !== null) useCover.setState({ selected: null });
          else closeCover();
          return;
        }
        const overlay = selectedOverlay(cover.selected);
        if (action === "delete" && (typeof cover.selected === "number" || overlay !== null)) {
          e.preventDefault();
          void (typeof cover.selected === "number" ? deleteText(cover.open, cover.selected) : deleteOverlay(cover.open, overlay!));
          return;
        }
        if (action && RESTS_IN_COVER.has(action)) {
          e.preventDefault();
          return;
        }
      }
      // Choosing reels plays the current reel; undo, redo and frame steps work as in the editor, the clip keys rest.
      const reels = useReels.getState();
      if (reels.open && !(action && WORKS_IN_REELS.has(action))) {
        const reel = candidateOf(s.snap?.project, reels.current);
        if (escape) closeReels();
        else if (action === "playPause" && !(e.key === " " && usesSpace(target))) {
          e.preventDefault();
          if (!e.repeat) {
            if (e.key === " ") suppressSpaceRelease();
            toggleReelPlay();
          }
        } else if (action === "goStart" && reel) s.seek(reel.startUs);
        else if (action === "goEnd") playEnding();
        return;
      }
      // A media item in the preview takes the playback keys; Esc shows the timeline again.
      const source = useSource.getState();
      if (source.asset) {
        if (escape && !usePreview.getState().full) {
          closeSource();
          return;
        }
        if (action && MOVES_SOURCE.has(action)) {
          if (action !== "goStart" && action !== "goEnd") e.preventDefault();
          runAction(action);
          return;
        }
      }
      if (escape) {
        if (usePreview.getState().full) void toggleFullPreview();
        else s.select([]);
        return;
      }
      if (!action) return;
      // Full screen hides the timeline; the keys that would change it unseen rest.
      if (usePreview.getState().full && RESTS_IN_FULL.has(action)) {
        e.preventDefault();
        return;
      }
      if (action === "cutPrev" || action === "cutNext") {
        // A focused clip keeps these keys to move between tracks.
        if (cover.open || e.defaultPrevented || !s.snap) return;
        e.preventDefault();
        return runAction(action);
      }
      if (e.key === " " && usesSpace(target)) return;
      e.preventDefault();
      // A focused button would also be clicked when Space is released; that release is held back instead of taking
      // focus away, so the keyboard user stays where they were. Holding Space acts once.
      if (e.key === " ") {
        if (e.repeat) return;
        suppressSpaceRelease();
      }
      runAction(action, e.repeat);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

function onJob(job: JobEvent) {
  const { toast, exportOpen, exportJobId } = useEditor.getState();
  useEditor.setState({ jobs: { ...useEditor.getState().jobs, [job.id]: job } });
  if (job.kind === "cover" || job.kind === "vision-models") onCoverJob(job);
  if (job.kind === "reframe") onReframeJob(job);
  if (job.kind === "faces") onFacesJob(job);
  if (job.kind === "reels") onReelsJob(job);
  if (job.kind === "beats" && job.status === "done") onBeatsFound(job.id.slice("beats:".length));
  if (job.kind === "beats" && job.status === "failed") toast({ kind: "error", text: `The beat could not be found: ${plainError(job.message ?? "")}` });
  if (job.kind === "captions" && job.status === "done") toast({ kind: "success", text: `Added ${job.output ?? "captions"}` });
  // The dialog says what an export was drawn without; with it closed a toast does, unless one already names it.
  if (job.kind === "export" && job.status === "done" && job.message && !exportOpen) warnExported(job.message);
  // The panel shows the failure too, with Open logs folder. In another project the toast goes.
  const retry = { label: "Try again", run: () => retrySpeech(job.id), valid: () => canRetrySpeech(job.id) };
  if (job.kind === "captions" && job.status === "failed") toast({ kind: "error", text: `Captions failed: ${plainError(job.message ?? "")}`, action: retry });
  if (job.kind === "transcript" && job.status === "done") toast({ kind: "success", text: `Transcript ready: ${job.output ?? "words"}` });
  if (job.kind === "transcript" && job.status === "failed") toast({ kind: "error", text: `Transcript failed: ${plainError(job.message ?? "")}`, action: retry });
  if (job.kind === "audio" && job.status === "failed") toast({ kind: "error", text: `${job.label} failed: ${plainError(job.message ?? "")}` });
  // Get set up's download finishes its step, also when the step was skipped or Get set up closed meanwhile.
  if (job.id === "speech-model:setup" && job.status === "done") useSetup.getState().finishStep("captions");
  // Captions model in Get set up shows its own download's failure with Try again while it is on screen.
  const setup = useSetup.getState();
  if (job.kind === "speech-model" && job.status === "failed" && !(job.id === "speech-model:setup" && setup.open && setup.current === "captions"))
    toast({ kind: "error", text: plainError(job.message ?? "") });
  if (job.kind === "proxy" && job.status === "failed")
    toast({ kind: "error", text: `${job.label} failed: ${plainError(job.message ?? "")}. The preview plays the original file, which can stutter.` });
  if (job.kind === "matte" && job.status === "failed")
    toast({ kind: "error", text: `${job.label} failed: ${plainError(job.message ?? "")}. The clip shows as recorded; choose None for its background to export it.` });
  // The cover editor reports its own model downloads.
  if (job.kind === "vision-models" && job.label === "Person model" && job.status === "failed")
    toast({
      kind: "error",
      text: `${job.label} failed: ${plainError(job.message ?? "")}`,
      action: { label: "Try again", run: () => api.startVisionModels(false, "background").catch((e) => toast({ kind: "error", text: plainError(errorText(e)) })) },
    });
  // Get set up and the cover editor show their own download's failure while they are on screen.
  if (job.kind === "vision-models" && job.label === "Cover models" && job.status === "failed" && !showsVisionDownload(job.id) && !useCover.getState().open)
    toast({ kind: "error", text: `Face and person models failed: ${plainError(job.message ?? "")}`, action: { label: "Get set up", run: () => (useSetup.getState().showStep("vision"), useSetup.getState().openSetup()) } });
  // The AI went on without them; Get set up downloads them later.
  if (job.kind === "vision-models" && job.label === AGENT_VISION_MODELS && job.status === "failed")
    toast({
      kind: "error",
      text: `Face and person models failed: ${plainError(job.message ?? "")}`,
      action: { label: "Get set up", run: () => (useSetup.getState().showStep("vision"), useSetup.getState().openSetup()) },
    });
  // The AI went on without it; it asks again with the next message.
  if (job.kind === "scene-model" && job.status === "failed") toast({ kind: "error", text: `Scene model failed: ${plainError(job.message ?? "")}` });
  // A cover shows the background once the person behind whom it goes is found.
  if (job.kind === "matte" && job.status === "done") useCover.setState((s) => ({ masks: s.masks + 1 }));
  // The dialog shows the result itself; with it closed, a toast reports it.
  if (job.kind === "export" && job.id === exportJobId && !exportOpen) {
    if (job.status === "done") {
      toast({ kind: "success", text: "Export finished", action: job.output ? { label: "Show in folder", run: () => revealItemInDir(job.output!) } : undefined });
      useEditor.setState({ exportJobId: null });
    }
    if (job.status === "failed") toast({ kind: "error", text: `Export failed: ${plainError(job.message ?? "")}`, action: { label: "Details", run: () => useEditor.setState({ exportOpen: true }) } });
  }
}

function useBackendEvents() {
  useEffect(() => {
    const offs: Promise<() => void>[] = [];
    offs.push(
      listen<Transport>("transport", (e) => {
        // The timeline playing, from the phone or the AI panel, shows the timeline again.
        if (e.payload.playing) closeSource();
        useEditor.setState({ playing: e.payload.playing, timeUs: e.payload.tUs });
      }),
    );
    offs.push(listen<Transport>("source-transport", (e) => useSource.setState({ playing: e.payload.playing, timeUs: e.payload.tUs })));
    offs.push(listen<string | null>("source-preview-error", (e) => useSource.setState({ error: e.payload })));
    offs.push(
      listen<{ revision: number; error: string | null }>("saved", (e) => {
        const snap = useEditor.getState().snap;
        if (e.payload.error) useEditor.setState({ saveState: "error" });
        else if (!snap || e.payload.revision >= snap.revision) useEditor.setState({ saveState: "saved" });
      }),
    );
    offs.push(listen<JobEvent>("job", (e) => onJob(e.payload)));
    offs.push(listen<ModelQuestion | null>("speech-model-question", (e) => useEditor.setState({ speechModelQuestion: e.payload })));
    offs.push(listen<ModelQuestion | null>("vision-model-question", (e) => useEditor.setState({ visionModelQuestion: e.payload })));
    offs.push(listen<ModelQuestion | null>("scene-model-question", (e) => useEditor.setState({ sceneModelQuestion: e.payload })));
    // Whether Get set up still offers the face and person models.
    void loadVisionModels();
    const offStyle = listenStyle();
    void loadStyle();
    offs.push(listen("transcripts-changed", () => useSpeech.setState((s) => ({ stored: s.stored + 1 }))));
    offs.push(listenPlugins());
    offs.push(listenEffects());
    offs.push(
      listen<string>("audio-ready", (e) => {
        useEditor.getState().reloadWaveform(e.payload);
        // Hesitations are heard in the sound.
        useSpeech.setState((s) => ({ stored: s.stored + 1 }));
      }),
    );
    offs.push(listen<Snapshot>("project-changed", (e) => useEditor.getState().setSnap(e.payload, true)));
    // null: the preview recovered, e.g. after a failed frame.
    offs.push(listen<string | null>("engine-error", (e) => useEditor.setState({ engineError: e.payload })));
    let hasMedia = false;
    let single: Asset["kind"] | null = null;
    let several = false;
    // The run seen last, also one already open at boot; a snapshot can clear `aiRun` before this event.
    let lastRun: string | null = useEditor.getState().aiRun;
    // The revision the run started from, unknown for a run already open at boot.
    let runStart: number | null = null;
    const unwatch = useEditor.subscribe((s, prev) => {
      // A change to the timeline, choosing a clip on it or an AI run shows the timeline, so no edit happens unseen;
      // going home stops the media item, which would play on with no transport.
      if ((s.aiRun && !prev.aiRun) || s.snap?.revision !== prev.snap?.revision || (s.selection !== prev.selection && s.selection.length > 0) || (s.view === "home" && prev.view !== "home")) closeSource();
      if (s.aiRun) lastRun = s.aiRun;
      if (s.aiRun && !prev.aiRun) {
        runStart = prev.snap ? (s.snap?.revision ?? null) : null;
        // A new run has nothing taken back yet, even if taking back an earlier one found no run.
        runWasDiscarded();
      }
    });
    offs.push(
      listen<string | null>("run-changed", (e) => {
        const { toast, snap } = useEditor.getState();
        const aiRun = lastRun;
        lastRun = e.payload;
        useEditor.setState({ aiRun: e.payload });
        // The run's last change arrived before this event, so Undo here removes the whole run; a run
        // that changed nothing offers none, as it would undo the step before it.
        const changed = !!snap && snap.revision !== runStart;
        // A plugin run from the Plugins tab says how it went itself; an agent's run of the same name does not.
        if (aiRun && !e.payload && !(aiRun.startsWith(PLUGIN_RUN) && usePlugins.getState().running)) {
          const discarded = runWasDiscarded();
          // A run the AI panel's agent made shows there, with what it changed and Undo; others get a toast.
          // A run the user took back offers no Undo: it would take back the change before it.
          if (!panelRunEnded(aiRun, snap && changed && !discarded ? snap : null)) {
            if (discarded) toast({ kind: "info", text: `Undid the AI edit: ${aiRun}` });
            else toast({ kind: "success", text: `AI edit done: ${aiRun}`, action: snap && changed ? undoAction(snap) : undefined });
          }
        }
      }),
    );
    // A project moved to the Trash that someone opened meanwhile stays; it shows again.
    offs.push(
      listen<string>("trash-failed", (e) => {
        useEditor.getState().toast({ kind: "error", text: e.payload });
        void refreshLibrary();
      }),
    );
    offs.push(
      listen("file-dialog-hidden", () =>
        useEditor.getState().toast({ kind: "error", text: "macOS didn't show the window for choosing a file. Quit Nuzky, open it again and try once more." }),
      ),
    );
    // A reloaded page starts on the projects, while the backend still remembers the editor.
    void api.setEditorShown(useEditor.getState().view === "editor");
    // The editor's own dialogs would cover the projects; an export goes on without its dialog.
    offs.push(
      listen("close-to-home", () => {
        closeExport();
        useYoutube.setState({ open: false, url: "" });
        useCapCut.setState({ opened: null });
        showHome();
      }),
    );
    offs.push(listen("close-while-installing", () => useEditor.getState().toast({ kind: "info", text: "Installing the update. Nuzky restarts in a moment." })));
    offs.push(
      listen<string>("close-save-failed", (e) =>
        useEditor.getState().toast({ kind: "error", text: `Your last changes could not be saved: ${e.payload}. Free some disk space and close again, or close again to quit without them.` }),
      ),
    );
    offs.push(
      getCurrentWebview().onDragDropEvent((e) => {
        const dpr = window.devicePixelRatio || 1;
        const p = e.payload;
        // While media files are dragged over the window, the timeline shows where they would land.
        if (p.type === "enter") {
          const media = p.paths.filter(isMedia);
          hasMedia = media.length > 0;
          // Several files of one kind, such as songs, are offered only the places that kind can go.
          single = new Set(media.map(mediaKind)).size === 1 ? mediaKind(media[0]) : null;
          several = media.length > 1;
        }
        if ((p.type === "enter" || p.type === "over") && hasMedia) useEditor.setState({ fileDrag: { x: p.position.x / dpr, y: p.position.y / dpr, kind: single, several } });
        if (p.type === "leave" || p.type === "drop") useEditor.setState({ fileDrag: null });
        if (p.type !== "drop" || useEditor.getState().snap?.recovery) return;
        // On the home screen a dropped project opens, and dropped videos start a new project.
        if (useEditor.getState().view === "home") {
          const project = p.paths.find((path) => path.toLowerCase().endsWith(".nuzky"));
          const media = p.paths.filter(isMedia);
          if (project) return openProject({ path: project, name: project.split(/[\\/]/).pop()?.replace(/\.nuzky$/i, "") ?? "project" });
          if (media.length > 0) return newProjectFromMedia(media);
        }
        const paths = p.paths.filter(isMedia);
        if (paths.length === 0) {
          useEditor.getState().toast({ kind: "error", text: "Those files are not video, audio or images Nuzky can open." });
          return;
        }
        // The same as while dragging, so a drop lands where the highlight said.
        const target = resolveDrop(p.position.x / dpr, p.position.y / dpr, single ? { kind: single, several } : null) ?? undefined;
        importPaths(paths, target);
      }),
    );
    return () => {
      unwatch();
      offStyle();
      offs.forEach((p) => p.then((off) => off()));
    };
  }, []);
}

const UI_CONTEXT_MS = 250;

/**
 * Tells agents what is selected and where the playhead is: at most every 250 ms, ending on the
 * latest values, also while playing. It follows the store directly, so the playhead moving never
 * re-renders the app.
 */
function useUiContext() {
  useEffect(() => {
    let timer = 0;
    let lastSent = 0;
    const send = () => {
      timer = 0;
      lastSent = Date.now();
      const { selection, timeUs, snap } = useEditor.getState();
      if (!snap) return;
      void api.setUiContext(selection, timeUs).catch((e) => useEditor.getState().toast({ kind: "error", text: errorText(e) }));
    };
    const unsubscribe = useEditor.subscribe((s, prev) => {
      if (timer || !s.snap?.sessionEpoch) return;
      if (s.selection === prev.selection && s.timeUs === prev.timeUs && s.snap.sessionEpoch === prev.snap?.sessionEpoch) return;
      timer = window.setTimeout(send, Math.max(0, UI_CONTEXT_MS - (Date.now() - lastSent)));
    });
    return () => {
      unsubscribe();
      window.clearTimeout(timer);
    };
  }, []);
}

function RecoveryDialog() {
  const dialog = useRef<HTMLDialogElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    dialog.current?.showModal();
    dialog.current?.querySelector<HTMLButtonElement>("[data-autofocus]")?.focus();
  }, []);
  const resolve = async (action: "keep" | "restore") => {
    setBusy(true);
    setError(null);
    try {
      const snap = await api.resolveRecovery(action, currentEpoch());
      useEditor.getState().setSnap(snap, true);
      useEditor.setState({ saveState: "saved" });
      if (action === "restore") useEditor.getState().toast({ kind: "success", text: "Previous version restored", action: undoAction(snap) });
    } catch (e) {
      setError(plainError(errorText(e)));
      setBusy(false);
    }
  };
  return (
    <dialog ref={dialog} onCancel={(e) => e.preventDefault()} aria-labelledby="recovery-title" aria-describedby="recovery-description" className="overlay fixed inset-0 m-auto w-[440px] rounded-[20px] p-6 text-fg backdrop:bg-black/45">
      <h2 id="recovery-title" className="text-[17px] font-semibold">An AI edit didn't finish</h2>
      <p id="recovery-description" className="mt-3 text-[13px] text-muted">Keep the changes it made, or go back to the version before it started?</p>
      {error && <p role="alert" className="mt-3 flex items-start gap-2 text-[13px] text-danger"><AlertCircle size={16} className="shrink-0" />{error}</p>}
      <div className="mt-6 flex justify-end gap-2">
        <Button pill disabled={busy} onClick={() => void resolve("restore")}>Restore previous version</Button>
        <Button pill variant="primary" data-autofocus disabled={busy} onClick={() => void resolve("keep")}>Keep changes</Button>
      </div>
    </dialog>
  );
}

// While the video plays, overlays drop their blur (index.css): a blur over moving pictures is redone every frame.
useEditor.subscribe((s, prev) => {
  if (s.playing !== prev.playing) document.documentElement.toggleAttribute("data-playing", s.playing);
});

useEditor.subscribe((s, prev) => {
  if (s.view === prev.view) return;
  void api.setEditorShown(s.view === "editor");
  if (s.view === "editor") void api.maximizeWindow();
});

// Test hook for WebDriver runs; native file dialogs cannot be automated.
if (import.meta.env.DEV)
  Object.assign(window, {
    __nuzky: { importPaths, importLutPaths, luts: useLuts, exportVideo, plugins: usePlugins, runPlugin, gallery: useGallery, store: useEditor, cover: useCover, reels: useReels, openReelsToFind, speech: useSpeech, names: useNames, api, agent: useAgent, dock: useDock, setup: useSetup, library: useLibrary, style: useStyle, addProjects, findDrafts, newProjectFromMedia, capcut: useCapCut, openDraft, openDraftFolder, openProject, refreshLibrary, trashProjects, updates: useUpdates, checkForUpdates, sounds: useSounds, stickers: useStickers, tour: useTour, youtube: useYoutube, previewWindow: appWindow, remote: useRemote, source: useSource, keys: useKeys, keysQuestion: useKeysQuestion, settings: useSettings, openSettings, voiceover: useVoiceover, micLevel: useMicLevel, answerNextSave, savesAsked },
  });

async function startEditor() {
  const boot = await api.boot();
  setLimits(boot.limits);
  useEditor.setState({ previewUrl: boot.previewUrl, playing: boot.transport.playing, timeUs: boot.transport.tUs });
  if (boot.engineError) useEditor.setState({ engineError: boot.engineError });
  useEditor.getState().setSnap(boot.snapshot, true, true);
  useEditor.setState({ saveState: "saved" });
  startUpdates(boot.version, boot.updateChecks, boot.updateNote);
  void checkYtdlp();
  // A warning stays until dismissed, so the reason a different project opened is not missed.
  // Once per launch: StrictMode and Try again can start the editor more than once.
  if (boot.startupNotice && !startupNoticeShown) {
    startupNoticeShown = true;
    useEditor.getState().toast({ kind: "warning", text: boot.startupNotice });
  }
  // The project opens as Nuzky last saved it, about a second after the last edit; an unfinished AI edit asks itself.
  if (boot.crashed && !crashNoticeShown) {
    crashNoticeShown = true;
    useEditor.getState().toast({
      kind: "warning",
      text: "Nuzky closed unexpectedly. Your project is as it was last saved.",
      action: { label: "Report a bug…", run: reportBug },
    });
  }
}

let startupNoticeShown = false;
let crashNoticeShown = false;

/** Until the editor has a project: "Starting", or why starting failed with ways to go on. */
function BootScreen() {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(true);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  /** `open` first opens or creates a project, for when the last one is what failed. */
  const start = async (open?: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await open?.();
      await startEditor();
    } catch (e) {
      setError(plainError(errorText(e)));
      setBusy(false);
      api.listProjects().then(setProjects, () => setProjects([]));
    }
  };
  useEffect(() => void start(), []);

  if (error === null)
    return (
      <div className="flex h-full items-center justify-center text-[13px] text-muted" role="status">
        Starting Nuzky…
      </div>
    );
  return (
    <div className="flex h-full items-center justify-center p-6">
      <div className="flex w-[440px] flex-col gap-4">
        <div role="alert" className="flex flex-col gap-2">
          <h1 className="text-[17px] font-semibold text-fg">Nuzky could not start</h1>
          <p className="flex items-start gap-2 text-[13px] text-danger">
            <AlertCircle size={16} className="mt-px shrink-0" />
            {error}
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="primary" pill disabled={busy} onClick={() => void start()}>
            Try again
          </Button>
          <Button pill disabled={busy} onClick={() => void start(() => api.newProject(1080, 1920))}>
            New project
          </Button>
        </div>
        {projects.length > 0 && (
          <div className="flex flex-col gap-1">
            <h2 className="text-[12px] font-semibold text-muted">Open another project</h2>
            <div className="max-h-64 overflow-y-auto">
              {projects.map((p) => (
                <button
                  key={p.path}
                  type="button"
                  disabled={busy}
                  onClick={() => void start(() => api.openProject(p.path))}
                  className="flex w-full items-center justify-between rounded-md px-2 py-1.5 text-left hover:bg-white/[.08] disabled:cursor-not-allowed disabled:opacity-40"
                >
                  <span className="truncate text-[13px] text-fg">{p.name}</span>
                  <span className="tabular shrink-0 pl-2 text-[11px] text-muted">{formatDuration(p.durationUs)}</span>
                </button>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// The editor behind the home screen and the dialogs load once the home screen is on screen.
const Editor = lazy(() => import("./components/Editor"));
const Dialogs = lazy(() => import("./components/Dialogs"));

/** True from the frame after the first one drawn with a project. */
function useDrawn(ready: boolean) {
  const [drawn, setDrawn] = useState(false);
  useEffect(() => {
    if (!ready) return;
    const frame = requestAnimationFrame(() => setTimeout(() => setDrawn(true)));
    return () => cancelAnimationFrame(frame);
  }, [ready]);
  return drawn;
}

export default function App() {
  const snap = useEditor((s) => s.snap);
  const view = useEditor((s) => s.view);
  const projectName = useEditor((s) => s.snap?.project.name ?? null);
  useEffect(() => titleWindow(view === "editor" ? projectName : null), [view, projectName]);
  const setupOpen = useSetup((s) => s.open);
  const dock = useDockLayout();
  const win = useWindowSize();
  // A column docked at either edge narrows the editor; in the inspector's place the panel is the inspector's width.
  const column = dock.open && (dock.mode === "left" || dock.mode === "right") ? dock.width + 6 : 0;
  const panes = paneLayout(usePanes(), win.w - column, win.h);
  useShortcuts();
  useBackendEvents();
  useUiContext();
  useEffect(listenAgentEvents, []);
  useEffect(listenRemote, []);
  const drawn = useDrawn(!!snap);

  if (!snap) return <BootScreen />;

  const home = view === "home";
  return (
    <>
      {drawn && (
        <Suspense>
          <Editor panes={panes} dock={dock} />
        </Suspense>
      )}
      {home && (
        <div className="fixed inset-0 z-[60]" inert={snap.recovery || setupOpen}>
          <Home />
        </div>
      )}
      {drawn && (
        <Suspense>
          <Dialogs />
        </Suspense>
      )}
      {drawn && <Tour />}
      {/* On the home screen they sit at the grid's left edge, clear of the sidebar; in the editor, clear of a panel docked left. */}
      <Toasts bottom={home ? 18 : panes.timeline + 18} left={home ? 268 : dock.open && dock.mode === "left" ? dock.width + 24 : 18} />
      <DisabledHint />
      <Tooltip />
      {snap.recovery && <RecoveryDialog key={snap.sessionEpoch} />}
    </>
  );
}

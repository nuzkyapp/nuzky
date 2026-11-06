import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { errorText, plainError } from "./api";
import { agentName, choose, chooseAgent, loadAgents, loadChats, newChat, openChat, send, sendBlocked, steer, steerBlocked, stop, useAgent, type AgentId, type ChatItem } from "./agent";
import { planReady, useChatGpt } from "./chatgpt";
import { useDock } from "./dock";
import { blocked, newProject, openProject, refreshLibrary, useLibrary, whenLabel } from "./library";
import { EFFORTS, chooseEffort, chooseModel, effortLabel, ownDefault, resolved, useModels } from "./models";
import { FORMATS } from "./presets";
import {
  AI_EDITING,
  MAIN_TRACK,
  canSplitClip,
  currentEpoch,
  deleteSelection,
  detachAudio,
  detachBlocker,
  displayTracks,
  duplicateSelection,
  findClip,
  isCaptionTrack,
  mainClips,
  projectDuration,
  splitAtPlayhead,
  useEditor,
} from "./store";
import { importPaths } from "../components/panel/assets";
import { exportVideo, loadOptions, outputSize } from "../components/ExportDialog";

/**
 * Share: a phone or another computer on the same network works on the open project through a web page that
 * Nuzky serves (src-tauri/src/remote.rs). The page asks; the editor here does each thing as if it was clicked here.
 */
interface RemoteState {
  /** The page's address with its secret and the same as a QR code, while sharing is on. */
  sharing: { url: string; qr: string } | null;
  /** The devices that have the page open. */
  pages: string[];
  dialog: boolean;
  error: string | null;
  /** Files from the page waiting for their turn on the timeline. */
  adding: string[];
  /** The export the page started, and what it makes. */
  exportJob: { id: string; width: number; height: number; durationUs: number } | null;
}

export const useRemote = create<RemoteState>(() => ({ sharing: null, pages: [], dialog: false, error: null, adding: [], exportJob: null }));

type Action =
  | { action: "add"; path: string }
  | { action: "send"; text: string }
  | { action: "choose"; id: string; label: string }
  | { action: "seek"; us: number }
  | { action: "stop" }
  | { action: "export" }
  | { action: "play" }
  | { action: "undo" }
  | { action: "redo" }
  | { action: "newChat" }
  | { action: "chats" }
  | { action: "split"; clipId: string }
  | { action: "duplicate"; clipId: string }
  | { action: "detachAudio"; clipId: string }
  | { action: "delete"; clipId: string }
  | { action: "agent"; agent: string }
  | { action: "model"; slug: string | null }
  | { action: "effort"; effort: string }
  | { action: "openChat"; id: string }
  | { action: "newProject"; format: string }
  | { action: "openProject"; id: string };

export async function openSharing() {
  useRemote.setState({ dialog: true, error: null });
  if (useRemote.getState().sharing) return;
  // The page lists the projects and the agents, which the editor otherwise loads only for the home screen and the AI panel.
  void refreshLibrary();
  if (!useAgent.getState().agents) void loadAgents();
  try {
    useRemote.setState({ sharing: await invoke<{ url: string; qr: string }>("remote_start") });
    publish();
  } catch (e) {
    useRemote.setState({ error: plainError(errorText(e)) });
  }
}

export async function stopSharing() {
  await invoke("remote_stop");
  useRemote.setState({ sharing: null, pages: [], dialog: false });
}

const fileName = (path: string) => path.split(/[\\/]/).pop() || path;

/** One file at a time, in the order they came, each after the last clip once the AI is not editing. */
let adding = Promise.resolve();
function add(path: string) {
  const name = fileName(path);
  useRemote.setState((s) => ({ adding: [...s.adding, name] }));
  adding = adding
    .then(async () => {
      await new Promise<void>((resolve) => {
        if (!useEditor.getState().aiRun) return resolve();
        const off = useEditor.subscribe((s) => {
          if (s.aiRun) return;
          off();
          resolve();
        });
      });
      const project = useEditor.getState().snap?.project;
      const end = project ? Math.max(0, ...mainClips(project).map((c) => c.startUs + c.durationUs)) : 0;
      await importPaths([path], { trackId: MAIN_TRACK, startUs: end, replaceId: null });
    })
    .finally(() => useRemote.setState((s) => ({ adding: s.adding.filter((_, i) => i !== s.adding.indexOf(name)) })));
}

function exportBlocked(): string | null {
  const { snap, aiRun, exportJobId, jobs } = useEditor.getState();
  if (!snap || projectDuration(snap.project) === 0) return "Add a video first";
  if (aiRun) return "Export when the AI is done";
  if (exportJobId && jobs[exportJobId]?.status === "running") return "Exporting";
  return null;
}

/** Exports with the settings the Export dialog last used, into a new file in the Videos folder. */
async function exportForPage() {
  const snap = useEditor.getState().snap;
  if (!snap || exportBlocked()) return;
  try {
    const path = await invoke<string>("remote_export_path", { name: snap.project.name });
    const options = loadOptions(snap.project.canvas, null);
    if (!(await exportVideo(path, options, currentEpoch(), false, false))) return;
    const id = useEditor.getState().exportJobId;
    const { w, h } = outputSize(snap.project.canvas, options.resolution);
    if (id) useRemote.setState({ exportJob: { id, width: w, height: h, durationUs: projectDuration(snap.project) } });
  } catch (e) {
    useEditor.getState().toast({ kind: "error", text: plainError(errorText(e)) });
  }
}

/** The page names projects by these, so it never learns where they are on this computer. */
const projectIds = new Map<string, string>();
const projectId = (path: string) => {
  if (!projectIds.has(path)) projectIds.set(path, `p${projectIds.size + 1}`);
  return projectIds.get(path)!;
};

/** Why the page cannot edit now; the same lock the editor's controls show. */
const editBlocked = () => (useEditor.getState().aiRun ? AI_EDITING : null);

/** A clip action from the page, on the clip it names, as if it was selected and clicked here. */
function onClip(clipId: string, run: () => unknown) {
  const snap = useEditor.getState().snap;
  if (!snap || editBlocked() || !findClip(snap.project, clipId)) return;
  useEditor.getState().select([clipId]);
  void run();
}

function act(a: Action) {
  if (a.action === "add") return add(a.path);
  if (a.action === "seek") return useEditor.getState().seek(a.us);
  if (a.action === "stop") return void stop();
  if (a.action === "export") return void exportForPage();
  if (a.action === "play") return useEditor.getState().togglePlay();
  if (a.action === "undo") return void useEditor.getState().undo();
  if (a.action === "redo") return void useEditor.getState().redo();
  if (a.action === "split") {
    // The playhead may have left the clip since the page drew it; the editor would then split the main track instead.
    const { snap, timeUs } = useEditor.getState();
    const found = snap && findClip(snap.project, a.clipId);
    if (found && canSplitClip(found.clip, Math.round(timeUs), snap.project.canvas.fps)) onClip(a.clipId, splitAtPlayhead);
    return;
  }
  if (a.action === "duplicate") return onClip(a.clipId, duplicateSelection);
  if (a.action === "detachAudio") {
    const { snap } = useEditor.getState();
    const found = snap && findClip(snap.project, a.clipId);
    if (found && !detachBlocker(snap.project, found.clip)) onClip(a.clipId, () => detachAudio([a.clipId]));
    return;
  }
  if (a.action === "delete") return onClip(a.clipId, deleteSelection);
  if (a.action === "agent") return pageAgents().some((g) => g.id === a.agent && g.ready) && chooseAgent(a.agent as AgentId);
  if (a.action === "model") {
    const agent = useAgent.getState().agent;
    if (a.slug === null ? ownDefault(agent) : useModels.getState().lists[agent]?.some((m) => m.slug === a.slug)) chooseModel(agent, a.slug);
    return;
  }
  if (a.action === "effort") {
    const agent = useAgent.getState().agent;
    if (resolved(agent).efforts.includes(a.effort)) chooseEffort(agent, a.effort);
    return;
  }
  if (a.action === "newChat") return newChat();
  if (a.action === "chats") return void loadChats();
  if (a.action === "openChat") return useAgent.getState().chats?.some((c) => c.id === a.id) && void openChat(a.id);
  if (a.action === "newProject") {
    const format = FORMATS.find((f) => f.label === a.format);
    // While the AI edits, the editor would ask here what to do with its changes; the page cannot answer that.
    if (format && !useEditor.getState().aiRun) newProject(format.width, format.height);
    return;
  }
  if (a.action === "openProject") {
    const p = useLibrary.getState().projects?.find((p) => projectId(p.path) === a.id);
    if (p && !blocked(p) && !useEditor.getState().aiRun) openProject(p);
    return;
  }
  if (a.action === "choose") {
    if (sendBlocked()) return;
    useAgent.setState({ dropSelection: true });
    return choose(a.id, a.label);
  }
  // The panel opens, so whoever sits here sees what the page asked. The page selects nothing, so the clip selected
  // here, often the one just added, is not what its message is about; the playhead is, as the page moves it.
  useDock.setState({ open: true });
  const working = useAgent.getState().status === "working";
  if (working ? steerBlocked() : sendBlocked(false)) return;
  useAgent.setState({ dropSelection: true });
  void (working ? steer(a.text) : send(a.text, "project", []));
}

/** The conversation as the page shows it: words, steps, choices and what each run changed. */
function pageItems(items: ChatItem[], past: number) {
  return items.slice(-60).flatMap((it, i): unknown[] => {
    const old = i + Math.max(0, items.length - 60) < past;
    switch (it.kind) {
      case "user":
        return [{ kind: "user", text: it.text }];
      case "agent":
        return it.text.trim() ? [{ kind: "agent", text: it.text }] : [];
      case "steps":
        return [{ kind: "steps", steps: it.steps.map((s) => ({ title: s.title, status: s.status })) }];
      case "options":
        return [{ kind: "options", id: it.id, question: it.question, options: it.options.map((o) => o.label), chosen: it.chosen, open: !old && !it.chosen }];
      case "run":
        return [{ kind: "run", label: it.label, changes: (it.changes ?? []).map((c) => c.text), stopped: it.stopped, undone: !!it.undone }];
      case "stopped":
        return [{ kind: "note", text: "Stopped" }];
      case "error":
        return [{ kind: "error", text: plainError(it.message) }];
    }
  });
}

/** The AI panel's agents as its menu lists them, with what it says beside each. */
function pageAgents() {
  const { agents, gemini } = useAgent.getState();
  const chatgpt = useChatGpt.getState().status;
  return [
    ...(agents ?? []).map((a) => ({ id: a.id as AgentId, name: a.name, ready: !!a.path, note: a.path ? (a.version ?? "Installed") : "Not installed" })),
    { id: "chatgpt" as const, name: "ChatGPT", ready: planReady(chatgpt), note: planReady(chatgpt) ? "Signed in" : chatgpt?.state === "signedIn" ? "Plan not allowed" : "Not signed in" },
    {
      id: "antigravity" as const,
      name: "Gemini",
      ready: !!gemini?.installed && gemini.connection.state === "connected",
      note: !gemini?.installed ? "Not installed" : gemini.connection.state === "connected" ? "Connected" : "Not connected",
    },
  ];
}

/** Changes when an agent puts another file under the same asset, so the page never shows a kept picture of the old one. */
function fileVersion(asset: { path: string; durationUs: number; width: number; height: number }) {
  let h = 2166136261;
  for (const ch of `${asset.path}|${asset.durationUs}|${asset.width}|${asset.height}`) h = Math.imul(h ^ ch.charCodeAt(0), 16777619);
  return (h >>> 0).toString(36);
}

function pageTracks() {
  const { snap, timeUs } = useEditor.getState();
  const project = snap?.project;
  if (!project) return [];
  const locked = editBlocked();
  return displayTracks(project).map((t) => ({
    id: t.id,
    kind: t.kind,
    captions: isCaptionTrack(t),
    main: t.id === MAIN_TRACK,
    clips: t.clips.map((c) => {
      const media = c.content.type === "media" ? project.assets.find((a) => a.id === (c.content as { assetId: string }).assetId) : undefined;
      // Sound detached from a video plays the video's file on an audio track: it has no picture there.
      const asset = t.kind === "audio" ? undefined : media;
      const why: Record<string, string> = {};
      if (locked) for (const k of ["split", "duplicate", "detachAudio", "delete"]) why[k] = locked;
      else {
        if (!canSplitClip(c, timeUs, project.canvas.fps)) why.split = "Move the playhead into this clip to split it";
        const detach = detachBlocker(project, c);
        if (detach) why.detachAudio = detach;
      }
      return {
        id: c.id,
        kind: c.content.type === "text" ? "text" : t.kind === "audio" ? "audio" : (media?.kind ?? "video"),
        assetId: asset && asset.kind !== "audio" ? asset.id : null,
        thumb: asset && asset.kind !== "audio" ? `/thumb/${encodeURIComponent(asset.id)}?v=${fileVersion(asset)}` : null,
        label: c.content.type === "text" ? c.content.text : (media?.name ?? ""),
        startUs: c.startUs,
        durationUs: c.durationUs,
        can: Object.fromEntries(["split", "duplicate", "detachAudio", "delete"].map((k) => [k, !why[k]])),
        why,
      };
    }),
  }));
}

function pageModels() {
  const { agent } = useAgent.getState();
  const models = useModels.getState();
  const { model, effort, efforts } = resolved(agent, models);
  return {
    models: (models.lists[agent] ?? []).map((m) => ({ slug: m.slug, name: m.displayName })),
    model: model?.slug ?? null,
    ownDefault: ownDefault(agent) ? (agent === "claude" ? "Claude Code setting" : "OpenCode's last model") : null,
    modelsLoading: !models.lists[agent] && !models.errors[agent],
    efforts: efforts.map((id) => ({ id, short: EFFORTS[id]?.short ?? effortLabel(id), hint: EFFORTS[id]?.hint ?? "" })),
    effort,
  };
}

function pageProjects() {
  const open = useEditor.getState().snap?.path;
  return (useLibrary.getState().projects ?? []).map((p) => {
    const state = p.path === open ? "open" : p.state === "busy" ? "busy" : p.state === "broken" ? "broken" : "ok";
    const when = state === "open" ? "Open now" : state === "busy" ? "In use elsewhere" : state === "broken" ? "Damaged file" : whenLabel(p.modifiedMs);
    return { id: projectId(p.path), name: p.name, durationUs: p.durationUs, when, state };
  });
}

function pageState() {
  const { snap, timeUs, aiRun, jobs, playing } = useEditor.getState();
  const agent = useAgent.getState();
  const { adding, exportJob } = useRemote.getState();
  const project = snap?.project;
  const job = exportJob ? jobs[exportJob.id] : null;
  return {
    project: project?.name ?? "",
    canvas: project ? { width: project.canvas.width, height: project.canvas.height } : null,
    durationUs: project ? projectDuration(project) : 0,
    timeUs,
    clips: project
      ? mainClips(project).map((c) => {
          const asset = c.content.type === "media" ? project.assets.find((a) => a.id === (c.content as { assetId: string }).assetId) : undefined;
          return { id: c.id, assetId: asset?.id ?? null, kind: asset?.kind ?? "video", startUs: c.startUs, durationUs: c.durationUs };
        })
      : [],
    adding,
    agent: agentName(agent.agent),
    working: agent.status !== "idle",
    stopping: agent.status === "stopping",
    sendBlocked: agent.status === "working" ? steerBlocked() : sendBlocked(false),
    lock: aiRun,
    items: pageItems(agent.items, agent.pastCount ?? 0),
    exportBlocked: exportBlocked(),
    exportId: exportJob?.id ?? null,
    exporting: job?.status === "running" ? job.progress : null,
    exportError: job?.status === "failed" ? plainError(job.message ?? "The export failed") : null,
    exported: job?.status === "done" && exportJob ? { width: exportJob.width, height: exportJob.height, durationUs: exportJob.durationUs } : null,
    playing,
    canUndo: !!snap?.canUndo,
    canRedo: !!snap?.canRedo,
    editBlocked: editBlocked(),
    tracks: pageTracks(),
    agents: pageAgents(),
    agentId: agent.agent,
    ...pageModels(),
    chats: agent.chats?.map((c) => ({ id: c.id, title: c.title, agent: agentName(c.agent), when: whenLabel(c.updatedMs) })) ?? null,
    chatId: agent.chat,
    projects: pageProjects(),
  };
}

/** At most every 250 ms, ending on the latest state; nothing while sharing is off. */
let timer = 0;
let lastSent = 0;
function publish() {
  if (timer || !useRemote.getState().sharing) return;
  timer = window.setTimeout(
    () => {
      timer = 0;
      lastSent = Date.now();
      if (!useRemote.getState().sharing) return;
      void invoke("remote_publish", { state: JSON.stringify(pageState()) }).catch(() => {});
    },
    Math.max(0, 250 - (Date.now() - lastSent)),
  );
}

export function listenRemote() {
  const offs = [
    listen<Action>("remote-action", (e) => act(e.payload)),
    listen<string[]>("remote-pages", (e) => useRemote.setState({ pages: e.payload })),
    listen("remote-stopped", () => {
      useRemote.setState({ sharing: null, pages: [], dialog: false });
      useEditor.getState().toast({ kind: "info", text: "Sharing turned off after 30 minutes with no one connected" });
    }),
  ];
  // sendBlocked() also reads the ChatGPT sign-in and its models.
  const unsubscribe = [useEditor, useAgent, useRemote, useChatGpt, useModels, useLibrary].map((store) => store.subscribe(publish));
  return () => {
    offs.forEach((p) => p.then((off) => off()));
    unsubscribe.forEach((off) => off());
  };
}

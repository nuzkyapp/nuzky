import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { errorText, plainError } from "./api";
import { agentName, choose, send, sendBlocked, steer, steerBlocked, stop, useAgent, type ChatItem } from "./agent";
import { useChatGpt } from "./chatgpt";
import { useDock } from "./dock";
import { useModels } from "./models";
import { MAIN_TRACK, currentEpoch, mainClips, projectDuration, useEditor } from "./store";
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
  | { action: "export" };

export async function openSharing() {
  useRemote.setState({ dialog: true, error: null });
  if (useRemote.getState().sharing) return;
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

function act(a: Action) {
  if (a.action === "add") return add(a.path);
  if (a.action === "seek") return useEditor.getState().seek(a.us);
  if (a.action === "stop") return void stop();
  if (a.action === "export") return void exportForPage();
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

function pageState() {
  const { snap, timeUs, aiRun, jobs } = useEditor.getState();
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
  const unsubscribe = [useEditor, useAgent, useRemote, useChatGpt, useModels].map((store) => store.subscribe(publish));
  return () => {
    offs.forEach((p) => p.then((off) => off()));
    unsubscribe.forEach((off) => off());
  };
}

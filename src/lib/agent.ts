import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { errorText, plainError } from "./api";
import { check as checkChatGpt, planReady, useChatGpt } from "./chatgpt";
import { forgetChatGptModels, loadModels as loadAgentModels, resolved, useModels } from "./models";
import { downloadModels, loadModels, useCover } from "./cover";
import { useDock } from "./dock";
import { formatTime } from "./time";
import { useEditor } from "./store";
import type { Snapshot } from "./types";

/** An agent the panel can run: the user's own installed Claude Code or Codex, or ChatGPT on the user's plan, which runs in Nuzky. */
export type AgentId = CliAgentId | "chatgpt";
export type CliAgentId = "claude" | "codex";
export interface AgentInfo {
  id: CliAgentId;
  name: string;
  /** Where the executable was found, or null when it is not installed. */
  path: string | null;
  version: string | null;
}

/** What went wrong with a turn, so the panel can say what to do next. */
/** `SESSION_GONE`: Claude Code or Codex no longer has the conversation a reopened chat would continue. */
export type AgentErrorCode = "NOT_INSTALLED" | "NOT_SIGNED_IN" | "USAGE_LIMIT" | "PLAN_UNAVAILABLE" | "AGENT_FAILED" | "SESSION_GONE";

/** Events from the backend for one chat, in order. */
export type AgentEvent =
  | { chat: string; type: "text"; text: string }
  | { chat: string; type: "tool"; id: string; name: string; status: "running" | "done" | "failed"; input?: unknown }
  | { chat: string; type: "done"; stopped: boolean }
  | { chat: string; type: "error"; code: AgentErrorCode; message: string };

/** One line of what a run changed, from comparing the project before and after it. */
export interface RunChange {
  text: string;
  /** Clips the line is about, to select them. */
  clipIds: string[];
  /** Where on the timeline to look. */
  atUs: number | null;
}

/** What a run that ended changed: sent just before it ends, and again with what the cut removed once counted. */
interface RunSummary {
  runId: string;
  changes: RunChange[];
  /** It cut something out of the video, so a recap can show it. */
  cut: boolean;
}

export interface Step {
  id: string;
  title: string;
  status: "running" | "done" | "failed" | "stopped" | "interrupted";
  detail: string | null;
}

/** A job the agent started that runs on after its turn, as the backend reports it. */
export interface AgentJob {
  job_id: string;
  kind: string;
  status: "running" | "done" | "failed" | "cancelled";
  progress: number | null;
  cancel_requested: boolean;
  error: string | null;
}

export interface Choice {
  label: string;
  detail: string | null;
}

/** A picture or video the user attached to a message; the panel's own copy when it was pasted. */
export interface Attachment {
  path: string;
  kind: "image" | "video";
  name: string;
  /** A small PNG data URL. */
  preview: string | null;
}

export type ChatItem =
  /** `at`: when it was sent, in ms. */
  | { kind: "user"; text: string; context: string | null; at?: number; attachments?: Attachment[] }
  /** `ended`: a tool call the panel does not show came after it, so the agent's next words are a new message. */
  | { kind: "agent"; text: string; ended?: boolean }
  | { kind: "steps"; steps: Step[] }
  | { kind: "options"; id: string; question: string | null; options: Choice[]; chosen: string | null }
  | {
      kind: "run";
      label: string;
      snap: Snapshot | null;
      stopped: boolean;
      changes: RunChange[] | null;
      runId: string | null;
      /** It cut something a recap can show. */
      cut: boolean;
      undone?: boolean;
    }
  | { kind: "stopped" }
  /** `retry`: the message that failed after the agent had begun answering, for Try again, with what was attached to it. */
  | { kind: "error"; code: AgentErrorCode; message: string; retry?: string; retryAttachments?: Attachment[] };

/** A chat this project keeps, as the list of past chats shows it. */
export interface ChatSummary {
  id: string;
  agent: AgentId;
  title: string;
  updatedMs: number;
}

export interface PromptContext {
  /** Selected clips at send time, or null when the user removed the chip or nothing is selected. */
  selection: string[] | null;
  playheadUs: number | null;
  frame: boolean;
  /** Sent from the Your style page: about the creator's style, not the open project. */
  style: boolean;
  /** Make a thumbnail: the agent gets the MCP prompt `thumbnail`, with the text as the user's wishes. */
  thumbnail: boolean;
  attachments: Attachment[];
}

/** What the panel is about: the open project, or the creator's style on the Your style page. */
export type AiAbout = "project" | "style";

interface AgentState {
  agents: AgentInfo[] | null;
  agentsError: string | null;
  agent: AgentId;
  chat: string | null;
  /** How many items came from a reopened chat: their runs can no longer be undone and their choices no longer answered. */
  pastCount: number | null;
  /** The chats this project keeps, newest first; null until loaded. */
  chats: ChatSummary[] | null;
  /** The message being answered, so Stop reaches it even while it is starting. */
  turn: string | null;
  items: ChatItem[];
  /** What the agent left running when its turn ended, such as an export it asked about; Stop stops it. One that failed meanwhile stays to say so. */
  jobs: AgentJob[];
  status: "idle" | "working" | "stopping";
  /** When the current answer started, in ms; it survives moving the panel. */
  startedAt: number | null;
  draft: string;
  /** Attached to the draft. */
  attachments: Attachment[];
  /** Attachments still being read, which the message waits for. */
  attaching: number;
  /** What the last message was about, so a choice or Try again stays on it. */
  about: AiAbout;
  /** Context chips the user removed from the next message. */
  dropSelection: boolean;
  dropPlayhead: boolean;
  frame: boolean;
  /** Make a thumbnail is in the field: the message asks for a cover and a YouTube thumbnail, the text is an optional note. */
  thumbnail: boolean;
  /** Sent while the agent worked, but it could not take it in that turn: it goes as the next message once the turn ends. `stopped`: Codex stopped for it. */
  next: { text: string; context: PromptContext; stopped: boolean } | null;
}

const AGENT_KEY = "nuzky.aiAgent";

const AGENTS: AgentId[] = ["claude", "codex", "chatgpt"];
const usable = (a: AgentInfo) => !!a.path;

export function agentName(id: AgentId, agents = useAgent.getState().agents): string {
  return id === "chatgpt" ? "ChatGPT" : (agents?.find((a) => a.id === id)?.name ?? (id === "codex" ? "Codex" : "Claude Code"));
}

/** Whether any agent can answer: an installed one, or ChatGPT signed in with its plan allowed. */
export const anyAgent = () => !!useAgent.getState().agents?.some(usable) || planReady(useChatGpt.getState().status);

export const useAgent = create<AgentState>(() => ({
  agents: null,
  agentsError: null,
  agent: AGENTS.includes(localStorage.getItem(AGENT_KEY) as AgentId) ? (localStorage.getItem(AGENT_KEY) as AgentId) : "claude",
  chat: null,
  pastCount: null,
  chats: null,
  turn: null,
  items: [],
  jobs: [],
  status: "idle",
  startedAt: null,
  about: "project",
  draft: "",
  attachments: [],
  attaching: 0,
  dropSelection: false,
  dropPlayhead: false,
  frame: false,
  thumbnail: false,
  next: null,
}));

const agentApi = {
  list: () => invoke<AgentInfo[]>("agent_list"),
  send: (agent: AgentId, chat: string, turn: string, text: string, context: PromptContext, model: string | null, effort: string | null) =>
    invoke<void>("agent_send", { agent, chat, turn, text, context, model, effort }),
  stop: (turn: string) => invoke<void>("agent_stop", { turn }),
  /** Whether the working agent took the message in this turn, it goes next, or Codex stopped for it to go next. */
  steer: (turn: string, text: string, context: PromptContext) => invoke<"taken" | "next" | "stopped">("agent_steer", { turn, text, context }),
  jobs: () => invoke<AgentJob[]>("agent_jobs"),
  chats: () => invoke<ChatSummary[]>("agent_chats"),
  open: (chat: string) => invoke<{ id: string; agent: AgentId; items: ChatItem[] }>("agent_chat_open", { chat }),
  save: (chat: string, title: string, items: ChatItem[]) => invoke<void>("agent_chat_save", { chat, title, items }),
};

/** Tool names as the agent reports them: `mcp__nuzky__get_state` (Claude) or `nuzky.get_state` (Codex). */
const toolName = (raw: string) => raw.replace(/^mcp__nuzky__/, "").replace(/^nuzky[./]/, "");

/** Bookkeeping calls the user does not need to see as steps. */
const HIDDEN = new Set(["begin_run", "end_run", "job"]);
/** The tool an agent offers choices with; the panel shows them as buttons, not as a step. */
const OFFER = "suggest_options";

const TITLES: Record<string, string> = {
  get_state: "Read the project",
  get_transcript: "Read the transcript",
  transcribe: "Transcribing",
  apply_edits: "Editing",
  edit_transcript: "Cutting by the transcript",
  correct_words: "Correcting words",
  apply_zooms: "Adding zooms",
  apply_motion: "Adding motion",
  build_captions: "Adding captions",
  inspect_frames: "Checking frames",
  activity: "Scanning picture and sound",
  import_media: "Importing media",
  add_sticker: "Adding a sticker",
  export_video: "Exporting",
  undo_run: "Undoing its changes",
};

const ANALYSES: Record<string, string> = {
  silences: "Finding pauses",
  loudness: "Measuring loudness",
  scenes: "Finding scene changes",
  fillers: "Finding filler words",
  retakes: "Finding retakes",
  emphasis: "Finding sentences to zoom",
};

const JOBS: Record<string, string> = {
  transcription: "Transcribing",
  export: "Exporting",
  activity: "Scanning picture and sound",
  analysis: "Analysing",
  reframe: "Following the face",
  matte: "Finding the person",
  segment: "Finding the person",
};

/** What a job the agent left running is doing, with its percentage once known, or why it failed. */
export function jobText(job: AgentJob): string {
  const title = JOBS[job.kind] ?? "Working";
  if (job.status === "failed") return `${title} failed. ${plainError(job.error ?? "")}`;
  if (job.cancel_requested) return "Stopping…";
  return job.progress === null ? `${title}…` : `${title} ${Math.round(job.progress * 100)}%`;
}

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === "object" ? (v as Record<string, unknown>) : {});

/** A short step title for a tool call, from what it was asked; tools from elsewhere keep their own name. */
function stepTitle(raw: string, input?: unknown): string {
  const name = toolName(raw);
  if (name === "analyze") return ANALYSES[String(obj(input).kind)] ?? "Analysing the sound";
  return TITLES[name] ?? name;
}

/** The detail worth showing next to a step: which frames, which file. */
function stepDetail(raw: string, input?: unknown): string | null {
  const args = obj(input);
  const name = toolName(raw);
  if (name === "inspect_frames" && Array.isArray(args.times_us)) return args.times_us.slice(0, 4).map((t) => formatTime(Number(t), false)).join(" · ") + (args.times_us.length > 4 ? " …" : "");
  if (name === "export_video" && typeof args.path === "string") return args.path.split(/[\\/]/).pop() ?? null;
  if (name === "apply_edits" && Array.isArray(args.edits)) return args.edits.length === 1 ? "1 change" : `${args.edits.length} changes`;
  if (name === "import_media" && Array.isArray(args.paths)) return args.paths.length === 1 ? String(args.paths[0]).split(/[\\/]/).pop()! : `${args.paths.length} files`;
  return null;
}

function offered(input: unknown): Omit<Extract<ChatItem, { kind: "options" }>, "id" | "kind" | "chosen"> | null {
  const args = obj(input);
  if (!Array.isArray(args.options)) return null;
  const options = args.options
    .map((o) => (typeof o === "string" ? { label: o, detail: null } : { label: String(obj(o).label ?? ""), detail: typeof obj(o).detail === "string" ? (obj(o).detail as string) : null }))
    .filter((o) => o.label.trim());
  return options.length ? { question: typeof args.question === "string" ? args.question : null, options } : null;
}

export async function loadAgents() {
  try {
    void checkChatGpt(false);
    const agents = await agentApi.list();
    const s = useAgent.getState();
    // Keep the chosen agent when it is installed or is ChatGPT; otherwise pick one that is installed, or else ChatGPT.
    const chosen = s.agent === "chatgpt" ? s.agent : (agents.find((a) => a.id === s.agent && usable(a)) ?? agents.find(usable))?.id;
    useAgent.setState({ agents, agentsError: null, agent: chosen ?? "chatgpt" });
  } catch (e) {
    useAgent.setState({ agentsError: plainError(errorText(e)) });
  }
}

export function chooseAgent(agent: AgentId) {
  const s = useAgent.getState();
  if (s.agent === agent || s.status !== "idle") return;
  localStorage.setItem(AGENT_KEY, agent);
  // Another agent does not know this conversation, so it starts a new one; the draft stays.
  useAgent.setState({ agent, ...endChat() });
}

export function newChat() {
  if (useAgent.getState().status !== "idle") return;
  useAgent.setState(endChat());
}

/** A new conversation stops what the old one left running. */
function endChat() {
  const { turn } = useAgent.getState();
  if (turn) void agentApi.stop(turn).catch((e) => useEditor.getState().toast({ kind: "error", text: plainError(errorText(e)) }));
  return { chat: null, pastCount: null, items: [], jobs: [] };
}

/** Lists the chats the open project keeps. */
export async function loadChats() {
  const epoch = useEditor.getState().snap?.sessionEpoch;
  try {
    const chats = await agentApi.chats();
    // Another project may have opened meanwhile.
    if (useEditor.getState().snap?.sessionEpoch === epoch) useAgent.setState({ chats });
  } catch (e) {
    useAgent.setState({ chats: [] });
    useEditor.getState().toast({ kind: "error", text: `Your past chats didn't load. ${plainError(errorText(e))}` });
  }
}

/** What was typed in each chat of this session, so a draft stays with its own chat. */
const drafts = new Map<string | null, { draft: string; attachments: Attachment[] }>();

/** Opens a chat the project keeps, to read and continue it; it stops what the current one left running. */
export async function openChat(id: string) {
  const s = useAgent.getState();
  if (s.status !== "idle" || s.chat === id) return;
  const epoch = useEditor.getState().snap?.sessionEpoch;
  try {
    const opened = await agentApi.open(id);
    if (useAgent.getState().status !== "idle" || useEditor.getState().snap?.sessionEpoch !== epoch) return;
    // What was still running when Nuzky closed did not finish.
    const items = (opened.items ?? []).map((it): ChatItem => (it.kind === "steps" ? { ...it, steps: it.steps.map((x) => (x.status === "running" ? { ...x, status: "interrupted" } : x)) } : it));
    const { chat, draft, attachments } = useAgent.getState();
    drafts.set(chat, { draft, attachments });
    useAgent.setState({ ...endChat(), agent: opened.agent, chat: opened.id, items, pastCount: items.length, draft: "", attachments: [], ...drafts.get(opened.id) });
    // Saved again as the chat last used, so the project reopens it next time.
    void saveChat(opened.id, items);
  } catch (e) {
    useEditor.getState().toast({ kind: "error", text: `That chat didn't open. ${plainError(errorText(e))}` });
    void loadChats();
  }
}

/** Keeps the chat with its project: after each turn, and when something in it changes between turns. */
async function saveChat(chat: string, items: ChatItem[]) {
  // A run's snapshot is the whole project, and only undoes it while this session lasts. A problem is about that moment:
  // reopened, its Try again or "Your message is still here" would no longer be true. What was said on the Your style
  // page, which shares the conversation, is about every project.
  let style = false;
  const kept = items
    .filter((it) => {
      if (it.kind === "user") style = it.context === "Your style";
      return !style && it.kind !== "error";
    })
    .map((it) => (it.kind === "run" ? { ...it, snap: null } : it));
  const first = kept.find((i) => i.kind === "user");
  if (first?.kind !== "user") return;
  const title = (first.text || (first.attachments ?? []).map((a) => a.name).join(", ")).replace(/\s+/g, " ").trim().slice(0, 120);
  try {
    await agentApi.save(chat, title, kept);
  } catch (e) {
    useEditor.getState().toast({ kind: "error", text: `This chat wasn't saved. ${plainError(errorText(e))}` });
    return;
  }
  void loadChats();
}

let watching = false;
/** Shows what the agent left running when its turn ended, until it ends or another turn starts. */
async function watchJobs() {
  if (watching) return;
  watching = true;
  try {
    while (useAgent.getState().status === "idle") {
      const chat = useAgent.getState().chat;
      const all = await agentApi.jobs();
      const s = useAgent.getState();
      if (s.status !== "idle" || s.chat !== chat) break;
      // One the panel showed that failed says so until the next message.
      const shown = new Set(s.jobs.map((j) => j.job_id));
      const jobs = all.filter((j) => j.status === "running" || (j.status === "failed" && shown.has(j.job_id)));
      useAgent.setState({ jobs });
      if (!jobs.some((j) => j.status === "running")) break;
      await new Promise((resolve) => window.setTimeout(resolve, 500));
    }
  } finally {
    watching = false;
  }
}

/** What the next message would carry, as the chips show it. */
function promptContext(about: AiAbout, attachments: Attachment[]): PromptContext {
  if (about === "style") return { selection: null, playheadUs: null, frame: false, style: true, thumbnail: false, attachments };
  const { selection, timeUs } = useEditor.getState();
  const { dropSelection, dropPlayhead, frame, thumbnail } = useAgent.getState();
  return { selection: dropSelection || selection.length === 0 ? null : [...selection], playheadUs: dropPlayhead ? null : Math.round(timeUs), frame, style: false, thumbnail, attachments };
}

/** Attaches pictures and videos to the draft: files the user chose, or what they pasted. */
export async function attach(files: (string | File)[]) {
  const chat = useAgent.getState().chat;
  useAgent.setState((s) => ({ attaching: s.attaching + files.length }));
  for (const file of files) {
    try {
      const attached =
        typeof file === "string"
          ? await invoke<Attachment>("agent_attach_file", { path: file })
          : await invoke<Attachment>("agent_attach_pasted", new Uint8Array(await file.arrayBuffer()), { headers: { "x-name": encodeURIComponent(file.name) } });
      const add = (list: Attachment[]) => (list.some((a) => a.path === attached.path) ? list : [...list, attached]);
      // Another chat opened meanwhile: it goes with the draft of the chat it was added in.
      if (useAgent.getState().chat === chat) useAgent.setState((s) => ({ attachments: add(s.attachments) }));
      else drafts.set(chat, { draft: drafts.get(chat)?.draft ?? "", attachments: add(drafts.get(chat)?.attachments ?? []) });
    } catch (e) {
      useEditor.getState().toast({ kind: "error", text: plainError(errorText(e)) });
    } finally {
      useAgent.setState((s) => ({ attaching: s.attaching - 1 }));
    }
  }
}

export function detach(path: string) {
  useAgent.setState((s) => ({ attachments: s.attachments.filter((a) => a.path !== path) }));
}

function describeContext(c: PromptContext): string | null {
  if (c.style) return "Your style";
  const parts: string[] = c.thumbnail ? ["Make a thumbnail"] : [];
  if (c.selection) parts.push(c.selection.length === 1 ? "1 clip" : `${c.selection.length} clips`);
  if (c.playheadUs !== null) parts.push(`${c.frame ? "Frame" : "Playhead"} ${formatTime(c.playheadUs, false)}`);
  return parts.length ? parts.join(" · ") : null;
}

/** Why a message cannot be sent right now, or null; `thumbnail`: whether it asks for one. */
export function sendBlocked(thumbnail = useAgent.getState().thumbnail): string | null {
  const s = useAgent.getState();
  if (s.agent === "chatgpt") {
    const { status } = useChatGpt.getState();
    const error = useModels.getState().errors.chatgpt;
    if (status?.state !== "signedIn") return "Sign in to ChatGPT first";
    if (!status.plan) return "Allow Nuzky to use your ChatGPT plan first";
    if (!resolved("chatgpt").model) return error ? `Your ChatGPT models didn't load. ${error}` : "Loading your ChatGPT models…";
  } else if (s.agents && !s.agents.find((a) => a.id === s.agent)?.path) return `${agentName(s.agent)} is not installed`;
  if (s.status !== "idle") return "Wait for the answer, or stop it";
  const last = s.items[s.items.length - 1];
  if (last?.kind === "error" && last.code === "SESSION_GONE") return "Start a new chat to send this";
  if (useEditor.getState().aiRun) return "Another agent is editing. Wait for it, or stop it in the top bar";
  const models = useCover.getState().models;
  if (thumbnail && models?.unavailable) return "Finding faces doesn't work on this computer, so the AI can't make a thumbnail here";
  if (thumbnail && models && !models.downloaded) return `Download the cover models first (${models.sizeMb} MB)`;
  return null;
}

/** Puts Make a thumbnail in the field of the AI panel, opening it: a note such as the hook is optional. */
export function makeThumbnail() {
  void loadModels();
  useAgent.setState({ thumbnail: true });
  useDock.setState({ open: true });
  // The panel may only now be drawn.
  window.setTimeout(() => document.querySelector<HTMLTextAreaElement>('aside[aria-label="AI"] textarea')?.focus(), 0);
}

/** Why Make it with AI cannot start now, or null; missing cover models are no reason, as it downloads them first. */
export function coverAiBlocked(): string | null {
  if (useCover.getState().models?.unavailable) return "Finding faces doesn't work on this computer, so the AI can't make a cover here";
  return sendBlocked(false);
}

/** Make it with AI in the cover editor: the agent picks the frame and writes the hook at once, after downloading the cover models when they are missing. */
export function makeCoverWithAi() {
  if (coverAiBlocked()) return;
  const start = () => {
    // The agent may have started answering while the models downloaded.
    const blocked = coverAiBlocked();
    if (blocked) return useEditor.getState().toast({ kind: "info", text: `Make it with AI did not start. ${blocked}.` });
    useAgent.setState({ thumbnail: true });
    useDock.setState({ open: true });
    void send("", "project");
  };
  const models = useCover.getState().models;
  if (models && !models.downloaded) void downloadModels(start);
  else start();
}

/** `attachments`: what goes with it; by default the draft's, when it sends the draft. */
export async function send(text = useAgent.getState().draft.trim(), about: AiAbout = useAgent.getState().about, attachments?: Attachment[]) {
  const s = useAgent.getState();
  const fromDraft = text === s.draft.trim();
  const attached = attachments ?? (fromDraft ? s.attachments : []);
  if ((!text && !attached.length && !(s.thumbnail && about === "project")) || sendBlocked()) return;
  const context = promptContext(about, attached);
  const thumbnailOnly = !text && context.thumbnail;
  useAgent.setState({
    // Earlier problems are over once a new message goes; their Try again would send this one.
    items: [
      ...s.items.filter((i) => i.kind !== "error"),
      { kind: "user", text: thumbnailOnly ? "Make a thumbnail" : text, context: describeContext(thumbnailOnly ? { ...context, thumbnail: false } : context), at: Date.now(), ...(attached.length ? { attachments: attached } : {}) },
    ],
    about,
    draft: fromDraft ? "" : s.draft,
    attachments: attachments === undefined && fromDraft ? [] : s.attachments,
    dropSelection: false,
    dropPlayhead: false,
    frame: false,
    thumbnail: false,
  });
  await start(text, context);
}

/** Why a message cannot go to the agent while it works, or null. */
export function steerBlocked(): string | null {
  const s = useAgent.getState();
  if (s.status === "stopping") return "Wait until it stops";
  if (s.thumbnail) return "Make a thumbnail waits for the answer, or stop it";
  // A second would go with the first one's selection and playhead.
  if (s.next || steering) return `${agentName(s.agent)} answers your last message first`;
  return null;
}

/** The turn's end waits for a message on its way to the working agent, as that decides whether it goes next. */
let steering: Promise<unknown> | null = null;

/** Sends a message to the agent while it works: it takes it at its next step, or as the next message when it cannot in this turn. */
export async function steer(text = useAgent.getState().draft.trim()) {
  const s = useAgent.getState();
  const fromDraft = text === s.draft.trim();
  const attached = fromDraft ? s.attachments : [];
  if ((!text && !attached.length) || s.status !== "working" || !s.turn || steerBlocked()) return;
  const context = promptContext(s.about, attached);
  const shown = { kind: "user" as const, text, context: describeContext(context), at: Date.now(), ...(attached.length ? { attachments: attached } : {}) };
  useAgent.setState({
    items: [...s.items, shown],
    draft: fromDraft ? "" : s.draft,
    attachments: fromDraft ? [] : s.attachments,
    dropSelection: false,
    dropPlayhead: false,
    frame: false,
  });
  const call = agentApi.steer(s.turn, text, context);
  const mine = (steering = call.catch(() => null));
  let steered;
  try {
    steered = await call;
  } catch (e) {
    // It did not reach the agent, so it is not part of the conversation.
    useAgent.setState((now) => ({ items: now.items.filter((it) => it !== shown), draft: now.draft ? `${text}\n${now.draft}` : text, attachments: [...attached, ...now.attachments] }));
    useEditor.getState().toast({ kind: "error", text: plainError(errorText(e)) });
    return;
  } finally {
    if (steering === mine) steering = null;
  }
  if (steered === "taken" || useAgent.getState().chat !== s.chat) return;
  useAgent.setState({ next: { text, context, stopped: steered === "stopped" } });
}

/** Starts a turn for a message the conversation already shows. */
async function start(text: string, context: PromptContext) {
  const s = useAgent.getState();
  // The chat is named here, before sending, so even an agent that fails at once is heard.
  const chat = s.chat ?? crypto.randomUUID();
  const turn = crypto.randomUUID();
  useAgent.setState({ chat, turn, jobs: [], status: "working", startedAt: Date.now() });
  try {
    const { model, effort } = resolved(s.agent);
    await agentApi.send(s.agent, chat, turn, text, context, model?.slug ?? null, effort);
    // Kept from its first message, so a crash during the answer does not lose the chat.
    if (useAgent.getState().chat === chat) void saveChat(chat, useAgent.getState().items);
  } catch (e) {
    const text = errorText(e);
    fail(text.startsWith("NOT_INSTALLED") ? "NOT_INSTALLED" : "AGENT_FAILED", plainError(text));
    void watchJobs();
  }
}

/** The run card's own Undo ran and undid the run: it says so instead of offering Undo again. */
export function markUndone(index: number) {
  useAgent.setState((s) => ({ items: s.items.map((it, i) => (i === index && it.kind === "run" ? { ...it, undone: true } : it)) }));
}

/** Answers the agent's question with the option picked. */
export function choose(id: string, label: string) {
  if (sendBlocked()) return;
  useAgent.setState((s) => ({ items: s.items.map((it) => (it.kind === "options" && it.id === id ? { ...it, chosen: label } : it)) }));
  void send(label);
}

export async function stop() {
  const { turn, status, jobs } = useAgent.getState();
  const between = status === "idle" && jobs.some((j) => j.status === "running");
  if (!turn || (status !== "working" && !between)) return;
  useAgent.setState(between ? { jobs: jobs.map((j) => ({ ...j, cancel_requested: true })) } : { status: "stopping" });
  try {
    await agentApi.stop(turn);
  } catch (e) {
    if (!between) useAgent.setState({ status: "working" });
    useEditor.getState().toast({ kind: "error", text: plainError(errorText(e)) });
  }
}

/**
 * Ends the turn with an error. A turn that failed before the agent answered gives its message back
 * to the field, so it is never lost; the user message leaves the conversation then. One that failed
 * later keeps its message on the error, for Try again.
 */
function fail(code: AgentErrorCode, message: string) {
  const s = useAgent.getState();
  let items = s.items;
  let draft = s.draft;
  let attachments = s.attachments;
  let retry: string | undefined;
  let retryAttachments: Attachment[] | undefined;
  const last = items[items.length - 1];
  // A message waiting for the next turn goes back to the field; it is the last one shown when nothing came after it.
  if (s.next && last?.kind !== "user") {
    draft = draft ? `${s.next.text}\n${draft}` : s.next.text;
    attachments = [...s.next.context.attachments, ...attachments];
  }
  if (last?.kind === "user") {
    // With the messages sent while it worked, which it did not answer either.
    const unanswered = items.slice(items.findLastIndex((it) => it.kind !== "user") + 1);
    items = items.slice(0, -unanswered.length);
    draft = [...unanswered.map((it) => (it.kind === "user" ? it.text : "")), ...(draft ? [draft] : [])].filter(Boolean).join("\n");
    attachments = [...unanswered.flatMap((it) => (it.kind === "user" ? (it.attachments ?? []) : [])), ...attachments];
  } else {
    const asked = [...items].reverse().find((i) => i.kind === "user");
    retry = asked?.kind === "user" ? asked.text : undefined;
    retryAttachments = asked?.kind === "user" ? asked.attachments : undefined;
  }
  useAgent.setState({ items: [...items, { kind: "error", code, message, retry, retryAttachments }], draft, attachments, status: "idle", next: null });
  // The sign-in ended or lost the plan meanwhile: the panel stops saying it uses the plan.
  if (s.agent === "chatgpt" && (code === "NOT_SIGNED_IN" || code === "PLAN_UNAVAILABLE")) void checkChatGpt(false);
}

function onEvent(e: AgentEvent) {
  const s = useAgent.getState();
  if (e.chat !== s.chat) return;
  if ((e.type === "done" || e.type === "error") && steering) return void steering.then(() => onEvent(e));
  const items = [...s.items];
  const last = items[items.length - 1];
  if (e.type === "text") {
    if (last?.kind === "agent" && !last.ended) items[items.length - 1] = { kind: "agent", text: last.text + e.text };
    else items.push({ kind: "agent", text: e.text });
    useAgent.setState({ items });
  } else if (e.type === "tool") {
    const name = toolName(e.name);
    if (HIDDEN.has(name)) {
      if (last?.kind === "agent" && !last.ended) {
        items[items.length - 1] = { ...last, ended: true };
        useAgent.setState({ items });
      }
      return;
    }
    if (name === OFFER) {
      const offer = offered(e.input);
      if (offer && !items.some((it) => it.kind === "options" && it.id === e.id)) useAgent.setState({ items: [...items, { kind: "options", id: e.id, chosen: null, ...offer }] });
      return;
    }
    const step: Step = { id: e.id, title: stepTitle(e.name, e.input), status: e.status, detail: stepDetail(e.name, e.input) };
    if (last?.kind === "steps") {
      const known = last.steps.find((x) => x.id === e.id);
      // Updates may come without the input; keep what the first one said.
      const steps = known ? last.steps.map((x) => (x.id === e.id ? { ...x, status: e.status, detail: step.detail ?? x.detail } : x)) : [...last.steps, step];
      items[items.length - 1] = { kind: "steps", steps };
    } else items.push({ kind: "steps", steps: [step] });
    useAgent.setState({ items });
  } else if (e.type === "error") {
    fail(e.code, e.message);
    void watchJobs();
  } else {
    // Codex stops for a message sent while it worked, which then goes next; the user's Stop, also Stop and edit, sends nothing more.
    const next = s.status === "working" && (!e.stopped || s.next?.stopped) ? s.next : null;
    const stopped = e.stopped && !next;
    // A stopped turn marks the step it was on; one that changed nothing says so.
    const marked = items.map((it) => (it.kind === "steps" ? { ...it, steps: it.steps.map((x) => (x.status === "running" ? { ...x, status: e.stopped ? ("stopped" as const) : ("done" as const) } : x)) } : it));
    const turnStart = marked.map((it) => it.kind).lastIndexOf("user") + 1;
    const turn = marked.slice(turnStart);
    // What the turn changed comes last, after the agent's closing words.
    // A run of a stopped turn was stopped, whether by the panel's Stop or Stop and edit in the top bar.
    const runs = turn.filter((it) => it.kind === "run").map((it) => (stopped ? { ...it, stopped: true } : it));
    const ordered = [...marked.slice(0, turnStart), ...turn.filter((it) => it.kind !== "run"), ...runs];
    const draft = s.next && !next ? (s.draft ? `${s.next.text}\n${s.draft}` : s.next.text) : s.draft;
    const attachments = s.next && !next ? [...s.next.context.attachments, ...s.attachments] : s.attachments;
    useAgent.setState({ items: stopped && !runs.length ? [...ordered, { kind: "stopped" }] : ordered, status: "idle", next: null, draft, attachments });
    if (next) void start(next.text, next.context);
    else void watchJobs();
  }
}

/** What the run ending next changed; it arrives just before the run ends. */
let pendingSummary: RunSummary | null = null;

/**
 * The run the panel's agent opened has ended. Returns true when the panel shows it (with Undo),
 * so the editor does not also toast it; a run from an agent in a terminal is not the panel's.
 */
export function panelRunEnded(label: string, snap: Snapshot | null): boolean {
  const s = useAgent.getState();
  const summary = pendingSummary;
  pendingSummary = null;
  if (s.status === "idle") return false;
  const run = {
    kind: "run" as const,
    label,
    snap,
    stopped: s.status === "stopping",
    changes: summary?.changes ?? null,
    runId: summary?.runId ?? null,
    cut: summary?.cut ?? false,
  };
  useAgent.setState({ items: [...s.items, run] });
  return true;
}

// Another project, or the app starting, reopens that project's last chat; the backend stops an agent still working on the old one.
useEditor.subscribe((s, prev) => {
  if (!s.snap || s.snap.sessionEpoch === prev.snap?.sessionEpoch) return;
  // What the agent did before another project opened is kept, even when it was cut short mid-answer.
  const old = useAgent.getState();
  if (old.chat) void saveChat(old.chat, old.items);
  useAgent.setState({ chat: null, pastCount: null, chats: null, items: [], jobs: [], status: "idle", next: null });
  void loadChats().then(() => {
    const { chats, chat, items, status } = useAgent.getState();
    if (chats?.length && chat === null && !items.length && status === "idle") void openChat(chats[0].id);
  });
});

// Between turns, a change such as a chosen option or an undone run is kept too.
useAgent.subscribe((s, prev) => {
  if (s.chat && s.items !== prev.items && s.status === "idle" && s.chat === prev.chat) void saveChat(s.chat, s.items);
});

/**
 * The panel's agent has its model list loaded, also with the panel closed, as Find reels and Make it with AI
 * send from elsewhere. ChatGPT's follows the signed-in account. Only then: nobody else needs the lists.
 */
function followModels() {
  const { agent, agents } = useAgent.getState();
  const { lists, errors, chatgptFor } = useModels.getState();
  const { status } = useChatGpt.getState();
  // Forgotten on sign-out whichever agent is in use, so signing in again loads them anew.
  if (status?.state !== "signedIn" || !status.plan) {
    if (chatgptFor !== null) forgetChatGptModels();
  } else if (agent === "chatgpt" && chatgptFor !== (status.email ?? "")) void loadAgentModels("chatgpt", status.email ?? "");
  if (agent !== "chatgpt" && !lists[agent] && !errors[agent] && agents?.find((a) => a.id === agent)?.path) void loadAgentModels(agent);
}
useChatGpt.subscribe((s, prev) => s.status !== prev.status && followModels());
useAgent.subscribe((s, prev) => (s.agent !== prev.agent || s.agents !== prev.agents) && followModels());

let listening = false;
export function listenAgentEvents() {
  if (listening) return;
  listening = true;
  void listen<AgentEvent>("agent", (e) => onEvent(e.payload));
  void listen<RunSummary>("run-summary", ({ payload }) => {
    // The counts arrive after the run's card when they took longer than ending the run.
    const { items } = useAgent.getState();
    if (!items.some((it) => it.kind === "run" && it.runId === payload.runId)) return void (pendingSummary = payload);
    const next = items.map((it) => (it.kind === "run" && it.runId === payload.runId ? { ...it, changes: payload.changes, cut: payload.cut } : it));
    useAgent.setState({ items: next });
  });
}

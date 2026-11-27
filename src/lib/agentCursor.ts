import { create } from "zustand";
import { agentName, useAgent } from "./agent";
import { type Action, plan } from "./agentPlan";
import { usePreview } from "./preview";
import { MAIN_TRACK, PLUGIN_RUN, type PanelTab, findClip, isCaptionTrack, useEditor } from "./store";
import { US } from "./time";
import type { Clip, Project } from "./types";

/**
 * The agent's pointer: while an AI agent edits, a pointer with its name acts out each change on the timeline and
 * looks through the clips between them. It also goes where the rest of the work happens: to the library tab a
 * change comes from, to the inspector tab of a setting it changed, and to the transcript, the preview or Export
 * while the agent reads there. The timeline keeps drawing the project as it was until the pointer gets there, at
 * most a second or two; the project, the preview and the export have the change at once. Settings › AI turns it off.
 */

const KEY = "nuzky.agentPointer";

export type Glyph = "arrow" | "scissors" | "resize" | "hand" | "grab";

/** How the timeline's clips go from the project it held to the one it draws next. */
export interface Release {
  from: Project;
  to: Project;
  ms: number;
  /** Clips whose length changed grow or shrink to it, as an edge dragged in a trim. */
  widths: boolean;
  /** New clips stay hidden until the pointer puts them in. */
  hideAdded: boolean;
  /** Where clips the pointer drags were on screen, so one taken to another track goes there with it. */
  rects?: Record<string, { x: number; y: number }>;
  /** Bands over the pieces cut out, which close with the gaps: to `left`, or fading where nothing closes. */
  bands?: { el: HTMLElement; left: number | null }[];
}

interface AgentCursorState {
  enabled: boolean;
  /** The project the timeline draws while the pointer is on its way to a change. */
  held: Project | null;
  /** The project the timeline held when everything was shown at once, so it jumps from it without sliding. */
  jumped: Project | null;
  release: Release | null;
  shown: boolean;
  name: string;
  color: string;
  step: string | null;
  glyph: Glyph;
  /** Where the label sits from the tip: left of it near the window's right edge, and under or over the row it works on. */
  label: { left: boolean; dy: number };
  /** The clip the pointer clicked, which the inspector shows while the agent works; the user's selection stays theirs. */
  inspected: string | null;
}

/** The label's place under the tip, and its height. */
const LABEL_DY = 21;
const LABEL_H = 24;

export const CLAY = "#d97757";
export const GEMINI = "#8ab4f8";
const WHITE = "#f2f2f5";

export const useAgentCursor = create<AgentCursorState>(() => ({
  enabled: localStorage.getItem(KEY) !== "0",
  held: null,
  jumped: null,
  release: null,
  shown: false,
  name: "",
  color: WHITE,
  step: null,
  glyph: "arrow",
  label: { left: false, dy: LABEL_DY },
  inspected: null,
}));

export function setAgentPointer(on: boolean) {
  localStorage.setItem(KEY, on ? "1" : "0");
  useAgentCursor.setState({ enabled: on });
  if (!on) flush();
}

/** The agent's name from its MCP client's, or the AI panel's agent, which runs in Nuzky. */
function who(client: string | null): { name: string; color: string } {
  const c = (client ?? "").toLowerCase();
  const panel = useAgent.getState();
  const name = c.includes("claude")
    ? "Claude Code"
    : c.includes("codex")
      ? "Codex"
      : c.includes("opencode")
        ? "OpenCode"
        : c.includes("gemini") || c.includes("antigravity")
          ? "Gemini"
          : c.includes("cursor")
            ? "Cursor"
            : panel.status !== "idle"
              ? agentName(panel.agent)
              : "AI agent";
  return { name, color: name === "Claude Code" ? CLAY : name === "Gemini" ? GEMINI : WHITE };
}

interface Nodes {
  root: HTMLElement;
  body: HTMLElement;
  glyph: HTMLElement;
  fx: HTMLElement;
}

let nodes: Nodes | null = null;

export function attachCursor(n: Nodes | null) {
  nodes = n;
  if (!n) flush();
}

class Stopped extends Error {}

/** The same curve for a dragged clip and the pointer dragging it. */
const SLIDE = "cubic-bezier(0.45, 0, 0.25, 1)";
const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");

let queue: { before: Project; after: Project }[] = [];
let playing = false;
let present = false;
/** Whether the pointer only looks through the clips, which an edit or the run's end cuts short where it is. */
let looking = false;
let ending = false;
let generation = 0;

/** Where in the window a tool's work belongs: a library tab, the preview, Export, the Cover tile, or the timeline itself. */
type Place = PanelTab | "preview" | "export" | "cover" | "timeline";

/** The tools whose work shows nowhere on the timeline, with what the label says meanwhile. A tool that edits it is acted out by its change. */
const READS: Record<string, { step: string; place: Place }> = {
  get_state: { step: "Reading the timeline", place: "timeline" },
  get_transcript: { step: "Reading the transcript", place: "transcript" },
  transcribe: { step: "Transcribing", place: "transcript" },
  analyze: { step: "Listening to the sound", place: "timeline" },
  activity: { step: "Scanning picture and sound", place: "timeline" },
  get_waveform: { step: "Listening to the sound", place: "timeline" },
  quiet_point: { step: "Finding a quiet spot to cut", place: "timeline" },
  audio_spectrum: { step: "Listening for hum and whine", place: "timeline" },
  inspect_frames: { step: "Checking frames", place: "preview" },
  frame_sheet: { step: "Looking through a clip", place: "preview" },
  describe_clip: { step: "Describing a clip", place: "preview" },
  track_object: { step: "Following an object", place: "preview" },
  inspect_image: { step: "Looking at a picture", place: "preview" },
  text_styles: { step: "Looking through text styles", place: "text" },
  search_sounds: { step: "Looking for a sound", place: "audio" },
  add_sound: { step: "Getting a sound", place: "audio" },
  find_broll: { step: "Looking for B-roll", place: "media" },
  import_media: { step: "Importing media", place: "media" },
  import_web_image: { step: "Getting a picture from the web", place: "media" },
  add_youtube_video: { step: "Getting a video from YouTube", place: "media" },
  inspect_thumbnail: { step: "Checking the thumbnail", place: "cover" },
  export_thumbnail: { step: "Exporting the thumbnail", place: "cover" },
  export_video: { step: "Exporting", place: "export" },
};
/** Where the pointer only stays over a control: pressing Export or the Cover tile would open it over the agent's work. */
const HOVERED: Partial<Record<Place, string>> = { export: '[data-tour="export"]', cover: '[data-testid="cover-tile"]' };
/** The library tab the changes of a tool come from. */
const EDITS: Record<string, PanelTab> = { edit_transcript: "transcript", correct_words: "transcript" };

/** The tool that reads now, until the pointer went where it reads. */
let reading: { step: string; place: Place; at: number } | null = null;
/** The tab the next change comes from, as its tool said just before it; the pointer opens it after acting the change out. */
let editing: { tab: PanelTab; at: number } | null = null;
/** What the pointer does once a change is acted out, which only the next change cuts short. */
let follow: (() => Promise<void>) | null = null;
let following = false;
/** When it last went to each place, so it does not go back and forth for every call. */
const visited = new Map<string, number>();
const AGAIN_MS = 8000;

const recent = (place: string) => performance.now() - (visited.get(place) ?? -AGAIN_MS) < AGAIN_MS;
/** Counted once the pointer got there, so a visit cut short on the way is made again. */
const went = (place: string) => void visited.set(place, performance.now());
/** Where the pointer is on its way to for a tool; another call of a tool that works there changes nothing. */
let visiting: Place | null = null;
let pos = { x: 0, y: 0 };
const running = new Set<Animation>();
let watchdog = 0;
let endCap = 0;

const agentRun = (run: string | null) => !!run && !run.startsWith(PLUGIN_RUN);

/** The timeline's lanes as they show on screen, or null when they do not. */
function lanes() {
  const scroller = document.querySelector<HTMLElement>("[data-timeline]");
  const { view: screen, launcherOpen } = useEditor.getState();
  if (!scroller || document.hidden || usePreview.getState().full || screen !== "editor" || launcherOpen) return null;
  const r = scroller.getBoundingClientRect();
  const ruler = scroller.querySelector("[data-ruler]")?.getBoundingClientRect();
  const view = { left: r.left + Number(scroller.dataset.header ?? 0), right: r.left + scroller.clientWidth, top: Math.max(r.top, ruler?.bottom ?? r.top), bottom: r.top + scroller.clientHeight };
  return view.right - view.left > 40 && view.bottom - view.top > 20 ? view : null;
}

type View = NonNullable<ReturnType<typeof lanes>>;

/** Where a time on a track is on screen, in the layout the timeline draws now. */
function at(trackId: string, us: number) {
  const row = document.querySelector<HTMLElement>(`[data-timeline] [data-track-id="${CSS.escape(trackId)}"]`);
  if (!row) return null;
  const r = row.getBoundingClientRect();
  return { x: r.left + (us / US) * useEditor.getState().zoom, y: r.top + r.height / 2, top: r.top, bottom: r.bottom };
}

const inView = (v: View, x: number, y: number) => x >= v.left + 4 && x <= v.right - 4 && y >= v.top && y <= v.bottom;

function active() {
  return useAgentCursor.getState().enabled && !!lanes();
}

const tr = (p: { x: number; y: number }) => `translate3d(${p.x}px, ${p.y}px, 0)`;

function place(p: { x: number; y: number }) {
  pos = p;
  if (nodes) nodes.root.style.transform = tr(p);
}

/**
 * Puts the label for what comes next, once, so it never jumps while the pointer works: clear of the row the pointer
 * works on, under it or over it where there is no room, and left of the pointer near the window's right edge.
 */
function aim(to: { x: number; y: number }, row?: { top: number; bottom: number }) {
  const room = (y: number) => y + LABEL_H <= window.innerHeight - 4;
  const dy = row ? (room(row.bottom + 6) ? row.bottom + 6 - to.y : row.top - 6 - LABEL_H - to.y) : room(to.y + LABEL_DY) ? LABEL_DY : -LABEL_DY - LABEL_H + 12;
  useAgentCursor.setState({ label: { left: to.x > window.innerWidth - 300, dy } });
}

let stepTimer = 0;
/** What the label says the agent does; once done it says only the agent's name. */
function say(step: string | null) {
  window.clearTimeout(stepTimer);
  useAgentCursor.setState({ step });
}

async function run(el: HTMLElement, frames: Keyframe[], ms: number, easing = "linear") {
  const gen = generation;
  const a = el.animate(frames, { duration: ms, easing, fill: "forwards" });
  running.add(a);
  try {
    await a.finished;
  } catch {
    throw new Stopped();
  } finally {
    running.delete(a);
  }
  if (gen !== generation) throw new Stopped();
  a.commitStyles();
  a.cancel();
}

async function pause(ms: number) {
  const gen = generation;
  await new Promise((r) => setTimeout(r, ms));
  if (gen !== generation) throw new Stopped();
}

const frame = () => new Promise((r) => requestAnimationFrame(() => r(null)));

const minJerk = (t: number) => t * t * t * (10 - 15 * t + 6 * t * t);

/** Moves the pointer as a hand does: along a slight arc, fast in the middle, a few pixels past the target and back. */
async function glide(to: { x: number; y: number }) {
  const n = nodes!;
  const from = { ...pos };
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const d = Math.hypot(dx, dy);
  if (d < 1) return;
  if (reducedMotion.matches) {
    await run(n.body, [{ opacity: 1 }, { opacity: 0 }], 80);
    place(to);
    await run(n.body, [{ opacity: 0 }, { opacity: 1 }], 120);
    return;
  }
  const ms = Math.min(650, Math.max(240, 150 + d * 0.55));
  const settle = Math.max(90, ms * 0.2);
  const [ux, uy] = [dx / d, dy / d];
  const over = Math.min(6, d * 0.025);
  const end = { x: to.x + ux * over, y: to.y + uy * over };
  const bow = Math.min(48, d * 0.1) * (dx >= 0 ? -1 : 1);
  const c = { x: from.x + dx / 2 - uy * bow, y: from.y + dy / 2 + ux * bow };
  const frames: Keyframe[] = [];
  for (let i = 0; i <= 18; i++) {
    const k = minJerk(i / 18);
    const p = { x: (1 - k) ** 2 * from.x + 2 * (1 - k) * k * c.x + k * k * end.x, y: (1 - k) ** 2 * from.y + 2 * (1 - k) * k * c.y + k * k * end.y };
    frames.push({ transform: tr(p), offset: ((i / 18) * ms) / (ms + settle), ...(i === 18 ? { easing: "cubic-bezier(0.2, 0.7, 0.3, 1)" } : {}) });
  }
  frames.push({ transform: tr(to), offset: 1 });
  await run(n.root, frames, ms + settle);
  place(to);
}

/** Drags in step with what it drags, on the same curve. */
async function drag(to: { x: number; y: number }, ms: number) {
  await run(nodes!.root, [{ transform: tr(pos) }, { transform: tr(to) }], ms, SLIDE);
  place(to);
}

/** Stays where it is for a moment while it only looks. */
const linger = (ms: number) => run(nodes!.root, [{ transform: tr(pos) }, { transform: tr(pos) }], ms);

/** Stops the pointer where it is while it only looks, as it gets work. */
function interrupt() {
  // What follows a change is finished even as the run ends or the agent reads on.
  if (!looking || !nodes || (following && !queue.length)) return;
  const m = new DOMMatrixReadOnly(getComputedStyle(nodes.root).transform);
  place({ x: m.m41, y: m.m42 });
  for (const a of [...running]) a.cancel();
  nodes.glyph.style.transform = "";
}

const libraryTab = (tab: PanelTab) => document.querySelector<HTMLElement>(`[data-tour="library"] [role="tab"][data-tab="${tab}"]`);

/** Where a control is, when the window shows all of it. */
function shown(el: HTMLElement | null) {
  const r = el?.getBoundingClientRect();
  return r && r.width >= 4 && r.height >= 4 && r.left >= 0 && r.top >= 0 && r.right <= window.innerWidth && r.bottom <= window.innerHeight ? r : null;
}

/** A dialog the user answers; a tab opened under it could take away what it belongs to. */
const dialogOpen = () => !!document.querySelector("dialog[open], [aria-modal='true']");

/** Goes to a control and presses it. A tab that is not the one shown opens, as a click on it does. */
async function click(el: HTMLElement | null) {
  const r = shown(el);
  if (!el || !r || dialogOpen()) return false;
  const to = { x: r.left + r.width / 2, y: r.top + r.height / 2 };
  aim(to, r);
  await glide(to);
  await down();
  if (el.isConnected && !dialogOpen() && el.getAttribute("aria-selected") === "false") el.click();
  await up();
  return true;
}

function ring() {
  if (!nodes || reducedMotion.matches) return;
  const r = document.createElement("div");
  r.className = "agent-ring";
  r.style.cssText = `left:${pos.x}px;top:${pos.y}px;border-color:${useAgentCursor.getState().color}`;
  nodes.fx.append(r);
  r.animate([{ transform: "scale(.2)", opacity: 0.9 }, { transform: "scale(1)", opacity: 0 }], { duration: 420, easing: "cubic-bezier(0.2, 0.7, 0.3, 1)" }).onfinish = () => r.remove();
}

async function down() {
  ring();
  if (!reducedMotion.matches) await run(nodes!.glyph, [{ transform: "scale(1)" }, { transform: "scale(0.86)" }], 80, "ease-out");
}

async function up() {
  if (!reducedMotion.matches) await run(nodes!.glyph, [{ transform: "scale(0.86)" }, { transform: "scale(1)" }], 150, "ease-out");
}

async function press() {
  await down();
  await up();
}

const glyph = (g: Glyph) => useAgentCursor.setState({ glyph: g });

/** An agent-coloured outline over a clip that fades, so the change is seen where it happened. */
function flash(id: string) {
  const el = document.querySelector<HTMLElement>(`[data-timeline] [data-clip-id="${CSS.escape(id)}"]`);
  if (!el || !nodes) return;
  const r = el.getBoundingClientRect();
  const f = document.createElement("div");
  f.className = "agent-flash";
  f.style.cssText = `left:${r.left - 5}px;top:${r.top - 5}px;width:${r.width + 10}px;height:${r.height + 10}px;border-color:${useAgentCursor.getState().color}`;
  nodes.fx.append(f);
  f.animate([{ opacity: 1 }, { opacity: 1, offset: 0.3 }, { opacity: 0 }], { duration: 900, easing: "ease-out" }).onfinish = () => f.remove();
}

/** Lets the timeline draw the next project; `to` is what it will draw. */
function release(before: Project, after: Project, ms: number, opts: { widths?: boolean; hideAdded?: boolean; rects?: Release["rects"]; bands?: Release["bands"] } = {}) {
  const more = queue.length > 0;
  const to = more ? after : (useEditor.getState().snap?.project ?? after);
  useAgentCursor.setState({ release: { from: before, to, ms, widths: !!opts.widths, hideAdded: !!opts.hideAdded, rects: opts.rects, bands: opts.bands }, held: more ? after : null });
  if (!more) window.clearTimeout(watchdog);
}

/** Puts in the clips that waited hidden for the pointer, left to right. */
function reveal(instant = false) {
  const els = [...document.querySelectorAll<HTMLElement>("[data-timeline] [data-clip-id]")].filter((el) => el.getAnimations().some((a) => a.id === "agent-hidden"));
  els.sort((a, b) => a.getBoundingClientRect().left - b.getBoundingClientRect().left);
  const gap = Math.min(30, 400 / Math.max(1, els.length));
  els.forEach((el, i) => {
    for (const a of el.getAnimations()) if (a.id === "agent-hidden") a.cancel();
    if (!instant) el.animate([{ opacity: 0, transform: "translateY(3px)" }, { opacity: 1, transform: "none" }], { duration: 180, delay: i * gap, easing: "ease-out", fill: "backwards" });
  });
}

function band(view: View, x: number, top: number, bottom: number) {
  const box = document.createElement("div");
  box.className = "agent-band-box";
  box.style.cssText = `left:${view.left}px;top:${view.top}px;width:${view.right - view.left}px;height:${view.bottom - view.top}px`;
  const b = document.createElement("div");
  b.className = "agent-band";
  const color = useAgentCursor.getState().color;
  const tint = `color-mix(in srgb, ${color} 20%, transparent)`;
  b.style.cssText = `left:${x - view.left}px;top:${Math.max(0, top - view.top)}px;height:${Math.min(bottom, view.bottom) - Math.max(top, view.top)}px;width:0;border-color:${color};background:linear-gradient(${tint}, ${tint}), rgb(10 10 11 / 0.5)`;
  box.append(b);
  nodes!.fx.append(box);
  return b;
}

type Cut = Extract<Action, { kind: "cut" }>;

/** Marks each piece cut out, snips, and the gaps close as the pieces go. */
async function cuts(actions: Cut[], before: Project, after: Project, view: View) {
  const zoom = useEditor.getState().zoom;
  const px = (us: number) => (us / US) * zoom;
  // A cut on the main track takes the same time out of every track that follows it, so its band spans them all.
  const rows = [...document.querySelectorAll("[data-timeline] [data-track-id]")].map((row) => row.getBoundingClientRect());
  const lanesTop = Math.min(...rows.map((row) => row.top));
  const lanesBottom = Math.max(...rows.map((row) => row.bottom));
  const marked: { el: HTMLElement; cut: Cut; x: number; y: number }[] = [];
  const quick = actions.length > 2;
  for (const cut of actions) {
    const a = at(cut.trackId, cut.fromUs);
    if (!a || !inView(view, a.x, a.y)) continue;
    const width = px(cut.toUs - cut.fromUs);
    const [top, bottom] = cut.trackId === MAIN_TRACK ? [lanesTop, lanesBottom] : [a.top, a.bottom];
    if (marked.length >= 6) {
      marked.push({ el: band(view, a.x, top, bottom), cut, x: a.x, y: a.y });
      marked.at(-1)!.el.style.width = `${width}px`;
      continue;
    }
    if (!marked.length) aim(a, a);
    await glide({ x: a.x, y: a.y });
    await down();
    const el = band(view, a.x, top, bottom);
    marked.push({ el, cut, x: a.x, y: a.y });
    const ms = quick ? 140 : Math.min(260, Math.max(120, width * 1.2));
    await Promise.all([run(el, [{ width: "0px" }, { width: `${width}px` }], ms, SLIDE), drag({ x: a.x + width, y: a.y }, ms)]);
    await up();
  }
  if (!marked.length) return release(before, after, 0);
  const last = marked.at(-1)!;
  glyph("scissors");
  await drag({ x: last.x + px(last.cut.toUs - last.cut.fromUs) / 2, y: last.y - 8 }, 120);
  await press();
  const ms = 260;
  // On the main track the clips after a piece close its gap; elsewhere the piece only goes. The timeline moves the
  // bands in the frame it moves the clips, so they close together.
  let closed = 0;
  const bands = marked.map(({ el, cut }) => {
    const left = cut.trackId === MAIN_TRACK ? parseFloat(el.style.left) - closed : null;
    if (cut.trackId === MAIN_TRACK) closed += px(cut.toUs - cut.fromUs);
    return { el, left };
  });
  release(before, after, ms, { bands });
  const joint = last.cut.trackId === MAIN_TRACK ? last.x - (closed - px(last.cut.toUs - last.cut.fromUs)) : pos.x;
  await Promise.all([drag({ x: joint, y: pos.y }, ms), pause(ms + 40)]);
  for (const { el } of marked) el.parentElement?.remove();
  glyph("arrow");
}

async function move(a: Extract<Action, { kind: "move" }>, before: Project, after: Project, view: View) {
  const from = at(a.trackId, a.fromUs + a.durationUs / 2);
  const now = findClip(after, a.clipId);
  if (!from || !now || !inView(view, from.x, from.y)) return release(before, after, 200);
  aim(from, from);
  await glide({ x: from.x, y: from.y + 4 });
  glyph("hand");
  await pause(120);
  glyph("grab");
  await down();
  const ms = 440;
  // A new track has no row until the timeline draws the project after the move.
  let to = now.track.id === a.trackId || document.querySelector(`[data-track-id="${CSS.escape(now.track.id)}"]`) ? at(now.track.id, a.toUs + a.durationUs / 2) : null;
  const was = document.querySelector(`[data-timeline] [data-clip-id="${CSS.escape(a.clipId)}"]`)?.getBoundingClientRect();
  release(before, after, ms, { rects: was ? { [a.clipId]: { x: was.left, y: was.top } } : undefined });
  if (!to) {
    await frame();
    to = at(now.track.id, a.toUs + a.durationUs / 2);
  }
  await drag(to ? { x: to.x, y: to.y + 4 } : pos, ms);
  await up();
  flash(a.clipId);
  glyph("hand");
  await pause(140);
  glyph("arrow");
}

async function trim(a: Extract<Action, { kind: "trim" }>, before: Project, after: Project, view: View) {
  const from = at(a.trackId, a.fromUs);
  const now = findClip(after, a.clipId)?.clip;
  if (!from || !now || !inView(view, from.x, from.y)) return release(before, after, 200, { widths: true });
  aim(from, from);
  await glide({ x: from.x, y: from.y });
  glyph("resize");
  await pause(100);
  await down();
  const ms = 380;
  const to = at(a.trackId, a.edge === "start" ? now.startUs : now.startUs + now.durationUs);
  release(before, after, ms, { widths: true });
  await drag({ x: to?.x ?? pos.x, y: from.y }, ms);
  await up();
  await pause(100);
  glyph("arrow");
}

async function split(a: Extract<Action, { kind: "split" }>, before: Project, after: Project, view: View) {
  const p = at(a.trackId, a.atUs);
  if (!p || !inView(view, p.x, p.y)) return release(before, after, 0);
  aim(p, p);
  await glide({ x: p.x, y: p.y });
  glyph("scissors");
  await pause(80);
  await down();
  release(before, after, 0);
  if (nodes) {
    const line = document.createElement("div");
    line.className = "agent-blade";
    line.style.cssText = `left:${p.x - 1}px;top:${p.top + 2}px;height:${p.bottom - p.top - 4}px;background:${useAgentCursor.getState().color}`;
    nodes.fx.append(line);
    line.animate([{ transform: "scaleY(0)", opacity: 1 }, { transform: "scaleY(1)", opacity: 1, offset: 0.3 }, { opacity: 0 }], { duration: 600, easing: "ease-out" }).onfinish = () => line.remove();
  }
  await up();
  await pause(120);
  glyph("arrow");
}

async function add(a: Extract<Action, { kind: "add" }>, before: Project, after: Project, view: View) {
  release(before, after, 0, { hideAdded: true });
  await frame();
  const el = a.clipIds.map((id) => document.querySelector<HTMLElement>(`[data-timeline] [data-clip-id="${CSS.escape(id)}"]`)).find((e) => {
    const r = e?.getBoundingClientRect();
    return r && inView(view, r.left + Math.min(r.width, 40) / 2, r.top + r.height / 2);
  });
  if (el) {
    const r = el.getBoundingClientRect();
    const to = { x: r.left + Math.min(r.width / 2, 60), y: r.top + r.height / 2 };
    aim(to, r);
    await glide(to);
    await down();
    reveal();
    await up();
  }
  reveal(true);
}

async function change(a: Extract<Action, { kind: "change" }>, before: Project, after: Project, view: View) {
  const p = at(a.trackId, a.atUs + Math.min(a.durationUs / 2, (60 / useEditor.getState().zoom) * US));
  if (!p || !inView(view, p.x, p.y)) return release(before, after, 0);
  aim(p, p);
  await glide({ x: p.x, y: p.y });
  await down();
  release(before, after, 0);
  // The inspector shows the clip it clicked, with the settings that changed.
  useAgentCursor.setState({ inspected: a.clipId });
  flash(a.clipId);
  await up();
  const [was, now] = [findClip(before, a.clipId)?.clip, findClip(after, a.clipId)?.clip];
  if (was && now) follow = () => settings(was, now);
}

const AUDIO = ["volume", "fadeInUs", "fadeOutUs", "cleanVoice", "dereverb", "normalize", "leveler", "duckDb", "mono"];
const SPEED = ["speed", "keepPitch", "reverse", "ramp"];
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/** The inspector's tab that shows what changed on a clip. */
function settingsTab(a: Clip, b: Clip) {
  const [ca, cb] = [a.content as Record<string, unknown>, b.content as Record<string, unknown>];
  const differs = (keys: string[]) => keys.some((k) => !same(ca[k], cb[k]));
  if (!same([a.animIn, a.animOut], [b.animIn, b.animOut])) return "animation";
  if (differs(["text", "style"])) return "text";
  if (differs(AUDIO)) return "audio";
  if (differs(SPEED)) return "speed";
  if (differs(["adjust", "lut", "log"]) || !same(a.effects, b.effects)) return "adjust";
  return cb.type === "text" ? "transform" : "video";
}

/** After a click on a changed clip: the pointer opens the tab of the setting that changed, in the inspector or the library. */
async function settings(was: Clip, now: Clip) {
  const gen = generation;
  await frame();
  await frame();
  if (gen !== generation) throw new Stopped();
  if (queue.length) return;
  const tab = settingsTab(was, now);
  const inspector = (id: string) => document.querySelector<HTMLElement>(`[role="tablist"][aria-label="Clip settings"] [data-tab="${id}"]`);
  // A sound on its own has its speed in Audio.
  const el = !same(was.transitionIn, now.transitionIn) ? libraryTab("transitions") : (inspector(tab) ?? (tab === "speed" ? inspector("audio") : null));
  const place = `${now.id} ${tab}`;
  if (recent(place) || !(await click(el))) return linger(ending ? 300 : 1000);
  went(place);
  // The run is over: the pointer has little time left to go back.
  await linger(ending ? 300 : between(600, 900));
}

/** The library tab that has what was added. */
function source(first: Action, after: Project): PanelTab | null {
  if (first.kind !== "add") return null;
  const track = after.tracks.find((t) => t.id === first.trackId);
  const content = track?.clips.find((c) => first.clipIds.includes(c.id))?.content;
  if (!track || !content) return null;
  return isCaptionTrack(track) ? "captions" : content.type === "text" ? "text" : track.kind === "audio" ? "audio" : content.type === "media" ? "media" : null;
}

/** The label says only the agent's name again, a moment after what it did. */
const done = (ms: number) => (stepTimer = window.setTimeout(() => useAgentCursor.setState({ step: null }), ms));

async function perform(before: Project, after: Project) {
  const view = lanes();
  const p = view && plan(before, after);
  // The tab the change comes from, as its tool said just before it.
  const said = editing && performance.now() - editing.at < 2000 ? editing.tab : null;
  editing = follow = null;
  if (!view || !p) {
    // A thumbnail the agent made or changed is nowhere on the timeline but on the Cover tile.
    if (view && !same(before.thumbnails, after.thumbnails)) {
      follow = async () => {
        say("Making the thumbnail");
        try {
          if (recent("cover") || !(await hover(HOVERED.cover!))) await linger(ending ? 300 : 1000);
          else went("cover");
        } finally {
          done(800);
        }
      };
    }
    return release(before, after, 0);
  }
  say(p.step);
  const first = p.actions[0];
  // What is added comes out of its library tab, opened first. The tab of a cut by the transcript opens after the cut,
  // so the timeline never waits for it.
  const from = source(first, after);
  if (from && !recent(from) && (await click(libraryTab(from)))) {
    went(from);
    await pause(160);
  }
  if (said && !from) {
    follow = async () => {
      if (recent(said) || !(await click(libraryTab(said)))) return linger(ending ? 300 : 1000);
      went(said);
      await linger(ending ? 300 : 700);
    };
  }
  if (first.kind === "cut") await cuts(p.actions as Cut[], before, after, view);
  else if (first.kind === "move") await move(first, before, after, view);
  else if (first.kind === "trim") await trim(first, before, after, view);
  else if (first.kind === "split") await split(first, before, after, view);
  else if (first.kind === "add") await add(first, before, after, view);
  else await change(first, before, after, view);
  done(800);
}

/** With reduced motion the pointer only appears where the change is, after it. */
async function jump(before: Project, after: Project) {
  const view = lanes();
  const a = view && plan(before, after)?.actions[0];
  if (!a) return;
  await frame();
  const clip = "clipId" in a ? findClip(after, a.clipId) : null;
  const [trackId, us] =
    a.kind === "move" && clip
      ? [clip.track.id, a.toUs + a.durationUs / 2]
      : a.kind === "trim" && clip
        ? [a.trackId, a.edge === "start" ? clip.clip.startUs : clip.clip.startUs + clip.clip.durationUs]
        : [a.trackId, a.kind === "cut" || a.kind === "trim" || a.kind === "move" ? a.fromUs : a.atUs];
  const p = at(trackId, us);
  if (!p || !inView(view, p.x, p.y)) return;
  aim(p, p);
  await glide({ x: p.x, y: p.y });
}

/** Where the pointer comes from and goes back to: the AI button. */
function home() {
  const r = document.querySelector("[data-ai-toggle]")?.getBoundingClientRect();
  return r ? { x: r.left + r.width / 2, y: r.top + r.height / 2 } : null;
}

/** Where the pointer lands as it comes out: in the empty lane under the tracks, off the clips, when there is room. */
function rest(view: View) {
  const rows = [...document.querySelectorAll("[data-timeline] [data-track-id]")].map((row) => row.getBoundingClientRect());
  const below = rows.length ? Math.max(...rows.map((row) => row.bottom)) : view.top;
  const x = view.left + (view.right - view.left) * 0.62;
  return { x, y: view.bottom - below >= 12 + LABEL_DY + LABEL_H ? below + 12 : view.top + Math.min(60, (view.bottom - view.top) / 2) };
}

/** How far along a clip from where it is the pointer looks next, so it works its way along the timeline. */
const REACH = 260;
/** Clips this near are the ones it looks through; now and then it goes to one further off, so it never stays on one for good. */
const NEAR = 400;
/** How fast it reads along a clip, in px per ms. */
const READ = 0.16;
const between = (a: number, b: number) => a + Math.random() * (b - a);

/** The parts of clips the timeline shows, each with its ends that are on screen. */
function clipsInView(view: View) {
  const { snap, zoom } = useEditor.getState();
  const [left, right] = [view.left + 8, view.right - 8];
  const seen: { from: number; to: number; y: number; top: number; bottom: number; ends: number[] }[] = [];
  for (const track of snap?.project.tracks ?? []) {
    const row = at(track.id, 0);
    if (!row || row.y < view.top || row.y > view.bottom) continue;
    for (const clip of track.clips) {
      const start = row.x + (clip.startUs / US) * zoom;
      const end = start + (clip.durationUs / US) * zoom;
      const [from, to] = [Math.max(left, start), Math.min(right, end)];
      if (to - from >= 12) seen.push({ from, to, y: row.y, top: row.top, bottom: row.bottom, ends: [start, end].filter((x) => x >= left && x <= right) });
    }
  }
  return seen;
}

/**
 * What the pointer does while the agent thinks: it looks through the clips near it as an editor does before a cut,
 * reading along one, going back over a part of it, stopping on a cut. It presses nothing and leaves no marks.
 */
async function look(view: View) {
  const nearest = clipsInView(view)
    .map((c) => ({ ...c, far: Math.hypot(Math.min(Math.max(pos.x, c.from), c.to) - pos.x, c.y - pos.y) }))
    .sort((a, b) => a.far - b.far)
    .slice(0, 4);
  const near = Math.random() < 0.15 ? nearest : nearest.filter((c, i) => i === 0 || c.far <= NEAR);
  const c = near[Math.floor(Math.random() * near.length)];
  if (!c) return linger(600);
  const x = Math.min(Math.max(pos.x + between(-REACH, REACH), c.from), c.to);
  const cut = c.ends.sort((a, b) => Math.abs(a - x) - Math.abs(b - x))[0];
  if (cut !== undefined && Math.abs(cut - pos.x) > 20 && Math.random() < 0.3) {
    aim({ x: cut, y: c.y }, c);
    await glide({ x: cut, y: c.y });
    return linger(between(450, 950));
  }
  aim({ x, y: c.y }, c);
  // Already there, as after stopping on a cut, it reads on at once.
  const there = Math.hypot(x - pos.x, c.y - pos.y) < 8;
  await glide({ x, y: c.y });
  if (!there) await linger(between(150, 400));
  // Mostly forwards, as the video plays; the other way where the clip ends.
  const ahead = (Math.random() < 0.75 ? 1 : -1) * between(70, 240);
  const along = (by: number) => Math.min(Math.max(x + by, c.from), c.to);
  const to = Math.abs(along(ahead) - x) >= 30 ? along(ahead) : along(-ahead);
  if (Math.abs(to - x) >= 12) {
    await drag({ x: to, y: c.y }, Math.max(300, Math.abs(to - x) / READ));
    if (Math.random() < 0.35) {
      await linger(between(120, 260));
      await drag({ x: to + (x - to) * between(0.3, 0.6), y: c.y }, Math.max(220, Math.abs(to - x) / READ / 2));
    }
  }
  await linger(Math.random() < 0.2 ? between(1000, 1600) : between(350, 900));
}

/** Goes over a control and stays a moment without pressing it. */
async function hover(selector: string) {
  const r = shown(document.querySelector<HTMLElement>(selector));
  if (!r) return false;
  const to = { x: r.left + r.width / 2, y: r.top + r.height / 2 };
  aim(to, r);
  await glide(to);
  await linger(between(900, 1400));
  return true;
}

/** Goes where a tool works and stays a moment; on the timeline it looks through the clips. */
async function visit(tool: { step: string; place: Place }, view: View) {
  say(tool.step);
  const hovered = HOVERED[tool.place];
  const el = tool.place === "timeline" ? null : tool.place === "preview" ? document.querySelector<HTMLElement>("canvas[data-preview-frame]") : hovered ? document.querySelector<HTMLElement>(hovered) : libraryTab(tool.place as PanelTab);
  const r = shown(el);
  if (!r || recent(tool.place)) return look(view);
  visiting = tool.place;
  if (tool.place === "preview") {
    const inside = () => ({ x: r.left + r.width * between(0.3, 0.7), y: r.top + r.height * between(0.3, 0.7) });
    const to = inside();
    aim(to);
    await glide(to);
    went(tool.place);
    await linger(between(500, 900));
    await glide(inside());
    return linger(between(600, 1000));
  }
  if (hovered ? await hover(hovered) : await click(el)) went(tool.place);
  if (!hovered) await linger(between(700, 1100));
}

/** The agent that owns the run called a tool: the pointer goes where that work happens. */
export function agentUsed(name: string) {
  if (!present || reducedMotion.matches || !agentRun(useEditor.getState().aiRun)) return;
  if (EDITS[name]) return void (editing = { tab: EDITS[name], at: performance.now() });
  const tool = READS[name];
  if (!tool) return;
  if (visiting === tool.place) return say(tool.step);
  reading = { ...tool, at: performance.now() };
  void pump();
}

const scaled = (from: number, to: number): Keyframe[] =>
  reducedMotion.matches ? [{ opacity: from }, { opacity: to }] : [{ opacity: from, transform: `scale(${0.6 + 0.4 * from})` }, { opacity: to, transform: `scale(${0.6 + 0.4 * to})` }];

async function enter() {
  const view = lanes();
  if (!view || !nodes) return;
  const to = rest(view);
  place(home() ?? to);
  aim(to);
  useAgentCursor.setState({ shown: true });
  present = true;
  await run(nodes.body, scaled(0, 1), 220, "ease-out");
  await glide(to);
}

async function leave() {
  if (!present || !nodes) return;
  const back = home();
  if (back) await glide(back);
  await run(nodes.body, scaled(1, 0), 180, "ease-in");
  hide();
}

function hide() {
  present = false;
  window.clearTimeout(stepTimer);
  useAgentCursor.setState({ shown: false, step: null, glyph: "arrow", inspected: null });
}

async function pump() {
  if (playing) return interrupt();
  playing = true;
  const gen = generation;
  try {
    if (!present && (agentRun(useEditor.getState().aiRun) || queue.length)) await enter();
    for (;;) {
      const acted = queue.length > 0;
      while (queue.length) {
        const before = queue[0].before;
        const after = queue.at(-1)!.after;
        queue = [];
        if (reducedMotion.matches) {
          release(before, after, 0);
          await jump(before, after);
          continue;
        }
        await perform(before, after);
      }
      // The last change of a run still gets what follows it, as the tab of a setting, before the pointer goes back.
      const after = present ? follow : null;
      follow = null;
      if (ending && !after) {
        await leave();
        ending = false;
        window.clearTimeout(endCap);
        break;
      }
      if (!ending && (!present || reducedMotion.matches || !agentRun(useEditor.getState().aiRun))) break;
      const view = lanes();
      const tool = reading && performance.now() - reading.at < 3000 ? reading : null;
      looking = true;
      try {
        // A change it acted out stays under it for a moment, so it reads before the pointer moves on. While the video
        // plays the timeline moves under the pointer, so it waits.
        if (after) {
          following = true;
          await after();
        } else if (acted || !view || useEditor.getState().playing) await linger(1000);
        else if (tool) {
          reading = null;
          try {
            await visit(tool, view);
          } finally {
            visiting = null;
            done(2500);
          }
        } else await look(view);
      } catch (e) {
        if (!(e instanceof Stopped) || gen !== generation) throw e;
      } finally {
        if (gen === generation) looking = following = false;
      }
    }
  } catch (e) {
    if (!(e instanceof Stopped)) {
      console.error(e);
      flush();
    }
  } finally {
    if (gen === generation) playing = false;
  }
  if (gen === generation && (queue.length || (ending && present))) void pump();
}

/** A hold lasts at most 5 s from its start, however many edits follow it or however an animation stalls. */
function armWatchdog() {
  window.clearTimeout(watchdog);
  watchdog = window.setTimeout(flush, 5000);
}

/** Shows the project as it is now and puts the pointer away at once. */
export function flush() {
  generation++;
  queue = [];
  playing = false;
  ending = false;
  looking = false;
  reading = editing = follow = visiting = null;
  following = false;
  visited.clear();
  for (const a of [...running]) a.cancel();
  running.clear();
  window.clearTimeout(watchdog);
  window.clearTimeout(endCap);
  reveal(true);
  nodes?.fx.replaceChildren();
  if (nodes) for (const a of nodes.body.getAnimations()) a.cancel();
  hide();
  useAgentCursor.setState({ held: null, jumped: useAgentCursor.getState().held, release: null });
}

useEditor.subscribe((s, prev) => {
  // What the user selects themselves is what the inspector shows again.
  // A snapshot makes the selection's list anew, so they compare by what they hold.
  if (useAgentCursor.getState().inspected && (s.cut !== prev.cut || (s.selection !== prev.selection && !same(s.selection, prev.selection)))) useAgentCursor.setState({ inspected: null });
  const started = agentRun(s.aiRun) && !agentRun(prev.aiRun);
  if (s.snap && prev.snap && s.snap.sessionEpoch !== prev.snap.sessionEpoch) return flush();
  // Playing, moving the playhead or dragging media in needs the timeline as it is.
  const busy = (s.playing && !prev.playing) || s.seeks !== prev.seeks || (!!s.fileDrag && !prev.fileDrag) || (!!s.assetDrag && !prev.assetDrag);
  // Clips the pointer has yet to put in wait hidden even once the timeline holds nothing back.
  if (busy && (present || useAgentCursor.getState().held)) return flush();
  // The home screen or the launcher covers the timeline, so the pointer goes with it.
  if ((present || useAgentCursor.getState().held) && (s.view !== prev.view || s.launcherOpen !== prev.launcherOpen) && !lanes()) return flush();
  if (started && active()) {
    ending = false;
    useAgentCursor.setState({ ...who(s.snap?.openRunAgent ?? null), glyph: "arrow" });
    say(null);
    void pump();
  }
  // The run's name arrives before its snapshot, which names the agent.
  const client = s.snap?.openRunAgent;
  if (agentRun(s.aiRun) && client && client !== prev.snap?.openRunAgent) useAgentCursor.setState(who(client));
  if (!s.aiRun && prev.aiRun && (present || playing)) {
    ending = true;
    window.clearTimeout(endCap);
    // The last change shows within seconds of the run's end, however much is left to act out.
    endCap = window.setTimeout(flush, 4000);
    void pump();
  }
  if (!s.snap || !prev.snap || s.snap === prev.snap || s.snap.revision <= prev.snap.revision) return;
  // While the video plays the timeline shows what plays, and while media is dragged it shows where it would land, so an
  // edit then is not acted out.
  if (!agentRun(s.aiRun) || !active() || s.playing || s.fileDrag || s.assetDrag) {
    if (useAgentCursor.getState().held) flush();
    return;
  }
  if (!reducedMotion.matches && !useAgentCursor.getState().held) {
    useAgentCursor.setState({ held: prev.snap.project });
    armWatchdog();
  }
  queue.push({ before: prev.snap.project, after: s.snap.project });
  void pump();
});

// A resized window moves the timeline under a resting pointer, so it goes back over the lanes.
window.addEventListener("resize", () => {
  const view = present && lanes();
  if (!view || useAgentCursor.getState().held) return;
  const to = { x: Math.min(Math.max(pos.x, view.left + 8), view.right - 8), y: Math.min(Math.max(pos.y, view.top + 8), view.bottom - 8) };
  aim(to);
  place(to);
});

usePreview.subscribe((s, prev) => s.full && !prev.full && (present || useAgentCursor.getState().held) && flush());

// Once the run is over, the person's first press or key shows everything at once. A press on a timeline that still
// draws the project before the last change would start a drag or a box from the clips as drawn, so it does only that;
// React draws the timeline as it is before the click or menu of the same press arrives, so those act on what is there.
for (const type of ["pointerdown", "keydown"] as const) {
  window.addEventListener(
    type,
    (e) => {
      if (useEditor.getState().aiRun || !(present || useAgentCursor.getState().held)) return;
      const stale = !!useAgentCursor.getState().held && e.type === "pointerdown" && !!(e.target as Element | null)?.closest?.("[data-timeline]");
      flush();
      if (stale) {
        e.stopPropagation();
        e.preventDefault();
      }
    },
    true,
  );
}

/** For the timeline: where a new clip's frames were in the held project, so it slides from there. */
function origin(from: Project, clip: Clip, trackId: string) {
  const c = clip.content;
  if (c.type !== "media") return null;
  for (const o of from.tracks.find((t) => t.id === trackId)?.clips ?? []) {
    const oc = o.content;
    if (oc.type === "media" && oc.assetId === c.assetId && oc.speed > 0 && c.sourceInUs >= oc.sourceInUs && c.sourceInUs < oc.sourceInUs + o.durationUs * oc.speed) {
      return o.startUs + (c.sourceInUs - oc.sourceInUs) / oc.speed;
    }
  }
  return null;
}

/** Called by the timeline as it draws the released project: its clips travel from where the pointer found them. */
export function playRelease(lanesEl: HTMLElement, r: Release, zoom: number) {
  const px = (us: number) => (us / US) * zoom;
  const width = (us: number) => Math.max(2, px(us) - 2);
  const was = new Map(r.from.tracks.flatMap((t) => t.clips.map((c) => [c.id, { clip: c, track: t.id }] as const)));
  const now = new Map(r.to.tracks.flatMap((t) => t.clips.map((c) => [c.id, { clip: c, track: t.id }] as const)));
  const timing = { duration: r.ms, easing: SLIDE, id: "agent-slide" };
  for (const { el, left } of r.bands ?? []) {
    const from = parseFloat(el.style.left);
    el.animate(left === null ? [{ opacity: 1 }, { opacity: 0 }] : [{ left: `${from}px`, width: el.style.width }, { left: `${left}px`, width: "0px" }], { ...timing, fill: "forwards" });
  }
  for (const el of lanesEl.querySelectorAll<HTMLElement>("[data-clip-id]")) {
    const b = now.get(el.dataset.clipId!);
    if (!b) continue;
    for (const a of el.getAnimations()) if (a.id === "cut-slide" || a.id === "agent-slide") a.cancel();
    const a = was.get(el.dataset.clipId!);
    if (!a) {
      const start = origin(r.from, b.clip, b.track);
      if (start === null && r.hideAdded) el.animate([{ opacity: 0 }, { opacity: 0 }], { id: "agent-hidden", duration: 4000 });
      else if (start !== null && r.ms > 0 && Math.abs(px(start - b.clip.startUs)) >= 1) el.animate([{ transform: `translateX(${px(start - b.clip.startUs)}px)` }, { transform: "none" }], timing);
      continue;
    }
    const rect = r.rects?.[el.dataset.clipId!];
    if (rect && r.ms > 0) {
      const now = el.getBoundingClientRect();
      el.animate([{ transform: `translate(${rect.x - now.left}px, ${rect.y - now.top}px)` }, { transform: "none" }], timing);
      continue;
    }
    if (a.track !== b.track || r.ms === 0) continue;
    const dx = px(a.clip.startUs - b.clip.startUs);
    const resize = r.widths && a.clip.durationUs !== b.clip.durationUs;
    if (Math.abs(dx) < 1 && !resize) continue;
    el.animate(
      resize
        ? [
            { transform: `translateX(${dx}px)`, width: `${width(a.clip.durationUs)}px` },
            { transform: "none", width: `${width(b.clip.durationUs)}px` },
          ]
        : [{ transform: `translateX(${dx}px)` }, { transform: "none" }],
      timing,
    );
  }
}

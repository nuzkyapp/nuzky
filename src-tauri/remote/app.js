// The page a phone or another computer opens to work on the project. Nuzky on the computer does the work; this page
// sends what the user asks for (src-tauri/src/remote.rs) and shows the state the editor publishes (src/lib/remote.ts).
"use strict";

const $ = (id) => document.getElementById(id);
const US = 1_000_000;
const SVG = (paths) => `<svg viewBox="0 0 24 24" aria-hidden="true">${paths}</svg>`;
const ICONS = {
  check: SVG('<path d="M20 6 9 17l-5-5"/>'),
  x: SVG('<path d="M18 6 6 18M6 6l12 12"/>'),
  down: SVG('<path d="m6 9 6 6 6-6"/>'),
  right: SVG('<path d="m9 18 6-6-6-6"/>'),
  left: SVG('<path d="m15 18-6-6 6-6"/>'),
  film: SVG('<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M7 3v18M3 7.5h4M3 12h18M3 16.5h4M17 3v18M17 7.5h4M17 16.5h4"/>'),
  image: SVG('<rect width="18" height="18" x="3" y="3" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.1-3.1a2 2 0 0 0-2.8 0L6 21"/>'),
  type: SVG('<path d="M4 7V4h16v3M9 20h6M12 4v16"/>'),
  cc: SVG('<rect width="18" height="14" x="3" y="5" rx="2"/><path d="M7 15h4M15 15h2M7 11h2M13 11h4"/>'),
  music: SVG('<path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>'),
  chats: SVG('<path d="M14 9a2 2 0 0 1-2 2H6l-4 4V4a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2z"/><path d="M18 9h2a2 2 0 0 1 2 2v11l-4-4h-6a2 2 0 0 1-2-2v-1"/>'),
  newchat: SVG('<path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.4 2.6a1 1 0 0 1 3 3l-9 9a2 2 0 0 1-.85.5l-2.87.84a.5.5 0 0 1-.62-.62l.84-2.87a2 2 0 0 1 .5-.85z"/>'),
};

/** What the editor last published; null until the first state arrives. */
let state = null;
/** When it arrived, so the playhead moves on between states while the video plays. */
let stateAt = 0;
/** Files on their way to the computer, in the order they were picked. */
const uploads = [];
/** A message written while files were still on their way, and how many clips the timeline has once they are on it. */
let held = null;
/** When the last file arrived, so a message waits for it only so long when the computer could not import it. */
let lastArrival = 0;
let heldTimer = 0;
/** The export and its phase whose sheet the user closed, so it does not open again until the next phase. */
let dismissed = null;
let closed = false;
/** The live stream is open; while it is not, nothing is sent. */
let online = false;
let offlineSince = Date.now();
/** The clip the user tapped; its actions take the AI's place. */
let selected = null;
/** The open sheet: "agents", "chats" or "projects". */
let sheet = null;
const PHONE = /iPhone|iPad|Android/.test(navigator.userAgent) || (/Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1);
const IOS = /iPhone|iPad/.test(navigator.userAgent) || (/Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1);

function post(action) {
  return fetch("/action", { method: "POST", headers: { "Content-Type": "application/json", "X-Nuzky": "1" }, body: JSON.stringify(action) }).then(
    (r) => {
      if (r.status === 403) expired();
      return r.ok;
    },
    () => false,
  );
}

let toastTimer = 0;
function toast(text, ms = 4000) {
  const el = $("toast");
  el.textContent = text;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.hidden = true), ms);
}

function clock(us) {
  const s = Math.max(0, Math.round(us / US));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function tenths(us) {
  const d = Math.max(0, Math.floor(us / 100_000));
  return `${Math.floor(d / 600)}:${String(Math.floor((d % 600) / 10)).padStart(2, "0")}.${d % 10}`;
}

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function icon(name) {
  const t = el("template");
  t.innerHTML = ICONS[name];
  return t.content.firstChild;
}

/** A control that looks disabled but still takes a tap, which says why: a phone shows no tooltip. */
function gate(b, why) {
  b.setAttribute("aria-disabled", String(!!why));
  b.dataset.why = why || "";
}

function setConn(kind, text) {
  const c = $("conn");
  c.className = `conn ${kind}`;
  c.lastElementChild.textContent = text;
}

function expired() {
  closed = true;
  setConn("gone", "Link expired. Scan the code again.");
  render();
}

/** Why nothing can be sent now, or null. */
function offReason() {
  if (closed) return "This link no longer works";
  if (!online || !state) return "Nuzky is not connected";
  return null;
}

const editReason = () => offReason() ?? state.editBlocked;
const agentBusy = () => offReason() ?? (state.working ? "Wait until the AI is done, or stop it" : null);

// ---- Live state ----

function connect() {
  const events = new EventSource("/events");
  events.addEventListener("open", () => {
    online = true;
    setConn("on", "Connected to Nuzky");
    render();
  });
  events.addEventListener("state", (e) => {
    state = JSON.parse(e.data);
    stateAt = performance.now();
    render();
    sendHeld();
  });
  events.addEventListener("frame", loadFrame);
  events.addEventListener("end", () => {
    events.close();
    closed = true;
    setConn("gone", "Sharing stopped on the computer");
    render();
  });
  events.addEventListener("error", () => {
    if (closed) return;
    if (online) offlineSince = Date.now();
    online = false;
    setConn("off", offlineText());
    render();
    // The browser retries a lost connection itself but gives up on an answer that is not the stream: a refused
    // cookie, or a server that came back otherwise. Which one, another request tells.
    if (events.readyState === EventSource.CLOSED)
      fetch("/app.css", { cache: "no-store" }).then(
        (r) => (r.status === 403 ? expired() : setTimeout(connect, 2000)),
        () => setTimeout(connect, 2000),
      );
  });
}

const offlineText = () => (Date.now() - offlineSince > 10_000 ? "Can't reach Nuzky. Check the Wi-Fi." : "Reconnecting…");
setInterval(() => {
  if (!online && !closed) setConn("off", offlineText());
}, 2000);

let frameLoading = false;
let frameAgain = false;
let frameNumber = 0;
function loadFrame() {
  if (frameLoading) {
    frameAgain = true;
    return;
  }
  frameLoading = true;
  const img = new Image();
  const done = () => {
    frameLoading = false;
    if (frameAgain) {
      frameAgain = false;
      loadFrame();
    }
  };
  img.onload = () => {
    const shown = $("frame");
    shown.src = img.src;
    // An empty timeline draws only black.
    shown.hidden = !state?.clips.length;
    done();
  };
  img.onerror = done;
  img.src = `/frame.jpg?n=${++frameNumber}`;
}

// ---- Rendering ----

function render() {
  const s = state;
  $("name").textContent = s?.project || "Nuzky";
  document.title = s?.project ? `${s.project} · Nuzky` : "Nuzky";
  const hasClips = !!s && s.clips.length > 0;
  const busyAdding = uploads.some((u) => u.status !== "failed") || (s?.adding.length ?? 0) > 0;
  $("add-first").hidden = hasClips || busyAdding || !s || closed;
  // The first frame with something on the timeline may come before the state that says so.
  if (hasClips && $("frame").hidden) loadFrame();
  if (!hasClips) $("frame").hidden = true;
  for (const id of ["add", "add-first", "projects", "title"]) $(id).disabled = closed;
  renderTransport(hasClips);
  renderTimeline(hasClips || busyAdding || uploads.length > 0);
  renderClipbar();
  renderLock();
  renderChat();
  renderComposer();
  renderUpload();
  renderExport();
  renderSheet();
  // The timeline places itself; without it the time still follows the project.
  if ($("tl").hidden) place();
  animate();
}

function renderTransport(hasClips) {
  const why = editReason();
  for (const [id, can] of [
    ["undo", state?.canUndo],
    ["redo", state?.canRedo],
  ]) {
    gate($(id), why);
    $(id).disabled = !why && !can;
  }
  const play = $("play");
  gate(play, offReason());
  play.disabled = !offReason() && !hasClips;
  play.classList.toggle("playing", !!state?.playing);
  play.setAttribute("aria-label", state?.playing ? "Pause" : "Play");
}

// ---- Timeline ----

const HEAD = 36;
const GAP = 4;
const laneHeight = (t) => (t.main ? 56 : t.kind === "video" ? 44 : 40);
/** Pixels per second, so the whole video is about four screens wide. */
let pps = 30;
let laneTops = [];
let headsKey = "";
let ticksKey = "";
/** Every clip's element, by clip id. One is never recreated or taken out and put back while its clip exists: either
 * fetches its thumbnail again (no-store), and until it arrives the clip shows its bare colour, which blinked on every
 * state the editor published. */
const clipEls = new Map();

function renderTimeline(shown) {
  const tl = $("tl");
  tl.hidden = !shown;
  if (!shown) return;
  const tracks = state?.tracks ?? [];
  const sec = Math.max(1, (state?.durationUs ?? 0) / US);
  pps = Math.min(64, Math.max(8, ((tl.clientWidth - HEAD) * 4) / sec));
  let y = GAP;
  laneTops = tracks.map((t) => {
    const top = y;
    y += laneHeight(t) + GAP;
    return top;
  });
  $("tl-body").style.height = `${y}px`;

  const key = tracks.map((t) => `${t.kind}${t.captions}${t.main}`).join();
  if (key !== headsKey) {
    headsKey = key;
    $("heads").replaceChildren(
      ...tracks.map((t, i) => {
        const tile = el("div", `ti ${t.captions ? "captions" : t.kind}`);
        tile.append(icon(t.captions ? "cc" : ({ text: "type", audio: "music" }[t.kind] ?? "film")));
        tile.style.top = `${laneTops[i] + laneHeight(t) / 2 - 12}px`;
        return tile;
      }),
    );
  }

  const seen = new Set();
  tracks.forEach((t, i) => {
    for (const c of t.clips) {
      seen.add(c.id);
      let d = clipEls.get(c.id);
      if (!d) {
        d = el("div");
        d.dataset.id = c.id;
        clipEls.set(c.id, d);
        $("tracks").append(d);
      }
      drawClip(d, c, t, laneTops[i]);
    }
  });
  for (const [id, d] of clipEls)
    if (!seen.has(id)) {
      d.remove();
      clipEls.delete(id);
    }

  for (const d of $("tracks").querySelectorAll(".pending")) d.remove();
  const main = Math.max(0, tracks.findIndex((t) => t.main));
  const waiting = [...uploads.filter((u) => u.status !== "done"), ...(state?.adding ?? []).map((name) => ({ name, sent: 1, size: 1 }))];
  waiting.forEach((u, i) => {
    const d = el("div", u.status === "failed" ? "clip pending failed" : "clip pending");
    d.title = u.name;
    if (u.status === "failed") d.dataset.upload = String(uploads.indexOf(u));
    d.style.left = `${(state?.durationUs ?? 0) / US * pps + 2 + i * 66}px`;
    d.style.top = `${laneTops[main] ?? GAP}px`;
    d.style.width = "64px";
    d.style.height = "56px";
    const bar = el("div", "bar");
    const fill = el("i");
    fill.style.width = `${Math.round((u.sent / (u.size || 1)) * 100)}%`;
    bar.append(fill);
    d.append(bar);
    $("tracks").append(d);
  });

  if (`${sec}:${pps}` !== ticksKey) {
    ticksKey = `${sec}:${pps}`;
    const label = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600].find((s) => s * pps >= 56) ?? 600;
    const step = label >= 5 ? label / 5 : 1;
    const ticks = [];
    for (let s = 0; s <= sec; s += step) {
      const tick = el("div", s % label ? "tick" : "tick l");
      tick.style.left = `${s * pps}px`;
      ticks.push(tick);
      if (s % label === 0) {
        const text = el("div", "tlab", clock(s * US));
        text.style.left = `${s * pps}px`;
        ticks.push(text);
      }
    }
    $("ticks").replaceChildren(...ticks);
  }
  place();
}

function drawClip(d, c, t, top) {
  const caption = c.kind === "text" && t.captions;
  const kind = caption ? "caption" : c.kind === "text" ? "title" : c.kind;
  const inset = c.kind === "text" ? 5 : 0;
  const w = Math.max(2, (c.durationUs / US) * pps - 2);
  d.x = (c.startUs / US) * pps + 1;
  d.w = w;
  d.style.left = `${d.x}px`;
  d.style.top = `${top + inset}px`;
  d.style.width = `${w}px`;
  d.style.height = `${laneHeight(t) - 2 * inset}px`;
  const thumb = c.thumb && (c.kind === "video" || c.kind === "image") ? `url("${c.thumb}")` : "";
  if (d.dataset.thumb !== thumb) {
    d.dataset.thumb = thumb;
    d.style.backgroundImage = thumb;
  }
  // Names show only whole (DESIGN.md): a clip too short for its name shows its length, a shorter one nothing, and a
  // caption too short for its first word a dot.
  const fits = (text, extra) => textWidth(text) + extra <= w;
  let label = caption ? c.label : fits(c.label, 30) ? c.label : w >= 56 ? `${(c.durationUs / US).toFixed(1)} s` : "";
  if (caption && !fits(c.label.split(/\s+/)[0], 8)) label = "";
  d.className = `clip ${kind}${caption && !label ? " short" : ""}${c.id === selected ? " sel" : ""}`;
  const key = `${kind}:${label}`;
  if (d.dataset.label === key) return;
  d.dataset.label = key;
  if (caption || !label) d.replaceChildren(label);
  else {
    const chip = el("div", "chip");
    chip.append(icon({ video: "film", image: "image", audio: "music", title: "type" }[kind]), label);
    d.replaceChildren(chip);
  }
  d.chip = d.querySelector(".chip");
}

const measure = document.createElement("canvas").getContext("2d");
const widths = new Map();
function textWidth(text) {
  if (!widths.has(text)) {
    measure.font = `500 11px ${getComputedStyle(document.body).fontFamily}`;
    widths.set(text, measure.measureText(text).width);
  }
  return widths.get(text);
}

/** Where the playhead shows: where the user drags it, or where the computer's is, moved on while it plays. */
function shownTime() {
  if (!state) return 0;
  if (seeking !== null) return seeking;
  if (!state.playing) return state.timeUs;
  return Math.min(state.durationUs, state.timeUs + (performance.now() - stateAt) * 1000);
}

function place() {
  const t = shownTime();
  $("time-now").textContent = tenths(t);
  $("time-total").textContent = tenths(state?.durationUs ?? 0);
  const tl = $("tl");
  if (tl.hidden) return;
  const middle = (tl.clientWidth - HEAD) / 2;
  const x = `translateX(${middle - (t / US) * pps}px)`;
  $("tracks").style.transform = x;
  $("ticks").style.transform = x;
  // A long clip, such as the music under the whole video, keeps its name in view while it passes under the playhead.
  const left = (t / US) * pps - middle;
  for (const d of clipEls.values())
    if (d.chip) d.chip.style.transform = d.x < left && d.w > 160 ? `translateX(${Math.min(left - d.x, d.w - 140)}px)` : "";
  $("ph").style.left = `${HEAD + middle}px`;
  tl.setAttribute("aria-valuemax", String(Math.round((state?.durationUs ?? 0) / US)));
  tl.setAttribute("aria-valuenow", String(Math.round(t / US)));
  tl.setAttribute("aria-valuetext", `${clock(t)} of ${clock(state?.durationUs ?? 0)}`);
}

let animating = false;
function animate() {
  if (animating || !state?.playing || !online) return;
  animating = true;
  const step = () => {
    if (!state?.playing || !online || closed) return void (animating = false);
    place();
    requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

let seeking = null;
let seekTimer = 0;
let seekEnd = 0;
function seekTo(us) {
  if (!state?.durationUs || offReason()) return;
  clearTimeout(seekEnd);
  seeking = Math.round(Math.min(state.durationUs, Math.max(0, us)));
  place();
  if (seekTimer) return;
  seekTimer = setTimeout(() => {
    seekTimer = 0;
    if (seeking !== null) void post({ action: "seek", us: seeking });
  }, 120);
}

/** The page's playhead gives way to the computer's once the computer has had time to answer. */
function releaseSeek() {
  seekEnd = setTimeout(() => {
    seeking = null;
    render();
  }, 400);
}

function findClip(id) {
  for (const t of state?.tracks ?? []) for (const c of t.clips) if (c.id === id) return c;
  return null;
}

function tapTimeline(e) {
  const failed = e.target.closest(".clip.failed");
  if (failed) return retry(uploads[Number(failed.dataset.upload)]);
  const lanes = $("lanes").getBoundingClientRect();
  const x = e.clientX - lanes.left;
  if (x < 0) return;
  const t = shownTime() + ((x - lanes.width / 2) / pps) * US;
  if (!e.target.closest(".ruler")) {
    const y = e.clientY - lanes.top;
    const tracks = state.tracks;
    const i = laneTops.findIndex((top, i) => y >= top - GAP / 2 && y < top + laneHeight(tracks[i]) + GAP / 2);
    // A short clip is hard to hit with a finger; a tap just beside it still picks it.
    let best = null;
    let bestOff = (22 / pps) * US;
    for (const c of tracks[i]?.clips ?? []) {
      const off = Math.max(0, c.startUs - t, t - c.startUs - c.durationUs);
      if (off <= bestOff) [best, bestOff] = [c, off];
    }
    if (best) return select(best.id);
    if (selected) return select(null);
  }
  selected = null;
  seekTo(t);
  releaseSeek();
}

function select(id) {
  selected = id;
  render();
}

function setupTimeline() {
  const tl = $("tl");
  let start = null;
  tl.addEventListener("pointerdown", (e) => {
    if (state && e.button <= 0) start = { id: e.pointerId, x: e.clientX, t: shownTime(), moved: false };
  });
  tl.addEventListener("pointermove", (e) => {
    if (start?.id !== e.pointerId) return;
    const dx = e.clientX - start.x;
    if (!start.moved && Math.abs(dx) < 8) return;
    if (!start.moved) tl.setPointerCapture(e.pointerId);
    start.moved = true;
    seekTo(start.t - (dx / pps) * US);
  });
  tl.addEventListener("pointerup", (e) => {
    if (start?.id !== e.pointerId) return;
    const tap = !start.moved;
    start = null;
    if (tap) tapTimeline(e);
    else releaseSeek();
  });
  tl.addEventListener("pointercancel", () => {
    if (start?.moved) releaseSeek();
    start = null;
  });
  tl.addEventListener("keydown", (e) => {
    if (!state) return;
    if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      e.preventDefault();
      seekTo(shownTime() + (e.key === "ArrowLeft" ? -US : US));
      releaseSeek();
    } else if (e.key === "Enter") {
      const t = shownTime();
      const c = state.tracks.find((tr) => tr.main)?.clips.find((c) => t >= c.startUs && t < c.startUs + c.durationUs);
      if (c) select(c.id);
    }
  });
}

function renderClipbar() {
  // The clip's actions would take the place of the AI's bar and its Stop.
  const c = selected && !state?.working && !state?.lock && findClip(selected);
  if (!c) selected = null;
  $("app").classList.toggle("selecting", !!c);
  $("clipbar").hidden = !c;
  if (!c) return;
  $("clip-name").replaceChildren(el("b", "", c.label), ` · ${tenths(c.startUs)} – ${tenths(c.startUs + c.durationUs)}`);
  for (const b of document.querySelectorAll(".act")) gate(b, editReason() ?? (c.can[b.dataset.act] ? null : c.why[b.dataset.act]));
}

// ---- The AI ----

function runningStep() {
  for (const it of [...(state?.items ?? [])].reverse()) {
    if (it.kind !== "steps") continue;
    const step = [...it.steps].reverse().find((s) => s.status === "running");
    if (step) return step.title;
  }
  return "";
}

function renderLock() {
  const s = state;
  const working = !!s && (s.working || !!s.lock) && !closed;
  $("lock").hidden = !working;
  $("ai-row").hidden = working;
  if (!working) return;
  $("lock-text").textContent = s.stopping ? "Stopping…" : "AI is editing";
  $("lock-step").textContent = s.working ? runningStep() || s.lock || "" : `${s.lock} · Stop it on the computer`;
  // Only the panel's own agent stops from here; another agent's run is stopped on the computer.
  $("stop").hidden = !s.working;
  $("stop").disabled = s.stopping;
}

function stepIcon(status) {
  if (status === "running") return el("span", "spin");
  return icon(status === "failed" ? "x" : "check");
}

function lastLine(items) {
  const it = items[items.length - 1];
  if (!it) return null;
  if (it.kind === "steps" && it.steps.length) return { text: it.steps[it.steps.length - 1].title, error: false };
  if (it.kind === "options") return { text: it.question || "Choose an option", error: false };
  if (it.kind === "run") return { text: it.label, error: false };
  return { text: it.text ?? "", error: it.kind === "error" };
}

function openChat() {
  $("chat").classList.add("open");
  selected = null;
  render();
  $("items").scrollTop = $("items").scrollHeight;
}

/** Options the agent offered that this page has seen, so a new question opens the conversation once. */
const offered = new Set();
let renderedItems = "";
/** The reader is at the newest message, so new ones scroll into view. Kept by scrolling, as the lock bar or a longer
 * message in the field shrinks the list in the same render that adds a message. */
let atEnd = true;
function renderChat() {
  const s = state;
  const items = s?.items ?? [];
  const model = s?.model && s.models.find((m) => m.slug === s.model)?.name;
  const chipKey = `${s?.agent}:${model}`;
  for (const b of document.querySelectorAll(".agentchip")) {
    b.disabled = closed;
    if (b.dataset.key === chipKey) continue;
    b.dataset.key = chipKey;
    b.replaceChildren(s?.agent ?? "AI", ...(model ? [el("span", "m", `· ${model}`)] : []), icon("down"));
  }
  gate($("new-chat"), agentBusy());
  for (const it of items) {
    if (it.kind !== "options" || offered.has(it.id)) continue;
    offered.add(it.id);
    // A question waits for an answer; on a phone it would hide in the one line under the timeline.
    if (it.open && !$("chat").classList.contains("open")) {
      $("chat").classList.add("open");
      selected = null;
      renderClipbar();
    }
  }
  const line = lastLine(items);
  $("last").hidden = !line?.text;
  $("last").classList.toggle("error", !!line?.error);
  $("last-text").textContent = line?.text ?? "";
  const key = JSON.stringify(items);
  if (key === renderedItems) return;
  renderedItems = key;
  const list = $("items");
  list.replaceChildren();
  if (!items.length) list.append(el("p", "empty", `Tell ${s?.agent ?? "the AI"} how to cut your video.`));
  for (const it of items) list.append(item(it));
  if (atEnd) list.scrollTop = list.scrollHeight;
}

function item(it) {
  switch (it.kind) {
    case "user":
      return el("div", "user", it.text);
    case "agent":
      return el("div", "agent-text", it.text.replace(/\*\*(.+?)\*\*/g, "$1").replace(/`([^`]+)`/g, "$1"));
    case "steps": {
      const box = el("div", "steps");
      for (const s of it.steps) {
        const row = el("div", `step ${s.status}`);
        row.append(stepIcon(s.status), el("span", "", s.title));
        box.append(row);
      }
      return box;
    }
    case "options": {
      const box = el("div", "options");
      if (it.question) box.append(el("div", "", it.question));
      const choices = el("div", "choices");
      for (const label of it.options) {
        const b = el("button", "pill small", label);
        b.type = "button";
        b.disabled = !it.open || closed;
        if (it.chosen === label) b.classList.add("chosen");
        b.addEventListener("click", () => void post({ action: "choose", id: it.id, label }));
        choices.append(b);
      }
      box.append(choices);
      return box;
    }
    case "run": {
      const box = el("div", "run");
      box.append(el("b", "", it.undone ? `Undone: ${it.label}` : it.stopped ? `Stopped: ${it.label}` : it.label));
      if (it.changes.length) {
        const ul = el("ul");
        for (const c of it.changes.slice(0, 8)) ul.append(el("li", "", c));
        box.append(ul);
      }
      return box;
    }
    case "error":
      return el("div", "error", it.text);
    default:
      return el("div", "muted", it.text);
  }
}

function renderComposer() {
  const text = $("text").value.trim();
  const reason = closed ? null : state?.sendBlocked;
  // One message waits for the videos at a time; another stays in the field until it has gone.
  $("send").disabled = !text || closed || !online || !state || !!reason || !!held;
  const note = held ? "It goes to the AI once your videos are on the timeline." : text && reason ? `${reason}.` : "";
  $("note").textContent = note;
  $("note").hidden = !note;
  $("text").disabled = closed;
  $("text").placeholder = state?.working ? `Tell ${state.agent} more…` : "Tell the AI how to cut it…";
}

/** Files on their way, or the first that did not arrive, in the line above the field. */
function renderUpload() {
  const failed = uploads.find((u) => u.status === "failed");
  const now = uploads.find((u) => u.status === "sending");
  const left = uploads.filter((u) => u.status === "waiting" || u.status === "sending").length;
  const text = failed
    ? `${failed.name} didn't arrive. Tap here to send it again.`
    : now
      ? `Sending ${now.name} · ${Math.round((now.sent / (now.size || 1)) * 100)} %${left > 1 ? ` · ${left - 1} more` : ""}. Keep this page open.`
      : "";
  $("upload").hidden = !text;
  $("upload-drop").hidden = !failed;
  $("upload").classList.toggle("error", !!failed);
  $("upload-text").textContent = text;
}

// ---- Sheets ----

function openSheet(name) {
  sheet = name;
  sheetKey = "";
  if (name === "chats") void post({ action: "chats" });
  renderSheet();
}

function closeSheet() {
  sheet = null;
  renderSheet();
}

let sheetKey = "";
function renderSheet() {
  $("scrim").hidden = !sheet;
  $("agents").hidden = sheet !== "agents" && sheet !== "chats";
  $("projects-sheet").hidden = sheet !== "projects";
  if (!sheet || !state) return;
  const s = state;
  const key = JSON.stringify([sheet, online, closed, s.agent, s.agents, s.agentId, s.models, s.model, s.ownDefault, s.modelsLoading, s.efforts, s.effort, s.chats, s.chatId, s.working, s.projects, s.editBlocked]);
  if (key === sheetKey) return;
  sheetKey = key;
  if (sheet === "projects") $("projects-sheet").replaceChildren(...projectsSheet());
  else $("agents").replaceChildren(...(sheet === "chats" ? chatsSheet() : agentsSheet()));
}

function sheetHead(title, back) {
  const head = el("div", "sheet-head");
  if (back) {
    const b = el("button", "round bare");
    b.type = "button";
    b.setAttribute("aria-label", "Back");
    b.append(icon("left"));
    b.addEventListener("click", back);
    head.append(b);
  }
  const done = el("button", "pill", "Done");
  done.type = "button";
  done.addEventListener("click", closeSheet);
  head.append(el("h2", "", title), done);
  return head;
}

/** A row of a list: a check or an icon, a title with an optional second line, and a note or a chevron at the end. */
function row({ check, lead, title, sub, meta, why, then, chevron }) {
  const b = el("button", "li");
  b.type = "button";
  if (check !== undefined || lead) {
    const ck = el("span", lead ? "ck lead" : "ck");
    if (check || lead) ck.append(icon(lead ?? "check"));
    b.append(ck);
  }
  const text = el("span", "lt");
  text.append(el("div", "", title));
  if (sub) text.append(sub);
  b.append(text);
  if (meta) b.append(el("span", "meta", meta));
  if (chevron) b.append(icon("right"));
  gate(b, why);
  if (then) b.addEventListener("click", then);
  return b;
}

function list(rows) {
  const box = el("div", "list");
  box.append(...rows);
  return box;
}

function agentsSheet() {
  const s = state;
  const busy = agentBusy();
  const out = [sheetHead("AI agent")];
  if (s.working) out.push(el("p", "hint", "You can change these once the AI is done."));
  out.push(
    list(
      s.agents.map((a) =>
        row({ check: a.id === s.agentId, title: a.name, meta: a.note, why: busy ?? (a.ready ? null : `${a.name}: ${a.note}`), then: () => void post({ action: "agent", agent: a.id }) }),
      ),
    ),
  );
  if (s.ownDefault || s.models.length || s.modelsLoading) {
    const rows = [];
    if (s.ownDefault) rows.push(row({ check: s.model === null, title: "Default", meta: s.ownDefault, why: busy, then: () => void post({ action: "model", slug: null }) }));
    for (const m of s.models) rows.push(row({ check: m.slug === s.model, title: m.name, why: busy, then: () => void post({ action: "model", slug: m.slug }) }));
    if (s.modelsLoading) rows.push(el("div", "li quiet", "Loading models…"));
    out.push(el("div", "sec", "Model"), list(rows));
  }
  if (s.efforts.length) {
    const seg = el("div", "seg");
    for (const e of s.efforts) {
      const b = el("button", e.id === s.effort ? "on" : "", e.short);
      b.type = "button";
      gate(b, busy);
      b.addEventListener("click", () => void post({ action: "effort", effort: e.id }));
      seg.append(b);
    }
    out.push(el("div", "sec", "Effort"), seg, el("p", "hint", s.efforts.find((e) => e.id === s.effort)?.hint ?? ""));
  }
  const newChat = () => {
    void post({ action: "newChat" });
    closeSheet();
  };
  out.push(
    el("div", "sec", "Chats"),
    list([row({ lead: "chats", title: "Past chats", chevron: true, then: () => openSheet("chats") }), row({ lead: "newchat", title: "New chat", why: busy, then: newChat })]),
  );
  return out;
}

function chatsSheet() {
  const s = state;
  const busy = agentBusy();
  const rows = (s.chats ?? []).map((c) =>
    row({
      check: c.id === s.chatId,
      title: c.title,
      sub: el("div", "", `${c.agent} · ${c.when}`),
      why: busy,
      then: () => {
        void post({ action: "openChat", id: c.id });
        closeSheet();
      },
    }),
  );
  if (!s.chats) rows.push(el("div", "li quiet", "Loading…"));
  else if (!rows.length) rows.push(el("div", "li quiet", "No past chats in this project yet."));
  return [sheetHead("Past chats", () => openSheet("agents")), list(rows)];
}

const FORMATS = [
  ["9:16", 18, 30],
  ["16:9", 32, 18],
  ["1:1", 24, 24],
  ["4:5", 22, 27],
];

function projectsSheet() {
  const s = state;
  const why = editReason();
  const formats = el("div", "formats");
  for (const [format, w, h] of FORMATS) {
    const b = el("button", "fmt");
    b.type = "button";
    b.setAttribute("aria-label", `New ${format} project`);
    const shape = el("i");
    shape.style.width = `${w}px`;
    shape.style.height = `${h}px`;
    b.append(shape, format);
    gate(b, why);
    b.addEventListener("click", () => {
      void post({ action: "newProject", format });
      closeSheet();
    });
    formats.append(b);
  }
  const rows = s.projects.map((p) => {
    const sub = el("div");
    if (p.state === "busy") sub.textContent = "In use elsewhere";
    else if (p.state === "broken") sub.textContent = "Damaged file";
    else sub.append(`${clock(p.durationUs)} · `, el("span", p.state === "open" ? "now" : "", p.when));
    const refused = { busy: `${p.name} is open in another Nuzky window or by an agent`, broken: `${p.name} can't be opened: its file is damaged`, open: null }[p.state];
    return row({
      title: p.name,
      sub,
      why: refused === undefined ? why : refused,
      then: () => {
        if (p.state !== "open") void post({ action: "openProject", id: p.id });
        closeSheet();
      },
    });
  });
  return [sheetHead("Projects"), el("div", "sec", "New project"), formats, el("div", "sec", "Recent"), list(rows)];
}

// ---- Export ----

function size(bytes) {
  if (!bytes) return "";
  return bytes >= 1e9 ? ` · ${(bytes / 1e9).toFixed(1)} GB` : ` · ${Math.max(1, Math.round(bytes / 1e6))} MB`;
}

function exportPhase(s) {
  return !s ? null : s.exporting !== null ? "running" : s.exported ? "done" : s.exportError ? "failed" : null;
}

function renderExport() {
  const s = state;
  const exportButton = $("export");
  const running = s?.exporting !== null && s?.exporting !== undefined;
  exportButton.textContent = running ? `Exporting ${Math.round(s.exporting * 100)} %` : "Export";
  const phase = exportPhase(s);
  // A finished export reopens even while a new one could not start.
  gate(exportButton, running || phase === "done" ? null : (offReason() ?? s.exportBlocked));
  gate($("export-again"), offReason() ?? s.exportBlocked);
  const sheet = $("sheet");
  if (!phase || dismissed === `${s.exportId}:${phase}`) {
    sheet.hidden = true;
    return;
  }
  sheet.hidden = false;
  $("sheet-title").textContent = phase === "running" ? `Exporting ${Math.round(s.exporting * 100)} %` : phase === "done" ? "Exported" : "The export didn't finish";
  $("sheet-progress").hidden = phase !== "running";
  $("sheet-progress").firstElementChild.style.width = `${Math.round((s.exporting ?? 0) * 100)}%`;
  $("sheet-error").hidden = phase !== "failed";
  $("sheet-error").textContent = s.exportError ?? "";
  $("sheet-done").hidden = phase !== "done";
  if (phase === "done") {
    const e = s.exported;
    $("sheet-name").textContent = `${s.project || "Video"}.mp4`;
    $("sheet-info").textContent = `${e.width} × ${e.height} · ${clock(e.durationUs)}${size(e.bytes)}`;
    if (!$("sheet-thumb").src) $("sheet-thumb").src = $("frame").src || "";
  }
}

// ---- Adding files ----

let sending = false;
function pickFiles() {
  if (!closed) $("files").click();
}

/** What Nuzky takes, as MEDIA_EXTENSIONS in remote.rs, so a refused file says why before it is sent. */
const MEDIA = /\.(mp4|mov|m4v|mkv|webm|avi|mts|mp3|wav|m4a|aac|flac|ogg|opus|png|jpe?g|heic|heif|webp|gif|bmp)$/i;

function addFiles(files) {
  const refused = files.filter((f) => !MEDIA.test(f.name));
  if (refused.length) toast(`${refused.map((f) => f.name).join(", ")}: Nuzky takes videos, photos and sound only`);
  for (const file of files.filter((f) => MEDIA.test(f.name))) uploads.push({ file, name: file.name, size: file.size, sent: 0, status: "waiting" });
  render();
  void sendFiles();
}

function retry(u) {
  u.status = "waiting";
  u.sent = 0;
  render();
  void sendFiles();
}

async function sendFiles() {
  if (sending) return;
  sending = true;
  for (let u = uploads.find((x) => x.status === "waiting"); u; u = uploads.find((x) => x.status === "waiting")) {
    u.status = "sending";
    const result = await upload(u);
    // A file the computer refused would be refused again, so it goes; one the network lost can be sent again.
    if (result === "refused") uploads.splice(uploads.indexOf(u), 1);
    else u.status = result;
    if (result === "done") lastArrival = Date.now();
    else if (held) held.clips--;
    render();
  }
  sending = false;
  sendHeld();
}

function upload(u) {
  return new Promise((resolve) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", `/upload?name=${encodeURIComponent(u.name)}`);
    xhr.setRequestHeader("X-Nuzky", "1");
    let drawn = 0;
    xhr.upload.addEventListener("progress", (e) => {
      u.sent = e.loaded;
      // A redraw per percent is enough.
      if (e.loaded - drawn > u.size / 100) {
        drawn = e.loaded;
        renderTimeline(true);
        renderUpload();
      }
    });
    xhr.addEventListener("load", () => {
      if (xhr.status === 403) expired();
      else if (xhr.status !== 200) toast(xhr.responseText || `${u.name} could not be sent.`);
      resolve(xhr.status === 200 ? "done" : "refused");
    });
    xhr.addEventListener("error", () => resolve("failed"));
    xhr.send(u.file);
  });
}

// ---- Talking to the AI ----

const onTheWay = () => uploads.filter((u) => u.status === "waiting" || u.status === "sending").length;

/** Sends a message; when it does not reach the computer, it goes back into the field. */
async function deliver(text) {
  if (await post({ action: "send", text })) return;
  const field = $("text");
  field.value = field.value.trim() ? `${text}\n${field.value}` : text;
  fitText();
  renderComposer();
  toast("The message didn't reach Nuzky. Check the Wi-Fi and send it again.", 6000);
}

function sendText() {
  const text = $("text").value.trim();
  if (!text || $("send").disabled) return;
  $("text").value = "";
  fitText();
  // The computer adds a file a moment after it arrives, so the message waits for its clip, not only for the upload.
  if (sending || onTheWay() || state?.adding.length) held = { text, clips: state.clips.length + state.adding.length + onTheWay() };
  else void deliver(text);
  render();
}

function sendHeld() {
  if (!held || sending || onTheWay() || !state || state.adding.length || state.sendBlocked || !online) return;
  // A file the computer could not import never becomes a clip; the message goes without it after a few seconds.
  if (state.clips.length < held.clips && Date.now() - lastArrival < 8000) {
    heldTimer ||= setTimeout(() => {
      heldTimer = 0;
      sendHeld();
    }, 1000);
    return;
  }
  const { text } = held;
  held = null;
  void deliver(text);
  renderComposer();
}

function fitText() {
  const t = $("text");
  t.style.height = "auto";
  t.style.height = `${Math.min(140, t.scrollHeight)}px`;
}

function setup() {
  // Runs before a control's own handler, so a control disabled with a reason only says why.
  document.addEventListener(
    "click",
    (e) => {
      const b = e.target.closest('[aria-disabled="true"]');
      if (!b) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      if (b.dataset.why) toast(`${b.dataset.why}.`);
    },
    true,
  );
  $("add").addEventListener("click", pickFiles);
  $("add-first").addEventListener("click", pickFiles);
  $("files").addEventListener("change", (e) => {
    addFiles([...e.target.files]);
    e.target.value = "";
  });
  $("text").addEventListener("input", () => {
    fitText();
    renderComposer();
  });
  $("text").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      sendText();
    }
  });
  $("send").addEventListener("click", sendText);
  $("stop").addEventListener("click", () => void post({ action: "stop" }));
  $("undo").addEventListener("click", () => void post({ action: "undo" }));
  $("redo").addEventListener("click", () => void post({ action: "redo" }));
  $("play").addEventListener("click", () => void post({ action: "play" }));
  for (const b of document.querySelectorAll(".act")) b.addEventListener("click", () => void post({ action: b.dataset.act, clipId: selected }));
  $("clip-done").addEventListener("click", () => select(null));
  $("items").addEventListener("scroll", (e) => (atEnd = e.target.scrollHeight - e.target.scrollTop - e.target.clientHeight < 40));
  $("last").addEventListener("click", openChat);
  $("lock-open").addEventListener("click", openChat);
  $("upload").addEventListener("click", () => {
    const failed = uploads.find((u) => u.status === "failed");
    if (failed) retry(failed);
  });
  $("upload-drop").addEventListener("click", () => {
    uploads.splice(0, uploads.length, ...uploads.filter((u) => u.status !== "failed"));
    render();
  });
  $("chat-close").addEventListener("click", () => {
    $("chat").classList.remove("open");
    render();
  });
  for (const b of document.querySelectorAll(".agentchip")) b.addEventListener("click", () => openSheet("agents"));
  $("past").addEventListener("click", () => openSheet("chats"));
  $("new-chat").addEventListener("click", () => void post({ action: "newChat" }));
  $("projects").addEventListener("click", () => openSheet("projects"));
  $("title").addEventListener("click", () => openSheet("projects"));
  $("scrim").addEventListener("click", closeSheet);
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape") return;
    if (sheet) closeSheet();
    else if (!$("sheet").hidden) $("sheet-close").click();
    else if ($("chat").classList.contains("open")) $("chat-close").click();
    else if (selected) select(null);
  });
  // A finished export opens again, so closing its sheet by mistake does not mean exporting once more.
  $("export").addEventListener("click", () => {
    const shownBefore = state.exporting !== null || (exportPhase(state) === "done" && $("sheet").hidden);
    dismissed = null;
    if (shownBefore) return renderExport();
    startExport();
  });
  $("export-again").addEventListener("click", startExport);
  function startExport() {
    dismissed = null;
    $("video").hidden = true;
    $("video").removeAttribute("src");
    $("sheet-thumb").removeAttribute("src");
    void post({ action: "export" });
  }
  $("sheet-close").addEventListener("click", () => {
    dismissed = `${state?.exportId}:${exportPhase(state)}`;
    $("video").pause();
    renderExport();
  });
  $("play-export").addEventListener("click", () => {
    const v = $("video");
    v.hidden = false;
    v.src = `/video?id=${encodeURIComponent(state?.exportId ?? "")}`;
    void v.play().catch(() => {});
  });
  // A computer downloads; only a phone saves.
  if (!PHONE) $("save-text").textContent = "Download";
  // Safari puts a download in Files; from there Share, Save Video puts it into Photos.
  if (IOS) $("save").addEventListener("click", () => toast("It goes to Files › Downloads. Open it there, tap Share, then Save Video.", 9000));
  const viewport = window.visualViewport;
  if (viewport) {
    // iOS keeps the layout the same height when the keyboard opens; the page follows what is visible instead.
    const fit = () => {
      document.documentElement.style.setProperty("--vh", `${viewport.height}px`);
      document.documentElement.style.setProperty("--kb", `${Math.max(0, window.innerHeight - viewport.height - viewport.offsetTop)}px`);
    };
    viewport.addEventListener("resize", fit);
    viewport.addEventListener("scroll", fit);
    fit();
  }
  // Over the open conversation the dock is fixed to the bottom; the conversation leaves it that much room.
  new ResizeObserver(() => document.documentElement.style.setProperty("--dock", `${$("dock").offsetHeight}px`)).observe($("dock"));
  window.addEventListener("resize", render);
  setupTimeline();
  connect();
  render();
}

setup();

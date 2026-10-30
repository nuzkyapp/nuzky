// The page a phone or another computer opens to work on the project. Nuzky on the computer does the work; this page
// sends what the user asks for (src-tauri/src/remote.rs) and shows the state the editor publishes (src/lib/remote.ts).
"use strict";

const $ = (id) => document.getElementById(id);
const US = 1_000_000;
const ICONS = {
  check: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg>',
  x: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12"/></svg>',
};

/** What the editor last published; null until the first state arrives. */
let state = null;
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

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function setConn(kind, text) {
  const c = $("conn");
  c.className = `conn ${kind}`;
  c.lastElementChild.textContent = text;
}

function expired() {
  closed = true;
  setConn("gone", "This link no longer works. Scan the code in Nuzky again.");
  render();
}

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
    setConn("off", "Reconnecting…");
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

setInterval(() => {
  if (!online && !closed && Date.now() - offlineSince > 10_000) setConn("off", "Can't reach Nuzky. Check the Wi-Fi, or scan the code again.");
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
  $("add").disabled = closed;
  $("add-first").disabled = closed;
  renderStrip(hasClips || busyAdding);
  renderLock();
  renderChat();
  renderComposer();
  renderExport();
}

function renderStrip(shown) {
  const strip = $("strip");
  strip.hidden = !shown;
  if (!shown) return;
  const clips = $("clips");
  clips.replaceChildren();
  const total = state?.durationUs || 1;
  for (const c of state?.clips ?? []) {
    const d = el("div", `clip ${c.kind}`);
    d.style.flex = `${c.durationUs / total} 1 0`;
    if (c.assetId && c.kind !== "audio") d.style.backgroundImage = `url("/thumb/${encodeURIComponent(c.assetId)}")`;
    clips.append(d);
  }
  const waiting = [...uploads.filter((u) => u.status !== "done"), ...(state?.adding ?? []).map((name) => ({ name, sent: 1, size: 1 }))];
  for (const u of waiting) {
    const d = el("div", u.status === "failed" ? "clip pending failed" : "clip pending");
    d.title = u.name;
    if (u.status === "failed") d.dataset.upload = String(uploads.indexOf(u));
    const bar = el("div", "bar");
    const fill = el("i");
    fill.style.width = `${Math.round((u.sent / (u.size || 1)) * 100)}%`;
    bar.append(fill);
    d.append(bar);
    clips.append(d);
  }
  const at = state && state.durationUs > 0 ? Math.min(1, (seeking ?? state.timeUs) / state.durationUs) : 0;
  $("head").hidden = !state?.clips.length;
  // The head sits within the clips, which leave the pending ones beside them.
  const clipsWidth = clips.scrollWidth - waiting.length * 66;
  $("head").style.left = `${at * Math.max(0, clipsWidth)}px`;
  strip.setAttribute("aria-valuemax", String(Math.round((state?.durationUs ?? 0) / US)));
  strip.setAttribute("aria-valuenow", String(Math.round((state?.timeUs ?? 0) / US)));
  strip.setAttribute("aria-valuetext", `${clock(state?.timeUs ?? 0)} of ${clock(state?.durationUs ?? 0)}`);
}

function runningStep() {
  for (const it of [...(state?.items ?? [])].reverse()) {
    if (it.kind !== "steps") continue;
    const step = [...it.steps].reverse().find((s) => s.status === "running");
    if (step) return step.title;
  }
  return "";
}

/** Files on their way, or the first that did not arrive, for the line above the field. */
function uploadLine() {
  const failed = uploads.find((u) => u.status === "failed");
  if (failed) return { text: `${failed.name} didn't arrive. Tap it on the strip to send it again.`, error: true };
  const now = uploads.find((u) => u.status === "sending");
  if (!now) return null;
  const left = uploads.filter((u) => u.status === "waiting" || u.status === "sending").length;
  const pct = Math.round((now.sent / (now.size || 1)) * 100);
  return { text: `Sending ${now.name} · ${pct} %${left > 1 ? ` · ${left - 1} more` : ""}. Keep this page open.`, error: false };
}

function renderLock() {
  const s = state;
  const working = !!s && (s.working || !!s.lock);
  $("lock").hidden = !working || closed;
  if (!working) return;
  $("lock-text").textContent = s.stopping ? "Stopping…" : "AI is editing";
  $("lock-step").textContent = runningStep();
  // Only the panel's own agent stops from here; another agent's run is stopped on the computer.
  $("stop").hidden = !s.working;
  $("stop").disabled = s.stopping;
}

function stepIcon(status) {
  if (status === "running") return el("span", "spin");
  const icon = el("span");
  icon.innerHTML = status === "failed" ? ICONS.x : ICONS.check;
  return icon.firstChild;
}

function lastLine(items) {
  const it = items[items.length - 1];
  if (!it) return null;
  if (it.kind === "steps" && it.steps.length) return { text: it.steps[it.steps.length - 1].title, error: false };
  if (it.kind === "options") return { text: it.question || "Choose an option", error: false };
  if (it.kind === "run") return { text: it.label, error: false };
  return { text: it.text ?? "", error: it.kind === "error" };
}

function renderLast() {
  const line = uploadLine() ?? lastLine(state?.items ?? []);
  $("last").hidden = !line?.text || $("chat").classList.contains("open");
  $("last").classList.toggle("error", !!line?.error);
  $("last-text").textContent = line?.text ?? "";
}

/** Options the agent offered that this page has seen, so a new question opens the conversation once. */
const offered = new Set();
let renderedItems = "";
function renderChat() {
  const items = state?.items ?? [];
  $("agent").textContent = state?.agent ?? "AI";
  for (const it of items) {
    if (it.kind !== "options" || offered.has(it.id)) continue;
    offered.add(it.id);
    // A question waits for an answer; on a phone it would hide in the one line under the strip.
    if (it.open && state) $("chat").classList.add("open");
  }
  renderLast();
  const key = JSON.stringify(items);
  if (key === renderedItems) return;
  renderedItems = key;
  const list = $("items");
  const atEnd = list.scrollHeight - list.scrollTop - list.clientHeight < 40;
  list.replaceChildren();
  if (!items.length) list.append(el("p", "empty", `Tell ${state?.agent ?? "the AI"} how to cut your video.`));
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
  $("send").disabled = !text || closed || !online || !state || !!reason;
  const note = held ? "It goes to the AI once your videos are on the timeline." : text && reason ? `${reason}.` : "";
  $("note").textContent = note;
  $("note").hidden = !note;
  $("text").disabled = closed;
}

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
  // Disabled only in look: a tap on a phone, which shows no tooltip, says why.
  exportButton.setAttribute("aria-disabled", String(!running && (closed || !online || !s || !!s.exportBlocked)));
  const phase = exportPhase(s);
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

// ---- Seeking on the timeline ----

let seeking = null;
let seekTimer = 0;
function seekTo(clientX) {
  if (!state?.durationUs) return;
  const box = $("clips").getBoundingClientRect();
  const pending = uploads.filter((u) => u.status !== "done" && u.status !== "failed").length + state.adding.length;
  const width = Math.max(1, box.width - pending * 66);
  seeking = Math.round(Math.min(1, Math.max(0, (clientX - box.left) / width)) * state.durationUs);
  renderStrip(true);
  if (seekTimer) return;
  seekTimer = setTimeout(() => {
    seekTimer = 0;
    if (seeking !== null) void post({ action: "seek", us: seeking });
  }, 120);
}

function setupStrip() {
  const strip = $("strip");
  strip.addEventListener("pointerdown", (e) => {
    const failed = e.target.closest(".clip.failed");
    if (failed) {
      const u = uploads[Number(failed.dataset.upload)];
      u.status = "waiting";
      u.sent = 0;
      render();
      void sendFiles();
      return;
    }
    strip.setPointerCapture(e.pointerId);
    seekTo(e.clientX);
  });
  strip.addEventListener("pointermove", (e) => {
    if (strip.hasPointerCapture(e.pointerId)) seekTo(e.clientX);
  });
  const end = () => setTimeout(() => (seeking = null), 400);
  strip.addEventListener("pointerup", end);
  strip.addEventListener("pointercancel", end);
  strip.addEventListener("keydown", (e) => {
    if (!state || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
    e.preventDefault();
    const to = Math.min(state.durationUs, Math.max(0, state.timeUs + (e.key === "ArrowLeft" ? -US : US)));
    void post({ action: "seek", us: to });
  });
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

async function sendFiles() {
  if (sending) return;
  sending = true;
  for (let u = uploads.find((x) => x.status === "waiting"); u; u = uploads.find((x) => x.status === "waiting")) {
    u.status = "sending";
    const ok = await upload(u);
    u.status = ok ? "done" : "failed";
    if (ok) lastArrival = Date.now();
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
        renderStrip(true);
        renderLast();
      }
    });
    xhr.addEventListener("load", () => {
      if (xhr.status === 403) expired();
      else if (xhr.status !== 200) toast(xhr.responseText || `${u.name} could not be sent.`);
      resolve(xhr.status === 200);
    });
    xhr.addEventListener("error", () => resolve(false));
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
  $("last").addEventListener("click", () => {
    $("chat").classList.add("open");
    $("items").scrollTop = $("items").scrollHeight;
    renderChat();
  });
  $("chat-close").addEventListener("click", () => {
    $("chat").classList.remove("open");
    renderLast();
  });
  $("export").addEventListener("click", () => {
    if (state?.exporting !== null && state?.exporting !== undefined) {
      dismissed = null;
      renderExport();
      return;
    }
    const reason = !online ? "Nuzky is not connected" : state?.exportBlocked;
    if (closed || reason) {
      if (reason) toast(`${reason}.`);
      return;
    }
    dismissed = null;
    $("video").hidden = true;
    $("video").removeAttribute("src");
    $("sheet-thumb").removeAttribute("src");
    void post({ action: "export" });
  });
  $("sheet-close").addEventListener("click", () => {
    dismissed = `${state?.exportId}:${exportPhase(state)}`;
    $("video").pause();
    renderExport();
  });
  $("play").addEventListener("click", () => {
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
  setupStrip();
  connect();
  render();
}

setup();

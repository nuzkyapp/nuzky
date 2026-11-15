//! Help improve Nuzky: what the person allowed Nuzky to send its author, collected and sent. Nothing goes out while
//! both are off, except a report the person sends themselves. Events wait in files in Nuzky's data folder and go to
//! account.nuzky.app in batches, with a random id of this install and never the account. Problems and usage carries
//! what went wrong and what was used, without typed text, the names of the project and its files, folders or secrets.
//! AI conversations carries the person's messages in the AI panel and what the agent answered and called, never what
//! the tools gave back, which holds the transcript and frames.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::{Duration, Instant, SystemTime};

use nuzky_agent::{AgentEvent, ToolStatus};
use nuzky_engine::model::Project;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::{CmdResult, diagnostics, err, store, updates};

const SERVER: &str = "https://account.nuzky.app/api/telemetry";
/// How often the waiting events are queued and the queue is sent; the first time a minute after starting.
const ROUND: Duration = Duration::from_secs(10 * 60);
const FIRST_ROUND: Duration = Duration::from_secs(60);
const TIMEOUT: Duration = Duration::from_secs(20);
const ERRORS_PER_LAUNCH: usize = 50;
const ACTIONS_KEPT: usize = 15;
const BATCH_EVENTS: usize = 400;
const BATCH_BYTES: usize = 400 * 1024;
const SENT_KEPT: usize = 5;
/// A batch another Nuzky window took this long ago and never finished goes again.
const TAKEN_EXPIRES: Duration = Duration::from_secs(60 * 60);
const STARTING: &str = "Nuzky is still starting. Try again in a moment.";
const UNREACHABLE: &str = "Nuzky couldn't reach its server. Check your connection and try again.";

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    install: String,
    problems: bool,
    conversations: bool,
    asked: bool,
    sent: u64,
}

/// What Settings › Privacy and the question after an update show.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryStatus {
    pub problems: bool,
    pub conversations: bool,
    /// The person answered the question, sharing or not, so it never asks by itself again.
    pub asked: bool,
    /// Events sent from this install since it last deleted what it shared.
    #[cfg_attr(test, ts(type = "number"))]
    pub sent: u64,
}

#[derive(Default)]
struct State {
    dir: PathBuf,
    settings: Settings,
    version: String,
    system: String,
    events: Vec<Value>,
    actions: VecDeque<String>,
    usage: BTreeMap<String, u32>,
    errors: usize,
    last_error: String,
    started: bool,
    /// The open project's name and its files' names, taken out of everything sent.
    names: Vec<String>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs `f` on the state once `init` has made it. Nothing in here may log: the logger calls in.
fn with<T>(f: impl FnOnce(&mut State) -> T) -> Option<T> {
    state().as_mut().map(f)
}

fn server() -> String {
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("NUZKY_TELEMETRY_URL")
        && url.starts_with("http://127.0.0.1:")
    {
        return url;
    }
    SERVER.into()
}

/// The end-to-end flow sends every few seconds (`NUZKY_TELEMETRY_ROUND_MS`, debug builds only).
fn rounds() -> (Duration, Duration) {
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var("NUZKY_TELEMETRY_ROUND_MS").ok().and_then(|ms| ms.parse().ok()) {
        return (Duration::from_millis(ms), Duration::from_millis(ms));
    }
    (FIRST_ROUND, ROUND)
}

/// Before anything logs, so a panic at start counts.
pub fn init() {
    let dir = store::data_dir().join("telemetry");
    let mut settings: Settings = std::fs::read(dir.join("settings.json"))
        .ok()
        .and_then(|json| serde_json::from_slice(&json).ok())
        .unwrap_or_default();
    if uuid::Uuid::parse_str(&settings.install).is_err() {
        settings.install = new_install();
    }
    *state() = Some(State {
        dir,
        settings,
        version: env!("CARGO_PKG_VERSION").into(),
        system: String::new(),
        events: Vec::new(),
        actions: VecDeque::new(),
        usage: BTreeMap::new(),
        errors: 0,
        last_error: String::new(),
        started: false,
        names: Vec::new(),
    });
}

/// Starts sending once the app knows its version.
pub fn start(app: &AppHandle) {
    let version = format!("{} ({})", app.package_info().version, env!("NUZKY_COMMIT"));
    let system = updates::system();
    with(|s| {
        s.version = version;
        s.system = system;
    });
    let (first, every) = rounds();
    std::thread::Builder::new()
        .name("telemetry".into())
        .spawn(move || {
            std::thread::sleep(first);
            loop {
                send_round();
                std::thread::sleep(every);
            }
        })
        .ok();
}

/// Quitting queues what waits; it goes with the next start.
pub fn quit() {
    with(|s| s.queue(None));
}

fn new_install() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn event(kind: &str, data: Value) -> Value {
    json!({ "kind": kind, "at": jiff::Timestamp::now().to_string(), "data": data })
}

fn cap(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

fn tenths(seconds: f64) -> f64 {
    (seconds * 10.0).round() / 10.0
}

/// An action's name and what it did, without its numbers: `import 3 files` counts as `import # files`.
fn usage_key(action: &str) -> String {
    let words = action.split_whitespace().filter(|word| !word.contains('='));
    let words: Vec<&str> =
        words.map(|word| if word.contains(|c: char| c.is_ascii_digit()) { "#" } else { word }).collect();
    words.join(" ").replace(',', "")
}

impl State {
    fn on(&self) -> bool {
        self.settings.problems || self.settings.conversations
    }

    fn forget(&self, mut text: String) -> String {
        for name in &self.names {
            text = text.replace(name.as_str(), "…");
        }
        text
    }

    /// Text from an error, a panic or a report, without typed text, folders, secrets or the names of the projects
    /// and files.
    fn clean(&self, text: &str) -> String {
        self.forget(diagnostics::scrub_names(text))
    }

    /// What the person wrote or the agent answered, as it was but for paths, secrets and the names of the projects
    /// and files.
    fn clean_said(&self, text: &str) -> String {
        self.forget(diagnostics::hide_folders(text))
    }

    /// A tool's input with its keys and numbers, each text cleaned as what was said. A key that is no plain field
    /// name, such as a file's, becomes "…", and the text under a key that names a secret goes.
    fn clean_input(&self, input: &Value) -> Value {
        match input {
            Value::String(text) => Value::String(cap(&self.clean_said(text), 300)),
            Value::Array(items) => items.iter().map(|item| self.clean_input(item)).collect(),
            Value::Object(fields) => fields
                .iter()
                .map(|(key, value)| {
                    let plain = key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                    let lower = key.to_ascii_lowercase();
                    let secret = ["key", "token", "secret", "password", "auth"].iter().any(|word| lower.contains(word));
                    let value = if secret && value.is_string() { json!("[hidden]") } else { self.clean_input(value) };
                    (if plain { key.clone() } else { "…".into() }, value)
                })
                .collect(),
            other => other.clone(),
        }
    }

    /// What another Nuzky window saved since, so no window writes back a choice the person changed in another.
    fn reload(&mut self) {
        let saved = std::fs::read(self.dir.join("settings.json")).ok();
        if let Some(mut saved) = saved.and_then(|json| serde_json::from_slice::<Settings>(&json).ok()) {
            if uuid::Uuid::parse_str(&saved.install).is_err() {
                saved.install = self.settings.install.clone();
            }
            self.settings = saved;
        }
    }

    /// Counts what was sent and keeps it for Show what was sent.
    fn sent(&mut self, events: &[Value]) {
        keep_sent(&self.dir, events);
        self.reload();
        self.settings.sent += events.len() as u64;
        self.save().ok();
    }

    fn actions(&self) -> Vec<String> {
        self.actions.iter().cloned().collect()
    }

    fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let temp = self.dir.join(format!("settings.json.{}", std::process::id()));
        std::fs::write(&temp, serde_json::to_vec_pretty(&self.settings)?)?;
        std::fs::rename(temp, self.dir.join("settings.json"))
    }

    fn status(&self) -> TelemetryStatus {
        let s = &self.settings;
        TelemetryStatus { problems: s.problems, conversations: s.conversations, asked: s.asked, sent: s.sent }
    }

    fn envelope(&self, events: Vec<Value>) -> Value {
        json!({ "install": self.settings.install, "version": self.version, "system": self.system, "events": events })
    }

    /// Writes the waiting events into the queue, in batches the server takes. `facts` add this launch's computer.
    fn queue(&mut self, facts: Option<Value>) {
        if !self.on() {
            self.events.clear();
            self.usage.clear();
            return;
        }
        if self.settings.problems {
            if let Some(facts) = facts.filter(|_| !self.started) {
                self.started = true;
                self.events.insert(0, event("start", facts));
            }
            if !self.usage.is_empty() {
                let actions = std::mem::take(&mut self.usage);
                self.events.push(event("usage", json!({ "actions": actions })));
            }
        }
        let queue = self.dir.join("queue");
        if self.events.is_empty() || std::fs::create_dir_all(&queue).is_err() {
            return;
        }
        let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let events = std::mem::take(&mut self.events);
        let last = events.len() - 1;
        let (mut batch, mut bytes) = (Vec::new(), 0);
        for (n, event) in events.into_iter().enumerate() {
            bytes += event.to_string().len();
            batch.push(event);
            if batch.len() == BATCH_EVENTS || bytes > BATCH_BYTES || n == last {
                let name = format!("{stamp}-{}-{n:03}.json", std::process::id());
                std::fs::write(queue.join(name), serde_json::to_vec(&batch).unwrap_or_default()).ok();
                (batch, bytes) = (Vec::new(), 0);
            }
        }
    }
}

/// Every log record at info or above: actions, which say what was used, and errors.
pub fn log(level: log::Level, target: &str, text: &str) {
    if target == "action" {
        let action = diagnostics::scrub_names(text);
        with(|s| {
            if s.actions.len() == ACTIONS_KEPT {
                s.actions.pop_front();
            }
            if s.settings.problems {
                *s.usage.entry(usage_key(&action)).or_default() += 1;
            }
            s.actions.push_back(action);
        });
    } else if level == log::Level::Error && !["panic", "job"].contains(&target) {
        with(|s| {
            if !s.settings.problems || s.errors == ERRORS_PER_LAUNCH || s.last_error == text {
                return;
            }
            s.errors += 1;
            s.last_error = text.to_owned();
            let data = json!({ "source": target, "message": cap(&s.clean(text), 2000), "actions": s.actions() });
            s.events.push(event("error", data));
        });
    }
}

/// A panic, queued at once, as the app may end with it. A panic while this thread holds the state is left out.
pub fn panic(thread: &str, location: &str, message: &str) {
    let mut guard = match STATE.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    let Some(s) = guard.as_mut().filter(|s| s.settings.problems) else { return };
    let message = cap(&s.clean(message), 2000);
    let data = json!({ "thread": thread, "location": location, "message": message, "actions": s.actions() });
    s.events.push(event("panic", data));
    s.queue(None);
}

/// The project's name and its files' names, so nothing sent names them, even a report sent with sharing off. Every
/// project opened since starting counts, as the log still names it.
pub(crate) fn forget_names(project: &Project) {
    let names = names_of(project);
    with(|s| remember(&mut s.names, names));
}

fn names_of(project: &Project) -> Vec<String> {
    let mut names = vec![project.name.clone()];
    for asset in &project.assets {
        let path = Path::new(&asset.path);
        names.push(asset.name.clone());
        names.extend(path.file_name().map(|name| name.to_string_lossy().into_owned()));
        names.extend(path.file_stem().map(|stem| stem.to_string_lossy().into_owned()));
    }
    names
}

/// Longest first, so a file's whole name goes before a shorter name inside it.
fn remember(known: &mut Vec<String>, names: Vec<String>) {
    known.extend(names.into_iter().filter(|name| name.chars().count() >= 3));
    known.sort();
    known.dedup();
    known.sort_by_key(|name| std::cmp::Reverse(name.len()));
}

/// Every project in the projects folder and its files' names, for a report, whose log may name any of them.
fn library_names() -> Vec<String> {
    let entries = std::fs::read_dir(store::projects_dir()).into_iter().flatten().flatten();
    let projects =
        entries.map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|x| x == store::EXTENSION));
    projects
        .flat_map(|path| {
            let mut names = store::load(&path).map(|project| names_of(&project)).unwrap_or_default();
            names.extend(path.file_stem().map(|stem| stem.to_string_lossy().into_owned()));
            names
        })
        .collect()
}

/// A job that ended. A failed one adds the project's file formats, which opens the files, so it waits on a thread
/// of its own rather than the job's.
pub(crate) fn job(app: &AppHandle, kind: &'static str, status: &'static str, seconds: f64, error: Option<String>) {
    if !with(|s| s.settings.problems).unwrap_or(false) {
        return;
    }
    let mut data = json!({ "kind": kind, "status": status, "seconds": tenths(seconds) });
    if status != "failed" {
        with(|s| s.events.push(event("job", data)));
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let files = diagnostics::file_lines(&app);
        with(|s| {
            data["error"] = json!(cap(&s.clean(error.as_deref().unwrap_or_default()), 2000));
            data["files"] = json!(files.iter().map(|line| s.clean(line)).collect::<Vec<_>>());
            data["actions"] = json!(s.actions());
            s.events.push(event("job", data));
        });
    });
}

struct Tool {
    id: String,
    name: String,
    input: Option<Value>,
    status: ToolStatus,
}

/// One answer in the AI panel, as its events stream in.
pub(crate) struct AiTurn {
    agent: &'static str,
    model: String,
    chat: String,
    prompt: String,
    answer: String,
    tools: Vec<Tool>,
    started: Instant,
}

impl AiTurn {
    /// None while both are off.
    pub(crate) fn new(agent: &'static str, model: Option<&str>, chat: &str, prompt: &str) -> Option<Self> {
        with(|s| s.on()).unwrap_or(false).then(|| Self {
            agent,
            model: model.unwrap_or("default").to_owned(),
            chat: chat.to_owned(),
            prompt: prompt.to_owned(),
            answer: String::new(),
            tools: Vec::new(),
            started: Instant::now(),
        })
    }

    pub(crate) fn see(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::Text { text } => self.answer.push_str(text),
            AgentEvent::Tool { id, name, status, input } => match self.tools.iter_mut().find(|tool| &tool.id == id) {
                Some(tool) => {
                    tool.status = *status;
                    if input.is_some() {
                        tool.input = input.clone();
                    }
                }
                None => {
                    self.tools.push(Tool { id: id.clone(), name: name.clone(), input: input.clone(), status: *status })
                }
            },
            AgentEvent::Done { stopped } => self.end(if *stopped { "stopped" } else { "done" }),
            AgentEvent::Error { code, .. } => {
                let code = serde_json::to_value(code).ok();
                self.end(code.as_ref().and_then(Value::as_str).unwrap_or("error"));
            }
            AgentEvent::Session { .. } => {}
        }
    }

    fn end(&self, end: &str) {
        let count = |failed: bool| {
            let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
            for tool in self.tools.iter().filter(|tool| !failed || tool.status == ToolStatus::Failed) {
                *counts.entry(&tool.name).or_default() += 1;
            }
            counts
        };
        let seconds = tenths(self.started.elapsed().as_secs_f64());
        with(|s| {
            if s.settings.problems {
                let data = json!({ "agent": self.agent, "model": self.model, "seconds": seconds, "end": end,
                                   "tools": count(false), "failed": count(true) });
                s.events.push(event("aiTurn", data));
            }
            if s.settings.conversations {
                let tools: Vec<Value> = self
                    .tools
                    .iter()
                    .take(20)
                    .map(|tool| {
                        let input = tool.input.as_ref().map(|input| s.clean_input(input));
                        let input = input.filter(|input| input.to_string().len() <= 2000);
                        json!({ "name": tool.name, "input": input, "status": tool.status })
                    })
                    .collect();
                let data = json!({ "chat": self.chat, "agent": self.agent, "model": self.model, "end": end,
                                   "prompt": cap(&s.clean_said(&self.prompt), 4000),
                                   "answer": cap(&s.clean_said(&self.answer), 8000), "tools": tools });
                s.events.push(event("aiChat", data));
            }
        });
    }
}

enum Failure {
    Later,
    Never,
}

fn post(url: &str, body: &Value) -> Result<(), Failure> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(url.starts_with("https://"))
        .max_redirects(0)
        .timeout_global(Some(TIMEOUT))
        .user_agent(concat!("Nuzky/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let bytes = serde_json::to_vec(body).unwrap_or_default();
    match agent.post(url).header("Content-Type", "application/json").send(&bytes[..]) {
        Ok(_) => Ok(()),
        Err(ureq::Error::StatusCode(400 | 413 | 415)) => Err(Failure::Never),
        Err(_) => Err(Failure::Later),
    }
}

/// The queued batches, oldest first, each renamed so no other Nuzky window sends it too.
fn take_queue(queue: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(queue) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    files.sort();
    let stale = |path: &Path| {
        let age = std::fs::metadata(path).and_then(|m| m.modified()).map(|at| at.elapsed().unwrap_or_default());
        age.is_ok_and(|age| age > TAKEN_EXPIRES)
    };
    let mut taken = Vec::new();
    for path in files {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let batch = name
            .strip_suffix(".json")
            .or_else(|| name.split_once(".json.taken").filter(|_| stale(&path)).map(|(b, _)| b));
        let Some(batch) = batch else { continue };
        let mine = queue.join(format!("{batch}.json.taken{}", std::process::id()));
        if std::fs::rename(&path, &mine).is_ok() {
            // Taken now, however old the batch: another window waits an hour before it takes it over.
            std::fs::File::options().write(true).open(&mine).and_then(|file| file.set_modified(SystemTime::now())).ok();
            taken.push(mine);
        }
    }
    taken
}

fn put_back(path: &Path) {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if let Some((batch, _)) = name.split_once(".json.taken") {
        std::fs::rename(path, path.with_file_name(format!("{batch}.json"))).ok();
    }
}

fn allowed(settings: &Settings, event: &Value) -> bool {
    if event["kind"] == "aiChat" { settings.conversations } else { settings.problems }
}

/// Drops from the queued batches what the person no longer allows; a batch a window is sending drops it as that
/// window sends it or puts it back.
fn drop_disallowed(queue: &Path, settings: &Settings) {
    let entries = std::fs::read_dir(queue).into_iter().flatten().flatten().map(|entry| entry.path());
    for path in entries.filter(|path| path.extension().is_some_and(|x| x == "json")) {
        keep_allowed(&path, settings);
    }
}

fn keep_allowed(path: &Path, settings: &Settings) {
    let events: Vec<Value> =
        std::fs::read(path).ok().and_then(|json| serde_json::from_slice(&json).ok()).unwrap_or_default();
    let kept: Vec<&Value> = events.iter().filter(|event| allowed(settings, event)).collect();
    if kept.is_empty() {
        std::fs::remove_file(path).ok();
    } else if kept.len() < events.len() {
        std::fs::write(path, serde_json::to_vec(&kept).unwrap_or_default()).ok();
    }
}

/// The newest batches as they went, for Show what was sent.
fn keep_sent(dir: &Path, batch: &[Value]) {
    let sent = dir.join("sent");
    if std::fs::create_dir_all(&sent).is_err() {
        return;
    }
    let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos();
    std::fs::write(sent.join(format!("{stamp}.json")), serde_json::to_vec_pretty(batch).unwrap_or_default()).ok();
    let mut kept: Vec<PathBuf> = std::fs::read_dir(&sent).into_iter().flatten().flatten().map(|e| e.path()).collect();
    kept.sort();
    for path in &kept[..kept.len().saturating_sub(SENT_KEPT)] {
        std::fs::remove_file(path).ok();
    }
}

fn send_round() {
    let facts = diagnostics::facts();
    let Some((dir, settings, envelope)) = with(|s| {
        s.reload();
        s.queue(Some(facts));
        (s.dir.clone(), s.settings.clone(), s.envelope(Vec::new()))
    }) else {
        return;
    };
    let queue = dir.join("queue");
    if !settings.problems && !settings.conversations {
        std::fs::remove_dir_all(&queue).ok();
        return;
    }
    let url = server();
    let taken = take_queue(&queue);
    for (n, path) in taken.iter().enumerate() {
        // What the person allows now, also when another window changed it during this round.
        let Some(now) = with(|s| {
            s.reload();
            s.settings.clone()
        }) else {
            break;
        };
        let events: Vec<Value> =
            std::fs::read(path).ok().and_then(|json| serde_json::from_slice(&json).ok()).unwrap_or_default();
        let events: Vec<Value> = events.into_iter().filter(|event| allowed(&now, event)).collect();
        // Delete what was sent, meanwhile, made this a new install; the old one's events go nowhere.
        if events.is_empty() || now.install != settings.install {
            std::fs::remove_file(path).ok();
            continue;
        }
        let mut body = envelope.clone();
        body["events"] = json!(events);
        match post(&url, &body) {
            Ok(()) => {
                std::fs::remove_file(path).ok();
                with(|s| s.sent(&events));
            }
            Err(Failure::Never) => {
                std::fs::remove_file(path).ok();
            }
            Err(Failure::Later) => {
                for rest in &taken[n..] {
                    keep_allowed(rest, &now);
                    put_back(rest);
                }
                break;
            }
        }
    }
}

pub(crate) fn status() -> TelemetryStatus {
    with(|s| s.status()).unwrap_or(TelemetryStatus { problems: false, conversations: false, asked: true, sent: 0 })
}

#[tauri::command]
pub fn telemetry_status() -> TelemetryStatus {
    status()
}

/// The answer to the question, or a switch in Settings › Privacy. What waits of one turned off is dropped.
#[tauri::command]
pub fn set_telemetry(problems: bool, conversations: bool) -> CmdResult<TelemetryStatus> {
    with(|s| {
        s.reload();
        s.settings.problems = problems;
        s.settings.conversations = conversations;
        s.settings.asked = true;
        if !problems {
            s.usage.clear();
            s.events.retain(|event| event["kind"] == "aiChat");
        }
        if !conversations {
            s.events.retain(|event| event["kind"] != "aiChat");
        }
        if s.on() {
            drop_disallowed(&s.dir.join("queue"), &s.settings);
        } else {
            std::fs::remove_dir_all(s.dir.join("queue")).ok();
        }
        s.save().map(|()| s.status())
    })
    .ok_or(STARTING)?
    .map_err(err)
}

/// Don't share: the question does not come back by itself.
#[tauri::command]
pub fn telemetry_not_now() -> CmdResult<()> {
    with(|s| {
        s.reload();
        s.settings.asked = true;
        s.save()
    })
    .ok_or(STARTING)?
    .map_err(err)
}

/// Deletes everything this install sent from the server, then goes on as a new install with both off.
#[tauri::command]
pub async fn delete_telemetry() -> CmdResult<TelemetryStatus> {
    let install = with(|s| s.settings.install.clone()).ok_or(STARTING)?;
    let url = format!("{}/delete", server());
    tauri::async_runtime::spawn_blocking(move || post(&url, &json!({ "install": install })))
        .await
        .map_err(err)?
        .map_err(|_| UNREACHABLE.to_owned())?;
    with(|s| {
        s.settings = Settings { install: new_install(), asked: true, ..Settings::default() };
        s.events.clear();
        s.usage.clear();
        std::fs::remove_dir_all(s.dir.join("queue")).ok();
        std::fs::remove_dir_all(s.dir.join("sent")).ok();
        s.save().map(|()| s.status())
    })
    .ok_or(STARTING)?
    .map_err(err)
}

/// Shows the newest batches sent, as they went, in the file manager.
#[tauri::command]
pub fn show_telemetry(app: AppHandle) -> CmdResult<()> {
    let sent = with(|s| s.dir.join("sent")).ok_or(STARTING)?;
    std::fs::create_dir_all(&sent).map_err(err)?;
    updates::open(&app, &sent.to_string_lossy())
}

/// The bug report, sent at once to Nuzky's author whether or not Problems and usage is on: the person asked.
#[tauri::command]
pub async fn send_report(app: AppHandle, description: String) -> CmdResult<()> {
    let report = diagnostics::diagnostics(app).await?;
    let library = tauri::async_runtime::spawn_blocking(library_names).await.map_err(err)?;
    let (report, body) = with(|s| {
        remember(&mut s.names, library);
        let text = cap(&s.clean(&without_job_labels(&report)), 60_000);
        let report = event("report", json!({ "text": text, "description": cap(&description, 4000) }));
        (report.clone(), s.envelope(vec![report]))
    })
    .ok_or(STARTING)?;
    let url = server();
    tauri::async_runtime::spawn_blocking(move || post(&url, &body))
        .await
        .map_err(err)?
        .map_err(|_| UNREACHABLE.to_owned())?;
    with(|s| s.sent(&[report]));
    Ok(())
}

/// A failed job's log line names the job as the person saw it, such as `Exporting <file> failed`, which can name a
/// file from any earlier launch; only why it failed stays.
fn without_job_labels(report: &str) -> String {
    let lines = report.lines().map(|line| {
        let failed = line.split_once(" ERROR job: ").and_then(|(at, rest)| Some((at, rest.split_once(" failed: ")?.1)));
        failed.map_or_else(|| line.to_owned(), |(at, why)| format!("{at} ERROR job: … failed: {why}"))
    });
    lines.collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_input_keeps_its_fields_but_no_secret_or_file_name() {
        let state = State { names: vec!["Client Deliverable".into()], ..State::default() };
        let input = json!({ "startUs": 1_000_000, "inputs": { "api_key": "sk-private" }, "text": "cut Client Deliverable",
                            "files": { "Client Deliverable.png": "/Users/jana/Client Deliverable.png" } });
        let clean = json!({ "startUs": 1_000_000, "inputs": { "api_key": "[hidden]" }, "text": "cut …",
                            "files": { "…": "….png" } });
        assert_eq!(state.clean_input(&input), clean);
    }

    #[test]
    fn a_failed_jobs_log_line_loses_its_label() {
        let report = "Last 2 lines of the log:\n  2026-11-15T13:00:00Z ERROR job: Exporting Client Deliverable.mp4 failed: \
                      Cannot write ….mp4\n  2026-11-15T13:00:01Z INFO action: job export failed";
        assert_eq!(
            without_job_labels(report),
            "Last 2 lines of the log:\n  2026-11-15T13:00:00Z ERROR job: … failed: Cannot write ….mp4\n  \
             2026-11-15T13:00:01Z INFO action: job export failed"
        );
    }

    #[test]
    fn usage_counts_an_action_by_its_name_without_its_numbers() {
        assert_eq!(usage_key("user splitClip atUs=12400000, updateTrack muted=true"), "user splitClip updateTrack");
        assert_eq!(usage_key("user splitClip, deleteClip"), "user splitClip deleteClip");
        assert_eq!(usage_key("import 3 files"), "import # files");
    }
}

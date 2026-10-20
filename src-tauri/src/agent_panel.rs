//! The AI panel: runs the user's own agent for one message at a time, with Nuzky's MCP bridge
//! as its only tools, or ChatGPT on the user's plan with Nuzky's tools run in the app, and streams
//! what it does to the UI as `agent` events.

use crate::attachments::{Attachment, AttachmentKind, model_image};
use crate::{AppState, CmdResult, chatgpt, connect, err, library, store};
use nuzky_agent::{AgentEvent, AgentId, AgentInfo, ChatGptRequest, McpServer, Message, Tools, Turn, TurnRequest};
use nuzky_chatgpt::Model as AgentModel;
use nuzky_mcp::{Access, Backend, Client};
use nuzky_session::host::Host;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::SystemTime;
use tauri::{AppHandle, Emitter, Manager, State};

/// One conversation with an agent; a later message resumes its session.
struct Chat {
    agent: AgentId,
    session: String,
    /// Whether a turn got far enough for the agent to keep the conversation.
    resumable: bool,
    /// ChatGPT's conversation, which Nuzky keeps, and the model it was last with.
    history: Arc<Mutex<Vec<Value>>>,
    model: Option<String>,
    project: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedChat {
    #[serde(default = "default_agent")]
    agent: AgentId,
    #[serde(default)]
    session: String,
    #[serde(default)]
    resumable: bool,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    history: Vec<Value>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    updated_ms: u64,
    #[serde(default)]
    items: Value,
}

fn default_agent() -> AgentId {
    AgentId::Claude
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSummary {
    #[serde(skip_deserializing)]
    id: String,
    #[serde(default = "default_agent")]
    agent: AgentId,
    #[serde(default)]
    title: String,
    #[serde(default)]
    updated_ms: u64,
}

#[derive(Serialize)]
pub struct OpenedChat {
    id: String,
    agent: AgentId,
    items: Value,
}

fn validate_chat_id(chat: &str) -> CmdResult<()> {
    if chat.is_empty() || chat.len() > 64 || !chat.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return Err("INVALID_CHAT: A chat id must contain 1 to 64 letters, digits or hyphens.".into());
    }
    Ok(())
}

/// The project's chats, in Nuzky's own data rather than beside the project, which may be shared.
pub(crate) fn chats_dir(project: &Path) -> PathBuf {
    use nuzky_session::hash::{FNV_OFFSET, hash_bytes};
    let mut hash = FNV_OFFSET;
    hash_bytes(&mut hash, library::key(project).as_bytes());
    store::data_dir().join("chats").join(format!("{hash:016x}"))
}

/// Pictures stay in the live conversation, but never in its saved copy.
fn strip_images(value: &mut Value) {
    if value.get("type").and_then(Value::as_str) == Some("input_image") {
        *value = serde_json::json!({"type": "input_text", "text": "[A picture you saw earlier; it is not kept.]"});
    } else {
        match value {
            Value::Array(values) => values.iter_mut().for_each(strip_images),
            Value::Object(values) => values.values_mut().for_each(strip_images),
            _ => {}
        }
    }
}

enum Start {
    Cli(TurnRequest),
    ChatGpt(ChatGptRequest),
}

/// Nuzky's tools for the panel's ChatGPT: a client of the project's session like a client of the MCP
/// bridge, with the same rules, runs and jobs, without the socket.
struct InApp(Backend);

impl Tools for InApp {
    fn call(&self, name: &str, arguments: Value) -> Value {
        serde_json::to_value(self.0.answer(name, arguments)).unwrap_or_default()
    }

    fn end(&self) -> anyhow::Result<()> {
        self.0.disconnect()
    }
}

#[derive(Default)]
pub struct AgentPanel {
    chats: Mutex<HashMap<String, Chat>>,
    /// Serialize snapshots and writes so an earlier save cannot replace a later one.
    save: Mutex<()>,
    /// The message the agent is working on, by the id the UI gave it.
    active: Mutex<Option<(String, Turn)>>,
    /// A message the user stopped while it was still starting; written under the `active` lock.
    cancelled: Mutex<Option<String>>,
    /// Counts stops, so a message still starting when one comes is stopped as soon as it runs.
    stops: AtomicU64,
    /// The owner the conversation's jobs run under, and the project's host they run in: they outlive
    /// each turn, so the agent can ask the user and read them in its next turn. Weak, as the host
    /// holds its project's lock.
    jobs: Mutex<Option<(String, Weak<Host>)>>,
}

impl AgentPanel {
    /// Stops the panel's agent, e.g. for Stop and edit in the top bar or when another project opens,
    /// also one whose message is still starting.
    pub fn stop(&self) {
        self.stops.fetch_add(1, Ordering::AcqRel);
        if let Some((_, turn)) = self.active.lock().unwrap().as_ref().filter(|(_, turn)| turn.running()) {
            turn.stop();
            // Between turns Stop and edit is about another agent, so what the panel's agent left running stays.
            self.stop_jobs();
        }
    }

    /// Stops the jobs the conversation started; its next turn starts them under a new owner.
    fn stop_jobs(&self) {
        if let Some((owner, host)) = self.jobs.lock().unwrap().take()
            && let Some(host) = host.upgrade()
        {
            host.jobs.end_conversation(&owner);
        }
    }

    fn job_owner(&self, host: &Arc<Host>) -> String {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some((owner, on)) = jobs.as_ref()
            && on.ptr_eq(&Arc::downgrade(host))
        {
            return owner.clone();
        }
        let owner = new_id();
        host.jobs.begin_conversation(&owner);
        *jobs = Some((owner.clone(), Arc::downgrade(host)));
        owner
    }
}

#[derive(Serialize, Clone)]
struct PanelEvent<'a> {
    chat: &'a str,
    #[serde(flatten)]
    event: &'a AgentEvent,
}

/// What went with the message, frozen when it was sent.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptContext {
    selection: Option<Vec<String>>,
    playhead_us: Option<i64>,
    #[serde(default)]
    frame: bool,
    /// Sent from the Your style page: the message is about the creator's style.
    #[serde(default)]
    style: bool,
    /// Make a thumbnail in the AI panel: the agent gets the MCP prompt `thumbnail` instead of the label.
    #[serde(default)]
    thumbnail: bool,
    #[serde(default)]
    attachments: Vec<Attachment>,
}

fn timecode(us: i64) -> String {
    let s = us.max(0) as f64 / 1e6;
    format!("{:02}:{:05.2}", (s / 60.0).floor() as i64, s % 60.0)
}

/// The message as the agent gets it: the user's words, then what they had selected.
fn prompt(text: &str, context: &PromptContext, project: &nuzky_engine::Project) -> String {
    let thumbnail;
    let text = if context.thumbnail {
        // What the user typed beside Make a thumbnail are their wishes.
        thumbnail = nuzky_mcp::thumbnail_prompt(if text.trim().is_empty() { "none" } else { text });
        thumbnail.as_str()
    } else {
        text
    };
    let mut lines = Vec::new();
    if context.style {
        lines.push(
            "[Nuzky] The user writes from the Your style page: this is about how you edit all their videos, not the open project. Read their style with get_style. Agree on the exact change with them first, then make it with change_style (their words as setOwn) and say in a sentence what changed; the page shows it at once.".to_owned(),
        );
    }
    if let Some(ids) = context.selection.as_ref().filter(|ids| !ids.is_empty()) {
        let clips: Vec<_> = project.tracks.iter().flat_map(|t| &t.clips).filter(|c| ids.contains(&c.id)).collect();
        let range = clips.iter().map(|c| c.start_us).min().zip(clips.iter().map(|c| c.end_us()).max());
        let range =
            range.map_or(String::new(), |(a, b)| format!(" covering {}–{} ({a}–{b} µs)", timecode(a), timecode(b)));
        lines.push(format!("[Nuzky] Selected clips: {}{range}.", ids.join(", ")));
    }
    if let Some(at) = context.playhead_us {
        lines.push(format!("[Nuzky] Playhead: {} ({at} µs).", timecode(at)));
        if context.frame {
            lines.push(format!(
                "[Nuzky] The user points at the frame at the playhead: look at it with inspect_frames at {at} µs."
            ));
        }
    }
    if context.thumbnail {
        // A frame the user chose stays: the prompt would otherwise pick one anew.
        for cover in &project.thumbnails {
            let name = match cover.format {
                nuzky_engine::model::ThumbnailFormat::Cover9x16 => "cover_9x16",
                nuzky_engine::model::ThumbnailFormat::Youtube16x9 => "youtube_16x9",
            };
            lines.push(match cover.image.as_deref().and_then(|id| project.asset(id)) {
                Some(image) => format!(
                    "[Nuzky] The project's {name} is made from the image {} ({}): keep that image unless the user's wishes say otherwise.",
                    image.name, image.id
                ),
                None => format!(
                    "[Nuzky] The project's {name} already uses the frame at {} ({} µs): keep that frame unless the user's wishes say otherwise.",
                    timecode(cover.time_us),
                    cover.time_us
                ),
            });
        }
    }
    for attached in &context.attachments {
        lines.push(match attached.kind {
            AttachmentKind::Image => {
                format!("[Nuzky] The user attached the picture {}; you see it with this message.", attached.path)
            }
            AttachmentKind::Video => format!(
                "[Nuzky] The user attached the video {}. To see or use it, add it to the project with import_media.",
                attached.path
            ),
        });
    }
    if lines.is_empty() { text.to_owned() } else { format!("{text}\n\n{}", lines.join("\n")) }
}

/// The message as the agent gets it, with the pictures attached to it.
fn message(text: &str, context: &PromptContext, project: &nuzky_engine::Project) -> CmdResult<Message> {
    let images = context
        .attachments
        .iter()
        .filter(|a| a.kind == AttachmentKind::Image)
        .map(|a| {
            model_image(Path::new(&a.path)).map_err(|error| {
                log::warn!("Attaching {}: {error:#}", a.path);
                format!("Nuzky couldn't read the picture {}. Attach it again.", a.name)
            })
        })
        .collect::<CmdResult<_>>()?;
    Ok(Message { text: prompt(text, context, project), images })
}

/// An empty folder of Nuzky's own outside the home folder, so no CLAUDE.md, AGENTS.md or
/// project settings above it load into the agent.
fn work_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_dir())
        .unwrap_or_else(std::env::temp_dir)
        .join("nuzky-agent")
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[tauri::command]
pub async fn agent_list() -> CmdResult<Vec<AgentInfo>> {
    tauri::async_runtime::spawn_blocking(|| [AgentId::Claude, AgentId::Codex].map(nuzky_agent::find).to_vec())
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn agent_chats(app: AppHandle) -> CmdResult<Vec<ChatSummary>> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = app.state::<AppState>().session.lock().unwrap().path.clone();
        let entries = match std::fs::read_dir(chats_dir(&path)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(err(error)),
        };
        let mut chats = Vec::new();
        for entry in entries {
            let path = entry.map_err(err)?.path();
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            if path.extension().is_none_or(|s| s != "json") || validate_chat_id(id).is_err() {
                continue;
            }
            let summary = std::fs::File::open(&path)
                .map_err(err)
                .and_then(|file| serde_json::from_reader::<_, ChatSummary>(std::io::BufReader::new(file)).map_err(err));
            match summary {
                Ok(mut summary) => {
                    summary.id = id.to_owned();
                    chats.push(summary);
                }
                Err(error) => log::warn!("Skipping saved chat {}: {error}", path.display()),
            }
        }
        chats.sort_by_key(|chat| std::cmp::Reverse(chat.updated_ms));
        Ok(chats)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn agent_chat_open(app: AppHandle, chat: String) -> CmdResult<OpenedChat> {
    validate_chat_id(&chat)?;
    tauri::async_runtime::spawn_blocking(move || {
        let panel = app.state::<AgentPanel>();
        if panel.active.lock().unwrap().as_ref().is_some_and(|(_, turn)| turn.running()) {
            return Err("AGENT_BUSY: The agent is still answering. Wait for it, or stop it.".into());
        }
        let project = app.state::<AppState>().session.lock().unwrap().path.clone();
        let file = std::fs::File::open(chats_dir(&project).join(format!("{chat}.json"))).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "CHAT_GONE: This chat is no longer saved.".to_owned()
            } else {
                err(error)
            }
        })?;
        let saved: SavedChat = serde_json::from_reader(std::io::BufReader::new(file)).map_err(err)?;
        // A chat this run of Nuzky already talked in is newer in memory: its file may predate a turn another
        // project's opening cut short.
        panel.chats.lock().unwrap().entry(chat.clone()).or_insert_with(|| Chat {
            agent: saved.agent,
            session: saved.session,
            resumable: saved.resumable,
            history: Arc::new(Mutex::new(saved.history)),
            model: saved.model,
            project,
        });
        Ok(OpenedChat { id: chat, agent: saved.agent, items: saved.items })
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn agent_chat_save(app: AppHandle, chat: String, title: String, items: Value) -> CmdResult<()> {
    validate_chat_id(&chat)?;
    tauri::async_runtime::spawn_blocking(move || {
        let panel = app.state::<AgentPanel>();
        let _save = panel.save.lock().unwrap();
        let (project, mut saved) = {
            let chats = panel.chats.lock().unwrap();
            let Some(chat) = chats.get(&chat) else { return Ok(()) };
            (
                chat.project.clone(),
                SavedChat {
                    agent: chat.agent,
                    session: chat.session.clone(),
                    resumable: chat.resumable,
                    model: chat.model.clone(),
                    history: chat.history.lock().unwrap().clone(),
                    title,
                    updated_ms: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_err(err)?.as_millis()
                        as u64,
                    items,
                },
            )
        };
        saved.history.iter_mut().for_each(strip_images);
        let dir = chats_dir(&project);
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir).map_err(err)?;
        nuzky_session::write_json_atomic(&dir.join(format!("{chat}.json")), &saved).map_err(err)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn agent_models(agent: AgentId) -> CmdResult<Vec<AgentModel>> {
    tauri::async_runtime::spawn_blocking(move || match agent {
        AgentId::Claude => {
            let failed = "MODELS_FAILED: Couldn't list Claude Code's models. Try again.";
            let path = nuzky_agent::find(agent).path.ok_or(failed)?;
            // Claude Code names its models only to a client that starts a session, so this starts one and
            // sends nothing to the model. The user's settings stay, as they pick what each alias runs.
            let args = [
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--setting-sources",
                "user",
                "--settings",
                r#"{"disableAllHooks":true}"#,
                "--strict-mcp-config",
                "--tools",
                "",
                "--disable-slash-commands",
                "--no-session-persistence",
            ];
            let request = br#"{"type":"control_request","request_id":"models","request":{"subtype":"initialize"}}
"#;
            let output =
                nuzky_agent::run_with_input(&path, &args, &[], None, Some(request), std::time::Duration::from_secs(15))
                    .map_err(|_| failed)?
                    .ok_or(failed)?;
            claude_models(&output.stdout).ok_or_else(|| failed.into())
        }
        AgentId::Codex => {
            let failed = "MODELS_FAILED: Couldn't list Codex's models. Try again.";
            let path = nuzky_agent::find(agent).path.ok_or(failed)?;
            let output = nuzky_agent::run(&path, &["debug", "models"], &[], None, std::time::Duration::from_secs(15))
                .map_err(|_| failed)?
                .ok_or(failed)?;
            if !output.status.success() {
                return Err(failed.into());
            }
            codex_models(&output.stdout).map_err(|_| failed.into())
        }
        AgentId::ChatGpt => chatgpt::chatgpt().models().map_err(err),
    })
    .await
    .map_err(err)?
}

/// The models from Claude Code's answer to `initialize`, without its Default, which the menu has
/// already, and once per model, since the user's settings can point two aliases at the same one.
fn claude_models(stdout: &[u8]) -> Option<Vec<AgentModel>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Entry {
        value: String,
        resolved_model: Option<String>,
        display_name: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        supports_effort: bool,
        #[serde(default)]
        supported_effort_levels: Vec<String>,
    }
    let answer = String::from_utf8_lossy(stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|v| v["type"] == "control_response" && v["response"]["request_id"] == "models")?;
    let entries: Vec<Entry> = serde_json::from_value(answer["response"]["response"]["models"].clone()).ok()?;
    let mut seen = std::collections::HashSet::new();
    Some(
        entries
            .into_iter()
            .filter(|m| {
                m.value != "default" && seen.insert(m.resolved_model.clone().unwrap_or_else(|| m.value.clone()))
            })
            .map(|m| {
                // "Sonnet" says which version only in its description, "Sonnet 5.5 · Efficient for…".
                let versioned = m.description.split(" · ").next().filter(|first| {
                    first.starts_with(&m.display_name) && !m.display_name.chars().any(|c| c.is_ascii_digit())
                });
                let efforts = if m.supports_effort {
                    m.supported_effort_levels
                        .into_iter()
                        .filter(|effort| nuzky_chatgpt::EFFORTS.contains(&effort.as_str()))
                        .collect()
                } else {
                    Vec::new()
                };
                AgentModel {
                    display_name: versioned.map_or(m.display_name.clone(), str::to_owned),
                    slug: m.value,
                    efforts,
                    default_effort: None,
                }
            })
            .collect(),
    )
}

fn codex_models(bytes: &[u8]) -> serde_json::Result<Vec<AgentModel>> {
    #[derive(Deserialize)]
    struct Level {
        effort: String,
    }
    #[derive(Deserialize)]
    struct Entry {
        slug: String,
        display_name: String,
        visibility: String,
        priority: i64,
        #[serde(default)]
        supported_reasoning_levels: Vec<Level>,
        default_reasoning_level: Option<String>,
    }
    #[derive(Deserialize)]
    struct Catalog {
        models: Vec<Entry>,
    }
    let mut models = serde_json::from_slice::<Catalog>(bytes)?.models;
    models.retain(|m| m.visibility == "list");
    models.sort_by_key(|m| m.priority);
    Ok(models
        .into_iter()
        .map(|m| {
            let efforts: Vec<String> = m
                .supported_reasoning_levels
                .into_iter()
                .map(|level| level.effort)
                .filter(|effort| nuzky_chatgpt::EFFORTS.contains(&effort.as_str()))
                .collect();
            let default_effort = m.default_reasoning_level.filter(|effort| efforts.contains(effort));
            AgentModel { slug: m.slug, display_name: m.display_name, efforts, default_effort }
        })
        .collect())
}

fn validate_model_effort(model: Option<&str>, effort: Option<&str>) -> CmdResult<()> {
    if let Some(model) = model
        && (model.is_empty()
            || model.len() > 100
            || !model.as_bytes()[0].is_ascii_alphanumeric()
            || !model.bytes().all(|c| c.is_ascii_alphanumeric() || b"._:/[]-".contains(&c)))
    {
        return Err("Choose a valid model name, starting with a letter or digit.".into());
    }
    if effort.is_some_and(|effort| !nuzky_chatgpt::EFFORTS.contains(&effort)) {
        return Err("Choose low, medium, high, xhigh or max reasoning effort.".into());
    }
    Ok(())
}

/// Sends a message in the chat the UI named, under the id it gave the message; the answer arrives
/// as `agent` events for the chat. The UI names both before sending, so even an agent that fails
/// at once reaches it, and Stop works while the message is still starting.
#[tauri::command]
#[allow(clippy::too_many_arguments, reason = "one argument per field the panel sends")]
pub async fn agent_send(
    app: AppHandle,
    agent: AgentId,
    chat: String,
    turn: String,
    text: String,
    context: PromptContext,
    model: Option<String>,
    effort: Option<String>,
) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || send(&app, agent, chat, turn, &text, &context, model, effort))
        .await
        .map_err(err)?
}

#[allow(clippy::too_many_arguments, reason = "one argument per field the panel sends")]
fn send(
    app: &AppHandle,
    agent: AgentId,
    chat: String,
    turn: String,
    text: &str,
    context: &PromptContext,
    model: Option<String>,
    effort: Option<String>,
) -> CmdResult<()> {
    validate_model_effort(model.as_deref(), effort.as_deref())?;
    validate_chat_id(&chat)?;
    let panel = app.state::<AgentPanel>();
    // Another project opening, or Stop, while this starts must stop it once it runs.
    let stops = panel.stops.load(Ordering::Acquire);
    // The panel's lock and the session's are never held together, so Stop and edit cannot deadlock with this.
    let busy = || panel.active.lock().unwrap().as_ref().is_some_and(|(_, turn)| turn.running());
    if busy() {
        return Err("AGENT_BUSY: The agent is still answering. Wait for it, or stop it.".into());
    }
    let state = app.state::<AppState>();
    let (project, path, host) = {
        let current = state.session.lock().unwrap();
        let session = current.host.session.state().map_err(err)?;
        if let Some(run) = session.open_run {
            return Err(format!(
                "AGENT_BUSY: Another agent is editing ({}). Wait for it, or stop it in the top bar.",
                run.label
            ));
        }
        (session.project, current.path.clone(), current.host.clone())
    };
    let owner = panel.job_owner(&host);
    let (session, resumable, history) = {
        let mut chats = panel.chats.lock().unwrap();
        if chats.get(&chat).is_some_and(|c| c.agent != agent) {
            chats.remove(&chat);
        }
        let chat = chats.entry(chat.clone()).or_insert_with(|| Chat {
            agent,
            session: new_id(),
            resumable: false,
            history: Arc::default(),
            model: None,
            project: path.clone(),
        });
        if agent == AgentId::ChatGpt && chat.model != model {
            // ChatGPT's reasoning goes back only to the model that wrote it.
            chat.history.lock().unwrap().retain(|item| item["type"] != "reasoning");
            chat.model = model.clone();
        }
        (chat.session.clone(), chat.resumable, chat.history.clone())
    };
    let id = chat;
    let request = if agent == AgentId::ChatGpt {
        let client = Client { id: owner, access: Access::Write };
        Start::ChatGpt(ChatGptRequest {
            api: chatgpt::chatgpt().api().to_owned(),
            model: model.ok_or("Choose a ChatGPT model first.")?,
            effort,
            guide: nuzky_mcp::guide(),
            tools: nuzky_mcp::tool_list().map_err(err)?,
            prompt: message(text, context, &project)?,
            history,
            token: Arc::new(|renew| chatgpt::chatgpt().access_token(renew)),
            run: Arc::new(InApp(Backend::shared(host, &path, client).map_err(err)?)),
        })
    } else {
        let info = nuzky_agent::find(agent);
        let exe = info.path.ok_or_else(|| format!("NOT_INSTALLED: {} is not installed.", agent.name()))?;
        // The bridge attaches to this window's project by its path: `--current` names whatever project
        // the window opened last made current, which may be another Nuzky window's.
        let bridge = connect::Command::this_app().map_err(err)?;
        let args = vec!["mcp".into(), "--project".into(), path.to_string_lossy().into_owned(), "--allow-write".into()];
        // The agent's own `nuzky mcp` reads the creator's style itself, so it needs the same data folder.
        let mut env: Vec<_> = ["XDG_RUNTIME_DIR", "NUZKY_DATA_DIR"]
            .into_iter()
            .filter_map(|var| std::env::var(var).ok().map(|value| (var.to_owned(), value)))
            .collect();
        env.push((nuzky_mcp::bridge::CONVERSATION_VAR.to_owned(), owner));
        Start::Cli(TurnRequest {
            agent,
            exe,
            session,
            resume: resumable,
            model,
            effort,
            prompt: message(text, context, &project)?,
            cwd: work_dir(),
            mcp: McpServer { command: bridge.program.into(), args, env },
        })
    };
    if panel.cancelled.lock().unwrap().as_deref() == Some(turn.as_str()) {
        app.emit("agent", PanelEvent { chat: &id, event: &AgentEvent::Done { stopped: true } }).ok();
        return Ok(());
    }
    let (handle, chat_id) = (app.clone(), id.clone());
    let emit = move |event| {
        let panel = handle.state::<AgentPanel>();
        if let AgentEvent::Session { id } = event {
            if let Some(chat) = panel.chats.lock().unwrap().get_mut(&chat_id) {
                chat.session = id;
                chat.resumable = true;
            }
            return;
        }
        handle.emit("agent", PanelEvent { chat: &chat_id, event: &event }).ok();
        if let AgentEvent::Error { .. } = event
            && let Some(chat) = panel.chats.lock().unwrap().get_mut(&chat_id)
            && !chat.resumable
        {
            // A turn that never got going leaves no conversation to resume, so the next one starts it afresh.
            chat.session = new_id();
        }
    };
    let started = match request {
        Start::Cli(request) => Turn::start(request, emit),
        Start::ChatGpt(request) => Turn::chatgpt(request, emit),
    }
    .map_err(err)?;
    let mut active = panel.active.lock().unwrap();
    if panel.stops.load(Ordering::Acquire) != stops || panel.cancelled.lock().unwrap().as_deref() == Some(turn.as_str())
    {
        started.stop();
    }
    *active = Some((turn, started));
    Ok(())
}

/// A message the user sends while the agent works on `turn`: "taken" in this turn, or "next" when the panel
/// sends it as the next message once this turn ends. Codex reads one message per turn, so once it has kept
/// this turn's it is "stopped" for it: its run ends first, as when its bridge disconnects, so the jobs it
/// started go on.
#[tauri::command]
pub async fn agent_steer(
    app: AppHandle,
    turn: String,
    text: String,
    context: PromptContext,
) -> CmdResult<&'static str> {
    tauri::async_runtime::spawn_blocking(move || steer(&app, &turn, &text, &context)).await.map_err(err)?
}

fn steer(app: &AppHandle, turn: &str, text: &str, context: &PromptContext) -> CmdResult<&'static str> {
    let (panel, state) = (app.state::<AgentPanel>(), app.state::<AppState>());
    let project = state.session.lock().unwrap().host.session.state().map_err(err)?.project;
    let prompt = message(text, context, &project)?;
    let this = |(id, started): &&(String, Turn)| id == turn && started.running();
    let resumable = match panel.active.lock().unwrap().as_ref().filter(this) {
        None => return Ok("next"),
        Some((_, started)) => {
            if started.steer(&prompt) {
                return Ok("taken");
            }
            started.resumable()
        }
    };
    if !resumable {
        return Ok("next");
    }
    let ended = state.session.lock().unwrap().host.session.stop_run();
    if let Some((_, started)) = panel.active.lock().unwrap().as_ref().filter(this) {
        started.stop();
    }
    match ended {
        Err(error) if !error.to_string().starts_with("INVALID_RUN") => Err(err(error)),
        _ => Ok("stopped"),
    }
}

/// Stops the agent: Nuzky ends the run first, so the editor unlocks at once whatever the agent does.
/// A message still starting is stopped as soon as it runs. Its jobs stop too, also between turns,
/// which is how a new conversation ends the old one's.
#[tauri::command]
pub fn agent_stop(state: State<'_, AppState>, panel: State<'_, AgentPanel>, turn: String) -> CmdResult<()> {
    panel.stop_jobs();
    {
        let active = panel.active.lock().unwrap();
        if !active.as_ref().is_some_and(|(id, started)| *id == turn && started.running()) {
            *panel.cancelled.lock().unwrap() = Some(turn);
            return Ok(());
        }
    }
    let ended = state.session.lock().unwrap().host.stop_run();
    // The agent stops even when ending the run failed, e.g. on a full disk.
    panel.stop();
    match ended {
        // The agent may end its run itself at this very moment; then there was nothing left to stop.
        Err(error) if !error.to_string().starts_with("INVALID_RUN") => Err(err(error)),
        _ => Ok(()),
    }
}

/// The conversation's jobs, for the panel to show between turns.
#[tauri::command]
pub fn agent_jobs(panel: State<'_, AgentPanel>) -> Vec<serde_json::Value> {
    let jobs = panel.jobs.lock().unwrap();
    jobs.as_ref().and_then(|(owner, host)| Some(host.upgrade()?.jobs.owned_by(owner))).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuzky_engine::Project;

    #[test]
    fn model_effort_validation_rejects_invalid_input() {
        for model in ["opus", "openai/gpt-6.1", "a:b_[c].-", "7", &"a".repeat(100)] {
            assert!(validate_model_effort(Some(model), None).is_ok(), "{model}");
        }
        for model in ["", "-m", "_opus", "é", "a b", "a\\b", "a\n", &"a".repeat(101)] {
            assert!(validate_model_effort(Some(model), None).is_err(), "{model}");
        }
        assert!(validate_model_effort(None, None).is_ok());
        for effort in ["low", "medium", "high", "xhigh", "max"] {
            assert!(validate_model_effort(None, Some(effort)).is_ok());
        }
        for effort in ["", "ultra", "High", "high "] {
            assert!(validate_model_effort(None, Some(effort)).is_err());
        }
    }

    #[test]
    fn claude_models_skip_default_and_aliases_of_the_same_model() {
        let model = |value: &str, resolved: Option<&str>, name: &str, description: &str, efforts: &[&str]| {
            serde_json::json!({"value": value, "resolvedModel": resolved, "displayName": name, "description": description,
                "supportsEffort": value != "gateway", "supportedEffortLevels": efforts})
        };
        let models = [
            model("default", Some("claude-opus-5-5"), "Default (recommended)", "", &["low", "high"]),
            model(
                "opus",
                Some("claude-opus-5-5"),
                "Opus 5.5",
                "Custom Opus model (claude-opus-5-5)",
                &["low", "medium", "high", "xhigh", "max", "ultra"],
            ),
            model(
                "sonnet",
                Some("claude-sonnet-5-5"),
                "Sonnet",
                "Sonnet 5.5 · Efficient for routine tasks",
                &["low", "high"],
            ),
            model("haiku", Some("claude-opus-5-5"), "Opus 5.5", "Custom Haiku model (claude-opus-5-5)", &["low"]),
            model("gateway", None, "Gateway", "Their model · fast", &["low"]),
        ];
        let answer = serde_json::json!({"type": "control_response",
            "response": {"subtype": "success", "request_id": "models", "response": {"models": models}}});
        let stdout = format!("{}\n{answer}\n", serde_json::json!({"type": "system", "subtype": "hook_started"}));
        let models = claude_models(stdout.as_bytes()).unwrap();
        let names: Vec<_> = models.iter().map(|m| (m.slug.as_str(), m.display_name.as_str())).collect();
        assert_eq!(names, [("opus", "Opus 5.5"), ("sonnet", "Sonnet 5.5"), ("gateway", "Gateway")]);
        assert_eq!(models[0].efforts, ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(models[1].efforts, ["low", "high"]);
        assert!(models[2].efforts.is_empty());
        assert!(models.iter().all(|m| m.default_effort.is_none()));
        assert!(claude_models(b"{\"type\":\"result\"}\n").is_none());
    }

    #[test]
    fn codex_models_filter_order_and_supported_default() {
        let models = codex_models(br#"{"models": [
            {"slug":"later","display_name":"Later","visibility":"list","priority":2,
             "supported_reasoning_levels":[{"effort":"low"},{"effort":"high"},{"effort":"ultra"}],"default_reasoning_level":"high"},
            {"slug":"hidden","display_name":"Hidden","visibility":"hidden","priority":0},
            {"slug":"first","display_name":"First","visibility":"list","priority":1,
             "supported_reasoning_levels":[{"effort":"medium"},{"effort":"ultra"}],"default_reasoning_level":"ultra"},
            {"slug":"missing","display_name":"Missing","visibility":"list","priority":3}
        ]}"#).unwrap();
        assert_eq!(models.iter().map(|m| m.slug.as_str()).collect::<Vec<_>>(), ["first", "later", "missing"]);
        assert_eq!(models[0].efforts, ["medium"]);
        assert_eq!(models[0].default_effort, None);
        assert_eq!(models[1].efforts, ["low", "high"]);
        assert_eq!(models[1].default_effort.as_deref(), Some("high"));
        assert!(models[2].efforts.is_empty());
        assert_eq!(models[2].default_effort, None);
        assert_eq!(
            serde_json::to_value(&models[1]).unwrap(),
            serde_json::json!({
                "slug": "later", "displayName": "Later", "efforts": ["low", "high"], "defaultEffort": "high"
            })
        );
        assert!(codex_models(b"not JSON").is_err());
        assert!(codex_models(br#"{}"#).is_err());
    }

    #[test]
    fn saved_chat_ids_cannot_escape_their_directory() {
        for id in ["a", "Chat-123", &"a".repeat(64)] {
            assert!(validate_chat_id(id).is_ok(), "{id}");
        }
        for id in ["", "../chat", "a/b", "a\\b", "a.json", "é", &"a".repeat(65)] {
            assert!(validate_chat_id(id).unwrap_err().starts_with("INVALID_CHAT:"), "{id}");
        }
    }

    #[test]
    fn saved_chat_history_keeps_text_without_frames() {
        let history = serde_json::json!([{"output": [{"type": "input_text", "text": "Look"},
            {"type": "input_image", "image_url": "data:image/png;base64,private"}]}]);
        let mut saved = history.clone();
        strip_images(&mut saved);
        assert_eq!(saved[0]["output"][0], history[0]["output"][0]);
        assert_eq!(
            saved[0]["output"][1],
            serde_json::json!({"type": "input_text",
            "text": "[A picture you saw earlier; it is not kept.]"})
        );
        assert_eq!(history[0]["output"][1]["type"], "input_image");
        let defaults: SavedChat = serde_json::from_str("{}").unwrap();
        assert!(defaults.history.is_empty() && !defaults.resumable && defaults.items.is_null());
        let mut summary: ChatSummary =
            serde_json::from_value(serde_json::json!({"history": history, "items": []})).unwrap();
        summary.id = "chat-1".into();
        assert_eq!(serde_json::to_value(summary).unwrap()["id"], "chat-1");
    }

    #[test]
    fn the_prompt_carries_what_was_selected_when_sent() {
        let project = Project::new("t");
        let context = PromptContext {
            selection: None,
            playhead_us: Some(7_200_000),
            frame: true,
            style: false,
            thumbnail: false,
            attachments: Vec::new(),
        };
        let text = prompt("Zkrať to", &context, &project);
        assert!(text.starts_with("Zkrať to\n\n[Nuzky] Playhead: 00:07.20 (7200000 µs)."), "{text}");
        assert!(text.contains("inspect_frames at 7200000 µs"));
        let none = PromptContext {
            selection: Some(Vec::new()),
            playhead_us: None,
            frame: true,
            style: false,
            thumbnail: false,
            attachments: Vec::new(),
        };
        assert_eq!(prompt("Ahoj", &none, &project), "Ahoj");
        let attachment = |path: &str, kind| Attachment { path: path.into(), kind, name: String::new(), preview: None };
        let attached = PromptContext {
            attachments: vec![
                attachment("/a/shot.png", AttachmentKind::Image),
                attachment("/a/clip.mov", AttachmentKind::Video),
            ],
            ..none
        };
        let text = prompt("", &attached, &project);
        assert!(
            text.contains("picture /a/shot.png; you see it")
                && text.contains("video /a/clip.mov. To see or use it, add it to the project with import_media"),
            "{text}"
        );
        let style = PromptContext {
            selection: None,
            playhead_us: None,
            frame: false,
            style: true,
            thumbnail: false,
            attachments: Vec::new(),
        };
        let text = prompt("Nestříhej mi rozloučení", &style, &project);
        assert!(
            text.starts_with("Nestříhej mi rozloučení\n\n[Nuzky] The user writes from the Your style page")
                && text.contains("change_style"),
            "{text}"
        );
        let thumbnail = PromptContext { thumbnail: true, playhead_us: Some(0), style: false, ..style };
        let text = prompt("", &thumbnail, &project);
        assert!(text.starts_with("Make a 9:16 cover") && text.contains("export_thumbnail"), "{text}");
        assert!(
            text.contains("wishes, which win over the steps: none") && text.contains("[Nuzky] Playhead: 00:00.00"),
            "{text}"
        );
        let text = prompt("Hook: I QUIT SUGAR", &thumbnail, &project);
        assert!(text.contains("wishes, which win over the steps: Hook: I QUIT SUGAR"), "{text}");
        let mut covered = Project::new("t");
        covered
            .thumbnails
            .push(serde_json::from_value(serde_json::json!({"format": "cover_9x16", "timeUs": 10_620_000})).unwrap());
        let text = prompt("", &thumbnail, &covered);
        assert!(
            text.contains("[Nuzky] The project's cover_9x16 already uses the frame at 00:10.62 (10620000 µs)"),
            "{text}"
        );
    }
}

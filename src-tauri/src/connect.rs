//! Connect your agent: writes the `nuzky` MCP server into the agents' own
//! configuration, after a backup, and changes nothing else in those files. The entry runs this
//! app's `mcp --current`, so the agent edits whichever project is open here, live, while Agent
//! access is on. Other agents get the same server as text to paste.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, value::RawValue};

pub const NAME: &str = "nuzky";

#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "AgentKind"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Agent {
    ClaudeCode,
    Codex,
    Antigravity,
}

impl Agent {
    pub const ALL: [Agent; 3] = [Agent::ClaudeCode, Agent::Codex, Agent::Antigravity];

    fn name(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "Claude Code",
            Agent::Codex => "Codex",
            // Gemini runs in Google's Antigravity CLI, which is what gets connected.
            Agent::Antigravity => "Gemini",
        }
    }

    pub fn id(self) -> nuzky_agent::AgentId {
        match self {
            Agent::ClaudeCode => nuzky_agent::AgentId::Claude,
            Agent::Codex => nuzky_agent::AgentId::Codex,
            Agent::Antigravity => nuzky_agent::AgentId::Antigravity,
        }
    }

    /// Where the agent keeps its user-wide MCP servers, honouring its own override variable.
    pub fn config(self, home: &Path) -> PathBuf {
        let from = |var: &str| std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from);
        match self {
            Agent::ClaudeCode => from("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.to_path_buf()).join(".claude.json"),
            Agent::Codex => from("CODEX_HOME").unwrap_or_else(|| home.join(".codex")).join("config.toml"),
            Agent::Antigravity => home.join(".gemini/config/mcp_config.json"),
        }
    }

    pub fn settings(self, home: &Path) -> Option<PathBuf> {
        (self == Agent::Antigravity).then(|| home.join(".gemini/antigravity-cli/settings.json"))
    }
}

/// Whether agents attached with `--current` may edit: off unless the user turns it on, or asked for
/// it at every launch.
static ACCESS: LazyLock<Arc<AtomicBool>> = LazyLock::new(|| Arc::new(AtomicBool::new(at_launch())));

#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "AgentAccess"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Access {
    pub on: bool,
    /// Turned on whenever Nuzky starts.
    pub at_launch: bool,
}

/// What the app's agent listener reads.
#[cfg(unix)]
pub fn access_flag() -> Arc<AtomicBool> {
    ACCESS.clone()
}

pub fn access() -> Access {
    Access { on: ACCESS.load(Ordering::Acquire), at_launch: at_launch() }
}

pub fn set_access(on: bool, at_launch: bool) -> Result<Access> {
    if at_launch != self::at_launch() {
        replace(&access_path(), serde_json::json!({"atLaunch": at_launch}).to_string().as_bytes())?;
    }
    ACCESS.store(on, Ordering::Release);
    Ok(access())
}

fn access_path() -> PathBuf {
    crate::store::data_dir().join("agent-access.json")
}

fn at_launch() -> bool {
    read(&access_path())
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|saved| saved["atLaunch"] == true)
}

/// What the agent runs to reach Nuzky.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
}

impl Command {
    /// This app's own executable; an AppImage runs from a temporary mount, so the AppImage itself.
    pub fn this_app() -> Result<Self> {
        let exe = std::env::current_exe().context("Finding the Nuzky executable")?;
        let var = |name| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        let program = app_image(&exe, var("APPDIR"), var("APPIMAGE")).unwrap_or(exe);
        Ok(Self {
            program: program.to_string_lossy().into_owned(),
            args: ["mcp", "--current", "--allow-write"].map(String::from).to_vec(),
        })
    }
}

/// The AppImage this executable runs from. Children inherit APPIMAGE, so started from another
/// AppImage's terminal it names that one; only an executable inside APPDIR is its own.
fn app_image(exe: &Path, dir: Option<PathBuf>, image: Option<PathBuf>) -> Option<PathBuf> {
    image.filter(|_| dir.is_some_and(|dir| exe.starts_with(dir)))
}

/// An agent Nuzky cannot connect itself, which gets the server and a first message to paste.
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "AgentSetup"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Setup {
    Claude,
    Cursor,
    /// OpenCode keeps local servers under `mcp` in its own `opencode.json`, in a different shape.
    Opencode,
    Other,
}

/// The `nuzky` server in the shape the agent reads, between where it goes and what to tell the agent
/// first.
pub fn setup(setup: Setup, command: &Command) -> Result<String> {
    let (server, (place, agent)) = match setup {
        Setup::Claude => (
            serde_json::json!({"mcpServers": {NAME: {"command": command.program, "args": command.args}}}),
            ("the Claude Desktop config (Settings, Developer, Edit Config), then restart Claude", "Claude"),
        ),
        Setup::Cursor => (
            serde_json::json!({"mcpServers": {NAME: {"command": command.program, "args": command.args}}}),
            ("Cursor's MCP settings (~/.cursor/mcp.json)", "Cursor"),
        ),
        // OpenCode's `mcp` entries name the program and arguments as one `command` array.
        Setup::Opencode => (
            serde_json::json!({"mcp": {NAME: {
                "type": "local",
                "command": [command.program.clone()].into_iter().chain(command.args.iter().cloned()).collect::<Vec<_>>(),
                "enabled": true,
                "environment": {},
            }}}),
            (
                "OpenCode's config file (opencode.json next to your project, or the global one — see opencode.ai/docs/config)",
                "OpenCode",
            ),
        ),
        Setup::Other => (
            serde_json::json!({"mcpServers": {NAME: {"command": command.program, "args": command.args}}}),
            ("your agent's MCP settings", "the agent"),
        ),
    };
    let entry = if setup == Setup::Opencode { "\"mcp\"" } else { "\"mcpServers\"" };
    Ok(format!(
        "Nuzky's MCP server lets an agent edit the video open in Nuzky, live.\n\n\
         1. Add it to {place}. If the file already has {entry}, add only the \"{NAME}\" entry:\n\n{}\n\n\
         2. Then tell {agent}:\n\n\
         Nuzky is the video editor open on this computer, and its MCP server nuzky edits the project open in it. \
         Start by reading the resource nuzky://guide and follow it. Change the video only through Nuzky's tools, \
         never its project file. Open a run with begin_run before you change anything and close it with end_run: \
         the whole run is one undo step for the user.\n",
        serde_json::to_string_pretty(&server)?
    ))
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// The entry runs this app.
    Connected,
    /// A `nuzky` entry runs something else, such as another copy of Nuzky.
    Other,
    /// No `nuzky` entry.
    Missing,
    /// The file is not valid; connecting would not touch it.
    Unreadable,
}

/// An agent Nuzky can be connected to, and the state of its `nuzky` MCP entry.
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "AgentConnection"))]
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub agent: Agent,
    pub name: &'static str,
    /// Its config file.
    pub path: String,
    /// `connected`: the entry runs this app. `other`: a `nuzky` entry runs something else. `missing`: no
    /// `nuzky` entry. `unreadable`: the file is not valid, and connecting would not touch it.
    #[cfg_attr(test, ts(inline))]
    pub state: State,
    /// Why the file could not be read.
    pub problem: Option<String>,
    /// The copies made before the last change, in order.
    pub backups: Vec<String>,
}

impl Connection {
    /// Paths in the home folder as `~/…`, as people read them.
    pub fn shown_from(mut self, home: &Path) -> Self {
        self.path = tilde(&self.path, home);
        self.backups = self.backups.iter().map(|backup| tilde(backup, home)).collect();
        self
    }
}

pub fn tilde(path: &str, home: &Path) -> String {
    match Path::new(path).strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.to_string(),
    }
}

/// Where the app's socket lives on Linux. Codex hands a server only a few variables, so its entry
/// passes this one on; without it the server looks for Nuzky in another folder.
const RUNTIME_DIR: &str = "XDG_RUNTIME_DIR";

/// What Nuzky's server said when it failed, for a person: without its code, and its agent-facing "no project is
/// open" as what it means here.
#[cfg(unix)]
fn server_issue(said: &str) -> String {
    match said.split_once(": ") {
        _ if said.is_empty() => String::new(),
        Some(("APP_NOT_RUNNING", _)) => ": it couldn't reach this Nuzky".into(),
        Some((code, rest)) if code.chars().all(|c| c.is_ascii_uppercase() || c == '_') => {
            format!(": {}", rest.trim_end_matches('.'))
        }
        _ => format!(": {}", said.trim_end_matches('.')),
    }
}

#[cfg(unix)]
fn claude_fields(text: &str) -> (&str, &str, &str) {
    let field = |name| text.lines().find_map(|line| line.trim().strip_prefix(name)).unwrap_or_default().trim();
    (field("Status:"), field("Scope:"), field("Issue:"))
}

#[cfg(unix)]
#[derive(Debug, Deserialize)]
struct CodexServer {
    enabled: bool,
    transport: CodexTransport,
}

#[cfg(unix)]
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum CodexTransport {
    Stdio(CodexStdio),
    #[serde(other)]
    Other,
}

#[cfg(unix)]
#[derive(Debug, Deserialize)]
struct CodexStdio {
    command: String,
    args: Vec<String>,
    env: Option<std::collections::BTreeMap<String, String>>,
    env_vars: Option<Vec<String>>,
    cwd: Option<PathBuf>,
}

#[cfg(unix)]
fn codex_server(text: &[u8]) -> std::result::Result<CodexStdio, String> {
    let server: CodexServer =
        serde_json::from_slice(text).map_err(|_| "Codex couldn't read its Nuzky settings. Press Connect again.")?;
    if !server.enabled {
        return Err("Codex has Nuzky turned off. Press Connect to turn it on.".into());
    }
    match server.transport {
        CodexTransport::Other => {
            Err("Codex's server named nuzky isn't this Nuzky. Press Connect to replace it.".into())
        }
        CodexTransport::Stdio(transport) => Ok(transport),
    }
}

#[cfg(unix)]
pub fn verify(agent: Agent, program: &Path, home: &Path) -> std::result::Result<(), String> {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let nonce = nuzky_engine::edit::new_id();
    if agent == Agent::ClaudeCode {
        let failed = "Claude Code's check of Nuzky's server failed";
        let output = nuzky_agent::run(
            program,
            &["mcp", "get", NAME],
            &[(nuzky_mcp::bridge::VERIFY_VAR, &nonce)],
            Some(home),
            Duration::from_secs(20),
        )
        .map_err(|_| format!("{failed}. Try again."))?
        .ok_or("Claude Code didn't answer within 20 seconds. Run claude in a terminal once to finish an update or sign-in it waits for, then try again.")?;
        let text = String::from_utf8_lossy(&output.stdout);
        let (status, scope, issue) = claude_fields(&text);
        let reached = nuzky_mcp::ipc::verified(&nonce);
        if !output.status.success() && String::from_utf8_lossy(&output.stderr).contains("No MCP server named") {
            return Err("Claude Code doesn't see Nuzky in its settings. Press Connect again.".into());
        }
        if ["Pending approval", "Rejected", "Disabled"].iter().any(|state| status.contains(state)) {
            return Err("Claude Code has Nuzky turned off. Turn it on with /mcp in Claude Code, then try again.".into());
        }
        if status.contains("Connected") && !status.to_ascii_lowercase().contains("failed") {
            return if reached {
                Ok(())
            } else {
                // "User config (available in all your projects)" reads as "its user config".
                let scope = scope.split(" (").next().unwrap_or_default().to_lowercase();
                Err(format!(
                    "Claude Code uses another server named nuzky, from its {scope}, not this Nuzky. Remove that one, then try again."
                ))
            };
        }
        return Err(format!("{failed}{}. Try again.", server_issue(issue)));
    }

    let mut command = if agent == Agent::Antigravity {
        // agy starts the entry with its own environment and the entry's `env` on top; `agy mcp list` starts nothing.
        let missing = "Gemini doesn't see Nuzky in its settings. Press Connect again.";
        let text = read(&agent.config(home)).ok().flatten().ok_or(missing)?;
        let found = entry(agent, Some(&text), false).ok().flatten().filter(|c| !c.program.is_empty()).ok_or(missing)?;
        if let Some(settings) = agent.settings(home)
            && let Ok(Some(text)) = read(&settings)
        {
            let shown = tilde(&settings.display().to_string(), home);
            // agy refuses to start at all with settings it cannot parse, whatever its servers are.
            let Ok(settings) = serde_json::from_str::<Value>(&text) else {
                return Err(format!("{shown} isn't valid JSON, so agy can't start. Fix it, then try again."));
            };
            // agy weighs deny over ask over allow, so such a rule beats the one Connect added.
            let blocking: Vec<&str> = ["deny", "ask"]
                .iter()
                .flat_map(|kind| settings["permissions"][kind].as_array().into_iter().flatten())
                .filter_map(Value::as_str)
                .filter(|rule| rule.starts_with("mcp(nuzky/") || *rule == "mcp(*)")
                .collect();
            if !blocking.is_empty() {
                return Err(format!(
                    "{shown} denies or asks before Nuzky's tools ({}), which wins over letting Gemini use them. Remove that there, then try again.",
                    blocking.join(", ")
                ));
            }
        }
        let config: Value = serde_json::from_str(&text).map_err(|_| missing)?;
        let env: std::collections::BTreeMap<String, String> =
            serde_json::from_value(config["mcpServers"][NAME]["env"].clone()).unwrap_or_default();
        let mut command = std::process::Command::new(found.program);
        command.args(found.args).envs(env).current_dir(home);
        command
    } else {
        let output = nuzky_agent::run(program, &["mcp", "get", NAME, "--json"], &[], Some(home), Duration::from_secs(10))
            .map_err(|_| "Codex couldn't read its Nuzky settings. Try again.")?
            .ok_or("Codex didn't answer within 10 seconds. Run codex in a terminal once to finish an update or sign-in it waits for, then try again.")?;
        if !output.status.success() {
            return Err("Codex doesn't see Nuzky in its settings. Press Connect again.".into());
        }
        let CodexStdio { command, args, env, env_vars, cwd } = codex_server(&output.stdout)?;
        // Mirrors the default environment variables Codex passes to its stdio servers.
        const DEFAULT_ENV: &[&str] =
            &["HOME", "LANG", "LOGNAME", "PATH", "SHELL", "TMPDIR", "USER", "__CF_USER_TEXT_ENCODING"];
        let mut command = std::process::Command::new(command);
        command.args(args).env_clear().current_dir(cwd.as_deref().unwrap_or(home));
        for name in
            DEFAULT_ENV.iter().copied().chain(env_vars.as_deref().unwrap_or_default().iter().map(String::as_str))
        {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.envs(env.unwrap_or_default());
        command
    };
    command.env(nuzky_mcp::bridge::VERIFY_VAR, &nonce);
    command.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let stopped = format!("Nuzky's server for {} stopped before it reached Nuzky", agent.name());
    let mut child = command.spawn().map_err(|error| format!("{stopped}: {error}. Try again."))?;
    let mut stderr = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let result = loop {
        if nuzky_mcp::ipc::verified(&nonce) {
            break Ok(());
        }
        if !matches!(child.try_wait(), Ok(None)) {
            break Err(true);
        }
        if Instant::now() >= deadline {
            break Err(false);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
    let _ = child.kill();
    let _ = child.wait();
    let stderr = reader.join().unwrap_or_default();
    match result {
        Ok(()) => Ok(()),
        Err(true) => {
            let last = stderr.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or_default().trim();
            Err(format!("{stopped}{}. Try again.", server_issue(last)))
        }
        Err(false) => {
            Err(format!("Nuzky's server for {} didn't reach Nuzky within 15 seconds. Try again.", agent.name()))
        }
    }
}

#[cfg(not(unix))]
pub fn verify(_agent: Agent, _program: &Path, _home: &Path) -> std::result::Result<(), String> {
    Err("AI editing isn't available on Windows yet.".into())
}

pub fn status(agent: Agent, path: &Path, command: &Command) -> Connection {
    let runtime_dir = std::env::var_os(RUNTIME_DIR).is_some_and(|v| !v.is_empty());
    let (state, problem) = match read(path).and_then(|text| entry(agent, text.as_deref(), runtime_dir)) {
        Ok(None) => (State::Missing, None),
        Ok(Some(found)) if &found == command => (State::Connected, None),
        Ok(Some(_)) => (State::Other, None),
        Err(error) => (State::Unreadable, Some(format!("{error:#}"))),
    };
    Connection { agent, name: agent.name(), path: path.display().to_string(), state, problem, backups: Vec::new() }
}

/// Writes the entry unless it is already there. The file is copied first, next to itself, and
/// replaced in one step; a symlinked config is written where the link points.
pub fn connect(
    agent: Agent,
    path: &Path,
    settings: Option<&Path>,
    command: &Command,
    stamp: u64,
) -> Result<Connection> {
    let settings = settings
        .map(|path| {
            resolved(path)
                .and_then(|resolved| {
                    let before = read(&resolved)?;
                    let text = with_permissions(before.as_deref())?;
                    Ok((resolved, before, text))
                })
                .with_context(|| {
                    format!(
                        "{} is not readable Antigravity settings, so Nuzky left it alone; agy can't start with it either",
                        path.display()
                    )
                })
        })
        .transpose()?;
    let path = resolved(path)?;
    let before = read(&path)?;
    let connection = status(agent, &path, command);
    let mut changes = Vec::new();
    match connection.state {
        State::Connected => {}
        State::Unreadable => bail!(
            "{} is not valid, so Nuzky left it alone: {}",
            path.display(),
            connection.problem.as_deref().unwrap_or_default()
        ),
        State::Missing | State::Other => {
            let text = match agent {
                Agent::ClaudeCode => with_json_entry(
                    before.as_deref(),
                    &serde_json::json!({"type": "stdio", "command": command.program, "args": command.args, "env": {}}),
                )?,
                Agent::Antigravity => with_json_entry(
                    before.as_deref(),
                    &serde_json::json!({"command": command.program, "args": command.args}),
                )?,
                Agent::Codex => with_codex_entry(before.as_deref(), command)?,
            };
            changes.push((path.clone(), before, text));
        }
    }
    if let Some((path, before, Some(text))) = settings {
        changes.push((path, before, text));
    }
    let mut backups = Vec::new();
    for (path, before, text) in changes {
        if before.is_some() {
            let backup = path.with_file_name(format!(
                "{}.nuzky-backup-{stamp}",
                path.file_name().context("Config has no file name")?.to_string_lossy()
            ));
            std::fs::copy(&path, &backup).with_context(|| format!("Backing up {}", path.display()))?;
            backups.push(backup.display().to_string());
        }
        replace(&path, text.as_bytes())?;
    }
    let mut connection = status(agent, &path, command);
    ensure!(connection.state == State::Connected, "{} did not take the Nuzky entry", path.display());
    connection.backups = backups;
    Ok(connection)
}

fn resolved(path: &Path) -> Result<PathBuf> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            dunce::canonicalize(path).with_context(|| format!("Following the link {}", path.display()))
        }
        _ => Ok(path.to_path_buf()),
    }
}

fn read(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Reading {}", path.display())),
    }
}

/// The new file next to the old one, then renamed over it, with the old file's permissions; a new
/// file is private, since agent configs can hold tokens.
fn replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path.parent().context("Config has no folder")?;
    std::fs::create_dir_all(directory).with_context(|| format!("Creating {}", directory.display()))?;
    let part = directory.join(format!(
        ".{}.nuzky-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&part)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).map_or(0o600, |m| m.permissions().mode() & 0o7777);
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&part, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result.with_context(|| format!("Writing {}", path.display()))
}

/// `runtime_dir`: this app's socket is under XDG_RUNTIME_DIR, so a Codex entry that does not pass it on runs nothing that reaches it.
fn entry(agent: Agent, text: Option<&str>, runtime_dir: bool) -> Result<Option<Command>> {
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else { return Ok(None) };
    let found = match agent {
        Agent::ClaudeCode | Agent::Antigravity => {
            let config: Value = serde_json::from_str(text).context("Not valid JSON")?;
            ensure!(config.is_object(), "Not a JSON object");
            config.get("mcpServers").and_then(|s| s.get(NAME)).map(|e| {
                if agent == Agent::Antigravity && e["disabled"] == true {
                    (None, None)
                } else {
                    (e.get("command").cloned(), e.get("args").cloned())
                }
            })
        }
        Agent::Codex => {
            let config: toml_edit::DocumentMut = text.parse().context("Not valid TOML")?;
            config.get("mcp_servers").and_then(|s| s.get(NAME)).map(|e| {
                let value = |key| e.get(key).and_then(|i| i.as_value()).map(toml_to_json);
                let passes_runtime_dir = value("env_vars")
                    .and_then(|v| v.as_array().map(|vars| vars.iter().any(|v| v == RUNTIME_DIR)))
                    .unwrap_or(false);
                // Codex does not start a server turned off, so that entry runs nothing.
                match e.get("enabled").and_then(|i| i.as_bool()) {
                    Some(false) => (None, None),
                    _ if runtime_dir && !passes_runtime_dir => (None, None),
                    _ => (value("command"), value("args")),
                }
            })
        }
    };
    Ok(found.map(|(program, args)| Command {
        program: program.and_then(|p| p.as_str().map(String::from)).unwrap_or_default(),
        args: args
            .and_then(|a| a.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()))
            .unwrap_or_default(),
    }))
}

fn toml_to_json(value: &toml_edit::Value) -> Value {
    match value {
        toml_edit::Value::String(s) => Value::String(s.value().clone()),
        toml_edit::Value::Array(a) => Value::Array(a.iter().map(toml_to_json).collect()),
        _ => Value::Null,
    }
}

/// Claude Code and Antigravity CLI keep user-wide servers under `mcpServers` in a JSON file they
/// also write themselves. Every other key keeps its exact text: only that object is rebuilt.
fn with_json_entry(text: Option<&str>, server: &Value) -> Result<String> {
    let mut top = match text.filter(|t| !t.trim().is_empty()) {
        Some(text) => serde_json::from_str::<Entries>(text).context("Not valid JSON")?.0,
        None => Vec::new(),
    };
    let servers = top.iter().position(|(key, _)| key == "mcpServers");
    let mut inner = match servers {
        Some(i) => serde_json::from_str::<Entries>(top[i].1.get()).context("mcpServers is not an object")?.0,
        None => Vec::new(),
    };
    let ours = RawValue::from_string(indent(&serde_json::to_string_pretty(server)?, "    "))?;
    match inner.iter().position(|(key, _)| key == NAME) {
        Some(i) => inner[i].1 = ours,
        None => inner.push((NAME.into(), ours)),
    }
    let rebuilt = RawValue::from_string(object(&inner, "  ")?)?;
    match servers {
        Some(i) => top[i].1 = rebuilt,
        None => top.push(("mcpServers".into(), rebuilt)),
    }
    Ok(object(&top, "")? + "\n")
}

fn with_permissions(text: Option<&str>) -> Result<Option<String>> {
    let mut top = match text {
        Some(text) => serde_json::from_str::<Entries>(text).context("Not valid JSON")?.0,
        None => Vec::new(),
    };
    let permissions = top.iter().position(|(key, _)| key == "permissions");
    let mut inner = match permissions {
        Some(i) => serde_json::from_str::<Entries>(top[i].1.get()).context("permissions is not an object")?.0,
        None => Vec::new(),
    };
    let allow = inner.iter().position(|(key, _)| key == "allow");
    let rule = "mcp(nuzky/*)";
    match allow {
        Some(i) => {
            let raw = inner[i].1.get();
            let rules: Vec<String> =
                serde_json::from_str(raw).context("permissions.allow is not an array of strings")?;
            if rules.iter().any(|r| r == rule) {
                return Ok(None);
            }
            inner[i].1 = RawValue::from_string(format!(
                "{}{}{}]",
                &raw[..raw.len() - 1],
                if rules.is_empty() { "" } else { ", " },
                serde_json::to_string(rule)?
            ))?;
        }
        None => inner
            .push(("allow".into(), RawValue::from_string(indent(&serde_json::to_string_pretty(&[rule])?, "    "))?)),
    }
    let rebuilt = RawValue::from_string(object(&inner, "  ")?)?;
    match permissions {
        Some(i) => top[i].1 = rebuilt,
        None => top.push(("permissions".into(), rebuilt)),
    }
    Ok(Some(object(&top, "")? + "\n"))
}

fn indent(text: &str, by: &str) -> String {
    text.replace('\n', &format!("\n{by}"))
}

fn object(entries: &[(String, Box<RawValue>)], outer: &str) -> Result<String> {
    if entries.is_empty() {
        return Ok("{}".into());
    }
    let lines = entries
        .iter()
        .map(|(key, value)| Ok(format!("{outer}  {}: {}", serde_json::to_string(key)?, value.get())))
        .collect::<Result<Vec<_>>>()?;
    Ok(format!("{{\n{}\n{outer}}}", lines.join(",\n")))
}

/// A JSON object's members in file order, each value as its original text.
struct Entries(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for Entries {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Entries;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> std::result::Result<Entries, M::Error> {
                let mut entries: Vec<(String, Box<RawValue>)> = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, Box<RawValue>>()? {
                    // JSON parsers keep the last of repeated keys; so does this.
                    entries.retain(|(k, _)| *k != key);
                    entries.push((key, value));
                }
                Ok(Entries(entries))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

/// Codex keeps servers as `[mcp_servers.<name>]` tables in `config.toml`; editing the document
/// keeps every comment and the formatting of the rest.
fn with_codex_entry(text: Option<&str>, command: &Command) -> Result<String> {
    let mut config: toml_edit::DocumentMut = text.unwrap_or_default().parse().context("Not valid TOML")?;
    let servers = config.entry("mcp_servers").or_insert_with(|| {
        let mut table = toml_edit::Table::new();
        table.set_implicit(true);
        toml_edit::Item::Table(table)
    });
    let servers = servers.as_table_like_mut().context("mcp_servers is not a table")?;
    let mut ours = toml_edit::Table::new();
    ours.insert("command", toml_edit::value(command.program.as_str()));
    ours.insert("args", toml_edit::value(command.args.iter().map(String::as_str).collect::<toml_edit::Array>()));
    ours.insert("env_vars", toml_edit::value(toml_edit::Array::from_iter([RUNTIME_DIR])));
    servers.insert(NAME, toml_edit::Item::Table(ours));
    Ok(config.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setup_text_names_where_the_entry_goes_in_the_agents_shape() {
        let text = setup(Setup::Opencode, &command()).unwrap();
        assert!(text.starts_with("Nuzky's MCP server lets an agent edit the video open in Nuzky, live."), "{text}");
        let json = text
            .split_once(":\n\n{")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.rsplit_once("\n\n2."))
            .map(|(json, _)| json);
        let json: Value = serde_json::from_str(&format!("{{{}", json.unwrap().trim())).unwrap();
        assert_eq!(
            json["mcp"][NAME],
            serde_json::json!({
                "type": "local",
                "command": [command().program, "mcp", "--current", "--allow-write"],
                "enabled": true,
                "environment": {},
            })
        );
        assert!(text.contains("Then tell OpenCode:\n\nNuzky is the video editor"), "{text}");
        // The other agents keep the `mcpServers` shape.
        let sha = setup(Setup::Other, &command()).unwrap();
        assert!(sha.contains(r#""mcpServers""#) && !sha.contains(r#""mcp""#), "{sha}");
    }

    #[cfg(unix)]
    #[test]
    fn claude_verification_fields_keep_status_scope_and_issue() {
        for status in [
            "✔ Connected",
            "✓ Connected",
            "✗ Failed to connect",
            "✗ Connection error",
            "! Connected · tools fetch failed",
            "Pending approval",
            "Rejected",
            "Disabled",
        ] {
            let text = format!(
                "nuzky:\n  Scope: User config (available in all your projects)\n  Status: {status}\n  Command: nuzky\n  Issue: Missing executable\n"
            );
            assert_eq!(
                claude_fields(&text),
                (status, "User config (available in all your projects)", "Missing executable")
            );
        }
        assert_eq!(claude_fields("No status here"), ("", "", ""));
        assert_eq!(claude_fields("  Status: ✓ Connected\n"), ("✓ Connected", "", ""));
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_server_is_explained_without_its_code() {
        assert_eq!(
            server_issue("APP_NOT_RUNNING: no project is open in Nuzky. Ask the user to open one, then call again"),
            ": it couldn't reach this Nuzky"
        );
        assert_eq!(server_issue("UNAUTHORIZED: endpoint rejected the token"), ": endpoint rejected the token");
        assert_eq!(server_issue("Missing executable."), ": Missing executable");
        assert_eq!(server_issue(""), "");
    }

    #[cfg(unix)]
    #[test]
    fn codex_verification_reads_stdio_and_rejects_disabled_or_other_transports() {
        let text = br#"{"enabled":true,"transport":{"type":"stdio","command":"/opt/Nuzky/nuzky-app","args":["mcp","--current"],"env":{"A":"b"},"env_vars":["XDG_RUNTIME_DIR"],"cwd":"/home/me"}}"#;
        let server = codex_server(text).unwrap();
        assert_eq!(server.command, "/opt/Nuzky/nuzky-app");
        assert_eq!(server.args, ["mcp", "--current"]);
        assert_eq!(server.env.unwrap()["A"], "b");
        assert_eq!(server.env_vars.unwrap(), [RUNTIME_DIR]);
        assert_eq!(server.cwd.unwrap(), Path::new("/home/me"));
        let nulls = br#"{"enabled":true,"transport":{"type":"stdio","command":"nuzky","args":[],"env":null,"env_vars":[],"cwd":null}}"#;
        assert!(codex_server(nulls).unwrap().env.is_none());
        let disabled = String::from_utf8_lossy(text).replace("\"enabled\":true", "\"enabled\":false");
        assert_eq!(
            codex_server(disabled.as_bytes()).unwrap_err(),
            "Codex has Nuzky turned off. Press Connect to turn it on."
        );
        assert_eq!(
            codex_server(br#"{"enabled":true,"transport":{"type":"streamable_http","url":"https://example.com"}}"#)
                .unwrap_err(),
            "Codex's server named nuzky isn't this Nuzky. Press Connect to replace it."
        );
        assert!(codex_server(b"not json").is_err());
    }

    fn command() -> Command {
        Command {
            program: "/opt/Nuzky/nuzky-app".into(),
            args: vec!["mcp".into(), "--current".into(), "--allow-write".into()],
        }
    }

    fn dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nuzky-connect-{}", nuzky_engine::edit::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const CLAUDE: &str = "{\n  \"numStartups\": 42,\n  \"projects\": {\n    \"/home/me\": {\n      \"allowedTools\": []\n    }\n  },\n  \"mcpServers\": {\n    \"other\": {\n      \"type\": \"stdio\",\n      \"command\": \"other-server\"\n    }\n  },\n  \"theme\": \"dark\"\n}\n";

    #[test]
    fn claude_code_gets_only_the_nuzky_entry_and_keeps_every_other_byte() {
        let dir = dir();
        let path = dir.join(".claude.json");
        std::fs::write(&path, CLAUDE).unwrap();
        assert_eq!(status(Agent::ClaudeCode, &path, &command()).state, State::Missing);
        let done = connect(Agent::ClaudeCode, &path, None, &command(), 7).unwrap();
        assert_eq!(done.state, State::Connected);
        assert_eq!(std::fs::read_to_string(dir.join(".claude.json.nuzky-backup-7")).unwrap(), CLAUDE);
        let text = std::fs::read_to_string(&path).unwrap();
        // Everything before the new entry is the same text, and so is everything after it.
        let (head, tail) = CLAUDE.split_once("\n  },\n  \"theme\"").unwrap();
        assert!(text.starts_with(head) && text.ends_with(&format!("\n  }},\n  \"theme\"{tail}")), "{text}");
        let config: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(config["mcpServers"]["other"]["command"], "other-server");
        assert_eq!(config["mcpServers"][NAME]["args"], serde_json::json!(["mcp", "--current", "--allow-write"]));
        assert_eq!(config["numStartups"], 42);
        // Connecting again changes nothing and makes no backup.
        let again = connect(Agent::ClaudeCode, &path, None, &command(), 8).unwrap();
        assert!(again.backups.is_empty() && !dir.join(".claude.json.nuzky-backup-8").exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn antigravity_gets_only_the_nuzky_entry_and_keeps_every_other_byte() {
        let dir = dir();
        let path = dir.join("mcp_config.json");
        std::fs::write(&path, CLAUDE).unwrap();
        assert_eq!(status(Agent::Antigravity, &path, &command()).state, State::Missing);
        let settings = dir.join("settings.json");
        let original = r#"{
  "theme": { "name": "dark" },
  "permissions": {
    "allow": ["mcp(other/\u0074ool)", "read /my files"],
    "deny": [ "shell(rm *)" ],
    "ask": ["mcp(nuzky/delete)"]
  },
  "other": [1,  2]
}
"#;
        std::fs::write(&settings, original).unwrap();
        let done = connect(Agent::Antigravity, &path, Some(&settings), &command(), 7).unwrap();
        assert_eq!(done.state, State::Connected);
        assert_eq!(std::fs::read_to_string(&done.backups[0]).unwrap(), CLAUDE);
        assert_eq!(std::fs::read_to_string(&done.backups[1]).unwrap(), original);
        let allowed = original.replace("\"read /my files\"]", "\"read /my files\", \"mcp(nuzky/*)\"]");
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), allowed);
        let shown = done.shown_from(&dir);
        assert_eq!(shown.backups, ["~/mcp_config.json.nuzky-backup-7", "~/settings.json.nuzky-backup-7"]);
        let modified = std::fs::metadata(&settings).unwrap().modified().unwrap();
        let again = connect(Agent::Antigravity, &path, Some(&settings), &command(), 8).unwrap();
        assert!(again.backups.is_empty());
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), allowed);
        assert_eq!(std::fs::metadata(&settings).unwrap().modified().unwrap(), modified);
        assert!(!dir.join("settings.json.nuzky-backup-8").exists());
        let text = std::fs::read_to_string(&path).unwrap();
        let (head, tail) = CLAUDE.split_once("\n  },\n  \"theme\"").unwrap();
        assert!(text.starts_with(head) && text.ends_with(&format!("\n  }},\n  \"theme\"{tail}")), "{text}");
        let config: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            config["mcpServers"][NAME],
            serde_json::json!({
                "command": command().program, "args": command().args,
            })
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_creates_a_private_config_with_its_folders() {
        use std::os::unix::fs::PermissionsExt;
        let dir = dir();
        let path = Agent::Antigravity.config(&dir);
        assert_eq!(path, dir.join(".gemini/config/mcp_config.json"));
        let settings = Agent::Antigravity.settings(&dir).unwrap();
        assert_eq!(settings, dir.join(".gemini/antigravity-cli/settings.json"));
        let done = connect(Agent::Antigravity, &path, Some(&settings), &command(), 1).unwrap();
        assert_eq!(done.state, State::Connected);
        assert!(done.backups.is_empty());
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&settings).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(
            std::fs::read_to_string(settings).unwrap(),
            "{\n  \"permissions\": {\n    \"allow\": [\n      \"mcp(nuzky/*)\"\n    ]\n  }\n}\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn antigravity_checks_settings_before_writing_either_file() {
        let dir = dir();
        let path = dir.join("mcp_config.json");
        let settings = dir.join("settings.json");
        for invalid in [
            "// comment\n{}",
            "{\"permissions\": {},}",
            "",
            "[]",
            "{\"permissions\":null}",
            "{\"permissions\":{\"allow\":false}}",
            "{\"permissions\":{\"allow\":[1]}}",
        ] {
            std::fs::write(&settings, invalid).unwrap();
            assert!(connect(Agent::Antigravity, &path, Some(&settings), &command(), 2).is_err());
            assert!(!path.exists());
            std::fs::write(&path, CLAUDE).unwrap();
            let error = connect(Agent::Antigravity, &path, Some(&settings), &command(), 2).unwrap_err().to_string();
            assert!(
                error.contains(&settings.display().to_string())
                    && error.contains("left it alone")
                    && error.contains("agy can't start"),
                "{error}"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), CLAUDE);
            assert_eq!(std::fs::read_to_string(&settings).unwrap(), invalid);
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
            std::fs::remove_file(&path).unwrap();
        }
        std::fs::remove_file(&settings).unwrap();
        std::fs::create_dir(&settings).unwrap();
        assert!(connect(Agent::Antigravity, &path, Some(&settings), &command(), 2).is_err());
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_writes_settings_where_the_symlink_points() {
        let dir = dir();
        let path = dir.join("mcp_config.json");
        let real = dir.join("real.json");
        let settings = dir.join("settings.json");
        std::fs::write(&real, "{}").unwrap();
        std::os::unix::fs::symlink(&real, &settings).unwrap();
        let done = connect(Agent::Antigravity, &path, Some(&settings), &command(), 3).unwrap();
        assert!(std::fs::symlink_metadata(&settings).unwrap().file_type().is_symlink());
        // On macOS the temporary folder is reached through the /var symlink, and the backup goes beside the real file.
        assert_eq!(
            done.backups,
            [dunce::canonicalize(&dir).unwrap().join("real.json.nuzky-backup-3").display().to_string()]
        );
        assert_eq!(std::fs::read_to_string(&done.backups[0]).unwrap(), "{}");
        assert!(std::fs::read_to_string(&real).unwrap().contains("mcp(nuzky/*)"));
        std::fs::write(&real, "not json").unwrap();
        assert_eq!(status(Agent::Antigravity, &path, &command()).state, State::Connected);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn antigravity_leaves_jsonc_alone() {
        let dir = dir();
        let path = dir.join("mcp_config.json");
        let jsonc = "{\n  // My servers\n  \"mcpServers\": {}\n}\n";
        std::fs::write(&path, jsonc).unwrap();
        assert_eq!(status(Agent::Antigravity, &path, &command()).state, State::Unreadable);
        assert!(connect(Agent::Antigravity, &path, None, &command(), 2).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), jsonc);
        assert!(!path.with_file_name("mcp_config.json.nuzky-backup-2").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn antigravity_reconnects_a_server_disabled_by_agy() {
        let dir = dir();
        let path = dir.join("mcp_config.json");
        let disabled = r#"{
  "mcpServers": {
    "nuzky": {
      "args": ["mcp", "--current", "--allow-write"],
      "command": "/opt/Nuzky/nuzky-app",
      "disabled": true
    }
  }
}
"#;
        std::fs::write(&path, disabled).unwrap();
        assert_eq!(status(Agent::Antigravity, &path, &command()).state, State::Other);
        let done = connect(Agent::Antigravity, &path, None, &command(), 3).unwrap();
        assert_eq!(done.state, State::Connected);
        assert_eq!(std::fs::read_to_string(&done.backups[0]).unwrap(), disabled);
        assert!(!std::fs::read_to_string(&path).unwrap().contains("disabled"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    const CODEX: &str = "# my settings\nmodel = \"gpt-6.1-sol\" # the best\n\n[mcp_servers.other]\ncommand = \"other\"\nargs = [\"--x\"]\n\n[profiles.fast]\nmodel_reasoning_effort = \"low\"\n";

    #[test]
    fn codex_gets_only_the_nuzky_table_and_keeps_comments() {
        let dir = dir();
        let path = dir.join("config.toml");
        std::fs::write(&path, CODEX).unwrap();
        let done = connect(Agent::Codex, &path, None, &command(), 3).unwrap();
        assert_eq!((done.state, !done.backups.is_empty()), (State::Connected, true));
        let text = std::fs::read_to_string(&path).unwrap();
        // Without the new table the file is exactly what it was, comments included.
        let ours = "[mcp_servers.nuzky]\ncommand = \"/opt/Nuzky/nuzky-app\"\nargs = [\"mcp\", \"--current\", \"--allow-write\"]\nenv_vars = [\"XDG_RUNTIME_DIR\"]\n\n";
        assert!(text.contains(ours), "{text}");
        assert_eq!(text.replace(ours, ""), CODEX);
        // Another Nuzky entry is replaced, after a new backup.
        let moved = Command { program: "/new/nuzky-app".into(), ..command() };
        assert_eq!(status(Agent::Codex, &path, &moved).state, State::Other);
        connect(Agent::Codex, &path, None, &moved, 4).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("[mcp_servers.nuzky]").count(), 1);
        assert!(text.contains("/new/nuzky-app") && !text.contains("/opt/Nuzky"));
        // An entry turned off in Codex runs nothing, so it is turned back on by connecting again.
        std::fs::write(&path, text.replace("[mcp_servers.nuzky]\n", "[mcp_servers.nuzky]\nenabled = false\n")).unwrap();
        assert_eq!(status(Agent::Codex, &path, &moved).state, State::Other);
        assert_eq!(connect(Agent::Codex, &path, None, &moved, 5).unwrap().state, State::Connected);
        assert!(!std::fs::read_to_string(&path).unwrap().contains("enabled"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_codex_entry_that_does_not_pass_on_the_runtime_dir_runs_nothing_where_the_socket_is_under_it() {
        let old = "[mcp_servers.nuzky]\ncommand = \"/opt/Nuzky/nuzky-app\"\nargs = [\"mcp\", \"--current\", \"--allow-write\"]\n";
        assert_eq!(entry(Agent::Codex, Some(old), false).unwrap(), Some(command()));
        assert_ne!(entry(Agent::Codex, Some(old), true).unwrap(), Some(command()));
        let new = with_codex_entry(Some(old), &command()).unwrap();
        assert_eq!(entry(Agent::Codex, Some(&new), true).unwrap(), Some(command()));
    }

    #[cfg(unix)]
    #[test]
    fn missing_files_are_created_private_and_broken_ones_are_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = dir();
        let codex = dir.join("codex/config.toml");
        connect(Agent::Codex, &codex, None, &command(), 1).unwrap();
        assert_eq!(std::fs::metadata(&codex).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(std::fs::read_to_string(&codex).unwrap().starts_with("[mcp_servers.nuzky]"));
        let claude = dir.join(".claude.json");
        connect(Agent::ClaudeCode, &claude, None, &command(), 1).unwrap();
        let config: Value = serde_json::from_str(&std::fs::read_to_string(&claude).unwrap()).unwrap();
        assert_eq!(config["mcpServers"][NAME]["command"], "/opt/Nuzky/nuzky-app");
        std::fs::write(&claude, "{ not json").unwrap();
        assert_eq!(status(Agent::ClaudeCode, &claude, &command()).state, State::Unreadable);
        assert!(connect(Agent::ClaudeCode, &claude, None, &command(), 2).is_err());
        assert_eq!(std::fs::read_to_string(&claude).unwrap(), "{ not json");
        assert!(!dir.join(".claude.json.nuzky-backup-2").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_an_executable_inside_its_appimage_names_the_appimage() {
        let image = Some(PathBuf::from("/home/me/Nuzky.AppImage"));
        let mount = Some(PathBuf::from("/tmp/.mount_Nuzky.Ab12Cd"));
        let inside = Path::new("/tmp/.mount_Nuzky.Ab12Cd/usr/bin/nuzky-app");
        assert_eq!(app_image(inside, mount.clone(), image.clone()), image);
        // A dev build started from another AppImage's terminal inherits its variables.
        let other = Some(PathBuf::from("/tmp/.mount_T3-Cod"));
        assert_eq!(app_image(Path::new("/src/target/debug/nuzky-app"), other, image.clone()), None);
        assert_eq!(app_image(inside, None, image), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_config_is_written_where_the_link_points() {
        let dir = dir();
        std::fs::create_dir_all(dir.join("dotfiles")).unwrap();
        let real = dir.join("dotfiles/claude.json");
        std::fs::write(&real, CLAUDE).unwrap();
        let link = dir.join(".claude.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        connect(Agent::ClaudeCode, &link, None, &command(), 5).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(status(Agent::ClaudeCode, &link, &command()).state, State::Connected);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

use crate::discover::AgentId;
use crate::event::{AgentEvent, ErrorCode};
use crate::{claude, codex};
use anyhow::{Context, Result, bail};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Nuzky's MCP bridge as the agent should start it.
#[derive(Clone, Debug)]
pub struct McpServer {
    pub command: PathBuf,
    pub args: Vec<String>,
    /// Passed explicitly: an agent may give its MCP servers only a few variables of its own.
    pub env: Vec<(String, String)>,
}

/// What Nuzky tells the agent about where it is, on top of the MCP server's own instructions.
pub(crate) const SYSTEM: &str = "You are the AI panel inside Nuzky, a video editor. The user watches the open project change as you work. \
Use only the nuzky tools and follow the Nuzky instructions you were given. When they carry the creator's editing style, \
it decides how you cut, caption and zoom unless the user says otherwise. One request is one run: begin_run once before \
changing anything and end_run with keep when you are done. Answer briefly in the user's language and say what you changed. \
For a job, use job action wait and call it again while running. Tell the user once what runs and roughly how long \
it takes from remaining_s. After that, write only when something changes. Never ask whether to keep waiting. \
When the user has to decide something, call suggest_options as the last thing in your turn. The [Nuzky] lines at the end \
of a message say what the user had selected when they sent it.";

/// What the user sends: their words, and the pictures they attached as JPEG files Nuzky prepared.
#[derive(Clone, Debug, Default)]
pub struct Message {
    pub text: String,
    pub images: Vec<PathBuf>,
}

impl From<&str> for Message {
    fn from(text: &str) -> Self {
        Message { text: text.to_owned(), images: Vec::new() }
    }
}

impl Message {
    /// Each image as base64, for the agents that take pictures inline.
    pub(crate) fn base64_images(&self) -> Result<Vec<String>> {
        use base64::Engine as _;
        self.images
            .iter()
            .map(|path| {
                let bytes =
                    std::fs::read(path).with_context(|| format!("Reading the attached picture {}", path.display()))?;
                Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct TurnRequest {
    pub agent: AgentId,
    pub exe: PathBuf,
    /// The conversation a later turn resumes. Claude also starts a new one under this id; Codex picks
    /// its own and reports it in `AgentEvent::Session`.
    pub session: String,
    pub resume: bool,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub prompt: Message,
    /// An empty folder of Nuzky's own, so no project instructions or settings load from it.
    pub cwd: PathBuf,
    pub mcp: McpServer,
}

/// What a line of the agent's output means for the turn.
pub(crate) enum Step {
    Event(AgentEvent),
    /// Ends the turn with this error; Nuzky stops the agent.
    Fail(ErrorCode, String),
    /// The agent's own closing error, which a stopped turn also produces.
    ResultError(String),
    /// Claude finished answering what it was sent; a message sent later starts no more work in this turn.
    Answered,
}

enum Parser {
    /// Claude keeps the conversation under Nuzky's id once a turn ends without an error.
    Claude(claude::Parser, String),
    Codex(codex::Parser),
}

impl Parser {
    fn line(&mut self, line: &str) -> Vec<Step> {
        match self {
            Parser::Claude(p, _) => p.line(line),
            Parser::Codex(p) => p.line(line),
        }
    }

    fn session(&self, end: &AgentEvent) -> Option<String> {
        match self {
            Parser::Claude(_, id) => matches!(end, AgentEvent::Done { .. }).then(|| id.clone()),
            Parser::Codex(p) => p.session(),
        }
    }
}

/// How long a stopped agent gets to finish before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(2);

/// Where messages the user sends while the agent works go; None once the turn takes no more.
pub(crate) enum Inbox {
    /// Claude Code reads them on its input and takes each at its next step.
    Claude(Arc<Mutex<Option<ChildStdin>>>),
    /// ChatGPT's turn adds them to the conversation before it asks again.
    ChatGpt(Arc<Mutex<Option<Vec<Message>>>>),
    /// Codex reads one message per turn; true once it has kept this turn's, so a stop for the next loses nothing.
    Codex(Arc<AtomicBool>),
}

/// One message to the agent: an installed CLI running as its own process group, or ChatGPT running in Nuzky.
pub struct Turn {
    /// The CLI's process group, on Windows its process tree; None for ChatGPT, whose turn ends itself once it sees `stopping`.
    pub(crate) pid: Option<u32>,
    pub(crate) stopping: Arc<AtomicBool>,
    pub(crate) finished: Arc<AtomicBool>,
    pub(crate) inbox: Inbox,
}

/// Variables of a parent Claude Code session (Nuzky started from one, as in development),
/// which would connect the agent to that session instead of starting its own.
fn parent_session_var(name: &str) -> bool {
    name == "CLAUDECODE"
        || name == "CLAUDE_PID"
        || name.starts_with("CLAUDE_CODE_")
        || name.starts_with("CLAUDE_AGENT_SDK")
}

fn failure_code(resume: bool, message: &str) -> ErrorCode {
    if resume
        && (message.contains("No conversation found with session ID")
            || message.contains("no rollout found for thread id"))
    {
        ErrorCode::SessionGone
    } else {
        ErrorCode::AgentFailed
    }
}

impl Turn {
    /// Starts the turn; `emit` gets its events from a reader thread, ending with `Done` or `Error`.
    pub fn start(req: TurnRequest, mut emit: impl FnMut(AgentEvent) + Send + 'static) -> Result<Turn> {
        std::fs::create_dir_all(&req.cwd).context("Creating the AI panel's folder")?;
        // Codex reads its pictures from files itself.
        let first = if req.agent == AgentId::Claude { claude::message(&req.prompt)? } else { req.prompt.text.clone() };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&req.cwd, std::fs::Permissions::from_mode(0o700))
                .context("Securing the AI panel's folder")?;
        }
        let mut cmd = match req.agent {
            AgentId::Claude => claude::command(&req),
            AgentId::Codex => codex::command(&req),
            AgentId::ChatGpt => bail!("ChatGPT runs in Nuzky, not as a process"),
            AgentId::Antigravity => bail!("Run Antigravity CLI in your terminal and connect it to Nuzky."),
        };
        cmd.current_dir(&req.cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_some_and(parent_session_var) {
                cmd.env_remove(name);
            }
        }
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        #[cfg(windows)]
        crate::discover::no_window(&mut cmd);
        let mut child = cmd.spawn().with_context(|| format!("NOT_INSTALLED: could not start {}", req.exe.display()))?;
        let pid = child.id();
        let stopping = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));

        let mut stdin = child.stdin.take().context("Agent input")?;
        let inbox = if req.agent == AgentId::Claude {
            // Its input stays open for what the user sends while it works.
            let _ = stdin.write_all(first.as_bytes());
            Arc::new(Mutex::new(Some(stdin)))
        } else {
            std::thread::spawn(move || {
                // Closing input tells Codex that the message is complete.
                let _ = stdin.write_all(first.as_bytes());
            });
            Arc::new(Mutex::new(None))
        };
        let open = inbox.clone();
        let kept = Arc::new(AtomicBool::new(false));
        let kept_now = kept.clone();
        let tail = Arc::new(Mutex::new(VecDeque::<String>::new()));
        let stderr = child.stderr.take().context("Agent errors")?;
        let errors = tail.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(std::result::Result::ok) {
                let mut tail = errors.lock().unwrap();
                if tail.len() == 8 {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        });
        let stdout = child.stdout.take().context("Agent output")?;
        let (stop_flag, done_flag) = (stopping.clone(), finished.clone());
        let mut parser = match req.agent {
            AgentId::Claude => Parser::Claude(claude::Parser::default(), req.session),
            AgentId::Codex => Parser::Codex(codex::Parser::new(pid)),
            AgentId::ChatGpt | AgentId::Antigravity => unreachable!("refused above"),
        };
        std::thread::spawn(move || {
            let mut failed: Option<(ErrorCode, String)> = None;
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                // Lines can be long (whole tool results); read_line keeps them whole.
                match reader.by_ref().read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                for step in parser.line(line.trim_end()) {
                    match step {
                        Step::Event(e) if failed.is_none() => emit(e),
                        Step::Fail(code, message) if failed.is_none() => {
                            failed = Some((code, message));
                            signal(pid, false);
                        }
                        Step::ResultError(message) if failed.is_none() && !stop_flag.load(Ordering::Acquire) => {
                            failed = Some((ErrorCode::AgentFailed, message));
                        }
                        // Claude answers what is already on its input after this, then exits.
                        Step::Answered => drop(open.lock().unwrap().take()),
                        _ => {}
                    }
                }
                if let Parser::Codex(p) = &parser
                    && p.session().is_some()
                {
                    kept_now.store(true, Ordering::Release);
                }
            }
            let status = child.wait();
            done_flag.store(true, Ordering::Release);
            let end = if let Some((code, message)) = failed {
                AgentEvent::Error { code, message }
            } else if stop_flag.load(Ordering::Acquire) {
                AgentEvent::Done { stopped: true }
            } else if status.as_ref().is_ok_and(|s| s.success()) {
                AgentEvent::Done { stopped: false }
            } else {
                let tail = tail.lock().unwrap().iter().cloned().collect::<Vec<_>>().join("\n");
                let code = status.ok().and_then(|s| s.code()).map_or("a signal".to_owned(), |c| format!("code {c}"));
                AgentEvent::Error {
                    code: ErrorCode::AgentFailed,
                    message: if tail.trim().is_empty() { format!("The agent exited with {code}.") } else { tail },
                }
            };
            let end = match end {
                AgentEvent::Error { code: ErrorCode::AgentFailed, message } => {
                    AgentEvent::Error { code: failure_code(req.resume, &message), message }
                }
                end => end,
            };
            if let Some(id) = parser.session(&end) {
                emit(AgentEvent::Session { id });
            }
            emit(end);
        });
        let inbox = if req.agent == AgentId::Claude { Inbox::Claude(inbox) } else { Inbox::Codex(kept) };
        Ok(Turn { pid: Some(pid), stopping, finished, inbox })
    }

    /// Gives the working agent another message, which it takes at its next step. False when it cannot
    /// take one: Codex reads only one message per turn, and a turn that is ending or stopping takes none.
    pub fn steer(&self, prompt: &Message) -> bool {
        if !self.running() || self.stopping.load(Ordering::Acquire) {
            return false;
        }
        match &self.inbox {
            Inbox::Claude(stdin) => stdin.lock().unwrap().as_mut().is_some_and(|stdin| {
                claude::message(prompt).is_ok_and(|line| stdin.write_all(line.as_bytes()).is_ok())
            }),
            Inbox::ChatGpt(queue) => queue.lock().unwrap().as_mut().map(|queue| queue.push(prompt.clone())).is_some(),
            Inbox::Codex(_) => false,
        }
    }

    /// Whether the agent can stop for a message it cannot take and go on with it in the next turn without
    /// losing this one: Codex, once it has kept this turn's message.
    pub fn resumable(&self) -> bool {
        matches!(&self.inbox, Inbox::Codex(kept) if kept.load(Ordering::Acquire))
    }

    /// Asks the agent to stop, and kills it and its MCP bridge if it has not after a short grace.
    pub fn stop(&self) {
        if self.finished.load(Ordering::Acquire) || self.stopping.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Inbox::Claude(stdin) = &self.inbox {
            // A message still on its input would otherwise be answered after the stop.
            drop(stdin.lock().unwrap().take());
        }
        let Some(pid) = self.pid else { return };
        signal(pid, false);
        let finished = self.finished.clone();
        std::thread::spawn(move || {
            std::thread::sleep(STOP_GRACE);
            if !finished.load(Ordering::Acquire) {
                signal(pid, true);
            }
        });
    }

    pub fn running(&self) -> bool {
        !self.finished.load(Ordering::Acquire)
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        self.stop();
    }
}

/// SIGINT lets the agent record the interrupted turn so the next one can resume; SIGKILL is the fallback.
/// The whole process group gets it, so the MCP bridge the agent started goes too. Windows has no SIGINT for a
/// child without a console window, so there the agent and everything it started end at once. Like a signal, that
/// happens a moment later: Stop and edit ends the agent's run itself first, before its bridge goes.
fn signal(pid: u32, kill: bool) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), if kill { libc::SIGKILL } else { libc::SIGINT });
    }
    #[cfg(windows)]
    if !kill {
        std::thread::spawn(move || crate::discover::kill_tree(pid));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_effort_flags_are_optional_on_new_and_resumed_turns() {
        for agent in [AgentId::Claude, AgentId::Codex] {
            for resume in [false, true] {
                for model in [None, Some("test-model".to_owned())] {
                    for effort in [None, Some("high".to_owned())] {
                        let req = TurnRequest {
                            agent,
                            exe: "unused".into(),
                            session: "session".into(),
                            resume,
                            model: model.clone(),
                            effort: effort.clone(),
                            prompt: Message::default(),
                            cwd: PathBuf::new(),
                            mcp: McpServer { command: "nuzky".into(), args: vec![], env: vec![] },
                        };
                        let command = match agent {
                            AgentId::Claude => claude::command(&req),
                            AgentId::Codex => codex::command(&req),
                            AgentId::ChatGpt | AgentId::Antigravity => unreachable!(),
                        };
                        let args: Vec<_> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
                        let model_flag = if agent == AgentId::Claude { "--model" } else { "-m" };
                        assert_eq!(args.windows(2).any(|a| a == [model_flag, "test-model"]), model.is_some());
                        let effort_args = if agent == AgentId::Claude {
                            ["--effort", "high"]
                        } else {
                            ["-c", "model_reasoning_effort=\"high\""]
                        };
                        assert_eq!(args.windows(2).any(|a| a == effort_args), effort.is_some());
                    }
                }
            }
        }
    }

    #[test]
    fn session_gone_only_classifies_missing_resumed_conversations() {
        for message in ["No conversation found with session ID: abc", "error: no rollout found for thread id abc"] {
            assert_eq!(failure_code(true, message), ErrorCode::SessionGone);
            assert_eq!(failure_code(false, message), ErrorCode::AgentFailed);
        }
        assert_eq!(failure_code(true, "connection failed"), ErrorCode::AgentFailed);
        assert_eq!(serde_json::to_value(ErrorCode::SessionGone).unwrap(), "SESSION_GONE");
    }
}

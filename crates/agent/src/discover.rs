use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentId {
    Claude,
    Codex,
    /// Runs in Nuzky on the user's ChatGPT sign-in, so there is nothing to install or find.
    #[serde(rename = "chatgpt")]
    ChatGpt,
}

impl AgentId {
    pub fn name(self) -> &'static str {
        match self {
            AgentId::Claude => "Claude Code",
            AgentId::Codex => "Codex",
            AgentId::ChatGpt => "ChatGPT",
        }
    }

    /// The executable, and the variable that points the panel at another one, such as the fake agent of the E2E tests.
    fn program(self) -> Option<(&'static str, &'static str)> {
        match self {
            AgentId::Claude => Some(("claude", "NUZKY_AGENT_CLAUDE")),
            AgentId::Codex => Some(("codex", "NUZKY_AGENT_CODEX")),
            AgentId::ChatGpt => None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub id: AgentId,
    pub name: &'static str,
    /// The executable, or None when it is not installed.
    pub path: Option<PathBuf>,
    pub version: Option<String>,
}

/// Where installers put these CLIs when the app was started without the user's shell `PATH`,
/// e.g. from a desktop launcher.
fn known_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(home) = home {
        for d in [".local/bin", ".claude/local", ".npm-global/bin", ".bun/bin", ".volta/bin"] {
            dirs.push(home.join(d));
        }
    }
    dirs.extend(["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin"].map(PathBuf::from));
    dirs
}

fn executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn locate(id: AgentId) -> Option<PathBuf> {
    let (program, override_var) = id.program()?;
    if let Some(path) = std::env::var_os(override_var) {
        return Some(PathBuf::from(path)).filter(|p| executable(p));
    }
    let name = if cfg!(windows) { format!("{program}.exe") } else { program.to_owned() };
    let path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    path.into_iter().chain(known_dirs(home.as_deref())).map(|dir| dir.join(&name)).find(|p| executable(p))
}

/// The first version-looking word of `--version`: "2.1.292 (Claude Code)", "codex-cli 0.160.1".
fn version_of(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()) && w.contains('.'))
        .map(str::to_owned)
}

/// Runs a short CLI query, draining both pipes while it runs; None means it timed out.
pub fn run(
    program: &Path,
    args: &[&str],
    env: &[(&str, &str)],
    cwd: Option<&Path>,
    timeout: Duration,
) -> std::io::Result<Option<Output>> {
    let mut command = Command::new(program);
    command.args(args).envs(env.iter().copied()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn()?;
    let read = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.read_to_end(&mut bytes).map(|_| bytes)
        })
    };
    let stdout = read(Box::new(child.stdout.take().unwrap()));
    let stderr = read(Box::new(child.stderr.take().unwrap()));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(Some(status)),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => break Ok(None),
            Err(error) => break Err(error),
        }
    };
    // A CLI's server can still hold its pipes open after the CLI exits.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL)
    };
    let _ = child.kill();
    let _ = child.wait();
    let stdout = stdout.join().unwrap()?;
    let stderr = stderr.join().unwrap()?;
    Ok(status?.map(|status| Output { status, stdout, stderr }))
}

fn version(path: &Path) -> Option<String> {
    let output = run(path, &["--version"], &[], None, Duration::from_secs(5)).ok()??;
    version_of(&String::from_utf8_lossy(&output.stdout))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SignIn {
    SignedIn,
    SignedOut,
    Unknown,
}

pub fn signed_in(id: AgentId, path: &Path) -> SignIn {
    let args: &[&str] = match id {
        AgentId::Claude => &["auth", "status", "--json"],
        AgentId::Codex => &["login", "status"],
        AgentId::ChatGpt => return SignIn::Unknown,
    };
    let Ok(Some(output)) = run(path, args, &[], None, Duration::from_secs(5)) else { return SignIn::Unknown };
    let logged_in = match id {
        AgentId::Claude => serde_json::from_slice::<serde_json::Value>(&output.stdout)
            .ok()
            .and_then(|value| value["loggedIn"].as_bool()),
        AgentId::Codex => match output.status.code() {
            Some(0) => Some(true),
            Some(1) => Some(false),
            _ => None,
        },
        AgentId::ChatGpt => None,
    };
    match logged_in {
        Some(true) => SignIn::SignedIn,
        Some(false) => SignIn::SignedOut,
        None => SignIn::Unknown,
    }
}

/// Finds the installed agent and its version; a missing one has no path.
pub fn find(id: AgentId) -> AgentInfo {
    let path = locate(id);
    let version = path.as_deref().and_then(version);
    AgentInfo { id, name: id.name(), path, version }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_come_from_both_agents_wording() {
        assert_eq!(version_of("2.1.292 (Claude Code)\n").as_deref(), Some("2.1.292"));
        assert_eq!(version_of("codex-cli 0.160.1\n").as_deref(), Some("0.160.1"));
        assert_eq!(version_of("unknown"), None);
    }
}

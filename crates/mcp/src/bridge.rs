//! The same stdio server attaches to the open app or owns a headless session.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use rmcp::model::ContentBlock;
use rmcp::{ServiceExt, model::CallToolResult};
use serde_json::Value;

use crate::{
    Server, catalog,
    tools::{self, Backend},
};

pub(crate) enum Target {
    Local(Arc<Backend>),
    App(Arc<crate::ipc::Remote>),
    Current(Arc<Current>),
}

impl Target {
    pub async fn call(self: Arc<Self>, name: String, args: Value) -> CallToolResult {
        match self.as_ref() {
            Self::Local(backend) => tools::call(backend.clone(), name, args).await,
            Self::App(remote) => {
                let remote = remote.clone();
                answer(tokio::task::spawn_blocking(move || remote.call(name, args)).await)
            }
            Self::Current(current) => {
                let current = current.clone();
                answer(tokio::task::spawn_blocking(move || current.call(name, args)).await)
            }
        }
    }
}

fn answer(result: Result<Result<CallToolResult>, tokio::task::JoinError>) -> CallToolResult {
    match result {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => CallToolResult::error(vec![ContentBlock::text(tools::error_message(format!("{e:#}")))]),
        Err(e) => CallToolResult::error(vec![ContentBlock::text(format!("IPC_FAILED: {e}"))]),
    }
}

/// Set by the AI panel for its bridge: the conversation the app began, which owns the jobs its turns start.
pub const CONVERSATION_VAR: &str = "NUZKY_CONVERSATION";

/// Set by the app's check that an agent's own `--current` command reaches it: the server first hands
/// the app this nonce, and without an app to take it, fails, so the agent's health check fails too.
pub const VERIFY_VAR: &str = "NUZKY_VERIFY";

/// Why `--current` found nothing to attach to.
const NOT_OPEN: &str = "APP_NOT_RUNNING: no project is open in Nuzky. Ask the user to open one, then call again";
const DETACHED: &str = "APP_CLOSED: Nuzky closed the project, quit or turned Agent access off. The next call attaches to the project open in Nuzky then; read get_state before editing";

/// `--current`: the project open in the app, attached at the first call and again whenever the app
/// let go, so opening a project or turning on Agent access needs no restart. Never headless.
pub(crate) struct Current {
    allow_write: bool,
    remote: Mutex<Option<Arc<crate::ipc::Remote>>>,
}

impl Current {
    fn call(&self, name: String, args: Value) -> Result<CallToolResult> {
        let remote = {
            let mut attached = self.remote.lock().unwrap();
            // The project the agent read may be gone, so this call says so instead of acting on another one.
            if attached.take_if(|remote| remote.closed()).is_some() {
                bail!(DETACHED);
            }
            match attached.as_ref() {
                Some(remote) => remote.clone(),
                None => {
                    let project = crate::ipc::current_project()?.context(NOT_OPEN)?;
                    let remote = crate::ipc::Remote::connect(&project, self.allow_write, true)?.context(NOT_OPEN)?;
                    attached.insert(Arc::new(remote)).clone()
                }
            }
        };
        remote.call(name, args).map_err(|error| {
            if !remote.closed() {
                return error;
            }
            // This call already said so; the next one attaches.
            self.remote.lock().unwrap().take_if(|attached| Arc::ptr_eq(attached, &remote));
            anyhow::anyhow!(DETACHED)
        })
    }
}

/// Attaches to the app when it has the project open, else edits it headless.
pub fn run(project: &Path, allow_write: bool, cache: PathBuf) -> Result<()> {
    if let Some(remote) = crate::ipc::Remote::connect(project, allow_write, false)? {
        return serve(Target::App(Arc::new(remote)));
    }
    serve(Target::Local(Arc::new(Backend::open(project, allow_write, cache)?)))
}

fn serve(target: Target) -> Result<()> {
    let target = Arc::new(target);
    let server = Server { target: target.clone(), tools: Arc::new(catalog()?) };
    let runtime =
        tokio::runtime::Builder::new_multi_thread().enable_all().build().context("MCP_FAILED: starting runtime")?;
    let (input, mut writer) = tokio::io::duplex(64 * 1024);
    let (handle, closing) = (runtime.handle().clone(), target.clone());
    // Its own thread: a read the runtime started would keep the runtime from shutting down while stdin stays open.
    std::thread::spawn(move || {
        use std::io::Read;
        use tokio::io::AsyncWriteExt;
        let mut buffer = [0; 8192];
        while let Ok(read @ 1..) = std::io::stdin().read(&mut buffer) {
            if handle.block_on(writer.write_all(&buffer[..read])).is_err() {
                break;
            }
        }
        // rmcp gives the calls in flight 5 s once its input ends; a job wait must end before that.
        match closing.as_ref() {
            Target::Local(backend) => backend.close(),
            Target::App(remote) => remote.close(),
            Target::Current(current) => {
                if let Some(remote) = current.remote.lock().unwrap().as_ref() {
                    remote.close();
                }
            }
        }
    });
    let result = runtime.block_on(async {
        match server.serve((input, tokio::io::stdout())).await {
            Ok(service) => service.waiting().await.context("MCP_FAILED: transport").map(|_| ()),
            Err(error) => Err(error).context("MCP_FAILED: initialization"),
        }
    });
    match target.as_ref() {
        Target::Local(backend) => {
            let finish = backend.disconnect();
            backend.host.jobs.shutdown();
            result.and(finish)
        }
        Target::App(_) | Target::Current(_) => result,
    }
}

pub fn run_args(args: &[String], mut cache: PathBuf) -> Result<()> {
    let mut project = None;
    let mut current = false;
    let mut allow_write = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--project" => {
                project = Some(PathBuf::from(args.next().context("INVALID_ARGUMENTS: --project requires a path")?))
            }
            // The project open in the app, so one agent configuration serves every project.
            "--current" => current = true,
            "--cache" => cache = PathBuf::from(args.next().context("INVALID_ARGUMENTS: --cache requires a directory")?),
            "--allow-write" => allow_write = true,
            _ => bail!("INVALID_ARGUMENTS: unknown MCP argument: {arg}"),
        }
    }
    match (project, current) {
        (Some(_), true) => bail!("INVALID_ARGUMENTS: use --project <path> or --current, not both"),
        (Some(project), false) => run(&project, allow_write, cache),
        (None, true) => {
            if let Some(nonce) = std::env::var_os(VERIFY_VAR) {
                let project = crate::ipc::current_project()?.context(NOT_OPEN)?;
                anyhow::ensure!(crate::ipc::verify(&project, &nonce.to_string_lossy())?, NOT_OPEN);
            }
            serve(Target::Current(Arc::new(Current { allow_write, remote: Mutex::default() })))
        }
        (None, false) => bail!("INVALID_ARGUMENTS: mcp requires --project <path> or --current"),
    }
}

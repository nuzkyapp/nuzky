use std::path::PathBuf;

use nuzky_engine::{Project, edit::EditOutcome, model::Clip};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    ReadOnly,
    Write,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndAction {
    Keep,
    Discard,
}

#[derive(Clone, Copy, Debug)]
pub enum RecoveryAction {
    Keep,
    Restore,
}

#[derive(Clone, Debug, Serialize)]
pub struct Stamp {
    pub revision: u64,
    pub session_epoch: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunInfo {
    pub run_id: String,
    pub label: String,
    /// The name the agent's MCP client gave when it connected, such as `claude-code`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionState {
    pub project: Project,
    pub speech_layout_key: String,
    #[serde(flatten)]
    pub stamp: Stamp,
    pub open_run: Option<RunInfo>,
    pub recovery_checkpoint: Option<PathBuf>,
    pub selection: Vec<String>,
    pub playhead_us: i64,
    pub undo_run_id: Option<String>,
    pub read_only: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClipChange {
    pub track_id: String,
    #[serde(flatten)]
    pub clip: Clip,
}

#[derive(Clone, Debug, Serialize)]
pub struct EditResult {
    #[serde(flatten)]
    pub stamp: Stamp,
    #[serde(flatten)]
    pub outcome: EditOutcome,
    pub changed: Vec<String>,
    /// Actual positions after magnetic packing, for created and changed clips.
    pub clips: Vec<ClipChange>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunResult {
    #[serde(flatten)]
    pub stamp: Stamp,
    pub run_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expect {
    pub revision: Option<u64>,
    pub speech_layout_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    User,
    Run {
        run_id: String,
        label: String,
    },
    Undo,
    Redo,
    Recovery,
    /// A version from the history, through `undo_to`.
    Restore,
}

#[derive(Clone, Debug)]
pub enum SessionEvent {
    Job {
        id: String,
        kind: &'static str,
        status: &'static str,
        progress: f32,
        error: Option<String>,
    },
    TranscriptsChanged,
    Changed {
        revision: u64,
        origin: Origin,
    },
    Run(Option<RunInfo>),
    /// The agent that owns the open run called this tool, so the editor's pointer goes where that work happens.
    Tool {
        name: String,
    },
    /// What the run that is ending changed, sent just before `Run(None)`.
    RunSummary {
        run_id: String,
        changes: Vec<crate::summary::RunChange>,
    },
    Saved {
        revision: u64,
        error: Option<String>,
    },
}

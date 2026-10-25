//! Nuzky's own tools a plugin may ask for after its edits, by name in plugin.json's `tools`. None of them downloads,
//! transcribes or prepares sound: they read only the project, the stored transcripts, the sound Nuzky already
//! prepared, the creator's style and the saved caption styles. They are planned in the plugin's own process, after its
//! edits and on the project they left, within its memory limit and killed by Stop, and their edits join its batch.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Result, anyhow, ensure};
use nuzky_engine::{
    Project,
    edit::{EditCmd, TimeRange},
};
use nuzky_session::{host::Host, transcripts::TranscriptStore};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    params::{ApplyMotion, ApplyZooms, Captions},
    plugins::Manifest,
};

/// The tools a plugin may use. Planning a call and the gallery's sentence for a plugin that uses one match on this, so a
/// new tool cannot be left out of either.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tool {
    BuildCaptions,
    ApplyZooms,
    ApplyMotion,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::BuildCaptions, Tool::ApplyZooms, Tool::ApplyMotion];

    pub fn name(self) -> &'static str {
        match self {
            Tool::BuildCaptions => "build_captions",
            Tool::ApplyZooms => "apply_zooms",
            Tool::ApplyMotion => "apply_motion",
        }
    }

    pub fn named(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|tool| tool.name() == name)
    }

    /// It reads the transcript, so a plugin that uses it needs the words.
    pub fn reads_words(self) -> bool {
        match self {
            Tool::BuildCaptions | Tool::ApplyZooms => true,
            Tool::ApplyMotion => false,
        }
    }

    /// "build_captions, apply_zooms, apply_motion".
    pub fn all_names() -> String {
        Tool::ALL.map(Tool::name).join(", ")
    }
}
const MAX_CALLS: usize = 20;
/// A tool's arguments are a few settings; a style's text is a font or colour name. Caption styles are copied into every
/// caption, so a long one would grow the project by the number of words.
const MAX_ARGS_BYTES: usize = 16 << 10;
const MAX_ARG_CHARS: usize = 200;
/// All the calls' edits together, which the app applies as one batch: an hour of captions is a few MB.
pub(crate) const MAX_PLANNED_BYTES: usize = 16 << 20;
static SLOW: Mutex<Option<(String, Duration)>> = Mutex::new(None);

/// A call as `run` returns it in `tools`: the tool's name and its MCP arguments without `run_id` and `request_id`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub tool: String,
    #[serde(default)]
    pub args: Map<String, Value>,
}

enum Checked {
    Captions(Box<Captions>),
    Zooms(ApplyZooms),
    Motion(ApplyMotion),
}

/// `CODE: tools[i] (tool): the rest`, so the error keeps the code the app and agents read first.
fn located(i: usize, tool: &str, error: anyhow::Error) -> anyhow::Error {
    let text = format!("{error:#}");
    match text.split_once(": ").filter(|(code, _)| code.bytes().all(|c| c.is_ascii_uppercase() || c == b'_')) {
        Some((code, rest)) => anyhow!("{code}: tools[{i}] ({tool}): {rest}"),
        None => anyhow!("PLUGIN_BAD_TOOL: tools[{i}] ({tool}): {text}"),
    }
}

/// The calls checked against the manifest and against each tool's own arguments, as the MCP tool checks them.
fn check(declared: &[String], calls: &[Call]) -> Result<Vec<Checked>> {
    ensure!(
        calls.len() <= MAX_CALLS,
        "PLUGIN_TOO_MANY_TOOLS: the plugin asked for {} tool calls; at most {MAX_CALLS} run at once",
        calls.len()
    );
    calls
        .iter()
        .enumerate()
        .map(|(i, call)| {
            let tool = call.tool.as_str();
            let known = Tool::named(tool).ok_or_else(|| {
                anyhow!("PLUGIN_FORBIDDEN_TOOL: tools[{i}] ({tool}): a plugin can use only {}", Tool::all_names())
            })?;
            ensure!(
                declared.iter().any(|name| name == tool),
                "PLUGIN_FORBIDDEN_TOOL: tools[{i}] ({tool}) is not in plugin.json's tools"
            );
            ensure!(
                !call.args.contains_key("run_id") && !call.args.contains_key("request_id"),
                "PLUGIN_BAD_TOOL: tools[{i}] ({tool}): leave out run_id and request_id; the call joins the plugin's run"
            );
            ensure!(
                serde_json::to_vec(&call.args)?.len() <= MAX_ARGS_BYTES && !long_text(&Value::Object(call.args.clone())),
                "PLUGIN_BAD_TOOL: tools[{i}] ({tool}): its arguments are at most {} KB, each text at most {MAX_ARG_CHARS} characters",
                MAX_ARGS_BYTES >> 10
            );
            let mut args = call.args.clone();
            args.insert("run_id".into(), json!(""));
            let args = Value::Object(args);
            let bad = |e: serde_json::Error| anyhow!("PLUGIN_BAD_TOOL: tools[{i}] ({tool}): {e}");
            Ok(match known {
                Tool::BuildCaptions => Checked::Captions(Box::new(serde_json::from_value(args).map_err(bad)?)),
                Tool::ApplyZooms => Checked::Zooms(serde_json::from_value(args).map_err(bad)?),
                Tool::ApplyMotion => Checked::Motion(serde_json::from_value(args).map_err(bad)?),
            })
        })
        .collect()
}

fn long_text(value: &Value) -> bool {
    match value {
        Value::String(text) => text.chars().count() > MAX_ARG_CHARS,
        Value::Array(items) => items.iter().any(long_text),
        Value::Object(fields) => {
            fields.iter().any(|(key, value)| key.chars().count() > MAX_ARG_CHARS || long_text(value))
        }
        _ => false,
    }
}

/// The tools the calls name, in order, after checking them; a preview names them without running them.
pub fn names(declared: &[String], calls: &[Call]) -> Result<Vec<String>> {
    check(declared, calls)?;
    Ok(calls.iter().map(|call| call.tool.clone()).collect())
}

/// Where the plugin's process finds what the tools read, and the tracks the project had before the plugin's edits: a
/// track made in the batch gets a new id when the batch applies.
#[derive(Serialize, Deserialize)]
pub(crate) struct Reach {
    pub transcripts: PathBuf,
    pub cache: PathBuf,
    /// The project's folder, which its media's paths start from.
    pub dir: PathBuf,
    pub tracks: HashSet<String>,
    pub plugin: String,
    pub declared: Vec<String>,
    /// Set by `slow_tools_for_tests` only.
    #[serde(default)]
    pub slow: Duration,
}

impl Reach {
    pub(crate) fn new(host: &Host, plugin: &Manifest, dir: &Path, before: &Project) -> Self {
        let slow =
            SLOW.lock().unwrap().as_ref().filter(|(id, _)| *id == plugin.id).map_or(Duration::ZERO, |(_, each)| *each);
        Reach {
            transcripts: host.transcripts.directory().to_path_buf(),
            cache: host.cache_dir.clone(),
            dir: dir.to_path_buf(),
            tracks: before.tracks.iter().map(|track| track.id.clone()).collect(),
            plugin: plugin.id.clone(),
            declared: plugin.tools.clone(),
            slow,
        }
    }
}

/// The edits of every call, each planned on the project the edits before it left, starting from `project`, and the
/// project after them. Runs in the plugin's process; nothing reaches the session.
pub(crate) fn plan(
    reach: &Reach,
    calls: &[Call],
    mut project: Project,
    progress: &dyn Fn(f32),
) -> Result<(Vec<EditCmd>, Project)> {
    let checked = check(&reach.declared, calls)?;
    let transcripts = TranscriptStore::at(reach.transcripts.clone())?;
    let (mut edits, mut planned) = (Vec::new(), 0);
    for (i, (call, checked)) in calls.iter().zip(checked).enumerate() {
        progress(i as f32 / calls.len() as f32);
        let mut media = project.clone();
        for asset in &mut media.assets {
            asset.path = reach.dir.join(&asset.path).to_string_lossy().into_owned();
        }
        let edit =
            edit((&transcripts, &reach.cache), checked, &project, &media).map_err(|e| located(i, &call.tool, e))?;
        std::thread::sleep(reach.slow);
        if let EditCmd::ReplaceCaptions { track_id, .. } = &edit {
            ensure!(
                reach.tracks.contains(track_id),
                "{}",
                located(
                    i,
                    &call.tool,
                    anyhow!("PLUGIN_BAD_TOOL: the captions it would make again were made in this run; make them once")
                )
            );
        }
        project.apply(edit.clone()).map_err(|e| located(i, &call.tool, anyhow!("PLUGIN_BAD_TOOL: {e:#}")))?;
        ensure!(
            serde_json::to_vec(&project)?.len() <= crate::limits::MAX_TOOL_BYTES,
            "PLUGIN_TOO_BIG: tools[{i}] ({}) makes the project larger than {} MB",
            call.tool,
            crate::limits::MAX_TOOL_BYTES >> 20
        );
        planned += serde_json::to_vec(&edit)?.len();
        ensure!(
            planned <= MAX_PLANNED_BYTES,
            "PLUGIN_TOO_BIG: tools[{i}] ({}) makes the tools' edits larger than {} MB together",
            call.tool,
            MAX_PLANNED_BYTES >> 20
        );
        edits.push(edit);
    }
    nuzky_session::validate(&project)
        .map_err(|e| anyhow!("PLUGIN_BAD_TOOL: the tools leave the project invalid: {e:#}"))?;
    Ok((edits, project))
}

fn edit(
    (transcripts, cache): (&TranscriptStore, &Path),
    call: Checked,
    project: &Project,
    media: &Project,
) -> Result<EditCmd> {
    Ok(match call {
        Checked::Captions(args) => {
            crate::tools::caption_plan(&args, project, media, (transcripts, cache), &|| false)?.0
        }
        Checked::Zooms(args) => {
            let derived = crate::transcript::derive(media, transcripts)?;
            ensure!(derived.untranscribed.is_empty(), "TRANSCRIPT_MISSING: transcribe all heard assets before zooms");
            ensure!(
                args.transcript_key == crate::transcript::word_key(project, &derived.words),
                "SPEECH_CHANGED: give ctx.transcriptKey, and zoom words of ctx.words that the plugin's own edits leave where they are"
            );
            EditCmd::ZoomRanges { ranges: crate::zooms::ranges(project, &derived.words, &args.zooms)? }
        }
        Checked::Motion(args) => EditCmd::ApplyMotion {
            clip_id: args.clip_id,
            range: args.range_us.map(|[start_us, end_us]| TimeRange { start_us, end_us }),
            kind: args.kind,
            strength: args.strength,
        },
    })
}

/// Makes every tool call of the plugin `id` take `each` in the plugin's process, so a test can stop a run while its
/// tools are planned.
#[doc(hidden)]
pub fn slow_tools_for_tests(id: &str, each: Duration) {
    *SLOW.lock().unwrap() = Some((id.to_owned(), each));
}

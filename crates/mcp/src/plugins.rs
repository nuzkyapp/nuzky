//! The user's plugins: a folder in Nuzky's data with plugin.json and plugin.js, whose `run(ctx)` returns edits that
//! apply as one batch of a run. The script runs in QuickJS with no modules, files, network, timers or processes, and
//! together with the check of its edits in a process of its own (`plugin-sandbox`) with a memory limit.
use std::cell::{Cell, RefCell};
use std::collections::{HashSet, VecDeque};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc::RecvTimeoutError,
};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use nuzky_engine::{
    Project,
    edit::{EditCmd, new_id},
};
use nuzky_session::{EndAction, Expect, SessionState, host::Host};
use rquickjs::{
    Ctx, Function, Object,
    context::EvalOptions,
    prelude::{Opt, Rest},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub const API: u64 = 1;
pub const GUIDE: &str = include_str!("../../../skills/nuzky-plugin/SKILL.md");
const MAX_SCRIPT_BYTES: u64 = 1 << 20;
const MAX_TEXT_BYTES: u64 = 64 << 10;
const MAX_TEXT_INPUT: usize = 500;
const MAX_EDITS: usize = 5_000;
/// What `run` may return, and how big its edits may make the project: a get_state answer must still fit a message.
const MAX_RESULT_BYTES: usize = 8 << 20;
const MAX_PROJECT_BYTES: usize = crate::limits::MAX_TOOL_BYTES;
/// Restyling a track copies one value into every clip, so no single text may be long enough to multiply into gigabytes.
const MAX_STRING_CHARS: usize = 10_000;
const MAX_LOG_LINES: usize = 200;
const RUN_LIMIT: Duration = Duration::from_secs(60);
/// Saving runs the script's top level once, to report syntax errors and a missing `run`.
const SAVE_LIMIT: Duration = Duration::from_secs(5);
/// What the plugin's process may take: QuickJS's 256 MB, the project and its copy at the 64 MB size limit, with room
/// to spare. One small batch of edits can copy gigabytes before any check sees it, which would end the app.
const MEMORY_LIMIT: u64 = 1 << 30;
const OUT_OF_MEMORY: &str = "PLUGIN_OUT_OF_MEMORY: the plugin took more than 1 GB of memory and was stopped";
const PROGRESS: &str = "progress ";
static PROGRAM: OnceLock<PathBuf> = OnceLock::new();

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub nuzky_plugin: u64,
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    /// The verb on the plugin's button.
    #[serde(default = "default_action")]
    pub action: String,
    #[serde(default)]
    pub needs: Vec<Need>,
    #[serde(default)]
    pub inputs: Vec<Input>,
}

fn default_action() -> String {
    "Run".into()
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Need {
    Words,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum Input {
    Number {
        id: String,
        label: String,
        min: f64,
        max: f64,
        step: f64,
        default: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
    Toggle {
        id: String,
        label: String,
        default: bool,
    },
    Choice {
        id: String,
        label: String,
        options: Vec<Choice>,
        default: String,
    },
    Text {
        id: String,
        label: String,
        default: String,
    },
    Color {
        id: String,
        label: String,
        default: String,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub value: String,
    pub label: String,
}

/// An installed plugin as the app and list_plugins show it.
#[derive(Serialize)]
pub struct Plugin {
    /// The folder's name.
    pub id: String,
    /// "ok", or "broken" with `error` saying why.
    pub status: &'static str,
    pub error: Option<String>,
    pub manifest: Option<Manifest>,
    pub readme: Option<String>,
}

impl Input {
    fn id(&self) -> &str {
        match self {
            Input::Number { id, .. }
            | Input::Toggle { id, .. }
            | Input::Choice { id, .. }
            | Input::Text { id, .. }
            | Input::Color { id, .. } => id,
        }
    }

    fn label(&self) -> &str {
        match self {
            Input::Number { label, .. }
            | Input::Toggle { label, .. }
            | Input::Choice { label, .. }
            | Input::Text { label, .. }
            | Input::Color { label, .. } => label,
        }
    }

    fn default(&self) -> Value {
        match self {
            Input::Number { default, .. } => json!(default),
            Input::Toggle { default, .. } => json!(default),
            Input::Choice { default, .. } | Input::Text { default, .. } | Input::Color { default, .. } => {
                json!(default)
            }
        }
    }

    fn check(&self, value: &Value) -> Result<()> {
        let (valid, expected) = match self {
            Input::Number { min, max, .. } => {
                (value.as_f64().is_some_and(|v| (*min..=*max).contains(&v)), format!("a number from {min} to {max}"))
            }
            Input::Toggle { .. } => (value.is_boolean(), "true or false".into()),
            Input::Choice { options, .. } => (
                options.iter().any(|option| *value == option.value),
                format!("one of {}", options.iter().map(|o| o.value.as_str()).collect::<Vec<_>>().join(", ")),
            ),
            Input::Text { .. } => (
                value.as_str().is_some_and(|text| text.chars().count() <= MAX_TEXT_INPUT),
                format!("text of at most {MAX_TEXT_INPUT} characters"),
            ),
            Input::Color { .. } => (value.as_str().is_some_and(is_color), "a colour written #RRGGBB".into()),
        };
        ensure!(valid, "INVALID_INPUT: {} must be {expected}", self.id());
        Ok(())
    }
}

fn is_color(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn is_id(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn manifest(value: &Value) -> Result<Manifest> {
    let api = value
        .get("nuzkyPlugin")
        .and_then(Value::as_u64)
        .context("PLUGIN_BROKEN: plugin.json: nuzkyPlugin must be 1")?;
    ensure!(api <= API, "PLUGIN_API_NEWER: This plugin needs a newer Nuzky. Update Nuzky.");
    ensure!(api == API, "PLUGIN_API_OLD: This plugin was made for an older Nuzky. Ask its author for a new version.");
    let manifest = Manifest::deserialize(value).map_err(|e| anyhow!("PLUGIN_BROKEN: plugin.json: {e}"))?;
    ensure!(is_id(&manifest.id), "PLUGIN_BROKEN: plugin.json: id is 1 to 40 lowercase letters, digits and dashes");
    let texts = [
        ("name", &manifest.name, 40),
        ("description", &manifest.description, 300),
        ("version", &manifest.version, 20),
        ("action", &manifest.action, 24),
    ];
    for (field, text, max) in texts {
        ensure!(
            !text.trim().is_empty() && text.chars().count() <= max,
            "PLUGIN_BROKEN: plugin.json: {field} needs 1 to {max} characters"
        );
    }
    ensure!(manifest.inputs.len() <= 20, "PLUGIN_BROKEN: plugin.json: at most 20 inputs");
    let mut ids = HashSet::new();
    for input in &manifest.inputs {
        let id = input.id();
        ensure!(
            (1..=40).contains(&id.len())
                && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                && ids.insert(id),
            "PLUGIN_BROKEN: plugin.json: input id {id:?} must be distinct, 1 to 40 letters, digits and _"
        );
        ensure!(
            !input.label().trim().is_empty() && input.label().chars().count() <= 40,
            "PLUGIN_BROKEN: plugin.json: the label of input {id} needs 1 to 40 characters"
        );
        match input {
            Input::Number { min, max, step, unit, .. } => ensure!(
                min.is_finite()
                    && max.is_finite()
                    && min < max
                    && step.is_finite()
                    && *step > 0.0
                    && unit.as_ref().is_none_or(|unit| unit.chars().count() <= 12),
                "PLUGIN_BROKEN: plugin.json: input {id} needs min below max, a positive step and a unit of at most 12 characters"
            ),
            Input::Choice { options, .. } => {
                let values: HashSet<_> = options.iter().map(|o| o.value.as_str()).collect();
                ensure!(
                    (1..=20).contains(&options.len())
                        && values.len() == options.len()
                        && options.iter().all(|o| {
                            (1..=40).contains(&o.value.chars().count())
                                && !o.label.trim().is_empty()
                                && o.label.chars().count() <= 40
                        }),
                    "PLUGIN_BROKEN: plugin.json: input {id} needs 1 to 20 options with distinct values and labels of 1 to 40 characters"
                )
            }
            _ => {}
        }
        input.check(&input.default()).map_err(|e| {
            anyhow!(
                "PLUGIN_BROKEN: plugin.json: the default of {}",
                e.to_string().trim_start_matches("INVALID_INPUT: ")
            )
        })?;
    }
    Ok(manifest)
}

/// Each input's value, checked against the manifest; those not given take their default.
fn inputs(manifest: &Manifest, given: &Map<String, Value>) -> Result<Map<String, Value>> {
    if let Some(unknown) = given.keys().find(|key| !manifest.inputs.iter().any(|input| input.id() == key.as_str())) {
        bail!("INVALID_INPUT: {unknown} is not an input of this plugin");
    }
    manifest
        .inputs
        .iter()
        .map(|input| {
            let value = given.get(input.id()).cloned().unwrap_or_else(|| input.default());
            input.check(&value)?;
            Ok((input.id().to_owned(), value))
        })
        .collect()
}

/// The folder that holds every plugin, one folder each; it survives updates.
pub fn dir() -> Result<PathBuf> {
    Ok(nuzky_engine::data_dir().context("STORE_UNAVAILABLE: no data directory for plugins")?.join("plugins"))
}

/// A plugin's own file; a link is refused, since a folder from someone else could point it at the user's files.
fn read(path: &Path, max: u64) -> Result<String> {
    let meta = std::fs::symlink_metadata(path).context("missing")?;
    ensure!(meta.is_file(), "not a plain file");
    let size = meta.len();
    ensure!(size <= max, "larger than {} KB", max >> 10);
    std::fs::read_to_string(path).context("unreadable")
}

fn open(folder: &Path) -> Result<(Manifest, String)> {
    ensure!(
        std::fs::symlink_metadata(folder).is_ok_and(|meta| meta.is_dir()),
        "PLUGIN_BROKEN: the plugin is a link, not a folder"
    );
    let text =
        read(&folder.join("plugin.json"), MAX_TEXT_BYTES).map_err(|e| anyhow!("PLUGIN_BROKEN: plugin.json: {e:#}"))?;
    let value = serde_json::from_str(&text).map_err(|e| anyhow!("PLUGIN_BROKEN: plugin.json: {e}"))?;
    let manifest = manifest(&value)?;
    ensure!(
        folder.file_name().is_some_and(|name| *name == *manifest.id),
        "PLUGIN_BROKEN: plugin.json: id {:?} differs from its folder's name",
        manifest.id
    );
    let script =
        read(&folder.join("plugin.js"), MAX_SCRIPT_BYTES).map_err(|e| anyhow!("PLUGIN_BROKEN: plugin.js: {e:#}"))?;
    Ok((manifest, script))
}

fn entry(folder: &Path) -> Plugin {
    let id = folder.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let readme = read(&folder.join("README.md"), MAX_TEXT_BYTES).ok();
    match open(folder) {
        Ok((manifest, _)) => Plugin { id, status: "ok", error: None, manifest: Some(manifest), readme },
        Err(error) => Plugin { id, status: "broken", error: Some(format!("{error:#}")), manifest: None, readme },
    }
}

pub fn list() -> Result<Vec<Plugin>> {
    let entries = match std::fs::read_dir(dir()?) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error).context("STORE_UNREADABLE: cannot read the plugins folder"),
    };
    let mut plugins: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|path| std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir()))
        .map(|path| entry(&path))
        .collect();
    plugins.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(plugins)
}

/// Writes the plugin under plugins/<id>/, keeping the files it replaces as plugin.prev.json and plugin.prev.js.
pub fn save(manifest: &Value, script: &str, readme: Option<&str>) -> Result<Plugin> {
    let manifest = self::manifest(manifest)?;
    ensure!(script.len() as u64 <= MAX_SCRIPT_BYTES, "PLUGIN_BROKEN: plugin.js is larger than 1 MB");
    ensure!(
        readme.is_none_or(|readme| readme.len() as u64 <= MAX_TEXT_BYTES),
        "PLUGIN_BROKEN: README.md is larger than 64 KB"
    );
    sandbox(script, "PLUGIN_BROKEN", SAVE_LIMIT, Rc::default(), Rc::new(|_| {}), |_, _| Ok(()))?;
    let folder = dir()?.join(&manifest.id);
    let failed = "STORE_WRITE_FAILED: cannot save the plugin";
    // A plugin folder from someone else may hold links; writing through one would overwrite the file it points at.
    ensure!(
        std::fs::symlink_metadata(&folder).map_or(true, |meta| meta.is_dir()),
        "PLUGIN_BROKEN: plugins/{} is not a folder",
        manifest.id
    );
    std::fs::create_dir_all(&folder).context(failed)?;
    for (file, previous) in [("plugin.json", "plugin.prev.json"), ("plugin.js", "plugin.prev.js")] {
        if let Ok(old) = read(&folder.join(file), MAX_SCRIPT_BYTES) {
            crate::sounds::publish(&folder.join(previous), |out| Ok(out.write_all(old.as_bytes())?)).context(failed)?;
        }
    }
    let json = serde_json::to_vec_pretty(&manifest)?;
    for (file, bytes) in
        [("plugin.js", Some(script.as_bytes())), ("README.md", readme.map(str::as_bytes)), ("plugin.json", Some(&json))]
    {
        if let Some(bytes) = bytes {
            crate::sounds::publish(&folder.join(file), |out| Ok(out.write_all(bytes)?)).context(failed)?;
        }
    }
    Ok(entry(&folder))
}

/// A plugin with its checked inputs and the `ctx` its `run` gets, read from the project as it is.
pub(crate) struct Prepared {
    manifest: Manifest,
    script: String,
    ctx: String,
}

pub(crate) fn prepare(
    host: &Host,
    state: &SessionState,
    id: &str,
    given: &Map<String, Value>,
    selection: Option<Vec<String>>,
    playhead_us: Option<i64>,
) -> Result<Prepared> {
    let folder = dir()?.join(id);
    ensure!(
        is_id(id) && folder.join("plugin.json").is_file(),
        "PLUGIN_NOT_FOUND: no plugin {id:?}; list_plugins shows the installed ones"
    );
    let (manifest, script) = open(&folder)?;
    let mut ctx = json!({
        "api": API,
        "project": crate::tools::state_value(state, crate::params::State { range: None, clip_ids: None })?,
        "inputs": inputs(&manifest, given)?,
        "selection": selection.unwrap_or_else(|| state.selection.clone()),
        "playheadUs": playhead_us.unwrap_or(state.playhead_us),
    });
    if manifest.needs.contains(&Need::Words) {
        let dir = host.session.path().parent().map(Path::to_path_buf).unwrap_or_default();
        let mut media = state.project.clone();
        for asset in &mut media.assets {
            asset.path = dir.join(&asset.path).to_string_lossy().into_owned();
        }
        let derived = crate::transcript::derive(&media, &host.transcripts)?;
        ensure!(
            derived.untranscribed.is_empty(),
            "TRANSCRIPT_MISSING: {} needs the words; transcribe the video first",
            manifest.name
        );
        ctx["words"] = crate::transcript::summary(&derived, None)?["words"].take();
    }
    Ok(Prepared { manifest, script, ctx: ctx.to_string() })
}

#[derive(Serialize, Deserialize)]
struct Output {
    edits: Vec<EditCmd>,
    message: Option<String>,
    logs: Vec<String>,
}

/// What the plugin's process answers: the plugin's edits, message and log, and the project after the edits or why
/// they do not apply.
type Checked = (Output, Result<Project, String>);

type Logs = Rc<RefCell<VecDeque<String>>>;

fn execute(script: &str, input: String, progress: impl Fn(f32) + 'static) -> Result<Output> {
    let logs = Logs::default();
    let result = sandbox(script, "PLUGIN_FAILED", RUN_LIMIT, logs.clone(), Rc::new(progress), |ctx, run| {
        let returned: rquickjs::Value = run.call((ctx.json_parse(input)?,))?;
        ctx.json_stringify(returned)?.map(|json| json.to_string()).transpose()
    });
    let logs: Vec<String> = logs.take().into();
    let (edits, message) = result.and_then(|json| edits(json.as_deref())).map_err(|error| {
        let tail = &logs[logs.len().saturating_sub(20)..];
        if tail.is_empty() { error } else { anyhow!("{error:#}\nLast lines of its log:\n{}", tail.join("\n")) }
    })?;
    Ok(Output { edits, message, logs })
}

fn edits(json: Option<&str>) -> Result<(Vec<EditCmd>, Option<String>)> {
    let shape = "PLUGIN_FAILED: run(ctx) must return an array of edits or {edits, message}";
    let json = json.context(shape)?;
    ensure!(
        json.len() <= MAX_RESULT_BYTES,
        "PLUGIN_TOO_BIG: the plugin returned more than {} MB of edits",
        MAX_RESULT_BYTES >> 20
    );
    let (list, message) = match serde_json::from_str(json)? {
        Value::Array(list) => (list, None),
        Value::Object(mut object) => match object.remove("edits") {
            Some(Value::Array(list)) => (list, object.remove("message")),
            _ => bail!(shape),
        },
        _ => bail!(shape),
    };
    ensure!(
        list.len() <= MAX_EDITS,
        "PLUGIN_TOO_MANY_EDITS: the plugin returned {} edits; at most {MAX_EDITS} apply at once",
        list.len()
    );
    let message = match message {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.chars().take(500).collect()),
        Some(_) => bail!("PLUGIN_FAILED: message must be text"),
    };
    let edits = list
        .into_iter()
        .enumerate()
        .map(|(i, edit)| {
            let kind = edit["type"].as_str().unwrap_or("no type").to_owned();
            ensure!(
                !has_long_string(&edit),
                "PLUGIN_TOO_BIG: edit {i} ({kind}) has a text longer than {MAX_STRING_CHARS} characters"
            );
            serde_json::from_value(edit).map_err(|e| anyhow!("PLUGIN_BAD_EDIT: edit {i} ({kind}): {e}"))
        })
        .collect::<Result<_>>()?;
    Ok((edits, message))
}

fn has_long_string(value: &Value) -> bool {
    match value {
        Value::String(text) => text.len() > MAX_STRING_CHARS && text.chars().count() > MAX_STRING_CHARS,
        Value::Array(items) => items.iter().any(has_long_string),
        Value::Object(fields) => {
            fields.iter().any(|(key, value)| key.len() > MAX_STRING_CHARS || has_long_string(value))
        }
        _ => false,
    }
}

fn kind(edit: &EditCmd) -> String {
    serde_json::to_value(edit).ok().and_then(|v| v["type"].as_str().map(str::to_owned)).unwrap_or_default()
}

/// The project after the edits, applied one by one to a copy as the session applies a batch. A plugin never adds a
/// file or points the project at one, so a project it edited opens anywhere without it.
fn check(project: &Project, edits: &[EditCmd]) -> Result<Project> {
    let made: HashSet<_> = project.reel_candidates.iter().filter_map(|c| c.project_path.as_ref()).collect();
    let new_path = |path: &Option<String>| path.as_ref().is_some_and(|path| !made.contains(path));
    let mut after = project.clone();
    for (i, edit) in edits.iter().enumerate() {
        let forbidden = match edit {
            EditCmd::AddAssets { .. } => true,
            EditCmd::SetReelCandidates { candidates } => candidates.iter().any(|c| new_path(&c.project_path)),
            EditCmd::UpdateReelCandidate { candidate } => new_path(&candidate.project_path),
            _ => false,
        };
        ensure!(
            !forbidden,
            "PLUGIN_FORBIDDEN_EDIT: edit {i} ({}): a plugin cannot add files to the project or point it at one",
            kind(edit)
        );
        after.apply(edit.clone()).map_err(|e| anyhow!("PLUGIN_BAD_EDIT: edit {i} ({}): {e:#}", kind(edit)))?;
        // Duplicating a clip copies its text and restyling a track copies a style into every clip, so the project is
        // measured often enough that what grows between two checks stays small.
        let spreads = matches!(edit, EditCmd::RestyleTrack { .. } | EditCmd::PasteAttributes { .. });
        if spreads || i % 16 == 15 || i + 1 == edits.len() {
            ensure!(
                serde_json::to_vec(&after)?.len() <= MAX_PROJECT_BYTES,
                "PLUGIN_TOO_BIG: edit {i} ({}) makes the project larger than {} MB",
                kind(edit),
                MAX_PROJECT_BYTES >> 20
            );
        }
    }
    nuzky_session::validate(&after)
        .map_err(|e| anyhow!("PLUGIN_BAD_EDIT: the edits leave the project invalid: {e:#}"))?;
    Ok(after)
}

/// The answer of a dry run, and the project after the edits when they apply.
fn dry((output, after): Checked) -> (Value, Option<Project>) {
    let (after, error) = match after {
        Ok(after) => (Some(after), None),
        Err(error) => (None, Some(error)),
    };
    let answer = json!({"dry_run": true, "valid": after.is_some(), "error": error, "edits": output.edits,
        "message": output.message, "logs": output.logs});
    (answer, after)
}

fn apply(host: &Host, run_id: &str, revision: u64, (output, after): Checked, cancel: &AtomicBool) -> Result<Value> {
    after.map_err(anyhow::Error::msg)?;
    // A cancelled job reports no result, so its edits must not land either.
    ensure!(!cancel.load(Ordering::Relaxed), "CANCELLED: job cancelled");
    let count = output.edits.len();
    if count > 0 {
        let expect = Expect { revision: Some(revision), speech_layout_key: None };
        host.session.apply_edits(run_id, &new_id(), output.edits, expect)?;
    }
    Ok(json!({"applied": count, "message": output.message, "logs": output.logs}))
}

pub(crate) enum Mode {
    DryRun,
    /// Into an agent's run, which it ends itself.
    Apply,
    /// Into the run the app opened for it, kept on success and discarded otherwise.
    App,
}

/// Runs the plugin as a job of `run`, so stopping that run stops the script.
pub(crate) fn start(
    host: &Arc<Host>,
    owner: &str,
    run: Option<String>,
    mode: Mode,
    prepared: Prepared,
    state: &SessionState,
) -> Result<Value> {
    let weak = Arc::downgrade(host);
    let project = state.project.clone();
    let revision = state.stamp.revision;
    let run_id = run.clone().unwrap_or_default();
    host.start_job(owner, run.as_deref(), "plugin", state.stamp.clone(), move |cancel, progress| {
        let checked = isolated(&prepared, &project, &cancel, move |done| progress.set("running", Some(done)));
        let host = weak.upgrade().context("APP_CLOSED: project is closed")?;
        match mode {
            Mode::DryRun => Ok(dry(checked?).0),
            Mode::Apply => apply(&host, &run_id, revision, checked?, &cancel),
            Mode::App => {
                let result = checked.and_then(|checked| apply(&host, &run_id, revision, checked, &cancel));
                match &result {
                    Ok(_) => {
                        host.session.end_run(&run_id, EndAction::Keep)?;
                    }
                    // Stop has already ended the run.
                    Err(_) => _ = host.session.end_run(&run_id, EndAction::Discard),
                }
                result
            }
        }
    })
}

/// Runs the plugin from the app's UI in a run of its own labelled after it, so the AI lock and Stop apply and its
/// edits are one undo step. Answers the job, whose `run_id` is that run.
pub fn run_in_app(
    host: &Arc<Host>,
    id: &str,
    inputs: &Map<String, Value>,
    selection: Vec<String>,
    playhead_us: i64,
) -> Result<Value> {
    let state = host.session.state()?;
    let prepared = prepare(host, &state, id, inputs, Some(selection), Some(playhead_us))?;
    let run = host.session.begin_run(format!("Plugin: {}", prepared.manifest.name))?;
    // Edits the user made since `state` was read fail the batch with STALE_REVISION.
    let started = start(host, "app", Some(run.run_id.clone()), Mode::App, prepared, &state);
    if started.is_err() {
        let _ = host.session.end_run(&run.run_id, EndAction::Discard);
    }
    started
}

/// What the plugin would do with these inputs, without touching the project: its edits, message and log, and the
/// project after the edits when they apply. Setting `cancel` stops it.
pub fn dry_run_in_app(
    host: &Host,
    id: &str,
    inputs: &Map<String, Value>,
    selection: Vec<String>,
    playhead_us: i64,
    cancel: Arc<AtomicBool>,
) -> Result<Value> {
    let state = host.session.state()?;
    let prepared = prepare(host, &state, id, inputs, Some(selection), Some(playhead_us))?;
    let (mut answer, after) = dry(isolated(&prepared, &state.project, &cancel, |_| {})?);
    answer["project"] = json!(after);
    Ok(answer)
}

/// Tests run plugins from their own binary, which has no `plugin-sandbox`, so they name Nuzky's.
#[doc(hidden)]
pub fn use_sandbox_program(program: PathBuf) {
    PROGRAM.set(program).ok();
}

/// Kills the plugin's process however its run ends.
struct Sandboxed(Child);

impl Drop for Sandboxed {
    fn drop(&mut self) {
        self.0.kill().ok();
        self.0.wait().ok();
    }
}

/// Runs the plugin and checks its edits on a copy of `project` in Nuzky's own executable started as
/// `plugin-sandbox`, killed once it takes more than `MEMORY_LIMIT`, runs too long or `cancel` is set, so neither the
/// script nor edits that would copy gigabytes can take the app's memory. Nothing it does reaches the project.
fn isolated(
    prepared: &Prepared,
    project: &Project,
    cancel: &AtomicBool,
    progress: impl Fn(f32) + Send + 'static,
) -> Result<Checked> {
    let program = match PROGRAM.get() {
        Some(program) => program.clone(),
        None => std::env::current_exe().context("PLUGIN_FAILED: cannot find Nuzky to run the plugin")?,
    };
    let mut command = Command::new(program);
    command.arg("plugin-sandbox").env_clear().stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // Where the loader finds the libraries Nuzky itself was started with, such as an FFmpeg outside the system's.
    #[cfg(target_os = "linux")]
    if let Some(paths) = std::env::var_os("LD_LIBRARY_PATH") {
        command.env("LD_LIBRARY_PATH", paths);
    }
    #[cfg(windows)]
    {
        // Windows programs expect it even with nothing else in their environment.
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = Sandboxed(command.spawn().context("PLUGIN_FAILED: cannot start the plugin's process")?);
    // It waits for its input, so the limit holds before it does anything.
    #[cfg(windows)]
    let _job = limit_memory(&child.0)?;
    let input = serde_json::to_vec(&(&prepared.script, &prepared.ctx, project))?;
    let mut stdin = child.0.stdin.take().context("PLUGIN_FAILED: no input")?;
    std::thread::spawn(move || stdin.write_all(&input));
    let stdout = std::io::BufReader::new(child.0.stdout.take().context("PLUGIN_FAILED: no output")?);
    let (send, answered) = std::sync::mpsc::channel();
    // Read as it arrives, so the answer counts once it is whole rather than once the process has ended.
    std::thread::spawn(move || {
        send.send(serde_json::Deserializer::from_reader(stdout).into_iter::<Result<Checked, String>>().next()).ok()
    });
    let stderr = std::io::BufReader::new(child.0.stderr.take().context("PLUGIN_FAILED: no output")?);
    let said = std::thread::spawn(move || {
        let mut said = String::new();
        for line in stderr.lines().map_while(Result::ok) {
            match line.strip_prefix(PROGRESS).and_then(|done| done.parse().ok()) {
                Some(done) => progress(done),
                None if said.len() < 4096 => said += &(line + "\n"),
                None => {}
            }
        }
        said
    });
    let deadline = Instant::now() + RUN_LIMIT + Duration::from_secs(1);
    let answer = loop {
        match answered.recv_timeout(Duration::from_millis(10)) {
            Ok(answer) => break answer,
            Err(RecvTimeoutError::Disconnected) => break None,
            Err(RecvTimeoutError::Timeout) => {}
        }
        ensure!(!cancel.load(Ordering::Relaxed), "CANCELLED: job cancelled");
        ensure!(
            Instant::now() < deadline,
            "PLUGIN_TIMEOUT: the plugin ran longer than {} seconds and was stopped",
            RUN_LIMIT.as_secs()
        );
        ensure!(resident(child.0.id()).is_none_or(|bytes| bytes <= MEMORY_LIMIT), OUT_OF_MEMORY);
    };
    if let Some(Ok(checked)) = answer {
        drop(child);
        said.join().ok();
        return checked.map_err(anyhow::Error::msg);
    }
    let status = child.0.wait()?;
    let said = said.join().unwrap_or_default();
    // The allocator's own words when the operating system's limit refused it more memory.
    ensure!(!said.contains("memory allocation of"), OUT_OF_MEMORY);
    log::error!("The plugin's process ended with {status}: {said}");
    bail!("PLUGIN_FAILED: the plugin's process stopped unexpectedly ({status})")
}

/// The `plugin-sandbox` command: the script, its ctx and the project come on stdin, the answer leaves on stdout as
/// `Result<Checked, String>`, the progress on stderr. It touches no file.
pub fn sandbox_main() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let limit = libc::rlimit { rlim_cur: MEMORY_LIMIT, rlim_max: MEMORY_LIMIT };
        // SAFETY: setrlimit only reads the structure given.
        unsafe { libc::setrlimit(libc::RLIMIT_DATA, &limit) };
    }
    // Should the app die without killing it.
    std::thread::spawn(|| {
        std::thread::sleep(RUN_LIMIT + Duration::from_secs(5));
        std::process::exit(1);
    });
    let (script, input, project): (String, String, Project) = serde_json::from_reader(std::io::stdin().lock())?;
    let shown = Cell::new(-1.0);
    let progress = move |done: f32| {
        let done = (done * 100.0).round() / 100.0;
        if shown.replace(done) != done {
            eprintln!("{PROGRESS}{done}");
        }
    };
    let answer = execute(&script, input, progress)
        .map(|output| {
            let after = check(&project, &output.edits).map_err(|error| format!("{error:#}"));
            (output, after)
        })
        .map_err(|error| format!("{error:#}"));
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    serde_json::to_writer(&mut out, &answer)?;
    Ok(out.flush()?)
}

#[cfg(target_os = "linux")]
fn resident(pid: u32) -> Option<u64> {
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: sysconf only reads a setting.
    Some(pages * unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64)
}

/// macOS sets no limit on a process's memory, so the parent watches it. The footprint also counts memory the system
/// compressed, which the resident size leaves out once memory runs short.
#[cfg(target_os = "macos")]
fn resident(pid: u32) -> Option<u64> {
    // SAFETY: rusage_info_v4 is plain numbers, so zeroes are a valid value.
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    // SAFETY: info is the structure RUSAGE_INFO_V4 fills.
    let done = unsafe { libc::proc_pid_rusage(pid as libc::c_int, libc::RUSAGE_INFO_V4, (&raw mut info).cast()) };
    (done == 0).then_some(info.ri_phys_footprint)
}

/// The job object holds Windows to the limit.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn resident(_: u32) -> Option<u64> {
    None
}

/// A job that refuses the process memory above `MEMORY_LIMIT` and kills it once the handle closes.
#[cfg(windows)]
fn limit_memory(child: &Child) -> Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    // SAFETY: a job without a name or attributes; OwnedHandle closes the handle CreateJobObjectW returns.
    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    ensure!(!handle.is_null(), "PLUGIN_FAILED: cannot limit the plugin's memory");
    let job = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
    limits.ProcessMemoryLimit = MEMORY_LIMIT as usize;
    // SAFETY: limits is the structure this information class names, and both handles are open.
    let set = unsafe {
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) != 0
            && AssignProcessToJobObject(handle, child.as_raw_handle()) != 0
    };
    ensure!(set, "PLUGIN_FAILED: cannot limit the plugin's memory");
    Ok(job)
}

/// Evaluates the script in a fresh runtime, then hands `run` to `body`. Errors thrown in the script read
/// `<failed>: plugin.js:<line>:<column>: <name>: <message>`.
fn sandbox<T>(
    script: &str,
    failed: &str,
    limit: Duration,
    logs: Logs,
    progress: Rc<dyn Fn(f32)>,
    body: impl for<'js> FnOnce(&Ctx<'js>, Function<'js>) -> rquickjs::Result<T>,
) -> Result<T> {
    let runtime = rquickjs::Runtime::new().context("PLUGIN_FAILED: cannot start the script engine")?;
    runtime.set_memory_limit(256 << 20);
    runtime.set_max_stack_size(1 << 20);
    let deadline = Instant::now() + limit;
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
    let context = rquickjs::Context::full(&runtime).context("PLUGIN_FAILED: cannot start the script engine")?;
    let result = context.with(|ctx| {
        let run = || {
            host_api(&ctx, logs, progress)?;
            let mut options = EvalOptions::default();
            options.strict = false;
            options.filename = Some("plugin.js".into());
            ctx.eval_with_options::<(), _>(script, options)?;
            // `const run = …` is not a property of the global object, so ask the script's own scope.
            let run: rquickjs::Value = ctx.eval("typeof run === 'function' ? run : undefined")?;
            match run.into_function() {
                Some(run) => body(&ctx, run),
                None => Err(rquickjs::Exception::throw_type(&ctx, "plugin.js defines no function run(ctx)")),
            }
        };
        run().map_err(|error| thrown(&ctx, error))
    });
    match result {
        Ok(value) => Ok(value),
        Err(_) if Instant::now() >= deadline => {
            bail!("PLUGIN_TIMEOUT: the plugin ran longer than {} seconds and was stopped", limit.as_secs())
        }
        Err(Thrown::OutOfMemory(at)) => {
            bail!("PLUGIN_OUT_OF_MEMORY: the plugin's script took more than 256 MB of memory and was stopped ({at})")
        }
        Err(Thrown::Null) => {
            bail!("{failed}: the script threw null, which also happens when it runs out of its 256 MB of memory")
        }
        Err(Thrown::Message(message)) => bail!("{failed}: {message}"),
    }
}

fn host_api<'js>(ctx: &Ctx<'js>, logs: Logs, progress: Rc<dyn Fn(f32)>) -> rquickjs::Result<()> {
    let nuzky = Object::new(ctx.clone())?;
    let report = move |done: f64, _label: Opt<String>| {
        if done.is_finite() {
            progress(done.clamp(0.0, 1.0) as f32);
        }
    };
    nuzky.set("progress", Function::new(ctx.clone(), report)?)?;
    let log = move |ctx: Ctx<'js>, values: Rest<rquickjs::Value<'js>>| -> rquickjs::Result<()> {
        let mut parts = Vec::new();
        for value in values.0 {
            parts.push(match value.as_string() {
                Some(text) => text.to_string()?,
                None => {
                    ctx.json_stringify(value)?.map(|json| json.to_string()).transpose()?.unwrap_or("undefined".into())
                }
            });
        }
        let mut logs = logs.borrow_mut();
        if logs.len() == MAX_LOG_LINES {
            logs.pop_front();
        }
        logs.push_back(parts.join(" ").chars().take(1000).collect());
        Ok(())
    };
    nuzky.set("log", Function::new(ctx.clone(), log)?)?;
    ctx.globals().set("nuzky", nuzky)
}

enum Thrown {
    OutOfMemory(String),
    /// QuickJS throws null when it runs out of memory while making its own error.
    Null,
    Message(String),
}

/// What the script threw, with where in plugin.js when it says.
fn thrown(ctx: &Ctx, error: rquickjs::Error) -> Thrown {
    if !matches!(error, rquickjs::Error::Exception) {
        return Thrown::Message(error.to_string());
    }
    let value = ctx.catch();
    if value.is_null() {
        return Thrown::Null;
    }
    let Some(exception) = value.as_exception() else {
        let text = ctx.json_stringify(value).ok().flatten().and_then(|json| json.to_string().ok());
        return Thrown::Message(format!("threw {}", text.unwrap_or_else(|| "a value that is not an Error".into())));
    };
    let name: String = exception.get("name").unwrap_or_else(|_| "Error".into());
    let text = exception.message().unwrap_or_default();
    let out_of_memory = name == "InternalError" && text == "out of memory";
    let message = format!("{name}: {text}");
    let stack = exception.stack().unwrap_or_default();
    let place = stack.find("plugin.js:").map(|start| {
        let rest = &stack[start..];
        let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == ':')).unwrap_or(rest.len());
        rest[..end].trim_end_matches(':')
    });
    let message = match place {
        Some(place) => format!("{place}: {message}"),
        None => message,
    };
    if out_of_memory { Thrown::OutOfMemory(message) } else { Thrown::Message(message) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(script: &str) -> String {
        let run = |_: &Ctx, run: Function| run.call::<_, rquickjs::Value>(()).map(drop);
        let logs = Logs::default();
        sandbox(script, "PLUGIN_FAILED", Duration::from_secs(20), logs, Rc::new(|_| {}), run).unwrap_err().to_string()
    }

    /// A script that fills QuickJS's 256 MB fails with an error or with a bare null, never with what it allocated.
    #[test]
    fn a_script_that_runs_out_of_memory_says_so() {
        let arrays = failure("function run() { const a = []; for (;;) a.push(new Float64Array(1 << 20)); }");
        assert!(arrays.starts_with("PLUGIN_OUT_OF_MEMORY: the plugin's script took more than 256 MB"), "{arrays}");
        for strings in [
            "function run() { const a = []; for (;;) a.push(String(Math.random()).repeat(100)); }",
            "const a = []; function run() { for (;;) a.push(String(Math.random()).repeat(100)); }",
        ] {
            let error = failure(strings);
            let said = error.starts_with("PLUGIN_OUT_OF_MEMORY") || error.ends_with("runs out of its 256 MB of memory");
            assert!(said, "{error}");
        }
        let null = failure("function run() { throw null; }");
        assert_eq!(
            null,
            "PLUGIN_FAILED: the script threw null, which also happens when it runs out of its 256 MB of memory"
        );
        let own = failure("function run() { throw new Error('bad input: InternalError: out of memory'); }");
        assert!(own.starts_with("PLUGIN_FAILED: plugin.js:1:"), "{own}");
    }

    /// The guide's JSON is what plugins copy, so a renamed edit or field must fail here before it fails them.
    #[test]
    fn the_guide_shows_edits_and_a_manifest_nuzky_accepts() {
        let mut edits = 0;
        for block in GUIDE.split("```json\n").skip(1) {
            let value: Value = serde_json::from_str(block.split("```").next().unwrap()).unwrap();
            if value.get("nuzkyPlugin").is_some() {
                manifest(&value).unwrap();
            } else {
                serde_json::from_value::<EditCmd>(value.clone()).unwrap_or_else(|e| panic!("{value}: {e}"));
                edits += 1;
            }
        }
        assert!(edits >= 15, "{edits}");
    }
}

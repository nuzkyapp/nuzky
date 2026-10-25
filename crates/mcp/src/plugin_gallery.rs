//! Plugins other people wrote, listed in github.com/nuzkyapp/nuzky-plugins, which the maintainer curates through pull
//! requests. Nuzky reads the list only from one fixed address, and a plugin's files only from that repository at the
//! commit the list names, by addresses Nuzky builds itself. Every name is checked before any request, every file
//! against the size and SHA-256 the list gives, and installing writes files only: it runs none of the plugin's code.
use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
    mpsc::{self, RecvTimeoutError},
};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use nuzky_engine::{edit::new_id, model::AssetKind, plugin_effects};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::plugin_files;
use crate::plugins::{self, Manifest, Need};

const REPO: &str = "https://raw.githubusercontent.com/nuzkyapp/nuzky-plugins";
const VERSION: u64 = 1;
const MAX_INDEX_BYTES: u64 = 1 << 20;
const INDEX_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_FILE_BYTES: u64 = 20 << 20;
/// A file's body may take this long plus a second per `SLOWEST` bytes; then the download stops, so a server that goes
/// silent mid-file never keeps the download's thread and its folder.
const BODY_GRACE: Duration = Duration::from_secs(if cfg!(test) { 2 } else { 30 });
const SLOWEST: u64 = 16 << 10;
const MAX_TOTAL_BYTES: u64 = 50 << 20;
const MAX_FILES: usize = 32;
/// Which installed plugins came from the gallery and what their files were, beside the plugins folder, so a plugin
/// never sees it as one of its files.
const RECORD: &str = "plugin-gallery.json";
const STAGING: &str = "gallery-part-";
pub const REFUSED: &str =
    "GALLERY_REFUSED: Nuzky won't install this plugin. It includes files plugins aren't allowed to have.";
/// The one name a plugin's folder holds outside the lowercase pattern: the README the app shows.
const README: &str = "README.md";
const CORRUPT: &str =
    "GALLERY_CORRUPT: A file didn't arrive as the gallery lists it, so nothing was installed. Try again.";
const FAILED: &str = "STORE_WRITE_FAILED: Couldn't save the plugin";
const SCRIPT: &str = "plugin.js";
const STALE: &str = "GALLERY_STALE: The gallery changed since you opened this plugin. Look at it again.";

/// The gallery as Nuzky last read it.
#[derive(Clone, Debug)]
pub struct Gallery {
    repo: String,
    commit: String,
    plugins: Vec<Listed>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Index {
    commit: String,
    plugins: Vec<Listed>,
}

#[derive(Clone, Debug, Deserialize)]
struct Listed {
    author: String,
    manifest: Value,
    files: Vec<Listing>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Listing {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Available,
    Installed,
    /// Installed from the gallery and unchanged, and the gallery has other files for it.
    Update,
    /// Installed from the gallery, then changed by the user or their AI.
    Changed,
    /// The user's own plugin has the same id.
    Clash,
    /// Made for a newer Nuzky.
    Newer,
    /// Lists files Nuzky won't install.
    Refused,
}

/// A plugin of the gallery as the app shows it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    /// What the user is shown of it; installing it needs the same, so it never installs what another list describes.
    pub key: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: String,
    /// What it reads and does, in Nuzky's words from its manifest; the author's own words are only `description`.
    pub summary: Vec<Line>,
    pub files: Vec<Shown>,
    pub size: u64,
    pub state: State,
    pub installed_version: Option<String>,
}

#[derive(Serialize)]
pub struct Shown {
    pub path: String,
    pub size: u64,
    /// The file is why Nuzky won't install the plugin.
    pub refused: bool,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Line {
    pub text: String,
    /// The installed version could not do this, so agreeing to it then does not cover it now.
    pub new: bool,
}

/// Why Nuzky won't install a listed plugin, with the listed file that is the reason when there is one.
struct Refusal {
    error: anyhow::Error,
    file: Option<String>,
}

fn repo() -> String {
    // Debug builds read the gallery from a local server through `NUZKY_GALLERY_URL`; no other address replaces GitHub.
    if cfg!(debug_assertions)
        && let Some(url) = std::env::var("NUZKY_GALLERY_URL").ok().and_then(|url| test_server(&url))
    {
        return url;
    }
    REPO.into()
}

/// `http://127.0.0.1:<port>` or `http://localhost:<port>` with nothing else, the only addresses a test server has.
fn test_server(url: &str) -> Option<String> {
    let uri: ureq::http::Uri = url.parse().ok()?;
    let authority = uri.authority()?;
    let (host, port) = (authority.host(), authority.port_u16()?);
    let local = uri.scheme_str() == Some("http") && matches!(host, "127.0.0.1" | "localhost");
    let bare =
        authority.as_str() == format!("{host}:{port}") && matches!(uri.path(), "" | "/") && uri.query().is_none();
    (local && bare).then(|| format!("http://{host}:{port}"))
}

fn agent(repo: &str, global: Option<Duration>, body: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(test_server(repo).is_none())
        .max_redirects(0)
        .user_agent(concat!("Nuzky/", env!("CARGO_PKG_VERSION")))
        .timeout_global(global)
        .timeout_recv_body(body)
        .timeout_resolve(Some(INDEX_TIMEOUT))
        .timeout_connect(Some(INDEX_TIMEOUT))
        .timeout_recv_response(Some(INDEX_TIMEOUT))
        .build()
        .into()
}

fn request_error(error: ureq::Error) -> anyhow::Error {
    match error {
        ureq::Error::StatusCode(403 | 429 | 500..=599) => {
            anyhow!("GALLERY_BUSY: GitHub is busy right now. Try again in a few minutes.")
        }
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Io(_) | ureq::Error::Timeout(_) => {
            anyhow!("OFFLINE: Can't reach the gallery. Check your internet connection.")
        }
        error => anyhow!("GALLERY_FAILED: The gallery answered with an error ({error})."),
    }
}

fn get(agent: &ureq::Agent, url: &str) -> Result<ureq::http::Response<ureq::Body>> {
    let response = agent.get(url).call().map_err(request_error)?;
    // Nothing follows a redirect: the files come from this address or not at all.
    ensure!(
        response.status() == 200,
        "GALLERY_FAILED: The gallery answered with an error ({}).",
        response.status().as_u16()
    );
    Ok(response)
}

/// Reads the gallery's list. Only while the user has the Gallery open.
pub fn fetch() -> Result<Gallery> {
    fetch_from(&repo())
}

fn fetch_from(repo: &str) -> Result<Gallery> {
    let mut response = get(&agent(repo, Some(INDEX_TIMEOUT), None), &format!("{repo}/main/index.json"))?;
    let unreadable = "GALLERY_CORRUPT: The gallery's list didn't arrive whole. Try again.";
    // The limit counts bytes on the wire; a gzip body unpacks to far more, so the text is capped too.
    let mut text = String::new();
    response
        .body_mut()
        .with_config()
        .limit(MAX_INDEX_BYTES)
        .reader()
        .take(MAX_INDEX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| anyhow!(unreadable))?;
    ensure!(text.len() as u64 <= MAX_INDEX_BYTES, unreadable);
    parse(repo, &text)
}

fn parse(repo: &str, text: &str) -> Result<Gallery> {
    let unreadable = "GALLERY_CORRUPT: The gallery's list isn't one Nuzky can read. Try again later.";
    let value: Value = serde_json::from_str(text).map_err(|_| anyhow!(unreadable))?;
    let version = value.get("nuzkyGallery").and_then(Value::as_u64).context(unreadable)?;
    ensure!(version <= VERSION, "GALLERY_NEWER: Update Nuzky to see this gallery.");
    let index = Index::deserialize(value).map_err(|_| anyhow!(unreadable))?;
    ensure!(is_hex(&index.commit, 40), unreadable);
    Ok(Gallery { repo: repo.to_owned(), commit: index.commit, plugins: index.plugins })
}

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// A name Windows keeps for a device, whatever follows its first dot.
fn device(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or_default();
    matches!(stem, "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.as_bytes()[3].is_ascii_digit())
}

/// `^[a-z0-9][a-z0-9._-]{0,63}(/[a-z0-9][a-z0-9._-]{0,63})?$`, no device names and no name Windows would shorten.
fn plain_path(path: &str) -> bool {
    let parts: Vec<_> = path.split('/').collect();
    parts.len() <= 2
        && parts.iter().all(|part| {
            let bytes = part.as_bytes();
            (1..=64).contains(&bytes.len())
                && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
                && bytes
                    .iter()
                    .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-'))
                && !part.ends_with('.')
                && !device(part)
        })
}

/// The entry's manifest and whether Nuzky would install its files: only what a plugin's folder holds as its own, each
/// within the size Nuzky reads.
fn checked(listed: &Listed) -> Result<Manifest> {
    check(listed).map_err(|refusal| refusal.error)
}

fn check(listed: &Listed) -> Result<Manifest, Refusal> {
    let manifest = plugins::manifest(&listed.manifest).map_err(|error| Refusal {
        error: if error.to_string().starts_with("PLUGIN_API_NEWER") { error } else { anyhow!("{REFUSED} ({error})") },
        file: None,
    })?;
    // The id names the plugin's folder, which Windows could not create under a device's name.
    if device(&manifest.id) {
        return Err(Refusal { error: anyhow!("{REFUSED} ({}: a name Windows keeps)", manifest.id), file: None });
    }
    let refuse = |file: Option<&str>, why: String| Refusal {
        error: anyhow!("{REFUSED} ({}: {why})", manifest.id),
        file: file.map(str::to_owned),
    };
    if listed.files.len() > MAX_FILES {
        return Err(refuse(None, format!("more than {MAX_FILES} files")));
    }
    let (mut paths, mut total) = (HashSet::new(), 0u64);
    for file in &listed.files {
        let path = file.path.as_str();
        if !(path == README || plain_path(path)) || path.starts_with("plugin.prev.") {
            return Err(refuse(Some(path), format!("{path:?} is not a name Nuzky installs")));
        }
        if !paths.insert(path) {
            return Err(refuse(Some(path), format!("{path} is listed twice")));
        }
        let max = match path {
            SCRIPT => plugins::MAX_SCRIPT_BYTES,
            "plugin.json" | README => plugins::MAX_TEXT_BYTES,
            _ if manifest.files.iter().any(|name| name == path) => plugin_files::MAX_BYTES,
            _ if manifest.effects.iter().any(|effect| effect_file(&effect.id) == path) => {
                plugin_effects::MAX_SOURCE_BYTES
            }
            _ => return Err(refuse(Some(path), format!("{path} is not one of the plugin's files"))),
        };
        if file.size > max.min(MAX_FILE_BYTES) {
            return Err(refuse(Some(path), format!("{path} is larger than Nuzky takes")));
        }
        if !is_hex(&file.sha256, 64) {
            return Err(refuse(Some(path), format!("{path} has no SHA-256")));
        }
        total += file.size;
    }
    if total > MAX_TOTAL_BYTES {
        return Err(refuse(None, format!("more than {} MB in all", MAX_TOTAL_BYTES >> 20)));
    }
    // As in the plugins folder, a plugin that only adds effects may leave out its script.
    let script = paths.contains(SCRIPT) || manifest.effects.is_empty();
    let effects = manifest.effects.iter().map(|effect| effect_file(&effect.id));
    let needed = ["plugin.json", SCRIPT].into_iter().filter(|path| script || *path != SCRIPT).map(str::to_owned);
    for needed in needed.chain(manifest.files.iter().cloned()).chain(effects) {
        if !paths.contains(needed.as_str()) {
            return Err(refuse(None, format!("{needed} is missing")));
        }
    }
    if !paths.contains(SCRIPT) {
        plugins::only_effects(&manifest).map_err(|error| refuse(None, format!("{error:#}")))?;
    }
    Ok(manifest)
}

/// The listed plugin as the user is shown it: its author, manifest and files.
fn fingerprint(listed: &Listed) -> String {
    let shown = serde_json::to_vec(&(&listed.author, &listed.manifest, &listed.files)).unwrap_or_default();
    Sha256::digest(shown).iter().map(|b| format!("{b:02x}")).collect()
}

fn effect_file(id: &str) -> String {
    format!("effects/{id}.wgsl")
}

fn need_line(need: Need) -> &'static str {
    match need {
        Need::Words => "Reads the words said in the video.",
        Need::Silences => "Reads where the sound goes quiet.",
        Need::Loudness => "Reads how loud the sound is.",
        Need::Beats => "Reads the beats of the music.",
    }
}

/// As the app writes a file's size.
fn size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{} KB", (bytes as f64 / 1024.0).round()),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

fn counted(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// One thing a plugin can do, compared between versions without the sizes of its files.
#[derive(Clone, PartialEq)]
enum Fact {
    Changes,
    Adds {
        pictures: usize,
        sounds: usize,
    },
    /// The names of the looks it adds to clips.
    Looks(Vec<String>),
    Reads(Need),
    Sealed,
    /// Opening it runs its script for a preview that changes nothing.
    Preview,
    OnPress,
    /// It has no script, so only the looks the user puts on clips do anything.
    OnClips,
}

/// What every plugin can and can't do, whatever its manifest says.
const ALWAYS: [Fact; 4] = [Fact::Changes, Fact::Sealed, Fact::Preview, Fact::OnPress];

/// In the order the detail says them: what it changes, what it brings, what it reads, then its limits.
/// `runs`: it has a script.
fn facts(manifest: &Manifest, runs: bool) -> Vec<Fact> {
    let mut facts: Vec<_> = runs.then_some(Fact::Changes).into_iter().collect();
    let (mut pictures, mut sounds) = (0, 0);
    for name in &manifest.files {
        match plugin_files::kind(name) {
            Ok(AssetKind::Image) => pictures += 1,
            Ok(_) => sounds += 1,
            Err(_) => {}
        }
    }
    if pictures + sounds > 0 {
        facts.push(Fact::Adds { pictures, sounds });
    }
    if !manifest.effects.is_empty() {
        facts.push(Fact::Looks(manifest.effects.iter().map(|effect| effect.name.clone()).collect()));
    }
    facts.extend(manifest.needs.iter().map(|need| Fact::Reads(*need)));
    facts.push(Fact::Sealed);
    facts.extend(if runs { vec![Fact::Preview, Fact::OnPress] } else { vec![Fact::OnClips] });
    facts
}

/// "A", "A and B", "A, B and C".
fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn sentence(fact: Fact, manifest: &Manifest, files: &[Listing]) -> String {
    match fact {
        Fact::Changes => "Can change anything in this project, as one step you can undo.".into(),
        Fact::Adds { pictures, sounds } => {
            let what = match (pictures, sounds) {
                (p, 0) => counted(p, "picture"),
                (0, s) => counted(s, "sound"),
                (p, s) => format!("{} and {}", counted(p, "picture"), counted(s, "sound")),
            };
            let bytes = manifest
                .files
                .iter()
                .filter(|name| plugin_files::kind(name).is_ok())
                .filter_map(|name| files.iter().find(|f| f.path == *name))
                .map(|f| f.size)
                .sum();
            format!("Adds {what} of its own to your video and Media ({}).", size(bytes))
        }
        Fact::Looks(names) if names.len() == 1 => format!("Adds a look you can put on clips: {}.", names[0]),
        Fact::Looks(names) => format!("Adds {} looks you can put on clips: {}.", names.len(), listed(&names)),
        Fact::Reads(need) => need_line(need).into(),
        Fact::Sealed => "Can't reach the internet, your files or other apps.".into(),
        Fact::Preview => {
            "Installing runs nothing. Opening it shows a preview: its script runs without changing your project.".into()
        }
        Fact::OnPress => format!(
            "It changes your project only when you press {}, or by itself after import or before export if you turn \
             that on.",
            if manifest.actions.len() > 1 { "one of its buttons" } else { "its button" }
        ),
        Fact::OnClips => "Installing changes no project. Its looks show only on clips you put them on.".into(),
    }
}

/// What the plugin reads and does, in fixed sentences Nuzky builds from its manifest and files. With `had`, what the
/// installed version could do, each line it could not do is new.
fn summary(manifest: &Manifest, files: &[Listing], had: Option<&[Fact]>) -> Vec<Line> {
    facts(manifest, files.iter().any(|file| file.path == SCRIPT))
        .into_iter()
        .map(|fact| {
            let new = had.is_some_and(|had| !had.contains(&fact));
            Line { text: sentence(fact, manifest, files), new }
        })
        .collect()
}

/// What the installed copy can do; one whose manifest no longer reads can only do what every plugin can.
fn installed_facts(folder: &Path) -> Vec<Fact> {
    std::fs::read(folder.join("plugin.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| plugins::manifest(&value).ok())
        .map_or_else(|| ALWAYS.to_vec(), |manifest| facts(&manifest, folder.join(SCRIPT).is_file()))
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Provenance {
    commit: String,
    /// Each file's SHA-256, by its path in the plugin's folder.
    files: BTreeMap<String, String>,
}

type Record = BTreeMap<String, Provenance>;

fn root(plugins: &Path) -> Result<&Path> {
    plugins.parent().context("STORE_UNAVAILABLE: the plugins folder has no parent")
}

/// An unreadable record counts as none, so a plugin it named needs the user's consent to replace.
fn record(plugins: &Path) -> Record {
    let path = root(plugins).map(|root| root.join(RECORD));
    path.ok()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_record(plugins: &Path, record: &Record) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(record)?;
    crate::sounds::publish(&root(plugins)?.join(RECORD), |file| Ok(file.write_all(&bytes)?))
}

/// Forgets that the plugin came from the gallery, once it is gone.
pub fn forget(plugins: &Path, id: &str) -> Result<()> {
    let mut record = record(plugins);
    if record.remove(id).is_some() { save_record(plugins, &record) } else { Ok(()) }
}

fn sha256(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let (mut hash, mut buffer) = (Sha256::new(), vec![0; 1 << 16]);
    loop {
        match file.read(&mut buffer).ok()? {
            0 => break,
            read => hash.update(&buffer[..read]),
        }
    }
    Some(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The folder's files by path, at most one folder deep, or None when it holds anything else, a link included.
fn files_in(folder: &Path, prefix: &str, depth: usize, found: &mut BTreeMap<String, PathBuf>) -> Option<()> {
    for entry in std::fs::read_dir(folder).ok()? {
        let entry = entry.ok()?;
        let name = format!("{prefix}{}", entry.file_name().to_str()?);
        let kind = entry.file_type().ok()?;
        if kind.is_file() {
            found.insert(name, entry.path());
        } else if kind.is_dir() && depth > 0 {
            files_in(&entry.path(), &format!("{name}/"), depth - 1, found)?;
        } else {
            return None;
        }
    }
    Some(())
}

/// Whether the folder holds exactly the files Nuzky installed there.
fn unchanged(folder: &Path, had: &Provenance) -> bool {
    let mut found = BTreeMap::new();
    files_in(folder, "", 1, &mut found).is_some()
        && found.keys().eq(had.files.keys())
        && found.iter().all(|(name, path)| sha256(path).as_ref() == had.files.get(name))
}

fn same_files(had: &Provenance, files: &[Listing]) -> bool {
    had.files.len() == files.len() && files.iter().all(|f| had.files.get(&f.path) == Some(&f.sha256))
}

fn state_of(plugins: &Path, record: &Record, id: &str, files: &[Listing]) -> State {
    match std::fs::symlink_metadata(plugins.join(id)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return State::Available,
        Ok(meta) if meta.is_dir() => {}
        _ => return State::Clash,
    }
    match record.get(id) {
        None => State::Clash,
        Some(had) if !unchanged(&plugins.join(id), had) => State::Changed,
        Some(had) if same_files(had, files) => State::Installed,
        Some(_) => State::Update,
    }
}

fn installed_version(folder: &Path) -> Option<String> {
    let text = std::fs::read_to_string(folder.join("plugin.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    Some(value.get("version")?.as_str()?.chars().take(20).collect())
}

/// One line of the author's text, as the list shows it.
fn line(text: &str, max: usize) -> String {
    let text: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// Every plugin of the gallery, with what installing it would do to the plugins folder.
pub fn entries(gallery: &Gallery, plugins: &Path) -> Vec<Entry> {
    let record = record(plugins);
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for listed in &gallery.plugins {
        let Some(id) = listed.manifest.get("id").and_then(Value::as_str).filter(|id| plugins::is_id(id)) else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        let text =
            |field: &str, max: usize| line(listed.manifest.get(field).and_then(Value::as_str).unwrap_or(""), max);
        let (state, summary, refused) = match check(listed) {
            Ok(manifest) => {
                let state = state_of(plugins, &record, id, &listed.files);
                // Replacing a copy installed from the gallery says what the new version can do that it couldn't.
                let had = matches!(state, State::Update | State::Changed).then(|| installed_facts(&plugins.join(id)));
                (state, summary(&manifest, &listed.files, had.as_deref()), None)
            }
            Err(refusal) if refusal.error.to_string().starts_with("PLUGIN_API_NEWER") => (State::Newer, vec![], None),
            Err(refusal) => {
                log::warn!("Gallery plugin {id} refused: {:#}", refusal.error);
                (State::Refused, vec![], refusal.file)
            }
        };
        let name = text("name", 40);
        entries.push(Entry {
            id: id.to_owned(),
            key: fingerprint(listed),
            name: if name.is_empty() { id.to_owned() } else { name },
            description: text("description", 300),
            version: text("version", 20),
            author: line(&listed.author, 60),
            summary,
            files: listed
                .files
                .iter()
                .map(|f| Shown {
                    path: line(&f.path, 130),
                    size: f.size,
                    refused: refused.as_deref() == Some(f.path.as_str()),
                })
                .collect(),
            size: listed.files.iter().map(|f| f.size).fold(0, u64::saturating_add),
            state,
            installed_version: (state != State::Available).then(|| installed_version(&plugins.join(id))).flatten(),
        });
    }
    entries
}

/// Moves a folder to the system's Trash.
pub type Trash = Arc<dyn Fn(&Path) -> Result<()> + Send + Sync>;

const RUNNING: u8 = 0;
const STOPPED: u8 = 1;
const SWAPPING: u8 = 2;

enum Step {
    Progress(f32),
    Done(Result<()>),
}

/// Installs the gallery's plugin `id`, as its entry with `key` showed it, into `plugins`. A plugin already there is
/// replaced only when `replacing` names its state as the user saw it; its folder goes to the Trash through `trash`.
/// Nothing is requested before every name and size is checked, the files are written into a folder of Nuzky's own
/// beside `plugins` and moved into place only once each matches the list, and `cancelled` stops this within 100 ms,
/// whether the server is silent or still sending.
#[allow(clippy::too_many_arguments)]
pub fn install(
    gallery: &Gallery,
    id: &str,
    key: &str,
    replacing: Option<State>,
    plugins: &Path,
    trash: Trash,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(f32),
) -> Result<()> {
    let listed = gallery
        .plugins
        .iter()
        .find(|listed| listed.manifest.get("id").and_then(Value::as_str) == Some(id))
        .context("GALLERY_GONE: The gallery no longer lists this plugin.")?
        .clone();
    ensure!(fingerprint(&listed) == key, STALE);
    checked(&listed)?;
    ensure!(
        std::fs::symlink_metadata(plugins).map_or(true, |meta| meta.is_dir()),
        "STORE_WRITE_FAILED: The plugins folder is a link, so Nuzky won't install into it."
    );
    let folder = plugins.join(id);
    ensure!(
        std::fs::symlink_metadata(&folder).map_or(true, |meta| meta.is_dir()),
        "STORE_WRITE_FAILED: plugins/{id} is a link or a file, so Nuzky won't replace it."
    );
    let state = state_of(plugins, &record(plugins), id, &listed.files);
    match state {
        State::Available => {}
        State::Installed => bail!("GALLERY_INSTALLED: This version is already installed."),
        _ if replacing == Some(state) => {}
        _ => bail!("GALLERY_CHANGED: The plugin changed on this computer. Look again before replacing it."),
    }
    std::fs::create_dir_all(plugins).context(FAILED)?;
    let staging = root(plugins)?.join(format!("{STAGING}{id}-{}", new_id()));
    std::fs::create_dir(&staging).context(FAILED)?;

    let flag = Arc::new(AtomicU8::new(RUNNING));
    let (tx, rx) = mpsc::channel();
    let (gallery, plugins, worker_flag) = (gallery.clone(), plugins.to_owned(), flag.clone());
    let worker = move || {
        let steps = tx.clone();
        let result = download(&gallery, &listed, &staging, &worker_flag, |p| drop(steps.send(Step::Progress(p))))
            .and_then(|()| {
                ensure!(
                    worker_flag.compare_exchange(RUNNING, SWAPPING, Ordering::SeqCst, Ordering::SeqCst).is_ok(),
                    "CANCELLED: the install stopped"
                );
                swap(&gallery, &listed, &plugins, &staging, state, trash.as_ref())
            });
        if staging.exists() {
            std::fs::remove_dir_all(&staging).ok();
        }
        tx.send(Step::Done(result)).ok();
    };
    std::thread::Builder::new().name("gallery-install".into()).spawn(worker).context(FAILED)?;
    loop {
        let step = rx.recv_timeout(Duration::from_millis(100));
        if let Ok(Step::Done(result)) = step {
            return result;
        }
        // Once the files are being moved into place, the install finishes.
        if cancelled() && flag.compare_exchange(RUNNING, STOPPED, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            bail!("CANCELLED: the install stopped");
        }
        match step {
            Ok(Step::Progress(p)) => progress(p),
            Err(RecvTimeoutError::Disconnected) => bail!("{FAILED}: the install stopped unexpectedly"),
            _ => {}
        }
    }
}

fn download(
    gallery: &Gallery,
    listed: &Listed,
    staging: &Path,
    flag: &AtomicU8,
    mut progress: impl FnMut(f32),
) -> Result<()> {
    let id = listed.manifest["id"].as_str().context("an id")?;
    let total = listed.files.iter().map(|f| f.size).sum::<u64>().max(1);
    let mut done = 0;
    let mut buffer = vec![0; 1 << 16];
    for file in &listed.files {
        let stopped = || flag.load(Ordering::SeqCst) != RUNNING;
        ensure!(!stopped(), "CANCELLED: the install stopped");
        let body = BODY_GRACE + Duration::from_secs(file.size / SLOWEST);
        let response = get(
            &agent(&gallery.repo, None, Some(body)),
            &format!("{}/{}/plugins/{id}/{}", gallery.repo, gallery.commit, file.path),
        )
        .map_err(|error| if error.to_string().starts_with("GALLERY_FAILED") { anyhow!(CORRUPT) } else { error })?;
        let path = staging.join(&file.path);
        if let Some((folder, _)) = file.path.split_once('/') {
            match std::fs::create_dir(staging.join(folder)) {
                Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => return Err(error).context(FAILED),
                _ => {}
            }
        }
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).context(FAILED)?;
        let mut reader = response.into_body().into_reader().take(file.size + 1);
        let (mut hash, mut length) = (Sha256::new(), 0u64);
        loop {
            ensure!(!stopped(), "CANCELLED: the install stopped");
            let read = reader
                .read(&mut buffer)
                .map_err(|_| anyhow!("OFFLINE: The download stopped. Check your internet connection."))?;
            if read == 0 {
                break;
            }
            length += read as u64;
            ensure!(length <= file.size, CORRUPT);
            hash.update(&buffer[..read]);
            out.write_all(&buffer[..read]).context(FAILED)?;
            done += read as u64;
            progress((done as f32 / total as f32).min(1.0));
        }
        let sum: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        ensure!(length == file.size && sum == file.sha256, CORRUPT);
        out.sync_all().context(FAILED)?;
    }
    // The list shows what the plugin does from its manifest, so the plugin must be the one it describes.
    let downloaded: Value = serde_json::from_slice(&std::fs::read(staging.join("plugin.json")).context(FAILED)?)
        .map_err(|_| anyhow!(CORRUPT))?;
    ensure!(downloaded == listed.manifest, CORRUPT);
    // As save_plugin does, an effect Nuzky could not draw is refused; checking reads the WGSL and runs nothing.
    for effect in checked(listed)?.effects {
        let file = effect_file(&effect.id);
        let source = std::fs::read_to_string(staging.join(&file)).map_err(|_| anyhow!(CORRUPT))?;
        plugins::effect_problem(&file, &source).map_err(|why| {
            anyhow!("GALLERY_BROKEN: Nuzky won't install this plugin. Its look {} has an error: {why}", effect.name)
        })?;
    }
    Ok(())
}

/// Puts the checked files in place of the plugin's folder, whose old version goes to the Trash.
fn swap(
    gallery: &Gallery,
    listed: &Listed,
    plugins: &Path,
    staging: &Path,
    seen: State,
    trash: &dyn Fn(&Path) -> Result<()>,
) -> Result<()> {
    let id = listed.manifest["id"].as_str().context("an id")?;
    let folder = plugins.join(id);
    let mut record = record(plugins);
    ensure!(
        state_of(plugins, &record, id, &listed.files) == seen,
        "GALLERY_CHANGED: The plugin changed on this computer while it downloaded. Look again before replacing it."
    );
    if seen != State::Available {
        ensure!(
            std::fs::symlink_metadata(&folder).is_ok_and(|meta| meta.is_dir()),
            "STORE_WRITE_FAILED: plugins/{id} is a link or a file, so Nuzky won't replace it."
        );
        trash(&folder).map_err(|e| anyhow!("STORE_WRITE_FAILED: Couldn't move your version to the Trash: {e:#}"))?;
    }
    std::fs::rename(staging, &folder).context(FAILED)?;
    // The plugin is in place, so the install succeeded; without the record the gallery only asks before replacing it.
    let files = listed.files.iter().map(|f| (f.path.clone(), f.sha256.clone())).collect();
    record.insert(id.to_owned(), Provenance { commit: gallery.commit.clone(), files });
    if let Err(error) = save_record(plugins, &record) {
        log::warn!("The gallery's record doesn't name {id}: {error:#}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::time::Instant;

    type Files = Vec<(String, Vec<u8>)>;
    type Alter = fn(&mut Vec<u8>);
    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    /// A top level that does not parse: installing must not run it, or it would fail.
    const BROKEN_SCRIPT: &[u8] = b"function run(ctx) { return [ ;";

    fn hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    fn manifest(id: &str) -> Value {
        serde_json::json!({"nuzkyPlugin": 2, "id": id, "name": "Beep", "version": "1.0.0", "action": "Beep",
            "description": "AUTHOR SAYS: the best plugin ever, trust me.", "needs": ["words", "beats"],
            "files": ["beep.wav", "logo.png"]})
    }

    /// The plugin's files as the gallery lists and serves them.
    fn plugin(id: &str) -> (Value, Files) {
        let json = serde_json::to_vec(&manifest(id)).unwrap();
        let files = vec![
            ("plugin.json".to_owned(), json),
            ("plugin.js".to_owned(), BROKEN_SCRIPT.to_vec()),
            ("beep.wav".to_owned(), vec![7; 3000]),
            ("logo.png".to_owned(), vec![9; 1500]),
        ];
        (manifest(id), files)
    }

    fn gallery_of(repo: &str, entries: &[(Value, Files)]) -> Gallery {
        let plugins = entries
            .iter()
            .map(|(manifest, files)| Listed {
                author: "Ann".into(),
                manifest: manifest.clone(),
                files: files
                    .iter()
                    .map(|(path, bytes)| Listing { path: path.clone(), size: bytes.len() as u64, sha256: hex(bytes) })
                    .collect(),
            })
            .collect();
        Gallery { repo: repo.into(), commit: COMMIT.into(), plugins }
    }

    /// A loopback server that answers each path with its bytes and records every request line it gets.
    struct Server {
        repo: String,
        files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl Server {
        fn start(id: &str, files: &[(String, Vec<u8>)]) -> Server {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let repo = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
            let served: BTreeMap<_, _> =
                files.iter().map(|(path, bytes)| (format!("/{COMMIT}/plugins/{id}/{path}"), bytes.clone())).collect();
            let (files, requests) = (Arc::new(Mutex::new(served)), Arc::new(Mutex::new(Vec::new())));
            let (served, seen) = (files.clone(), requests.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let path = request_path(&stream);
                    seen.lock().unwrap().push(path.clone());
                    let body = served.lock().unwrap().get(&path).cloned();
                    let reply = match body {
                        Some(body) => {
                            let mut reply = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            )
                            .into_bytes();
                            reply.extend(body);
                            reply
                        }
                        None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                    };
                    stream.write_all(&reply).ok();
                }
            });
            Server { repo, files, requests }
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    /// Reads a request's head and gives its path.
    fn request_path(stream: &std::net::TcpStream) -> String {
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut first = String::new();
        reader.read_line(&mut first).ok();
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).unwrap_or(0) <= 2 {
                break;
            }
        }
        first.split(' ').nth(1).unwrap_or_default().to_owned()
    }

    /// A loopback server that answers `slow` with the headers for all of it, then sends its first `until` bytes in
    /// 64 KB pieces `pause` apart and goes silent for a minute; every other file comes at once.
    fn dripping(id: &str, files: &Files, slow: &str, pause: Duration, until: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let repo = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let served: BTreeMap<_, _> =
            files.iter().map(|(path, bytes)| (format!("/{COMMIT}/plugins/{id}/{path}"), bytes.clone())).collect();
        let slow = format!("/{COMMIT}/plugins/{id}/{slow}");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let path = request_path(&stream);
                let body = served.get(&path).cloned().unwrap_or_default();
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                if stream.write_all(head.as_bytes()).is_err() || path != slow {
                    stream.write_all(&body).ok();
                    continue;
                }
                for piece in body[..until.min(body.len())].chunks(1 << 16) {
                    if stream.write_all(piece).and_then(|()| stream.flush()).is_err() {
                        break;
                    }
                    std::thread::sleep(pause);
                }
                std::thread::sleep(Duration::from_secs(60));
            }
        });
        repo
    }

    /// Waits up to `limit` for the data folder to be as it was.
    fn settles(home: &Home, before: &BTreeMap<String, Vec<u8>>, limit: Duration) -> bool {
        let started = Instant::now();
        while home.snapshot() != *before && started.elapsed() < limit {
            std::thread::sleep(Duration::from_millis(50));
        }
        home.snapshot() == *before
    }

    /// A data folder with a plugins folder and a Trash of its own.
    struct Home(PathBuf);

    impl Home {
        fn new() -> Home {
            let dir = std::env::temp_dir().join(format!("nuzky-gallery-{}", new_id()));
            std::fs::create_dir_all(dir.join("data/plugins")).unwrap();
            std::fs::create_dir_all(dir.join("Trash")).unwrap();
            Home(dir)
        }
        fn plugins(&self) -> PathBuf {
            self.0.join("data/plugins")
        }
        fn trash(&self) -> Trash {
            let trash = self.0.join("Trash");
            Arc::new(move |path: &Path| {
                let to = trash.join(format!("{}-{}", path.file_name().unwrap().to_string_lossy(), new_id()));
                Ok(std::fs::rename(path, to)?)
            })
        }
        fn trashed(&self) -> Vec<PathBuf> {
            std::fs::read_dir(self.0.join("Trash")).unwrap().map(|e| e.unwrap().path()).collect()
        }
        /// Everything under the data folder, with each file's bytes.
        fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
            let mut found = BTreeMap::new();
            let mut stack = vec![self.0.join("data")];
            // A Stop's download may still be removing its staging folder, so what vanishes meanwhile is skipped.
            while let Some(dir) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path.strip_prefix(&self.0).unwrap().to_string_lossy().into_owned();
                    let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
                    if meta.is_dir() {
                        stack.push(path.clone());
                        found.insert(name + "/", vec![]);
                    } else {
                        found.insert(name, std::fs::read(&path).unwrap_or_default());
                    }
                }
            }
            found
        }
        fn install(&self, gallery: &Gallery, id: &str, replacing: Option<State>) -> Result<()> {
            install(gallery, id, &key(gallery, id), replacing, &self.plugins(), self.trash(), || false, |_| {})
        }
    }

    /// The key the app gets with the gallery's entry for `id`.
    fn key(gallery: &Gallery, id: &str) -> String {
        let listed = gallery.plugins.iter().find(|listed| listed.manifest["id"] == id);
        listed.map(fingerprint).unwrap_or_default()
    }

    impl Drop for Home {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn a_plugin_installs_without_running_its_code_and_records_its_files() {
        let (manifest, files) = plugin("beep");
        let server = Server::start("beep", &files);
        let home = Home::new();
        let gallery = gallery_of(&server.repo, &[(manifest, files.clone())]);
        assert_eq!(entries(&gallery, &home.plugins())[0].state, State::Available);
        let mut shown = vec![];
        install(
            &gallery,
            "beep",
            &key(&gallery, "beep"),
            None,
            &home.plugins(),
            home.trash(),
            || false,
            |p| shown.push(p),
        )
        .unwrap();
        for (path, bytes) in &files {
            assert_eq!(&std::fs::read(home.plugins().join("beep").join(path)).unwrap(), bytes, "{path}");
        }
        assert_eq!(shown.last(), Some(&1.0));
        let record = record(&home.plugins());
        assert_eq!(record["beep"].commit, COMMIT);
        assert_eq!(record["beep"].files["plugin.js"], hex(BROKEN_SCRIPT));
        // Only the plugin's own files, by paths Nuzky built, with no query.
        let mut asked = server.requests();
        asked.sort();
        let mut expected: Vec<_> = files.iter().map(|(path, _)| format!("/{COMMIT}/plugins/beep/{path}")).collect();
        expected.sort();
        assert_eq!(asked, expected);
        // Nothing of Nuzky's own is left beside the plugins folder.
        let left: Vec<_> = std::fs::read_dir(home.0.join("data")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left.len(), 2, "{left:?}");
        let entry = &entries(&gallery, &home.plugins())[0];
        assert_eq!((entry.state, entry.installed_version.as_deref()), (State::Installed, Some("1.0.0")));
        let error = home.install(&gallery, "beep", None).unwrap_err();
        assert!(error.to_string().starts_with("GALLERY_INSTALLED"), "{error:#}");
    }

    #[test]
    fn the_index_is_read_from_main_and_a_newer_one_asks_for_an_update() {
        let (manifest, files) = plugin("beep");
        let server = Server::start("beep", &files);
        let index = serde_json::json!({"nuzkyGallery": 1, "commit": COMMIT, "plugins": [
            {"author": "Ann", "manifest": manifest, "files": [{"path": "plugin.json", "size": 2, "sha256": hex(b"{}")}]}]});
        server.files.lock().unwrap().insert("/main/index.json".into(), serde_json::to_vec(&index).unwrap());
        let gallery = fetch_from(&server.repo).unwrap();
        assert_eq!((gallery.commit.as_str(), gallery.plugins.len()), (COMMIT, 1));
        assert_eq!(server.requests(), ["/main/index.json"]);
        let newer = parse(&server.repo, r#"{"nuzkyGallery": 2, "something": "else"}"#).unwrap_err();
        assert_eq!(newer.to_string(), "GALLERY_NEWER: Update Nuzky to see this gallery.");
        for commit in ["main", "0123456789ABCDEF0123456789abcdef01234567", "../../evil"] {
            let text = serde_json::json!({"nuzkyGallery": 1, "commit": commit, "plugins": []}).to_string();
            assert!(parse(&server.repo, &text).unwrap_err().to_string().starts_with("GALLERY_CORRUPT"), "{commit}");
        }
        let error = fetch_from("http://127.0.0.1:9/").unwrap_err();
        assert!(error.to_string().starts_with("OFFLINE"), "{error:#}");
    }

    #[test]
    fn each_bad_path_is_refused_before_any_request() {
        let bad = [
            "../plugin.js",
            "/etc/passwd",
            "a/b/c.png",
            "Logo.png",
            "logo .png",
            ".hidden.png",
            "a\\b.png",
            "con.png",
            "lpt1.wav",
            "nul",
            "logo.png.",
            "plugin.prev.js",
            "plugin.prev.json",
            "sub/logo.png",
            "readme.md",
            "Readme.md",
            "README.MD",
            "sub/README.md",
            "notes.txt",
            "",
        ];
        for path in bad {
            let (mut manifest, mut files) = plugin("beep");
            files.push((path.to_owned(), b"x".to_vec()));
            if path.ends_with(".png") || path.ends_with(".wav") {
                manifest["files"].as_array_mut().unwrap().push(path.into());
                files[0].1 = serde_json::to_vec(&manifest).unwrap();
            }
            let marked = assert_refused(manifest, files, path);
            assert!(marked.iter().all(|marked| marked == path), "{path:?}: {marked:?}");
        }
        let (manifest, mut files) = plugin("beep");
        files.push(files[3].clone());
        assert_refused(manifest.clone(), files, "a file listed twice");
        let (_, files) = plugin("beep");
        assert_refused(manifest.clone(), files[1..].to_vec(), "no plugin.json");
        let (_, files) = plugin("beep");
        assert_refused(
            manifest.clone(),
            files.iter().filter(|(p, _)| p != "logo.png").cloned().collect(),
            "a file the manifest names is missing",
        );
        let (_, files) = plugin("beep");
        assert_refused(
            manifest.clone(),
            files.iter().filter(|(p, _)| p != "plugin.js").cloned().collect(),
            "no plugin.js",
        );
    }

    /// The files the detail marks as why.
    fn assert_refused(manifest: Value, files: Files, case: &str) -> Vec<String> {
        let server = Server::start("beep", &files);
        let home = Home::new();
        let before = home.snapshot();
        let gallery = gallery_of(&server.repo, &[(manifest, files)]);
        let error = home.install(&gallery, "beep", None).unwrap_err();
        assert!(error.to_string().starts_with("GALLERY_REFUSED"), "{case}: {error:#}");
        let entry = entries(&gallery, &home.plugins());
        assert_eq!(entry.first().map(|e| e.state), Some(State::Refused), "{case}");

        assert!(server.requests().is_empty(), "{case}: {:?}", server.requests());
        assert_eq!(home.snapshot(), before, "{case}");
        entry[0].files.iter().filter(|f| f.refused).map(|f| f.path.clone()).collect()
    }

    #[test]
    fn a_refused_plugin_marks_the_file_that_is_why() {
        let (manifest, mut files) = plugin("beep");
        files.push(("notes.txt".into(), b"x".to_vec()));
        let gallery = gallery_of("http://127.0.0.1:9", &[(manifest, files)]);
        let entry = entries(&gallery, &Home::new().plugins()).remove(0);
        let marked: Vec<_> = entry.files.iter().filter(|f| f.refused).map(|f| f.path.as_str()).collect();
        assert_eq!((entry.state, marked), (State::Refused, vec!["notes.txt"]));
    }

    #[test]
    fn a_readme_at_the_top_installs_with_the_plugin() {
        let (manifest, mut files) = plugin("beep");
        files.push((README.into(), b"# Beep\n\nBeeps on every beat.".to_vec()));
        let server = Server::start("beep", &files);
        let home = Home::new();
        let gallery = gallery_of(&server.repo, &[(manifest, files.clone())]);
        assert_eq!(entries(&gallery, &home.plugins())[0].state, State::Available);
        home.install(&gallery, "beep", None).unwrap();
        assert_eq!(std::fs::read(home.plugins().join("beep/README.md")).unwrap(), files[4].1);
        assert_eq!(entries(&gallery, &home.plugins())[0].state, State::Installed);
        // Only within the size Nuzky reads a README.
        let (manifest, mut files) = plugin("beep");
        files.push((README.into(), vec![b'a'; plugins::MAX_TEXT_BYTES as usize + 1]));
        assert_eq!(assert_refused(manifest, files, "a README over 64 KB"), [README]);
    }

    #[test]
    fn a_declared_size_over_the_limits_makes_no_request() {
        let cases = [
            ("plugin.js", plugins::MAX_SCRIPT_BYTES + 1),
            ("plugin.json", plugins::MAX_TEXT_BYTES + 1),
            ("beep.wav", MAX_FILE_BYTES + 1),
        ];
        for (path, size) in cases {
            let (manifest, files) = plugin("beep");
            let server = Server::start("beep", &files);
            let home = Home::new();
            let mut gallery = gallery_of(&server.repo, &[(manifest, files)]);
            gallery.plugins[0].files.iter_mut().find(|f| f.path == path).unwrap().size = size;
            let error = home.install(&gallery, "beep", None).unwrap_err();
            assert!(error.to_string().starts_with("GALLERY_REFUSED"), "{path}: {error:#}");
            assert!(server.requests().is_empty(), "{path}");
        }
        // 3 files of 19 MB each pass one by one but not together.
        let (mut manifest, mut files) = plugin("beep");
        manifest["files"] = serde_json::json!(["a.wav", "b.wav", "c.wav"]);
        files.truncate(2);
        files[0].1 = serde_json::to_vec(&manifest).unwrap();
        let server = Server::start("beep", &files);
        let mut gallery = gallery_of(&server.repo, &[(manifest, files)]);
        for name in ["a.wav", "b.wav", "c.wav"] {
            gallery.plugins[0].files.push(Listing { path: name.into(), size: 19 << 20, sha256: hex(b"") });
        }
        let error = Home::new().install(&gallery, "beep", None).unwrap_err();
        assert!(error.to_string().contains("in all"), "{error:#}");
        assert!(server.requests().is_empty());
    }

    #[test]
    fn a_file_that_differs_from_the_list_installs_nothing() {
        let (manifest, files) = plugin("beep");
        let altered: [(&str, Alter); 3] = [
            ("flipped", |bytes| bytes[10] ^= 1),
            ("longer", |bytes| bytes.push(0)),
            ("shorter", |bytes| {
                bytes.pop();
            }),
        ];
        for (case, alter) in altered {
            let server = Server::start("beep", &files);
            let home = Home::new();
            let before = home.snapshot();
            let gallery = gallery_of(&server.repo, &[(manifest.clone(), files.clone())]);
            alter(server.files.lock().unwrap().get_mut(&format!("/{COMMIT}/plugins/beep/beep.wav")).unwrap());
            let error = home.install(&gallery, "beep", None).unwrap_err();
            assert!(error.to_string().starts_with("GALLERY_CORRUPT"), "{case}: {error:#}");
            assert_eq!(home.snapshot(), before, "{case}: the plugins folder and the data folder are as they were");
        }
        // A plugin.json that is not the manifest the list describes.
        let server = Server::start("beep", &files);
        let home = Home::new();
        let mut lying = manifest.clone();
        lying["description"] = "Something it does not do.".into();
        let gallery = gallery_of(&server.repo, &[(lying, files.clone())]);
        let error = home.install(&gallery, "beep", None).unwrap_err();
        assert!(error.to_string().starts_with("GALLERY_CORRUPT"), "{error:#}");
        assert!(!home.plugins().join("beep").exists());
    }

    #[test]
    fn the_summary_says_what_the_manifest_does_and_nothing_the_author_wrote() {
        let (manifest, files) = plugin("beep");
        let gallery = gallery_of("http://127.0.0.1:9", &[(manifest, files)]);
        let entry = entries(&gallery, &Home::new().plugins()).remove(0);
        let lines: Vec<_> = entry.summary.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            lines,
            [
                "Can change anything in this project, as one step you can undo.",
                "Adds 1 picture and 1 sound of its own to your video and Media (4 KB).",
                "Reads the words said in the video.",
                "Reads the beats of the music.",
                "Can't reach the internet, your files or other apps.",
                "Installing runs nothing. Opening it shows a preview: its script runs without changing your project.",
                "It changes your project only when you press its button, or by itself after import or before export if \
                 you turn that on.",
            ]
        );
        assert!(entry.summary.iter().all(|line| !line.new), "a plugin not installed has nothing new");
        assert!(lines.iter().all(|line| !line.contains("AUTHOR") && !line.contains("playhead")));
        assert!(entry.description.starts_with("AUTHOR SAYS"));
    }

    const VHS: &[u8] = b"fn effect(p: Pixel) -> vec4<f32> {\n    return vec4<f32>(p.color.rgb * 0.5, p.color.a);\n}\n";

    /// A plugin that only adds the look `vhs`, without plugin.js, as the guide's VHS example.
    fn look(wgsl: &[u8]) -> (Value, Files) {
        let manifest = serde_json::json!({"nuzkyPlugin": 2, "id": "vhs", "name": "VHS", "version": "1.0.0",
            "description": "A worn tape.", "effects": [{"id": "vhs", "name": "VHS"}]});
        let files = vec![
            ("plugin.json".to_owned(), serde_json::to_vec(&manifest).unwrap()),
            ("effects/vhs.wgsl".to_owned(), wgsl.to_vec()),
        ];
        (manifest, files)
    }

    #[test]
    fn a_plugin_that_only_adds_a_look_says_so_and_installs_its_checked_shader() {
        let (manifest, files) = look(VHS);
        let server = Server::start("vhs", &files);
        let gallery = gallery_of(&server.repo, &[(manifest.clone(), files.clone())]);
        let home = Home::new();
        let entry = entries(&gallery, &home.plugins()).remove(0);
        assert_eq!(entry.state, State::Available);
        let lines: Vec<_> = entry.summary.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            lines,
            [
                "Adds a look you can put on clips: VHS.",
                "Can't reach the internet, your files or other apps.",
                "Installing changes no project. Its looks show only on clips you put them on.",
            ]
        );
        home.install(&gallery, "vhs", None).unwrap();
        assert_eq!(std::fs::read(home.plugins().join("vhs/effects/vhs.wgsl")).unwrap(), VHS);
        assert!(!home.plugins().join("vhs/plugin.js").exists());

        // Without a script nothing would press a button, so a manifest with one is refused before any request.
        let (mut buttons, mut files) = look(VHS);
        buttons["actions"] = serde_json::json!([{"id": "go", "label": "Go"}]);
        files[0].1 = serde_json::to_vec(&buttons).unwrap();
        let refused = entries(&gallery_of(&server.repo, &[(buttons, files)]), &Home::new().plugins()).remove(0);
        assert_eq!(refused.state, State::Refused);
        // Only the files of the looks it names.
        let (manifest, mut files) = look(VHS);
        files.push(("effects/other.wgsl".into(), VHS.to_vec()));
        let other = entries(&gallery_of(&server.repo, &[(manifest, files)]), &Home::new().plugins()).remove(0);
        assert!(other.state == State::Refused && other.files[2].refused, "effects/other.wgsl");
    }

    #[test]
    fn a_look_nuzky_could_not_draw_is_not_installed() {
        let looped =
            b"fn effect(p: Pixel) -> vec4<f32> {\n    var c = p.color;\n    loop { break; }\n    return c;\n}\n";
        let (manifest, files) = look(looped);
        let server = Server::start("vhs", &files);
        let home = Home::new();
        let before = home.snapshot();
        let error = home.install(&gallery_of(&server.repo, &[(manifest, files)]), "vhs", None).unwrap_err();
        assert!(
            error.to_string().starts_with(
                "GALLERY_BROKEN: Nuzky won't install this plugin. Its look VHS has an error: effects/vhs.wgsl:3:5:"
            ),
            "{error:#}"
        );
        assert_eq!(home.snapshot(), before, "a refused look left something behind");
    }

    #[test]
    fn a_plugin_with_sections_and_several_buttons_installs_and_says_it_runs_from_them() {
        let (mut manifest, mut files) = plugin("beep");
        manifest["inputs"] = serde_json::json!([{"type": "toggle", "id": "loud", "label": "Loud", "default": false}]);
        manifest["sections"] = serde_json::json!([{"label": "Sound", "inputs": ["loud"]}]);
        manifest["actions"] = serde_json::json!([{"id": "beep", "label": "Beep"}, {"id": "mute", "label": "Mute"}]);
        files[0].1 = serde_json::to_vec(&manifest).unwrap();
        let server = Server::start("beep", &files);
        let gallery = gallery_of(&server.repo, &[(manifest, files)]);
        let home = Home::new();
        let entry = entries(&gallery, &home.plugins()).remove(0);
        assert_eq!(entry.state, State::Available);
        assert_eq!(
            entry.summary.last().unwrap().text,
            "It changes your project only when you press one of its buttons, or by itself after import or before \
             export if you turn that on."
        );
        home.install(&gallery, "beep", None).unwrap();
        assert!(home.plugins().join("beep/plugin.json").is_file());
    }

    #[test]
    fn one_picture_alone_is_said_with_its_size() {
        let (mut manifest, mut files) = plugin("beep");
        manifest["files"] = serde_json::json!(["logo.png"]);
        manifest["needs"] = serde_json::json!(["words"]);
        files.retain(|(path, _)| path != "beep.wav");
        files[0].1 = serde_json::to_vec(&manifest).unwrap();
        let gallery = gallery_of("http://127.0.0.1:9", &[(manifest, files)]);
        let entry = entries(&gallery, &Home::new().plugins()).remove(0);
        let lines: Vec<_> = entry.summary.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(lines[1], "Adds 1 picture of its own to your video and Media (1 KB).");
    }

    #[test]
    fn an_update_marks_what_the_installed_version_could_not_do() {
        let (manifest, files) = plugin("beep");
        let server = Server::start("beep", &files);
        let home = Home::new();
        home.install(&gallery_of(&server.repo, &[(manifest, files)]), "beep", None).unwrap();
        // 1.1 also reads silences and brings a second picture; it no longer reads beats.
        let (mut newer, mut files) = plugin("beep");
        newer["version"] = "1.1.0".into();
        newer["needs"] = serde_json::json!(["words", "silences"]);
        newer["files"] = serde_json::json!(["beep.wav", "logo.png", "logo2.png"]);
        files.push(("logo2.png".into(), vec![1; 10]));
        files[0].1 = serde_json::to_vec(&newer).unwrap();
        let gallery = gallery_of(&server.repo, &[(newer, files)]);
        let entry = entries(&gallery, &home.plugins()).remove(0);
        assert_eq!(entry.state, State::Update);
        let new: Vec<_> = entry.summary.iter().filter(|line| line.new).map(|line| line.text.as_str()).collect();
        assert_eq!(
            new,
            [
                "Adds 2 pictures and 1 sound of its own to your video and Media (4 KB).",
                "Reads where the sound goes quiet.",
            ]
        );
        // A copy edited until its manifest no longer reads could do only what every plugin can.
        std::fs::write(home.plugins().join("beep/plugin.json"), "{").unwrap();
        let entry = entries(&gallery, &home.plugins()).remove(0);
        assert_eq!(entry.state, State::Changed);
        let old: Vec<_> = entry.summary.iter().filter(|line| !line.new).map(|line| line.text.as_str()).collect();
        assert_eq!(
            old,
            [
                "Can change anything in this project, as one step you can undo.",
                "Can't reach the internet, your files or other apps.",
                "Installing runs nothing. Opening it shows a preview: its script runs without changing your project.",
                "It changes your project only when you press its button, or by itself after import or before export if \
                 you turn that on.",
            ]
        );
    }

    #[test]
    fn a_plugin_for_a_newer_nuzky_is_shown_but_not_installed() {
        let (mut manifest, files) = plugin("beep");
        manifest["nuzkyPlugin"] = (plugins::API + 1).into();
        let server = Server::start("beep", &files);
        let gallery = gallery_of(&server.repo, &[(manifest, files)]);
        let home = Home::new();
        assert_eq!(entries(&gallery, &home.plugins())[0].state, State::Newer);
        assert!(home.install(&gallery, "beep", None).unwrap_err().to_string().starts_with("PLUGIN_API_NEWER"));
        assert!(server.requests().is_empty());
    }

    #[test]
    fn replacing_needs_consent_and_the_old_folder_goes_to_the_trash() {
        let (manifest, files) = plugin("beep");
        let server = Server::start("beep", &files);
        let home = Home::new();
        let gallery = gallery_of(&server.repo, &[(manifest.clone(), files.clone())]);
        // The user's own plugin of the same id.
        let own = home.plugins().join("beep");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(own.join("plugin.js"), "mine").unwrap();
        assert_eq!(entries(&gallery, &home.plugins())[0].state, State::Clash);
        for consent in [None, Some(State::Update), Some(State::Changed)] {
            assert!(home.install(&gallery, "beep", consent).unwrap_err().to_string().starts_with("GALLERY_CHANGED"));
        }
        assert!(server.requests().is_empty() && home.trashed().is_empty());
        home.install(&gallery, "beep", Some(State::Clash)).unwrap();
        let trashed = home.trashed();
        assert_eq!(trashed.len(), 1);
        assert_eq!(std::fs::read_to_string(trashed[0].join("plugin.js")).unwrap(), "mine");
        assert_eq!(entries(&gallery, &home.plugins())[0].state, State::Installed);

        // Edited after installing.
        std::fs::write(own.join("plugin.js"), "changed by the AI").unwrap();
        assert_eq!(entries(&gallery, &home.plugins())[0].state, State::Changed);
        assert!(home.install(&gallery, "beep", None).is_err());
        home.install(&gallery, "beep", Some(State::Changed)).unwrap();
        assert_eq!(home.trashed().len(), 2);
        assert_eq!(std::fs::read(own.join("plugin.js")).unwrap(), BROKEN_SCRIPT);

        // A new version in the gallery, the installed one untouched.
        let (mut newer, mut files) = plugin("beep");
        newer["version"] = "1.1.0".into();
        files[0].1 = serde_json::to_vec(&newer).unwrap();
        files[1].1 = b"function run() { return []; }".to_vec();
        *server.files.lock().unwrap() =
            files.iter().map(|(path, bytes)| (format!("/{COMMIT}/plugins/beep/{path}"), bytes.clone())).collect();
        let gallery = gallery_of(&server.repo, &[(newer, files.clone())]);
        let entry = entries(&gallery, &home.plugins()).remove(0);
        assert_eq!((entry.state, entry.installed_version.as_deref()), (State::Update, Some("1.0.0")));
        assert!(home.install(&gallery, "beep", None).is_err());
        home.install(&gallery, "beep", Some(State::Update)).unwrap();
        assert_eq!(home.trashed().len(), 3);
        assert_eq!(std::fs::read(own.join("plugin.js")).unwrap(), files[1].1);
        assert_eq!(entries(&gallery, &home.plugins())[0].state, State::Installed);
        forget(&home.plugins(), "beep").unwrap();
        assert_eq!(entries(&gallery, &home.plugins())[0].state, State::Clash);
    }

    #[cfg(unix)]
    #[test]
    fn a_plugin_folder_that_is_a_link_is_refused_and_never_trashed_through() {
        let (manifest, files) = plugin("beep");
        let server = Server::start("beep", &files);
        let home = Home::new();
        let theirs = home.0.join("someone-elses");
        std::fs::create_dir_all(&theirs).unwrap();
        std::fs::write(theirs.join("precious.txt"), "keep me").unwrap();
        std::os::unix::fs::symlink(&theirs, home.plugins().join("beep")).unwrap();
        let gallery = gallery_of(&server.repo, &[(manifest, files)]);
        assert_eq!(entries(&gallery, &home.plugins())[0].state, State::Clash);
        let error = home.install(&gallery, "beep", Some(State::Clash)).unwrap_err();
        assert!(error.to_string().contains("is a link"), "{error:#}");
        assert!(server.requests().is_empty() && home.trashed().is_empty());
        assert_eq!(std::fs::read_to_string(theirs.join("precious.txt")).unwrap(), "keep me");
        assert!(std::fs::symlink_metadata(home.plugins().join("beep")).unwrap().is_symlink());
    }

    #[test]
    fn stop_ends_an_install_promptly_while_the_server_is_silent() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let repo = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        std::thread::spawn(move || {
            let held: Vec<_> = listener.incoming().take(1).collect();
            std::thread::sleep(Duration::from_secs(2));
            drop(held);
        });
        let (manifest, files) = plugin("beep");
        let gallery = gallery_of(&repo, &[(manifest, files)]);
        let home = Home::new();
        let before = home.snapshot();
        let started = Instant::now();
        let error = install(
            &gallery,
            "beep",
            &key(&gallery, "beep"),
            None,
            &home.plugins(),
            home.trash(),
            || started.elapsed() > Duration::from_millis(300),
            |_| {},
        )
        .unwrap_err();
        assert!(error.to_string().starts_with("CANCELLED"), "{error:#}");
        assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
        // Its folder goes once the connection ends, and nothing was installed.
        let gone = Instant::now();
        while home.snapshot() != before && gone.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(home.snapshot(), before);
    }

    #[test]
    fn stop_ends_an_install_while_the_server_still_sends() {
        let (manifest, mut files) = plugin("beep");
        files[2].1 = vec![7; MAX_FILE_BYTES as usize];
        // 20 MB in 64 KB pieces 20 ms apart: never 100 ms without news, and about 6 s in all.
        let repo = dripping("beep", &files, "beep.wav", Duration::from_millis(20), usize::MAX);
        let gallery = gallery_of(&repo, &[(manifest, files)]);
        let home = Home::new();
        let before = home.snapshot();
        let pressed = std::cell::Cell::new(None::<Instant>);
        let error = install(
            &gallery,
            "beep",
            &key(&gallery, "beep"),
            None,
            &home.plugins(),
            home.trash(),
            || pressed.get().is_some(),
            |done| {
                // Stop, once some of the large file has arrived.
                if done > 0.001 && pressed.get().is_none() {
                    pressed.set(Some(Instant::now()));
                }
            },
        )
        .expect_err("the install went on after Stop");
        assert!(error.to_string().starts_with("CANCELLED"), "{error:#}");
        let waited = pressed.get().unwrap().elapsed();
        assert!(waited < Duration::from_millis(500), "Stop took {waited:?}");
        assert!(settles(&home, &before, Duration::from_secs(5)), "something was installed or left behind");
    }

    #[test]
    fn a_server_gone_silent_mid_file_ends_the_download_and_leaves_nothing() {
        let (manifest, files) = plugin("beep");
        let silent = || dripping("beep", &files, "beep.wav", Duration::ZERO, 1000);
        let gallery = gallery_of(&silent(), &[(manifest.clone(), files.clone())]);
        let home = Home::new();
        let before = home.snapshot();
        let started = Instant::now();
        let error = home.install(&gallery, "beep", None).unwrap_err();
        assert!(error.to_string().starts_with("OFFLINE"), "{error:#}");
        assert!(started.elapsed() < BODY_GRACE + Duration::from_secs(3), "{:?}", started.elapsed());
        assert_eq!(home.snapshot(), before);

        // Stopped, the install ends at once and the download's own thread soon after, its folder with it.
        let gallery = gallery_of(&silent(), &[(manifest, files)]);
        let started = Instant::now();
        let stop = || started.elapsed() > Duration::from_millis(300);
        let error =
            install(&gallery, "beep", &key(&gallery, "beep"), None, &home.plugins(), home.trash(), stop, |_| {})
                .unwrap_err();
        assert!(error.to_string().starts_with("CANCELLED"), "{error:#}");
        assert!(settles(&home, &before, BODY_GRACE + Duration::from_secs(3)), "the stopped download left its folder");
    }

    #[test]
    fn a_replace_that_fails_keeps_the_record_of_the_version_installed() {
        let (manifest, files) = plugin("beep");
        let server = Server::start("beep", &files);
        let home = Home::new();
        home.install(&gallery_of(&server.repo, &[(manifest, files)]), "beep", None).unwrap();
        let (mut newer, mut files) = plugin("beep");
        newer["version"] = "1.1.0".into();
        files[0].1 = serde_json::to_vec(&newer).unwrap();
        *server.files.lock().unwrap() =
            files.iter().map(|(path, bytes)| (format!("/{COMMIT}/plugins/beep/{path}"), bytes.clone())).collect();
        let gallery = gallery_of(&server.repo, &[(newer, files)]);
        let before = home.snapshot();
        let full: Trash = Arc::new(|_: &Path| bail!("the Trash is full"));
        let error = install(
            &gallery,
            "beep",
            &key(&gallery, "beep"),
            Some(State::Update),
            &home.plugins(),
            full,
            || false,
            |_| {},
        )
        .unwrap_err();
        assert!(error.to_string().starts_with("STORE_WRITE_FAILED"), "{error:#}");
        assert_eq!(home.snapshot(), before, "the record or the installed version changed");
        assert_eq!(entries(&gallery, &home.plugins())[0].state, State::Update);
    }

    #[test]
    fn only_a_bare_loopback_address_stands_in_for_github() {
        assert_eq!(test_server("http://127.0.0.1:8000").as_deref(), Some("http://127.0.0.1:8000"));
        assert_eq!(test_server("http://localhost:8000/").as_deref(), Some("http://localhost:8000"));
        let others = [
            "http://127.0.0.1:8000@192.0.2.1",
            "http://user@127.0.0.1:8000",
            "http://127.0.0.1.example.com:8000",
            "http://192.0.2.1:8000",
            "https://127.0.0.1:8000",
            "http://127.0.0.1",
            "http://127.0.0.1:8000/elsewhere",
            "http://127.0.0.1:8000/?q",
            REPO,
        ];
        for url in others {
            assert_eq!(test_server(url), None, "{url}");
        }
    }

    #[test]
    fn a_plugin_named_like_a_windows_device_is_refused_before_any_request() {
        for id in ["aux", "con", "nul", "prn", "com1", "lpt9"] {
            let (manifest, files) = plugin(id);
            let server = Server::start(id, &files);
            let home = Home::new();
            let gallery = gallery_of(&server.repo, &[(manifest, files)]);
            assert_eq!(entries(&gallery, &home.plugins())[0].state, State::Refused, "{id}");
            let error = home.install(&gallery, id, None).unwrap_err();
            assert!(error.to_string().starts_with("GALLERY_REFUSED"), "{id}: {error:#}");
            assert!(server.requests().is_empty() && !home.plugins().join(id).exists(), "{id}");
        }
    }

    #[test]
    fn an_install_answers_only_to_the_entry_the_user_was_shown() {
        let (manifest, files) = plugin("beep");
        let server = Server::start("beep", &files);
        let shown = gallery_of(&server.repo, &[(manifest.clone(), files.clone())]);
        // The list read again meanwhile says the plugin also reads loudness.
        let (mut newer, mut newer_files) = plugin("beep");
        newer["needs"] = serde_json::json!(["words", "beats", "loudness"]);
        newer_files[0].1 = serde_json::to_vec(&newer).unwrap();
        let current = gallery_of(&server.repo, &[(newer, newer_files)]);
        let home = Home::new();
        let install_shown = |gallery: &Gallery| {
            install(gallery, "beep", &key(&shown, "beep"), None, &home.plugins(), home.trash(), || false, |_| {})
        };
        let error = install_shown(&current).unwrap_err();
        assert_eq!(error.to_string(), STALE);
        assert!(server.requests().is_empty() && !home.plugins().join("beep").exists());
        // The same entry in a list read again installs.
        install_shown(&gallery_of(&server.repo, &[(manifest, files)])).unwrap();
        assert!(home.plugins().join("beep/plugin.json").is_file());
    }
}

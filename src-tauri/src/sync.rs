//! Keeps Your style and My caption styles the same on every computer signed in to the same Nuzky
//! account. A computer sends its change to the account a few seconds after it, and looks for the
//! others' changes when Nuzky starts, when its window comes to the front, when Settings › Account
//! shows and every five minutes. When both changed the style meanwhile the newer change wins, and
//! the older one stays in Versions on the computer it was made on. Caption styles merge by name:
//! one added, changed or deleted on either side stays so, and when both changed the same style
//! this computer's wins. The account keeps every revision with the computer it came from, which
//! the History dialog lists and restores.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use nuzky_account::{Account, Remote, Revision, Stored};
use nuzky_engine::edit::CaptionPreset;
use nuzky_mcp::caption_styles;
use nuzky_mcp::style::{Shared, Store, StyleAction};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::{CmdResult, err};

const STYLE: &str = "style";
const CAPTIONS: &str = "caption-styles";
/// How often a computer looks for the others' changes while nothing else asks it to.
const EVERY: Duration = Duration::from_secs(5 * 60);
/// A change waits this long for the next one, so a burst of them is sent once.
const SETTLE: Duration = Duration::from_secs(2);
/// The window coming to the front looks for changes at most this often.
const FRONT_EVERY: Duration = Duration::from_secs(60);
/// Times a document is read again when someone stored another revision while this one was sent.
const ATTEMPTS: usize = 3;

/// What Settings › Account shows of syncing.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub syncing: bool,
    /// When this computer and the account last agreed, in milliseconds since 1970.
    pub synced_ms: Option<u64>,
    /// Why the last sync failed, in plain words.
    pub error: Option<String>,
}

static STATUS: Mutex<SyncStatus> = Mutex::new(SyncStatus { syncing: false, synced_ms: None, error: None });
static NUDGED: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
static FRONT: Mutex<Option<Instant>> = Mutex::new(None);

/// Syncs soon, a few seconds after the last call.
pub fn nudge() {
    *NUDGED.0.lock().unwrap() = true;
    NUDGED.1.notify_one();
}

/// Nuzky's window came to the front, where the user may have come from another computer.
pub fn came_to_front() {
    let mut last = FRONT.lock().unwrap();
    if last.is_none_or(|at| at.elapsed() >= FRONT_EVERY) {
        *last = Some(Instant::now());
        nudge();
    }
}

#[tauri::command]
pub fn sync_status() -> SyncStatus {
    STATUS.lock().unwrap().clone()
}

#[tauri::command]
pub fn sync_now() {
    nudge();
}

/// What restoring a revision does to one caption style.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptionChange {
    /// Not in My styles now; restoring brings it back.
    Back,
    /// Only in My styles now; restoring deletes it.
    Goes,
    /// Restoring changes how it looks.
    Changes,
    Same,
}

/// A revision as the History dialog previews it.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionView {
    pub revision: Revision,
    /// For Your style: EDIT.md in that revision and now; none without a style.
    pub text: Option<String>,
    pub now_text: Option<String>,
    /// For caption styles: every style in that revision or now, by name.
    pub caption_styles: Vec<(String, CaptionChange)>,
    /// Restoring it would change nothing.
    pub same: bool,
    /// Stands for the style or caption styles as this preview compared with, so Restore never
    /// replaces a change that came after it.
    pub seen: String,
}

/// What the Undo of the last restore puts back: the style's version before it, or My caption
/// styles as they were.
enum Before {
    Style(u64),
    Captions(Vec<CaptionPreset>),
}

static BEFORE: Mutex<Option<Before>> = Mutex::new(None);

/// The same text for the same content within this run of Nuzky.
fn fingerprint(content: &impl Serialize) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(content).unwrap_or_default().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// The style's fingerprint, whenever and by whom it changed.
fn style_fingerprint(style: &Shared) -> String {
    let mut style = style.clone();
    style.at_ms = 0;
    style.label.clear();
    fingerprint(&style)
}

fn kind(kind: &str) -> Result<&'static str> {
    [STYLE, CAPTIONS]
        .into_iter()
        .find(|known| *known == kind)
        .with_context(|| format!("INVALID_ARGUMENTS: no synced {kind}"))
}

/// The revisions of Your style (`style`) or My caption styles (`caption-styles`) the account keeps.
#[tauri::command]
pub async fn sync_history(kind: String) -> CmdResult<Vec<Revision>> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<Revision>> {
        let kind = self::kind(&kind)?;
        crate::account::account().history(kind)?.context("SIGNED_OUT: sign in to see the history")
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
pub async fn sync_revision(kind: String, revision: u64) -> CmdResult<RevisionView> {
    tauri::async_runtime::spawn_blocking(move || -> Result<RevisionView> {
        let kind = self::kind(&kind)?;
        let (revision, body) = kept(kind, revision)?;
        if kind == STYLE {
            let then: Shared = serde_json::from_value(body).context("NEWER_NUZKY: reading that version")?;
            let now = Store::default().shared()?;
            return Ok(RevisionView {
                revision,
                text: then.text().map(str::to_owned),
                now_text: now.text().map(str::to_owned),
                caption_styles: vec![],
                same: then.same(&now),
                seen: style_fingerprint(&now),
            });
        }
        let then = restorable(&body);
        let now = caption_styles::list()?;
        let key = |style: &CaptionPreset| caption_styles::normalized(&style.name);
        let mut changes: Vec<(String, CaptionChange)> = then
            .iter()
            .map(|style| {
                let change = match now.iter().find(|n| key(n) == key(style)) {
                    None => CaptionChange::Back,
                    Some(n) if n != style => CaptionChange::Changes,
                    Some(_) => CaptionChange::Same,
                };
                (style.name.clone(), change)
            })
            .collect();
        changes.extend(
            now.iter()
                .filter(|n| !then.iter().any(|t| key(t) == key(n)))
                .map(|n| (n.name.clone(), CaptionChange::Goes)),
        );
        let same = changes.iter().all(|(_, change)| *change == CaptionChange::Same);
        Ok(RevisionView {
            revision,
            text: None,
            now_text: None,
            caption_styles: changes,
            same,
            seen: fingerprint(&now),
        })
    })
    .await
    .map_err(err)?
    .map_err(err)
}

/// Makes a revision the style or My caption styles again, as a change of this computer's, which then
/// syncs to the others.
#[tauri::command]
pub async fn sync_restore(app: AppHandle, kind: String, revision: u64, seen: String) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || -> Result<()> {
        let kind = self::kind(&kind)?;
        let (shown, body) = kept(kind, revision)?;
        if kind == STYLE {
            let then: Shared = serde_json::from_value(body).context("NEWER_NUZKY: reading that version")?;
            let label =
                format!("Restored: {}", if shown.label.is_empty() { "an earlier version" } else { &shown.label });
            let store = Store::default();
            let now = store.shared()?;
            anyhow::ensure!(style_fingerprint(&now) == seen, "RESTORE_CHANGED: your style changed meanwhile");
            let before = store.view()?.version;
            anyhow::ensure!(store.take_shared(&then, &now, &label)?, "RESTORE_CHANGED: your style changed meanwhile");
            *BEFORE.lock().unwrap() = Some(Before::Style(before));
        } else {
            let now = caption_styles::list()?;
            anyhow::ensure!(fingerprint(&now) == seen, "RESTORE_CHANGED: your caption styles changed meanwhile");
            anyhow::ensure!(
                caption_styles::replace(&now, restorable(&body))?,
                "RESTORE_CHANGED: your caption styles changed meanwhile"
            );
            *BEFORE.lock().unwrap() = Some(Before::Captions(now));
            app.emit("caption-styles-changed", ()).ok();
        }
        nudge();
        Ok(())
    })
    .await
    .map_err(err)?
    .map_err(err)
}

/// Puts back what the last restore replaced on this computer, also what had not synced yet.
#[tauri::command]
pub async fn sync_undo_restore(app: AppHandle) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || -> Result<()> {
        let before = BEFORE.lock().unwrap().take().context("NOTHING_TO_UNDO: nothing was restored")?;
        match before {
            Before::Style(index) => {
                Store::default().act(StyleAction::Restore { index })?;
            }
            Before::Captions(styles) => {
                let now = caption_styles::list()?;
                anyhow::ensure!(
                    caption_styles::replace(&now, styles)?,
                    "RESTORE_CHANGED: your caption styles changed meanwhile"
                );
                app.emit("caption-styles-changed", ()).ok();
            }
        }
        nudge();
        Ok(())
    })
    .await
    .map_err(err)?
    .map_err(err)
}

/// A revision the account still keeps, with its body.
fn kept(kind: &str, revision: u64) -> Result<(Revision, Value)> {
    crate::account::account().revision(kind, revision)?.context("NOT_KEPT: the account no longer keeps that version")
}

/// The caption styles of a revision this version can read and use, in their order.
fn restorable(body: &Value) -> Vec<CaptionPreset> {
    let mut styles = Vec::new();
    for style in body.get("styles").and_then(Value::as_array).into_iter().flatten() {
        if let Ok(style) = serde_json::from_value::<CaptionPreset>(style.clone())
            && caption_styles::check(&style, &styles).is_ok()
        {
            styles.push(style);
        }
    }
    styles
}

/// Syncs on its own thread from now on, first at once, in builds that offer the account.
pub fn start(app: AppHandle, account: impl Fn() -> Account + Send + 'static) {
    if !crate::account::account_available() {
        return;
    }
    let spawned = std::thread::Builder::new().name("account-sync".into()).spawn(move || {
        nudge();
        loop {
            {
                let (nudged, woken) = &NUDGED;
                let guard = nudged.lock().unwrap();
                let (mut guard, _) = woken.wait_timeout_while(guard, EVERY, |nudged| !*nudged).unwrap();
                *guard = false;
            }
            std::thread::sleep(SETTLE);
            *NUDGED.0.lock().unwrap() = false;
            run(&app, &account());
        }
    });
    if let Err(error) = spawned {
        log::error!("Syncing with the Nuzky account could not start: {error}");
    }
}

fn run(app: &AppHandle, account: &Account) {
    let set = |change: &dyn Fn(&mut SyncStatus)| {
        let status = {
            let mut status = STATUS.lock().unwrap();
            change(&mut status);
            status.clone()
        };
        app.emit("sync-changed", status).ok();
    };
    let dir = crate::store::data_dir();
    // Another Nuzky with the same data folder syncs it meanwhile: this one waits.
    let lock = std::fs::create_dir_all(&dir)
        .and_then(|()| {
            std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("account-sync.lock"))
        })
        .and_then(|file| file.lock().map(|()| file));
    let _lock = match lock {
        Ok(lock) => lock,
        Err(error) => return log::warn!("Not syncing with the Nuzky account: {error}"),
    };
    set(&|s| s.syncing = true);
    match once(account, &dir) {
        Ok(None) => set(&|s| *s = SyncStatus::default()),
        Ok(Some(changed)) => {
            if changed.captions {
                app.emit("caption-styles-changed", ()).ok();
            }
            set(&|s| *s = SyncStatus { syncing: false, synced_ms: Some(now_ms()), error: None });
        }
        Err(error) => {
            log::warn!("Syncing with the Nuzky account failed: {error:#}");
            let error = Some(words(&format!("{error:#}")));
            set(&|s| *s = SyncStatus { syncing: false, error: error.clone(), ..s.clone() });
        }
    }
}

/// What failed, in words that say what to do next.
fn words(error: &str) -> String {
    let (code, rest) = error.split_once(": ").unwrap_or(("", error));
    match code {
        "NEWER_NUZKY" => "Another computer synced with a newer Nuzky. Update Nuzky to keep syncing.".into(),
        "TOO_LARGE" => "Your style is larger than the 256 KB your account keeps. Shorten EDIT.md to sync it.".into(),
        "STORE_UNREADABLE" => {
            "Nuzky can't read your caption styles on this computer, so it doesn't sync them. Save a caption style to replace the file.".into()
        }
        "SYNC_FAILED" | "SIGNED_OUT" => "Nuzky couldn't reach your account. It tries again in a few minutes.".into(),
        _ => format!("Syncing stopped: {}. Nuzky tries again in a few minutes.", if code.is_empty() { rest } else { error }),
    }
}

/// What this computer kept of the account, to tell its own changes from the others'.
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct State {
    /// The account it is of; another account starts afresh.
    subject: String,
    style: Option<Kept<Shared>>,
    caption_styles: Option<Kept<Vec<Value>>>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Kept<T> {
    revision: u64,
    /// The document as both had it then.
    content: T,
}

impl State {
    fn path(dir: &std::path::Path) -> PathBuf {
        dir.join("account-sync.json")
    }

    fn load(dir: &std::path::Path) -> Self {
        std::fs::read(Self::path(dir)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    fn save(&self, dir: &std::path::Path) -> Result<()> {
        nuzky_session::write_json_atomic(&Self::path(dir), self).context("Saving what was synced")
    }
}

struct Changed {
    captions: bool,
}

/// One sync of both documents; none when signed out.
fn once(account: &Account, dir: &std::path::Path) -> Result<Option<Changed>> {
    let mut state = State::load(dir);
    let known = |state: &State| {
        let mut known = vec![];
        known.extend(state.style.as_ref().map(|kept| (STYLE, kept.revision)));
        known.extend(state.caption_styles.as_ref().map(|kept| (CAPTIONS, kept.revision)));
        known
    };
    let Some(mut documents) = account.documents(&known(&state))? else { return Ok(None) };
    if documents.subject != state.subject {
        let asked = !known(&state).is_empty();
        state = State { subject: documents.subject.clone(), ..State::default() };
        if asked {
            let Some(fresh) = account.documents(&[])? else { return Ok(None) };
            documents = fresh;
        }
    }
    let remote = |kind: &str| documents.kinds.get(kind).cloned().unwrap_or(Remote { revision: 0, body: None });

    let style = sync_style(account, &documents.subject, &mut state.style, remote(STYLE));
    state.save(dir)?;
    let captions = sync_captions(account, &documents.subject, &mut state.caption_styles, remote(CAPTIONS));
    state.save(dir)?;
    style?;
    Ok(Some(Changed { captions: captions? }))
}

/// The document's body as this computer reads it: the one it kept when the revision is the same.
fn body<T: Clone + for<'de> Deserialize<'de>>(remote: &Remote, kept: &Option<Kept<T>>) -> Result<Option<T>> {
    match (&remote.body, kept) {
        (Some(Value::Null), _) | (None, _) if remote.revision == 0 => Ok(None),
        (Some(body), _) => {
            serde_json::from_value(body.clone()).map(Some).context("NEWER_NUZKY: reading the account's document")
        }
        (None, Some(kept)) if kept.revision == remote.revision => Ok(Some(kept.content.clone())),
        (None, _) => bail!("SYNC_FAILED: the account left out a document this computer doesn't have"),
    }
}

fn store(account: &Account, subject: &str, kind: &str, revision: u64, body: &Value, label: &str) -> Result<Stored> {
    account
        .store(subject, kind, revision, body, computer(), label)?
        .context("SIGNED_OUT: signed out or into another account while syncing")
}

/// This computer's name as the user sees it, which the history shows beside each change.
fn computer() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        #[cfg(target_os = "macos")]
        let name = std::process::Command::new("scutil")
            .args(["--get", "ComputerName"])
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned());
        #[cfg(target_os = "windows")]
        let name = std::env::var("COMPUTERNAME").ok();
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let name = std::fs::read_to_string("/etc/hostname").ok();
        name.map(|name| name.trim().to_owned()).filter(|name| !name.is_empty()).unwrap_or_else(|| "A computer".into())
    })
}

fn sync_style(account: &Account, subject: &str, kept: &mut Option<Kept<Shared>>, mut remote: Remote) -> Result<()> {
    let styles = Store::default();
    for _ in 0..ATTEMPTS {
        // The account lost what this computer synced, as after a restore: start afresh.
        if kept.as_ref().is_some_and(|kept| kept.revision > remote.revision) {
            *kept = None;
        }
        let local = styles.shared()?;
        let theirs = body::<Shared>(&remote, kept)?;
        let base = kept.as_ref().map(|kept| kept.revision).unwrap_or(0);
        let mine_changed = kept.as_ref().map_or(!local.is_default(), |kept| !local.same(&kept.content));
        let theirs_changed = remote.revision != base;
        let take = match (&theirs, mine_changed, theirs_changed) {
            (_, false, false) => return Ok(()),
            (Some(theirs), _, true) if theirs.same(&local) => {
                *kept = Some(Kept { revision: remote.revision, content: local });
                return Ok(());
            }
            (Some(theirs), false, true) => Some(theirs.clone()),
            (Some(theirs), true, true) if theirs.at_ms > local.at_ms => Some(theirs.clone()),
            _ => None,
        };
        match take {
            Some(theirs) => {
                if styles.take_shared(&theirs, &local, "From another computer")? {
                    *kept = Some(Kept { revision: remote.revision, content: theirs });
                    return Ok(());
                }
            }
            None => {
                match store(account, subject, STYLE, remote.revision, &serde_json::to_value(&local)?, &local.label)? {
                    Stored::Revision(revision) => {
                        *kept = Some(Kept { revision, content: local });
                        return Ok(());
                    }
                    Stored::Conflict(now) => remote = now,
                }
            }
        }
    }
    bail!("SYNC_FAILED: your style kept changing while it synced")
}

/// True when this computer's caption styles changed.
fn sync_captions(
    account: &Account,
    subject: &str,
    kept: &mut Option<Kept<Vec<Value>>>,
    mut remote: Remote,
) -> Result<bool> {
    #[derive(Clone, Deserialize)]
    struct Body {
        styles: Vec<Value>,
    }
    // What the next attempt merges against when writing here was refused: the list this computer had.
    let mut base: Option<Vec<Value>> = None;
    for _ in 0..ATTEMPTS {
        if kept.as_ref().is_some_and(|kept| kept.revision > remote.revision) {
            *kept = None;
        }
        let local = caption_styles::list()?;
        let theirs = match &remote.body {
            Some(body) if remote.revision > 0 => {
                serde_json::from_value::<Body>(body.clone())
                    .context("NEWER_NUZKY: reading the account's caption styles")?
                    .styles
            }
            _ => body::<Vec<Value>>(&remote, kept)?.unwrap_or_default(),
        };
        let agreed = kept.as_ref().map_or(&[][..], |kept| &kept.content[..]);
        let (merged, mine) = merge(base.as_deref().unwrap_or(agreed), &local, &theirs);
        if merged != theirs {
            let label = described(&theirs, &merged);
            match store(account, subject, CAPTIONS, remote.revision, &json!({ "styles": merged }), &label)? {
                Stored::Revision(revision) => remote = Remote { revision, body: Some(json!({ "styles": merged })) },
                Stored::Conflict(now) => {
                    remote = now;
                    continue;
                }
            }
        }
        if mine == local {
            *kept = Some(Kept { revision: remote.revision, content: merged });
            return Ok(false);
        }
        if caption_styles::replace(&local, mine)? {
            *kept = Some(Kept { revision: remote.revision, content: merged });
            return Ok(true);
        }
        // Changed here meanwhile: the account has the merge, so merge again against the list this
        // computer had then. What came from the account stays, and this computer's new change, a
        // style renamed or deleted since, applies to it.
        base = Some(local.iter().filter_map(|style| serde_json::to_value(style).ok()).collect());
        remote = Remote { revision: remote.revision, body: Some(json!({ "styles": merged })) };
    }
    bail!("SYNC_FAILED: your caption styles kept changing while they synced")
}

/// The caption styles of the account and of this computer after merging `local` and `remote`, both
/// changed from `base` by name. A style this version cannot read stays in the account as it is.
fn merge(base: &[Value], local: &[CaptionPreset], remote: &[Value]) -> (Vec<Value>, Vec<CaptionPreset>) {
    let key = |style: &Value| style.get("name").and_then(Value::as_str).map(caption_styles::normalized);
    // As this version reads a style, so fields it doesn't know never count as a change.
    let read = |style: &Value| {
        serde_json::from_value::<CaptionPreset>(style.clone()).ok().and_then(|s| serde_json::to_value(s).ok())
    };
    let base: HashMap<String, Option<Value>> =
        base.iter().filter_map(|style| Some((key(style)?, read(style)))).collect();
    let local: Vec<(String, Value)> = local
        .iter()
        .filter_map(|style| Some((caption_styles::normalized(&style.name), serde_json::to_value(style).ok()?)))
        .collect();
    let mine = |name: &str| local.iter().find(|(key, _)| key == name).map(|(_, style)| style.clone());

    let mut merged = Vec::new();
    let mut seen = HashSet::new();
    for style in remote {
        let Some(name) = key(style) else {
            merged.push(style.clone());
            continue;
        };
        if !seen.insert(name.clone()) {
            continue;
        }
        let before = base.get(&name).cloned().flatten();
        let here = mine(&name);
        if here != before && here != read(style) {
            merged.extend(here);
        } else {
            merged.push(style.clone());
        }
    }
    for (name, style) in &local {
        let deleted_there = base.get(name).is_some_and(|before| before.as_ref() == Some(style));
        if seen.insert(name.clone()) && !deleted_there {
            merged.push(style.clone());
        }
    }

    let mut mine = Vec::new();
    for style in &merged {
        let Ok(style) = serde_json::from_value::<CaptionPreset>(style.clone()) else { continue };
        if caption_styles::check(&style, &mine).is_ok() {
            mine.push(style);
        }
    }
    (merged, mine)
}

/// What changed between two lists of caption styles: "Added Team yellow", "Deleted Mine, changed Bold pop".
fn described(before: &[Value], after: &[Value]) -> String {
    let name = |style: &Value| style.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
    let find = |list: &[Value], wanted: &str| {
        list.iter()
            .find(|style| caption_styles::normalized(&name(style)) == caption_styles::normalized(wanted))
            .cloned()
    };
    let mut parts = Vec::new();
    for (verb, names) in [
        ("added", after.iter().map(name).filter(|n| find(before, n).is_none()).collect::<Vec<_>>()),
        ("deleted", before.iter().map(name).filter(|n| find(after, n).is_none()).collect()),
        ("changed", after.iter().filter(|s| find(before, &name(s)).is_some_and(|b| b != **s)).map(name).collect()),
    ] {
        if !names.is_empty() {
            parts.push(format!("{verb} {}", names.join(", ")));
        }
    }
    let text = parts.join("; ");
    let mut chars = text.chars();
    chars.next().map_or_else(|| "Caption styles".into(), |first| first.to_uppercase().chain(chars).collect())
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuzky_engine::edit::caption_preset;

    fn style(name: &str, color: &str) -> CaptionPreset {
        let mut style = caption_preset("hormozi").unwrap().clone();
        style.name = name.into();
        style.style.color = color.into();
        style
    }

    fn value(style: &CaptionPreset) -> Value {
        serde_json::to_value(style).unwrap()
    }

    fn names(styles: &[Value]) -> Vec<String> {
        styles
            .iter()
            .map(|s| format!("{} {}", s["name"].as_str().unwrap(), s["style"]["color"].as_str().unwrap()))
            .collect()
    }

    #[test]
    fn merging_keeps_what_each_side_added_changed_or_deleted() {
        let (a, b, c) = (style("A", "#ffffff"), style("B", "#ffffff"), style("C", "#ffffff"));
        let base = [value(&a), value(&b), value(&c)];
        // Here: A changed, C deleted, D added. There: B changed, E added.
        let local = [style("A", "#ff0000"), b.clone(), style("D", "#ffffff")];
        let remote = [value(&a), value(&style("B", "#00ff00")), value(&c), value(&style("E", "#ffffff"))];
        let (merged, mine) = merge(&base, &local, &remote);
        assert_eq!(names(&merged), ["A #ff0000", "B #00ff00", "E #ffffff", "D #ffffff"]);
        assert_eq!(mine.iter().map(value).collect::<Vec<_>>(), merged);
    }

    #[test]
    fn merging_a_style_deleted_there_and_unchanged_here_deletes_it() {
        let (a, b) = (style("A", "#ffffff"), style("B", "#ffffff"));
        let (merged, mine) = merge(&[value(&a), value(&b)], &[a.clone(), b.clone()], &[value(&a)]);
        assert_eq!(names(&merged), ["A #ffffff"]);
        assert_eq!(mine, [a]);
    }

    #[test]
    fn merging_a_style_changed_on_both_sides_keeps_this_computers() {
        let a = style("A", "#ffffff");
        let (merged, _) = merge(&[value(&a)], &[style("A", "#ff0000")], &[value(&style("A", "#00ff00"))]);
        assert_eq!(names(&merged), ["A #ff0000"]);
        // Deleted there but changed here, it comes back.
        let (merged, _) = merge(&[value(&a)], &[style("A", "#ff0000")], &[]);
        assert_eq!(names(&merged), ["A #ff0000"]);
    }

    #[test]
    fn a_style_renamed_here_while_the_merge_was_sent_stays_renamed() {
        // The account has [A, B, D] from merging [A, D] here with B from there; D was renamed E meanwhile.
        let (a, b, d) = (style("A", "#ffffff"), style("B", "#ffffff"), style("D", "#ffffff"));
        let sent_from = [value(&a), value(&d)];
        let (merged, mine) = merge(&sent_from, &[a.clone(), style("E", "#ffffff")], &[value(&a), value(&b), value(&d)]);
        assert_eq!(names(&merged), ["A #ffffff", "B #ffffff", "E #ffffff"]);
        assert_eq!(mine.len(), 3);
    }

    #[test]
    fn merging_the_first_time_keeps_both_sides() {
        let (merged, mine) = merge(&[], &[style("Mine", "#ffffff")], &[value(&style("Theirs", "#ffffff"))]);
        assert_eq!(names(&merged), ["Theirs #ffffff", "Mine #ffffff"]);
        assert_eq!(mine.len(), 2);
    }

    #[test]
    fn a_style_from_a_newer_nuzky_stays_in_the_account_untouched() {
        let mut newer = value(&style("New", "#ffffff"));
        newer["style"]["sparkle"] = json!(true);
        let unreadable = json!({ "name": "Odd", "style": "made by a future Nuzky" });
        let a = style("A", "#ffffff");
        let base = [newer.clone(), unreadable.clone(), value(&a)];
        let (merged, mine) = merge(&base, &[style("New", "#ffffff"), style("A", "#ff0000")], &base);
        assert_eq!(merged, [newer, unreadable, value(&style("A", "#ff0000"))]);
        assert_eq!(mine.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["New", "A"]);
    }

    #[test]
    fn a_style_that_is_not_valid_here_stays_out_of_this_computer() {
        let mut broken = style("Broken", "#ffffff");
        broken.style.color = "#€".into();
        let (merged, mine) = merge(&[], &[], &[value(&broken), value(&style("Reel", "#ffffff"))]);
        assert_eq!(merged.len(), 2);
        assert!(mine.is_empty(), "an invalid colour and a built-in name stay out: {mine:?}");
    }
}

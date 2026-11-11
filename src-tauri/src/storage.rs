//! What Nuzky keeps on disk, by group, and clearing the groups Nuzky makes again on its own. Clearing touches only the
//! named folders of Nuzky's cache, never source media, projects or the data folder. It keeps what the open project
//! uses, so the preview, waveforms and filmstrips go on as they were, and leaves work another Nuzky process is doing.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use nuzky_engine::model::{Asset, AssetKind};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{AppState, CmdResult, err, store};

/// The prepared 48 kHz sound with its peaks, loudness blocks, cleaned and echo-free versions, and downloaded sounds
/// not added yet.
const SOUND: &[&str] = &["pcm", "voice", "dereverb", "sounds"];
/// Preview copies of heavy video, outlines of the person, cover masks, project cards and pictures sent to agents.
const PICTURES: &[&str] = &["proxy", "matte", "masks", "posters", "attachments"];
const YOUTUBE: &[&str] = &["youtube"];
/// A file being written is never removed under its writer; one left from a crash goes once it is this old.
const UNFINISHED_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum StorageGroupId {
    Sound,
    Pictures,
    Youtube,
    Models,
    Data,
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StorageGroup {
    pub id: StorageGroupId,
    pub bytes: u64,
    /// Part of `bytes` that Clear keeps, as the open project uses it.
    pub kept_bytes: u64,
    /// Nuzky makes the group again by itself, so Settings offers Clear; models and data are never cleared there.
    pub clearable: bool,
    /// Why Clear cannot run now, as a sentence that says what to do.
    pub blocked: Option<String>,
    /// The folder, for Show in folder: the data folder and the models.
    pub path: Option<String>,
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    pub groups: Vec<StorageGroup>,
}

/// One measure or clear at a time, so two clicks never walk the same folders at once.
static WALK: Mutex<()> = Mutex::new(());

#[tauri::command]
pub async fn storage_usage(app: AppHandle) -> CmdResult<StorageUsage> {
    let (cache, blocked, keep) = look(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _walk = WALK.lock().unwrap();
        usage(&cache, &blocked, &keep)
    })
    .await
    .map_err(err)
}

/// Clears one group Nuzky makes again by itself and returns the usage after it. `STORAGE_BUSY:` says what to wait
/// for; files another program holds open, as Windows refuses to remove, stay and are still counted.
#[tauri::command]
pub async fn clear_storage(app: AppHandle, group: String) -> CmdResult<StorageUsage> {
    let (id, folders) = match group.as_str() {
        "sound" => (StorageGroupId::Sound, SOUND),
        "pictures" => (StorageGroupId::Pictures, PICTURES),
        "youtube" => (StorageGroupId::Youtube, YOUTUBE),
        _ => return Err(format!("INVALID_INPUT: Nuzky doesn't clear {group:?}")),
    };
    let (cache, blocked, keep) = look(&app)?;
    if let Some(reason) = blocked.iter().find(|(group, _)| *group == id).map(|(_, reason)| reason) {
        return Err(format!("STORAGE_BUSY: {reason}"));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _walk = WALK.lock().unwrap();
        clear(&cache, folders, &keep);
        log::info!(target: "action", "storage cleared {group}");
        usage(&cache, &blocked, &keep)
    })
    .await
    .map_err(err)
}

/// What is needed from the app's state, taken at once: the cache, why each group cannot be cleared now, and the file
/// name prefixes of what the open project uses.
type Look = (PathBuf, Vec<(StorageGroupId, String)>, Vec<String>);

fn look(app: &AppHandle) -> CmdResult<Look> {
    let state = app.state::<AppState>();
    let host = state.session.lock().unwrap().host.clone();
    let mut blocked = Vec::new();
    let editing = host.session.state().is_ok_and(|s| s.open_run.is_some())
        || app.state::<crate::agent_panel::AgentPanel>().working();
    if editing {
        let reason = "Wait for the AI to finish editing. It may be using these files.";
        blocked.extend(
            [StorageGroupId::Sound, StorageGroupId::Pictures, StorageGroupId::Youtube].map(|g| (g, reason.into())),
        );
    }
    let kinds = state.running_kinds();
    for group in [StorageGroupId::Sound, StorageGroupId::Pictures] {
        if let Some(reason) = job_blocks(group, &kinds) {
            blocked.push((group, reason));
        }
    }
    if !state.sounds.idle() {
        blocked.push((StorageGroupId::Sound, "Wait for the sound to finish downloading.".into()));
    }
    if !state.youtube.idle() {
        let reason = "Close Video from YouTube first. It uses these downloads.";
        blocked.extend([StorageGroupId::Youtube, StorageGroupId::Sound].map(|g| (g, reason.into())));
    }
    let cache = state.cache_dir.clone();
    let keep = state.project().map(|project| kept(&cache, &project.assets)).unwrap_or_default();
    Ok((cache, blocked, keep))
}

/// A running job that may read or write the group, named in the words the UI uses. Kinds not named here block both
/// groups, so a new kind of job is safe before anyone thinks of it.
fn job_blocks(group: StorageGroupId, kinds: &[String]) -> Option<String> {
    let unrelated: &[&str] = match group {
        StorageGroupId::Sound => &["proxy", "matte", "cover-mask", "cover-pick", "faces", "reframe", "follow"],
        _ => &["audio", "voice", "dereverb", "level"],
    };
    let never = ["speech-model", "vision-models", "screen-recording", "voiceover"];
    let kind = kinds.iter().find(|kind| !unrelated.contains(&kind.as_str()) && !never.contains(&kind.as_str()))?;
    let uses = if group == StorageGroupId::Sound { "the prepared sound" } else { "these pictures" };
    let what = match kind.as_str() {
        "export" | "cover-export" => "the export",
        "captions" | "transcript" | "transcription" => "speech recognition",
        "style" => "learning your style",
        "audio" | "voice" | "dereverb" | "level" => "Nuzky to finish preparing the sound",
        "proxy" => "the preview copy of your video",
        "matte" | "cover-mask" | "cover-pick" => "Nuzky to finish finding the person",
        _ => "the analysis",
    };
    let wait =
        if what.starts_with("Nuzky") { format!("Wait for {what}") } else { format!("Wait for {what} to finish") };
    Some(format!("{wait}. It uses {uses}."))
}

/// The start of every cache file name made from the assets: their sound by id and file revision, their preview copy
/// and person outlines by file revision. Both come from the engine's own paths, so they follow its naming.
fn kept(cache: &Path, assets: &[Asset]) -> Vec<String> {
    let stem = |path: &Path| path.file_stem().map(|s| s.to_string_lossy().into_owned());
    let mut keep = Vec::new();
    for asset in assets {
        if let Some(stem) = stem(&nuzky_engine::audio::pcm_path(cache, asset)) {
            keep.push(format!("{stem}."));
        }
        if asset.kind != AssetKind::Audio {
            let proxy = nuzky_engine::proxy::proxy_path(cache, Path::new(&asset.path));
            // The file's revision, before the proxy's version; mattes start with it too.
            if let Some(key) = proxy.file_name().and_then(|n| n.to_str()).and_then(|n| n.split('.').next()) {
                keep.push(format!("{key}."));
            }
        }
    }
    keep
}

fn usage(cache: &Path, blocked: &[(StorageGroupId, String)], keep: &[String]) -> StorageUsage {
    let reason = |id| blocked.iter().find(|(group, _)| *group == id).map(|(_, reason)| reason.clone());
    let cleared = |id, folders: &[&str]| {
        let (bytes, kept_bytes) = measure(cache, folders, keep);
        StorageGroup { id, bytes, kept_bytes, clearable: true, blocked: reason(id), path: None }
    };
    // The folders themselves may be links (a moved Library folder); what is inside them is walked without following any.
    let real = |dir: PathBuf| dunce::canonicalize(&dir).unwrap_or(dir);
    let data = real(store::data_dir());
    let models = real(nuzky_analysis::models_dir());
    let whole = |id, dir: &Path, bytes| StorageGroup {
        id,
        bytes,
        kept_bytes: 0,
        clearable: false,
        blocked: None,
        path: Some(dir.to_string_lossy().into_owned()),
    };
    let models_bytes = size(&models);
    // The models live inside the data folder and have a group of their own.
    let data_bytes = size(&data).saturating_sub(if models.starts_with(&data) { models_bytes } else { 0 });
    StorageUsage {
        groups: vec![
            cleared(StorageGroupId::Sound, SOUND),
            cleared(StorageGroupId::Pictures, PICTURES),
            cleared(StorageGroupId::Youtube, YOUTUBE),
            whole(StorageGroupId::Models, &models, models_bytes),
            whole(StorageGroupId::Data, &data, data_bytes),
        ],
    }
}

/// Bytes in `folders` of the cache, and those of them clearing keeps.
fn measure(cache: &Path, folders: &[&str], keep: &[String]) -> (u64, u64) {
    let (mut bytes, mut kept) = (0, 0);
    for folder in folders {
        walk(&cache.join(folder), &mut |path, meta| {
            bytes += meta.len();
            if keep.iter().any(|p| name_of(path).starts_with(p.as_str())) {
                kept += meta.len();
            }
        });
    }
    (bytes, kept)
}

/// Removes every file in `folders` of the cache but those starting with a `keep` prefix, those whose lock another
/// writer holds, lock files and files still being written. Errors leave a file where it is.
fn clear(cache: &Path, folders: &[&str], keep: &[String]) {
    let now = SystemTime::now();
    for folder in folders {
        let root = cache.join(folder);
        let protected = protected(&root, keep);
        let mut dirs = Vec::new();
        walk_with_dirs(&root, &mut dirs, &mut |path, meta| {
            let name = name_of(path);
            let unfinished = (name.contains(".part") || name.ends_with(".tmp"))
                && meta
                    .modified()
                    .ok()
                    .and_then(|at| now.duration_since(at).ok())
                    .is_none_or(|age| age < UNFINISHED_AGE);
            if name.ends_with(".lock") || unfinished || protected.iter().any(|p| name.starts_with(p.as_str())) {
                return;
            }
            if let Err(error) = std::fs::remove_file(path) {
                log::warn!("Kept {}: {error}", path.display());
            }
        });
        // Deepest first; the folders themselves stay, as writers expect them.
        for dir in dirs.iter().rev() {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

/// `keep`, and the start of the files whose lock in `dir` is held, by a job of this app or another Nuzky process.
/// A lock `X.f32.lock` guards `X.f32` and what is made beside it, as its peaks `X.p1.peaks`.
fn protected(dir: &Path, keep: &[String]) -> Vec<String> {
    let mut protected = keep.to_vec();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(target) = name.strip_suffix(".lock") else { continue };
        let Ok(lock) = std::fs::File::options().read(true).write(true).open(entry.path()) else { continue };
        if let Err(std::fs::TryLockError::WouldBlock) = lock.try_lock() {
            let stem = Path::new(target).file_stem().map_or(target.into(), |s| s.to_string_lossy().into_owned());
            protected.push(stem);
        }
    }
    protected
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The bytes of every file under `dir`.
fn size(dir: &Path) -> u64 {
    let mut bytes = 0;
    walk(dir, &mut |_, meta| bytes += meta.len());
    bytes
}

fn walk(dir: &Path, file: &mut impl FnMut(&Path, &std::fs::Metadata)) {
    walk_with_dirs(dir, &mut Vec::new(), file);
}

/// Calls `file` with every regular file under `dir`, adding the folders under it to `dirs`. Links are neither followed
/// nor reported, so nothing outside `dir` is reached; nor is `dir` itself when it is a link.
fn walk_with_dirs(dir: &Path, dirs: &mut Vec<PathBuf>, file: &mut impl FnMut(&Path, &std::fs::Metadata)) {
    if !std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir()) {
        return;
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() {
            dirs.push(path.clone());
            walk_with_dirs(&path, dirs, file);
        } else if kind.is_file()
            && let Ok(meta) = entry.metadata()
        {
            file(&path, &meta);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0u8; bytes]).unwrap();
    }

    #[test]
    fn clearing_keeps_the_open_project_and_work_in_progress_and_never_leaves_the_cache() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tmp-test")
            .join(format!("storage-{}", nuzky_engine::edit::new_id()));
        let cache = root.join("cache");
        let outside = root.join("Videos/take.mov");
        write(&outside, 100);
        let open = Asset {
            id: "talk".into(),
            name: "take.mov".into(),
            path: outside.to_string_lossy().into_owned(),
            kind: AssetKind::Video,
            duration_us: 1_000_000,
            width: 1080,
            height: 1920,
            fps: 30.0,
            has_audio: true,
            rotation: 0,
            mirror: false,
            credit: None,
            part: None,
        };
        let keep = kept(&cache, std::slice::from_ref(&open));
        let pcm = nuzky_engine::audio::pcm_path(&cache, &open);
        let peaks = nuzky_engine::audio::peaks_for(&pcm);
        let cleaned = nuzky_engine::voice::voice_pcm_path(&cache, &open);
        let proxy = nuzky_engine::proxy::proxy_path(&cache, Path::new(&open.path));
        for kept in [&pcm, &peaks, &cleaned, &proxy] {
            write(kept, 10);
        }
        let other = cache.join("pcm/old.123-456-0000000000000000.v4.f32");
        let other_peaks = cache.join("pcm/old.123-456-0000000000000000.v4.p1.peaks");
        let poster = cache.join("posters/0011223344556677-1.png");
        let video = cache.join("youtube/abc/picture.mp4");
        for gone in [&other, &other_peaks, &poster, &video] {
            write(gone, 1000);
        }
        // Another Nuzky process extracts this sound now, into a part file beside it.
        let busy = cache.join("pcm/busy.9-9-0000000000000000.v4.f32");
        write(&busy.with_extension("p1.peaks"), 7);
        write(&cache.join("pcm/.nuzky-pcm-x.part"), 7);
        let lock = std::fs::File::create(cache.join("pcm/busy.9-9-0000000000000000.v4.f32.lock")).unwrap();
        lock.lock().unwrap();
        // The build scripts' folder in ~/.cache/nuzky on Linux, and a link out of the cache.
        write(&cache.join("deps/ffmpeg/lib.a"), 100);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, cache.join("pcm/link.f32")).unwrap();

        let (bytes, kept_bytes) = measure(&cache, SOUND, &keep);
        assert_eq!((bytes, kept_bytes), (2000 + 30 + 14, 30), "the sound group counts its own files only");
        clear(&cache, SOUND, &keep);
        clear(&cache, PICTURES, &keep);
        clear(&cache, YOUTUBE, &keep);

        for gone in [&other, &other_peaks, &poster, &video] {
            assert!(!gone.exists(), "{} stayed", gone.display());
        }
        assert!(!cache.join("youtube/abc").exists(), "an emptied folder stayed");
        for stays in [&pcm, &peaks, &cleaned, &proxy] {
            assert!(stays.exists(), "the open project's {} went", stays.display());
        }
        assert!(busy.with_extension("p1.peaks").exists(), "the peaks another process writes went");
        assert!(cache.join("pcm/.nuzky-pcm-x.part").exists(), "a file being written went");
        assert!(cache.join("deps/ffmpeg/lib.a").exists() && outside.exists(), "a file outside the groups went");
        #[cfg(unix)]
        assert!(std::fs::symlink_metadata(cache.join("pcm/link.f32")).is_ok() && outside.exists());
        assert_eq!(measure(&cache, SOUND, &keep).0, 30 + 14);
        drop(lock);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_job_that_uses_a_group_says_what_to_wait_for() {
        let kinds = |k: &[&str]| k.iter().map(|k| k.to_string()).collect::<Vec<_>>();
        assert_eq!(
            job_blocks(StorageGroupId::Sound, &kinds(&["export"])).as_deref(),
            Some("Wait for the export to finish. It uses the prepared sound.")
        );
        assert_eq!(job_blocks(StorageGroupId::Sound, &kinds(&["proxy", "speech-model"])), None);
        assert_eq!(
            job_blocks(StorageGroupId::Pictures, &kinds(&["proxy"])).as_deref(),
            Some("Wait for the preview copy of your video to finish. It uses these pictures.")
        );
        assert!(job_blocks(StorageGroupId::Pictures, &kinds(&["something-new"])).is_some());
    }
}

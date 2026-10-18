//! The Video from YouTube dialog: look up a link, download the video, play it in a preview of its own while the
//! user chooses a part, and add that part to the project. Downloading and joining live in `nuzky_mcp::videos`,
//! shared with agents.
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nuzky_engine::Project;
use nuzky_engine::edit::EditCmd;
use nuzky_engine::model::{Asset, Part};
use nuzky_mcp::videos::{self, Video};
use nuzky_session::Expect;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::engine::{Engine, Msg};
use crate::preview_server::PreviewServer;
use crate::{AppState, CmdResult, Filmstrip, Snapshot, err};

const PICTURE: &str = "youtube-picture";
const SOUND: &str = "youtube-sound";

/// The download running, and the preview of the downloaded video with its picture, while the dialog is open.
#[derive(Default)]
pub struct YoutubeVideos {
    download: Mutex<Option<Arc<AtomicBool>>>,
    preview: Mutex<Option<(Engine, Asset)>>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress<'a> {
    id: &'a str,
    progress: f32,
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(err)?.map_err(err)
}

/// The video a YouTube link points at: title, channel and length.
#[tauri::command]
pub async fn youtube_video_link(url: String) -> CmdResult<Video> {
    blocking(move || videos::link(&url)).await
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoPreview {
    url: String,
    width: u32,
    height: u32,
    duration_us: i64,
}

/// Downloads the video, with `youtube-video-progress`, and starts its preview: frames on the returned socket,
/// playback through `youtube-transport` and the `youtube_preview_*` commands. `youtube_video_cancel` stops it.
#[tauri::command]
pub async fn youtube_video_fetch(app: AppHandle, id: String) -> CmdResult<VideoPreview> {
    let state = app.state::<AppState>();
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(old) = state.youtube.download.lock().unwrap().replace(cancel.clone()) {
        old.store(true, Ordering::Relaxed);
    }
    state.youtube.preview.lock().unwrap().take();
    let (cache, app2, stop) = (state.cache_dir.clone(), app.clone(), cancel.clone());
    let result = blocking(move || {
        let mut last = -1.0f32;
        let download = videos::fetch(&id, &cache, &stop, |p| {
            if p - last >= 0.01 || p >= 1.0 {
                last = p;
                app2.emit("youtube-video-progress", Progress { id: &id, progress: p }).ok();
            }
        })?;
        let picture = nuzky_engine::media::probe(&download.picture, PICTURE.into())?;
        let sound = nuzky_engine::media::probe(&download.sound, SOUND.into())?;
        nuzky_engine::audio::ensure_pcm(&cache, &sound, |_| nuzky_session::jobs::check_cancel(&stop))?;
        let mut project = Project::new("YouTube");
        project.canvas.width = picture.width.max(2) & !1;
        project.canvas.height = picture.height.max(2) & !1;
        project.apply(EditCmd::AddAssets { assets: vec![picture.clone(), sound] })?;
        for asset_id in [PICTURE, SOUND] {
            project.apply(EditCmd::AddClip { asset_id: asset_id.into(), start_us: Some(0), track_id: None })?;
        }
        Ok((project, picture))
    })
    .await;
    let mut download = state.youtube.download.lock().unwrap();
    if download.as_ref().is_some_and(|d| Arc::ptr_eq(d, &cancel)) {
        *download = None;
    }
    let (project, picture) = result?;
    if cancel.load(Ordering::Relaxed) {
        return Err("CANCELLED: the download stopped".into());
    }
    let server = Arc::new(PreviewServer::start().map_err(err)?);
    let preview = VideoPreview {
        url: server.url.clone(),
        width: project.canvas.width,
        height: project.canvas.height,
        duration_us: picture.duration_us,
    };
    let engine = Engine::start_other(
        app.clone(),
        ("youtube-transport", "youtube-preview-error"),
        server,
        Arc::new(project),
        state.cache_dir.clone(),
    );
    *state.youtube.preview.lock().unwrap() = Some((engine, picture));
    Ok(preview)
}

/// Stops the download, as the dialog closing or the user's Stop does.
#[tauri::command]
pub fn youtube_video_cancel(state: State<'_, AppState>) {
    if let Some(cancel) = state.youtube.download.lock().unwrap().take() {
        cancel.store(true, Ordering::Relaxed);
    }
}

/// Ends the preview when the dialog closes.
#[tauri::command]
pub fn youtube_preview_close(state: State<'_, AppState>) {
    state.youtube.preview.lock().unwrap().take();
}

fn send(state: &AppState, msg: Msg) {
    if let Some((engine, _)) = state.youtube.preview.lock().unwrap().as_ref() {
        engine.send(msg);
    }
}

#[tauri::command]
pub fn youtube_preview_play(state: State<'_, AppState>) {
    send(&state, Msg::Play);
}

#[tauri::command]
pub fn youtube_preview_pause(state: State<'_, AppState>) {
    send(&state, Msg::Pause);
}

#[tauri::command]
pub fn youtube_preview_seek(state: State<'_, AppState>, t_us: i64) {
    send(&state, Msg::Seek(t_us));
}

/// The space the preview has, in device pixels.
#[tauri::command]
pub fn youtube_preview_resize(state: State<'_, AppState>, width: u32, height: u32) {
    send(&state, Msg::Resize(width, height));
}

/// `count` frames of the downloaded video `interval_us` apart from `from_us`, for the dialog's strips.
#[tauri::command]
pub async fn youtube_filmstrip(
    app: AppHandle,
    from_us: i64,
    interval_us: i64,
    count: u32,
) -> CmdResult<Filmstrip> {
    let state = app.state::<AppState>();
    let picture = state.youtube.preview.lock().unwrap().as_ref().map(|(_, picture)| picture.clone());
    let picture = picture.ok_or("CANCELLED: the video is no longer open")?;
    let cache = state.cache_dir.clone();
    blocking(move || crate::thumbs::filmstrip_of(&cache, &picture, from_us, interval_us, count)).await
}

/// Adds the downloaded video, or its `part` with up to 3 s around it, to the project's media as one undo step.
#[tauri::command]
pub async fn youtube_video_add(
    app: AppHandle,
    video: Video,
    part: Option<Part>,
    expected_epoch: Option<String>,
) -> CmdResult<Snapshot> {
    let state = app.state::<AppState>();
    let cache = state.cache_dir.clone();
    let asset = blocking(move || videos::asset(&video, part, &cache, &AtomicBool::new(false), |_| {})).await?;
    let path = asset.path.clone();
    let snap = state
        .apply_batch(vec![EditCmd::AddAssets { assets: vec![asset] }], None, Expect::default(), expected_epoch.as_deref())
        .inspect_err(|_| drop(std::fs::remove_file(&path)))?;
    crate::jobs::prepare_media(&state, &snap.project);
    Ok(snap)
}

/// The downloads of the last session, which nothing needs any more.
pub fn forget(cache_dir: &Path) {
    videos::forget(cache_dir);
}

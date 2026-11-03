//! Plays an asset from Media without adding it to the open project's timeline.
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use nuzky_engine::Project;
use nuzky_engine::edit::EditCmd;
use nuzky_engine::model::AssetKind;
use tauri::{AppHandle, Manager, State};

use crate::engine::{Engine, Msg};
use crate::preview_server::PreviewServer;
use crate::youtube_videos::VideoPreview;
use crate::{AppState, CmdResult, err};

/// Its accept thread never ends, so every opening after a close shares the first one.
static SERVER: OnceLock<Arc<PreviewServer>> = OnceLock::new();

/// Opens an asset in its own preview, reusing the engine when switching media.
#[tauri::command]
pub async fn source_preview_open(app: AppHandle, asset_id: String) -> CmdResult<VideoPreview> {
    let state = app.state::<AppState>();
    let asset = state.project()?.asset(&asset_id).cloned().ok_or("Unknown asset")?;
    let mut project = Project::new(&asset.name);
    let (width, height) = if asset.kind == AssetKind::Audio { (1280, 720) } else { (asset.width, asset.height) };
    project.canvas.width = width.max(2) & !1;
    project.canvas.height = height.max(2) & !1;
    if asset.fps > 0.0 {
        project.canvas.fps = asset.fps.round().clamp(1.0, 120.0) as u32;
    }
    project.apply(EditCmd::AddAssets { assets: vec![asset] }).map_err(err)?;
    project.apply(EditCmd::AddClip { asset_id, start_us: Some(0), track_id: None }).map_err(err)?;
    let mut preview = VideoPreview {
        url: String::new(),
        width: project.canvas.width,
        height: project.canvas.height,
        duration_us: project.duration_us(),
        fps: f64::from(project.canvas.fps),
    };
    let mut current = state.source_preview.lock().unwrap();
    if let Some((engine, url)) = current.as_ref() {
        engine.send(Msg::Project(Arc::new(project)));
        engine.send(Msg::Pause);
        engine.send(Msg::Seek(0));
        preview.url = url.clone();
    } else {
        let server = match SERVER.get() {
            Some(server) => server.clone(),
            None => {
                let server = Arc::new(PreviewServer::start().map_err(err)?);
                SERVER.get_or_init(|| server).clone()
            }
        };
        preview.url = server.url.clone();
        let engine = Engine::start_other(
            app.clone(),
            ("source-transport", "source-preview-error"),
            server,
            Arc::new(project),
            state.cache_dir.clone(),
        );
        engine.muted.store(state.engine.muted.load(Ordering::Relaxed), Ordering::Relaxed);
        *current = Some((engine, preview.url.clone()));
    }
    Ok(preview)
}

/// Ends the source preview when Media closes it.
#[tauri::command]
pub fn source_preview_close(state: State<'_, AppState>) {
    state.source_preview.lock().unwrap().take();
}

fn send(state: &AppState, msg: Msg) {
    if let Some((engine, _)) = state.source_preview.lock().unwrap().as_ref() {
        engine.send(msg);
    }
}

#[tauri::command]
pub fn source_preview_play(state: State<'_, AppState>) {
    send(&state, Msg::Play);
}

#[tauri::command]
pub fn source_preview_pause(state: State<'_, AppState>) {
    send(&state, Msg::Pause);
}

#[tauri::command]
pub fn source_preview_seek(state: State<'_, AppState>, t_us: i64) {
    send(&state, Msg::Seek(t_us));
}

/// The space the preview has, in device pixels.
#[tauri::command]
pub fn source_preview_resize(state: State<'_, AppState>, width: u32, height: u32) {
    send(&state, Msg::Resize(width, height));
}

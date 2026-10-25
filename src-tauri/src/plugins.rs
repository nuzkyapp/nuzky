use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use anyhow::{Result, ensure};
use nuzky_mcp::plugins;
use serde_json::{Map, Value, json};
use tauri::{AppHandle, Emitter, Manager};

use crate::{AppState, CmdResult, err, lock_session};

#[tauri::command]
pub async fn list_plugins() -> CmdResult<Vec<plugins::Plugin>> {
    tauri::async_runtime::spawn_blocking(plugins::list).await.map_err(err)?.map_err(err)
}

#[tauri::command]
pub async fn preview_plugin(
    app: AppHandle,
    id: String,
    values: Map<String, Value>,
    selection: Vec<String>,
    playhead_us: i64,
) -> CmdResult<Value> {
    let cancel = Arc::new(AtomicBool::new(false));
    let state = app.state::<AppState>();
    std::mem::replace(&mut *state.plugin_preview.lock().unwrap(), cancel.clone()).store(true, Ordering::Relaxed);
    let host = state.session.lock().unwrap().host.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut preview = plugins::dry_run_in_app(&host, &id, &values, selection, playhead_us, cancel).map_err(err)?;
        // The rows come also with an error, so the panel can offer them again when a chosen one is gone.
        let items = preview["items"].take();
        if preview["valid"] != true {
            return Ok(json!({"project": null, "error": preview["error"], "items": items}));
        }
        // As the snapshot draws it, so the app compares the two like for like.
        let mut project = preview["project"].take();
        let safe_area =
            serde_json::from_value::<nuzky_engine::Project>(project.clone()).map_err(err)?.canvas.safe_area();
        project["canvas"]["safeArea"] = json!(safe_area);
        Ok(json!({"project": project, "error": null, "items": items}))
    })
    .await
    .map_err(err)?
}

/// Stops the preview still running, once the plugin it was for is closed.
#[tauri::command]
pub fn cancel_plugin_preview(app: AppHandle) {
    app.state::<AppState>().plugin_preview.lock().unwrap().store(true, Ordering::Relaxed);
}

#[tauri::command]
pub async fn run_plugin(
    app: AppHandle,
    id: String,
    values: Map<String, Value>,
    action: Option<String>,
    selection: Vec<String>,
    playhead_us: i64,
    expected_epoch: Option<String>,
) -> CmdResult<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let (host, epoch, mut job) = {
            let current = lock_session(&state.session, expected_epoch.as_deref())?;
            let host = current.host.clone();
            let epoch = host.session.stamp().session_epoch;
            let job =
                plugins::run_in_app(&host, &id, &values, action.as_deref(), selection, playhead_us).map_err(err)?;
            (host, epoch, job)
        };
        let job_id = job["job_id"].as_str().unwrap().to_owned();
        while job["status"] == "running" {
            std::thread::sleep(Duration::from_millis(100));
            job = host.jobs.get(&job_id, false).map_err(err)?;
        }
        if job["status"] != "done" {
            return Err(job["error"].as_str().unwrap_or("CANCELLED: plugin stopped").to_owned());
        }
        let result = &job["result"];
        let snapshot = if result["changed"] == true {
            Some(lock_session(&state.session, Some(&epoch))?.snapshot(Vec::new())?)
        } else {
            None
        };
        Ok(json!({"snapshot": snapshot, "message": result["message"]}))
    })
    .await
    .map_err(err)?
}

fn folder(id: &str) -> Result<PathBuf> {
    ensure!(Path::new(id).file_name().is_some_and(|name| *name == *id), "PLUGIN_NOT_FOUND: no plugin {id:?}");
    let folder = plugins::dir()?.join(id);
    ensure!(folder.is_dir(), "PLUGIN_NOT_FOUND: no plugin {id:?}");
    Ok(folder)
}

#[tauri::command]
pub async fn remove_plugin(app: AppHandle, id: String) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::library::to_trash(&folder(&id).map_err(err)?).map_err(err)?;
        app.emit("plugins-changed", ()).map_err(err)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub fn show_plugin_folder(app: AppHandle, id: Option<String>) -> CmdResult<()> {
    let path = if let Some(id) = id {
        folder(&id).map_err(err)?
    } else {
        let path = plugins::dir().map_err(err)?;
        std::fs::create_dir_all(&path).map_err(err)?;
        path
    };
    crate::updates::open(&app, &path.to_string_lossy())
}

pub fn watch(app: AppHandle) {
    let stamp = || {
        let mut entries: Vec<_> = plugins::dir()
            .ok()
            .and_then(|dir| std::fs::read_dir(dir).ok())
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            // The plugin's own files too: an editor that rewrites plugin.js in place leaves the folder's time alone.
            .flat_map(|entry| ["", "plugin.json", "plugin.js"].map(|file| entry.path().join(file)))
            .filter_map(|path| Some((path.clone(), path.metadata().ok()?.modified().ok()?)))
            .collect();
        entries.sort();
        entries
    };
    let spawned = std::thread::Builder::new().name("plugins-watch".into()).spawn(move || {
        let mut last = stamp();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let now = stamp();
            if now != last {
                last = now;
                app.emit("plugins-changed", ()).ok();
            }
        }
    });
    if let Err(error) = spawned {
        log::error!("Changes to plugins made elsewhere will show only when the panel opens again: {error}");
    }
}

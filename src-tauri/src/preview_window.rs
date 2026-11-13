//! The preview in a window of its own, to move to another screen or show full screen. It shows the frames the
//! editor's preview gets, from the same engine and socket, so both stay on the same frame; its keys go to the
//! editor, which plays and seeks as if they were pressed there.
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl, WebviewWindow};

use crate::engine::{Msg, Transport};
use crate::{AppState, CmdResult, err};

pub const LABEL: &str = "preview";

/// Where the window was when it last closed, so it opens there again, on the other screen, until Nuzky quits.
static PLACE: Mutex<Option<(PhysicalPosition<i32>, PhysicalSize<u32>)>> = Mutex::new(None);

fn flow() -> bool {
    #[cfg(all(target_os = "macos", debug_assertions))]
    return crate::test_bridge::active();
    #[cfg(not(all(target_os = "macos", debug_assertions)))]
    std::env::var_os("TAURI_WEBVIEW_AUTOMATION").is_some()
}

/// Opens the window, or brings it forward when it is open. Async, as Tauri creates windows on Windows only so.
#[tauri::command]
pub async fn open_preview_window(app: AppHandle) -> CmdResult<()> {
    let flow = flow();
    if let Some(window) = app.get_webview_window(LABEL) {
        if !flow {
            window.unminimize().ok();
            window.set_focus().map_err(err)?;
        }
        app.emit_to("main", "preview-window", true).ok();
        return Ok(());
    }
    let mut builder = tauri::WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Nuzky Preview")
        .inner_size(960.0, 600.0)
        .min_inner_size(320.0, 240.0)
        .theme(Some(tauri::Theme::Dark))
        .background_color(tauri::window::Color(0, 0, 0, 255))
        .disable_drag_drop_handler()
        .visible(false)
        .focused(!flow);
    if flow {
        builder = builder.background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled);
    }
    let window = builder.build().map_err(err)?;
    let place = *PLACE.lock().unwrap();
    if let Some((position, size)) = place.filter(|(position, _)| on_a_screen(&app, *position)) {
        window.set_size(size).ok();
        window.set_position(position).ok();
    }
    #[cfg(all(target_os = "macos", debug_assertions))]
    if flow {
        crate::test_bridge::keep_away(&window).map_err(err)?;
    }
    // Under gamescope a window never shown keeps no size and draws nothing; it is off any screen of a person there.
    if !flow || cfg!(target_os = "linux") {
        window.show().map_err(err)?;
    }
    app.emit_to("main", "preview-window", true).ok();
    Ok(())
}

#[tauri::command]
pub fn close_preview_window(app: AppHandle) -> CmdResult<()> {
    if let Some(window) = app.get_webview_window(LABEL) {
        window.close().map_err(err)?;
    }
    Ok(())
}

/// A screen unplugged since would leave the window out of sight.
fn on_a_screen(app: &AppHandle, position: PhysicalPosition<i32>) -> bool {
    let (x, y) = (position.x + 40, position.y + 20);
    app.available_monitors().unwrap_or_default().iter().any(|monitor| {
        let (at, size) = (monitor.position(), monitor.size());
        (at.x..at.x + size.width as i32).contains(&x) && (at.y..at.y + size.height as i32).contains(&y)
    })
}

/// Before the window closes: where it was. A window in full screen covered its screen, so it opens on that screen
/// in its first size.
pub fn closing(window: &WebviewWindow) {
    let place = if window.is_fullscreen().unwrap_or(false) {
        window.current_monitor().ok().flatten().map(|monitor| {
            let scale = monitor.scale_factor();
            let size = PhysicalSize::new((960.0 * scale) as u32, (600.0 * scale) as u32);
            let (at, screen) = (monitor.position(), monitor.size());
            let centre = |start: i32, whole: u32, part: u32| start + (whole.saturating_sub(part) / 2) as i32;
            (
                PhysicalPosition::new(centre(at.x, screen.width, size.width), centre(at.y, screen.height, size.height)),
                size,
            )
        })
    } else {
        window.outer_position().ok().zip(window.inner_size().ok())
    };
    if place.is_some() {
        *PLACE.lock().unwrap() = place;
    }
}

/// Once the window is gone, frames are drawn for the editor's preview alone again.
pub fn closed(app: &AppHandle) {
    app.state::<AppState>().engine.send(Msg::WindowBox(None));
    app.emit_to("main", "preview-window", false).ok();
}

/// The editor window is gone, so the app quits; the preview window must not keep it running.
pub fn editor_closed(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        window.destroy().ok();
    }
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PreviewProject {
    duration_us: i64,
    width: u32,
    height: u32,
}

impl PreviewProject {
    pub fn of(project: &nuzky_engine::Project) -> Self {
        Self { duration_us: project.duration_us(), width: project.canvas.width, height: project.canvas.height }
    }
}

/// The project's length and shape for the window's timecode and frame, on every change.
pub fn publish(app: &AppHandle, project: &nuzky_engine::Project) {
    if app.get_webview_window(LABEL).is_some() {
        app.emit_to(LABEL, "preview-window-project", PreviewProject::of(project)).ok();
    }
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewWindowBoot {
    preview_url: String,
    transport: Transport,
    engine_error: Option<String>,
    project: PreviewProject,
}

#[tauri::command]
pub fn preview_window_boot(state: State<'_, AppState>) -> CmdResult<PreviewWindowBoot> {
    Ok(PreviewWindowBoot {
        preview_url: state.preview_url.clone(),
        transport: *state.engine.transport.lock().unwrap(),
        engine_error: state.engine.error.lock().unwrap().clone(),
        project: PreviewProject::of(&state.project()?),
    })
}

/// The window's box in device pixels.
#[tauri::command]
pub fn set_preview_window_box(state: State<'_, AppState>, width: u32, height: u32) {
    state.engine.send(Msg::WindowBox(Some((width, height))));
}

/// A key pressed in the window, which the editor runs as its own.
#[tauri::command]
pub fn preview_window_action(app: AppHandle, action: String) {
    app.emit_to("main", "preview-window-action", action).ok();
}

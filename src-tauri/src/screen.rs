//! Record screen in Media: the whole screen or one window, recorded by macOS's own `screencapture` into Nuzky's
//! folder while the user works in another app, and added to the project's media when they stop. Only the user's
//! click starts it, and macOS shows that the screen is being recorded. Nothing is recorded on other systems yet.
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nuzky_engine::edit::{EditCmd, new_id};
use nuzky_session::Expect;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{AppState, CmdResult, Snapshot};

/// A window the user can record, as the system lists it, frontmost first.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenWindow {
    pub id: u32,
    pub app: String,
    /// Empty until Nuzky may record the screen: macOS hides other apps' window titles before that.
    pub title: String,
}

impl ScreenWindow {
    fn name(&self) -> String {
        match self.title.trim() {
            "" => self.app.clone(),
            title if title == self.app => self.app.clone(),
            title => format!("{} · {title}", self.app),
        }
    }
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenRecording {
    /// Milliseconds since 1970 when the recording started.
    pub started_ms: f64,
    /// The window recorded, or null for the whole screen.
    pub window: Option<ScreenWindow>,
}

struct Recording {
    child: Child,
    stdin: Option<ChildStdin>,
    path: PathBuf,
    shown: ScreenRecording,
    /// How often each window was in front, for naming a recording of the whole screen after what it showed.
    seen: Arc<Mutex<HashMap<(String, String), u32>>>,
    done: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct ScreenRecorder {
    current: Mutex<Option<Recording>>,
}

impl ScreenRecorder {
    pub fn recording(&self) -> bool {
        self.current.lock().unwrap().is_some()
    }

    /// Stops the recording, as quitting does, and deletes it: it was never added to a project.
    pub fn quit(&self) {
        if let Some(recording) = self.current.lock().unwrap().take() {
            let path = finish(recording);
            std::fs::remove_file(path).ok();
        }
    }
}

/// `screencapture`, or in debug builds the tests' stand-in, which needs no permission.
fn program() -> (PathBuf, bool) {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("NUZKY_SCREENCAPTURE") {
        return (path.into(), false);
    }
    (PathBuf::from("/usr/sbin/screencapture"), true)
}

const UNSUPPORTED: &str =
    "SCREEN_UNSUPPORTED: Nuzky records the screen on macOS for now. Record it with another app and import the video.";

/// Whether Nuzky may record the screen; the first time, macOS asks the user.
fn may_record() -> CmdResult<()> {
    if !cfg!(target_os = "macos") {
        return Err(UNSUPPORTED.into());
    }
    #[cfg(target_os = "macos")]
    if program().1 && !mac::may_record() {
        return Err("SCREEN_PERMISSION: Allow Nuzky in System Settings › Privacy & Security › Screen & System Audio Recording, then quit and open Nuzky again.".into());
    }
    Ok(())
}

/// The windows of other apps, frontmost first. Asking for them is where macOS first asks the user to allow
/// recording, before they pick what to record.
#[tauri::command]
pub fn screen_windows() -> CmdResult<Vec<ScreenWindow>> {
    may_record()?;
    Ok(windows())
}

#[cfg(target_os = "macos")]
fn windows() -> Vec<ScreenWindow> {
    mac::windows()
}

#[cfg(not(target_os = "macos"))]
fn windows() -> Vec<ScreenWindow> {
    Vec::new()
}

#[tauri::command]
pub fn screen_recording(state: State<'_, AppState>) -> Option<ScreenRecording> {
    state.screen.current.lock().unwrap().as_ref().map(|r| r.shown.clone())
}

/// Starts recording the whole screen, or `window`, until `screen_record_stop`. When it ends by itself, such as
/// from the system's own Stop, `screen-recording-ended` says so.
#[tauri::command]
pub fn screen_record_start(app: AppHandle, window: Option<ScreenWindow>) -> CmdResult<ScreenRecording> {
    may_record()?;
    let state = app.state::<AppState>();
    let mut current = state.screen.current.lock().unwrap();
    if current.is_some() {
        return Err("RECORDING: the screen is already being recorded".into());
    }
    let (program, _) = program();
    let dir = crate::store::data_dir().join("recordings");
    std::fs::create_dir_all(&dir).map_err(crate::err)?;
    let path = dir.join(format!("screen-{}.mov", new_id()));
    let mut command = Command::new(&program);
    command.args(["-x", "-v"]);
    if let Some(window) = &window {
        command.arg(format!("-l{}", window.id));
    }
    let mut child = command
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Nuzky could not start recording the screen ({e})"))?;
    // A refusal, such as a window that closed, comes at once.
    let deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            std::fs::remove_file(&path).ok();
            let said = child.stderr.take().map(std::io::read_to_string).and_then(Result::ok).unwrap_or_default();
            let said = said.trim();
            return Err(if said.is_empty() {
                "The screen could not be recorded. Check that Nuzky may record the screen in System Settings › Privacy & Security."
                    .into()
            } else {
                format!("The screen could not be recorded: {said}")
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let started_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64() * 1000.0;
    let shown = ScreenRecording { started_ms, window };
    let (seen, done) = (Arc::new(Mutex::new(HashMap::new())), Arc::new(AtomicBool::new(false)));
    let stdin = child.stdin.take();
    *current = Some(Recording { child, stdin, path, shown: shown.clone(), seen: seen.clone(), done: done.clone() });
    drop(current);
    std::thread::spawn(move || watch(app, seen, done));
    Ok(shown)
}

/// Once a second: which window is in front, and whether the recording ended by itself.
fn watch(app: AppHandle, seen: Arc<Mutex<HashMap<(String, String), u32>>>, done: Arc<AtomicBool>) {
    while !done.load(Ordering::Acquire) {
        if let Some(front) = windows().into_iter().next() {
            *seen.lock().unwrap().entry((front.app, front.title)).or_default() += 1;
        }
        let state = app.state::<AppState>();
        let ended = match state.screen.current.lock().unwrap().as_mut() {
            Some(recording) if Arc::ptr_eq(&recording.done, &done) => matches!(recording.child.try_wait(), Ok(Some(_))),
            _ => return,
        };
        if ended {
            app.emit("screen-recording-ended", ()).ok();
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Ends `screencapture` so it finishes the file: a key on its input, then Ctrl+C, then it is killed.
fn finish(mut recording: Recording) -> PathBuf {
    recording.done.store(true, Ordering::Release);
    if let Some(mut stdin) = recording.stdin.take() {
        stdin.write_all(b"q\n").ok();
    }
    let wait = |child: &mut Child, for_: Duration| {
        let deadline = Instant::now() + for_;
        while Instant::now() < deadline {
            if !matches!(child.try_wait(), Ok(None)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    };
    if !wait(&mut recording.child, Duration::from_secs(15)) {
        #[cfg(unix)]
        unsafe {
            libc::kill(recording.child.id() as i32, libc::SIGINT);
        }
        if !wait(&mut recording.child, Duration::from_secs(5)) {
            recording.child.kill().ok();
            recording.child.wait().ok();
        }
    }
    recording.path
}

/// What the recording showed: the window recorded, else the window in front most of the time.
fn name(shown: &ScreenRecording, seen: &HashMap<(String, String), u32>) -> String {
    if let Some(window) = &shown.window {
        return window.name();
    }
    let mut apps: HashMap<&str, u32> = HashMap::new();
    for ((app, _), n) in seen {
        *apps.entry(app).or_default() += n;
    }
    let Some(app) = apps.into_iter().max_by_key(|&(app, n)| (n, std::cmp::Reverse(app))).map(|(app, _)| app) else {
        return "Screen recording".into();
    };
    let title = seen
        .iter()
        .filter(|((a, _), _)| a == app)
        .max_by_key(|((_, title), n)| (**n, std::cmp::Reverse(title.clone())))
        .map(|((_, title), _)| title.clone())
        .unwrap_or_default();
    ScreenWindow { id: 0, app: app.to_owned(), title }.name()
}

/// Stops the recording. `keep` adds it to the project's media as one undo step; otherwise it is deleted.
#[tauri::command]
pub async fn screen_record_stop(
    app: AppHandle,
    keep: bool,
    expected_epoch: Option<String>,
) -> CmdResult<Option<Snapshot>> {
    let recording = app.state::<AppState>().screen.current.lock().unwrap().take();
    let Some(recording) = recording else { return Ok(None) };
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let (shown, seen) = (recording.shown.clone(), recording.seen.clone());
        let path = finish(recording);
        if !keep {
            std::fs::remove_file(&path).ok();
            return Ok(None);
        }
        let probed = nuzky_engine::media::probe(&path, new_id());
        let mut asset = match probed {
            Ok(asset) if asset.kind == nuzky_engine::model::AssetKind::Video => asset,
            _ => {
                std::fs::remove_file(&path).ok();
                return Err("Nothing was recorded. Check that Nuzky may record the screen in System Settings › Privacy & Security.".into());
            }
        };
        asset.name = name(&shown, &seen.lock().unwrap());
        let snap = state
            .apply_batch(vec![EditCmd::AddAssets { assets: vec![asset] }], None, Expect::default(), expected_epoch.as_deref())
            .inspect_err(|_| drop(std::fs::remove_file(&path)))?;
        crate::jobs::prepare_media(&state, &snap.project);
        Ok(Some(snap))
    })
    .await
    .map_err(crate::err)?
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, ns_string};
    use std::ffi::c_void;

    use super::ScreenWindow;

    #[repr(C)]
    #[derive(Default)]
    struct Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32)
        -> *mut NSArray<NSDictionary<NSString, AnyObject>>;
        fn CGRectMakeWithDictionaryRepresentation(dict: *const c_void, rect: *mut Rect) -> bool;
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    const ON_SCREEN_ONLY: u32 = 1 << 0;
    const EXCLUDE_DESKTOP: u32 = 1 << 4;

    /// Whether Nuzky may record the screen; the first time, macOS asks the user, and later the settings open.
    pub fn may_record() -> bool {
        unsafe {
            if CGPreflightScreenCaptureAccess() {
                return true;
            }
            if !CGRequestScreenCaptureAccess() {
                std::process::Command::new("open")
                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
                    .spawn()
                    .ok();
            }
        }
        false
    }

    /// Ordinary windows of other apps, at least 100 points each way, frontmost first.
    pub fn windows() -> Vec<ScreenWindow> {
        let list = unsafe { Retained::from_raw(CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP, 0)) };
        let Some(list) = list else { return Vec::new() };
        let own = std::process::id() as i64;
        let number = |info: &NSDictionary<NSString, AnyObject>, key: &NSString| {
            info.objectForKey(key).and_then(|o| o.downcast::<NSNumber>().ok()).map(|n| n.as_i64())
        };
        let text = |info: &NSDictionary<NSString, AnyObject>, key: &NSString| {
            info.objectForKey(key)
                .and_then(|o| o.downcast::<NSString>().ok())
                .map(|s| s.to_string())
                .unwrap_or_default()
        };
        (0..list.count())
            .filter_map(|i| {
                let info = list.objectAtIndex(i);
                let layer = number(&info, ns_string!("kCGWindowLayer"))?;
                let pid = number(&info, ns_string!("kCGWindowOwnerPID"))?;
                let id = number(&info, ns_string!("kCGWindowNumber"))?;
                let bounds = info.objectForKey(ns_string!("kCGWindowBounds"))?;
                let mut rect = Rect::default();
                let sized =
                    unsafe { CGRectMakeWithDictionaryRepresentation(Retained::as_ptr(&bounds).cast(), &mut rect) };
                let app = text(&info, ns_string!("kCGWindowOwnerName"));
                (layer == 0 && pid != own && sized && rect.width >= 100.0 && rect.height >= 100.0 && !app.is_empty())
                    .then(|| ScreenWindow { id: id as u32, app, title: text(&info, ns_string!("kCGWindowName")) })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_screen_recording_is_named_after_the_window_in_front_most_often() {
        let shown = ScreenRecording { started_ms: 0.0, window: None };
        let seen = HashMap::from([
            (("Ghostty".into(), "✳ Claude Code".into()), 6),
            (("Ghostty".into(), "zsh".into()), 2),
            (("Safari".into(), "Docs".into()), 5),
        ]);
        assert_eq!(name(&shown, &seen), "Ghostty · ✳ Claude Code");
        assert_eq!(name(&shown, &HashMap::new()), "Screen recording");
        let window = ScreenWindow { id: 7, app: "Figma".into(), title: String::new() };
        assert_eq!(name(&ScreenRecording { started_ms: 0.0, window: Some(window) }, &seen), "Figma");
    }
}

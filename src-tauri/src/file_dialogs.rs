//! Notices when macOS leaves a file dialog off the screen. On macOS 27 a window that had been open for a while
//! once stopped showing them: Export… and Open darkened the window for the dialog's sheet and the keyboard went
//! to it, but the window server never put the sheet on the screen, so nothing seemed to happen. The sheet is
//! closed again, as Cancel would, and the window tells the user to reopen Nuzky; a newly started one showed them.
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSSavePanel, NSWindow, NSWindowWillBeginSheetNotification};
use objc2_foundation::{
    NSArray, NSDictionary, NSNotification, NSNotificationCenter, NSNumber, NSOperationQueue, NSString,
};
use tauri::{AppHandle, Emitter, Manager};

/// A sheet slides in within half a second; this leaves room for a busy computer.
const GRACE: Duration = Duration::from_secs(3);
/// Counts the sheets begun, so a check looks only at the sheet it was started for.
static SHEETS: AtomicU64 = AtomicU64::new(0);

pub fn watch(app: &tauri::App) -> Result<()> {
    let window = app.get_webview_window("main").context("the main window is missing")?;
    let ns_window = window.ns_window()?.cast::<AnyObject>();
    let app = app.handle().clone();
    let began = RcBlock::new(move |_: NonNull<NSNotification>| {
        let app = app.clone();
        let sheet = SHEETS.fetch_add(1, Ordering::Relaxed) + 1;
        std::thread::spawn(move || check(&app, sheet));
    });
    // SAFETY: the window lives as long as the app, and the main queue runs the block on the main thread.
    let observer = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSWindowWillBeginSheetNotification),
            Some(&*ns_window),
            Some(&NSOperationQueue::mainQueue()),
            &began,
        )
    };
    std::mem::forget(observer);
    Ok(())
}

fn check(app: &AppHandle, sheet: u64) {
    std::thread::sleep(GRACE);
    let handle = app.clone();
    let window = app.get_webview_window("main");
    let _ = app.run_on_main_thread(move || {
        if SHEETS.load(Ordering::Relaxed) != sheet {
            return;
        }
        let Some(ns_window) = window.and_then(|w| w.ns_window().ok()) else { return };
        // SAFETY: on the main thread, with the window still open.
        let ns_window = unsafe { &*ns_window.cast::<NSWindow>() };
        if close_hidden(ns_window) {
            log::warn!("macOS did not show a file dialog within {GRACE:?}; closed it");
            handle.emit("file-dialog-hidden", ()).ok();
        }
    });
}

/// A window on another Space or in the Dock hides its dialog too; that one is left alone.
fn close_hidden(window: &NSWindow) -> bool {
    let Some(panel) = window.attachedSheet().and_then(|sheet| sheet.downcast::<NSSavePanel>().ok()) else {
        return false;
    };
    if !on_screen(window.windowNumber()) || on_screen(panel.windowNumber()) {
        return false;
    }
    // SAFETY: on the main thread; Cancel ends the sheet and answers its caller with no file.
    unsafe { panel.cancel(None) };
    true
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCopyWindowInfo(
        option: u32,
        relative_to_window: u32,
    ) -> *mut NSArray<NSDictionary<NSString, AnyObject>>;
}

/// `kCGWindowListOptionIncludingWindow`: only the window asked about.
const INCLUDING_WINDOW: u32 = 1 << 3;

/// Asks the window server, since AppKit kept the hidden sheets attached as if shown.
pub(crate) fn on_screen(number: isize) -> bool {
    // SAFETY: a Copy function, so the array is ours to release.
    let Some(windows) = (unsafe { Retained::from_raw(CGWindowListCopyWindowInfo(INCLUDING_WINDOW, number as u32)) })
    else {
        return false;
    };
    windows.iter().any(|window| {
        window
            .objectForKey(&NSString::from_str("kCGWindowIsOnscreen"))
            .and_then(|value| value.downcast::<NSNumber>().ok())
            .is_some_and(|value| value.boolValue())
    })
}

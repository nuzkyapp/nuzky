//! Which webview may call which command, checked against the real capabilities and app manifest. The handler here
//! answers every command it is reached with, so an error can only come from Tauri's ACL check before it.

use crate::commands::{COMMANDS, PREVIEW_WINDOW_COMMANDS};
use std::collections::BTreeSet;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, mock_builder};
use tauri::webview::InvokeRequest;
use tauri::{WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const LOCAL_URLS: [&str; 2] =
    [if cfg!(windows) { "http://tauri.localhost/" } else { "tauri://localhost/" }, "http://localhost:1420/"];

fn app() -> tauri::App<MockRuntime> {
    mock_builder()
        .invoke_handler(|invoke| {
            invoke.resolver.resolve(());
            true
        })
        .build(tauri::generate_context!(test = true))
        .unwrap()
}

fn call(webview: &WebviewWindow<MockRuntime>, cmd: &str, url: &str) -> Result<(), String> {
    let request = InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: url.parse().unwrap(),
        body: InvokeBody::default(),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.into(),
    };
    tauri::test::get_ipc_response(webview, request).map(|_| ()).map_err(|error| error.to_string())
}

#[test]
fn main_webview_may_call_every_command() {
    let app = app();
    let main = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default()).build().unwrap();
    for url in LOCAL_URLS {
        for command in COMMANDS {
            assert_eq!(call(&main, command, url), Ok(()), "{command} from {url}");
        }
    }
}

#[test]
fn other_webviews_and_remote_pages_get_nothing() {
    const PROBES: [&str; 5] = ["run_plugin", "allow_plugin_access", "apply_edit", "boot", "plugin:event|listen"];
    let app = app();
    let main = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default()).build().unwrap();
    let panel = WebviewWindowBuilder::new(&app, "plugin-panel", WebviewUrl::App("index.html".into())).build().unwrap();
    for command in PROBES {
        for url in LOCAL_URLS {
            let refused = call(&panel, command, url).expect_err(&format!("plugin-panel reached {command} from {url}"));
            // A child webview of the main window would carry the window label "main", which the mock runtime cannot
            // make without the `unstable` feature. Tauri's debug message names what the grant matches on instead.
            if cfg!(debug_assertions) {
                assert!(!refused.contains("windows:"), "{command} is granted by window label: {refused}");
            }
        }
        call(&main, command, "https://example.com/").expect_err(&format!("a remote page reached {command}"));
    }
}

#[test]
fn preview_window_gets_only_its_own_commands() {
    let app = app();
    let window = WebviewWindowBuilder::new(&app, "preview", WebviewUrl::default()).build().unwrap();
    for url in LOCAL_URLS {
        for command in COMMANDS {
            let allowed = PREVIEW_WINDOW_COMMANDS.contains(command);
            assert_eq!(call(&window, command, url).is_ok(), allowed, "{command} from {url}");
        }
        // Without arguments an allowed call fails on them, after the permission check.
        let refused = |command: &str| call(&window, command, url).is_err_and(|e| e.contains("not allowed"));
        assert!(!refused("plugin:event|listen"), "the preview window follows playback");
        assert!(refused("plugin:window|maximize"), "the preview window may change only its full screen");
    }
    call(&window, "preview_window_boot", "https://example.com/")
        .expect_err("a remote page reached the preview window's commands");
}

#[test]
fn commands_list_matches_the_handler() {
    let source = include_str!("lib.rs");
    let start = source.find("generate_handler![").expect("lib.rs registers its commands") + "generate_handler![".len();
    let end = start + source[start..].find(']').unwrap();
    let handled: BTreeSet<&str> = source[start..end]
        .split(',')
        .map(|path| path.trim().rsplit("::").next().unwrap())
        .filter(|name| !name.is_empty())
        .collect();
    let listed: BTreeSet<&str> = COMMANDS.iter().copied().collect();
    assert_eq!(listed.len(), COMMANDS.len(), "src-tauri/src/commands.rs lists a command twice");
    let missing: Vec<_> = handled.difference(&listed).collect();
    let extra: Vec<_> = listed.difference(&handled).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "add {missing:?} to and remove {extra:?} from src-tauri/src/commands.rs, which must list what generate_handler! does"
    );
}

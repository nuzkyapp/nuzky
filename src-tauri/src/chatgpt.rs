//! Sign in with ChatGPT (`crates/chatgpt`): the commands behind the ChatGPT dialog. The frontend gets
//! the status only, never a token.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nuzky_chatgpt::{ChatGpt, Status};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::{CmdResult, err};

/// The sign-in waiting for the browser, so Cancel and a new attempt can end it.
static WAITING: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

/// Debug builds can sign in to a test server on 127.0.0.1 (`NUZKY_CHATGPT_ISSUER`), which also
/// stands in for the API, and keep the tokens in a file (`NUZKY_TEST_KEYCHAIN`), as the end-to-end
/// flows do; release builds always use OpenAI and the system keychain.
pub(crate) fn chatgpt() -> ChatGpt {
    let dir = crate::store::data_dir();
    #[cfg(debug_assertions)]
    if let Ok(issuer) = std::env::var("NUZKY_CHATGPT_ISSUER")
        && issuer.starts_with("http://127.0.0.1:")
    {
        let vault = match std::env::var_os("NUZKY_TEST_KEYCHAIN") {
            Some(path) => nuzky_chatgpt::Vault::File(path.into()),
            None => nuzky_chatgpt::Vault::Keychain,
        };
        return ChatGpt::with(issuer, dir, vault);
    }
    ChatGpt::new(dir)
}

/// Debug builds hand the address to `NUZKY_TEST_BROWSER` instead of the system browser.
fn open_browser(app: &AppHandle, url: &str) -> anyhow::Result<()> {
    #[cfg(debug_assertions)]
    if let Some(browser) = std::env::var_os("NUZKY_TEST_BROWSER") {
        std::process::Command::new(browser).arg(url).spawn()?;
        return Ok(());
    }
    Ok(app.opener().open_url(url, None::<&str>)?)
}

/// The account; with `renew`, its tokens renewed when they are about to expire.
#[tauri::command]
pub async fn chatgpt_status(renew: bool) -> CmdResult<Status> {
    tauri::async_runtime::spawn_blocking(move || chatgpt().status(renew).map_err(err)).await.map_err(err)?
}

/// Opens the browser to sign in and answers once it came back, was cancelled or timed out.
#[tauri::command]
pub async fn chatgpt_sign_in(app: AppHandle, another_account: bool) -> CmdResult<Status> {
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(earlier) = WAITING.lock().unwrap().replace(cancel.clone()) {
        earlier.store(true, Ordering::Relaxed);
    }
    let result = tauri::async_runtime::spawn_blocking({
        let cancel = cancel.clone();
        move || chatgpt().sign_in(another_account, &cancel, |url| open_browser(&app, url)).map_err(err)
    })
    .await
    .map_err(err)?;
    let mut waiting = WAITING.lock().unwrap();
    if waiting.as_ref().is_some_and(|current| Arc::ptr_eq(current, &cancel)) {
        *waiting = None;
    }
    result
}

#[tauri::command]
pub fn chatgpt_cancel_sign_in() {
    if let Some(waiting) = WAITING.lock().unwrap().take() {
        waiting.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
pub async fn chatgpt_sign_out() -> CmdResult<Status> {
    tauri::async_runtime::spawn_blocking(|| chatgpt().sign_out().map_err(err)).await.map_err(err)?
}

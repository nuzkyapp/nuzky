//! The optional Nuzky account (`crates/account`): the commands behind Settings › Account. The
//! frontend gets the status only, never a token.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nuzky_account::{Account, Provider, Status};
use tauri::AppHandle;

use crate::{CmdResult, err};

/// The sign-in waiting for the browser, so Cancel and a new attempt can end it.
static WAITING: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

/// Debug builds can sign in to a test server on 127.0.0.1 (`NUZKY_ACCOUNT_ISSUER`) and keep the
/// tokens in a file (`NUZKY_TEST_ACCOUNT_KEYCHAIN`), as the end-to-end flow does; release builds
/// always use account.nuzky.app and the system keychain.
fn account() -> Account {
    let dir = crate::store::data_dir();
    #[cfg(debug_assertions)]
    if let Ok(issuer) = std::env::var("NUZKY_ACCOUNT_ISSUER")
        && issuer.starts_with("http://127.0.0.1:")
    {
        let vault = match std::env::var_os("NUZKY_TEST_ACCOUNT_KEYCHAIN") {
            Some(path) => nuzky_account::Vault::File(path.into()),
            None => nuzky_account::Vault::Keychain,
        };
        return Account::with(issuer, dir, vault);
    }
    Account::new(dir)
}

/// Whether Settings shows the account. Only debug builds do, until the account keeps something
/// for the user.
#[tauri::command]
pub fn account_available() -> bool {
    cfg!(debug_assertions)
}

/// The account; with `renew`, its tokens renewed when they are about to expire.
#[tauri::command]
pub async fn account_status(renew: bool) -> CmdResult<Status> {
    tauri::async_runtime::spawn_blocking(move || account().status(renew).map_err(err)).await.map_err(err)?
}

/// Opens the browser to sign in and answers once it came back, was cancelled or timed out.
#[tauri::command]
pub async fn account_sign_in(app: AppHandle, provider: Provider, another_account: bool) -> CmdResult<Status> {
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(earlier) = WAITING.lock().unwrap().replace(cancel.clone()) {
        earlier.store(true, Ordering::Relaxed);
    }
    let result = tauri::async_runtime::spawn_blocking({
        let cancel = cancel.clone();
        move || {
            account()
                .sign_in(provider, another_account, &cancel, |url| crate::chatgpt::open_browser(&app, url))
                .map_err(err)
        }
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
pub fn account_cancel_sign_in() {
    if let Some(waiting) = WAITING.lock().unwrap().take() {
        waiting.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
pub async fn account_sign_out() -> CmdResult<Status> {
    tauri::async_runtime::spawn_blocking(|| account().sign_out().map_err(err)).await.map_err(err)?
}

/// Opens the account's page in the browser, where the user changes their name or password or deletes the account.
#[tauri::command]
pub fn account_manage(app: AppHandle) -> CmdResult<()> {
    crate::chatgpt::open_browser(&app, &account().manage_url()).map_err(err)
}

//! What the sign-in keeps: the tokens in the system keychain, and beside Nuzky's data the host id
//! and which account Nuzky is registered with, which are not secret.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::oauth::{PLAN_SCOPE, TokenResponse};

const KEYCHAIN_SERVICE: &str = "Nuzky";

/// This computer's sign-in. The host id stays for the life of the data folder; the account stays
/// after signing out, so signing in again reuses Nuzky's registration with it.
#[derive(Serialize, Deserialize)]
pub(crate) struct Registration {
    /// `urn:uuid:…`, sent as `ext_agent_host_id`.
    pub host_id: String,
    /// The client ChatGPT issued to Nuzky for the account (`oaiapp_…`).
    #[serde(default)]
    pub client_id: Option<String>,
    /// The account's `sub`, checked when the same registration signs in again.
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

impl Registration {
    fn path(dir: &Path) -> PathBuf {
        dir.join("chatgpt.json")
    }

    /// None when there is none yet; an unreadable file counts as none and is replaced by the next
    /// sign-in.
    pub fn load(dir: &Path) -> Option<Self> {
        let text = fs::read(Self::path(dir)).ok()?;
        serde_json::from_slice(&text).inspect_err(|error| log::warn!("Reading chatgpt.json failed: {error}")).ok()
    }

    /// The host id is kept before the first sign-in uses it.
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        if let Some(reg) = Self::load(dir) {
            return Ok(reg);
        }
        let reg =
            Self { host_id: format!("urn:uuid:{}", uuid::Uuid::new_v4()), client_id: None, subject: None, email: None };
        reg.save(dir)?;
        Ok(reg)
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        write_private(&Self::path(dir), &serde_json::to_vec_pretty(self)?)
    }
}

/// One token set, replaced as a whole, so the access token, its expiry, the scopes and the rotated
/// refresh token always belong together. No Debug, so it cannot end up in a log.
#[derive(Serialize, Deserialize)]
pub(crate) struct Tokens {
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Kept to name the account to ChatGPT on the next sign-in (`id_token_hint`).
    pub id_token: String,
    /// Unix seconds.
    pub expires_at: u64,
    pub scopes: Vec<String>,
}

impl Tokens {
    /// From the code exchange. Without a refresh token Nuzky would be signed out within the hour.
    pub fn issued(client_id: &str, id_token: String, response: TokenResponse, now: u64) -> Result<Self> {
        Ok(Self {
            client_id: client_id.into(),
            refresh_token: response.refresh_token.context("ChatGPT sent no refresh token")?,
            id_token,
            expires_at: now + response.expires_in.unwrap_or(3600),
            scopes: scopes(response.scope.as_deref().unwrap_or_default()),
            access_token: response.access_token,
        })
    }

    /// After a refresh. What the answer leaves out stays as it was, as OAuth says.
    pub fn renewed(self, response: TokenResponse, now: u64) -> Self {
        Self {
            client_id: self.client_id,
            refresh_token: response.refresh_token.unwrap_or(self.refresh_token),
            id_token: response.id_token.unwrap_or(self.id_token),
            expires_at: now + response.expires_in.unwrap_or(3600),
            scopes: response.scope.as_deref().map_or(self.scopes, scopes),
            access_token: response.access_token,
        }
    }

    pub fn plan(&self) -> bool {
        self.scopes.iter().any(|scope| scope == PLAN_SCOPE)
    }
}

fn scopes(granted: &str) -> Vec<String> {
    granted.split_whitespace().map(str::to_owned).collect()
}

/// Where the tokens live.
pub enum Vault {
    /// The system keychain: Keychain on macOS, Credential Manager on Windows, the Secret Service
    /// (GNOME Keyring, KWallet) on Linux.
    Keychain,
    /// A file with owner-only access, standing in for the keychain in tests.
    File(PathBuf),
}

impl Vault {
    /// Fails when there is no keychain to keep a sign-in in, such as Linux without a Secret Service.
    pub(crate) fn check(&self) -> Result<()> {
        match self {
            Vault::Keychain => keyring::Entry::store_status().as_ref().map(|_| ()).map_err(unavailable),
            Vault::File(_) => Ok(()),
        }
    }

    pub(crate) fn load(&self, host_id: &str) -> Result<Option<Tokens>> {
        let text = match self {
            Vault::Keychain => match read_keychain(host_id)? {
                Some(text) => text,
                None => return Ok(None),
            },
            Vault::File(path) => match fs::read_to_string(path) {
                Ok(text) => text,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            },
        };
        // Damaged tokens are no sign-in; the next sign-in replaces them.
        Ok(serde_json::from_str(&text)
            .inspect_err(|error| log::warn!("Reading the ChatGPT tokens failed: {error}"))
            .ok())
    }

    pub(crate) fn save(&self, host_id: &str, tokens: &Tokens) -> Result<()> {
        let text = serde_json::to_string(tokens)?;
        match self {
            Vault::Keychain => write_keychain(host_id, &text),
            Vault::File(path) => write_private(path, text.as_bytes()),
        }
    }

    pub(crate) fn delete(&self, host_id: &str) -> Result<()> {
        match self {
            Vault::Keychain => delete_keychain(host_id),
            Vault::File(path) => match fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
                _ => Ok(()),
            },
        }
    }
}

/// One entry per data folder, so a test or a second data folder never reads another's tokens.
fn entry(host_id: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, &format!("ChatGPT {host_id}")).map_err(|error| unavailable(&error))
}

#[cfg(not(windows))]
fn read_keychain(host_id: &str) -> Result<Option<String>> {
    match entry(host_id)?.get_password() {
        Ok(text) => Ok(Some(text)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(unavailable(&error)),
    }
}

#[cfg(not(windows))]
fn write_keychain(host_id: &str, text: &str) -> Result<()> {
    entry(host_id)?.set_password(text).map_err(|error| unavailable(&error))
}

#[cfg(not(windows))]
fn delete_keychain(host_id: &str) -> Result<()> {
    match entry(host_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(unavailable(&error)),
    }
}

/// Credential Manager keeps at most 2560 bytes in an entry, fewer than the tokens take, so on Windows they are
/// split over the entry and numbered ones after it.
#[cfg(windows)]
const PART_BYTES: usize = 2560;

#[cfg(windows)]
fn part(host_id: &str, index: usize) -> Result<keyring::Entry> {
    match index {
        0 => entry(host_id),
        _ => keyring::Entry::new(KEYCHAIN_SERVICE, &format!("ChatGPT {host_id} part {}", index + 1))
            .map_err(|error| unavailable(&error)),
    }
}

#[cfg(windows)]
fn read_keychain(host_id: &str) -> Result<Option<String>> {
    let mut bytes = Vec::new();
    for index in 0.. {
        match part(host_id, index)?.get_secret() {
            Ok(chunk) => bytes.extend(chunk),
            Err(keyring::Error::NoEntry) if index == 0 => return Ok(None),
            Err(keyring::Error::NoEntry) => break,
            Err(error) => return Err(unavailable(&error)),
        }
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(windows)]
fn write_keychain(host_id: &str, text: &str) -> Result<()> {
    let parts: Vec<&[u8]> = text.as_bytes().chunks(PART_BYTES).collect();
    for (index, bytes) in parts.iter().enumerate() {
        part(host_id, index)?.set_secret(bytes).map_err(|error| unavailable(&error))?;
    }
    // Parts a longer sign-in left behind would read as the end of this one.
    delete_parts(host_id, parts.len())
}

#[cfg(windows)]
fn delete_keychain(host_id: &str) -> Result<()> {
    delete_parts(host_id, 0)
}

#[cfg(windows)]
fn delete_parts(host_id: &str, from: usize) -> Result<()> {
    for index in from.. {
        match part(host_id, index)?.delete_credential() {
            Ok(()) => {}
            Err(keyring::Error::NoEntry) => break,
            Err(error) => return Err(unavailable(&error)),
        }
    }
    Ok(())
}

fn unavailable(error: &keyring::Error) -> anyhow::Error {
    anyhow!(
        "KEYCHAIN_UNAVAILABLE: Nuzky keeps your ChatGPT sign-in in the system keychain, which it could not use ({error})"
    )
}

/// Held while the sign-in is read or changed, so two windows never refresh the same rotating token.
pub(crate) fn lock(dir: &Path) -> Result<File> {
    fs::create_dir_all(dir).with_context(|| format!("Creating {}", dir.display()))?;
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join("chatgpt.lock"))?;
    lock.lock().context("Locking the ChatGPT sign-in")?;
    Ok(lock)
}

/// Replaces the file in one step, readable only by its owner.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let result = (|| {
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.with_context(|| format!("Saving {}", path.display()))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn credential_manager_keeps_a_sign_in_longer_than_one_entry() {
        let host = format!("test-{}", uuid::Uuid::new_v4().simple());
        let long = format!("{{\"tokens\":\"{}\"}}", "x".repeat(6000));
        write_keychain(&host, &long).unwrap();
        let read = read_keychain(&host);
        let shorter = r#"{"tokens":"short"}"#;
        let replaced = write_keychain(&host, shorter).and_then(|()| read_keychain(&host));
        let deleted = delete_keychain(&host).and_then(|()| read_keychain(&host));
        assert_eq!(read.unwrap().as_deref(), Some(long.as_str()));
        assert_eq!(replaced.unwrap().as_deref(), Some(shorter));
        assert_eq!(deleted.unwrap(), None);
    }
}

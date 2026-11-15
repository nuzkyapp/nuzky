//! Where a sign-in keeps its tokens: the system keychain, one entry per sign-in and data folder.
//! Beside Nuzky's data, files written in one step and readable only by their owner, and the lock
//! held while a sign-in is read or changed.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use serde::de::DeserializeOwned;

const KEYCHAIN_SERVICE: &str = "Nuzky";

/// Where the tokens live.
pub enum Vault {
    /// The system keychain: Keychain on macOS, Credential Manager on Windows, the Secret Service
    /// (GNOME Keyring, KWallet) on Linux.
    Keychain,
    /// A file with owner-only access, standing in for the keychain in tests.
    File(PathBuf),
}

/// One sign-in's entry: `name` says whose it is ("ChatGPT"), `host_id` keeps a test or a second
/// data folder from reading another's tokens.
#[derive(Clone, Copy)]
pub struct Entry<'a> {
    pub name: &'a str,
    pub host_id: &'a str,
}

impl Vault {
    /// Fails when there is no keychain to keep a sign-in in, such as Linux without a Secret Service.
    pub fn check(&self, name: &str) -> Result<()> {
        match self {
            Vault::Keychain => keyring::Entry::store_status().as_ref().map(|_| ()).map_err(|e| unavailable(name, e)),
            Vault::File(_) => Ok(()),
        }
    }

    pub fn load<T: DeserializeOwned>(&self, entry: Entry) -> Result<Option<T>> {
        let text = match self {
            Vault::Keychain => match read_keychain(entry)? {
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
            .inspect_err(|error| log::warn!("Reading the {} tokens failed: {error}", entry.name))
            .ok())
    }

    pub fn save<T: Serialize>(&self, entry: Entry, tokens: &T) -> Result<()> {
        let text = serde_json::to_string(tokens)?;
        match self {
            Vault::Keychain => write_keychain(entry, &text),
            Vault::File(path) => write_private(path, text.as_bytes()),
        }
    }

    pub fn delete(&self, entry: Entry) -> Result<()> {
        match self {
            Vault::Keychain => delete_keychain(entry),
            Vault::File(path) => match fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
                _ => Ok(()),
            },
        }
    }
}

fn keychain_entry(entry: Entry) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, &format!("{} {}", entry.name, entry.host_id))
        .map_err(|error| unavailable(entry.name, &error))
}

#[cfg(not(windows))]
fn read_keychain(entry: Entry) -> Result<Option<String>> {
    match keychain_entry(entry)?.get_password() {
        Ok(text) => Ok(Some(text)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(unavailable(entry.name, &error)),
    }
}

#[cfg(not(windows))]
fn write_keychain(entry: Entry, text: &str) -> Result<()> {
    keychain_entry(entry)?.set_password(text).map_err(|error| unavailable(entry.name, &error))
}

#[cfg(not(windows))]
fn delete_keychain(entry: Entry) -> Result<()> {
    match keychain_entry(entry)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(unavailable(entry.name, &error)),
    }
}

/// Credential Manager keeps at most 2560 bytes in an entry, fewer than the tokens take, so on Windows they are
/// split over the entry and numbered ones after it, never more than a sign-in takes many times over.
#[cfg(windows)]
const PART_BYTES: usize = 2560;
#[cfg(windows)]
const MAX_PARTS: usize = 16;

#[cfg(windows)]
fn part(entry: Entry, index: usize) -> Result<keyring::Entry> {
    match index {
        0 => keychain_entry(entry),
        _ => keyring::Entry::new(KEYCHAIN_SERVICE, &format!("{} {} part {}", entry.name, entry.host_id, index + 1))
            .map_err(|error| unavailable(entry.name, &error)),
    }
}

#[cfg(windows)]
fn read_keychain(entry: Entry) -> Result<Option<String>> {
    let mut bytes = Vec::new();
    for index in 0.. {
        match part(entry, index)?.get_secret() {
            Ok(chunk) => bytes.extend(chunk),
            Err(keyring::Error::NoEntry) if index == 0 => return Ok(None),
            Err(keyring::Error::NoEntry) => break,
            Err(error) => return Err(unavailable(entry.name, &error)),
        }
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(windows)]
fn write_keychain(entry: Entry, text: &str) -> Result<()> {
    let parts: Vec<&[u8]> = text.as_bytes().chunks(PART_BYTES).collect();
    anyhow::ensure!(
        parts.len() <= MAX_PARTS,
        "KEYCHAIN_UNAVAILABLE: the {} sign-in is too large for Credential Manager",
        entry.name
    );
    for (index, bytes) in parts.iter().enumerate() {
        part(entry, index)?.set_secret(bytes).map_err(|error| unavailable(entry.name, &error))?;
    }
    // Parts a longer sign-in left behind would read as the end of this one.
    delete_parts(entry, parts.len())
}

#[cfg(windows)]
fn delete_keychain(entry: Entry) -> Result<()> {
    delete_parts(entry, 0)
}

/// Every part from `from` on, also past a gap a removal cut short left.
#[cfg(windows)]
fn delete_parts(entry: Entry, from: usize) -> Result<()> {
    for index in from..MAX_PARTS {
        match part(entry, index)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(error) => return Err(unavailable(entry.name, &error)),
        }
    }
    Ok(())
}

fn unavailable(name: &str, error: &keyring::Error) -> anyhow::Error {
    anyhow!(
        "KEYCHAIN_UNAVAILABLE: Nuzky keeps your {name} sign-in in the system keychain, which it could not use ({error})"
    )
}

/// Held while the sign-in is read or changed, so two windows never refresh the same rotating token.
/// `file` is the lock's name in `dir`, `name` whose sign-in it guards.
pub fn lock(dir: &Path, file: &str, name: &str) -> Result<File> {
    fs::create_dir_all(dir).with_context(|| format!("Creating {}", dir.display()))?;
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join(file))?;
    lock.lock().with_context(|| format!("Locking the {name} sign-in"))?;
    Ok(lock)
}

/// Replaces the file in one step, readable only by its owner.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
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
        let entry = Entry { name: "ChatGPT", host_id: &host };
        let long = format!("{{\"tokens\":\"{}\"}}", "x".repeat(6000));
        write_keychain(entry, &long).unwrap();
        let read = read_keychain(entry);
        let shorter = r#"{"tokens":"short"}"#;
        let replaced = write_keychain(entry, shorter).and_then(|()| read_keychain(entry));
        let deleted = delete_keychain(entry).and_then(|()| read_keychain(entry));
        // A sign-out cut short after the first part: signing out again still removes the rest.
        write_keychain(entry, &long).unwrap();
        part(entry, 0).unwrap().delete_credential().unwrap();
        let left =
            delete_keychain(entry).and_then(|()| part(entry, 1)?.get_secret().map_err(|e| unavailable(entry.name, &e)));
        assert_eq!(read.unwrap().as_deref(), Some(long.as_str()));
        assert_eq!(replaced.unwrap().as_deref(), Some(shorter));
        assert_eq!(deleted.unwrap(), None);
        assert!(left.is_err(), "a part outlived signing out");
    }
}

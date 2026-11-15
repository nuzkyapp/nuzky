//! What the sign-in keeps: the tokens in the system keychain, and beside Nuzky's data the host id
//! and which account Nuzky is registered with, which are not secret.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use nuzky_oauth::{Entry, write_private};
use serde::{Deserialize, Serialize};

use crate::oauth::{PLAN_SCOPE, TokenResponse};

/// Whose sign-in the keychain entry and the messages name.
pub(crate) const NAME: &str = "ChatGPT";

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

    /// This data folder's tokens in the keychain.
    pub fn entry(&self) -> Entry<'_> {
        Entry { name: NAME, host_id: &self.host_id }
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

/// Held while the sign-in is read or changed, so two windows never refresh the same rotating token.
pub(crate) fn lock(dir: &Path) -> Result<File> {
    nuzky_oauth::lock(dir, "chatgpt.lock", NAME)
}

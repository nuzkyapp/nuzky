//! What Nuzky's sign-ins through the system browser share (RFC 8252): a listener on 127.0.0.1 the
//! browser comes back to, PKCE, the token and revocation requests, and the tokens in the system
//! keychain. Sign in with ChatGPT (`crates/chatgpt`) uses it.

pub mod listener;
mod vault;

use std::time::Duration;

use anyhow::{Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::digest;
use ring::rand::{SecureRandom, SystemRandom};
use serde::Deserialize;

pub use listener::{CALLBACK_PATH, Listener, Request};
pub use vault::{Entry, Vault, lock, write_private};

/// Token answers are a few kilobytes.
pub const MAX_BYTES: u64 = 256 * 1024;

/// An agent that never follows a redirect and gives up after 15 s.
pub fn http() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(concat!("Nuzky/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// The secrets of one sign-in attempt, fresh each time.
pub struct Attempt {
    pub state: String,
    pub nonce: String,
    pub verifier: String,
    pub redirect_uri: String,
}

impl Attempt {
    pub fn new(redirect_uri: String) -> Self {
        Self { state: random(), nonce: random(), verifier: random(), redirect_uri }
    }

    /// PKCE S256 for this attempt's verifier.
    pub fn challenge(&self) -> String {
        challenge(&self.verifier)
    }
}

/// 32 random bytes as base64url: 43 characters, also a valid PKCE verifier.
fn random() -> String {
    let mut bytes = [0u8; 32];
    SystemRandom::new().fill(&mut bytes).expect("the system's random number generator works");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// PKCE S256: the base64url SHA-256 of the verifier, without padding.
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()))
}

/// Percent-encoding with spaces as %20, which every server reads as a space.
pub fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>().replace('+', "%20")
}

/// `url` with the query, each value encoded.
pub fn with_query(url: &str, query: &[(&str, &str)]) -> String {
    let query: Vec<String> = query.iter().map(|(key, value)| format!("{key}={}", encode(value))).collect();
    format!("{url}?{}", query.join("&"))
}

/// One parameter of a query.
pub fn param(query: &str, name: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes()).find(|(key, _)| key == name).map(|(_, value)| value.into_owned())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Callback {
    Code(String),
    Denied,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CallbackError {
    /// Not the answer to this attempt.
    WrongState,
    Failed(String),
}

/// The browser's return to /auth/callback; `who` names the service in what goes wrong.
pub fn parse_callback(query: &str, state: &str, who: &str) -> Result<Callback, CallbackError> {
    if param(query, "state").as_deref() != Some(state) {
        return Err(CallbackError::WrongState);
    }
    if let Some(error) = param(query, "error") {
        if error == "access_denied" {
            return Ok(Callback::Denied);
        }
        let why = param(query, "error_description").unwrap_or(error);
        return Err(CallbackError::Failed(format!("{who} refused the sign-in: {why}")));
    }
    match param(query, "code").filter(|code| !code.is_empty()) {
        Some(code) => Ok(Callback::Code(code)),
        None => Err(CallbackError::Failed(format!("{who} sent no authorization code"))),
    }
}

#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub expires_in: Option<u64>,
    /// The granted scopes, separated by spaces.
    pub scope: Option<String>,
}

pub enum TokenError {
    /// The token endpoint answered with an OAuth error code.
    Refused { code: String, description: Option<String> },
    /// The network, or an answer that is no OAuth error.
    Other(anyhow::Error),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::Refused { code, description } => {
                write!(f, "{code}{}", description.as_ref().map(|d| format!(": {d}")).unwrap_or_default())
            }
            TokenError::Other(error) => write!(f, "{error:#}"),
        }
    }
}

/// A request to the token endpoint at `url`: a code exchange or a refresh.
pub fn token_request(http: &ureq::Agent, url: &str, form: &[(&str, &str)]) -> Result<TokenResponse, TokenError> {
    let (status, body) = post_form(http, url, form).map_err(TokenError::Other)?;
    if status == 200 {
        // serde's message names the line and column, never the text, which holds the tokens.
        return serde_json::from_str(&body).map_err(|error| TokenError::Other(anyhow!("Reading the tokens: {error}")));
    }
    match oauth_error(&body) {
        Some((code, description)) if (400..500).contains(&status) => Err(TokenError::Refused { code, description }),
        _ => Err(TokenError::Other(anyhow!("The token endpoint answered {status}"))),
    }
}

/// `{"error": "code"}` as OAuth has it, or `{"error": {"code": …}}` as OpenAI's API does.
fn oauth_error(body: &str) -> Option<(String, Option<String>)> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let text = |v: &serde_json::Value| v.as_str().map(str::to_owned);
    match &value["error"] {
        serde_json::Value::String(code) => Some((code.clone(), text(&value["error_description"]))),
        error @ serde_json::Value::Object(_) => Some((text(&error["code"])?, text(&error["message"]))),
        _ => None,
    }
}

/// Ends a renewable session at the revocation endpoint `url` (RFC 7009). True once the server
/// confirmed it; a failed connection or a server error is tried three times. `who` names the
/// service in the log.
pub fn revoke(http: &ureq::Agent, url: &str, client_id: &str, refresh_token: &str, who: &str) -> bool {
    let form = [("token", refresh_token), ("token_type_hint", "refresh_token"), ("client_id", client_id)];
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(500 * attempt));
        }
        match post_form(http, url, &form) {
            Ok((200, _)) => return true,
            Ok((status, _)) if status < 500 => {
                log::warn!("Ending the {who} session was refused ({status})");
                return false;
            }
            Ok((status, _)) => log::warn!("Ending the {who} session failed ({status})"),
            Err(error) => log::warn!("Ending the {who} session failed: {error:#}"),
        }
    }
    false
}

fn post_form(http: &ureq::Agent, url: &str, form: &[(&str, &str)]) -> Result<(u16, String)> {
    let mut response = http.post(url).header("Accept", "application/json").send_form(form.iter().copied())?;
    let status = response.status().as_u16();
    let body = response.body_mut().with_config().limit(MAX_BYTES).read_to_string()?;
    Ok((status, body))
}

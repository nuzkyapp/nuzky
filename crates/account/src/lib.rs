//! The optional Nuzky account at account.nuzky.app (`account/` in this repository). The system
//! browser signs in, with an email and password or with Google, a listener on 127.0.0.1 takes the
//! code back, and the tokens live only in the system keychain; the frontend gets the status, never
//! a token. Nothing in Nuzky needs the account. Signed in, it keeps documents the app syncs between
//! the user's computers, which the app reads and writes here, so the token never leaves this crate.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail, ensure};
pub use nuzky_oauth::Vault;
use nuzky_oauth::{Attempt, Callback, CallbackError, Entry, Listener, TokenError, TokenResponse};
use serde::{Deserialize, Serialize};

pub const ISSUER: &str = "https://account.nuzky.app";
/// Nuzky's one client at the account server; it has no secret and must use PKCE.
const CLIENT_ID: &str = "nuzky-desktop";
/// `nuzky` lets the token reach the account's settings later without signing in again.
const SCOPE: &str = "openid email profile offline_access nuzky";
const AUTHORIZE: &str = "/api/auth/oauth2/authorize";
const TOKEN: &str = "/api/auth/oauth2/token";
const REVOKE: &str = "/api/auth/oauth2/revoke";
const USERINFO: &str = "/api/auth/oauth2/userinfo";
const SYNC: &str = "/api/auth/sync";
/// The server keeps each document under 256 KB; the answer holds every kind.
const MAX_DOCUMENTS: u64 = 4 << 20;
/// Whose sign-in the keychain entry and the messages name.
const NAME: &str = "Nuzky account";
const WHO: &str = "Nuzky's account server";
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Tokens this close to expiring are renewed first; access tokens live an hour.
const RENEW_BEFORE: u64 = 5 * 60;
/// Refresh errors after which the refresh token is no use: it expired, was revoked, or the account was deleted.
const UNUSABLE_REFRESH: [&str; 2] = ["invalid_grant", "invalid_client"];

const PAGE_DENIED: &str = "Signing in was cancelled. Go back to Nuzky; you can close this tab.";
const PAGE_FAILED: &str = "Signing in didn't work. Go back to Nuzky to see why.";
const PAGE_STALE: &str = "This sign-in page is out of date. Go back to Nuzky and sign in again.";

/// What Settings › Account shows. It never carries a token.
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "AccountStatus"))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Status {
    /// `email`: the account signed in to before, when there was one.
    SignedOut {
        email: Option<String>,
        notice: Option<Notice>,
    },
    SignedIn {
        email: String,
        name: Option<String>,
    },
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "AccountNotice"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Notice {
    /// The sign-in could not be renewed: it expired, or the account was deleted.
    Ended,
    /// Signed out on this computer, but the server did not confirm ending the session.
    NotRevoked,
}

/// How the browser signs in: Google goes straight to Google's account chooser, Email to Nuzky's page.
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "AccountProvider"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Google,
    Email,
}

/// One document as the account keeps it: revision 0 without a body is none yet. The body is left out
/// when the app named this revision, so it has it already.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Remote {
    pub revision: u64,
    #[serde(default)]
    pub body: Option<serde_json::Value>,
}

/// The account's documents by kind, and the account they belong to.
#[derive(Debug)]
pub struct Documents {
    pub subject: String,
    pub kinds: BTreeMap<String, Remote>,
}

#[derive(Debug, PartialEq)]
pub enum Stored {
    /// The document's new revision.
    Revision(u64),
    /// Someone stored another revision meanwhile; this is the document as it is now.
    Conflict(Remote),
}

pub struct Account {
    issuer: String,
    /// Holds `account.json` (this computer's host id and the account signed in to) and the lock.
    dir: PathBuf,
    vault: Vault,
}

impl Account {
    /// account.nuzky.app, with the tokens in the system keychain.
    pub fn new(dir: PathBuf) -> Self {
        Self { issuer: ISSUER.into(), dir, vault: Vault::Keychain }
    }

    /// Another server and vault, for tests.
    pub fn with(issuer: String, dir: PathBuf, vault: Vault) -> Self {
        Self { issuer, dir, vault }
    }

    /// Where the user changes their name or password or deletes the account, in the browser.
    pub fn manage_url(&self) -> String {
        format!("{}/account", self.issuer)
    }

    /// The account as stored; with `renew`, its tokens renewed first when they expire within five
    /// minutes. Without the network the sign-in stays as it was.
    pub fn status(&self, renew: bool) -> Result<Status> {
        let _lock = lock(&self.dir)?;
        let Some(mut reg) = Registration::load(&self.dir) else { return Ok(signed_out(None, None)) };
        let Some(tokens) = self.vault.load::<Tokens>(reg.entry())? else { return Ok(signed_out(reg.email, None)) };
        if renew && tokens.expires_at <= now() + RENEW_BEFORE && !self.refresh(&mut reg, tokens)? {
            return Ok(signed_out(reg.email, Some(Notice::Ended)));
        }
        Ok(signed_in(reg))
    }

    /// False once the server refused the refresh token, which is then removed. The name and email
    /// are read again, as they may have changed in the browser.
    fn refresh(&self, reg: &mut Registration, tokens: Tokens) -> Result<bool> {
        let http = nuzky_oauth::http();
        let form =
            [("grant_type", "refresh_token"), ("client_id", CLIENT_ID), ("refresh_token", &tokens.refresh_token)];
        match nuzky_oauth::token_request(&http, &self.url(TOKEN), &form) {
            Ok(fresh) => {
                let tokens = tokens.renewed(fresh, now());
                self.vault.save(reg.entry(), &tokens)?;
                match userinfo(&http, &self.url(USERINFO), &tokens.access_token) {
                    Ok(who) if reg.subject.as_deref() == Some(who.sub.as_str()) => {
                        reg.email = Some(who.email);
                        reg.name = who.name;
                        reg.save(&self.dir)?;
                    }
                    Ok(_) => log::warn!("The Nuzky account server named another account after renewing"),
                    Err(error) => log::warn!("Reading the Nuzky account after renewing failed: {error:#}"),
                }
                Ok(true)
            }
            Err(TokenError::Refused { code, .. }) if UNUSABLE_REFRESH.contains(&code.as_str()) => {
                self.vault.delete(reg.entry())?;
                Ok(false)
            }
            Err(error) => {
                log::warn!("Renewing the Nuzky account sign-in failed; keeping the stored tokens: {error}");
                Ok(true)
            }
        }
    }

    /// Signs in through the system browser, which `open` shows the server's page in, and waits up
    /// to five minutes for it to come back. `another_account` asks for the sign-in page even when
    /// the browser is still signed in; Google always goes to Google.
    pub fn sign_in(
        &self,
        provider: Provider,
        another_account: bool,
        cancel: &AtomicBool,
        open: impl FnOnce(&str) -> Result<()>,
    ) -> Result<Status> {
        self.vault.check(NAME)?;
        {
            let _lock = lock(&self.dir)?;
            Registration::load_or_create(&self.dir)?;
        }
        let listener = Listener::bind()?;
        let attempt = Attempt::new(listener.redirect_uri()?);
        let challenge = attempt.challenge();
        let mut query = vec![
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", attempt.redirect_uri.as_str()),
            ("scope", SCOPE),
            ("state", &attempt.state),
            ("nonce", &attempt.nonce),
            ("code_challenge_method", "S256"),
            ("code_challenge", &challenge),
        ];
        // Google chosen in Nuzky goes to Google even when the browser is signed in to another account here.
        if provider == Provider::Google {
            query.push(("provider", "google"));
        }
        if another_account || provider == Provider::Google {
            query.push(("prompt", "login"));
        }
        if open(&nuzky_oauth::with_query(&self.url(AUTHORIZE), &query)).is_err() {
            bail!("SIGN_IN_FAILED: Could not open the browser");
        }

        let deadline = Instant::now() + SIGN_IN_TIMEOUT;
        loop {
            let request = listener.next(deadline, cancel)?;
            match nuzky_oauth::parse_callback(&request.query, &attempt.state, WHO) {
                // Not this attempt's answer, such as a tab left from an earlier one: keep waiting.
                Err(CallbackError::WrongState) => request.reply(400, PAGE_STALE),
                Err(CallbackError::Failed(why)) => {
                    request.reply(400, PAGE_FAILED);
                    bail!("SIGN_IN_FAILED: {why}");
                }
                Ok(Callback::Denied) => {
                    request.reply(200, PAGE_DENIED);
                    bail!("SIGN_IN_DENIED: Signing in was cancelled in the browser.");
                }
                Ok(Callback::Code(code)) => {
                    let result = self.finish(&attempt, &code, cancel);
                    match result {
                        Ok(_) => request.redirect(&format!("{}/done", self.issuer)),
                        Err(_) => request.reply(200, PAGE_FAILED),
                    }
                    return result;
                }
            }
        }
    }

    /// Exchanges the code, asks who signed in and keeps the tokens, ending the session of an
    /// account signed in to before only once this one is kept.
    fn finish(&self, attempt: &Attempt, code: &str, cancel: &AtomicBool) -> Result<Status> {
        let http = nuzky_oauth::http();
        let form = [
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("code_verifier", &attempt.verifier),
            ("redirect_uri", &attempt.redirect_uri),
        ];
        let response = nuzky_oauth::token_request(&http, &self.url(TOKEN), &form).map_err(|error| match error {
            TokenError::Refused { code, description } => {
                anyhow!(
                    "SIGN_IN_FAILED: {WHO} did not accept the sign-in ({}). Try again.",
                    description.unwrap_or(code)
                )
            }
            TokenError::Other(error) => anyhow!("SIGN_IN_FAILED: Could not reach {WHO}: {error}"),
        })?;
        let tokens =
            Tokens::issued(response, now()).context("SIGN_IN_FAILED: The server did not let Nuzky stay signed in")?;
        let who = userinfo(&http, &self.url(USERINFO), &tokens.access_token)
            .context("SIGN_IN_FAILED: Could not read which account signed in")?;

        let (reg, replaced) = {
            let _lock = lock(&self.dir)?;
            // Cancel, or another sign-in started meanwhile, wins over this answer.
            if cancel.load(Ordering::Relaxed) {
                bail!("CANCELLED: Signing in was cancelled.");
            }
            let mut reg = Registration::load_or_create(&self.dir)?;
            let replaced = self.vault.load::<Tokens>(reg.entry()).ok().flatten();
            self.vault.save(reg.entry(), &tokens)?;
            reg.subject = Some(who.sub);
            reg.email = Some(who.email);
            reg.name = who.name;
            reg.save(&self.dir)?;
            (reg, replaced)
        };
        if let Some(old) = replaced {
            nuzky_oauth::revoke(&http, &self.url(REVOKE), CLIENT_ID, &old.refresh_token, NAME);
        }
        Ok(signed_in(reg))
    }

    /// Ends the session on the server and removes the tokens. The email stays, so Settings can say
    /// which account was signed in to.
    pub fn sign_out(&self) -> Result<Status> {
        let _lock = lock(&self.dir)?;
        let Some(reg) = Registration::load(&self.dir) else { return Ok(signed_out(None, None)) };
        let revoked = match self.vault.load::<Tokens>(reg.entry()) {
            Ok(Some(tokens)) => {
                nuzky_oauth::revoke(&nuzky_oauth::http(), &self.url(REVOKE), CLIENT_ID, &tokens.refresh_token, NAME)
            }
            Ok(None) => true,
            Err(error) => {
                log::warn!("Reading the Nuzky account sign-in to end it failed: {error:#}");
                false
            }
        };
        self.vault.delete(reg.entry())?;
        Ok(signed_out(reg.email, (!revoked).then_some(Notice::NotRevoked)))
    }

    /// The account's documents, without the bodies of the revisions in `known`; none when signed out.
    pub fn documents(&self, known: &[(&str, u64)]) -> Result<Option<Documents>> {
        let revisions: Vec<(&str, String)> =
            known.iter().map(|(kind, revision)| (*kind, revision.to_string())).collect();
        let query: Vec<(&str, &str)> = revisions.iter().map(|(kind, revision)| (*kind, revision.as_str())).collect();
        let url = nuzky_oauth::with_query(&self.url(SYNC), &query);
        #[derive(Deserialize)]
        struct Answer {
            documents: BTreeMap<String, Remote>,
        }
        let Some((subject, status, body)) = self.authorized(None, |http, token| {
            http.get(&url)
                .header("Authorization", format!("Bearer {token}"))
                .header("Accept", "application/json")
                .call()
        })?
        else {
            return Ok(None);
        };
        ensure!(status == 200, "SYNC_FAILED: {WHO} answered {status}");
        let answer: Answer = serde_json::from_str(&body).context("SYNC_FAILED: Reading the account's documents")?;
        Ok(Some(Documents { subject, kinds: answer.documents }))
    }

    /// Stores `body` as the next revision of `kind` over `revision`, the one this computer last had
    /// from the account `subject`; none when signed out or signed in to another account meanwhile.
    pub fn store(&self, subject: &str, kind: &str, revision: u64, body: &serde_json::Value) -> Result<Option<Stored>> {
        let url = format!("{}/{}", self.url(SYNC), nuzky_oauth::encode(kind));
        let request = serde_json::to_vec(&serde_json::json!({ "revision": revision, "body": body }))?;
        let Some((_, status, answer)) = self.authorized(Some(subject), |http, token| {
            http.put(&url)
                .header("Authorization", format!("Bearer {token}"))
                .header("Accept", "application/json")
                .header("Content-Type", "application/json")
                .send(&request[..])
        })?
        else {
            return Ok(None);
        };
        #[derive(Deserialize)]
        struct Revision {
            revision: u64,
        }
        match status {
            200 => Ok(Some(Stored::Revision(
                serde_json::from_str::<Revision>(&answer).context("SYNC_FAILED: Reading the new revision")?.revision,
            ))),
            409 => Ok(Some(Stored::Conflict(
                serde_json::from_str(&answer).context("SYNC_FAILED: Reading the account's document")?,
            ))),
            413 => bail!("TOO_LARGE: {WHO} keeps at most 256 KB of each"),
            _ => bail!("SYNC_FAILED: {WHO} answered {status}"),
        }
    }

    /// Sends a request with the access token, renewed first when it is about to expire, and once
    /// more after a renewal when the server refused it. Answers the account, the status and the
    /// body; none when signed out, the sign-in ended or it is another account than `subject`.
    fn authorized(
        &self,
        subject: Option<&str>,
        send: impl Fn(&ureq::Agent, &str) -> Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<Option<(String, u16, String)>> {
        let http = nuzky_oauth::http();
        for renew in [false, true] {
            let Some((signed_in, token)) = self.access(renew)? else { return Ok(None) };
            if subject.is_some_and(|subject| subject != signed_in) {
                return Ok(None);
            }
            let mut response =
                send(&http, &token).map_err(|error| anyhow!("SYNC_FAILED: Could not reach {WHO}: {error}"))?;
            let status = response.status().as_u16();
            if status == 401 && !renew {
                continue;
            }
            let body = response.body_mut().with_config().limit(MAX_DOCUMENTS).read_to_string()?;
            return Ok(Some((signed_in, status, body)));
        }
        bail!("SYNC_FAILED: {WHO} did not accept the sign-in")
    }

    /// The account and its access token, renewed first when it expires within five minutes, or at
    /// once with `renew`; none when signed out or the server no longer renews it.
    fn access(&self, renew: bool) -> Result<Option<(String, String)>> {
        let _lock = lock(&self.dir)?;
        let Some(mut reg) = Registration::load(&self.dir) else { return Ok(None) };
        let Some(tokens) = self.vault.load::<Tokens>(reg.entry())? else { return Ok(None) };
        let tokens = if renew || tokens.expires_at <= now() + RENEW_BEFORE {
            if !self.refresh(&mut reg, tokens)? {
                return Ok(None);
            }
            let Some(tokens) = self.vault.load::<Tokens>(reg.entry())? else { return Ok(None) };
            tokens
        } else {
            tokens
        };
        let subject = reg.subject.context("SYNC_FAILED: the sign-in names no account")?;
        Ok(Some((subject, tokens.access_token)))
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.issuer)
    }
}

/// This computer's sign-in, beside Nuzky's data. Nothing here is secret.
#[derive(Serialize, Deserialize)]
struct Registration {
    /// Keeps this data folder's keychain entry apart from another's.
    host_id: String,
    /// The account's `sub`, which stays when its email changes.
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

impl Registration {
    fn path(dir: &Path) -> PathBuf {
        dir.join("account.json")
    }

    /// None when there is none yet; an unreadable file counts as none and is replaced by the next sign-in.
    fn load(dir: &Path) -> Option<Self> {
        let text = fs::read(Self::path(dir)).ok()?;
        serde_json::from_slice(&text).inspect_err(|error| log::warn!("Reading account.json failed: {error}")).ok()
    }

    fn load_or_create(dir: &Path) -> Result<Self> {
        if let Some(reg) = Self::load(dir) {
            return Ok(reg);
        }
        let reg =
            Self { host_id: format!("urn:uuid:{}", uuid::Uuid::new_v4()), subject: None, email: None, name: None };
        reg.save(dir)?;
        Ok(reg)
    }

    fn save(&self, dir: &Path) -> Result<()> {
        nuzky_oauth::write_private(&Self::path(dir), &serde_json::to_vec_pretty(self)?)
    }

    fn entry(&self) -> Entry<'_> {
        Entry { name: NAME, host_id: &self.host_id }
    }
}

/// One token set, replaced as a whole. No Debug, so it cannot end up in a log.
#[derive(Serialize, Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    /// Unix seconds.
    expires_at: u64,
}

impl Tokens {
    /// From the code exchange. Without a refresh token Nuzky would be signed out within the hour.
    fn issued(response: TokenResponse, now: u64) -> Result<Self> {
        Ok(Self {
            refresh_token: response.refresh_token.context("no refresh token")?,
            expires_at: now + response.expires_in.unwrap_or(3600),
            access_token: response.access_token,
        })
    }

    /// After a refresh; a server that keeps the refresh token leaves it out.
    fn renewed(self, response: TokenResponse, now: u64) -> Self {
        Self {
            refresh_token: response.refresh_token.unwrap_or(self.refresh_token),
            expires_at: now + response.expires_in.unwrap_or(3600),
            access_token: response.access_token,
        }
    }
}

#[derive(Deserialize)]
struct UserInfo {
    sub: String,
    email: String,
    #[serde(default)]
    name: Option<String>,
}

/// Who the access token belongs to, as the server says over TLS.
fn userinfo(http: &ureq::Agent, url: &str, access_token: &str) -> Result<UserInfo> {
    let mut response = http
        .get(url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/json")
        .call()?;
    let status = response.status().as_u16();
    ensure!(status == 200, "the account server answered {status}");
    let body = response.body_mut().with_config().limit(nuzky_oauth::MAX_BYTES).read_to_string()?;
    let who: UserInfo = serde_json::from_str(&body).context("Reading the account")?;
    ensure!(!who.sub.is_empty() && !who.email.is_empty(), "the account server named no account");
    Ok(UserInfo { name: who.name.filter(|name| !name.trim().is_empty()), ..who })
}

/// Held while the sign-in is read or changed, so two windows never refresh the same rotating token.
fn lock(dir: &Path) -> Result<fs::File> {
    nuzky_oauth::lock(dir, "account.lock", NAME)
}

fn signed_in(reg: Registration) -> Status {
    Status::SignedIn { email: reg.email.unwrap_or_default(), name: reg.name }
}

fn signed_out(email: Option<String>, notice: Option<Notice>) -> Status {
    Status::SignedOut { email, notice }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

//! The OAuth and OpenID Connect requests of OpenAI's sign-in for open-source apps, and the checks
//! on what comes back.

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use nuzky_oauth::{Attempt, MAX_BYTES};
use ring::signature;
use serde::Deserialize;

/// The client id of a first sign-in, which registers Nuzky with the account. Never saved.
pub(crate) const DYNAMIC_CLIENT: &str = "dynamic_agent_client";
/// The name ChatGPT suggests for Nuzky's registration; the user can change it before agreeing.
const AGENT_NAME: &str = "Nuzky";
const SCOPE: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const RESOURCE: &str = "https://api.openai.com/v1";
/// Without it, the sign-in tells who the user is but does not let Nuzky use their plan.
pub(crate) const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
/// Refresh errors after which the refresh token is no use; the user has to sign in again.
pub(crate) const UNUSABLE_REFRESH: [&str; 6] = [
    "invalid_grant",
    "invalid_refresh_token",
    "token_expired",
    "refresh_token_expired",
    "refresh_token_invalidated",
    "refresh_token_reused",
];
/// Allowed difference between this computer's clock and OpenAI's when checking expiry.
const CLOCK_SKEW: u64 = 60;

pub(crate) use nuzky_oauth::{CallbackError, TokenError, TokenResponse};

pub(crate) struct Authorize<'a> {
    /// The client issued to Nuzky for this account; None registers Nuzky with a new one.
    pub client_id: Option<&'a str>,
    pub host_id: &'a str,
    /// The account's last ID token, so ChatGPT skips choosing the account.
    pub id_token_hint: Option<&'a str>,
    pub login_hint: Option<&'a str>,
    /// Asks the user again to let Nuzky use their plan.
    pub consent: bool,
}

/// The authorization page. It can hold the last ID token, so it is never logged.
pub(crate) fn authorize_url(issuer: &str, request: &Authorize, attempt: &Attempt) -> String {
    let challenge = attempt.challenge();
    let mut query = vec![("client_id", request.client_id.unwrap_or(DYNAMIC_CLIENT))];
    if request.client_id.is_none() {
        query.push(("agent_name_hint", AGENT_NAME));
    }
    query.push(("ext_agent_host_id", request.host_id));
    query.extend(request.id_token_hint.map(|hint| ("id_token_hint", hint)));
    query.extend(request.login_hint.map(|hint| ("login_hint", hint)));
    query.extend([
        ("response_type", "code"),
        ("redirect_uri", &attempt.redirect_uri),
        ("scope", SCOPE),
        ("resource", RESOURCE),
        ("state", &attempt.state),
        ("nonce", &attempt.nonce),
        ("code_challenge_method", "S256"),
        ("code_challenge", &challenge),
    ]);
    if request.consent {
        query.push(("prompt", "consent"));
    }
    nuzky_oauth::with_query(&format!("{issuer}/api/accounts/authorize"), &query)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Callback {
    Code { code: String, client_id: String },
    Denied,
}

/// The browser's return to /auth/callback. `saved_client` is the client this attempt signed in
/// with; without one, ChatGPT names the client it issued.
pub(crate) fn parse_callback(query: &str, state: &str, saved_client: Option<&str>) -> Result<Callback, CallbackError> {
    let code = match nuzky_oauth::parse_callback(query, state, "ChatGPT")? {
        nuzky_oauth::Callback::Denied => return Ok(Callback::Denied),
        nuzky_oauth::Callback::Code(code) => code,
    };
    let issued = nuzky_oauth::param(query, "client_id").filter(|id| !id.is_empty());
    let client_id = match saved_client {
        Some(saved) if issued.as_deref().is_none_or(|issued| issued == saved) => saved.to_owned(),
        Some(_) => return Err(CallbackError::Failed("ChatGPT answered for another registration of Nuzky".into())),
        None => issued
            .filter(|issued| issued != DYNAMIC_CLIENT)
            .ok_or_else(|| CallbackError::Failed("ChatGPT did not finish registering Nuzky".into()))?,
    };
    Ok(Callback::Code { code, client_id })
}

pub(crate) fn exchange(
    http: &ureq::Agent,
    issuer: &str,
    client_id: &str,
    code: &str,
    attempt: &Attempt,
) -> Result<TokenResponse, TokenError> {
    nuzky_oauth::token_request(
        http,
        &format!("{issuer}/api/accounts/oauth/token"),
        &[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("code_verifier", &attempt.verifier),
            ("redirect_uri", &attempt.redirect_uri),
            ("resource", RESOURCE),
        ],
    )
}

/// Without `scope`, so the grant stays as it was.
pub(crate) fn refresh(
    http: &ureq::Agent,
    issuer: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenResponse, TokenError> {
    nuzky_oauth::token_request(
        http,
        &format!("{issuer}/api/accounts/oauth/token"),
        &[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("resource", RESOURCE),
        ],
    )
}

/// Ends the renewable session. True once OpenAI confirmed it.
pub(crate) fn revoke(http: &ureq::Agent, issuer: &str, client_id: &str, refresh_token: &str) -> bool {
    nuzky_oauth::revoke(http, &format!("{issuer}/api/accounts/oauth/revoke"), client_id, refresh_token, "ChatGPT")
}

#[derive(Deserialize)]
pub(crate) struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
struct Jwk {
    kty: String,
    kid: Option<String>,
    alg: Option<String>,
    #[serde(rename = "use")]
    usage: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

/// OpenAI's signing keys, read fresh for every sign-in, so a new key is never missing.
pub(crate) fn jwks(http: &ureq::Agent, issuer: &str) -> Result<Jwks> {
    let mut response = http.get(&format!("{issuer}/.well-known/jwks.json")).call()?;
    ensure!(response.status() == 200, "the keys answered {}", response.status());
    let body = response.body_mut().with_config().limit(MAX_BYTES).read_to_string()?;
    serde_json::from_str(&body).context("Reading the keys")
}

#[derive(Deserialize)]
struct Header {
    alg: String,
    kid: Option<String>,
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: Audience,
    azp: Option<String>,
    exp: u64,
    nonce: Option<String>,
    sub: String,
    email: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub subject: String,
    pub email: Option<String>,
}

/// Who signed in, once the ID token's RS256 signature matches one of OpenAI's keys and it was
/// issued by `issuer` for `client_id`, has not expired and carries this attempt's nonce.
pub(crate) fn verify_id_token(
    token: &str,
    jwks: &Jwks,
    issuer: &str,
    client_id: &str,
    nonce: &str,
    now: u64,
) -> Result<Identity> {
    let [header, payload, sig] = token.split('.').collect::<Vec<_>>()[..] else { bail!("The ID token is not a JWT") };
    let header: Header = serde_json::from_slice(&decode(header)?).context("Reading the ID token's header")?;
    ensure!(header.alg == "RS256", "The ID token is signed with {}, not RS256", header.alg);
    let signed = &token[..token.len() - sig.len() - 1];
    let sig = decode(sig)?;
    let verified = jwks
        .keys
        .iter()
        .filter(|key| key.kty == "RSA" && key.alg.as_deref().is_none_or(|alg| alg == "RS256"))
        .filter(|key| key.usage.as_deref().is_none_or(|usage| usage == "sig"))
        .filter(|key| header.kid.is_none() || key.kid == header.kid)
        .any(|key| {
            let (Some(Ok(n)), Some(Ok(e))) = (key.n.as_deref().map(decode), key.e.as_deref().map(decode)) else {
                return false;
            };
            signature::RsaPublicKeyComponents { n, e }
                .verify(&signature::RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &sig)
                .is_ok()
        });
    ensure!(verified, "The ID token's signature matches none of OpenAI's keys");
    let claims: Claims = serde_json::from_slice(&decode(payload)?).context("Reading the ID token")?;
    ensure!(claims.iss == issuer, "The ID token comes from {}, not {issuer}", claims.iss);
    let audience = match &claims.aud {
        Audience::One(aud) => aud == client_id,
        Audience::Many(auds) => auds.iter().any(|aud| aud == client_id),
    };
    ensure!(audience && claims.azp.as_deref().is_none_or(|azp| azp == client_id), "The ID token is for another app");
    ensure!(claims.exp + CLOCK_SKEW > now, "The ID token has expired");
    ensure!(claims.nonce.as_deref() == Some(nonce), "The ID token is from another sign-in");
    ensure!(!claims.sub.is_empty(), "The ID token names no account");
    Ok(Identity { subject: claims.sub, email: claims.email })
}

fn decode(part: &str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(part).context("The ID token is not base64url")
}

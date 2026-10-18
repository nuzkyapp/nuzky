//! Whether a newer Nuzky was released. Only the version is read from `latest.json` on the newest
//! published GitHub release; nothing is downloaded or run. Download opens the release page.
use std::io::Read;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use semver::Version;
use serde::Deserialize;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::{CmdResult, err};

/// GitHub serves this from the newest release that is neither a draft nor a prerelease, so
/// publishing a draft is what tells installed copies about it.
const LATEST_JSON: &str = "https://github.com/nuzkyapp/nuzky/releases/latest/download/latest.json";
/// Download opens this fixed page, never an address from the file.
const RELEASE_PAGE: &str = "https://github.com/nuzkyapp/nuzky/releases/latest";
const X_PROFILE: &str = "https://x.com/mach_builds";
/// Tests serve the file locally through `NUZKY_UPDATE_URL`; no other address replaces GitHub.
const TEST_SERVER: &str = "http://127.0.0.1:";
const TIMEOUT: Duration = Duration::from_secs(10);
/// The file holds a version and a date; anything this big is not it.
const MAX_BYTES: u64 = 64 * 1024;

/// `NUZKY_NO_UPDATE_CHECK=1` keeps update checks off the network, for tests and packagers.
pub fn enabled() -> bool {
    std::env::var("NUZKY_NO_UPDATE_CHECK").as_deref() != Ok("1")
}

#[derive(Deserialize)]
struct Latest {
    version: String,
}

/// The released version when it is newer than `current`. Other fields are ignored, so a later
/// release can add what an in-app installer needs without silencing this version.
fn newer(json: &str, current: &Version) -> Result<Option<Version>> {
    let latest: Latest = serde_json::from_str(json).context("Reading latest.json")?;
    let text = latest.version.strip_prefix('v').unwrap_or(&latest.version);
    ensure!(text.len() <= 32, "Released version is too long");
    let version = Version::parse(text).context("Reading the released version")?;
    Ok(version.cmp_precedence(current).is_gt().then_some(version))
}

fn fetch() -> Result<String> {
    let test = std::env::var("NUZKY_UPDATE_URL").ok().filter(|url| url.starts_with(TEST_SERVER));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(test.is_none())
        .max_redirects(5)
        .timeout_global(Some(TIMEOUT))
        .user_agent(concat!("Nuzky/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut response = agent.get(test.as_deref().unwrap_or(LATEST_JSON)).call().context("Requesting latest.json")?;
    // The limit counts bytes on the wire; a gzip body unpacks to far more, so the text is capped too.
    let mut text = String::new();
    response.body_mut().with_config().limit(MAX_BYTES).reader().take(MAX_BYTES + 1).read_to_string(&mut text)?;
    ensure!(text.len() as u64 <= MAX_BYTES, "latest.json is too big");
    Ok(text)
}

/// The newer version, or None when this one is the latest.
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> CmdResult<Option<String>> {
    if !enabled() {
        return Err("UPDATES_OFF: update checks are turned off".into());
    }
    let current = app.package_info().version.clone();
    let found = tauri::async_runtime::spawn_blocking(move || newer(&fetch()?, &current)).await.map_err(err)?;
    found.map(|version| version.map(|v| v.to_string())).map_err(|error| {
        log::warn!("Update check failed: {error:#}");
        format!("{error:#}")
    })
}

#[tauri::command]
pub fn open_release_page(app: AppHandle) -> CmdResult<()> {
    app.opener().open_url(RELEASE_PAGE, None::<&str>).map_err(err)
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackKind {
    Bug,
    Idea,
    Message,
}

fn feedback_url(kind: FeedbackKind, version: &str, system: &str) -> String {
    // The forms in .github/ISSUE_TEMPLATE take these fields from the address by their ids.
    let fields: &[(&str, &str)] = match kind {
        FeedbackKind::Bug => &[("template", "bug_report.yml"), ("version", version), ("system", system)],
        FeedbackKind::Idea => &[("template", "feature_request.yml"), ("version", version)],
        FeedbackKind::Message => return X_PROFILE.into(),
    };
    let mut url = tauri::Url::parse("https://github.com/nuzkyapp/nuzky/issues/new").unwrap();
    url.query_pairs_mut().extend_pairs(fields);
    url.into()
}

/// The system as a person reading a bug report wants it, with its version where the system says it.
pub(crate) fn system() -> String {
    let name = match std::env::consts::OS {
        "linux" => {
            let pretty = std::fs::read_to_string("/etc/os-release").ok().and_then(|release| {
                release.lines().find_map(|line| {
                    line.strip_prefix("PRETTY_NAME=").map(|name| name.trim().trim_matches(['\"', '\'']).to_owned())
                })
            });
            pretty.map_or_else(|| "Linux".into(), |name| format!("Linux ({name})"))
        }
        "macos" => std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
            .ok()
            .and_then(|plist| {
                let after = plist.split("<key>ProductVersion</key>").nth(1)?;
                let version = after.split("<string>").nth(1)?.split("</string>").next()?;
                Some(format!("macOS {}", version.trim()))
            })
            .unwrap_or_else(|| "macOS".into()),
        "windows" => "Windows".into(),
        os => os.into(),
    };
    format!("{name} · {}", std::env::consts::ARCH)
}

/// Opens a web page or a folder. In debug builds with `NUZKY_OPENED_URLS` set it only writes down what it would open,
/// so the UI flows open nothing.
pub(crate) fn open(app: &AppHandle, target: &str) -> CmdResult<()> {
    if cfg!(debug_assertions)
        && let Some(path) = std::env::var_os("NUZKY_OPENED_URLS")
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(err)?;
        return writeln!(file, "{target}").map_err(err);
    }
    if target.starts_with("https://") {
        app.opener().open_url(target, None::<&str>).map_err(err)
    } else {
        app.opener().open_path(target, None::<&str>).map_err(err)
    }
}

/// Opens a prefilled issue form or the X profile in the browser; the person reads and submits the issue there.
#[tauri::command]
pub fn open_feedback(app: AppHandle, kind: FeedbackKind) -> CmdResult<()> {
    open(&app, &feedback_url(kind, &app.package_info().version.to_string(), &system()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_links_open_the_issue_forms_with_version_and_system() {
        let fields = |kind| {
            let url = tauri::Url::parse(&feedback_url(kind, "0.2.0", "Linux (Fedora Linux 44) · x86_64")).unwrap();
            assert_eq!(url.origin().ascii_serialization(), "https://github.com");
            assert_eq!(url.path(), "/nuzkyapp/nuzky/issues/new");
            url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect::<Vec<_>>()
        };
        let pair = |k: &str, v: &str| (k.to_owned(), v.to_owned());
        assert_eq!(
            fields(FeedbackKind::Bug),
            [
                pair("template", "bug_report.yml"),
                pair("version", "0.2.0"),
                pair("system", "Linux (Fedora Linux 44) · x86_64")
            ]
        );
        assert_eq!(fields(FeedbackKind::Idea), [pair("template", "feature_request.yml"), pair("version", "0.2.0")]);
        assert_eq!(feedback_url(FeedbackKind::Message, "0.2.0", "macOS 15.3 · aarch64"), X_PROFILE);
        for (form, ids) in [
            (include_str!("../../.github/ISSUE_TEMPLATE/bug_report.yml"), ["version", "system"].as_slice()),
            (include_str!("../../.github/ISSUE_TEMPLATE/feature_request.yml"), ["version"].as_slice()),
        ] {
            for id in ids {
                assert!(form.contains(&format!("id: {id}\n")), "the form has no field {id}");
            }
        }
    }

    fn check(json: &str, current: &str) -> Option<String> {
        newer(json, &Version::parse(current).unwrap()).unwrap().map(|v| v.to_string())
    }

    #[test]
    fn offers_only_a_newer_version() {
        assert_eq!(check(r#"{"version":"0.2.0"}"#, "0.1.0").as_deref(), Some("0.2.0"));
        assert_eq!(check(r#"{"version":"0.1.10"}"#, "0.1.9").as_deref(), Some("0.1.10"));
        assert_eq!(check(r#"{"version":"0.1.0"}"#, "0.1.0"), None);
        assert_eq!(check(r#"{"version":"0.1.0"}"#, "0.2.0"), None);
        // A prerelease comes before its release; build metadata says nothing about which is newer.
        assert_eq!(check(r#"{"version":"0.2.0-beta.1"}"#, "0.2.0"), None);
        assert_eq!(check(r#"{"version":"0.2.0"}"#, "0.2.0-beta.1").as_deref(), Some("0.2.0"));
        assert_eq!(check(r#"{"version":"0.1.0+ci.7"}"#, "0.1.0"), None);
    }

    #[test]
    fn reads_tags_and_ignores_what_later_releases_add() {
        assert_eq!(check(r#"{"version":"v0.2.0"}"#, "0.1.0").as_deref(), Some("0.2.0"));
        let installer = r#"{"version":"0.3.0","notes":"x","pub_date":"2027-01-01T00:00:00Z",
            "platforms":{"linux-x86_64":{"url":"https://example.com/a","signature":"s"}}}"#;
        assert_eq!(check(installer, "0.1.0").as_deref(), Some("0.3.0"));
    }

    #[test]
    fn refuses_what_is_not_a_version() {
        let current = Version::new(0, 1, 0);
        for json in
            [r#"{"version":"latest"}"#, r#"{"version":"1.2"}"#, r#"{"tag":"0.2.0"}"#, "<html>Not Found</html>", ""]
        {
            assert!(newer(json, &current).is_err(), "{json}");
        }
        let long = format!(r#"{{"version":"1.0.0-{}"}}"#, "a".repeat(40));
        assert!(newer(&long, &current).is_err());
    }

    /// The release tag must be `v` plus tauri.conf.json's version (release.yml), which latest.json then announces
    /// to installed copies; the crates report Cargo.toml's and the website package.json's.
    #[test]
    fn every_package_has_the_same_version() {
        let parse = |text: &str| serde_json::from_str::<serde_json::Value>(text).unwrap();
        let tauri = parse(include_str!("../tauri.conf.json"));
        let npm = parse(include_str!("../../package.json"));
        let lock = parse(include_str!("../../package-lock.json"));
        let cargo = env!("CARGO_PKG_VERSION");
        for (file, version) in [
            ("src-tauri/tauri.conf.json", &tauri["version"]),
            ("package.json", &npm["version"]),
            ("package-lock.json", &lock["version"]),
            ("package-lock.json", &lock["packages"][""]["version"]),
        ] {
            assert_eq!(version.as_str(), Some(cargo), "{file} has version {version}, Cargo.toml has {cargo}");
        }
    }
}

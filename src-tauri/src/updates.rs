//! Whether a newer Nuzky was released, and installing it when the user asks. The check reads only the version from
//! `latest.json` on the newest published GitHub release. Install and restart downloads the package that file names for
//! this kind of install, installs it only when its signature matches the key built into Nuzky (tauri.conf.json), and
//! restarts. Nothing is downloaded or installed without the user's click.
use std::io::Read;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use semver::Version;
use serde::Deserialize;
use tauri::{AppHandle, Manager, State, ipc::Channel};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::{Error as UpdateError, Updater, UpdaterExt};

use crate::{AppState, CmdResult, err};

/// GitHub serves this from the newest release that is neither a draft nor a prerelease, so
/// publishing a draft is what tells installed copies about it. The installer reads the same file (tauri.conf.json).
const LATEST_JSON: &str = "https://github.com/nuzkyapp/nuzky/releases/latest/download/latest.json";
/// What's new and Download from GitHub open this fixed page, never an address from the file.
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

fn test_server() -> Option<String> {
    std::env::var("NUZKY_UPDATE_URL").ok().filter(|url| url.starts_with(TEST_SERVER))
}

fn fetch() -> Result<String> {
    let test = test_server();
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

/// Stops the update being downloaded.
#[derive(Default)]
pub struct Download(Mutex<Option<Box<dyn Fn() + Send>>>);

/// A build from source, a debug build, or a release without a package for this kind of install.
const NO_PACKAGE: &str =
    "UPDATE_NO_PACKAGE: Nuzky can't update this copy by itself. Download the new version from GitHub.";

/// Downloads the newest release for this kind of install, checks its signature, saves the project, installs it and
/// restarts Nuzky. `progress` gets the percentage downloaded. On Windows the installer ends Nuzky and starts the new one.
#[tauri::command]
pub async fn install_update(app: AppHandle, download: State<'_, Download>, progress: Channel<u32>) -> CmdResult<()> {
    if !enabled() {
        return Err("UPDATES_OFF: update checks are turned off".into());
    }
    if download.0.lock().unwrap().is_some() {
        return Err("UPDATE_BUSY: The update is already downloading.".into());
    }
    refuse_while_busy(&app)?;
    let packaged = packaged();
    // The UI flows serve a debug build a signed package; it downloads and checks it, but never replaces itself.
    let flow = cfg!(debug_assertions) && test_server().is_some();
    if !(packaged || flow) {
        return Err(NO_PACKAGE.into());
    }
    let updater = updater(&app).map_err(failure)?;
    let task = tauri::async_runtime::spawn(async move {
        let Some(update) = updater.check().await? else { return Ok(None) };
        let (mut done, mut shown) = (0, 0);
        let on_chunk = |chunk: usize, total: Option<u64>| {
            done += chunk as u64;
            if let Some(total) = total.filter(|&total| total > 0) {
                let percent = (done * 100 / total).min(100) as u32;
                if percent > shown {
                    shown = percent;
                    progress.send(percent).ok();
                }
            }
        };
        let bytes = update
            .download(on_chunk, || {
                progress.send(100).ok();
            })
            .await?;
        Ok(Some((update, bytes)))
    });
    let stop = task.inner().abort_handle();
    *download.0.lock().unwrap() = Some(Box::new(move || stop.abort()));
    let downloaded = task.await;
    download.0.lock().unwrap().take();
    let Some((update, bytes)) = downloaded.map_err(|_| "CANCELLED: the update was stopped")?.map_err(failure)? else {
        return Err(format!("UPDATE_LATEST: Nuzky {} is the latest version.", app.package_info().version));
    };
    if !packaged {
        return Err(NO_PACKAGE.into());
    }
    // Work may have started while it downloaded.
    refuse_while_busy(&app)?;
    save(&app)?;
    tauri::async_runtime::spawn_blocking(move || update.install(bytes)).await.map_err(err)?.map_err(failure)?;
    log::info!(target: "action", "update installed, restarting");
    // Quitting stops what runs and saves again, as on any quit.
    app.request_restart();
    Ok(())
}

#[tauri::command]
pub fn cancel_update(download: State<'_, Download>) {
    if let Some(stop) = download.0.lock().unwrap().as_ref() {
        stop();
    }
}

/// The updater tauri.conf.json configures. In debug builds the UI flows give the address of their own release on this
/// computer (`NUZKY_UPDATE_URL`) and the public key of their test packages (`NUZKY_UPDATE_PUBKEY`).
fn updater(app: &AppHandle) -> tauri_plugin_updater::Result<Updater> {
    let handle = app.clone();
    // Only Windows runs this: its installer starts while Nuzky ends with `std::process::exit`, which skips quitting.
    let builder = app.updater_builder().on_before_exit(move || {
        crate::quit(&handle);
        handle.cleanup_before_exit();
    });
    #[cfg(debug_assertions)]
    let builder = {
        let mut builder = builder;
        if let Some(url) = test_server() {
            builder = builder.endpoints(vec![url.parse()?])?;
        }
        if let Ok(key) = std::env::var("NUZKY_UPDATE_PUBKEY") {
            builder = builder.pubkey(key);
        }
        builder
    };
    builder.build()
}

/// Whether this copy came from a release package that the updater can replace. A build from source has none: on macOS
/// the updater would replace the folder holding the binary, on Windows install a second copy.
fn packaged() -> bool {
    use tauri::utils::{config::BundleType, platform};
    let exe = platform::current_exe().unwrap_or_default();
    !cfg!(debug_assertions)
        && match platform::bundle_type() {
            // Every macOS build says App, so the binary must be inside one.
            Some(BundleType::App) => {
                exe.parent().is_some_and(|dir| dir.ends_with("Contents/MacOS"))
                    && exe.ancestors().nth(3).is_some_and(|app| app.extension().is_some_and(|ext| ext == "app"))
            }
            // The AppImage file is what gets replaced and started again, not the binary unpacked from it.
            Some(BundleType::AppImage) => std::env::var_os("APPIMAGE").is_some(),
            bundle => bundle.is_some(),
        }
}

/// Restarting cancels running work, as quitting does, and would cut an AI edit short.
fn refuse_while_busy(app: &AppHandle) -> CmdResult<()> {
    let state = app.state::<AppState>();
    let host = state.session.lock().unwrap().host.clone();
    if host.session.state().is_ok_and(|s| s.open_run.is_some())
        || app.state::<crate::agent_panel::AgentPanel>().working()
    {
        return Err("UPDATE_BUSY: Updating restarts Nuzky. Wait for the AI to finish.".into());
    }
    if state.running_work().is_some() {
        return Err("UPDATE_BUSY: Updating restarts Nuzky. Wait for the export or analysis to finish.".into());
    }
    Ok(())
}

fn save(app: &AppHandle) -> CmdResult<()> {
    let host = app.state::<AppState>().session.lock().unwrap().host.clone();
    host.session
        .disconnect()
        .map_err(|error| format!("UPDATE_SAVE: Nuzky didn't update, as it couldn't save your project: {error:#}"))
}

/// What happened, as the UI shows it. The code says whether trying again can help (`UPDATE_NETWORK`, `UPDATE_PASSWORD`,
/// `UPDATE_FAILED`) or only the installer from GitHub (`UPDATE_NO_PACKAGE`, `UPDATE_SIGNATURE`, `UPDATE_FOLDER`).
fn failure(error: UpdateError) -> String {
    use UpdateError::*;
    log::warn!("Update failed: {error}");
    match error {
        Minisign(_) | Base64(_) | SignatureUtf8(_) | SignedVersionMismatch { .. } | MissingSignedVersion => {
            "UPDATE_SIGNATURE: The download isn't signed by Nuzky, so it wasn't installed. Download the new version \
             from GitHub instead."
                .into()
        }
        TargetNotFound(_) | TargetsNotFound(_) | InvalidUpdaterFormat | BinaryNotFoundInArchive => NO_PACKAGE.into(),
        Reqwest(_) | Network(_) | ReleaseNotFound => {
            "UPDATE_NETWORK: Couldn't download the update. Check your internet connection and try again.".into()
        }
        AuthenticationFailed | PackageInstallFailed | DebInstallFailed => {
            "UPDATE_PASSWORD: The update wasn't installed. It needs your password; try again and enter it.".into()
        }
        // An AppImage in a folder the user can't change, or a macOS app the user can't replace.
        Io(ref io) if io.kind() == std::io::ErrorKind::PermissionDenied && cfg!(unix) => FOLDER.into(),
        TempDirNotOnSameMountPoint => FOLDER.into(),
        PreviousAppNotRestored(path) => format!(
            "UPDATE_FOLDER: The update failed, and the previous Nuzky is now at {}. Download the new version from \
             GitHub.",
            path.display()
        ),
        other => format!("UPDATE_FAILED: Couldn't install the update ({other}). Try again."),
    }
}

const FOLDER: &str = if cfg!(target_os = "macos") {
    "UPDATE_FOLDER: Nuzky can't replace itself where it is. Move it to Applications, or download the new version from \
     GitHub."
} else {
    "UPDATE_FOLDER: Nuzky can't replace itself in its folder. Move the AppImage to a folder of yours, such as your home \
     folder, or download the new version from GitHub."
};

#[tauri::command]
pub fn open_release_page(app: AppHandle) -> CmdResult<()> {
    open(&app, RELEASE_PAGE)
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackKind {
    Idea,
    Message,
}

const NEW_ISSUE: &str = "https://github.com/nuzkyapp/nuzky/issues/new";
/// GitHub refuses addresses much longer than this.
const MAX_URL: usize = 7000;

fn feedback_url(kind: FeedbackKind, version: &str) -> String {
    match kind {
        // The forms in .github/ISSUE_TEMPLATE take these fields from the address by their ids.
        FeedbackKind::Idea => form(&[("template", "feature_request.yml"), ("version", version)]),
        FeedbackKind::Message => X_PROFILE.into(),
    }
}

fn form(fields: &[(&str, &str)]) -> String {
    let mut url = tauri::Url::parse(NEW_ISSUE).unwrap();
    url.query_pairs_mut().extend_pairs(fields);
    url.into()
}

/// The bug report form with the description and a summary of the report filled in. A long description is cut and the
/// summary loses lines from its end until the address fits; the report the person attaches holds them whole.
pub(crate) fn bug_url(version: &str, description: &str, summary: &[String]) -> String {
    // Counted in bytes: each one takes up to three characters of the address.
    let whole = description.trim();
    let mut bytes = 0;
    let mut description: String = whole
        .chars()
        .take_while(|c| {
            bytes += c.len_utf8();
            bytes <= 1200
        })
        .collect();
    if description.len() < whole.len() {
        description.push('…');
    }
    let system = system();
    let fields = |lines: &[String]| {
        form(&[
            ("template", "bug_report.yml"),
            ("version", version),
            ("system", &system),
            ("what-happened", &description),
            ("diagnostics", &lines.join("\n")),
        ])
    };
    (0..=summary.len())
        .rev()
        .map(|n| fields(&summary[..n]))
        .find(|url| url.len() <= MAX_URL)
        .unwrap_or_else(|| fields(&[]))
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
    open(&app, &feedback_url(kind, &app.package_info().version.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(url: &str) -> Vec<(String, String)> {
        let url = tauri::Url::parse(url).unwrap();
        assert_eq!(url.origin().ascii_serialization(), "https://github.com");
        assert_eq!(url.path(), "/nuzkyapp/nuzky/issues/new");
        url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
    }

    #[test]
    fn feedback_links_open_the_issue_forms_with_what_nuzky_knows() {
        let pair = |k: &str, v: &str| (k.to_owned(), v.to_owned());
        assert_eq!(
            fields(&feedback_url(FeedbackKind::Idea, "0.2.0")),
            [pair("template", "feature_request.yml"), pair("version", "0.2.0")]
        );
        assert_eq!(feedback_url(FeedbackKind::Message, "0.2.0"), X_PROFILE);
        let summary = ["Nuzky 0.2.0 (abc1234)".to_owned(), "Last actions:".to_owned()];
        assert_eq!(
            fields(&bug_url("0.2.0 (abc1234)", "  The sound is late.\n", &summary)),
            [
                pair("template", "bug_report.yml"),
                pair("version", "0.2.0 (abc1234)"),
                pair("system", &system()),
                pair("what-happened", "The sound is late."),
                pair("diagnostics", "Nuzky 0.2.0 (abc1234)\nLast actions:"),
            ]
        );
        for (form, ids) in [
            (
                include_str!("../../.github/ISSUE_TEMPLATE/bug_report.yml"),
                ["version", "system", "what-happened", "diagnostics"].as_slice(),
            ),
            (include_str!("../../.github/ISSUE_TEMPLATE/feature_request.yml"), ["version"].as_slice()),
        ] {
            for id in ids {
                assert!(form.lines().any(|line| line.trim() == format!("id: {id}")), "the form has no field {id}");
            }
        }
    }

    /// GitHub answers a too long address with an error page, so a long summary is cut to fit and keeps its start.
    #[test]
    fn a_long_summary_is_cut_to_fit_the_address() {
        let summary: Vec<String> =
            (0..400).map(|n| format!("  2026-10-19T14:02:{n:02}Z user splitClip atUs={n}")).collect();
        let url = bug_url("0.2.0", &"ž".repeat(3000), &summary);
        assert!(bug_url("0.2.0", &"🎬".repeat(600), &[]).len() <= MAX_URL);
        assert!(url.len() <= MAX_URL, "{}", url.len());
        let diagnostics = fields(&url).into_iter().find(|(k, _)| k == "diagnostics").unwrap().1;
        assert!(diagnostics.starts_with(&summary[0]) && diagnostics.lines().count() > 20, "{diagnostics}");
    }

    /// The key in tauri.conf.json is the one the release job signs with (`TAURI_SIGNING_PRIVATE_KEY`), so installed
    /// copies accept its packages. The signature was made with that key over the text below.
    #[test]
    fn the_built_in_key_accepts_what_the_release_key_signed() {
        use base64::Engine as _;
        const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUZHgvakd3NWk0VVlqVEhNZ003WVg3QUs3VzJiWjNGbFRqSmRmTktPVWE4TmNHa3U4MDFXUklpbDR6aXBGR2VXYis2VUpvbTlnSUNrdURrei9GYnc5bWNFOEs4NnlKNFFRPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyNDk5MDQ3CWZpbGU6cHJvb2YudHh0CXZlcnNpb246MC4xLjAtYWxwaGEKUVh0RDczVDE0eXZuV3hYT2JrOWF4bHExNENyOHVvajdGQzdFdDhMYWdjMWNPbHl1b3lZalNEWVQzKzJ2cmtUVnNtRnF0dmRiM2JBSjJXWlcwbXhlRGc9PQo=";
        let decode =
            |text: &str| String::from_utf8(base64::engine::general_purpose::STANDARD.decode(text).unwrap()).unwrap();
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let updater = &config["plugins"]["updater"];
        assert_eq!(updater["endpoints"], serde_json::json!([LATEST_JSON]));
        let key = minisign_verify::PublicKey::decode(&decode(updater["pubkey"].as_str().unwrap())).unwrap();
        let signature = minisign_verify::Signature::decode(&decode(SIGNATURE)).unwrap();
        key.verify(b"Nuzky update packages are signed with this key.\n", &signature, false).unwrap();
        assert!(key.verify(b"Nuzky update packages are signed with another key.\n", &signature, false).is_err());
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

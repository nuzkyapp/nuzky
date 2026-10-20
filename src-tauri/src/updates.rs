//! Whether a newer Nuzky was released, and installing it when the user asks. The check reads only the version from
//! `latest.json` on the newest published GitHub release. Install and restart downloads the package that file names for
//! this kind of install, installs it only when its signature matches the key built into Nuzky (tauri.conf.json), and
//! restarts. Nothing is downloaded or installed without the user's click.
use std::io::Read;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, ipc::Channel};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::{Error as UpdateError, Update, Updater, UpdaterExt};

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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    version: String,
    /// This copy can install it itself; otherwise only the installers on GitHub can.
    can_install: bool,
}

/// The newer version, or None when this one is the latest.
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> CmdResult<Option<Found>> {
    if !enabled() {
        return Err("UPDATES_OFF: update checks are turned off".into());
    }
    let current = app.package_info().version.clone();
    let found = tauri::async_runtime::spawn_blocking(move || newer(&fetch()?, &current)).await.map_err(err)?;
    let found = found.map_err(|error| {
        log::warn!("Update check failed: {error:#}");
        format!("{error:#}")
    })?;
    Ok(found.map(|version| Found { version: version.to_string(), can_install: can_install() }))
}

/// Where installing an update is. The downloaded package waits here for the restart, so it is never downloaded twice.
#[derive(Default)]
pub struct Updates(Mutex<Stage>);

#[derive(Default)]
enum Stage {
    #[default]
    None,
    /// Stops the download.
    Downloading(Box<dyn Fn() + Send>),
    /// Downloaded, its signature checked.
    Ready(Box<(Update, Vec<u8>)>),
    Installing,
    /// The new version is in place; it runs from the next start.
    Installed,
    /// Quitting, to start the updated AppImage once it has.
    Restarting,
}

/// A build from source, a debug build, or a release without a package for this kind of install.
const NO_PACKAGE: &str =
    "UPDATE_NO_PACKAGE: Nuzky can't update this copy by itself. Download the new version from GitHub.";
/// Errors that trying again cannot fix; the installers on GitHub can.
const FINAL: [&str; 4] = ["UPDATE_NO_PACKAGE", "UPDATE_SIGNATURE", "UPDATE_SOURCE", "UPDATE_FOLDER"];
/// latest.json names the package of each kind of install; only the release's own downloads are taken.
const PACKAGES: &str = "https://github.com/nuzkyapp/nuzky/releases/download/";

/// Downloads the newest release for this kind of install and checks its signature against the key built into Nuzky.
/// The package then waits for `restart_to_update`. `progress` gets the percentage downloaded. Returns its version, at
/// once when it is already downloaded.
#[tauri::command]
pub async fn download_update(app: AppHandle, updates: State<'_, Updates>, progress: Channel<u32>) -> CmdResult<String> {
    if !enabled() {
        return Err("UPDATES_OFF: update checks are turned off".into());
    }
    let task = {
        let mut stage = updates.0.lock().unwrap();
        match &*stage {
            Stage::None => {}
            Stage::Ready(ready) => return Ok(ready.0.version.clone()),
            _ => return Err("UPDATE_BUSY: The update is already being downloaded or installed.".into()),
        }
        if !can_install() {
            return Err(NO_PACKAGE.into());
        }
        let updater = updater(&app).map_err(failure)?;
        let task = tauri::async_runtime::spawn(async move {
            let Some(update) = updater.check().await.map_err(failure)? else { return Ok(None) };
            if !trusted(update.download_url.as_str()) {
                log::warn!("Update refused: its package is at {}", update.download_url);
                return Err(String::from(
                    "UPDATE_SOURCE: The update isn't on Nuzky's GitHub releases, so it wasn't downloaded. Download the \
                     new version from GitHub instead.",
                ));
            }
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
            let bytes = update.download(on_chunk, || {}).await.map_err(failure)?;
            Ok(Some((update, bytes)))
        });
        let stop = task.inner().abort_handle();
        *stage = Stage::Downloading(Box::new(move || stop.abort()));
        task
    };
    let downloaded = task.await;
    let mut stage = updates.0.lock().unwrap();
    *stage = Stage::None;
    match downloaded.map_err(|_| "CANCELLED: the update was stopped")?? {
        Some((update, bytes)) => {
            let version = update.version.clone();
            *stage = Stage::Ready(Box::new((update, bytes)));
            Ok(version)
        }
        None => Err(format!("UPDATE_LATEST: Nuzky {} is the latest version.", app.package_info().version)),
    }
}

#[tauri::command]
pub fn cancel_update(updates: State<'_, Updates>) {
    if let Stage::Downloading(stop) = &*updates.0.lock().unwrap() {
        stop();
    }
}

/// Saves the project, installs the downloaded update and restarts into it; refused while work runs that restarting
/// would cancel. `note` is for the restarted UI (`take_note`). Returns the work that started while it installed: Nuzky
/// then goes on, and the new version starts with the next restart.
#[tauri::command]
pub async fn restart_to_update(app: AppHandle, updates: State<'_, Updates>, note: String) -> CmdResult<Option<String>> {
    let ready = {
        let mut stage = updates.0.lock().unwrap();
        if !matches!(*stage, Stage::Ready(_) | Stage::Installed) {
            return Err("UPDATE_BUSY: The update isn't downloaded yet.".into());
        }
        if let Some(work) = busy(&app) {
            return Err(format!("UPDATE_BUSY: {}", wait_for(work)));
        }
        match std::mem::replace(&mut *stage, Stage::Installing) {
            Stage::Ready(ready) => Some(ready),
            _ => {
                *stage = Stage::Installed;
                None
            }
        }
    };
    if let Some(ready) = ready {
        if let Err(error) = save(&app) {
            *updates.0.lock().unwrap() = Stage::Ready(ready);
            return Err(error);
        }
        // On Windows a started installer ends Nuzky at once, so the note is there before it starts.
        write_note(&note);
        let handle = app.clone();
        let installed =
            tauri::async_runtime::spawn_blocking(move || (install(&handle, &ready.0, &ready.1), ready)).await;
        let mut stage = updates.0.lock().unwrap();
        *stage = Stage::None;
        match installed {
            Ok((Ok(()), _)) => *stage = Stage::Installed,
            Ok((Err(error), ready)) => {
                remove_note();
                let error = failure(error);
                // Kept for Try again, such as after a closed password prompt.
                if !FINAL.iter().any(|code| error.starts_with(code)) {
                    *stage = Stage::Ready(ready);
                }
                return Err(error);
            }
            Err(error) => {
                remove_note();
                return Err(err(error));
            }
        }
        log::info!(target: "action", "update installed");
    }
    if let Some(work) = busy(&app) {
        remove_note();
        return Ok(Some(wait_for(work)));
    }
    save(&app)?;
    write_note(&note);
    log::info!(target: "action", "restarting to update");
    restart(&app);
    Ok(None)
}

/// Whether an install is under way; the window then stays open, so quitting doesn't cut it short.
pub fn installing(app: &AppHandle) -> bool {
    matches!(*app.state::<Updates>().0.lock().unwrap(), Stage::Installing)
}

fn wait_for(work: &str) -> String {
    format!("Restart when {work} finishes.")
}

fn trusted(url: &str) -> bool {
    url.starts_with(PACKAGES) || (cfg!(debug_assertions) && test_server().is_some() && url.starts_with(TEST_SERVER))
}

/// The updater tauri.conf.json configures. In debug builds the UI flows give the address of their own release on this
/// computer (`NUZKY_UPDATE_URL`) and the public key of their test packages (`NUZKY_UPDATE_PUBKEY`).
fn updater(app: &AppHandle) -> tauri_plugin_updater::Result<Updater> {
    let handle = app.clone();
    let builder = app.updater_builder().on_before_exit(move || before_installer(&handle));
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

/// Only Windows runs this: its installer starts while Nuzky ends with `std::process::exit`, which skips quitting. It
/// only saves, so when Windows refuses to start the installer, Nuzky goes on as it was.
fn before_installer(app: &AppHandle) {
    if let Err(error) = save(app) {
        log::error!("{error}");
    }
}

fn can_install() -> bool {
    packaged() || fake_install().is_some()
}

/// In debug builds a folder stands in for the installed copy (`NUZKY_UPDATE_INSTALL`), so the UI flows install and
/// restart without replacing the build. This copy can install while the folder exists.
#[cfg(debug_assertions)]
fn fake_install() -> Option<std::path::PathBuf> {
    test_server()?;
    Some(std::path::PathBuf::from(std::env::var_os("NUZKY_UPDATE_INSTALL")?)).filter(|dir| dir.is_dir())
}

#[cfg(not(debug_assertions))]
fn fake_install() -> Option<std::path::PathBuf> {
    None
}

#[cfg_attr(not(debug_assertions), allow(unused_variables))]
fn install(app: &AppHandle, update: &Update, bytes: &[u8]) -> tauri_plugin_updater::Result<()> {
    // A debug build never replaces itself.
    #[cfg(debug_assertions)]
    {
        let Some(dir) = fake_install() else { return Err(UpdateError::InvalidUpdaterFormat) };
        update.verify(bytes)?;
        // While `hold` exists the install goes on, so a flow can start work meanwhile.
        let until = std::time::Instant::now() + Duration::from_secs(60);
        while dir.join("hold").exists() && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        // What happens on Windows when it refuses to start the installer.
        if dir.join("refuse").exists() {
            before_installer(app);
            return Err(UpdateError::Io(std::io::Error::other("ShellExecuteW failed with code 5")));
        }
        Ok(std::fs::write(dir.join("installed"), bytes)?)
    }
    #[cfg(not(debug_assertions))]
    update.install(bytes)
}

fn restart(app: &AppHandle) {
    // A debug build is never started again; the UI flows reload the page in its place.
    if cfg!(debug_assertions) {
        return;
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_some()
        && tauri::utils::platform::bundle_type() == Some(tauri::utils::config::BundleType::AppImage)
    {
        *app.state::<Updates>().0.lock().unwrap() = Stage::Restarting;
        return app.exit(0);
    }
    app.request_restart();
}

/// Once Nuzky has quit, starts the updated AppImage as `request_restart` would, but without what tied it to this
/// AppImage's mount, which kept the replaced file mounted, and on disk, until the new Nuzky quit: the entries AppRun put
/// in front of PATH, XDG_DATA_DIRS and the like, and the pipe whose open end keeps the AppImage runtime mounting it.
#[cfg(target_os = "linux")]
pub fn start_again(app: &AppHandle) {
    use std::os::unix::process::CommandExt;
    if !matches!(*app.state::<Updates>().0.lock().unwrap(), Stage::Restarting) {
        return;
    }
    let Some(appimage) = std::env::var_os("APPIMAGE") else { return };
    let mut command = std::process::Command::new(appimage);
    command.args(std::env::args_os().skip(1)).current_dir("/");
    // Only a system call between fork and exec. Without close_range (Linux before 5.11) the pipe stays, as before.
    unsafe {
        command.pre_exec(|| {
            libc::syscall(libc::SYS_close_range, 3, libc::c_uint::MAX, libc::CLOSE_RANGE_CLOEXEC);
            Ok(())
        })
    };
    if let Ok(mount) = std::env::var("APPDIR")
        && mount.len() > 1
    {
        for (key, value) in std::env::vars_os() {
            let Some(value) = value.to_str().filter(|value| value.contains(&mount)) else { continue };
            match without_mount(value, &mount) {
                Some(kept) => command.env(key, kept),
                None => command.env_remove(key),
            };
        }
    }
    if let Err(error) = command.spawn() {
        log::error!("Cannot start the updated Nuzky: {error}");
    }
}

/// A list of paths without those in `mount`; None when none is left.
#[cfg(target_os = "linux")]
fn without_mount(value: &str, mount: &str) -> Option<String> {
    let inside = |path: &str| path.strip_prefix(mount).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
    let kept: Vec<&str> = value.split(':').filter(|path| !inside(path)).collect();
    kept.iter().any(|path| !path.is_empty()).then(|| kept.join(":"))
}

/// Left by the Nuzky that restarted to update, for the one it started. Its project is saved, and its view, the version
/// it updated from and the AI panel's draft go to the new one.
fn note_path() -> std::path::PathBuf {
    crate::store::data_dir().join("update-note.json")
}

fn write_note(note: &str) {
    if let Err(error) = std::fs::write(note_path(), note) {
        log::warn!("Cannot keep what the restarted Nuzky shows: {error}");
    }
}

fn remove_note() {
    std::fs::remove_file(note_path()).ok();
}

/// The note the Nuzky before this one left when it restarted to update. It ended for the update, not by a crash; on
/// Windows its installer ended it without quitting.
pub fn take_note() -> Option<String> {
    let note = std::fs::read_to_string(note_path()).ok()?;
    remove_note();
    crate::diagnostics::ended_for_update();
    Some(note)
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

/// What restarting would cut short, in the words the UI uses (`restartBlocker` in src/lib/updates.ts).
fn busy(app: &AppHandle) -> Option<&'static str> {
    let state = app.state::<AppState>();
    let host = state.session.lock().unwrap().host.clone();
    if host.session.state().is_ok_and(|s| s.open_run.is_some())
        || app.state::<crate::agent_panel::AgentPanel>().working()
    {
        return Some("an AI edit");
    }
    work_of(&state.running_kinds())
}

/// Preparing sound, a preview copy or a person outline starts again after the restart.
fn work_of(kinds: &[String]) -> Option<&'static str> {
    let any = |names: &[&str]| kinds.iter().any(|kind| names.contains(&kind.as_str()));
    if any(&["export", "cover-export"]) {
        Some("the export")
    } else if any(&["captions", "transcript", "transcription"]) {
        Some("speech recognition")
    } else if any(&["speech-model", "vision-models"]) {
        Some("the model download")
    } else if any(&["style"]) {
        Some("learning your style")
    } else {
        let restarts = ["audio", "voice", "dereverb", "level", "proxy", "matte"];
        kinds.iter().any(|kind| !restarts.contains(&kind.as_str())).then_some("the analysis")
    }
}

fn save(app: &AppHandle) -> CmdResult<()> {
    let host = app.state::<AppState>().session.lock().unwrap().host.clone();
    host.session.disconnect().map_err(|error| {
        log::error!("Cannot save the project before updating: {error:#}");
        "UPDATE_SAVE: Nuzky didn't update, as it couldn't save your project. Check that the disk has free space, then \
         try again."
            .into()
    })
}

/// What happened, as the UI shows it, with the details only in the log. The code says whether trying again can help
/// (`UPDATE_NETWORK`, `UPDATE_PASSWORD`, `UPDATE_FAILED`) or only the installer from GitHub (`FINAL`).
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
        _ => "UPDATE_FAILED: Couldn't install the update. Try again.".into(),
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

    /// latest.json could name any address for a package; only the release's own downloads are fetched.
    #[test]
    fn takes_packages_only_from_the_release_downloads() {
        let url = |text: &str| tauri::Url::parse(text).unwrap().to_string();
        assert!(trusted(&url("https://github.com/nuzkyapp/nuzky/releases/download/v0.2.0/Nuzky_0.2.0_amd64.AppImage")));
        for other in [
            "https://github.com/someone/nuzky/releases/download/v0.2.0/Nuzky.AppImage",
            "https://github.com/nuzkyapp/nuzky/releases/download/../../../someone/x/releases/download/v1/a",
            "https://github.com.example.com/nuzkyapp/nuzky/releases/download/v0.2.0/a",
            "http://github.com/nuzkyapp/nuzky/releases/download/v0.2.0/a",
            "https://example.com/nuzkyapp/nuzky/releases/download/v0.2.0/a",
        ] {
            assert!(!trusted(&url(other)), "{other}");
        }
    }

    /// Restarting cancels what runs; what starts again by itself with the project does not count.
    #[test]
    fn names_the_work_a_restart_would_cut_short() {
        let work = |kinds: &[&str]| work_of(&kinds.iter().map(|kind| kind.to_string()).collect::<Vec<_>>());
        assert_eq!(work(&[]), None);
        assert_eq!(work(&["audio", "voice", "dereverb", "level", "proxy", "matte"]), None);
        assert_eq!(work(&["audio", "cover-export"]), Some("the export"));
        assert_eq!(work(&["analysis", "transcription", "export"]), Some("the export"));
        assert_eq!(work(&["captions"]), Some("speech recognition"));
        assert_eq!(work(&["vision-models"]), Some("the model download"));
        assert_eq!(work(&["style"]), Some("learning your style"));
        assert_eq!(work(&["proxy", "cover-mask"]), Some("the analysis"));
    }

    /// The paths AppRun put into the old mount go; the system's and the new mount's stay.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_restarted_appimage_forgets_the_old_mount() {
        let old = "/tmp/.mount_NuzkyAB";
        assert_eq!(
            without_mount("/tmp/.mount_NuzkyCD/usr/share/:/tmp/.mount_NuzkyAB/usr/share/:/usr/share", old).as_deref(),
            Some("/tmp/.mount_NuzkyCD/usr/share/:/usr/share")
        );
        assert_eq!(without_mount("/tmp/.mount_NuzkyAB//usr/lib/gtk-3.0", old), None);
        assert_eq!(
            without_mount("/tmp/.mount_NuzkyAB/usr/share/perl5/:/tmp/.mount_NuzkyAB/usr/lib/perl5/:", old),
            None
        );
        assert_eq!(without_mount("/tmp/.mount_NuzkyAB", old), None);
        assert_eq!(without_mount("/tmp/.mount_NuzkyABC/usr/bin", old).as_deref(), Some("/tmp/.mount_NuzkyABC/usr/bin"));
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

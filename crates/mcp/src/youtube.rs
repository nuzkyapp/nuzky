//! Music from a YouTube or YouTube Music link, through yt-dlp: the one the user installed, or one
//! Nuzky downloads from yt-dlp's own releases when the user asks; Nuzky does not ship it. yt-dlp only
//! ever gets a watch address Nuzky built from the video's id, so nothing else in a pasted link
//! reaches it, and it ignores the user's yt-dlp configuration.
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use nuzky_engine::edit::new_id;
use serde::Deserialize;

/// The sound of a video: AAC in MP4 when YouTube has it, as it nearly always does.
pub const SOUND: &str = "bestaudio[ext=m4a]/bestaudio";
/// The picture of a video without its sound, which Nuzky joins itself, as users have no ffmpeg program for
/// yt-dlp to merge them: at most 1080p, H.264 when YouTube has it, as every computer decodes it.
pub const PICTURE: &str = "bestvideo[height<=1080][vcodec^=avc1]/bestvideo[height<=1080]";
const MAX_JSON_BYTES: u64 = 16 << 20;
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(30);
const PROGRESS: &str =
    "download:nuzky %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s";

fn valid_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Whether the text is meant as a web address rather than words to search for.
pub fn is_link(text: &str) -> bool {
    let text = text.trim().to_ascii_lowercase();
    !text.contains(char::is_whitespace)
        && (text.starts_with("http://")
            || text.starts_with("https://")
            || text.contains("youtube.com/")
            || text.contains("youtu.be/"))
}

/// The id of the one video a youtube.com, music.youtube.com or youtu.be link points at.
pub fn video_id(link: &str) -> Result<String> {
    let invalid = || anyhow!("INVALID_LINK: that is not a link to a YouTube or YouTube Music video");
    let link = link.trim();
    let rest = link.strip_prefix("https://").or_else(|| link.strip_prefix("http://")).unwrap_or(link);
    let (host, rest) = rest.split_once('/').unwrap_or((rest, ""));
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let path = path.split('#').next().unwrap_or_default().trim_end_matches('/');
    let query = query.split('#').next().unwrap_or_default();
    let param = |name: &str| query.split('&').find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='));
    let id = match host.to_ascii_lowercase().as_str() {
        "youtu.be" | "www.youtu.be" => path,
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com" => match path.split_once('/') {
            None if path == "watch" => param("v").unwrap_or_default(),
            None if path == "playlist" => {
                bail!("PLAYLIST: that is a link to a playlist. Open one song and copy its link instead")
            }
            Some(("shorts" | "live" | "embed", id)) => id,
            _ => "",
        },
        _ => "",
    };
    ensure!(valid_id(id), invalid());
    Ok(id.to_owned())
}

/// The only address yt-dlp is given.
pub fn watch_url(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

// ---------- Finding yt-dlp ----------

/// Where installers put yt-dlp when the app was started without the user's shell `PATH`, as from the
/// Dock, Finder or a desktop launcher.
fn known_dirs() -> Vec<PathBuf> {
    let home = dirs::home_dir();
    // pipx puts yt-dlp in ~/.local/bin on every system; deno's own installer, in ~/.deno/bin.
    let user = [".local", ".deno"].map(|dir| home.as_ref().map(|h| h.join(dir).join("bin")));
    if cfg!(windows) {
        let winget = dirs::data_local_dir().map(|d| d.join("Microsoft").join("WinGet").join("Links"));
        user.into_iter().chain([winget, home.map(|h| h.join("scoop").join("shims"))]).flatten().collect()
    } else {
        user.into_iter()
            .flatten()
            .chain(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/snap/bin"].map(PathBuf::from))
            .collect()
    }
}

/// The app's PATH, then the folders installers use.
fn search_dirs() -> Vec<PathBuf> {
    let path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    path.into_iter().chain(known_dirs()).collect()
}

fn executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// The first yt-dlp in `dirs`.
fn locate(dirs: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    let name = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };
    dirs.into_iter().map(|dir| dir.join(name)).find(|path| executable(path))
}

/// Nuzky's own yt-dlp first, as Nuzky keeps it up to date, then the one the user installed.
/// `NUZKY_YTDLP` stands in for the user's, such as the tests' fake one.
pub fn find() -> Option<PathBuf> {
    let own = own_copy();
    if executable(&own) {
        return Some(own);
    }
    if let Some(path) = std::env::var_os("NUZKY_YTDLP") {
        return Some(PathBuf::from(path)).filter(|path| executable(path));
    }
    locate(search_dirs())
}

fn missing() -> anyhow::Error {
    if release_file().is_some() {
        return anyhow!(
            "YTDLP_MISSING: Nuzky gets music from YouTube with yt-dlp, a free tool that is not on this computer yet. The user can paste the link in the Music tab of Nuzky's sound library and click Download yt-dlp under it."
        );
    }
    anyhow!(UNSUPPORTED)
}

// ---------- Nuzky's own yt-dlp ----------

const LATEST: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";
/// Every file Nuzky downloads comes from here.
const DOWNLOADS: &str = "https://github.com/yt-dlp/yt-dlp/releases/download/";
const UNSUPPORTED: &str = "YTDLP_UNSUPPORTED: Nuzky gets music from YouTube with yt-dlp, which is not installed and which Nuzky can't download for this computer. Install it from github.com/yt-dlp/yt-dlp, then try again.";
/// Tests serve the release from this host through `NUZKY_YTDLP_RELEASES`; nothing else replaces GitHub.
const TEST_SERVER: &str = "http://127.0.0.1:";
/// YouTube changes often enough to break an old yt-dlp.
const REFRESH: Duration = Duration::from_secs(7 * 24 * 3600);
const RETRY: Duration = Duration::from_secs(10 * 60);

/// The file of yt-dlp's release that runs on this computer, as yt-dlp names it.
fn release_file() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", _) => Some("yt-dlp_macos"),
        ("linux", "x86_64") => Some("yt-dlp_linux"),
        ("linux", "aarch64") => Some("yt-dlp_linux_aarch64"),
        ("windows", "x86_64") => Some("yt-dlp.exe"),
        ("windows", "aarch64") => Some("yt-dlp_arm64.exe"),
        _ => None,
    }
}

/// Whether Nuzky has its own yt-dlp.
pub fn downloaded() -> bool {
    executable(&own_copy())
}

fn own_copy() -> PathBuf {
    crate::sounds::data_dir().join("tools").join(if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" })
}

/// When Nuzky last checked its yt-dlp against the newest release.
fn checked_stamp() -> PathBuf {
    own_copy().with_file_name("yt-dlp.checked")
}

#[derive(Deserialize)]
struct Release {
    assets: Vec<ReleaseFile>,
}

#[derive(Deserialize)]
struct ReleaseFile {
    name: String,
    size: u64,
    /// "sha256:<hex>", as GitHub computed it when the file was published.
    digest: Option<String>,
    browser_download_url: String,
}

/// Where the newest release's `file` downloads from, with its size and SHA-256.
fn newest(file: &str) -> Result<(String, u64, String)> {
    let test = std::env::var("NUZKY_YTDLP_RELEASES").ok().filter(|url| url.starts_with(TEST_SERVER));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(test.is_none())
        .max_redirects(5)
        .user_agent(crate::sounds::AGENT)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut response = agent.get(test.as_deref().unwrap_or(LATEST)).call().map_err(|error| match error {
        ureq::Error::StatusCode(403 | 429) => {
            anyhow!("RATE_LIMITED: GitHub gets too many requests from this network. Try again in an hour.")
        }
        ureq::Error::StatusCode(code) => {
            anyhow!("SOURCE_FAILED: GitHub answered with an error ({code}). Try again later.")
        }
        error => anyhow!("OFFLINE: Can't reach GitHub to download yt-dlp. Check your internet connection. ({error})"),
    })?;
    let mut text = String::new();
    response.body_mut().with_config().limit(4 << 20).reader().read_to_string(&mut text).map_err(|e| {
        anyhow!("OFFLINE: Can't reach GitHub to download yt-dlp. Check your internet connection. ({e})")
    })?;
    let release: Release = serde_json::from_str(&text).context("SOURCE_FAILED: GitHub sent something unexpected")?;
    let found = release
        .assets
        .into_iter()
        .find(|a| a.name == file)
        .context("SOURCE_FAILED: yt-dlp's newest release has no file for this computer yet. Try again later.")?;
    let sha256 = found
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .filter(|sum| sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_hexdigit()))
        .context("SOURCE_FAILED: GitHub gave no checksum for yt-dlp")?
        .to_ascii_lowercase();
    let url = found.browser_download_url;
    ensure!(
        url.starts_with(DOWNLOADS) || (test.is_some() && url.starts_with(TEST_SERVER)),
        "SOURCE_FAILED: GitHub pointed yt-dlp somewhere else"
    );
    Ok((url, found.size, sha256))
}

/// Downloads the newest yt-dlp for this computer from yt-dlp's GitHub releases to `path`, checked
/// against the size and SHA-256 GitHub gives for it, unless `path` already is that file.
fn install_to(path: &Path, cancel: &AtomicBool, progress: impl FnMut(f32)) -> Result<()> {
    let file = release_file().context(UNSUPPORTED)?;
    // The lookup runs on its own thread, so Stop is felt at once even while GitHub keeps silent.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || tx.send(newest(file)).ok());
    let (url, size, sha256) = loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(found) => break found?,
            Err(RecvTimeoutError::Timeout) => nuzky_session::jobs::check_cancel(cancel)?,
            Err(RecvTimeoutError::Disconnected) => bail!("SOURCE_FAILED: looking for yt-dlp stopped unexpectedly"),
        }
    };
    let integrity = crate::model_download::Integrity { size, sha256: &sha256 };
    crate::model_download::download(&url, path, integrity, cancel, progress).map_err(|error| {
        let text = format!("{error:#}");
        if text.starts_with("CANCELLED") {
            error
        } else if text.starts_with("MODEL_INVALID") {
            anyhow!("YTDLP_CORRUPT: yt-dlp did not download correctly. Try again.")
        } else {
            anyhow!("YTDLP_DOWNLOAD_FAILED: yt-dlp didn't download. Check your internet connection and free disk space, then try again. ({text})")
        }
    })?;
    #[cfg(unix)]
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    Ok(())
}

/// Downloads yt-dlp for Nuzky's own use; `cancel` stops it within 100 ms.
pub fn install(cancel: &AtomicBool, progress: impl FnMut(f32)) -> Result<()> {
    install_to(&own_copy(), cancel, progress)?;
    std::fs::write(checked_stamp(), b"")?;
    Ok(())
}

/// Once a week, after yt-dlp has run, Nuzky's own copy is brought up to date in the background, so
/// nothing waits for it and Windows lets it replace the file. A failed try, such as offline, is repeated
/// after ten minutes of use; a finished download is then only moved into place.
fn refresh() {
    static TRIED: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let stale = std::fs::metadata(checked_stamp())
        .and_then(|m| m.modified())
        .map_or(true, |at| at.elapsed().unwrap_or_default() > REFRESH);
    if !executable(&own_copy()) || !stale {
        return;
    }
    let mut tried = TRIED.lock().unwrap();
    if tried.is_some_and(|at| at.elapsed() < RETRY) {
        return;
    }
    *tried = Some(Instant::now());
    std::thread::spawn(|| {
        if let Err(error) = install(&AtomicBool::new(false), |_| {}) {
            log::warn!("Updating yt-dlp failed: {error:#}");
        }
    });
}

fn command(format: &str, args: &[&str]) -> Result<Command> {
    let program = find().ok_or_else(missing)?;
    // yt-dlp runs deno and ffmpeg from its PATH, which an app started from the Dock lacks.
    let path = std::env::join_paths(program.parent().map(Path::to_path_buf).into_iter().chain(search_dirs()))?;
    let mut command = Command::new(program);
    command
        .args(["--ignore-config", "--no-playlist", "--encoding", "utf-8", "--socket-timeout", "20", "-f", format])
        .args(args)
        .env("PATH", path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    #[cfg(windows)]
    {
        // A console program started from the app would otherwise flash a terminal window.
        std::os::windows::process::CommandExt::creation_flags(&mut command, CREATE_NO_WINDOW);
    }
    Ok(command)
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Stops yt-dlp with what it started: its standalone builds run Python as a child, and fixing a file runs ffmpeg.
fn stop(child: &mut Child) {
    #[cfg(unix)]
    // SAFETY: the child leads its own process group, so this reaches only yt-dlp and its children.
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let mut taskkill = Command::new("taskkill");
        taskkill.args(["/T", "/F", "/PID", &child.id().to_string()]).stdout(Stdio::null()).stderr(Stdio::null());
        std::os::windows::process::CommandExt::creation_flags(&mut taskkill, CREATE_NO_WINDOW);
        taskkill.status().ok();
    }
    child.kill().ok();
    child.wait().ok();
}

/// Reads stderr on its own thread, so a chatty yt-dlp never blocks on a full pipe.
fn collect_stderr(child: &mut Child) -> std::thread::JoinHandle<String> {
    let stderr = child.stderr.take();
    std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(stderr) = stderr {
            BufReader::new(stderr).take(64 << 10).read_to_string(&mut text).ok();
        }
        text
    })
}

/// The plain error for what yt-dlp printed when it failed. `installed` is a yt-dlp the user installed,
/// which Nuzky can't update, so a failure it can't name offers Nuzky's own copy instead.
fn failure(stderr: &str, installed: bool) -> anyhow::Error {
    let error = stderr.lines().rev().find(|l| l.starts_with("ERROR:")).unwrap_or_default();
    let has = |text: &str| error.to_lowercase().contains(text);
    if has("private video") {
        anyhow!("SOUND_UNAVAILABLE: This video is private. Only videos anyone can watch can be added.")
    } else if has("not a bot") {
        anyhow!("SOURCE_REFUSED: YouTube refused the download. Try again later, or update yt-dlp.")
    } else if has("confirm your age") || has("members-only") || has("join this channel") {
        anyhow!("SOUND_UNAVAILABLE: YouTube wants you to sign in to play this video, so Nuzky can't add it.")
    } else if ["video unavailable", "is unavailable", "removed", "does not exist", "available in your country"]
        .iter()
        .any(|text| has(text))
    {
        anyhow!("SOUND_UNAVAILABLE: This video is unavailable. It may be removed or blocked in your country.")
    } else if has("http error 429") || has("too many requests") {
        anyhow!("RATE_LIMITED: YouTube gets too many requests from this network. Try again in a few minutes.")
    } else if [
        "unable to download webpage",
        "resolve",
        "getaddrinfo",
        "timed out",
        "unreachable",
        "connection",
        "network",
    ]
    .iter()
    .any(|text| has(text))
    {
        anyhow!("OFFLINE: Can't reach YouTube. Check your internet connection. ({})", error.trim())
    } else {
        let error = error.trim_start_matches("ERROR:").trim();
        if installed {
            anyhow!(
                "YTDLP_OUTDATED: The yt-dlp installed on this computer could not get this video, most likely because it is out of date. The user can paste the link in the Music tab of Nuzky's sound library and click Download yt-dlp under it; Nuzky keeps its own copy up to date. ({error})"
            )
        } else {
            anyhow!("SOURCE_FAILED: yt-dlp could not get this video. Updating yt-dlp often helps. ({error})")
        }
    }
}

// ---------- Looking up a video ----------

/// What yt-dlp says about a video, as much as the library shows.
#[derive(Deserialize)]
pub struct Video {
    pub id: String,
    pub title: Option<String>,
    /// YouTube Music's own field for a song.
    pub artist: Option<String>,
    pub channel: Option<String>,
    pub uploader: Option<String>,
    /// Seconds.
    pub duration: Option<f64>,
    /// The extension of the sound yt-dlp would download.
    pub ext: Option<String>,
    filesize: Option<u64>,
    filesize_approx: Option<u64>,
    live_status: Option<String>,
}

/// Looks up one video and the size of its `format` without downloading it: a few seconds, and at most half a
/// minute. `cancel` stops yt-dlp within 50 ms.
pub fn video(id: &str, format: &str, max_bytes: u64, cancel: &AtomicBool) -> Result<Video> {
    ensure!(valid_id(id), "INVALID_LINK: {id:?} is not a YouTube video id");
    let mut child = command(format, &["--skip-download", "--dump-single-json", "--no-warnings", "--", &watch_url(id)])?
        .spawn()
        .map_err(|e| anyhow!("YTDLP_FAILED: yt-dlp could not start ({e})"))?;
    let stderr = collect_stderr(&mut child);
    let stdout = child.stdout.take().context("yt-dlp has no output")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(MAX_JSON_BYTES).read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + RESOLVE_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() > deadline {
            stop(&mut child);
            bail!("OFFLINE: YouTube did not answer within half a minute. Check your internet connection.");
        }
        if cancel.load(Ordering::Relaxed) {
            stop(&mut child);
            bail!("CANCELLED: the lookup stopped");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let stderr = stderr.join().unwrap_or_default();
    refresh();
    ensure!(status.success(), failure(&stderr, !executable(&own_copy())));
    let bytes = reader.join().map_err(|_| anyhow!("yt-dlp output was lost"))??;
    let video: Video = serde_json::from_slice(&bytes)
        .map_err(|e| anyhow!("SOURCE_FAILED: yt-dlp sent something unexpected. Updating yt-dlp often helps. ({e})"))?;
    ensure!(video.id == id, "SOURCE_FAILED: yt-dlp answered about another video");
    ensure!(
        !matches!(video.live_status.as_deref(), Some("is_live" | "is_upcoming" | "post_live")),
        "SOUND_UNAVAILABLE: Live streams can't be added. Try again once the stream is over and processed."
    );
    ensure!(
        video.filesize.or(video.filesize_approx).is_none_or(|size| size <= max_bytes),
        "SOUND_UNAVAILABLE: this video is too long to download"
    );
    Ok(video)
}

// ---------- Downloading ----------

/// Downloads the `format` of the video to `path` through a temporary file beside it. `cancel` stops it
/// within 100 ms and leaves nothing behind.
pub fn download(
    id: &str,
    format: &str,
    path: &Path,
    max_bytes: u64,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<()> {
    ensure!(valid_id(id), "INVALID_LINK: {id:?} is not a YouTube video id");
    let dir = path.parent().context("The download needs a directory")?;
    std::fs::create_dir_all(dir)?;
    let part = dir.join(format!(".nuzky-part-{}", new_id()));
    let max = format!("{max_bytes}");
    let result = (|| {
        // yt-dlp reads -o as a template, where % starts a field.
        let output = part.to_str().context("The cache folder's name is not valid text")?.replace('%', "%%");
        let mut child = command(
            format,
            &[
                "--quiet",
                "--progress",
                "--newline",
                "--progress-template",
                PROGRESS,
                "--no-part",
                "--no-mtime",
                "--abort-on-unavailable-fragments",
                "--max-filesize",
                &max,
                "-o",
                &output,
                "--",
                &watch_url(id),
            ],
        )?
        .spawn()
        .map_err(|e| anyhow!("YTDLP_FAILED: yt-dlp could not start ({e})"))?;
        let stderr = collect_stderr(&mut child);
        let (tx, rx) = mpsc::channel();
        let stdout = child.stdout.take().context("yt-dlp has no output")?;
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(p) = line.strip_prefix("nuzky ").and_then(fraction) {
                    tx.send(p).ok();
                }
            }
        });
        let status = loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(p) => progress(p),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
            }
            if cancel.load(Ordering::Relaxed) {
                stop(&mut child);
                bail!("CANCELLED: the download stopped");
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
        };
        let stderr = stderr.join().unwrap_or_default();
        refresh();
        ensure!(status.success(), failure(&stderr, !executable(&own_copy())));
        // Over --max-filesize yt-dlp skips the download and still succeeds.
        let size = part.metadata().map(|m| m.len()).unwrap_or(0);
        ensure!(size > 0, "SOUND_UNAVAILABLE: this video is too long to download");
        ensure!(size <= max_bytes, "SOUND_UNAVAILABLE: this video is too long to download");
        std::fs::OpenOptions::new().write(true).open(&part)?.sync_all()?;
        std::fs::rename(&part, path).context("Saving the download")
    })();
    if result.is_err() {
        std::fs::remove_file(&part).ok();
    }
    result
}

/// How far a download is, from yt-dlp's "downloaded total estimate" line; NA where it does not know.
fn fraction(line: &str) -> Option<f32> {
    let mut numbers = line.split_whitespace().map(|n| n.parse::<f64>().ok());
    let done = numbers.next()??;
    let total = numbers.next()?.or(numbers.next()?)?;
    (total > 0.0).then(|| (done / total).clamp(0.0, 1.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_to_one_video_give_its_id() {
        for link in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?v=dQw4w9WgXcQ&t=42s",
            "http://m.youtube.com/watch?feature=share&v=dQw4w9WgXcQ",
            "www.youtube.com/watch?v=dQw4w9WgXcQ#comments",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ&si=abc",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=RDAMVMdQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ?si=Xy",
            "youtu.be/dQw4w9WgXcQ",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "  https://WWW.YouTube.com/watch?v=dQw4w9WgXcQ\n",
        ] {
            assert_eq!(video_id(link).unwrap(), "dQw4w9WgXcQ", "{link}");
        }
        assert_eq!(watch_url("dQw4w9WgXcQ"), "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    }

    #[test]
    fn playlists_and_other_links_are_refused() {
        let playlist =
            video_id("https://www.youtube.com/playlist?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG").unwrap_err();
        assert!(playlist.to_string().starts_with("PLAYLIST:"), "{playlist}");
        let music = video_id("https://music.youtube.com/playlist?list=OLAK5uy_abc").unwrap_err();
        assert!(music.to_string().starts_with("PLAYLIST:"), "{music}");
        for link in [
            "",
            "lofi beats",
            "https://www.youtube.com/",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQx",
            "https://www.youtube.com/watch?v=dQw4w9WgX%5D",
            "https://www.youtube.com/@channel",
            "https://www.youtube.com/channel/UCuAXFkgsw1L7xaCfnd5JJOw",
            "https://evil.example/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com.evil.example/watch?v=dQw4w9WgXcQ",
            "https://youtube.com@evil.example/watch?v=dQw4w9WgXcQ",
            "file:///etc/passwd",
            "https://vimeo.com/76979871",
        ] {
            let error = video_id(link).unwrap_err();
            assert!(error.to_string().starts_with("INVALID_LINK:"), "{link}: {error}");
        }
    }

    #[test]
    fn links_are_told_from_search_words() {
        assert!(is_link("https://youtu.be/dQw4w9WgXcQ"));
        assert!(is_link("music.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(is_link("https://vimeo.com/76979871"));
        assert!(!is_link("upbeat"));
        assert!(!is_link("youtube music lofi"));
    }

    #[test]
    fn yt_dlp_errors_become_plain_ones() {
        let code = |stderr: &str| failure(stderr, false).to_string().split(':').next().unwrap().to_owned();
        assert_eq!(
            code("ERROR: [youtube] abc: Private video. Sign in if you've been granted access"),
            "SOUND_UNAVAILABLE"
        );
        assert_eq!(code("WARNING: x\nERROR: [youtube] aaaaaaaaaaa: This video is unavailable"), "SOUND_UNAVAILABLE");
        assert_eq!(code("ERROR: [youtube] abc: Sign in to confirm you’re not a bot"), "SOURCE_REFUSED");
        assert_eq!(
            code(
                "ERROR: [youtube] abc: Unable to download webpage: <urlopen error [Errno 8] nodename nor servname provided>"
            ),
            "OFFLINE"
        );
        assert_eq!(
            code("ERROR: [youtube] abc: Unable to download webpage: HTTP Error 429: Too Many Requests"),
            "RATE_LIMITED"
        );
        assert_eq!(code("ERROR: something new"), "SOURCE_FAILED");
        assert_eq!(code(""), "SOURCE_FAILED");
        let installed = failure("ERROR: unable to download video data: HTTP Error 403: Forbidden", true).to_string();
        assert!(installed.starts_with("YTDLP_OUTDATED:"), "{installed}");
    }

    #[test]
    fn progress_reads_yt_dlp_numbers() {
        assert_eq!(fraction("512 1024 NA"), Some(0.5));
        assert_eq!(fraction("512 NA 2048"), Some(0.25));
        assert_eq!(fraction("512 NA NA"), None);
        assert_eq!(fraction("NA NA NA"), None);
    }

    /// The app started from the Dock or a desktop launcher has a short PATH; yt-dlp is found in the
    /// folders installers use, and only a file it may run counts.
    #[test]
    fn yt_dlp_is_found_in_the_given_folders() {
        let root = std::env::temp_dir().join(format!("nuzky-ytdlp-{}", new_id()));
        let (empty, installed) = (root.join("empty"), root.join("bin"));
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(&installed).unwrap();
        let name = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };
        assert_eq!(locate([empty.clone(), installed.clone()]), None);
        std::fs::write(installed.join(name), b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            assert_eq!(locate([empty.clone(), installed.clone()]), None, "a file it may not run");
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(installed.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(locate([empty, installed.clone()]), Some(installed.join(name)));
        std::fs::remove_dir_all(root).ok();
    }

    /// Downloads the real yt-dlp from GitHub as Download yt-dlp does, into a folder of its own, and runs it.
    #[test]
    #[ignore]
    fn downloads_yt_dlp_from_its_releases() {
        let dir = std::env::temp_dir().join(format!("nuzky-ytdlp-{}", new_id()));
        let path = dir.join(if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" });
        let mut steps = Vec::new();
        install_to(&path, &AtomicBool::new(false), |p| steps.push(p)).unwrap();
        let version = Command::new(&path).arg("--version").output().unwrap();
        eprintln!("yt-dlp {} at {path:?}", String::from_utf8_lossy(&version.stdout).trim());
        assert!(version.status.success() && steps.last() == Some(&1.0), "{version:?} {:?}", steps.last());
        assert!(executable(&path));
        install_to(&path, &AtomicBool::new(false), |_| panic!("an up-to-date copy is downloaded again")).unwrap();
        std::fs::remove_dir_all(dir).ok();
    }

    /// Run with yt-dlp installed: it is found, also with the short PATH of an app started from the
    /// Dock, Finder or a desktop launcher, and it runs.
    #[test]
    #[ignore]
    fn finds_the_installed_yt_dlp() {
        let found = find().expect("yt-dlp is installed");
        let short = locate(["/usr/bin", "/bin"].map(PathBuf::from).into_iter().chain(known_dirs()));
        let version = Command::new(&found).arg("--version").output().unwrap();
        assert!(version.status.success(), "{found:?}");
        eprintln!(
            "yt-dlp {} at {found:?}, without the shell PATH at {short:?}",
            String::from_utf8_lossy(&version.stdout).trim()
        );
        assert!(short.is_some(), "not found without the shell PATH");
    }
}

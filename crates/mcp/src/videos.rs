//! Videos from YouTube links, through the same yt-dlp as music (`crate::youtube`). The whole video downloads into
//! the cache, picture and sound apart, for the user to watch and choose a part. Adding it joins the two into one
//! file in Nuzky's own folder, keeping only the part and up to 3 s around it, which a clip's edge can reveal later.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use anyhow::{Result, anyhow, ensure};
use nuzky_engine::edit::new_id;
use nuzky_engine::model::{Asset, AssetKind, Part};
use serde::{Deserialize, Serialize};

use crate::youtube::{PICTURE, SOUND};

/// About an hour of 1080p.
const MAX_BYTES: u64 = 4 << 30;
/// The share of a download the picture takes; the sound is the rest.
const PICTURE_SHARE: f32 = 0.9;
/// How long a lookup's addresses are used to download; YouTube keeps them working for hours.
const INFO_LIFETIME: Duration = Duration::from_secs(30 * 60);

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "YoutubeVideo"))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub duration_us: i64,
}

/// The video a YouTube link points at. What yt-dlp found stays in the cache, so `fetch` downloads without looking
/// it up again. `cancel` stops the lookup.
pub fn link(url: &str, cache_dir: &Path, cancel: &AtomicBool) -> Result<Video> {
    let id = crate::youtube::video_id(url)?;
    let video = crate::youtube::video(&id, PICTURE, MAX_BYTES, cancel)?;
    if let Err(error) = save_info(cache_dir, &id, &video.info) {
        log::warn!("Keeping what yt-dlp found failed: {error:#}");
    }
    let text =
        |t: Option<&str>| t.map(|t| t.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|t| !t.is_empty());
    let channel = [&video.channel, &video.uploader, &video.artist].into_iter().find_map(|a| text(a.as_deref()));
    Ok(Video {
        id,
        title: text(video.title.as_deref()).unwrap_or_else(|| "Untitled".into()),
        channel: channel.unwrap_or_else(|| "Unknown".into()),
        duration_us: (video.duration.unwrap_or(0.0).max(0.0) * 1e6) as i64,
    })
}

/// The picture and the sound of a downloaded video.
pub struct Download {
    pub picture: PathBuf,
    pub sound: PathBuf,
}

fn cache(cache_dir: &Path) -> PathBuf {
    cache_dir.join("youtube")
}

fn info_path(cache_dir: &Path, id: &str) -> PathBuf {
    cache(cache_dir).join(format!("{id}.info.json"))
}

fn save_info(cache_dir: &Path, id: &str, info: &[u8]) -> Result<()> {
    let path = info_path(cache_dir, id);
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir)?;
    let part = dir.join(format!(".nuzky-part-{}", new_id()));
    std::fs::write(&part, info).and_then(|()| std::fs::rename(&part, &path)).inspect_err(|_| {
        std::fs::remove_file(&part).ok();
    })?;
    Ok(())
}

/// What `link` found for the video, while its addresses still work.
fn fresh_info(cache_dir: &Path, id: &str) -> Option<PathBuf> {
    let path = info_path(cache_dir, id);
    let age = path.metadata().and_then(|m| m.modified()).ok()?.elapsed().ok()?;
    (age < INFO_LIFETIME).then_some(path)
}

/// Deletes the downloaded videos, as the next app start does.
pub fn forget(cache_dir: &Path) {
    std::fs::remove_dir_all(cache(cache_dir)).ok();
}

/// Downloads the whole video into the cache, or finds it there, picture and sound at once. Only the latest video
/// stays: a long one takes gigabytes. `cancel` stops it within about 100 ms and leaves nothing behind.
pub fn fetch(id: &str, cache_dir: &Path, cancel: &AtomicBool, mut progress: impl FnMut(f32)) -> Result<Download> {
    ensure!(
        crate::sounds::settings().youtube_rights,
        "RIGHTS_UNCONFIRMED: before the first video or song from a YouTube link, the user must agree that they are responsible for the rights to it and for following YouTube's terms"
    );
    let dir = cache(cache_dir);
    let download = Download { picture: dir.join(format!("{id}.picture")), sound: dir.join(format!("{id}.sound")) };
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if !entry.file_name().to_string_lossy().starts_with(&format!("{id}.")) {
                std::fs::remove_file(entry.path()).ok();
            }
        }
    }
    let info = fresh_info(cache_dir, id);
    // Either failing stops the other, so its error is the one told.
    let stop = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel();
    let results = std::thread::scope(|scope| {
        let workers = [(PICTURE, &download.picture), (SOUND, &download.sound)].into_iter().enumerate().map(
            |(i, (format, path))| {
                let (tx, stop, info) = (tx.clone(), &stop, info.as_deref());
                scope.spawn(move || {
                    if path.is_file() {
                        tx.send((i, 1.0)).ok();
                        return Ok(());
                    }
                    let result = crate::youtube::download(id, format, path, info, MAX_BYTES, stop, |p| {
                        tx.send((i, p)).ok();
                    });
                    if result.is_err() {
                        stop.store(true, Ordering::Relaxed);
                    }
                    result
                })
            },
        );
        let workers: Vec<_> = workers.collect();
        drop(tx);
        let mut done = [0.0f32; 2];
        loop {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok((i, p)) => {
                    done[i] = p;
                    progress(done[0] * PICTURE_SHARE + done[1] * (1.0 - PICTURE_SHARE));
                }
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if cancel.load(Ordering::Relaxed) {
                stop.store(true, Ordering::Relaxed);
            }
        }
        workers
            .into_iter()
            .map(|w| w.join().unwrap_or_else(|_| Err(anyhow!("The download crashed"))))
            .collect::<Vec<_>>()
    });
    ensure!(!cancel.load(Ordering::Relaxed), "CANCELLED: the download stopped");
    let mut errors: Vec<_> = results.into_iter().filter_map(Result::err).collect();
    errors.sort_by_key(|e| format!("{e}").starts_with("CANCELLED"));
    match errors.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(download),
    }
}

/// The video as a project asset: its `part`, or all of it, joined into a new file in Nuzky's own folder, since
/// a cache may be emptied while the project still needs the file.
pub fn asset(
    video: &Video,
    part: Option<Part>,
    cache_dir: &Path,
    cancel: &AtomicBool,
    progress: impl FnMut(f32),
) -> Result<Asset> {
    if let Some(part) = part {
        ensure!(
            0 <= part.start_us && part.start_us < part.end_us && part.start_us < video.duration_us,
            "INVALID_ARGUMENTS: the part must start before it ends, inside the video's {} s",
            video.duration_us / 1_000_000
        );
    }
    let download = fetch(&video.id, cache_dir, cancel, progress)?;
    let path = crate::sounds::data_dir().join("videos").join(format!("youtube-{}-{}.mp4", video.id, new_id()));
    std::fs::create_dir_all(path.parent().unwrap())?;
    let kept = nuzky_engine::join::join(&download.picture, Some(&download.sound), &path, part, cancel)?;
    let probed = nuzky_engine::media::probe(&path, new_id())
        .map_err(|e| anyhow!("SOUND_UNAVAILABLE: {} is not a video Nuzky can play ({e:#})", video.title))
        .and_then(|asset| {
            ensure!(asset.kind == AssetKind::Video, "SOUND_UNAVAILABLE: {} has no picture", video.title);
            Ok(asset)
        });
    let Ok(mut asset) = probed else {
        std::fs::remove_file(&path).ok();
        return probed;
    };
    asset.name = video.title.clone();
    asset.part = kept;
    Ok(asset)
}

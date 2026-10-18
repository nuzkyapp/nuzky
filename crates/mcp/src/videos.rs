//! Videos from YouTube links, through the same yt-dlp as music (`crate::youtube`). The whole video downloads into
//! the cache, picture and sound apart, for the user to watch and choose a part. Adding it joins the two into one
//! file in Nuzky's own folder, keeping only the part and up to 3 s around it, which a clip's edge can reveal later.
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{Result, anyhow, ensure};
use nuzky_engine::edit::new_id;
use nuzky_engine::model::{Asset, AssetKind, Part};
use serde::{Deserialize, Serialize};

use crate::youtube::{PICTURE, SOUND};

/// About an hour of 1080p.
const MAX_BYTES: u64 = 4 << 30;
/// The share of a download the picture takes; the sound is the rest.
const PICTURE_SHARE: f32 = 0.9;

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "YoutubeVideo"))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub duration_us: i64,
}

/// The video a YouTube link points at.
pub fn link(url: &str) -> Result<Video> {
    let id = crate::youtube::video_id(url)?;
    let video = crate::youtube::video(&id, PICTURE, MAX_BYTES)?;
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

/// Deletes the downloaded videos, as the next app start does.
pub fn forget(cache_dir: &Path) {
    std::fs::remove_dir_all(cache(cache_dir)).ok();
}

/// Downloads the whole video into the cache, or finds it there. Only the latest video stays: a long one takes
/// gigabytes. `cancel` stops it within 100 ms and leaves nothing behind.
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
    if !download.picture.is_file() {
        crate::youtube::download(id, PICTURE, &download.picture, MAX_BYTES, cancel, |p| progress(p * PICTURE_SHARE))?;
    }
    if !download.sound.is_file() {
        crate::youtube::download(id, SOUND, &download.sound, MAX_BYTES, cancel, |p| {
            progress(PICTURE_SHARE + p * (1.0 - PICTURE_SHARE))
        })?;
    }
    Ok(download)
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

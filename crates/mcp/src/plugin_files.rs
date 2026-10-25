//! Pictures and sounds a plugin ships in its own folder, such as a logo or a beep. A plugin names them by file name
//! only, so it reaches no other file. Each is read without following links, copied into Nuzky's own folder under the
//! hash of its bytes and probed there as FFmpeg reads any media, so a project never refers to the plugin's folder and
//! keeps working after the plugin is deleted.
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, anyhow, ensure};
use nuzky_engine::edit::new_id;
use nuzky_engine::model::{Asset, AssetKind};
use sha2::{Digest, Sha256};

use crate::web_image::{DECODE_PIXELS, MAX_PIXELS, MAX_SIDE};

pub(crate) const MAX_FILES: usize = 20;
const MAX_BYTES: u64 = 20 << 20;
const PICTURES: [&str; 4] = ["png", "jpg", "jpeg", "webp"];
const SOUNDS: [&str; 7] = ["mp3", "wav", "m4a", "aac", "ogg", "opus", "flac"];
const NAMES: &str = "1 to 60 letters, digits, dots, dashes and _, not starting with a dot, ending in .png, .jpg, .jpeg, .webp, .mp3, .wav, .m4a, .aac, .ogg, .opus or .flac";

/// What a file of this name must be, when the name is one a plugin may use.
pub(crate) fn kind(name: &str) -> Result<AssetKind> {
    let ext = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    let kind = if PICTURES.contains(&ext.as_str()) {
        Some(AssetKind::Image)
    } else if SOUNDS.contains(&ext.as_str()) {
        Some(AssetKind::Audio)
    } else {
        None
    };
    let plain = (1..=60).contains(&name.len())
        && !name.starts_with('.')
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'));
    kind.filter(|_| plain).with_context(|| format!("{name:?} is not a file name a plugin may use: {NAMES}"))
}

/// Whether the plugin's folder holds this file as a plain file of its own, without reading it.
pub(crate) fn present(folder: &Path, name: &str) -> Result<()> {
    let meta = std::fs::symlink_metadata(folder.join(name)).context("missing from the plugin's folder")?;
    ensure!(meta.is_file(), "not a plain file");
    #[cfg(unix)]
    ensure!(std::os::unix::fs::MetadataExt::nlink(&meta) == 1, "a link to another file");
    ensure!(meta.len() <= MAX_BYTES, "larger than {} MB", MAX_BYTES >> 20);
    Ok(())
}

/// The file at `path` as a new asset named `name`, whose path is its copy in Nuzky's own folder. `own` reads a
/// plugin's file, which must be a plain file and not a link of any kind; an agent's file may be reached through one.
pub(crate) fn stock(path: &Path, name: &str, own: bool) -> Result<Asset> {
    let kind = kind(name)?;
    let bytes = read(path, own)?;
    let hash: String = Sha256::digest(&bytes).iter().take(16).map(|b| format!("{b:02x}")).collect();
    let ext = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    let kept = crate::sounds::data_dir().join("plugin-files").join(format!("{hash}.{ext}"));
    let fresh = !kept.is_file();
    if fresh {
        crate::sounds::publish(&kept, |file| Ok(std::io::Write::write_all(file, &bytes)?))?;
    }
    match probe(&kept, kind) {
        Ok(mut asset) => {
            asset.name = name.to_owned();
            Ok(asset)
        }
        Err(error) => {
            if fresh {
                std::fs::remove_file(&kept).ok();
            }
            Err(error)
        }
    }
}

fn read(path: &Path, own: bool) -> Result<Vec<u8>> {
    if own {
        ensure!(!std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_symlink()), "a link, not a file");
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // Opening a pipe without O_NONBLOCK would wait for a writer.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(
        &mut options,
        libc::O_NONBLOCK | if own { libc::O_NOFOLLOW } else { 0 },
    );
    #[cfg(windows)]
    if own {
        std::os::windows::fs::OpenOptionsExt::custom_flags(
            &mut options,
            windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT,
        );
    }
    let file = options.open(path).context("missing or unreadable")?;
    let meta = file.metadata().context("unreadable")?;
    ensure!(meta.is_file(), "not a plain file");
    if own {
        ensure!(single(&file, &meta), "a link to another file");
    }
    ensure!(meta.len() <= MAX_BYTES, "larger than {} MB", MAX_BYTES >> 20);
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).context("unreadable")?;
    ensure!(bytes.len() as u64 <= MAX_BYTES, "larger than {} MB", MAX_BYTES >> 20);
    Ok(bytes)
}

/// Not a hard link, which could be another name for one of the user's files.
#[cfg(unix)]
fn single(_: &std::fs::File, meta: &std::fs::Metadata) -> bool {
    std::os::unix::fs::MetadataExt::nlink(meta) == 1
}

#[cfg(windows)]
fn single(file: &std::fs::File, meta: &std::fs::Metadata) -> bool {
    use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: info is the structure the call fills, and the handle is open.
    let read = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } != 0;
    read && info.nNumberOfLinks == 1 && meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

/// A picture is measured before `probe`, which decodes without a limit, so a header that lies cannot fill memory.
fn probe(path: &Path, kind: AssetKind) -> Result<Asset> {
    let what = if kind == AssetKind::Image { "picture" } else { "sound" };
    let unreadable = || anyhow!("not a {what} Nuzky can read");
    if kind == AssetKind::Image {
        let (width, height) =
            nuzky_engine::media::picture_size(path, DECODE_PIXELS, || false).map_err(|_| unreadable())?;
        let (width, height) = (width as usize, height as usize);
        ensure!(
            width <= MAX_SIDE && height <= MAX_SIDE && width * height <= MAX_PIXELS,
            "{width}×{height} px, more than Nuzky takes ({MAX_SIDE} px a side, 25 megapixels)"
        );
    }
    match nuzky_engine::media::probe(path, new_id()) {
        Ok(asset) if asset.kind == kind => Ok(asset),
        _ => Err(unreadable()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_names_only_plain_files_of_its_own() {
        for name in ["logo.png", "Beep_2.WAV", "end-card.v2.jpeg", "a.flac"] {
            assert!(kind(name).is_ok(), "{name}");
        }
        for name in [
            "",
            "../logo.png",
            "/etc/logo.png",
            "sub/logo.png",
            "sub\\logo.png",
            ".logo.png",
            "..",
            "plugin.js",
            "plugin.json",
            "clip.mp4",
            "logo",
            "C:logo.png",
            "logo .png",
        ] {
            assert!(kind(name).is_err(), "{name}");
        }
        assert!(kind(&format!("{}.png", "a".repeat(57))).is_err());
    }
}

//! Pictures and videos the user attaches to a message in the AI panel: from a file they chose, or
//! pasted, which Nuzky keeps in its own data since the clipboard has no file to point at.

use crate::{CmdResult, err, store, thumbs};
use nuzky_engine::model::{Asset, AssetKind};
use nuzky_engine::thumbnail::{ImageKind, RenderedThumbnail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    Image,
    Video,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub path: String,
    pub kind: AttachmentKind,
    pub name: String,
    /// A small PNG data URL the panel shows.
    #[serde(default)]
    pub preview: Option<String>,
}

/// About what the models look at closely; a larger picture only costs more of the user's plan.
const MODEL_SIDE: u32 = 1568;
const PREVIEW_HEIGHT: f32 = 128.0;

fn attach(path: &Path, name: String) -> CmdResult<Attachment> {
    let asset = nuzky_engine::media::probe(path, String::new())
        .map_err(|_| format!("{name} isn't a picture or video Nuzky can read."))?;
    let kind = match asset.kind {
        AssetKind::Image => AttachmentKind::Image,
        AssetKind::Video => AttachmentKind::Video,
        AssetKind::Audio => return Err(format!("{name} is only sound. Attach a picture or a video.")),
    };
    let preview = thumbs::picture(&store::cache_dir(), &asset, PREVIEW_HEIGHT).unwrap_or_else(|error| {
        log::warn!("No preview of the attachment {}: {error:#}", path.display());
        None
    });
    Ok(Attachment { path: path.to_string_lossy().into_owned(), kind, name, preview })
}

/// A file the user chose with +.
#[tauri::command]
pub async fn agent_attach_file(path: String) -> CmdResult<Attachment> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(path);
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        attach(&path, name)
    })
    .await
    .map_err(err)?
}

/// What the user pasted, as the request's body; `x-name` is its file name, URI-encoded.
#[tauri::command]
pub async fn agent_attach_pasted(request: tauri::ipc::Request<'_>) -> CmdResult<Attachment> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Paste the picture again.".into());
    };
    let name = request
        .headers()
        .get("x-name")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| percent_encoding::percent_decode_str(v).decode_utf8().ok().map(String::from))
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Pasted image.png".into());
    // FFmpeg tells the format by the content; the extension only names the file.
    let extension = Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.len() <= 8 && e.bytes().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("png")
        .to_ascii_lowercase();
    let dir = store::data_dir().join("attachments");
    std::fs::create_dir_all(&dir).map_err(err)?;
    let path = dir.join(format!("{}.{extension}", uuid::Uuid::new_v4()));
    std::fs::write(&path, bytes).map_err(err)?;
    let kept = path.clone();
    let attached = tauri::async_runtime::spawn_blocking(move || attach(&kept, name)).await.map_err(err)?;
    if attached.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    attached
}

/// The picture as the agent gets it: a JPEG no larger than the models look at, turned the way Nuzky shows
/// it, in the cache under the file it was made from, so sending it again reuses it.
pub fn model_image(path: &Path) -> anyhow::Result<PathBuf> {
    use nuzky_session::hash::{FNV_OFFSET, hash_bytes};
    let meta = std::fs::metadata(path)?;
    let mtime = meta.modified()?.duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let mut hash = FNV_OFFSET;
    hash_bytes(&mut hash, path.as_os_str().as_encoded_bytes());
    hash_bytes(&mut hash, &meta.len().to_le_bytes());
    hash_bytes(&mut hash, &mtime.to_le_bytes());
    let out = store::cache_dir().join("attachments").join(format!("{hash:016x}.jpg"));
    if out.is_file() {
        return Ok(out);
    }
    let cache = store::cache_dir();
    let asset: Asset = nuzky_engine::media::probe(path, String::new())?;
    let (rgba, width, height) = thumbs::frame(&cache, &asset, asset.height.max(1) as f32, MODEL_SIDE)?
        .ok_or_else(|| anyhow::anyhow!("No picture"))?;
    let rendered = RenderedThumbnail {
        width,
        height,
        rgba,
        hidden: Vec::new(),
        bounds: Vec::new(),
        frame_bounds: [[[0.0; 2]; 4]; 2],
    };
    let bytes = nuzky_engine::thumbnail::encode(&rendered, ImageKind::Jpeg)?;
    std::fs::create_dir_all(out.parent().unwrap())?;
    let part = out.with_extension(format!("part-{}", uuid::Uuid::new_v4()));
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, &out)?;
    Ok(out)
}

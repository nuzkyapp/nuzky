//! The user's LUT library: `.cube` files copied into Nuzky's own folder (`color::luts_dir`), each named by the hash
//! of its bytes, so a project names a LUT by an id that is never a path. `index.json` keeps the names they were
//! imported under, in the order they were imported.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use base64::Engine as _;
use nuzky_engine::color::{self, Lut3d};
use nuzky_engine::model::LogProfile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::{AppState, CmdResult, err, thumbs};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LutInfo {
    pub id: String,
    pub name: String,
}

/// Imports, removals and reads of the index one at a time.
static INDEX: Mutex<()> = Mutex::new(());

fn dir() -> Result<PathBuf, String> {
    color::luts_dir().ok_or_else(|| "Nuzky can't find its data folder to keep LUTs in.".to_owned())
}

fn read_index(dir: &Path) -> Vec<LutInfo> {
    let listed: Vec<LutInfo> =
        std::fs::read(dir.join("index.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    listed.into_iter().filter(|l| color::lut_file(dir, &l.id).is_some_and(|f| f.is_file())).collect()
}

fn write_index(dir: &Path, list: &[LutInfo]) -> std::io::Result<()> {
    let part = dir.join(format!(".nuzky-part-{}-index.json", std::process::id()));
    std::fs::write(&part, serde_json::to_vec_pretty(list)?)?;
    std::fs::rename(&part, dir.join("index.json"))
}

/// Copies the `.cube` file at `path` into the library, once its LUT reads; the same file imported again keeps the
/// name it came with first.
pub fn import(dir: &Path, path: &Path) -> Result<LutInfo, String> {
    let text = color::read_cube_text(path)?;
    color::parse_cube(&text)?;
    let hash = Sha256::digest(text.as_bytes());
    let id: String = hash[..8].iter().map(|b| format!("{b:02x}")).collect();
    let stem = path.file_stem().map(|s| s.to_string_lossy().trim().to_owned()).unwrap_or_default();
    let name: String = if stem.is_empty() { "LUT".into() } else { stem.chars().take(255).collect() };
    let _turn = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't make Nuzky's LUT folder: {e}."))?;
    let file = color::lut_file(dir, &id).expect("a hash is an id");
    if !file.is_file() {
        let part = dir.join(format!(".nuzky-part-{}-{id}.cube", std::process::id()));
        std::fs::write(&part, &text)
            .and_then(|()| std::fs::rename(&part, &file))
            .map_err(|e| format!("Couldn't copy the LUT into Nuzky's folder: {e}."))?;
    }
    let mut list = read_index(dir);
    let info = match list.iter().find(|l| l.id == id) {
        Some(known) => known.clone(),
        None => {
            let info = LutInfo { id, name };
            list.push(info.clone());
            write_index(dir, &list).map_err(|e| format!("Couldn't save the list of LUTs: {e}."))?;
            info
        }
    };
    Ok(info)
}

#[tauri::command]
pub async fn list_luts() -> CmdResult<Vec<LutInfo>> {
    let dir = dir()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _turn = INDEX.lock().unwrap_or_else(|e| e.into_inner());
        read_index(&dir)
    })
    .await
    .map_err(err)
}

/// A paused preview draws again, so a clip whose LUT just came or went shows as the export will.
fn redraw(app: &AppHandle) {
    app.state::<AppState>().engine.send(crate::engine::Msg::Redraw);
}

#[tauri::command]
pub async fn import_lut(app: AppHandle, path: String) -> CmdResult<LutInfo> {
    log::info!(target: "action", "import a LUT");
    let dir = dir()?;
    let info = tauri::async_runtime::spawn_blocking(move || import(&dir, Path::new(&path))).await.map_err(err)??;
    redraw(&app);
    Ok(info)
}

/// Moves the LUT's file to the Trash and takes it off the list; clips that use it draw without it.
#[tauri::command]
pub async fn remove_lut(app: AppHandle, id: String) -> CmdResult<()> {
    log::info!(target: "action", "remove a LUT");
    let dir = dir()?;
    let removed = tauri::async_runtime::spawn_blocking(move || {
        let _turn = INDEX.lock().unwrap_or_else(|e| e.into_inner());
        let file = color::lut_file(&dir, &id).ok_or("That isn't one of Nuzky's LUTs.")?;
        if file.is_file() {
            trash::delete(&file).map_err(|e| format!("Couldn't move the LUT to the Trash: {e}."))?;
        }
        let list: Vec<LutInfo> = read_index(&dir).into_iter().filter(|l| l.id != id).collect();
        write_index(&dir, &list).map_err(|e| format!("Couldn't save the list of LUTs: {e}."))
    })
    .await
    .map_err(err)?;
    redraw(&app);
    removed
}

/// A look a Filters tile shows: a log profile, then a LUT of the library at a strength.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Look {
    #[serde(default)]
    log: LogProfile,
    lut: Option<String>,
    #[serde(default = "full")]
    strength: f32,
}

fn full() -> f32 {
    1.0
}

/// Each LUT read once: an id names the bytes of its file.
fn cached(dir: &Path, id: &str) -> Option<Arc<Lut3d>> {
    static READ: OnceLock<Mutex<HashMap<String, Arc<Lut3d>>>> = OnceLock::new();
    let read = READ.get_or_init(Default::default);
    if let Some(lut) = read.lock().unwrap().get(id) {
        return Some(lut.clone());
    }
    let lut = Arc::new(color::read_cube(&color::lut_file(dir, id)?).ok()?);
    read.lock().unwrap().insert(id.to_owned(), lut.clone());
    Some(lut)
}

/// The asset's thumbnail picture in each look, as PNG data URLs, `None` for a look whose LUT is not in the
/// library; the tiles of the Filters tab show them beside the picture as recorded.
#[tauri::command]
pub async fn color_previews(
    app: AppHandle,
    asset_id: String,
    looks: Vec<Look>,
) -> CmdResult<Option<Vec<Option<String>>>> {
    let state = app.state::<AppState>();
    let Some(asset) = state.project()?.asset(&asset_id).cloned() else { return Ok(None) };
    let cache = state.cache_dir.clone();
    let dir = color::luts_dir();
    tauri::async_runtime::spawn_blocking(move || {
        let Some((rgba, w, h)) = thumbs::frame(&cache, &asset, 120.0, 240).map_err(err)? else { return Ok(None) };
        let previews = looks
            .iter()
            .map(|look| {
                let lut = match &look.lut {
                    Some(id) => Some(cached(dir.as_deref()?, id)?),
                    None => None,
                };
                let mut picture = rgba.clone();
                color::apply_to_rgba(
                    &mut picture,
                    look.log,
                    lut.as_deref().map(|l| (l, look.strength.clamp(0.0, 1.0))),
                );
                png_url(&picture, w, h)
            })
            .collect();
        Ok(Some(previews))
    })
    .await
    .map_err(err)?
}

fn png_url(rgba: &[u8], w: u32, h: u32) -> Option<String> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.write_header().ok()?.write_image_data(rgba).ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> String {
        let mut text = String::from("LUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    text += &format!("{r} {g} {b}\n");
                }
            }
        }
        text
    }

    #[test]
    fn importing_copies_the_lut_under_its_hash_and_keeps_its_name() {
        let root = std::env::temp_dir().join(format!("nuzky-luts-{}", std::process::id()));
        let (picked, dir) = (root.join("picked"), root.join("luts"));
        std::fs::create_dir_all(&picked).unwrap();
        let file = picked.join("Teal Orange.cube");
        std::fs::write(&file, identity()).unwrap();
        let info = import(&dir, &file).unwrap();
        assert_eq!(info.name, "Teal Orange");
        assert!(color::is_lut_id(&info.id));
        assert_eq!(std::fs::read_to_string(dir.join(format!("{}.cube", info.id))).unwrap(), identity());
        // The same file under another name is the same LUT.
        let again = picked.join("Copy.cube");
        std::fs::copy(&file, &again).unwrap();
        assert_eq!(import(&dir, &again).unwrap(), info);
        assert_eq!(read_index(&dir), [info]);
        // A broken file says why and leaves the library as it was.
        let broken = picked.join("Broken.cube");
        std::fs::write(&broken, "LUT_3D_SIZE 2\n0 0 0\n").unwrap();
        assert!(import(&dir, &broken).unwrap_err().contains("cut short"));
        assert_eq!(read_index(&dir).len(), 1);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "only the LUT and the index");
        std::fs::remove_dir_all(&root).unwrap();
    }
}

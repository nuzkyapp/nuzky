//! The user's own caption styles (My styles), kept beside the projects for the app and agents, and the list of every
//! text and caption style an agent can apply.

use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use nuzky_engine::edit::{CaptionPreset, TextPreset, caption_preset, caption_presets, text_presets};
use nuzky_engine::model::{Canvas, TextStyle};
use serde_json::{Value, json};

fn path() -> Result<PathBuf> {
    Ok(nuzky_engine::data_dir()
        .context("STORE_UNAVAILABLE: no data directory for caption styles")?
        .join("caption-styles.json"))
}

fn normalized(name: &str) -> String {
    name.trim().replace(['_', '-'], " ").to_lowercase()
}

fn name_available(name: &str, styles: &[CaptionPreset]) -> Result<()> {
    ensure!((1..=40).contains(&name.chars().count()), "INVALID_ARGUMENTS: a style name needs 1 to 40 characters");
    ensure!(
        caption_preset(name).is_none() && !styles.iter().any(|style| normalized(&style.name) == normalized(name)),
        "NAME_TAKEN: a caption style named {name:?} already exists"
    );
    Ok(())
}

fn validate(style: &CaptionPreset) -> Result<()> {
    // Packages work across canvases; edits enforce the bounds of the actual canvas when applied.
    nuzky_session::text_style(&style.style, (7680, 7680)).map_err(|error| {
        anyhow::anyhow!("INVALID_ARGUMENTS: {}", error.to_string().trim_start_matches("INVALID_PROJECT: "))
    })?;
    ensure!(
        [&style.anim_in, &style.anim_out]
            .into_iter()
            .flatten()
            .all(|animation| (0..=10_000_000).contains(&animation.duration_us)),
        "INVALID_ARGUMENTS: caption animations last from 0 to 10 seconds"
    );
    Ok(())
}

pub fn list() -> Result<Vec<CaptionPreset>> {
    let bytes = match std::fs::read(path()?) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error).context("STORE_UNREADABLE: cannot read your caption styles"),
    };
    serde_json::from_slice(&bytes).context("STORE_UNREADABLE: your caption styles file is damaged")
}

fn write(styles: Vec<CaptionPreset>) -> Result<Vec<CaptionPreset>> {
    let path = path()?;
    std::fs::create_dir_all(path.parent().unwrap()).context("STORE_WRITE_FAILED: cannot save your caption styles")?;
    nuzky_session::write_json_atomic(&path, &styles).context("STORE_WRITE_FAILED: cannot save your caption styles")?;
    Ok(styles)
}

pub fn save(mut style: CaptionPreset) -> Result<Vec<CaptionPreset>> {
    let mut styles = list()?;
    style.name = style.name.trim().into();
    name_available(&style.name, &styles)?;
    validate(&style)?;
    styles.push(style);
    write(styles)
}

pub fn rename(from: &str, to: &str) -> Result<Vec<CaptionPreset>> {
    let mut styles = list()?;
    let index = styles
        .iter()
        .position(|style| normalized(&style.name) == normalized(from))
        .with_context(|| format!("UNKNOWN_STYLE: no caption style named {from:?}"))?;
    let mut style = styles.remove(index);
    style.name = to.trim().into();
    name_available(&style.name, &styles)?;
    styles.insert(index, style);
    write(styles)
}

pub fn delete(name: &str) -> Result<Vec<CaptionPreset>> {
    let mut styles = list()?;
    let index = styles
        .iter()
        .position(|style| normalized(&style.name) == normalized(name))
        .with_context(|| format!("UNKNOWN_STYLE: no caption style named {name:?}"))?;
    styles.remove(index);
    write(styles)
}

pub fn find(name: &str) -> Result<CaptionPreset> {
    if let Some(preset) = caption_preset(name) {
        return Ok(preset.clone());
    }
    let styles = list()?;
    if let Some(style) = styles.iter().find(|style| normalized(&style.name) == normalized(name)) {
        return Ok(style.clone());
    }
    let names: Vec<_> = caption_presets()
        .iter()
        .map(|preset| preset.name.to_lowercase().replace(' ', "_"))
        .chain(styles.into_iter().map(|style| style.name))
        .collect();
    anyhow::bail!("INVALID_ARGUMENTS: unknown style_preset {name:?}; use one of {}", names.join(", "))
}

/// Every built-in title and caption style and My styles, saved like them for 1080p, with their sizes for `canvas`, for
/// `text_styles`.
pub fn text_styles(canvas: &Canvas) -> Result<Value> {
    let scaled = |mut style: TextStyle| {
        style.scale_pixels(canvas.unit());
        style
    };
    let titles: Vec<_> =
        text_presets().iter().map(|p| TextPreset { style: scaled(p.style.clone()), ..p.clone() }).collect();
    let captions: Vec<_> =
        caption_presets().iter().map(|p| CaptionPreset { style: scaled(p.style.clone()), ..p.clone() }).collect();
    // A damaged My styles file leaves the built-in styles usable and says why.
    let (mine, error) = match list() {
        Ok(mine) => (mine.into_iter().map(|p| CaptionPreset { style: scaled(p.style.clone()), ..p }).collect(), None),
        Err(error) => (vec![], Some(format!("{error:#}"))),
    };
    let mut result = json!({"titles": titles, "captions": captions, "my_styles": mine});
    if let Some(error) = error {
        result["my_styles_error"] = json!(error);
    }
    Ok(result)
}

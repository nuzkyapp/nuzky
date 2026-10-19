//! Fonts of text that this computer lacks, so the renderer draws that text in Inter.

use std::collections::BTreeSet;
use std::sync::{LazyLock, Mutex};

use nuzky_engine::{
    Project,
    edit::EditCmd,
    model::{Clip, ClipContent, TextStyle},
    text::{DEFAULT_FAMILY, TextRenderer},
};

/// The renderer's own font lookup, loaded once on the first font asked about.
static RENDERER: LazyLock<Mutex<TextRenderer>> = LazyLock::new(|| Mutex::new(TextRenderer::new()));

fn text_style(clip: &Clip) -> Option<&TextStyle> {
    match &clip.content {
        ClipContent::Text { style, .. } => Some(style),
        _ => None,
    }
}

/// Font families of the project's text clips and cover texts.
pub fn families(project: &Project) -> impl Iterator<Item = &str> {
    let clips = project.tracks.iter().flat_map(|track| &track.clips).filter_map(text_style);
    let covers = project.thumbnails.iter().flat_map(|thumb| thumb.texts.iter().map(|text| &text.style));
    clips.chain(covers).filter_map(|style| style.font_family.as_deref())
}

/// Font families the edit gives text, so a retry of the same edits names the same ones.
pub fn edit_families(edit: &EditCmd) -> Vec<&str> {
    if let EditCmd::RestyleTrack { style: Some(patch), .. } = edit {
        return patch.font_family.iter().flatten().map(String::as_str).collect();
    }
    let styles: Vec<&TextStyle> = match edit {
        EditCmd::AddText { style, .. }
        | EditCmd::AddCaptions { style, .. }
        | EditCmd::ReplaceCaptions { style, .. } => {
            vec![style]
        }
        EditCmd::UpdateClip { style, .. } => style.iter().collect(),
        EditCmd::PasteClips { clips, .. } => clips.iter().filter_map(|copied| text_style(&copied.clip)).collect(),
        EditCmd::PasteAttributes { source, .. } => text_style(&source.clip).into_iter().collect(),
        EditCmd::SetThumbnail { thumbnail } => thumbnail.texts.iter().map(|text| &text.style).collect(),
        _ => Vec::new(),
    };
    styles.into_iter().filter_map(|style| style.font_family.as_deref()).collect()
}

/// The families this computer does not have, each once, so their text is drawn in Inter.
pub fn missing<'a>(families: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let families: BTreeSet<&str> = families.into_iter().collect();
    if families.is_empty() {
        return Vec::new();
    }
    let renderer = RENDERER.lock().unwrap();
    families.into_iter().filter(|family| !renderer.has_family(family)).map(str::to_owned).collect()
}

/// One warning per family this computer does not have.
pub fn warnings<'a>(families: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    missing(families)
        .into_iter()
        .map(|family| {
            format!(
                "Font \"{family}\" is not installed on this computer; its text is drawn in {DEFAULT_FAMILY}. Install it or pick an installed font."
            )
        })
        .collect()
}

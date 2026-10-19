//! CapCut drafts: the folder CapCut keeps each project in, read from its timeline with serde alone,
//! draft_content.json on Windows and draft_info.json on macOS. Tracks hold segments that name their
//! material by `material_id`, and every time is in microseconds. Read as CapCut 8.3 to 9.5 for macOS
//! write them. JianYing, CapCut's Chinese edition, encrypts its drafts: such a draft is recognised and
//! refused, never decrypted.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    ImportedTimeline, Keyframe, MediaClip, Read, TextClip, TextShadow, TextStyle, Track, TrackKind, Transform,
    Transition, TransitionKind, local_path,
};

/// The timeline files of a draft, Windows' first.
const TIMELINES: [&str; 2] = ["draft_content.json", "draft_info.json"];
/// Times further from zero than this, eleven days, are no timeline's.
const MAX_US: f64 = 1e12;
/// Pixels of text per unit of CapCut's text size, on a frame 1080 pixels across its shorter side:
/// size 12 at 152% draws text 93 pixels high, measured on two reels CapCut exported.
const PX_PER_SIZE: f32 = 5.1;
/// The folders searched for a copy of missing media, and how many entries at most.
const COPY_DEPTH: usize = 3;
const COPY_ENTRIES: usize = 5_000;

pub const ENCRYPTED: &str = "JianYing encrypts its drafts, so Nuzky cannot read this one. Learn from the exported video and its recording instead.";
/// The same, said where a draft is being opened as a project.
pub const ENCRYPTED_OPEN: &str = "JianYing encrypts its drafts, so Nuzky cannot open this one. Export the video from JianYing and start a project from it instead.";

/// Where CapCut keeps its drafts on this computer: under the local app data on Windows, under Movies
/// in the home folder elsewhere.
pub fn default_folder() -> Option<PathBuf> {
    let base = if cfg!(windows) { dirs::data_local_dir() } else { dirs::home_dir().map(|h| h.join("Movies")) };
    base.map(|b| b.join("CapCut").join("User Data").join("Projects").join("com.lveditor.draft"))
}

/// The timeline file of the draft in `folder`, the newer one when it has both.
pub fn timeline_file(folder: &Path) -> Option<PathBuf> {
    let modified = |p: &PathBuf| std::fs::metadata(p).ok().filter(|m| m.is_file()).and_then(|m| m.modified().ok());
    TIMELINES.iter().map(|name| folder.join(name)).filter_map(|p| Some((modified(&p)?, p))).max().map(|(_, p)| p)
}

/// A file CapCut writes its timeline to.
pub fn is_timeline(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| TIMELINES.iter().any(|t| t.eq_ignore_ascii_case(n)))
}

/// A draft as CapCut lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub path: PathBuf,
    pub name: String,
    /// Milliseconds since 1970.
    pub modified_ms: i64,
    /// None when the draft does not say.
    pub duration_us: Option<i64>,
}

#[derive(Default, Deserialize)]
struct Meta {
    #[serde(default)]
    draft_name: String,
    #[serde(default)]
    tm_draft_modified: Option<i64>,
    #[serde(default)]
    tm_duration: Option<i64>,
}

fn meta(folder: &Path) -> Meta {
    let path = folder.join("draft_meta_info.json");
    let small = std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= 1 << 20);
    small.then(|| std::fs::read(&path).ok()).flatten().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// The drafts in `folder`, newest first, or the draft `folder` is. Drafts with nothing on their
/// timeline are left out.
pub fn drafts(folder: &Path) -> Vec<Draft> {
    let draft = |path: PathBuf| -> Option<Draft> {
        let timeline = timeline_file(&path)?;
        let meta = meta(&path);
        let modified_ms = match meta.tm_draft_modified {
            Some(us) if us > 0 => us / 1000,
            _ => std::fs::metadata(&timeline)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as i64),
        };
        let name = match meta.draft_name.trim() {
            "" => path.file_name()?.to_string_lossy().into_owned(),
            name => name.to_owned(),
        };
        Some(Draft { path, name, modified_ms, duration_us: meta.tm_duration })
    };
    if timeline_file(folder).is_some() {
        return draft(folder.to_path_buf()).into_iter().collect();
    }
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut out: Vec<Draft> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| draft(e.path()))
        .filter(|d| d.duration_us != Some(0))
        .collect();
    out.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms).then_with(|| a.name.cmp(&b.name)));
    out
}

#[derive(Deserialize)]
struct Content {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    fps: Option<f64>,
    #[serde(default)]
    canvas_config: Option<Canvas>,
    tracks: Vec<Value>,
    materials: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Deserialize)]
struct Canvas {
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct CapTrack {
    #[serde(rename = "type", default)]
    kind: String,
    /// Bits: 1 muted, 2 hidden, 4 locked.
    #[serde(default)]
    attribute: i64,
    #[serde(default)]
    segments: Vec<Value>,
}

#[derive(Clone, Copy, Deserialize)]
struct Range {
    start: f64,
    duration: f64,
}

impl Range {
    fn us(self) -> Result<(i64, i64)> {
        let (start, duration) = (us(self.start)?, us(self.duration)?);
        ensure!(duration >= 0, "a part lasts {duration} µs");
        Ok((start, duration))
    }
}

fn us(value: f64) -> Result<i64> {
    ensure!(value.is_finite() && value.abs() <= MAX_US, "the time {value} µs is out of range");
    Ok(value.round() as i64)
}

#[derive(Deserialize)]
struct Segment {
    #[serde(default)]
    material_id: String,
    target_timerange: Range,
    #[serde(default)]
    source_timerange: Option<Range>,
    #[serde(default)]
    speed: Option<f64>,
    #[serde(default)]
    volume: Option<f32>,
    #[serde(default)]
    reverse: bool,
    #[serde(default)]
    visible: Option<bool>,
    #[serde(default)]
    clip: Option<Clip>,
    #[serde(default)]
    common_keyframes: Vec<Value>,
    #[serde(default)]
    extra_material_refs: Vec<String>,
    #[serde(default)]
    enable_adjust: Option<bool>,
    #[serde(default)]
    enable_smart_color_adjust: bool,
}

#[derive(Default, Deserialize)]
struct Clip {
    #[serde(default)]
    scale: Option<Xy>,
    #[serde(default)]
    transform: Option<Xy>,
    #[serde(default)]
    rotation: Option<f32>,
    #[serde(default)]
    alpha: Option<f32>,
    #[serde(default)]
    flip: Option<Flip>,
}

#[derive(Default, Deserialize)]
struct Flip {
    #[serde(default)]
    horizontal: bool,
    #[serde(default)]
    vertical: bool,
}

#[derive(Clone, Copy, Default, Deserialize)]
struct Xy {
    #[serde(default)]
    x: f32,
    #[serde(default)]
    y: f32,
}

impl Clip {
    /// CapCut moves by halves of the frame, with y up; Nuzky by whole frames, with y down.
    fn transform(&self) -> Transform {
        let at = self.transform.unwrap_or_default();
        Transform {
            scale: self.scale.map_or(1.0, |s| s.x),
            x: at.x / 2.0,
            y: -at.y / 2.0,
            rotation: self.rotation.unwrap_or(0.0),
        }
    }
}

#[derive(Deserialize)]
struct CapKeyframes {
    #[serde(default)]
    property_type: String,
    #[serde(default)]
    keyframe_list: Vec<CapKeyframe>,
}

#[derive(Deserialize)]
struct CapKeyframe {
    time_offset: f64,
    #[serde(default)]
    values: Vec<f32>,
}

/// What was not read, counted by kind.
#[derive(Default)]
struct Unread(BTreeMap<String, usize>);

impl Unread {
    fn add(&mut self, what: impl Into<String>) {
        *self.0.entry(what.into()).or_default() += 1;
    }

    fn lines(self) -> Vec<String> {
        self.0.into_iter().map(|(what, n)| if n == 1 { what } else { format!("{what} ({n} times)") }).collect()
    }
}

/// Reads a draft's timeline from its bytes; `folder` is the draft's, where its own copies of media are.
pub fn read(bytes: &[u8], folder: &Path) -> Result<Read> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let root: Value = match serde_json::from_slice(bytes) {
        Ok(root) => root,
        // An encrypted timeline is not JSON from its first byte on.
        Err(_)
            if !bytes.trim_ascii_start().is_empty()
                && !bytes.trim_ascii_start().starts_with(b"{")
                && !bytes.trim_ascii_start().starts_with(b"[") =>
        {
            bail!(ENCRYPTED)
        }
        Err(error) => return Err(error).context("Its timeline is broken"),
    };
    let content: Content = Content::deserialize(&root).context("It is not a CapCut draft")?;
    let materials = Materials::new(&content.materials);
    let canvas = content.canvas_config.filter(|c| c.width > 0 && c.height > 0);
    let name = match meta(folder).draft_name.trim() {
        "" => content
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        name => name.to_owned(),
    };
    let mut out = ImportedTimeline {
        name,
        fps: content.fps.filter(|f| f.is_finite() && *f > 0.0),
        size: canvas.map(|c| (c.width, c.height)),
        ..ImportedTimeline::default()
    };
    // Text sizes are on a frame 1080 pixels across its shorter side.
    let text_scale = canvas.map_or(1.0, |c| c.width.min(c.height) as f32 / 1080.0);
    let canvas_width = canvas.map(|c| c.width as f32);
    let mut unread = Unread::default();
    let mut copies = Copies { folder, found: HashMap::new() };
    for (index, value) in content.tracks.iter().enumerate() {
        let track: CapTrack = match CapTrack::deserialize(value) {
            Ok(track) => track,
            Err(_) => {
                unread.add("A broken track");
                continue;
            }
        };
        let kind = match track.kind.as_str() {
            "video" => TrackKind::Video,
            "audio" => TrackKind::Audio,
            "text" | "subtitle" => TrackKind::Video,
            "effect" => {
                unread.add("Effects");
                continue;
            }
            "filter" => {
                unread.add("Filters");
                continue;
            }
            "adjust" => {
                unread.add("Adjustment layers");
                continue;
            }
            "sticker" => {
                unread.add("Stickers");
                continue;
            }
            other => {
                unread.add(format!("A {} track", if other.is_empty() { "nameless" } else { other }));
                continue;
            }
        };
        let mut read = Track {
            name: format!("{} {}", track.kind, index + 1),
            kind,
            hidden: track.attribute & 2 != 0,
            muted: track.attribute & 1 != 0,
            clips: Vec::new(),
            texts: Vec::new(),
        };
        let mut segments: Vec<Segment> = Vec::new();
        for value in &track.segments {
            match Segment::deserialize(value) {
                Ok(segment) => segments.push(segment),
                Err(error) => unread.add(format!("A broken part of a {} track: {error}", track.kind)),
            }
        }
        segments.sort_by(|a, b| a.target_timerange.start.total_cmp(&b.target_timerange.start));
        let mut transition: Option<Transition> = None;
        for segment in &segments {
            let before = transition.take();
            let part = || -> Result<()> {
                if segment.visible == Some(false) {
                    return Ok(());
                }
                let Some((category, material)) = materials.get(&segment.material_id) else {
                    unread.add("A part whose material is missing");
                    return Ok(());
                };
                match (track.kind.as_str(), category) {
                    ("text" | "subtitle", "texts") => {
                        if let Some(text) = text_clip(segment, material, text_scale, canvas_width, &mut unread)? {
                            settings(segment, &materials, &mut unread);
                            if !segment.common_keyframes.is_empty() {
                                unread.add("Keyframes on texts");
                            }
                            read.texts.push(text);
                        }
                    }
                    ("video", "videos") | ("audio", "audios") => {
                        let clip = media_clip(segment, material, &materials, &mut copies, &mut unread)?;
                        if let Some(mut clip) = clip {
                            // Into this clip from the one it follows straight on.
                            let joins = read.clips.last().is_some_and(|c| (c.end_us() - clip.start_us).abs() < 1_000);
                            clip.transition_in = before.filter(|_| joins);
                            settings(segment, &materials, &mut unread);
                            read.clips.push(clip);
                        }
                        transition = segment.extra_material_refs.iter().find_map(|id| materials.transition(id));
                    }
                    (_, "drafts") => unread.add("A compound clip"),
                    (_, "stickers") => unread.add("Stickers"),
                    (_, "text_templates") => unread.add("Text templates"),
                    (kind, category) => {
                        unread.add(format!("A part Nuzky does not read on a {kind} track ({category})"))
                    }
                }
                Ok(())
            };
            if let Err(error) = part() {
                unread.add(format!("A part of a {} track: {error:#}", track.kind));
            }
        }
        out.tracks.push(read);
    }
    Ok(Read { timeline: out, unread: unread.lines() })
}

/// Every material by its id, with the kind of list it is in.
struct Materials<'a>(HashMap<&'a str, (&'a str, &'a Value)>);

impl<'a> Materials<'a> {
    fn new(all: &'a BTreeMap<String, Value>) -> Self {
        let mut out = HashMap::new();
        for (category, list) in all {
            for material in list.as_array().into_iter().flatten() {
                if let Some(id) = material.get("id").and_then(Value::as_str) {
                    out.insert(id, (category.as_str(), material));
                }
            }
        }
        Self(out)
    }

    fn get(&self, id: &str) -> Option<(&'a str, &'a Value)> {
        self.0.get(id).copied()
    }

    /// The transition this material is, out of the clip whose material it is.
    fn transition(&self, id: &str) -> Option<Transition> {
        let (category, material) = self.get(id)?;
        if category != "transitions" {
            return None;
        }
        let name = material.get("name").and_then(Value::as_str).unwrap_or("").to_owned();
        let whole = us(material.get("duration").and_then(Value::as_f64).unwrap_or(0.0)).ok()?.max(0);
        Some(Transition { kind: transition_kind(&name), name, before_us: whole / 2, after_us: whole - whole / 2 })
    }

    /// A speed that changes along the clip, which CapCut calls a curve.
    fn curved(&self, refs: &[String]) -> bool {
        refs.iter().filter_map(|id| self.get(id)).any(|(category, material)| {
            category == "speeds"
                && material.get("mode").and_then(Value::as_i64) == Some(1)
                && material.get("curve_speed").is_some_and(|c| !c.is_null())
        })
    }

    fn refs<'r>(&'r self, refs: &'r [String], category: &'r str) -> impl Iterator<Item = &'a Value> + 'r {
        refs.iter().filter_map(|id| self.get(id)).filter(move |(c, _)| *c == category).map(|(_, m)| m)
    }
}

/// What CapCut does to a clip that Nuzky does not, each said once per clip that has it on.
fn settings(segment: &Segment, materials: &Materials, unread: &mut Unread) {
    let refs = &segment.extra_material_refs;
    let number = |m: &Value, key: &str| m.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let on = |m: &Value, key: &str| m.get(key).and_then(Value::as_bool).unwrap_or(false);
    // Every clip keeps its adjustments at 0, and automatic colour stays off until switched on.
    let kind = |m: &Value| m.get("type").and_then(Value::as_str).unwrap_or("").to_owned();
    let adjusted: Vec<String> =
        materials.refs(refs, "effects").filter(|m| number(m, "value") != 0.0).map(kind).collect();
    if adjusted.iter().any(|k| k == "filter") {
        unread.add("Filters");
    }
    let automatic = segment.enable_smart_color_adjust && adjusted.iter().any(|k| k == "smart_color_adjust");
    let by_hand =
        segment.enable_adjust != Some(false) && adjusted.iter().any(|k| k != "filter" && k != "smart_color_adjust");
    if automatic || by_hand {
        unread.add("Color adjustments");
    }
    if materials.refs(refs, "realtime_denoises").any(|m| on(m, "is_denoise")) {
        unread.add("Noise reduction");
    }
    let lists = [
        ("video_effects", "Effects"),
        ("filters", "Filters"),
        ("audio_effects", "Voice effects"),
        ("masks", "Masks"),
        ("common_mask", "Masks"),
        ("chromas", "Chroma key"),
    ];
    for (category, what) in lists {
        if materials.refs(refs, category).next().is_some() {
            unread.add(what);
        }
    }
    if materials.refs(refs, "loudnesses").any(|m| on(m, "enable")) {
        unread.add("Normalize loudness");
    }
    if materials.refs(refs, "vocal_beautifys").any(|m| on(m, "enable")) {
        unread.add("Enhance voice");
    }
    if materials.refs(refs, "vocal_separations").any(|m| number(m, "choice") != 0.0) {
        unread.add("Vocal isolation");
    }
    let animated = |m: &Value| m.get("animations").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
    if materials.refs(refs, "material_animations").any(animated) {
        unread.add("Animations");
    }
    let custom = |m: &Value| {
        m.get("type").and_then(Value::as_str).is_some_and(|t| t != "canvas_color")
            || m.get("color").and_then(Value::as_str).is_some_and(|c| !c.is_empty() && !c.starts_with("#000000"))
    };
    if materials.refs(refs, "canvases").any(custom) {
        unread.add("Canvas backgrounds");
    }
    if let Some(clip) = &segment.clip {
        if clip.alpha.is_some_and(|a| a < 1.0) {
            unread.add("Opacity");
        }
        if clip.flip.as_ref().is_some_and(|f| f.horizontal || f.vertical) {
            unread.add("Flipped clips");
        }
    }
}

/// The most common of CapCut's transitions by the name it gives them, all others as one.
fn transition_kind(name: &str) -> TransitionKind {
    let name = name.to_lowercase();
    let has = |part: &str| name.contains(part);
    if has("mix") || has("dissolve") || name == "fade" {
        TransitionKind::Dissolve
    } else if has("black") && has("fade") {
        TransitionKind::FadeBlack
    } else if has("zoom") || has("pull in") || has("pull out") {
        TransitionKind::Zoom
    } else if has("slide") || has("push") {
        TransitionKind::Slide
    } else {
        TransitionKind::Other
    }
}

/// Copies of media kept in the draft's own folder, found once per file name.
struct Copies<'a> {
    folder: &'a Path,
    found: HashMap<String, Option<PathBuf>>,
}

impl Copies<'_> {
    /// The local file a material's path names. CapCut names files in the draft's folder from a
    /// placeholder for it; media missing where the path says may have a copy in the draft's folder,
    /// as when the draft was copied from another computer.
    fn resolve(&mut self, raw: &str) -> Option<PathBuf> {
        let named = match raw.strip_prefix("##_draftpath_placeholder_").and_then(|r| r.split_once("_##")) {
            Some((_, rest)) => {
                local_path(&self.folder.join(rest.trim_start_matches(['/', '\\'])).to_string_lossy(), self.folder)
            }
            None => local_path(raw, self.folder),
        };
        if named.as_ref().is_some_and(|p| p.exists()) {
            return named;
        }
        let Some(name) = raw.rsplit(['/', '\\']).next().filter(|n| !n.is_empty() && *n != "." && *n != "..") else {
            return named;
        };
        let folder = self.folder;
        self.found.entry(name.to_owned()).or_insert_with(|| copy_in(folder, name)).clone().or(named)
    }
}

/// A file of this name in `folder` or a few folders below it, never through a link.
fn copy_in(folder: &Path, name: &str) -> Option<PathBuf> {
    let mut folders = vec![(folder.to_path_buf(), 0)];
    let mut seen = 0;
    while let Some((dir, depth)) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > COPY_ENTRIES {
                return None;
            }
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_file() && entry.file_name().to_str() == Some(name) {
                return Some(entry.path());
            }
            if kind.is_dir() && depth < COPY_DEPTH {
                folders.push((entry.path(), depth + 1));
            }
        }
    }
    None
}

fn media_clip(
    segment: &Segment,
    material: &Value,
    materials: &Materials,
    copies: &mut Copies,
    unread: &mut Unread,
) -> Result<Option<MediaClip>> {
    let (start_us, duration_us) = segment.target_timerange.us()?;
    let still = material.get("type").and_then(Value::as_str) == Some("photo");
    // A photo shows the same picture all along, whatever range of its own it names.
    let (source_in_us, source_us) = match segment.source_timerange {
        Some(range) if !still => range.us()?,
        _ if still => (0, duration_us),
        _ => bail!("it has no source range"),
    };
    ensure!(source_in_us >= 0, "it starts before its media");
    if segment.reverse {
        unread.add("A clip played backwards");
        return Ok(None);
    }
    let mut speed = if still { 1.0 } else { segment.speed.unwrap_or(1.0) };
    ensure!(speed.is_finite() && speed > 0.0, "its speed is {speed}");
    if materials.curved(&segment.extra_material_refs) && duration_us > 0 && !still {
        unread.add("A speed ramp, read as its average speed");
        speed = source_us as f64 / duration_us as f64;
    }
    let media = material.get("path").and_then(Value::as_str).unwrap_or("").to_owned();
    let path = copies.resolve(&media);
    let name = ["material_name", "name"]
        .iter()
        .find_map(|k| material.get(*k).and_then(Value::as_str).filter(|n| !n.is_empty()))
        .map_or_else(|| media.rsplit(['/', '\\']).next().unwrap_or("").to_owned(), str::to_owned);
    if path.is_none() {
        unread.add(format!("Media that is not a file on this computer: {name}"));
        return Ok(None);
    }
    if cropped(material) {
        unread.add("Cropping");
    }
    let size = |key: &str| material.get(key).and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok());
    let media_size = size("width").zip(size("height")).filter(|(w, h)| *w > 0 && *h > 0);
    let fade = |key: &str| -> i64 {
        let fades = materials.refs(&segment.extra_material_refs, "audio_fades");
        fades.filter_map(|m| m.get(key)?.as_f64()).find_map(|v| us(v).ok()).unwrap_or(0).max(0)
    };
    let transform = segment.clip.as_ref().map(Clip::transform);
    let keyframes = match &segment.clip {
        Some(clip) => keyframes(segment, clip, source_in_us, speed, unread),
        None => Vec::new(),
    };
    Ok(Some(MediaClip {
        name,
        media,
        path,
        source_in_us,
        source_out_us: source_in_us + source_us,
        start_us,
        duration_us,
        speed,
        volume: Some(segment.volume.unwrap_or(1.0)),
        transform,
        keyframes,
        transition_in: None,
        still,
        media_size,
        fade_in_us: fade("fade_in_duration"),
        fade_out_us: fade("fade_out_duration"),
    }))
}

/// A part of the picture cut away: CapCut keeps the corners left, from 0 to 1 of the picture.
fn cropped(material: &Value) -> bool {
    let Some(crop) = material.get("crop") else { return false };
    let at = |key: &str, whole: f64| crop.get(key).and_then(Value::as_f64).unwrap_or(whole);
    [("upper_left_x", 0.0), ("upper_left_y", 0.0), ("lower_right_x", 1.0), ("lower_right_y", 1.0)]
        .iter()
        .any(|(key, whole)| (at(key, *whole) - whole).abs() > 1e-4)
}

/// Keyframes of where the picture sits, at every time any of its properties has one. CapCut times
/// them in the media's own time, from the start of the file; each property goes straight from one
/// to the next.
fn keyframes(segment: &Segment, clip: &Clip, source_in_us: i64, speed: f64, unread: &mut Unread) -> Vec<Keyframe> {
    let mut lists: BTreeMap<&str, Vec<(i64, f32)>> = BTreeMap::new();
    for value in &segment.common_keyframes {
        let Ok(list) = CapKeyframes::deserialize(value) else {
            unread.add("Broken keyframes");
            continue;
        };
        let property = match list.property_type.as_str() {
            "KFTypePositionX" => "x",
            "KFTypePositionY" => "y",
            "KFTypeScaleX" | "UNIFORM_SCALE" => "scale",
            "KFTypeScaleY" => "scale_y",
            "KFTypeRotation" => "rotation",
            other => {
                let what = match other.trim_start_matches("KFType") {
                    "Alpha" => "opacity",
                    "Volume" => "volume",
                    "Saturation" | "Contrast" | "Brightness" => "colour",
                    rest => rest,
                };
                unread.add(format!("Keyframes on the {what}"));
                continue;
            }
        };
        let points: Vec<(i64, f32)> = list
            .keyframe_list
            .iter()
            .filter_map(|k| {
                let value = *k.values.first()?;
                let at = ((k.time_offset - source_in_us as f64) / speed).round();
                (value.is_finite() && at.is_finite() && at.abs() <= MAX_US).then_some((at as i64, value))
            })
            .collect();
        if !points.is_empty() {
            lists.entry(property).or_default().extend(points);
        }
    }
    // Scale is one number in Nuzky: the width's, or the height's when only that moves.
    if let Some(y) = lists.remove("scale_y") {
        lists.entry("scale").or_insert(y);
    }
    for points in lists.values_mut() {
        points.sort_by_key(|(at, _)| *at);
    }
    let mut times: Vec<i64> = lists.values().flatten().map(|(at, _)| *at).collect();
    times.sort_unstable();
    times.dedup();
    let base = clip.transform();
    let value_at = |property: &str, at: i64, fixed: f32| -> f32 {
        let Some(points) = lists.get(property) else { return fixed };
        match points.iter().position(|(t, _)| *t > at) {
            Some(0) => points[0].1,
            None => points[points.len() - 1].1,
            Some(i) => {
                let ((t0, v0), (t1, v1)) = (points[i - 1], points[i]);
                v0 + (v1 - v0) * (at - t0) as f32 / (t1 - t0) as f32
            }
        }
    };
    times
        .into_iter()
        .map(|at| Keyframe {
            at_us: at,
            transform: Transform {
                scale: value_at("scale", at, base.scale),
                // Keyframed positions are in CapCut's terms like the clip's own.
                x: lists.get("x").map_or(base.x, |_| value_at("x", at, 0.0) / 2.0),
                y: lists.get("y").map_or(base.y, |_| -value_at("y", at, 0.0) / 2.0),
                rotation: value_at("rotation", at, base.rotation),
            },
        })
        .collect()
}

#[derive(Deserialize)]
struct TextContent {
    #[serde(default)]
    text: String,
    #[serde(default)]
    styles: Vec<Value>,
}

fn text_clip(
    segment: &Segment,
    material: &Value,
    text_scale: f32,
    canvas_width: Option<f32>,
    unread: &mut Unread,
) -> Result<Option<TextClip>> {
    let (start_us, duration_us) = segment.target_timerange.us()?;
    let raw = material.get("content").and_then(Value::as_str).unwrap_or("");
    let Ok(content) = serde_json::from_str::<TextContent>(raw) else {
        unread.add("A text in a format Nuzky does not read");
        return Ok(None);
    };
    let first = content.styles.first();
    let style = |key: &str| first.and_then(|s| s.get(key)).filter(|v| !v.is_null());
    let font_title = material.get("font_title").and_then(Value::as_str).filter(|t| !t.is_empty() && *t != "none");
    let font_path = style("font").and_then(|f| f.get("path")).and_then(Value::as_str).unwrap_or("");
    let (font, heavy) = font(font_title, font_path);
    let size = style("size").and_then(Value::as_f64).or_else(|| material.get("font_size").and_then(Value::as_f64));
    let color =
        style("fill").and_then(solid).or_else(|| material.get("text_color").and_then(Value::as_str).and_then(hex));
    let size_px = size.map_or(0.0, |s| s as f32 * PX_PER_SIZE * text_scale);
    let outline = style("strokes")
        .and_then(|s| s.get(0))
        .and_then(|s| Some((solid(s)?, s.get("width")?.as_f64()? as f32 * size_px)))
        .filter(|(_, width)| *width > 0.0);
    // CapCut wraps lines at a share of the frame's width, before the text's own scale.
    let number = |key: &str| material.get(key).and_then(Value::as_f64);
    let wrap = number("line_max_width")
        .filter(|w| w.is_finite() && *w > 0.0 && *w <= 4.0)
        .zip(canvas_width)
        .map(|(share, width)| share as f32 * width);
    let shadow = text_shadow(style("shadows").and_then(|s| s.get(0)), material, size_px);
    // CapCut keeps both spacings as shares of the size; its usual line spacing is 0.02.
    let spacing = |key: &str, usual: f64| number(key).filter(|v| v.is_finite()).map_or(0.0, |v| (v - usual) as f32);
    let text_style = (size_px > 0.0).then(|| TextStyle {
        font,
        size: size_px,
        color: color.unwrap_or_else(|| "#ffffff".into()),
        bold: heavy || style("bold").and_then(Value::as_bool).unwrap_or(false),
        outline,
        wrap,
        shadow,
        letter_spacing: spacing("letter_spacing", 0.0).clamp(-0.5, 2.0),
        line_spacing: spacing("line_spacing", 0.02).clamp(-0.6, 3.0),
    });
    let depth = material.get("shadow_thickness_projection_enable").and_then(Value::as_bool).unwrap_or(false);
    let looks = [
        (depth, "Text shadows with depth, drawn flat"),
        (number("background_style").is_some_and(|s| s != 0.0), "Text backgrounds"),
        (number("alignment").is_some_and(|a| a != 1.0), "Text aligned left or right"),
        (number("typesetting").is_some_and(|t| t != 0.0), "Vertical text"),
        (number("text_alpha").is_some_and(|a| a < 1.0), "Opacity"),
        (
            content.styles.iter().filter_map(|s| s.get("fill").and_then(solid)).collect::<HashSet<_>>().len() > 1,
            "Words in other colours",
        ),
    ];
    for (_, what) in looks.iter().filter(|(has, _)| *has) {
        unread.add(*what);
    }
    Ok(Some(TextClip {
        text: content.text,
        start_us,
        duration_us,
        transform: segment.clip.as_ref().map(Clip::transform),
        style: text_style,
        caption: material.get("type").and_then(Value::as_str) == Some("subtitle"),
    }))
}

/// `#rrggbb` of a fill or a stroke of one colour, which CapCut gives as red, green and blue from 0 to 1.
fn solid(paint: &Value) -> Option<String> {
    let rgb = paint.get("content")?.get("solid")?.get("color")?.as_array()?;
    let channel =
        |i: usize| rgb.get(i)?.as_f64().filter(|v| v.is_finite()).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    Some(format!("#{:02x}{:02x}{:02x}", channel(0)?, channel(1)?, channel(2)?))
}

/// The shadow of a text of `size` pixels, from its first style's shadow or else the material's.
/// CapCut's Blur percentage is `diffuse` × 6 and `shadow_smoothing` / 3; Nuzky blurs half the size
/// at 100 %. Its Distance runs to 100, a whole size away.
fn text_shadow(styled: Option<&Value>, material: &Value, size: f32) -> Option<TextShadow> {
    let number = |value: Option<&Value>| value.and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v as f32);
    let own = |key: &str| number(material.get(key));
    let (color, opacity, blur, distance, angle) = match styled {
        Some(s) => (
            solid(s),
            number(s.get("alpha")),
            number(s.get("diffuse")).map(|d| d * 6.0),
            number(s.get("distance")),
            number(s.get("angle")),
        ),
        None if material.get("has_shadow").and_then(Value::as_bool) == Some(true) => (
            material.get("shadow_color").and_then(Value::as_str).and_then(hex),
            own("shadow_alpha"),
            own("shadow_smoothing").map(|s| s / 3.0),
            own("shadow_distance"),
            own("shadow_angle"),
        ),
        None => return None,
    };
    Some(TextShadow {
        color: color.unwrap_or_else(|| "#000000".into()),
        opacity: opacity.unwrap_or(0.9).clamp(0.0, 1.0),
        blur: blur.unwrap_or(0.15).clamp(0.0, 1.0) * size / 2.0,
        distance: distance.unwrap_or(5.0).clamp(0.0, 100.0) / 100.0 * size,
        angle: angle.unwrap_or(-45.0).clamp(-360.0, 360.0),
    })
}

/// `#rrggbb` of CapCut's `#rrggbb` or `#rrggbbaa`.
fn hex(color: &str) -> Option<String> {
    let rgb = color.strip_prefix('#')?.get(..6)?;
    rgb.chars().all(|c| c.is_ascii_hexdigit()).then(|| format!("#{}", rgb.to_ascii_lowercase()))
}

/// The font family a text is drawn in and whether its style is bold, from the name CapCut shows or
/// the file it draws with. None for CapCut's own default font.
fn font(title: Option<&str>, path: &str) -> (Option<String>, bool) {
    let file = path.rsplit(['/', '\\']).next().unwrap_or("");
    let system = path.contains("SystemFont");
    let name = match title {
        Some(title) => title,
        None if system || file.is_empty() => return (None, false),
        None => file,
    };
    let stem = name
        .rsplit_once('.')
        .filter(|(_, ext)| matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc"))
        .map_or(name, |(stem, _)| stem);
    const WEIGHTS: &[&str] =
        &["regular", "medium", "book", "light", "thin", "italic", "semibold", "bold", "extrabold", "black", "heavy"];
    let (family, weight) = match stem.rsplit_once('-') {
        Some((family, weight))
            if WEIGHTS.iter().any(|w| weight.to_ascii_lowercase().replace(' ', "").starts_with(w)) =>
        {
            (family, weight.to_ascii_lowercase())
        }
        _ => (stem, String::new()),
    };
    // A downloaded font is a file named "font" in a folder named by its hash.
    if family.trim().is_empty() || family.eq_ignore_ascii_case("font") {
        return (None, false);
    }
    let heavy = ["bold", "black", "heavy"].iter().any(|w| weight.contains(w));
    (Some(family.trim().to_owned()), heavy)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The start of a CapCut 9.5 draft for macOS, cut down to three clips of a recording, three
    /// captions and a song, its texts, paths, fonts and ids replaced.
    const MAC: &[u8] = include_bytes!("../fixtures/capcut-mac/draft_info.json");

    fn mac() -> Value {
        serde_json::from_slice(MAC).unwrap()
    }

    fn read_value(draft: &Value, folder: &Path) -> Result<Read> {
        read(&serde_json::to_vec(draft).unwrap(), folder)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// A folder of its own for a test, empty.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nuzky-capcut-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_the_cut_its_zoom_and_its_captions_from_a_macos_draft() {
        let read = read(MAC, Path::new("/drafts/Reel")).unwrap();
        let t = &read.timeline;
        assert!(read.unread.is_empty(), "{:?}", read.unread);
        assert_eq!((t.name.as_str(), t.fps, t.size), ("Reel", Some(30.0), Some((1080, 1920))));
        let main = &t.tracks[0];
        assert_eq!(main.kind, TrackKind::Video);
        let times: Vec<_> =
            main.clips.iter().map(|c| (c.start_us, c.duration_us, c.source_in_us, c.source_out_us, c.speed)).collect();
        assert_eq!(
            times,
            [
                (0, 2_200_000, 8_433_333, 10_633_333, 1.0),
                (2_200_000, 2_266_666, 10_800_000, 13_066_666, 1.0),
                (4_466_666, 5_333_334, 23_766_666, 29_100_000, 1.0),
            ]
        );
        assert_eq!(main.clips[0].path.as_deref(), Some(Path::new("/Users/creator/Movies/talk.mov")));
        assert_eq!(main.clips[0].volume, Some(1.0));
        assert_eq!((main.clips[0].media_size, main.clips[0].still), (Some((1080, 1920)), false));
        // CapCut moves the picture up by 0.055 of half the frame: Nuzky moves it up by a quarter of that.
        let framed = main.clips[0].transform.unwrap();
        assert!(close(framed.scale, 1.3) && close(framed.x, 0.0) && close(framed.y, -0.02734375), "{framed:?}");
        // A slow zoom from 1.1 to 1.3 over the whole clip, keyed in the media's own time.
        let zoom: Vec<_> = main.clips[0].keyframes.iter().map(|k| (k.at_us, k.transform.scale)).collect();
        assert_eq!(zoom.iter().map(|z| z.0).collect::<Vec<_>>(), [0, 2_200_000]);
        assert!(close(zoom[0].1, 1.1) && close(zoom[1].1, 1.3), "{zoom:?}");
        assert!(main.clips[1].keyframes.is_empty() && close(main.clips[1].transform.unwrap().scale, 1.4));

        let texts = &t.tracks[1].texts;
        let said: Vec<_> = texts.iter().map(|c| (c.text.as_str(), c.start_us, c.duration_us)).collect();
        assert_eq!(
            said,
            [("Hello there", 0, 733_333), ("two words", 733_333, 233_333), ("and three more", 966_666, 500_000)]
        );
        assert!(texts.iter().all(|t| t.caption), "CapCut made them from the speech");
        let place = texts[0].transform.unwrap();
        assert!(close(place.scale, 1.52) && close(place.y, 0.144_423_8), "{place:?}");
        let style = texts[0].style.clone().unwrap();
        assert_eq!((style.font.as_deref(), style.color.as_str(), style.bold), (Some("Montserrat"), "#ffffff", false));
        // Size 12 is 61 pixels of a 1080 pixel wide frame before the caption's scale of 152%.
        assert!(close(style.size, 61.2), "{style:?}");
        let (outline, width) = style.outline.unwrap();
        assert!(outline == "#000000" && (width - 3.672).abs() < 0.01, "{width}");
        // Lines wrap at 82% of the frame's width, before the caption's own scale.
        assert!(close(style.wrap.unwrap(), 885.6), "{:?}", style.wrap);
        // The material's shadow: black at 41 %, Blur 23 % (smoothing 0.69), Distance 5, -45°.
        let shadow = style.shadow.clone().unwrap();
        assert_eq!(shadow.color, "#000000");
        assert!(close(shadow.opacity, 0.41) && close(shadow.angle, -45.0), "{shadow:?}");
        assert!(close(shadow.blur, 0.23 * 61.2 / 2.0) && close(shadow.distance, 0.05 * 61.2), "{shadow:?}");
        // CapCut's usual line spacing is 0.02 of the size: no extra space.
        assert!(close(style.letter_spacing, 0.0) && close(style.line_spacing, 0.0));

        let song = &t.tracks[2].clips[0];
        assert_eq!(t.tracks[2].kind, TrackKind::Audio);
        assert!(song.path.as_ref().unwrap().ends_with("song.mp3") && (song.volume.unwrap() - 0.1187).abs() < 1e-3);
        assert_eq!(t.duration_us(), 91_900_000);
    }

    #[test]
    fn a_styled_shadow_and_spacing_come_over_and_a_text_without_shadow_has_none() {
        let mut draft = mac();
        let texts = draft["materials"]["texts"].as_array_mut().unwrap();
        // As CapCut writes the creator's Reel look: the style's own shadow wins over the material's.
        let mut content: Value = serde_json::from_str(texts[0]["content"].as_str().unwrap()).unwrap();
        content["styles"][0]["shadows"] = json!([{"alpha": 0.5, "angle": 135.0, "diffuse": 0.038333334,
            "distance": 20.0, "content": {"render_type": "solid", "solid": {"color": [1.0, 0.0, 0.0]}}}]);
        texts[0]["content"] = content.to_string().into();
        texts[0]["letter_spacing"] = json!(0.1);
        texts[0]["line_spacing"] = json!(0.12);
        texts[1]["has_shadow"] = json!(false);
        texts[2]["shadow_thickness_projection_enable"] = json!(true);
        let read = read_value(&draft, Path::new("/drafts/Reel")).unwrap();
        let styles: Vec<_> = read.timeline.tracks[1].texts.iter().map(|t| t.style.clone().unwrap()).collect();
        let shadow = styles[0].shadow.clone().unwrap();
        assert_eq!(shadow.color, "#ff0000");
        assert!(close(shadow.opacity, 0.5) && close(shadow.angle, 135.0), "{shadow:?}");
        assert!(close(shadow.blur, 0.23 * 61.2 / 2.0) && close(shadow.distance, 0.2 * 61.2), "{shadow:?}");
        assert!(close(styles[0].letter_spacing, 0.1) && close(styles[0].line_spacing, 0.1), "{:?}", styles[0]);
        assert_eq!(styles[1].shadow, None);
        assert_eq!(read.unread, ["Text shadows with depth, drawn flat"]);
    }

    #[test]
    fn speed_plays_more_of_the_media_and_moves_its_keyframes() {
        let mut draft = mac();
        let clip = &mut draft["tracks"][0]["segments"][2];
        clip["speed"] = json!(2.0);
        clip["source_timerange"]["duration"] = json!(10_666_668);
        clip["common_keyframes"] = json!([{"property_type": "KFTypeScaleX", "keyframe_list": [
            {"time_offset": 23_766_666, "values": [1.0], "curveType": "Line"},
            {"time_offset": 27_766_666, "values": [1.5], "curveType": "Line"},
        ]}, {"property_type": "KFTypePositionY", "keyframe_list": [
            {"time_offset": 25_766_666, "values": [0.2], "curveType": "Line"},
        ]}]);
        let read = read_value(&draft, Path::new("/d")).unwrap();
        let fast = &read.timeline.tracks[0].clips[2];
        assert_eq!((fast.duration_us, fast.source_out_us, fast.speed), (5_333_334, 34_433_334, 2.0));
        // 4 s of media at twice the speed is 2 s of the cut; the move up lands halfway.
        let keys: Vec<_> = fast.keyframes.iter().map(|k| (k.at_us, k.transform.scale, k.transform.y)).collect();
        assert_eq!(keys.iter().map(|k| k.0).collect::<Vec<_>>(), [0, 1_000_000, 2_000_000]);
        assert!(close(keys[1].1, 1.25) && close(keys[0].2, -0.1) && close(keys[2].2, -0.1), "{keys:?}");

        // A speed ramp is read as its average speed, and said.
        let mut draft = mac();
        draft["materials"]["speeds"] =
            json!([{"id": "ramp", "type": "speed", "mode": 1, "speed": 1.0, "curve_speed": {"name": "Montage"}}]);
        let clip = &mut draft["tracks"][0]["segments"][1];
        clip["extra_material_refs"] = json!(["ramp"]);
        clip["source_timerange"]["duration"] = json!(4_533_332);
        let read = read_value(&draft, Path::new("/d")).unwrap();
        assert_eq!(read.timeline.tracks[0].clips[1].speed, 2.0);
        assert_eq!(read.unread, ["A speed ramp, read as its average speed"]);
    }

    #[test]
    fn transitions_are_named_and_the_unknown_one_is_another_transition() {
        let mut draft = mac();
        draft["materials"]["transitions"] = json!([
            {"id": "t1", "type": "transition", "name": "Cube Rotate", "duration": 1_000_000, "is_overlap": true},
            {"id": "t2", "type": "transition", "name": "Mix", "duration": 466_666, "is_overlap": true},
        ]);
        // A transition belongs to the clip before the cut.
        draft["tracks"][0]["segments"][0]["extra_material_refs"] = json!(["t1"]);
        draft["tracks"][0]["segments"][1]["extra_material_refs"] = json!(["t2"]);
        let read = read_value(&draft, Path::new("/d")).unwrap();
        let into: Vec<_> = read.timeline.tracks[0].clips.iter().map(|c| c.transition_in.clone()).collect();
        assert_eq!(into[0], None);
        assert_eq!(
            into[1],
            Some(Transition {
                name: "Cube Rotate".into(),
                kind: TransitionKind::Other,
                before_us: 500_000,
                after_us: 500_000
            })
        );
        assert_eq!(into[2].as_ref().map(|t| (t.kind, t.before_us)), Some((TransitionKind::Dissolve, 233_333)));
        for (name, kind) in [
            ("Dissolve II", TransitionKind::Dissolve),
            ("Black Fade", TransitionKind::FadeBlack),
            ("Pull in", TransitionKind::Zoom),
            ("Zoom Transition", TransitionKind::Zoom),
            ("Slide", TransitionKind::Slide),
            ("Woosh", TransitionKind::Other),
            ("Black smoke", TransitionKind::Other),
        ] {
            assert_eq!(transition_kind(name), kind, "{name}");
        }
    }

    #[test]
    fn an_encrypted_draft_is_refused_with_what_to_do_instead() {
        let encrypted = b"Hk3vQ0bS8vZ2Yz4uXw9pR1tL7mN5kJ2hG6fD0sA8qW3eR5tY7uI9oP1aS3dF5gH7jK9lZ1xC3vB5nM7";
        for bytes in [encrypted.as_slice(), &[0x8a, 0x01, 0xff, 0x7b, 0x22]] {
            let error = read(bytes, Path::new("/d")).unwrap_err().to_string();
            assert_eq!(error, ENCRYPTED, "{bytes:?}");
        }
        assert!(ENCRYPTED.contains("JianYing") && ENCRYPTED.contains("Learn from the exported video"));
        // JSON that is merely broken, or not a draft, says so instead.
        for broken in [b"{\"tracks\": [".as_slice(), b"", b"  \n"] {
            let error = read(broken, Path::new("/d")).unwrap_err();
            assert!(format!("{error:#}").starts_with("Its timeline is broken"), "{error:#}");
        }
        let otio = read(br#"{"OTIO_SCHEMA": "Timeline.1"}"#, Path::new("/d")).unwrap_err();
        assert_eq!(otio.to_string(), "It is not a CapCut draft");
        // A byte order mark is no encryption.
        let marked = [b"\xEF\xBB\xBF".as_slice(), MAC].concat();
        assert_eq!(read(&marked, Path::new("/d")).unwrap().timeline.tracks.len(), 3);
        let deep = "[".repeat(100_000);
        assert!(read(deep.as_bytes(), Path::new("/d")).is_err(), "deep nesting is an error, not a stack overflow");
    }

    #[test]
    fn hidden_and_muted_tracks_and_photos_come_along_and_learning_leaves_them_out() {
        let mut draft = mac();
        let mut overlay = draft["tracks"][0].clone();
        overlay["attribute"] = json!(2);
        let tracks = draft["tracks"].as_array_mut().unwrap();
        tracks.push(overlay);
        tracks.push(json!({"type": "effect", "attribute": 0, "segments": []}));
        draft["tracks"][2]["attribute"] = json!(1);
        draft["tracks"][0]["segments"][1]["reverse"] = json!(true);
        draft["materials"]["videos"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "still", "type": "photo", "path": "/Users/creator/Pictures/logo.png"}));
        draft["tracks"][0]["segments"][2]["material_id"] = json!("still");
        draft["tracks"][0]["segments"][0]["material_id"] = json!("gone");
        let read = read_value(&draft, Path::new("/d")).unwrap();
        let tracks = &read.timeline.tracks;
        assert_eq!(tracks.len(), 4, "only the effects are left out");
        // The photo keeps its place and shows its one picture all along, whatever range it names.
        let photo = &tracks[0].clips[..];
        assert_eq!(photo.len(), 1);
        assert!(photo[0].still && photo[0].speed == 1.0 && photo[0].source_in_us == 0, "{photo:?}");
        assert_eq!(
            (photo[0].start_us, photo[0].duration_us, photo[0].source_out_us),
            (4_466_666, 5_333_334, 5_333_334)
        );
        assert!(tracks[3].hidden && !tracks[0].hidden && tracks[3].clips.len() == 3);
        // A muted track keeps its clips' own volume, so turning its sound back on brings them back as they were.
        assert!(tracks[2].muted && tracks[2].clips[0].volume.is_some_and(|v| v > 0.0));
        assert_eq!(read.unread, ["A clip played backwards", "A part whose material is missing", "Effects"]);

        // Learning hears and sees only what the finished video plays.
        let played = read.played();
        let tracks = &played.timeline.tracks;
        assert_eq!(tracks.len(), 3);
        assert!(tracks[0].clips.is_empty());
        assert_eq!(tracks[2].clips[0].volume, Some(0.0));
        assert_eq!(
            played.unread,
            ["A clip played backwards", "A hidden track", "A part whose material is missing", "A photo", "Effects"]
        );
    }

    #[test]
    fn media_is_found_in_copies_inside_the_draft_and_never_on_the_network() {
        let dir = folder("copies");
        std::fs::create_dir_all(dir.join("Resources/combination")).unwrap();
        std::fs::create_dir_all(dir.join("assets/video")).unwrap();
        std::fs::write(dir.join("Resources/combination/part.mp4"), b"").unwrap();
        std::fs::write(dir.join("assets/video/broll.mp4"), b"").unwrap();
        let mut draft = mac();
        let videos = draft["materials"]["videos"].as_array_mut().unwrap();
        let mut media = |id: &str, path: &str| {
            let mut m = videos[0].clone();
            m["id"] = json!(id);
            m["path"] = json!(path);
            m["material_name"] = json!("");
            videos.push(m);
        };
        media(
            "inside",
            "##_draftpath_placeholder_0E685133-18CE-45ED-8CB8-2904A212EC80_##/Resources/combination/part.mp4",
        );
        media("moved", r"C:\Users\someone\Videos\broll.mp4");
        media("shared", r"\\nas\videos\talk.mov");
        let segments = draft["tracks"][0]["segments"].as_array_mut().unwrap();
        for (segment, id) in segments.iter_mut().zip(["inside", "moved", "shared"]) {
            segment["material_id"] = json!(id);
        }
        let read = read_value(&draft, &dir).unwrap();
        let paths: Vec<_> = read.timeline.tracks[0].clips.iter().map(|c| c.path.clone().unwrap()).collect();
        assert_eq!(paths, [dir.join("Resources/combination/part.mp4"), dir.join("assets/video/broll.mp4")]);
        assert_eq!(read.unread, [r"Media that is not a file on this computer: talk.mov"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn drafts_are_listed_newest_first_without_empty_ones() {
        let dir = folder("list");
        let draft = |name: &str, meta: Option<Value>| {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join("draft_info.json"), MAC).unwrap();
            if let Some(meta) = meta {
                std::fs::write(dir.join(name).join("draft_meta_info.json"), meta.to_string()).unwrap();
            }
        };
        draft(
            "0001",
            Some(
                json!({"draft_name": "Older reel", "tm_draft_modified": 1_000_000_000_000i64, "tm_duration": 5_000_000}),
            ),
        );
        draft(
            "0002",
            Some(
                json!({"draft_name": "Newer reel", "tm_draft_modified": 2_000_000_000_000i64, "tm_duration": 7_000_000}),
            ),
        );
        draft(
            "0003",
            Some(json!({"draft_name": "Empty", "tm_draft_modified": 3_000_000_000_000i64, "tm_duration": 0})),
        );
        draft("no meta", None);
        std::fs::create_dir_all(dir.join("not a draft")).unwrap();
        let found: Vec<_> = drafts(&dir).into_iter().map(|d| (d.name, d.duration_us)).collect();
        assert_eq!(
            found[1..],
            [("Newer reel".to_owned(), Some(7_000_000)), ("Older reel".to_owned(), Some(5_000_000))]
        );
        assert_eq!(found[0], ("no meta".to_owned(), None), "dated by its timeline file, written just now");
        let one = drafts(&dir.join("0001"));
        assert_eq!((one.len(), one[0].modified_ms), (1, 1_000_000_000));
        // Reading a draft's folder reads its timeline, named as CapCut lists it.
        let read = crate::read(&dir.join("0002")).unwrap();
        assert_eq!(read.timeline.name, "Newer reel");
        assert!(crate::read(&dir.join("not a draft")).unwrap_err().to_string().contains("is not a CapCut draft"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

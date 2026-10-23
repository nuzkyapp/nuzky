//! A timeline cut in another editor, such as a CapCut draft, as a new Nuzky project. Every clip lands
//! where the editor put it, with its trim, speed, volume, framing and keyframes; texts stay editable
//! text and the transitions Nuzky has keep their cuts. Media missing from this computer keeps its
//! clips' place, without picture or sound, so nothing after them moves. What does not come over is
//! said in plain words, once per kind. The timeline itself is only read, never changed.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use nuzky_engine::edit::{EditCmd, MAX_SPEED, MAX_TRANSITION_US, MIN_SPEED, new_id};
use nuzky_engine::media::probe;
use nuzky_engine::model::{
    Asset, AssetKind, BOX_RADIUS, CAPTIONS_TRACK, Clip, ClipContent, Imported, Keyframe, MAX_LEFT_OUT, Project,
    TextAlign, TextStyle, Track, TrackKind, Transform, Transition, TransitionKind,
};
use nuzky_interchange::{self as timeline, MediaClip, Read, TextClip};

/// Clips this close count as back to back, as editors round their times.
const JOIN_US: i64 = 1_000;

/// The CapCut draft in `draft`, its folder or its timeline file, as a project.
pub fn capcut_draft(draft: &Path, fonts: &[String]) -> Result<Project> {
    let folder = if draft.is_dir() { draft } else { draft.parent().unwrap_or(draft) };
    let file = if draft.is_dir() { timeline::capcut::timeline_file(draft) } else { Some(draft.to_path_buf()) };
    let file = file.filter(|f| timeline::capcut::is_timeline(f)).with_context(|| {
        format!("{} is not a CapCut draft: it has no draft_info.json or draft_content.json", draft.display())
    })?;
    let read = timeline::read(&file).map_err(|error| {
        if error.to_string() == timeline::capcut::ENCRYPTED {
            anyhow::anyhow!(timeline::capcut::ENCRYPTED_OPEN)
        } else {
            error
        }
    })?;
    let modified_ms = timeline::capcut::drafts(folder).first().map_or(0, |d| d.modified_ms.max(0));
    project(read, "CapCut", folder, modified_ms, fonts)
}

/// The project the timeline `read` from `from` makes, not saved yet. `fonts` are the families this
/// computer draws text in; a text in another one is drawn in Nuzky's own until it is installed.
pub fn project(read: Read, editor: &str, from: &Path, modified_ms: i64, fonts: &[String]) -> Result<Project> {
    let Read { timeline, unread } = read;
    let mut left = Left::default();
    let name = match timeline.name.trim() {
        "" => format!("{editor} draft"),
        name => name.to_owned(),
    };
    let mut project = Project::new(name.clone());
    if let Some((width, height)) = timeline.size.filter(|(w, h)| (16..=7680).contains(w) && (16..=7680).contains(h)) {
        (project.canvas.width, project.canvas.height) = (width & !1, height & !1);
    }
    if let Some(fps) = timeline.fps.map(|f| f.round()).filter(|f| (1.0..=240.0).contains(f)) {
        project.canvas.fps = fps as u32;
    }
    // Media that does not play leads the list: it is what a creator notices first.
    let mut missing = Left::default();
    let assets = assets(&timeline, &project, &mut missing);
    let mut tracks: Vec<Track> = Vec::new();
    let mut main: Option<(Vec<Clip>, &timeline::Track)> = None;
    let mut captions = false;
    for track in &timeline.tracks {
        if !track.clips.is_empty() {
            let kind = if track.kind == timeline::TrackKind::Audio { TrackKind::Audio } else { TrackKind::Video };
            let clips: Vec<Clip> =
                track.clips.iter().filter_map(|c| media(c, kind, &assets, &project, &mut left)).collect();
            // The lowest video track is the main one when it runs back to back from the start, as CapCut's does.
            let first_video = kind == TrackKind::Video && main.is_none() && !tracks.iter().any(|t| t.kind == kind);
            if first_video {
                match back_to_back(&clips) {
                    Some(packed) => {
                        main = Some((packed, track));
                        continue;
                    }
                    None => left.add("The main track has gaps, so its clips are on an overlay track"),
                }
            }
            let keep_in_place = kind == TrackKind::Audio
                && clips.iter().all(|c| match &c.content {
                    ClipContent::Media { asset_id, .. } => assets.kind(asset_id) == Some(AssetKind::Audio),
                    ClipContent::Text { .. } => false,
                });
            let name = if kind == TrackKind::Audio { "Audio" } else { "Overlay" };
            lanes(&mut tracks, clips, kind, name, track, keep_in_place, &mut left);
        }
        if !track.texts.is_empty() {
            let clips: Vec<Clip> = track.texts.iter().filter_map(|t| text(t, &project, fonts, &mut left)).collect();
            // Captions made from the speech go where Nuzky makes its own, so regenerating them keeps their look.
            let spoken = !captions && !track.hidden && track.texts.iter().all(|t| t.caption);
            captions |= spoken;
            let name = if spoken { CAPTIONS_TRACK } else { "Text" };
            lanes(&mut tracks, clips, TrackKind::Text, name, track, false, &mut left);
        }
    }
    if let Some((clips, from)) = main {
        let track = &mut project.tracks[0];
        (track.clips, track.hidden, track.muted) = (clips, from.hidden, from.muted);
    }
    project.tracks.extend(tracks);
    project.assets = assets.list;
    for (i, track) in project.tracks.iter_mut().enumerate() {
        for (j, clip) in track.clips.iter_mut().enumerate() {
            if (i > 0 || j == 0) && clip.transition_in.take().is_some() {
                left.add(if i > 0 { "Transitions on overlay tracks" } else { "A transition into the first clip" });
            }
        }
    }
    let mut left_out = missing.lines();
    left_out.extend(unread);
    left_out.extend(left.lines());
    left_out.truncate(MAX_LEFT_OUT);
    project.imported =
        Some(Imported { editor: editor.into(), path: from.display().to_string(), modified_ms, left_out });
    // The engine's own rules for fades and transitions that run longer than their clips, and no empty tracks.
    project.apply(EditCmd::RenameProject { name })?;
    nuzky_session::validate(&project).context("The timeline makes a project Nuzky cannot open")?;
    Ok(project)
}

/// What did not come over, counted by kind.
#[derive(Default)]
struct Left(BTreeMap<String, usize>);

impl Left {
    fn add(&mut self, what: impl Into<String>) {
        *self.0.entry(what.into()).or_default() += 1;
    }

    fn lines(self) -> Vec<String> {
        self.0.into_iter().map(|(what, n)| if n == 1 { what } else { format!("{what} ({n} times)") }).collect()
    }
}

/// The project's media by the file each clip names, one asset per file.
struct Assets {
    list: Vec<Asset>,
    by_path: HashMap<PathBuf, String>,
}

impl Assets {
    fn of(&self, path: &Path) -> Option<&Asset> {
        let id = self.by_path.get(&key(path))?;
        self.list.iter().find(|a| &a.id == id)
    }

    fn kind(&self, id: &str) -> Option<AssetKind> {
        self.list.iter().find(|a| a.id == id).map(|a| a.kind)
    }
}

fn key(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Reads each local file the timeline plays. A file that is not on this computer, or that Nuzky
/// cannot open, still gets an asset shaped as the timeline says, so its clips keep their place.
fn assets(timeline: &timeline::ImportedTimeline, project: &Project, left: &mut Left) -> Assets {
    let mut out = Assets { list: Vec::new(), by_path: HashMap::new() };
    // The stand-ins for files that do not open, with why and how many clips they hold.
    let mut missing: BTreeMap<String, (&str, String, usize)> = BTreeMap::new();
    for track in &timeline.tracks {
        for clip in &track.clips {
            let Some(path) = &clip.path else { continue };
            let key = key(path);
            if let Some(id) = out.by_path.get(&key) {
                if let Some((_, _, clips)) = missing.get_mut(id) {
                    *clips += 1;
                    let asset = out.list.iter_mut().find(|a| &a.id == id).expect("a listed stand-in");
                    asset.duration_us = asset.duration_us.max(clip.source_out_us);
                }
                continue;
            }
            // Only a file is read: a folder, a device or a pipe could hang or never end.
            let read = if path.is_file() { probe(path, new_id()).ok() } else { None };
            let why = if path.is_file() { "Media Nuzky cannot open" } else { "Missing media" };
            let asset = read.unwrap_or_else(|| {
                let name = Some(clip.name.trim()).filter(|n| !n.is_empty()).map_or_else(
                    || path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    str::to_owned,
                );
                let kind = if clip.still {
                    AssetKind::Image
                } else if track.kind == timeline::TrackKind::Audio {
                    AssetKind::Audio
                } else {
                    AssetKind::Video
                };
                let (width, height) = match kind {
                    AssetKind::Audio => (0, 0),
                    _ => clip.media_size.unwrap_or((project.canvas.width, project.canvas.height)),
                };
                let id = new_id();
                missing.insert(id.clone(), (why, name.clone(), 1));
                Asset {
                    id,
                    name,
                    path: path.to_string_lossy().into_owned(),
                    kind,
                    duration_us: if kind == AssetKind::Image { 0 } else { clip.source_out_us.max(0) },
                    width,
                    height,
                    fps: if kind == AssetKind::Video { f64::from(project.canvas.fps) } else { 0.0 },
                    // A file put back later plays its sound; one that is there but does not open would only
                    // fail each time Nuzky prepares its sound.
                    has_audio: kind != AssetKind::Image && !path.is_file(),
                    rotation: 0,
                    mirror: false,
                    credit: None,
                    part: None,
                }
            });
            out.by_path.insert(key, asset.id.clone());
            out.list.push(asset);
        }
    }
    for (why, name, clips) in missing.into_values() {
        left.add(if clips == 1 { format!("{why}: {name}") } else { format!("{why}: {name} ({clips} clips)") });
    }
    out
}

fn transform(t: Option<timeline::Transform>) -> Transform {
    let Some(t) = t else { return Transform::default() };
    let finite = |v: f32, or: f32| if v.is_finite() { v } else { or };
    Transform {
        x: finite(t.x, 0.0),
        y: finite(t.y, 0.0),
        scale: finite(t.scale, 1.0).abs().max(0.001),
        rotation: finite(t.rotation, 0.0),
        ..Transform::default()
    }
}

fn media(clip: &MediaClip, track: TrackKind, assets: &Assets, project: &Project, left: &mut Left) -> Option<Clip> {
    let asset = assets.of(clip.path.as_deref()?)?;
    if clip.duration_us <= 0 {
        return None;
    }
    if track == TrackKind::Video && asset.kind == AssetKind::Audio {
        left.add("Sound files on video tracks");
        return None;
    }
    // A picture FFmpeg reads as something else, such as an iPhone photo made of tiles, would show wrong.
    if clip.still && asset.kind != AssetKind::Image {
        left.add(format!("A picture Nuzky cannot show: {}", clip.name));
        return None;
    }
    let still = asset.kind == AssetKind::Image;
    let mut speed = if still { 1.0 } else { clip.speed as f32 };
    if !(MIN_SPEED..=MAX_SPEED).contains(&speed) {
        left.add(format!("Speeds outside {MIN_SPEED}× to {MAX_SPEED}×, set to the nearest"));
        speed = speed.clamp(MIN_SPEED, MAX_SPEED);
    }
    let mut source_in_us = if still { 0 } else { clip.source_in_us.max(0) };
    if !still {
        // A clip that names a little more than its file has plays its last part instead, at its place.
        let frame = project.frame_duration_us().ceil();
        let fits = |source_in: i64, speed: f32| {
            source_in as f64 + clip.duration_us as f64 * speed as f64 <= asset.duration_us as f64 + frame * speed as f64
        };
        if !fits(source_in_us, speed) {
            source_in_us = (asset.duration_us - (clip.duration_us as f64 * speed as f64).ceil() as i64).max(0);
        }
        if !fits(source_in_us, speed) {
            left.add("A clip longer than its media");
            return None;
        }
    }
    let half = clip.duration_us / 2;
    if clip.fade_in_us > half || clip.fade_out_us > half {
        left.add("Sound fades longer than half their clip, shortened to half");
    }
    let transition_in = clip.transition_in.as_ref().and_then(|t| {
        let kind = match t.kind {
            timeline::TransitionKind::Dissolve => TransitionKind::Dissolve,
            timeline::TransitionKind::FadeBlack => TransitionKind::FadeBlack,
            timeline::TransitionKind::Zoom => TransitionKind::ZoomIn,
            timeline::TransitionKind::Slide => TransitionKind::SlideLeft,
            timeline::TransitionKind::Other => {
                left.add(format!("Transition: {}", if t.name.is_empty() { "without a name" } else { &t.name }));
                return None;
            }
        };
        Some(Transition { kind, duration_us: (t.before_us + t.after_us).clamp(0, MAX_TRANSITION_US) })
    });
    let content = ClipContent::Media {
        asset_id: asset.id.clone(),
        source_in_us,
        volume: clip.volume.filter(|v| v.is_finite()).unwrap_or(1.0).max(0.0),
        transform: transform(clip.transform),
        speed,
        keep_pitch: speed != 1.0,
        adjust: Default::default(),
        fade_in_us: clip.fade_in_us.clamp(0, half),
        fade_out_us: clip.fade_out_us.clamp(0, half),
        clean_voice: false,
        dereverb: 0.0,
        normalize: false,
        leveler: 0.0,
        shape: None,
        duck_db: 0.0,
        background: Default::default(),
    };
    let mut out = Clip::new(new_id(), clip.start_us.max(0), clip.duration_us, content);
    out.keyframes = keyframes(clip);
    out.transition_in = transition_in;
    Some(out)
}

fn keyframes(clip: &MediaClip) -> Vec<Keyframe> {
    let mut out: Vec<Keyframe> = Vec::new();
    for key in &clip.keyframes {
        if out.last().is_none_or(|last| key.at_us > last.t_us) {
            out.push(Keyframe { t_us: key.at_us, transform: transform(Some(key.transform)), ease: Default::default() });
        }
    }
    out
}

fn text(clip: &TextClip, project: &Project, fonts: &[String], left: &mut Left) -> Option<Clip> {
    if clip.duration_us <= 0 {
        return None;
    }
    let style = match &clip.style {
        Some(s) => TextStyle {
            font_family: s.font.clone(),
            font_size: s.size,
            color: s.color.clone(),
            bold: s.bold,
            stroke_width: s.outline.as_ref().map_or(0.0, |(_, width)| *width),
            stroke_color: s.outline.as_ref().map_or_else(|| "#000000".into(), |(color, _)| color.clone()),
            background: None,
            max_width: s.wrap,
            highlight: None,
            keywords: None,
            shadow: s.shadow.as_ref().map(|s| nuzky_engine::model::TextShadow {
                color: s.color.clone(),
                opacity: s.opacity,
                blur: s.blur,
                distance: s.distance,
                angle: s.angle,
            }),
            letter_case: Default::default(),
            letter_spacing: s.letter_spacing,
            line_spacing: s.line_spacing,
            align: TextAlign::Center,
            box_radius: BOX_RADIUS,
        },
        None => crate::params::reel().style,
    };
    if let Some(font) = &style.font_family
        && !fonts.is_empty()
        && !fonts.contains(font)
    {
        left.add(format!("Font not on this computer: {font}"));
    }
    let content = ClipContent::Text {
        text: clip.text.clone(),
        style: style.bounded(&project.canvas),
        transform: transform(clip.transform),
        words: Vec::new(),
    };
    Some(Clip::new(new_id(), clip.start_us.max(0), clip.duration_us, content))
}

/// The clips as Nuzky's magnetic main track holds them, back to back from the start, or none when
/// they leave a gap.
fn back_to_back(clips: &[Clip]) -> Option<Vec<Clip>> {
    let mut clips = clips.to_vec();
    clips.sort_by_key(|c| c.start_us);
    let mut end = 0;
    for clip in &mut clips {
        if (clip.start_us - end).abs() >= JOIN_US {
            return None;
        }
        clip.start_us = end;
        end += clip.duration_us;
    }
    Some(clips)
}

/// Adds the clips of one of the timeline's tracks as tracks of `kind`, as few as hold them without
/// overlap: editors round their times, and a clip a hair over the one before it starts where it ends.
fn lanes(
    tracks: &mut Vec<Track>,
    mut clips: Vec<Clip>,
    kind: TrackKind,
    name: &str,
    from: &timeline::Track,
    keep_in_place: bool,
    left: &mut Left,
) {
    clips.sort_by_key(|c| c.start_us);
    let mut lanes: Vec<Vec<Clip>> = Vec::new();
    for mut clip in clips {
        let end = |lane: &Vec<Clip>| lane.last().map_or(0, Clip::end_us);
        if let Some(lane) = lanes.iter_mut().find(|l| end(l) <= clip.start_us + JOIN_US) {
            let over = end(lane) - clip.start_us;
            if over > 0 && clip.duration_us > over {
                clip.start_us += over;
                clip.duration_us -= over;
                if let ClipContent::Media { source_in_us, speed, .. } = &mut clip.content {
                    *source_in_us += (over as f64 * *speed as f64).round() as i64;
                }
            }
            if clip.start_us >= end(lane) {
                lane.push(clip);
                continue;
            }
        }
        lanes.push(vec![clip]);
    }
    if lanes.len() > 1 {
        left.add("Overlapping clips, put on tracks of their own");
    }
    for clips in lanes {
        let name = if name == CAPTIONS_TRACK && tracks.iter().any(Track::is_captions) { "Text" } else { name };
        tracks.push(Track {
            id: new_id(),
            kind,
            name: name.into(),
            muted: from.muted,
            hidden: from.hidden,
            keep_in_place,
            clips,
        });
    }
}

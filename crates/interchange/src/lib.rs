//! Timelines cut in other editors, read into one neutral model, whichever editor made them. Each
//! format reads its files into an `ImportedTimeline` and says in plain words what it did not read.
//! A timeline file comes from anyone: reading opens only that file and what lies beside it, never
//! the media it names nor the network, and a broken file is an error, never a crash.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

pub mod capcut;
pub mod otio;

/// A larger file is not a timeline.
const MAX_FILE: u64 = 64 << 20;

/// A timeline as another editor cut it. Times are microseconds from the timeline's start.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportedTimeline {
    pub name: String,
    /// Frames per second, when the file says.
    pub fps: Option<f64>,
    /// Width and height in pixels, when the file says.
    pub size: Option<(u32, u32)>,
    /// Bottom to top: the lowest video track is the main one, those above lie over it.
    pub tracks: Vec<Track>,
    pub markers: Vec<Marker>,
}

impl ImportedTimeline {
    pub fn duration_us(&self) -> i64 {
        let ends = self.tracks.iter().flat_map(|t| {
            t.clips.iter().map(MediaClip::end_us).chain(t.texts.iter().map(|c| c.start_us + c.duration_us))
        });
        ends.max().unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub name: String,
    pub kind: TrackKind,
    /// Left out of the finished video.
    pub hidden: bool,
    /// Its clips play no sound, whatever their volume.
    pub muted: bool,
    /// In timeline order.
    pub clips: Vec<MediaClip>,
    pub texts: Vec<TextClip>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaClip {
    pub name: String,
    /// The media as the file names it: a path or an address.
    pub media: String,
    /// The local file it names, a relative path from the timeline file's folder; none for anything
    /// else, such as a web address.
    pub path: Option<PathBuf>,
    /// From the start of the media file.
    pub source_in_us: i64,
    pub source_out_us: i64,
    pub start_us: i64,
    pub duration_us: i64,
    /// Media time per timeline time: 2 plays twice as fast, 0 holds a frame.
    pub speed: f64,
    /// Linear gain, when the file says.
    pub volume: Option<f32>,
    /// Where the picture sits, when the file says.
    pub transform: Option<Transform>,
    /// From the clip's start.
    pub keyframes: Vec<Keyframe>,
    /// Into this clip from the one before it, or from nothing.
    pub transition_in: Option<Transition>,
    /// A still picture, such as a photo, shown for the whole clip.
    pub still: bool,
    /// Width and height of the media's picture, when the file says.
    pub media_size: Option<(u32, u32)>,
    /// How long its sound fades in and out.
    pub fade_in_us: i64,
    pub fade_out_us: i64,
}

impl MediaClip {
    pub fn end_us(&self) -> i64 {
        self.start_us + self.duration_us
    }
}

/// As Nuzky has it: scale 1 fits the frame, x and y move the picture by fractions of the frame,
/// down for y, and rotation turns it clockwise in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub scale: f32,
    pub x: f32,
    pub y: f32,
    pub rotation: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe {
    pub at_us: i64,
    pub transform: Transform,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    /// As the editor calls it, such as SMPTE_Dissolve.
    pub name: String,
    pub kind: TransitionKind,
    /// How long it plays before the cut and after it.
    pub before_us: i64,
    pub after_us: i64,
}

/// The Nuzky transition one looks like, for the most common ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionKind {
    Dissolve,
    FadeBlack,
    Zoom,
    Slide,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextClip {
    pub text: String,
    pub start_us: i64,
    pub duration_us: i64,
    pub transform: Option<Transform>,
    /// How it is drawn, when the file says.
    pub style: Option<TextStyle>,
    /// Words said in the video, as captions made from speech are.
    pub caption: bool,
}

/// As Nuzky draws text; sizes are pixels of the timeline's frame, before the clip's scale.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    /// None for the editor's default font.
    pub font: Option<String>,
    pub size: f32,
    /// `#rrggbb`
    pub color: String,
    pub bold: bool,
    /// Colour and width.
    pub outline: Option<(String, f32)>,
    /// How wide its lines run before they wrap, when the file says.
    pub wrap: Option<f32>,
    pub shadow: Option<TextShadow>,
    /// Extra space after every letter, as a share of the size.
    pub letter_spacing: f32,
    /// Extra space between lines, as a share of the size, on top of a 1.2 line height.
    pub line_spacing: f32,
}

/// A text shadow in the numbers CapCut shows, as Nuzky keeps one; blur and distance follow the size.
#[derive(Clone, Debug, PartialEq)]
pub struct TextShadow {
    /// `#rrggbb`
    pub color: String,
    /// 0 to 1.
    pub opacity: f32,
    /// Blur, 0 to 100 %.
    pub blur: f32,
    /// Distance, 0 to 100.
    pub distance: f32,
    /// -180 to 180 degrees counterclockwise from the right: -45 falls down and to the right.
    pub angle: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub name: String,
    pub start_us: i64,
    pub duration_us: i64,
    pub color: String,
    pub comment: String,
}

/// A timeline file as read: the timeline, and one line for each kind of thing in it that was not read.
#[derive(Clone, Debug)]
pub struct Read {
    pub timeline: ImportedTimeline,
    pub unread: Vec<String>,
}

impl Read {
    /// Only what the finished video plays sound and picture of from its media: hidden tracks and
    /// stills left out and muted tracks silent, each kind said among what was not read.
    pub fn played(mut self) -> Self {
        let hidden = self.timeline.tracks.iter().filter(|t| t.hidden).count();
        self.timeline.tracks.retain(|t| !t.hidden);
        let mut stills = 0;
        for track in &mut self.timeline.tracks {
            stills += track.clips.iter().filter(|c| c.still).count();
            track.clips.retain(|c| !c.still);
            if track.muted {
                track.clips.iter_mut().for_each(|c| c.volume = Some(0.0));
            }
        }
        for (n, what) in [(hidden, "A hidden track"), (stills, "A photo")] {
            match n {
                0 => {}
                1 => self.unread.push(what.into()),
                n => self.unread.push(format!("{what} ({n} times)")),
            }
        }
        self.unread.sort();
        self
    }
}

/// Reads the timeline file at `path`, by its extension, or the CapCut draft in the folder `path`.
pub fn read(path: &Path) -> Result<Read> {
    let meta = std::fs::metadata(path).with_context(|| format!("Cannot find {}", path.display()))?;
    if meta.is_dir() {
        let file = capcut::timeline_file(path).with_context(|| {
            format!("{} is not a CapCut draft: it has no draft_info.json or draft_content.json", path.display())
        })?;
        return read(&file);
    }
    ensure!(meta.is_file(), "{} is not a file", path.display());
    ensure!(meta.len() <= MAX_FILE, "{} is too large for a timeline", path.display());
    let bytes = std::fs::read(path).with_context(|| format!("Cannot read {}", path.display()))?;
    let folder = std::path::absolute(path)?.parent().map(Path::to_path_buf).unwrap_or_default();
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("otio") => otio::read(&bytes, &folder),
        Some("json") if capcut::is_timeline(path) => capcut::read(&bytes, &folder),
        _ => bail!(
            "{} is not a timeline Nuzky reads. Export the timeline from your editor as OpenTimelineIO (.otio), or choose a CapCut draft's folder.",
            path.display()
        ),
    }
}

/// The local file a media reference names, a relative one from `folder`. Any other address, a
/// web one or a share on another computer above all, names nothing Nuzky opens.
pub fn local_path(target: &str, folder: &Path) -> Option<PathBuf> {
    // One letter before the colon is a Windows drive, not a scheme.
    let scheme = target
        .split_once(':')
        .map(|(s, _)| s)
        .filter(|s| s.len() > 1 && s.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)));
    let path = match scheme {
        Some(s) if s.eq_ignore_ascii_case("file") => {
            let rest = &target[s.len() + 1..];
            let rest = rest.strip_prefix("//").map_or(rest, |r| r.strip_prefix("localhost").unwrap_or(r));
            // file://host/… is a file on another computer.
            if !rest.starts_with('/') {
                return None;
            }
            let decoded = percent_encoding::percent_decode_str(rest).decode_utf8().ok()?;
            let path = decoded.as_ref();
            // file:///C:/… names C:/… on Windows.
            let drive =
                path.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic) && path.as_bytes().get(2) == Some(&b':');
            PathBuf::from(if cfg!(windows) && drive { &path[1..] } else { path })
        }
        Some(_) => return None,
        None if target.is_empty() => return None,
        None => PathBuf::from(target),
    };
    // Two slashes start a path on another computer on Windows (\\server\share, //server/share or
    // \\?\UNC\…), which even looking at reaches over the network.
    let text = path.to_string_lossy();
    if ["//", r"\\", r"/\", r"\/"].iter().any(|start| text.starts_with(start)) {
        return None;
    }
    Some(if path.is_absolute() { path } else { folder.join(path) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_files_are_media() {
        let folder = Path::new("/home/me/edits");
        assert_eq!(local_path("file:///home/me/My%20Talk.mov", folder), Some("/home/me/My Talk.mov".into()));
        assert_eq!(local_path("file://localhost/home/me/a.mov", folder), Some("/home/me/a.mov".into()));
        assert_eq!(local_path("FILE:/home/me/a.mov", folder), Some("/home/me/a.mov".into()));
        assert_eq!(local_path("media/a.mov", folder), Some("/home/me/edits/media/a.mov".into()));
        assert_eq!(local_path("/abs/a.mov", folder), Some("/abs/a.mov".into()));
        #[cfg(windows)]
        assert_eq!(local_path("file:///C:/Users/me/My%20Talk.mov", folder), Some("C:/Users/me/My Talk.mov".into()));
        for foreign in [
            "http://example.com/a.mov",
            "https://x/a.mov",
            "smb://nas/a.mov",
            "file://nas/a.mov",
            "file:////nas/share/a.mov",
            "file:///%2Fnas/share/a.mov",
            r"\\nas\share\a.mov",
            r"\\?\UNC\nas\share\a.mov",
            "//nas/share/a.mov",
            "ftp:a",
            "",
        ] {
            assert_eq!(local_path(foreign, folder), None, "{foreign}");
        }
    }
}

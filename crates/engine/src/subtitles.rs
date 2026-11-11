//! Captions as SubRip (`.srt`) files for YouTube or another editor: one file per captions track, so one per
//! language, as YouTube and players take them.
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::export::write_text;
use crate::model::{ClipContent, Project, Track};

/// The SRT text of a captions track: its captions in time order, numbered from 1 and cut at the end of the video.
pub fn srt(track: &Track, duration_us: i64) -> String {
    let mut clips: Vec<_> = track.clips.iter().collect();
    clips.sort_by_key(|clip| clip.start_us);
    let mut out = String::new();
    let mut number = 0;
    for clip in clips {
        let ClipContent::Text { text, .. } = &clip.content else { continue };
        // A blank line ends a caption in SRT, so a caption never carries one.
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
        let (start, end) = (ms(clip.start_us.max(0)), ms(clip.end_us().min(duration_us)));
        if lines.is_empty() || end <= start {
            continue;
        }
        number += 1;
        out += &format!("{number}\n{} --> {}\n{}\n\n", stamp(start), stamp(end), lines.join("\n"));
    }
    out
}

fn ms(us: i64) -> i64 {
    (us + 500).div_euclid(1000)
}

fn stamp(ms: i64) -> String {
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// The SRT files of the project's captions beside `base`, whose extension they replace: `reel.srt` for one
/// captions track, `reel.cs.srt` and `reel.en.srt` for two. `speech` is the language the speech was recognised
/// in, which names its captions. Hidden tracks are included: hiding the captions in the picture is how a video
/// goes to YouTube with its captions in a file of their own. Empty when the project has no captions.
pub fn files(project: &Project, speech: Option<&str>, base: &Path) -> Vec<(PathBuf, String)> {
    let duration = project.duration_us();
    let mut tracks: Vec<(&Track, String)> = project
        .tracks
        .iter()
        .filter(|track| track.is_captions())
        .map(|track| (track, srt(track, duration)))
        .filter(|(_, text)| !text.is_empty())
        .collect();
    tracks.sort_by_key(|(track, _)| track.translation.is_some());
    let one = tracks.len() == 1;
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    for (track, text) in tracks {
        let language = if one { None } else { track.translation.as_deref().or(speech) };
        let name = |n: usize| {
            let parts: Vec<String> =
                language.map(str::to_owned).into_iter().chain((n > 1).then(|| n.to_string())).collect();
            base.with_extension(if parts.is_empty() { "srt".into() } else { format!("{}.srt", parts.join(".")) })
        };
        let path = (1..).map(name).find(|path| files.iter().all(|(taken, _)| taken != path)).expect("a free name");
        files.push((path, text));
    }
    files
}

/// Writes each file through a temporary one beside it, replacing an existing one only with `replace_existing`.
pub fn write(files: &[(PathBuf, String)], replace_existing: bool) -> Result<()> {
    for (path, text) in files {
        write_text(text, path, replace_existing).with_context(|| format!("Cannot write {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BOX_RADIUS, CAPTIONS_TRACK, Clip, TextAlign, TextStyle, TrackKind};

    fn caption(start_us: i64, duration_us: i64, text: &str) -> Clip {
        let style = TextStyle {
            font_family: None,
            font_size: 95.0,
            color: "#ffffff".into(),
            bold: false,
            font_weight: None,
            stroke_width: 7.5,
            stroke_color: "#000000".into(),
            background: None,
            max_width: None,
            highlight: None,
            keywords: None,
            shadow: None,
            letter_case: Default::default(),
            letter_spacing: 0.0,
            line_spacing: 0.0,
            align: TextAlign::Center,
            box_radius: BOX_RADIUS,
        };
        let content = ClipContent::Text { text: text.into(), style, transform: Default::default(), words: Vec::new() };
        Clip::new(format!("c{start_us}"), start_us, duration_us, content)
    }

    fn track(translation: Option<&str>, clips: Vec<Clip>) -> Track {
        Track {
            id: format!("t-{}", translation.unwrap_or("speech")),
            kind: TrackKind::Text,
            name: CAPTIONS_TRACK.into(),
            muted: false,
            hidden: false,
            keep_in_place: false,
            clips,
            translation: translation.map(str::to_owned),
        }
    }

    #[test]
    fn srt_numbers_captions_in_time_order_and_cuts_them_at_the_end() {
        let track = track(
            None,
            vec![
                caption(2_000_000, 1_500_000, "druhý"),
                caption(0, 1_000_400, " první \n\n  řádek "),
                caption(3_600_000_000, 1_000_000, "after the end"),
                caption(1_200_000, 500_000, "  "),
            ],
        );
        assert_eq!(
            srt(&track, 3_000_000),
            "1\n00:00:00,000 --> 00:00:01,000\nprvní\nřádek\n\n2\n00:00:02,000 --> 00:00:03,000\ndruhý\n\n"
        );
        assert_eq!(stamp(3_723_045), "01:02:03,045");
    }
}

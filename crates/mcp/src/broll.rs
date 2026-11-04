//! B-roll from screen recordings: where the speaker names an app or a tool, the silent video named after it, such
//! as a screen recording Nuzky made of that app's window, goes over the whole frame for as long as they talk about
//! it. Only silent videos are offered, so the voice under them stays the only sound.
use anyhow::{Result, ensure};
use nuzky_analysis::word_token;
use nuzky_engine::Project;
use nuzky_engine::edit::EditCmd;
use nuzky_engine::model::{Asset, AssetKind, ClipContent, TrackKind};
use nuzky_engine::speech::TimelineWord;
use serde::Serialize;

/// Words of a file's name that say what kind of file it is, not what it shows.
const GENERIC: &[&str] = &[
    "screen",
    "recording",
    "record",
    "recorded",
    "capture",
    "window",
    "video",
    "clip",
    "copy",
    "untitled",
    "final",
    "the",
    "and",
    "for",
    "with",
    "obrazovky",
    "obrazovka",
    "zaznam",
    "nahravka",
    "snimek",
    "kopie",
    "mov",
    "mp4",
    "m4v",
    "mkv",
    "webm",
    "avi",
    "gif",
];

#[derive(Clone, Debug, Serialize)]
pub struct Broll {
    /// INCLUSIVE word numbers of the transcript.
    pub from: usize,
    pub to: usize,
    pub start_us: i64,
    pub end_us: i64,
    /// How long the recording shows: from start_us to end_us, or less when the recording is shorter.
    pub duration_us: i64,
    pub text: String,
    pub asset_id: String,
    pub asset_name: String,
    /// The words heard that name the recording, as said.
    pub said: Vec<String>,
    pub reason: String,
}

impl Broll {
    /// The recording over the whole frame from the first word to the last.
    pub fn edit(&self) -> EditCmd {
        EditCmd::AddOverlay {
            asset_id: self.asset_id.clone(),
            start_us: self.start_us,
            duration_us: Some(self.duration_us),
        }
    }
}

/// The videos B-roll can come from: silent ones, as screen recordings are.
pub fn recordings(project: &Project) -> Vec<&Asset> {
    project.assets.iter().filter(|a| a.kind == AssetKind::Video && !a.has_audio && a.duration_us > 0).collect()
}

/// What a name says, as words are compared: "Ghostty · ✳ Claude Code" gives ghostty, claude and code.
fn name_words(name: &str) -> Vec<String> {
    let mut words: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .map(word_token)
        .filter(|w| w.chars().count() >= 3 && !w.chars().all(|c| c.is_ascii_digit()) && !GENERIC.contains(&w.as_str()))
        .collect();
    words.dedup();
    words
}

/// A spoken word names a word of a name when they are the same or, both of four letters or more, differ only in an
/// ending of up to two letters, as Czech bends names: "Figmě" names Figma, "Claudu" names Claude.
fn names(spoken: &str, name: &str) -> bool {
    if spoken == name {
        return true;
    }
    let (s, n) = (spoken.chars().count(), name.chars().count());
    let shared = spoken.chars().zip(name.chars()).take_while(|(a, b)| a == b).count();
    s >= 4 && n >= 4 && shared >= 4.max(n - 2) && s.abs_diff(n) <= 2
}

struct Found<'a> {
    asset: &'a Asset,
    /// Indices of the words heard that name it, and how many words of its name they cover.
    said: Vec<usize>,
    covered: usize,
    of: usize,
}

fn best<'a>(
    words: &[TimelineWord],
    [from, to]: [usize; 2],
    recordings: &[(&'a Asset, Vec<String>)],
) -> Option<Found<'a>> {
    let spoken: Vec<String> = words[from..=to].iter().map(|w| word_token(&w.text)).collect();
    recordings
        .iter()
        .filter_map(|(asset, name)| {
            let mut said = Vec::new();
            let mut covered = 0;
            for part in name {
                let heard: Vec<usize> = (0..spoken.len()).filter(|&i| names(&spoken[i], part)).collect();
                if !heard.is_empty() {
                    covered += 1;
                    said.extend(heard.into_iter().map(|i| from + i));
                }
            }
            said.sort_unstable();
            said.dedup();
            (covered > 0).then_some(Found { asset, said, covered, of: name.len() })
        })
        // The most words of its name heard, then the largest share of them; the first in the project on a tie.
        .reduce(|a, b| if (b.covered, b.covered * a.of) > (a.covered, a.covered * b.of) { b } else { a })
}

/// The recording is already over this part of the timeline.
fn placed(project: &Project, asset_id: &str, start_us: i64, end_us: i64) -> bool {
    project.tracks.iter().skip(1).filter(|t| t.kind == TrackKind::Video).flat_map(|t| &t.clips).any(|c| {
        matches!(&c.content, ClipContent::Media { asset_id: a, .. } if a == asset_id)
            && c.start_us < end_us
            && c.start_us + c.duration_us > start_us
    })
}

/// B-roll for each sentence that names a recording, consecutive sentences naming the same one as one, or for the
/// INCLUSIVE word range `range` only. Sentences whose recording already lies over them are left out.
pub fn suggest(project: &Project, words: &[TimelineWord], range: Option<[usize; 2]>) -> Result<Vec<Broll>> {
    let spans = match range {
        Some([from, to]) => {
            ensure!(from <= to && to < words.len(), "INVALID_RANGE: words {from} to {to} are not in the transcript");
            vec![[from, to]]
        }
        None => crate::transcript::sentences(words),
    };
    let recordings: Vec<_> = recordings(project).into_iter().map(|a| (a, name_words(&a.name))).collect();
    let mut found: Vec<(Found, [usize; 2])> = Vec::new();
    for span in spans {
        let Some(this) = best(words, span, &recordings) else { continue };
        match found.last_mut() {
            Some((last, at)) if last.asset.id == this.asset.id && at[1] + 1 == span[0] => {
                at[1] = span[1];
                last.said.extend(this.said);
            }
            _ => found.push((this, span)),
        }
    }
    Ok(found
        .into_iter()
        .map(|(found, [from, to])| {
            let (start_us, end_us) = (words[from].start_us, words[to].end_us);
            let said: Vec<String> = found
                .said
                .iter()
                .map(|&i| words[i].text.trim_matches(|c: char| !c.is_alphanumeric()).to_owned())
                .collect();
            let mut reason =
                format!("\"{}\" is said here and names the recording \"{}\"", said.join(" "), found.asset.name);
            let length = found.asset.duration_us - found.asset.part.as_ref().map_or(0, |p| p.start_us);
            let duration_us = length.min(end_us - start_us);
            if length < end_us - start_us {
                reason += &format!(
                    "; it lasts {:.1} s, shorter than the {:.1} s said",
                    length as f64 / 1e6,
                    (end_us - start_us) as f64 / 1e6
                );
            }
            Broll {
                from,
                to,
                start_us,
                end_us,
                duration_us,
                text: words[from..=to].iter().map(|w| w.text.trim()).collect::<Vec<_>>().join(" "),
                asset_id: found.asset.id.clone(),
                asset_name: found.asset.name.clone(),
                said,
                reason,
            }
        })
        .filter(|b| !placed(project, &b.asset_id, b.start_us, b.end_us))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_named_by_its_words_and_their_czech_endings() {
        assert_eq!(name_words("Ghostty · ✳ Claude Code"), ["ghostty", "claude", "code"]);
        assert_eq!(name_words("Screen Recording 2026-11-04 at 9.41.12.mov"), Vec::<String>::new());
        assert_eq!(name_words("Záznam obrazovky Figma"), ["figma"]);
        for (spoken, name) in [("claude", "claude"), ("claudu", "claude"), ("figme", "figma"), ("captions", "caption")]
        {
            assert!(names(spoken, name), "{spoken} names {name}");
        }
        for (spoken, name) in [("cap", "captions"), ("clips", "claude"), ("cut", "cutting"), ("codexify", "code")] {
            assert!(!names(spoken, name), "{spoken} does not name {name}");
        }
    }
}

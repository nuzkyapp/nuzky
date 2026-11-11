//! Captions in another language. The agent reads the speech captions as sentences, translates each one itself, and
//! Nuzky lays the translations out as a captions track of their own over the time of the sentences they translate.
//! The translated track holds no spoken words, so nothing made from the transcript ever writes into it.

use anyhow::{Context, Result, ensure};
use nuzky_analysis::{CaptionGrouping, Word, group_words};
use nuzky_engine::Project;
use nuzky_engine::edit::{CAPTION_Y, CaptionPreset, CaptionSegment, EditCmd, TransformPatch};
use nuzky_engine::model::{Canvas, Clip, ClipContent, TextStyle, Track, language_name};
use sha2::{Digest, Sha256};

/// A pause this long between captions ends a sentence, as get_transcript's sentences end.
const SENTENCE_GAP_US: i64 = 600_000;
/// A sentence that runs on without punctuation is closed after this long, so its translation never sits on screen
/// for half a minute.
const MAX_SENTENCE_US: i64 = 12_000_000;
/// Translated captions are drawn at this share of the speech captions' size.
const TRANSLATION_SIZE: f32 = 0.7;
/// A translated caption reads as a short line under the speech, not word by word.
pub const TRANSLATION_GROUPING: CaptionGrouping =
    CaptionGrouping { max_words: 5, max_chars: 30, break_gap_us: 300_000 };

pub struct Sentence {
    pub start_us: i64,
    pub end_us: i64,
    pub text: String,
}

/// The captions track of the speech that `track_id` names, or the project's only one.
pub fn source_track<'a>(project: &'a Project, track_id: Option<&str>) -> Result<&'a Track> {
    let speech: Vec<&Track> = project.tracks.iter().filter(|t| t.is_speech_captions()).collect();
    match track_id {
        Some(id) => speech
            .into_iter()
            .find(|t| t.id == id)
            .with_context(|| format!("UNKNOWN_TRACK: {id} is not a captions track of the speech")),
        None => {
            ensure!(speech.len() <= 1, "AMBIGUOUS_CAPTIONS: several captions tracks of the speech; give track_id");
            speech.into_iter().next().context("NO_CAPTIONS: make the captions of the speech first with build_captions")
        }
    }
}

/// The track that holds the captions translated into `language`, if there is one.
pub fn translation_track<'a>(project: &'a Project, language: &str) -> Option<&'a Track> {
    project.tracks.iter().find(|t| t.is_captions() && t.translation.as_deref() == Some(language))
}

/// The captions of `track` joined into sentences, in time order: a sentence ends after a caption ending in . ! ? or …,
/// at a pause of 600 ms or more, and after 12 s.
pub fn sentences(track: &Track) -> Vec<Sentence> {
    let mut clips: Vec<(&Clip, &str)> = track
        .clips
        .iter()
        .filter_map(|c| match &c.content {
            ClipContent::Text { text, .. } if !text.trim().is_empty() => Some((c, text.trim())),
            _ => None,
        })
        .collect();
    clips.sort_by_key(|(c, _)| c.start_us);
    let mut out: Vec<Sentence> = Vec::new();
    let mut open = false;
    for (clip, text) in clips {
        match out.last_mut() {
            Some(last)
                if open
                    && clip.start_us - last.end_us < SENTENCE_GAP_US
                    && clip.end_us() - last.start_us <= MAX_SENTENCE_US =>
            {
                last.text = format!("{} {text}", last.text);
                last.end_us = clip.end_us();
            }
            _ => out.push(Sentence { start_us: clip.start_us, end_us: clip.end_us(), text: text.to_owned() }),
        }
        open = !ends_sentence(text);
    }
    out
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', '”', '“', '»', '«', ')', ']']).ends_with(['.', '!', '?', '…'])
}

/// What the sentences say and when, so translations of captions that changed since are refused.
pub fn key(sentences: &[Sentence]) -> String {
    let mut hash = Sha256::new();
    for s in sentences {
        hash.update(format!("{}\t{}\t{}\n", s.start_us, s.end_us, s.text));
    }
    hash.finalize().iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// The captions of the translated sentences, each over the time of the sentence it translates: its words share
/// that time by their length and go on screen in groups as `grouping` allows. Empty translations leave a sentence
/// without a caption. No caption keeps words, as none of them is heard.
pub fn translated_segments(
    sentences: &[Sentence],
    translations: &[String],
    grouping: CaptionGrouping,
    frame_us: i64,
) -> Vec<CaptionSegment> {
    let mut segments = Vec::new();
    for (i, (sentence, text)) in sentences.iter().zip(translations).enumerate() {
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        let span = sentence.end_us - sentence.start_us;
        let total: i64 = words.iter().map(|w| w.chars().count() as i64 + 1).sum();
        let mut said = 0;
        let timed: Vec<Word> = words
            .iter()
            .map(|w| {
                let start_us = sentence.start_us + span * said / total;
                said += w.chars().count() as i64 + 1;
                Word {
                    start_us,
                    end_us: sentence.start_us + span * said / total,
                    text: (*w).to_owned(),
                    probability: 1.0,
                }
            })
            .collect();
        let next = sentences.get(i + 1).map_or(i64::MAX, |s| s.start_us);
        for mut segment in group_words(&timed, grouping, frame_us) {
            // Never past the sentence, so the translation of the last one never makes the video longer.
            segment.end_us = segment.end_us.min(next).min(sentence.end_us).max(segment.start_us + 1);
            segment.words.clear();
            segments.push(segment);
        }
    }
    segments
}

/// The look of new translated captions: the speech captions' own at 70 % of their size, without a highlight or
/// key words, which need the spoken words, and their place: under the speech captions, by as much as the lowest of
/// them and the tallest translation draw.
pub fn translated_look(
    source: &Track,
    segments: &[CaptionSegment],
    canvas: &Canvas,
) -> (CaptionPreset, TransformPatch) {
    let first = source.clips.iter().find(|c| matches!(c.content, ClipContent::Text { .. }));
    let mut style = match first.map(|c| &c.content) {
        Some(ClipContent::Text { style, .. }) => style.clone(),
        _ => crate::params::reel().style,
    };
    let wrap = |style: &TextStyle| {
        style.max_width.unwrap_or_else(|| {
            canvas.safe_area().map_or(canvas.width as f32 * 0.9, |area| area.centered_width(canvas.width as f32))
        })
    };
    let height = canvas.height.max(1) as f32;
    let place = crate::transcript::placement(source).unwrap_or_default();
    let (x, scale) = (place.x.unwrap_or(0.0), place.scale.unwrap_or(1.0));
    let speech_bottom = source
        .clips
        .iter()
        .filter(|c| c.keyframes.is_empty())
        .filter_map(|c| match &c.content {
            ClipContent::Text { text, style, transform, .. } => Some(
                (0.5 + transform.y) * height
                    + crate::fonts::text_height(text, style, wrap(style)) * transform.scale / 2.0,
            ),
            ClipContent::Media { .. } | ClipContent::Adjustment { .. } => None,
        })
        .fold((0.5 + place.y.unwrap_or(CAPTION_Y)) * height, f32::max);
    style.font_size *= TRANSLATION_SIZE;
    style.stroke_width *= TRANSLATION_SIZE;
    style.highlight = None;
    style.keywords = None;
    let tallest =
        segments.iter().map(|s| crate::fonts::text_height(&s.text, &style, wrap(&style)) * scale).fold(0.0, f32::max);
    let centre = speech_bottom + 0.2 * style.font_size * scale + tallest / 2.0;
    let preset = CaptionPreset {
        name: String::new(),
        style,
        anim_in: first.and_then(|c| c.anim_in),
        anim_out: first.and_then(|c| c.anim_out),
    };
    let transform = TransformPatch {
        x: Some(x),
        y: Some((centre / height - 0.5).min(0.42)),
        scale: Some(scale),
        ..TransformPatch::default()
    };
    (preset, transform)
}

/// The edit that puts the translations into `language`'s track: a new one, or the same track again, keeping its look
/// and place unless `look` or `transform` say otherwise.
pub fn translation_edit(
    project: &Project,
    language: &str,
    segments: Vec<CaptionSegment>,
    look: Option<CaptionPreset>,
    transform: Option<TransformPatch>,
    source: &Track,
) -> Result<EditCmd> {
    ensure!(
        language_name(language).is_some(),
        "INVALID_LANGUAGE: captions translate into cs, sk, en, de, pl, es, fr, it or uk"
    );
    ensure!(!segments.is_empty(), "NO_CAPTIONS: every translation is empty");
    let kept = translation_track(project, language).and_then(|track| {
        let clip = track.clips.iter().find(|c| matches!(c.content, ClipContent::Text { .. }))?;
        let ClipContent::Text { style, .. } = &clip.content else { return None };
        let look =
            CaptionPreset { name: String::new(), style: style.clone(), anim_in: clip.anim_in, anim_out: clip.anim_out };
        Some((track, look, crate::transcript::placement(track)))
    });
    Ok(match kept {
        Some((track, kept_look, place)) => {
            let CaptionPreset { style, anim_in, anim_out, .. } = look.unwrap_or(kept_look);
            EditCmd::ReplaceCaptions {
                track_id: track.id.clone(),
                segments,
                style,
                anim_in,
                anim_out,
                transform: transform.or(place),
                ranges: None,
            }
        }
        None => {
            let (derived, place) = translated_look(source, &segments, &project.canvas);
            let CaptionPreset { style, anim_in, anim_out, .. } = look.unwrap_or(derived);
            EditCmd::AddCaptions {
                segments,
                style,
                anim_in,
                anim_out,
                transform: Some(transform.unwrap_or(place)),
                translation: Some(language.to_owned()),
            }
        }
    })
}

/// A full style asked for, a preset scaled to the canvas, or `None` for the translation's own look.
pub fn asked_look(style: Option<TextStyle>, preset: Option<&str>, canvas: (u32, u32)) -> Result<Option<CaptionPreset>> {
    Ok(match (style, preset) {
        (Some(_), Some(_)) => anyhow::bail!("INVALID_ARGUMENTS: give style or style_preset, not both"),
        (Some(style), None) => Some(CaptionPreset { name: String::new(), style, anim_in: None, anim_out: None }),
        (None, Some(name)) => {
            let mut preset = crate::caption_styles::find(name)?;
            preset.style.scale_pixels(nuzky_engine::model::unit_of(canvas.0, canvas.1));
            Some(preset)
        }
        (None, None) => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuzky_engine::model::TrackKind;

    fn track(captions: &[(i64, i64, &str)]) -> Track {
        Track {
            id: "cs".into(),
            kind: TrackKind::Text,
            name: nuzky_engine::model::CAPTIONS_TRACK.into(),
            muted: false,
            hidden: false,
            keep_in_place: false,
            translation: None,
            clips: captions
                .iter()
                .enumerate()
                .map(|(i, &(start, end, text))| {
                    Clip::new(
                        format!("c{i}"),
                        start,
                        end - start,
                        ClipContent::Text {
                            text: text.into(),
                            style: crate::params::reel().style,
                            transform: Default::default(),
                            words: Vec::new(),
                        },
                    )
                })
                .collect(),
        }
    }

    /// Reel captions of one or two words join back into the sentences they were cut from: at a full stop, at a
    /// pause, and before a sentence would run past 12 s.
    #[test]
    fn captions_join_into_sentences() {
        let t = track(&[
            (0, 400_000, "Dneska vám"),
            (400_000, 900_000, "ukážu,"),
            (900_000, 1_500_000, "jak stříhat."),
            (1_500_000, 2_000_000, "Nejdřív"),
            (2_000_000, 2_400_000, "klipy"),
            (3_200_000, 3_600_000, "a pak"),
            (3_600_000, 4_000_000, "titulky!"),
        ]);
        let found: Vec<(i64, i64, String)> =
            sentences(&t).into_iter().map(|s| (s.start_us, s.end_us, s.text)).collect();
        assert_eq!(
            found,
            [
                (0, 1_500_000, "Dneska vám ukážu, jak stříhat.".to_owned()),
                (1_500_000, 2_400_000, "Nejdřív klipy".to_owned()),
                (3_200_000, 4_000_000, "a pak titulky!".to_owned()),
            ]
        );
        let long: Vec<(i64, i64, &str)> = (0..30).map(|i| (i * 500_000, i * 500_000 + 500_000, "slovo")).collect();
        let long = sentences(&track(&long));
        assert_eq!(long.len(), 2);
        assert!(long.iter().all(|s| s.end_us - s.start_us <= MAX_SENTENCE_US));
    }

    /// A translation shares its sentence's time by the length of its words, never starts before the sentence nor
    /// runs past it, and keeps no words to highlight.
    #[test]
    fn translations_share_their_sentence_time() {
        let s = |start_us, end_us| Sentence { start_us, end_us, text: String::new() };
        let sentences = [s(1_000_000, 3_000_000), s(3_000_000, 4_000_000), s(5_000_000, 6_000_000)];
        let translations =
            ["Today I'll show you how to edit a video.".to_owned(), String::new(), "Captions!".to_owned()];
        let segments = translated_segments(&sentences, &translations, TRANSLATION_GROUPING, 33_334);
        let texts: Vec<&str> = segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["Today I'll show you how", "to edit a video.", "Captions!"]);
        assert_eq!(segments[0].start_us, 1_000_000);
        assert!(segments[1].start_us > 2_000_000 && segments[1].end_us <= 3_000_000, "{:?}", segments[1].start_us);
        assert_eq!((segments[2].start_us, segments[2].end_us), (5_000_000, 6_000_000));
        assert!(segments.iter().all(|s| s.words.is_empty()));
    }
}

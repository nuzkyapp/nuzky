//! What a recap shows of a cut: the recording before it, what went on its timeline, and the words, retakes and
//! pauses removed, counted the way the run summary counts them.

use anyhow::Result;
use nuzky_engine::{
    Project,
    edit::{EditCmd, Editor, MAIN_TRACK},
    model::{Asset, AssetKind, ClipContent},
    recap::{Recap, RecapCut, RecapTotals, RecapWord},
};
use nuzky_session::{summary, transcripts::TranscriptStore};

use crate::transcript::{self, Derived};

/// The recording as imported: every video the main track plays now or played in a kept version (`kept`, in the
/// order they first appear), whole, one after another, with the project's word corrections.
pub fn raw_import(project: &Project, kept: &[Asset]) -> Result<Project> {
    let mut raw = Project::new(&project.name);
    raw.canvas = project.canvas.clone();
    raw.word_corrections = project.word_corrections.clone();
    let mut editor = Editor::new(raw);
    let playing =
        project.tracks.iter().filter(|t| t.id == MAIN_TRACK).flat_map(|t| &t.clips).filter_map(|c| match &c.content {
            ClipContent::Media { asset_id, .. } => project.asset(asset_id),
            ClipContent::Text { .. } => None,
        });
    let mut assets: Vec<Asset> = Vec::new();
    for asset in kept.iter().chain(playing).filter(|a| a.kind == AssetKind::Video) {
        if !assets.iter().any(|a| a.id == asset.id) {
            // The whole file, also the footage a YouTube video's part keeps hidden around it.
            assets.push(Asset { part: None, ..project.asset(&asset.id).unwrap_or(asset).clone() });
        }
    }
    let mut cmds = vec![EditCmd::AddAssets { assets: assets.clone() }];
    cmds.extend(assets.into_iter().map(|a| EditCmd::AddClip {
        asset_id: a.id,
        start_us: None,
        track_id: Some(MAIN_TRACK.into()),
    }));
    editor.apply_batch(cmds, None)?;
    Ok(editor.project)
}

/// Missing transcripts only matter for files whose main-track picture is cut.
pub fn untranscribed(before: &Project, cuts: &[RecapCut], derived: &Derived) -> Vec<String> {
    derived
        .untranscribed
        .iter()
        .filter(|id| {
            before.tracks.iter().filter(|t| t.id == MAIN_TRACK).flat_map(|t| &t.clips).any(|clip| {
                matches!(&clip.content, ClipContent::Media { asset_id, .. } if asset_id == *id)
                    && cuts.iter().any(|cut| cut.start_us < clip.end_us() && cut.end_us > clip.start_us)
            })
        })
        .cloned()
        .collect()
}

/// The recap of the cut from `before` to `after`, and the files it touches that have no transcript.
pub fn recap(before: Project, after: Project, transcripts: &TranscriptStore) -> Result<(Recap, Vec<String>)> {
    let derived = transcript::derive(&main_track(&before), transcripts)?;
    let recap = build(before, after, &derived);
    let missing = untranscribed(&recap.before, &recap.cuts, &derived);
    Ok((recap, missing))
}

/// Only the main track, heard whatever its volume, so the words counted are those of the cut recording and
/// never an overlay's, and a muted take keeps its own.
fn main_track(project: &Project) -> Project {
    let mut main = project.clone();
    main.tracks.retain(|t| t.id == MAIN_TRACK);
    for track in &mut main.tracks {
        track.muted = false;
        for clip in &mut track.clips {
            if let ClipContent::Media { volume, .. } = &mut clip.content {
                *volume = 1.0;
            }
        }
    }
    main
}

fn build(before: Project, after: Project, derived: &Derived) -> Recap {
    let change = summary::cut(&before, &after);
    let mut ranges = Vec::new();
    for clip in before.tracks.iter().filter(|t| t.id == MAIN_TRACK).flat_map(|t| &t.clips) {
        let ClipContent::Media { asset_id, source_in_us, speed, .. } = &clip.content else { continue };
        let gone = change.gone_assets.contains(asset_id);
        let whole = [(0, before.asset(asset_id).map_or(0, |a| a.duration_us))];
        let passages =
            if gone { whole.as_slice() } else { change.passages.get(asset_id).map_or(&[][..], Vec::as_slice) };
        for &(a, b) in passages {
            let map = |t| {
                (clip.start_us + ((t - source_in_us) as f64 / f64::from(*speed)).round() as i64)
                    .clamp(clip.start_us, clip.end_us())
            };
            let (a, b) = (map(a), map(b));
            if a < b {
                ranges.push((a, b));
            }
        }
    }
    ranges.sort();
    let mut cuts: Vec<RecapCut> = Vec::new();
    for (start_us, end_us) in ranges {
        match cuts.last_mut() {
            Some(last) if start_us <= last.end_us => last.end_us = last.end_us.max(end_us),
            _ => cuts.push(RecapCut { start_us, end_us, words: 0, retakes: 0 }),
        }
    }
    let transcribed = untranscribed(&before, &cuts, derived).is_empty();
    let mut recap = Recap { before, after, cuts, words: Vec::new(), transcribed, tick: None, whoosh: None };
    if !transcribed {
        return recap;
    }
    let owner = |i: usize| {
        let w = &derived.words[i];
        let mid = w.start_us + (w.end_us - w.start_us) / 2;
        recap.cuts.iter().position(|c| mid >= c.start_us && mid < c.end_us)
    };
    let owners: Vec<_> = (0..derived.words.len()).map(owner).collect();
    recap.words = derived
        .words
        .iter()
        .zip(&owners)
        .map(|(w, cut)| RecapWord {
            start_us: w.start_us,
            end_us: w.end_us,
            text: w.text.clone(),
            removed: cut.is_some(),
        })
        .collect();
    for cut in owners.iter().flatten() {
        recap.cuts[*cut].words += 1;
    }
    let retakes = nuzky_analysis::retakes(&derived.words, &derived.languages);
    // A take dropped whole is one retake, however many attempts at sentences it held.
    let gone = |i: usize| change.gone_assets.contains(&derived.words[i].asset_id);
    let removed = |from: usize, to: usize| !gone(from) && owners[from..=to].iter().all(Option::is_some);
    let mut attempts = std::collections::HashSet::new();
    for group in retakes.groups {
        if group.sentences.iter().any(|a| owners[a.from..=a.to].iter().any(Option::is_none)) {
            for a in group.sentences.iter().filter(|a| removed(a.from, a.to)) {
                attempts.insert((a.from, a.to));
            }
        }
    }
    for a in retakes.unfinished.iter().filter(|a| removed(a.from, a.to)) {
        attempts.insert((a.from, a.to));
    }
    for (from, _) in attempts {
        recap.cuts[owners[from].unwrap()].retakes += 1;
    }
    for asset in &change.gone_assets {
        if derived.sources.get(asset).is_some_and(|words| !words.is_empty())
            && let Some(clip) = recap
                .before
                .tracks
                .iter()
                .filter(|t| t.id == MAIN_TRACK)
                .flat_map(|t| &t.clips)
                .find(|c| matches!(&c.content, ClipContent::Media { asset_id, .. } if asset_id == asset))
            && let Some(cut) = recap.cuts.iter_mut().find(|c| clip.start_us >= c.start_us && clip.start_us < c.end_us)
        {
            cut.retakes += 1;
        }
    }
    recap
}

pub fn line(totals: &RecapTotals) -> Option<String> {
    let mut parts = Vec::new();
    for (n, noun) in [(totals.words, "word"), (totals.retakes, "retake"), (totals.pauses, "pause")] {
        if let Some(n) = n.filter(|n| *n > 0) {
            parts.push(format!("{n} {noun}{}", if n == 1 { "" } else { "s" }));
        }
    }
    let last = parts.pop()?;
    Some(format!("Removed {}", if parts.is_empty() { last } else { format!("{} and {last}", parts.join(", ")) }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuzky_engine::{edit::TimeRange, speech::Word};
    use serde_json::json;
    use std::collections::HashMap;

    fn project(ids: &[&str]) -> Project {
        let mut e = Editor::new(Project::new("Takes"));
        let assets = ids
            .iter()
            .map(|id| {
                serde_json::from_value(json!({
                    "id": id, "name": id, "path": format!("/{id}"), "kind": "video",
                    "durationUs": 10_000_000, "width": 1080, "height": 1920, "fps": 30,
                    "hasAudio": true, "rotation": 0, "mirror": false
                }))
                .unwrap()
            })
            .collect();
        e.apply(EditCmd::AddAssets { assets }, None).unwrap();
        for id in ids {
            e.apply(EditCmd::AddClip { asset_id: (*id).into(), start_us: None, track_id: None }, None).unwrap();
        }
        e.project
    }
    fn words(text: &str) -> Vec<Word> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, text)| Word {
                start_us: i as i64 * 500_000,
                end_us: i as i64 * 500_000 + 200_000,
                text: text.into(),
                probability: 1.0,
            })
            .collect()
    }
    fn derived(p: &Project, words: Vec<Word>) -> Derived {
        Derived::new(p, HashMap::from([("take".into(), words)]), vec![])
    }
    fn delete(p: &Project, ranges: &[(i64, i64)]) -> Project {
        let mut e = Editor::new(p.clone());
        e.apply(
            EditCmd::RippleDeleteRanges {
                ranges: ranges.iter().map(|&(start_us, end_us)| TimeRange { start_us, end_us }).collect(),
                keep_track_ids: None,
            },
            None,
        )
        .unwrap();
        e.project
    }

    #[test]
    fn recap_words_use_midpoints_and_half_open_cut_edges() {
        let p = project(&["take"]);
        let w = vec![
            Word { start_us: 800_000, end_us: 1_200_000, text: "start".into(), probability: 1.0 },
            Word { start_us: 1_800_000, end_us: 2_200_000, text: "end".into(), probability: 1.0 },
        ];
        let r = build(p.clone(), delete(&p, &[(1_000_000, 2_000_000)]), &derived(&p, w));
        assert_eq!(r.words.iter().map(|w| w.removed).collect::<Vec<_>>(), [true, false]);
        assert_eq!(r.cuts[0].words, 1);
        assert_eq!(line(&r.totals()).as_deref(), Some("Removed 1 word"));
    }

    #[test]
    fn recap_retakes_follow_the_actual_kept_attempt() {
        let p = project(&["take"]);
        let w = words("Dneska vám ukážu video. Dneska vám ukážu video.");
        let d = derived(&p, w.clone());
        let groups = nuzky_analysis::retakes(&d.words, &d.languages).groups;
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].keep, 1);
        let r = build(p.clone(), delete(&p, &[(2_000_000, 4_000_000)]), &d);
        assert_eq!(r.totals().retakes, Some(1));
        assert_eq!(line(&r.totals()).as_deref(), Some("Removed 4 words and 1 retake"));
        let r = build(p.clone(), delete(&p, &[(0, 4_000_000)]), &derived(&p, w));
        assert_eq!(r.totals().retakes, Some(0));
    }

    #[test]
    fn recap_unfinished_attempt_and_gone_take_count_once() {
        let p = project(&["take"]);
        let w = words("Měl jsem několik… Pak ale přišel domů.");
        let d = derived(&p, w);
        assert_eq!(nuzky_analysis::retakes(&d.words, &d.languages).unfinished.len(), 1);
        let r = build(p.clone(), delete(&p, &[(0, 1_500_000)]), &d);
        assert_eq!(r.totals().retakes, Some(1));
        let p = project(&["take", "other"]);
        let r = build(p.clone(), delete(&p, &[(0, 10_000_000)]), &derived(&p, words("A whole take.")));
        assert_eq!(r.totals().retakes, Some(1));
        let mut p = project(&["other", "take"]);
        p.apply(EditCmd::AddPictureInPicture { asset_id: "take".into(), start_us: 0, duration_us: None }).unwrap();
        let r = build(p.clone(), delete(&p, &[(10_000_000, 20_000_000)]), &derived(&p, words("Hello.")));
        assert_eq!(r.totals().retakes, Some(1), "an earlier overlay is not the gone main-track take");
    }

    #[test]
    fn recap_a_dropped_take_repeating_a_kept_one_is_one_retake() {
        let p = project(&["take", "other"]);
        let same = words("Dneska vám ukážu video.");
        let d = Derived::new(&p, HashMap::from([("take".into(), same.clone()), ("other".into(), same)]), vec![]);
        assert_eq!(nuzky_analysis::retakes(&d.words, &d.languages).groups.len(), 1);
        let r = build(p.clone(), delete(&p, &[(0, 10_000_000)]), &d);
        assert_eq!(r.totals().retakes, Some(1));
    }

    #[test]
    fn recap_pauses_have_no_words_or_retakes() {
        let p = project(&["take"]);
        let r = build(
            p.clone(),
            delete(&p, &[(1_000_000, 2_000_000), (4_000_000, 5_000_000)]),
            &derived(&p, words("Hello.")),
        );
        assert_eq!((r.totals().words, r.totals().retakes, r.totals().pauses), (Some(0), Some(0), Some(2)));
        assert_eq!(line(&r.totals()).as_deref(), Some("Removed 2 pauses"));
    }

    #[test]
    fn recap_missing_cut_transcript_hides_all_counts() {
        let p = project(&["take", "other"]);
        let d = Derived::new(&p, HashMap::from([("take".into(), words("Hello."))]), vec!["other".into()]);
        let r = build(p.clone(), delete(&p, &[(0, 1_000_000), (10_000_000, 11_000_000)]), &d);
        assert!(!r.transcribed);
        assert!(r.words.is_empty());
        assert!(r.cuts.iter().all(|c| c.words == 0 && c.retakes == 0));
        assert_eq!((r.totals().words, r.totals().retakes, r.totals().pauses), (None, None, None));
        assert_eq!(line(&r.totals()), None);
    }

    #[test]
    fn recap_raw_import_uses_import_order_and_only_main_track_takes() {
        let mut p = project(&["left", "current", "overlay", "audio"]);
        p.assets[3].kind = AssetKind::Audio;
        p.tracks[0]
            .clips
            .retain(|c| matches!(&c.content, ClipContent::Media { asset_id, .. } if asset_id == "current"));
        p.apply(EditCmd::AddPictureInPicture { asset_id: "overlay".into(), start_us: 0, duration_us: None }).unwrap();
        p.apply(EditCmd::AddClip { asset_id: "audio".into(), start_us: Some(0), track_id: None }).unwrap();
        p.canvas.fps = 60;
        let mut dropped = p.assets[0].clone();
        dropped.id = "dropped".into();
        let kept = [p.assets[0].clone(), dropped, p.assets[1].clone(), p.assets[3].clone()];
        p.word_corrections.push(nuzky_engine::model::WordCorrection {
            asset_id: "current".into(),
            source_start_us: 0,
            original: "Wrong".into(),
            text: "Right".into(),
        });
        let r = raw_import(&p, &kept).unwrap();
        assert_eq!(r.name, p.name);
        assert_eq!(r.canvas, p.canvas);
        assert_eq!(r.word_corrections, p.word_corrections);
        // A take removed from the library since is still part of the recording, as history describes it.
        assert_eq!(r.assets.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["left", "dropped", "current"]);
        let clips = &r.tracks[0].clips;
        assert_eq!(clips.len(), 3);
        for (i, c) in clips.iter().enumerate() {
            assert_eq!((c.start_us, c.duration_us), (i as i64 * 10_000_000, 10_000_000));
            assert!(matches!(&c.content, ClipContent::Media { source_in_us: 0, speed, .. } if *speed == 1.0));
        }
        assert_eq!(r.tracks.len(), 1);
    }

    #[test]
    fn recap_counts_the_main_track_words_even_muted_and_never_an_overlays() {
        let mut p = project(&["take"]);
        let other = serde_json::from_value(json!({
            "id": "other", "name": "other", "path": "/other", "kind": "video", "durationUs": 10_000_000,
            "width": 1080, "height": 1920, "fps": 30, "hasAudio": true, "rotation": 0, "mirror": false
        }))
        .unwrap();
        p.apply(EditCmd::AddAssets { assets: vec![other] }).unwrap();
        p.apply(EditCmd::AddPictureInPicture { asset_id: "other".into(), start_us: 0, duration_us: None }).unwrap();
        if let ClipContent::Media { volume, .. } = &mut p.tracks[0].clips[0].content {
            *volume = 0.0;
        }
        let sources = HashMap::from([("take".into(), words("Main take.")), ("other".into(), words("Overlay talk."))]);
        let d = Derived::new(&main_track(&p), sources, vec![]);
        assert_eq!(d.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["Main", "take."]);
    }

    #[test]
    fn recap_line_numbers_and_plural_forms_match_totals() {
        let p = project(&["take"]);
        let r = build(p.clone(), delete(&p, &[(0, 500_000), (2_000_000, 3_000_000)]), &derived(&p, words("Hello.")));
        assert_eq!(line(&r.totals()).as_deref(), Some("Removed 1 word and 1 pause"));
        let mut totals = r.totals();
        totals.words = Some(212);
        totals.retakes = Some(5);
        totals.pauses = Some(25);
        assert_eq!(line(&totals).as_deref(), Some("Removed 212 words, 5 retakes and 25 pauses"));
        totals.words = Some(1);
        totals.retakes = Some(1);
        totals.pauses = Some(0);
        assert_eq!(line(&totals).as_deref(), Some("Removed 1 word and 1 retake"));
        totals.words = Some(0);
        totals.retakes = Some(0);
        assert_eq!(line(&totals), None);
    }
    #[test]
    fn recap_maps_speed_and_repeated_source_and_merges_touching_cuts() {
        let mut p = project(&["take"]);
        if let ClipContent::Media { source_in_us, speed, .. } = &mut p.tracks[0].clips[0].content {
            *source_in_us = 1_000_000;
            *speed = 2.0;
        }
        p.tracks[0].clips[0].duration_us = 4_000_000;
        let mut repeated = p.tracks[0].clips[0].clone();
        repeated.id = "repeated".into();
        repeated.start_us = 4_000_000;
        p.tracks[0].clips.push(repeated);
        let r = build(p.clone(), delete(&p, &[(0, 1_000_000), (4_000_000, 5_000_000)]), &derived(&p, vec![]));
        assert_eq!(
            r.cuts.iter().map(|c| (c.start_us, c.end_us)).collect::<Vec<_>>(),
            [(0, 1_000_000), (4_000_000, 5_000_000)]
        );
        let gone = build(p.clone(), Project::new("empty"), &derived(&p, vec![]));
        assert_eq!(gone.cuts.len(), 1);
        assert_eq!((gone.cuts[0].start_us, gone.cuts[0].end_us), (0, 8_000_000));
    }

    #[test]
    fn recap_reads_stored_transcripts_with_word_corrections() {
        use nuzky_session::transcripts::{Record, VERSION};
        let dir = std::env::temp_dir().join(format!("recap-transcripts-{}", nuzky_engine::edit::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut p = project(&["take"]);
        let source = dir.join("source");
        std::fs::write(&source, b"recap test media fingerprint").unwrap();
        p.assets[0].path = source.to_string_lossy().into_owned();
        let store = TranscriptStore::at(dir.join("transcripts")).unwrap();
        store
            .put(
                &p.assets[0],
                &Record {
                    version: VERSION,
                    fingerprint: store.fingerprint(&p.assets[0]).unwrap(),
                    duration_us: 10_000_000,
                    model: "test".into(),
                    language: "en".into(),
                    words: words("Wrong."),
                    segments: vec![],
                    alignment: None,
                },
            )
            .unwrap();
        p.apply(EditCmd::CorrectWords {
            corrections: vec![nuzky_engine::model::WordCorrection {
                asset_id: "take".into(),
                source_start_us: 0,
                original: "Wrong.".into(),
                text: "Correct.".into(),
            }],
        })
        .unwrap();
        let (r, missing) = recap(p.clone(), delete(&p, &[(0, 500_000)]), &store).unwrap();
        assert!(missing.is_empty());
        assert_eq!(r.words[0].text, "Correct.");
        assert_eq!(r.totals().words, Some(1));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

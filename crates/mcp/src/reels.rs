//! Reels from a long video: an agent proposes moments of the transcript, the user picks some, and each
//! picked one becomes a project of its own beside the source, playing the same media files.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use nuzky_analysis::{Range, SceneParams, scene_cuts_in};
use nuzky_engine::{
    Project,
    edit::{EditCmd, MAIN_TRACK, TimeRange, new_id},
    effects::source_time,
    model::{AssetKind, ClipContent, MAX_REEL_HOOK_CHARS, ReelCandidate, ReelStatus},
    speech::{TimelineWord, is_heard},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::transcript::{self, Derived};

pub const DEFAULT_MAX_DURATION_US: i64 = 60_000_000;
const WIDTH: u32 = 1080;
const HEIGHT: u32 = 1920;
/// The blur the inspector's Canvas: Blur starts with.
const BLUR: f32 = 0.5;
/// A shot this short that opens a reel, with words said in it before an original cut of the picture, reads as a
/// flash of another shot.
const OPENING_SLIVER_US: i64 = 600_000;

/// How a reel fills its 9:16 frame from a wider video.
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "ReelFraming"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Framing {
    /// The middle of the picture fills the frame and its sides are cut off.
    #[default]
    Crop,
    /// The whole picture fits the frame, with a blurred copy of it behind.
    Blur,
}

/// A moment an agent proposes; the transcript gives the rest of its candidate.
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReelProposal {
    /// INCLUSIVE word range: from is the first word of a sentence, to the last word of one.
    pub from: usize,
    pub to: usize,
    /// Short, in the video's language, up to 100 characters.
    pub title: String,
    /// Why it works on its own, up to 300 characters.
    pub why: String,
    /// 0 to 1, higher is better.
    pub score: f32,
}

/// A reel project written beside its source.
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MadeReel {
    pub id: String,
    pub path: PathBuf,
    pub duration_us: i64,
    /// Font families of the reel's text.
    #[serde(skip)]
    pub fonts: BTreeSet<String>,
}

/// A full stop, question or exclamation mark, an ellipsis or a pause of 600 ms ends a sentence; a comma does
/// not, so a reel never stops halfway through one.
fn ends_sentence(words: &[TimelineWord], i: usize) -> bool {
    words[i].text.trim_end().ends_with(['.', '!', '?', '…'])
        || words.get(i + 1).is_none_or(|next| next.start_us - words[i].end_us >= transcript::SENTENCE_GAP_US)
}

/// The cut that keeps only the words `from` to `to` as edit_transcript keep does, with pauses shortened to
/// `pause_us`. That cut leaves clips without speech alone, so those before and after the clips that play the kept
/// words go too. With `shots`, which says when to stop, it also looks at the picture where the reel starts and
/// ends and keeps no silent sliver of a shot before or after an original cut there. The pauses it shortens are
/// not looked at: reading the picture around each of them decoded about the whole reel, 25 s for a minute of 4K
/// HEVC.
fn cut(
    project: &Project,
    derived: &Derived,
    from: usize,
    to: usize,
    pause_us: i64,
    shots: Option<&dyn Fn() -> bool>,
) -> Result<transcript::Cut> {
    let mut ranges = transcript::edit_ranges(project, derived, None, Some(&[[from, to]]), Some(pause_us), &[])?;
    if let Some(cancelled) = shots {
        let (first, last) = (derived.words[from].start_us, derived.words[to].end_us);
        let edges: Vec<_> = ranges
            .iter()
            .filter_map(|r| {
                if r.end_us <= first {
                    Some(TimeRange { start_us: r.end_us + 1, end_us: r.end_us + transcript::SLIVER_US })
                } else if r.start_us >= last {
                    Some(TimeRange { start_us: r.start_us - transcript::SLIVER_US, end_us: r.start_us })
                } else {
                    None
                }
            })
            .collect();
        let cuts = picture_cuts(project, &edges, cancelled)?;
        ranges = transcript::without_shot_slivers(project, derived, ranges, &cuts);
    }
    let cut = transcript::plan_cut(project, derived, ranges)?;
    let preview = &cut.preview;
    // A clip playing any part of a word, so a word split across two clips keeps both pieces.
    let speech: Vec<_> = preview
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter().filter(move |clip| is_heard(preview, track, clip)))
        .filter(|clip| {
            let ClipContent::Media { asset_id, source_in_us, speed, .. } = &clip.content else { return false };
            let end = *source_in_us as f64 + clip.duration_us as f64 * f64::from(*speed);
            let words = derived.sources.get(asset_id).map_or(&[][..], Vec::as_slice);
            words.iter().any(|w| (w.start_us as f64) < end && w.end_us > *source_in_us)
        })
        .collect();
    let (Some(start), Some(end)) = (speech.iter().map(|c| c.start_us).min(), speech.iter().map(|c| c.end_us()).max())
    else {
        return Ok(cut);
    };
    let edges: Vec<_> = [(0, start), (end, preview.duration_us())]
        .into_iter()
        .filter(|(start_us, end_us)| end_us > start_us)
        .map(|(start_us, end_us)| TimeRange { start_us, end_us })
        .collect();
    if edges.is_empty() {
        return Ok(cut);
    }
    let reel = Derived { sources: derived.sources.clone(), words: cut.words.clone(), ..Derived::default() };
    transcript::plan_cut(preview, &reel, edges)
}

/// Where the picture of the main track's videos jumps to another shot inside `windows`, in timeline order. A video
/// FFmpeg cannot read has none, so the reel's edges stay where its words put them.
fn picture_cuts(project: &Project, windows: &[TimeRange], cancelled: &dyn Fn() -> bool) -> Result<Vec<i64>> {
    let mut cuts = Vec::new();
    for clip in project.tracks.iter().filter(|t| t.id == MAIN_TRACK).flat_map(|t| &t.clips) {
        let ClipContent::Media { asset_id, source_in_us, speed, .. } = &clip.content else { continue };
        let Some(asset) = project.assets.iter().find(|a| &a.id == asset_id && a.kind == AssetKind::Video) else {
            continue;
        };
        let inside: Vec<_> = windows
            .iter()
            .map(|w| (w.start_us.max(clip.start_us), w.end_us.min(clip.end_us())))
            .filter(|(start, end)| start < end)
            .map(|(start, end)| Range { start_us: source_time(clip, start), end_us: source_time(clip, end) })
            .collect();
        if inside.is_empty() {
            continue;
        }
        match scene_cuts_in(asset, &inside, SceneParams::default(), cancelled) {
            Ok(found) => {
                cuts.extend(found.iter().map(|cut| {
                    clip.start_us + ((cut.time_us - source_in_us) as f64 / f64::from(*speed)).round() as i64
                }))
            }
            Err(error) if cancelled() => return Err(error),
            Err(error) => log::warn!("Reel edges stay at the words, the picture is not readable: {error:#}"),
        }
    }
    cuts.sort_unstable();
    Ok(cuts)
}

/// The first word said after an original cut of the picture in the first `OPENING_SLIVER_US` of the reel from
/// `from` to `to`; `from` when no word is said before such a cut.
fn past_opening_cut(project: &Project, derived: &Derived, from: usize, to: usize) -> Result<usize> {
    let words = &derived.words;
    let start = words[from].start_us - transcript::BEFORE_WORD_US;
    let opening = TimeRange { start_us: start + 1, end_us: start + OPENING_SLIVER_US };
    let Some(cut) = picture_cuts(project, &[opening], &|| false)?.pop() else { return Ok(from) };
    Ok((from..=to).find(|&i| words[i].start_us >= cut).unwrap_or(from))
}

/// The pause length a candidate was measured with.
fn pause_of(candidate: &ReelCandidate) -> i64 {
    candidate.shorten_pauses_us.unwrap_or(transcript::DEFAULT_PAUSE_US)
}

/// Recomputes a candidate's words and cut, with the pause length it was measured with, without changing the project.
pub fn bounds(project: &Project, derived: &Derived, id: &str, from: usize, to: usize) -> Result<ReelCandidate> {
    let candidate =
        project.reel_candidates.iter().find(|c| c.id == id).with_context(|| format!("UNKNOWN_REEL: {id}"))?;
    ensure!(candidate.project_path.is_none(), "REEL_MADE: reel {id} is already made");
    ensure!(derived.untranscribed.is_empty(), "TRANSCRIPT_MISSING: transcribe all heard assets before trimming reels");
    let words = &derived.words;
    ensure!(
        from <= to && to < words.len(),
        "REEL_OUTSIDE_TRANSCRIPT: words {from}-{to} are not in the transcript of {} words",
        words.len()
    );
    let (hook, duration_us) = hook_and_duration(project, derived, from, to, pause_of(candidate))?;
    Ok(ReelCandidate {
        from,
        to,
        start_us: words[from].start_us,
        end_us: words[to].end_us,
        hook,
        duration_us,
        ..candidate.clone()
    })
}

fn hook_and_duration(
    project: &Project,
    derived: &Derived,
    from: usize,
    to: usize,
    pause_us: i64,
) -> Result<(String, i64)> {
    let words = &derived.words;
    let end = (from..=to).find(|&i| ends_sentence(words, i)).unwrap_or(to);
    let hook = words[from..=end].iter().map(|w| w.text.trim()).collect::<Vec<_>>().join(" ");
    let duration_us = cut(project, derived, from, to, pause_us, None)?.preview.duration_us();
    Ok((hook.chars().take(MAX_REEL_HOOK_CHARS).collect(), duration_us))
}

/// Candidates for `proposals`, each inside the transcript, from the start of a sentence to the end of one, apart
/// from each other, and at most `max_duration_us` long once made with pauses shortened to `pause_us`. A candidate
/// whose words run on into an original cut of the picture in its first moments starts at the cut instead, with the
/// first word said after it. Made reels do not count: their words may have moved since.
pub fn plan(
    project: &Project,
    derived: &Derived,
    proposals: &[ReelProposal],
    max_duration_us: i64,
    pause_us: i64,
) -> Result<Vec<ReelCandidate>> {
    ensure!(derived.untranscribed.is_empty(), "TRANSCRIPT_MISSING: transcribe all heard assets before proposing reels");
    ensure!(!proposals.is_empty(), "INVALID_ARGUMENTS: propose at least one reel");
    ensure!(max_duration_us > 0, "INVALID_ARGUMENTS: max_duration_us must be positive");
    let words = &derived.words;
    let mut candidates = Vec::new();
    for p in proposals {
        let (from, to) = (p.from, p.to);
        ensure!(
            from <= to && to < words.len(),
            "REEL_OUTSIDE_TRANSCRIPT: words {from}-{to} are not in the transcript of {} words",
            words.len()
        );
        ensure!(from == 0 || ends_sentence(words, from - 1), "REEL_BOUNDARY: word {from} does not start a sentence");
        ensure!(ends_sentence(words, to), "REEL_BOUNDARY: word {to} {:?} does not end a sentence", words[to].text);
        let from = past_opening_cut(project, derived, from, to)?;
        let (hook, duration_us) = hook_and_duration(project, derived, from, to, pause_us)?;
        ensure!(
            duration_us <= max_duration_us,
            "REEL_TOO_LONG: words {from}-{to} make a {:.1} s reel, longer than {:.1} s",
            duration_us as f64 / 1e6,
            max_duration_us as f64 / 1e6
        );
        candidates.push(ReelCandidate {
            id: new_id(),
            from,
            to,
            start_us: words[from].start_us,
            end_us: words[to].end_us,
            title: p.title.trim().to_owned(),
            hook,
            why: p.why.trim().to_owned(),
            duration_us,
            score: p.score,
            status: ReelStatus::Proposed,
            project_path: None,
            thumbnail: None,
            shorten_pauses_us: Some(pause_us),
        });
    }
    let mut all: Vec<&ReelCandidate> = candidates.iter().collect();
    all.sort_by_key(|c| c.from);
    for pair in all.windows(2) {
        ensure!(
            pair[0].to < pair[1].from,
            "REEL_OVERLAP: words {}-{} and {}-{} overlap",
            pair[0].from,
            pair[0].to,
            pair[1].from,
            pair[1].to
        );
    }
    Ok(candidates)
}

/// Writes a project beside `source` for each candidate in `ids`: only its words, cut as edit_transcript keep
/// cuts with the pause length the candidate was measured with, starting and ending without a silent sliver of a
/// shot at an original cut of the picture, on a 9:16 canvas filled as `framing` says. Returns them with the edits that mark them made in
/// the source, which the caller applies; see [`forget`] when that fails.
pub fn make(
    source: &Path,
    project: &Project,
    derived: &Derived,
    ids: &[String],
    framing: Framing,
    step: &mut dyn FnMut(usize) -> Result<()>,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Vec<MadeReel>, Vec<EditCmd>)> {
    ensure!(!ids.is_empty(), "INVALID_ARGUMENTS: give the ids of the reels to make");
    ensure!(derived.untranscribed.is_empty(), "TRANSCRIPT_MISSING: transcribe all heard assets before making reels");
    let mut seen = HashSet::new();
    let mut planned = Vec::new();
    for id in ids {
        ensure!(seen.insert(id), "INVALID_ARGUMENTS: reel {id} is listed twice");
        let candidate =
            project.reel_candidates.iter().find(|c| &c.id == id).with_context(|| format!("UNKNOWN_REEL: {id}"))?;
        if let Some(path) = &candidate.project_path {
            bail!("REEL_MADE: reel {id} is already made at {path}");
        }
        let words = &derived.words;
        ensure!(
            words.get(candidate.from).is_some_and(|w| w.start_us == candidate.start_us)
                && words.get(candidate.to).is_some_and(|w| w.end_us == candidate.end_us),
            "SPEECH_CHANGED: the video changed since reel {id} was proposed; propose it again"
        );
        let reel = cut(project, derived, candidate.from, candidate.to, pause_of(candidate), None)?.preview;
        ensure!(
            reel.duration_us() == candidate.duration_us,
            "SPEECH_CHANGED: the video changed since reel {id} was proposed; propose it again"
        );
        planned.push(candidate);
    }
    let dir = source.parent().context("INVALID_PROJECT: the project has no directory")?;
    let stem = source.file_stem().context("INVALID_PROJECT: the project has no name")?.to_string_lossy();
    // A made reel's file may have moved away, but its name still belongs to it.
    let named: HashSet<&str> = project.reel_candidates.iter().filter_map(|c| c.project_path.as_deref()).collect();
    let mut made = Vec::new();
    let mut n = 1;
    for (i, candidate) in planned.iter().enumerate() {
        let reel = match step(i).and_then(|()| reel_project(project, derived, candidate, framing, cancelled)) {
            Ok(reel) => reel,
            Err(error) => {
                forget(&made, project);
                return Err(error);
            }
        };
        let path = loop {
            let path = dir.join(format!("{stem}-reel-{n}.nuzky"));
            n += 1;
            if nuzky_session::vacant(&path) && !named.contains(&*path.to_string_lossy()) {
                break path;
            }
        };
        if let Err(error) = nuzky_session::publish_new(&path, &reel) {
            forget(&made, project);
            return Err(error);
        }
        made.push(MadeReel {
            id: candidate.id.clone(),
            path,
            duration_us: reel.duration_us(),
            fonts: crate::fonts::families(&reel).map(str::to_owned).collect(),
        });
    }
    let edits = planned
        .iter()
        .zip(&made)
        .map(|(candidate, made)| EditCmd::UpdateReelCandidate {
            candidate: ReelCandidate {
                status: ReelStatus::Made,
                project_path: Some(made.path.to_string_lossy().into_owned()),
                duration_us: made.duration_us,
                ..(*candidate).clone()
            },
        })
        .collect();
    Ok((made, edits))
}

fn reel_project(
    project: &Project,
    derived: &Derived,
    candidate: &ReelCandidate,
    framing: Framing,
    cancelled: &dyn Fn() -> bool,
) -> Result<Project> {
    let mut reel = cut(project, derived, candidate.from, candidate.to, pause_of(candidate), Some(cancelled))?.preview;
    reel.name = candidate.title.clone();
    reel.reel_candidates.clear();
    reel.thumbnails.clear();
    reel.imported = None;
    frame(&mut reel, framing)?;
    nuzky_session::validate(&reel)?;
    Ok(reel)
}

/// Removes the reels `make` wrote unless `source` names them: a failed save keeps the live edit, so its
/// projects stay, while any other failure leaves the source without them.
pub fn forget(made: &[MadeReel], source: &Project) -> bool {
    let named = |reel: &MadeReel| {
        source.reel_candidates.iter().any(|c| c.project_path.as_deref() == Some(&*reel.path.to_string_lossy()))
    };
    if made.iter().any(named) {
        return false;
    }
    for reel in made {
        let _ = std::fs::remove_file(&reel.path);
    }
    true
}

/// A 9:16 canvas. Cropping scales every main-track picture, keyframes too, until it covers the frame, keeping its
/// own zoom; a wider picture then loses its sides evenly.
fn frame(reel: &mut Project, framing: Framing) -> Result<()> {
    let blur = if framing == Framing::Blur { BLUR } else { 0.0 };
    reel.apply(EditCmd::SetCanvas { width: WIDTH, height: HEIGHT, background: None, background_blur: Some(blur) })?;
    if framing != Framing::Crop {
        return Ok(());
    }
    let assets = reel.assets.clone();
    let main = reel.tracks.iter_mut().find(|t| t.id == MAIN_TRACK).context("INVALID_PROJECT: no main track")?;
    for clip in &mut main.clips {
        let ClipContent::Media { asset_id, transform, .. } = &mut clip.content else { continue };
        let Some(asset) = assets.iter().find(|a| &a.id == asset_id && a.kind != AssetKind::Audio) else { continue };
        if asset.width == 0 || asset.height == 0 {
            continue;
        }
        let (x, y) = (WIDTH as f32 / asset.width as f32, HEIGHT as f32 / asset.height as f32);
        let fill = x.max(y) / x.min(y);
        transform.scale *= fill;
        for key in &mut clip.keyframes {
            key.transform.scale *= fill;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuzky_engine::model::{Asset, Keyframe, Transform};

    const PAUSE: i64 = transcript::DEFAULT_PAUSE_US;

    #[test]
    fn reel_bounds_match_the_plan_and_allow_mid_sentence_words() {
        let (mut project, mut sources) = transcript::tests::fixture();
        for (i, word) in sources.get_mut("talk").unwrap().iter_mut().enumerate() {
            word.start_us = 500_000 + i as i64 * 500_000;
            word.end_us = word.start_us + 400_000;
        }
        let derived = Derived::new(&project, sources, vec![]);
        let proposal = ReelProposal { from: 0, to: 2, title: "First".into(), why: "Why".into(), score: 0.5 };
        project.reel_candidates = plan(&project, &derived, &[proposal], DEFAULT_MAX_DURATION_US, PAUSE).unwrap();
        let original = &project.reel_candidates[0];
        let same = bounds(&project, &derived, &original.id, 0, 2).unwrap();
        assert_eq!((same.duration_us, same.hook), (original.duration_us, original.hook.clone()));
        let trimmed = bounds(&project, &derived, &original.id, 1, 1).unwrap();
        assert_eq!((trimmed.from, trimmed.to, trimmed.start_us, trimmed.end_us), (1, 1, 1_000_000, 1_400_000));
        assert_eq!(trimmed.hook, "word1");
        assert_eq!(trimmed.duration_us, cut(&project, &derived, 1, 1, PAUSE, None).unwrap().preview.duration_us());
        assert_eq!(
            (trimmed.title, trimmed.why, trimmed.score),
            (original.title.clone(), original.why.clone(), original.score)
        );
        assert_eq!(project.reel_candidates[0].from, 0);
    }

    #[test]
    fn stopping_make_after_the_first_reel_leaves_no_file_or_edit() {
        let (mut project, sources) = transcript::tests::fixture();
        let derived = Derived::new(&project, sources, vec![]);
        let proposals: Vec<_> = [(0, 2), (3, 5)]
            .into_iter()
            .map(|(from, to)| ReelProposal { from, to, title: format!("Reel {from}"), why: String::new(), score: 0.5 })
            .collect();
        project.reel_candidates = plan(&project, &derived, &proposals, DEFAULT_MAX_DURATION_US, PAUSE).unwrap();
        let before = serde_json::to_value(&project).unwrap();
        let dir = std::env::temp_dir().join(format!("reels-stop-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ids = project.reel_candidates.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        let result = make(
            &dir.join("source.nuzky"),
            &project,
            &derived,
            &ids,
            Framing::Crop,
            &mut |i| {
                if i == 1 {
                    assert!(dir.join("source-reel-1.nuzky").is_file());
                    bail!("CANCELLED: stopped");
                }
                Ok(())
            },
            &|| false,
        );
        assert!(result.unwrap_err().to_string().starts_with("CANCELLED"));
        assert_eq!(serde_json::to_value(&project).unwrap(), before);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// B-roll before and after the take stays out of a reel, and cropping scales a moving picture too.
    #[test]
    fn a_reel_drops_the_clips_around_its_words_and_crops_keyframes() {
        let (mut project, mut sources) = transcript::tests::fixture();
        let broll = Asset {
            id: "broll".into(),
            name: "broll".into(),
            path: std::env::temp_dir().join("broll.mov").to_string_lossy().into(),
            duration_us: 6_000_000,
            width: 1920,
            height: 1080,
            ..project.assets[0].clone()
        };
        project.apply(EditCmd::AddAssets { assets: vec![broll] }).unwrap();
        project.apply(EditCmd::AddClip { asset_id: "broll".into(), start_us: Some(0), track_id: None }).unwrap();
        project.apply(EditCmd::AddClip { asset_id: "broll".into(), start_us: None, track_id: None }).unwrap();
        sources.insert("broll".into(), vec![]);
        // The take plays from 6 s to 16 s between the two pieces of B-roll, split inside word3 (9.5 to 9.9 s).
        let take = project.tracks[0].clips[1].id.clone();
        project.apply(EditCmd::SplitClip { clip_id: take, at_us: 9_600_000 }).unwrap();
        let derived = Derived::new(&project, sources, vec![]);
        let reel = cut(&project, &derived, 3, 5, PAUSE, None).unwrap().preview;
        let assets: Vec<_> = reel.tracks[0]
            .clips
            .iter()
            .map(|c| match &c.content {
                ClipContent::Media { asset_id, .. } => asset_id.as_str(),
                ClipContent::Text { .. } => "",
            })
            .collect();
        assert!(assets.iter().all(|a| *a == "talk"), "{assets:?}");
        // The words, 80 ms before and 120 ms after them, and two 600 ms pauses down to 300 ms.
        assert_eq!(reel.duration_us(), 2_400_000 + 80_000 + 120_000 - 2 * 300_000);

        let mut wide = project.clone();
        wide.tracks[0].clips[0].keyframes =
            vec![Keyframe { t_us: 0, transform: Transform::default(), ease: Default::default() }];
        frame(&mut wide, Framing::Crop).unwrap();
        let fill = (1920.0 / 1080.0) / (1080.0 / 1920.0);
        let clip = &wide.tracks[0].clips[0];
        let ClipContent::Media { transform, .. } = &clip.content else { panic!() };
        assert!((transform.scale - fill).abs() < 1e-4 && (clip.keyframes[0].transform.scale - fill).abs() < 1e-4);
    }
}

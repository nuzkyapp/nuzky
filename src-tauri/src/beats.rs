//! Beat cuts in the music clip's inspector: the song's beat found as a job, the three rhythms planned as dry runs and
//! the cut made exactly as for agents (`analyze(kind: "beats")`, `cut_to_beats`), as one undo step.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, ensure};
use nuzky_analysis::Beats;
use nuzky_engine::audio::{ensure_pcm, pcm_path};
use nuzky_engine::edit::MAIN_TRACK;
use nuzky_engine::model::{Asset, AssetKind, Clip, ClipContent, Project, TrackKind};
use nuzky_mcp::beats::{self as planner, Every, MIN_CONFIDENCE, Options, Plan};
use nuzky_session::{Expect, host::Host};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::jobs::{Reporter, check_cancelled, crashed, register, unregister};
use crate::{AppState, CmdResult, Snapshot};

/// Each song's beat by its sound cache, which belongs to the file's path, size and time. Kept until Nuzky quits:
/// finding it again takes about 0.2 s once the sound is prepared.
#[derive(Default)]
pub struct BeatCuts {
    found: Mutex<HashMap<PathBuf, Arc<Beats>>>,
    last: Mutex<Option<LastCut>>,
}

/// The last cut made here. Picking another rhythm replaces it while it is the newest step.
#[derive(Clone)]
struct LastCut {
    epoch: String,
    clip_id: String,
    rhythm: Rhythm,
    before: Project,
    after: Project,
    /// The main-track clips it made, so it still counts as the video's cut after later edits.
    made: HashSet<String>,
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Rhythm {
    /// A cut on every bar after the intro.
    Calm,
    /// The planner's default: every bar, then every beat from the first drop.
    Builds,
    /// A cut on every beat after the intro.
    Fast,
}

impl Rhythm {
    const ALL: [Rhythm; 3] = [Rhythm::Calm, Rhythm::Builds, Rhythm::Fast];

    fn every(self) -> Option<Every> {
        match self {
            Rhythm::Calm => Some(Every::Bar),
            Rhythm::Builds => None,
            Rhythm::Fast => Some(Every::Beat),
        }
    }
}

/// The beat of the song's file, for the ticks on its clip: source microseconds, mapped through the clip by the UI.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongBeats {
    /// Every beat heard, from where the beat is steady.
    beats_us: Vec<i64>,
    /// Index into `beats_us` of the first beat that starts a bar; every fourth after it does too.
    first_bar: usize,
    drops: Vec<SongDrop>,
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongDrop {
    at_us: i64,
    /// The drop is where the beat comes in, so it only ends the intro.
    beat_starts: bool,
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RhythmPlan {
    rhythm: Rhythm,
    /// Main-track clips after the cut.
    clips: usize,
    /// Where each new main-track clip starts on the timeline, the first at 0.
    cut_us: Vec<i64>,
    /// Where the cut main track ends.
    end_us: i64,
    /// Where the video ends after the cut: later than `end_us` while titles or overlays run past it.
    project_end_us: i64,
    past_end: PastEnd,
    /// Clips that show footage shown before, because the videos ran out.
    repeated: usize,
    /// Cuts that do not show, because no other video was long enough and the shot before goes on across them.
    continued: usize,
    /// The beat comes in only after the cut ends, so nothing is cut to it.
    beat_after_end: bool,
    captions_removed: usize,
    /// The intro before the beat is cut on bar lines, because no video is as long as it.
    intro_cut: bool,
    /// An earlier rhythm whose cuts are the same.
    same_as: Option<Rhythm>,
    /// Why this rhythm cannot cut, in words for the user.
    error: Option<String>,
}

/// Clips other than the song that still run after the cut main track ends, keeping the video going.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PastEnd {
    titles: usize,
    captions: usize,
    overlays: usize,
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentCut {
    rhythm: Rhythm,
    /// The cut is the newest step, so picking a rhythm replaces it.
    newest: bool,
}

/// What each rhythm would do to the project as it is, or as it was before the cut it would replace.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BeatPlans {
    clip_id: String,
    song: SongBeats,
    /// False when the song has no steady beat to cut to; then there are no rhythms.
    steady: bool,
    /// On the timeline, at the clip's speed.
    bpm: f64,
    /// Where the beat comes in on the timeline; null when that is before the clip starts in the song.
    beat_from_us: Option<i64>,
    /// The drop Builds to the drop turns faster at, on the timeline; null when the song has none after its intro.
    builds_drop_us: Option<i64>,
    /// Distinct videos and pictures on the main track.
    videos: usize,
    pictures: usize,
    /// Where the main track ends now, before cutting.
    footage_end_us: i64,
    song_end_us: i64,
    rhythms: Vec<RhythmPlan>,
    current: Option<CurrentCut>,
    /// The cover shows a frame of the timeline, which will show other footage.
    cover_frame: bool,
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BeatCutDone {
    snapshot: Snapshot,
    clips: usize,
    repeated: usize,
    captions_removed: usize,
}

/// The music clip's file, which must be on disk.
fn song(project: &Project, clip_id: &str) -> Result<Asset> {
    let (_, clip) = planner::music_clip(project, Some(clip_id))?;
    let ClipContent::Media { asset_id, .. } = &clip.content else { unreachable!("music_clip returns media") };
    let asset = project.asset(asset_id).with_context(|| format!("UNKNOWN_ASSET: {asset_id}"))?;
    ensure!(Path::new(&asset.path).is_file(), "MEDIA_MISSING: {}", asset.name);
    Ok(asset.clone())
}

impl BeatCuts {
    fn beats(&self, cache: &Path, asset: &Asset) -> Option<Arc<Beats>> {
        self.found.lock().unwrap().get(&pcm_path(cache, asset)).cloned()
    }

    /// The last cut of this clip in this session, with whether it is the newest step and still in the video.
    fn last(&self, host: &Host, project: &Project, epoch: &str, clip_id: &str) -> Option<(LastCut, bool)> {
        let last = self.last.lock().unwrap().clone().filter(|l| l.epoch == epoch && l.clip_id == clip_id)?;
        let newest = last.after == *project && host.session.peek(false).is_some_and(|p| p.project == last.before);
        let main = project.tracks.iter().find(|t| t.id == MAIN_TRACK)?;
        (newest || main.clips.iter().any(|c| last.made.contains(&c.id))).then_some((last, newest))
    }
}

/// A rhythm's cut of `project` and the project it leaves, checked as the session would check it.
fn dry_run(
    project: &Project,
    beats: &Beats,
    clip_id: &str,
    rhythm: Rhythm,
    keep_sound: bool,
) -> Result<(Plan, Project)> {
    let options = Options {
        music_clip_id: Some(clip_id.into()),
        every: rhythm.every(),
        keep_video_sound: keep_sound,
        ..Options::default()
    };
    let plan = planner::plan(project, beats, &options)?;
    let mut after = project.clone();
    for edit in plan.edits.clone() {
        after.apply(edit).map_err(|e| anyhow!("EDIT_REJECTED: {e:#}"))?;
    }
    nuzky_session::validate(&after)?;
    planner::report(project, &after, &plan)?;
    Ok((plan, after))
}

fn captions_removed(before: &Project, after: &Project) -> usize {
    let kept: HashSet<&str> = after.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.id.as_str())).collect();
    before
        .tracks
        .iter()
        .filter(|t| t.is_captions())
        .flat_map(|t| &t.clips)
        .filter(|c| !kept.contains(c.id.as_str()))
        .count()
}

/// Sound does not keep the video going, so only pictures and text count.
fn past_end(after: &Project, end_us: i64) -> PastEnd {
    let mut past = PastEnd::default();
    for track in after.tracks.iter().filter(|t| t.id != MAIN_TRACK && t.kind != TrackKind::Audio) {
        let n = track.clips.iter().filter(|c| c.end_us() > end_us).count();
        match track.kind {
            TrackKind::Text if track.is_captions() => past.captions += n,
            TrackKind::Text => past.titles += n,
            _ => past.overlays += n,
        }
    }
    past
}

/// Main-track cuts after which the same footage goes on, so the cut does not show.
fn continued(after: &Project) -> usize {
    // The file, and where in it the clip starts and stops; a picture is the same all through.
    fn source<'a>(after: &Project, c: &'a Clip) -> Option<(&'a str, i64, i64)> {
        let ClipContent::Media { asset_id, source_in_us, speed, .. } = &c.content else { return None };
        let picture = after.asset(asset_id).is_some_and(|a| a.kind == AssetKind::Image);
        let used = if picture { 0 } else { (c.duration_us as f64 * *speed as f64).ceil() as i64 };
        Some((asset_id, *source_in_us, *source_in_us + used))
    }
    let Some(main) = after.tracks.iter().find(|t| t.id == MAIN_TRACK) else { return 0 };
    main.clips
        .windows(2)
        .filter(|w| match (source(after, &w[0]), source(after, &w[1])) {
            (Some((a, _, out)), Some((b, at, _))) => a == b && (at - out).abs() <= 1,
            _ => false,
        })
        .count()
}

fn plans(cuts: &BeatCuts, host: &Host, clip_id: &str) -> Result<Option<BeatPlans>> {
    let state = host.session.state()?;
    let project = &state.project;
    let asset = song(project, clip_id)?;
    let Some(beats) = cuts.beats(&host.cache_dir, &asset) else { return Ok(None) };
    let last = cuts.last(host, project, &state.stamp.session_epoch, clip_id);
    // Picking a rhythm replaces the newest cut, so the counts are those of the video before it.
    let base = last.as_ref().filter(|l| l.1).map_or(project, |l| &l.0.before);
    let (_, music) = planner::music_clip(base, Some(clip_id))?;
    let ClipContent::Media { source_in_us, speed, .. } = music.content else {
        unreachable!("music_clip returns media")
    };
    let on_timeline = |s: i64| music.start_us + ((s - source_in_us) as f64 / speed as f64).round() as i64;
    let steady_index = beats.beats_us.iter().position(|&b| b == beats.steady_from_us).unwrap_or(0);
    let heard = beats.beats_us.get(steady_index..).unwrap_or_default().to_vec();
    let first_bar = (beats.first_downbeat as isize - steady_index as isize).rem_euclid(4) as usize;
    let song = SongBeats {
        beats_us: heard,
        first_bar,
        drops: beats
            .drops
            .iter()
            .map(|d| SongDrop { at_us: d.at_us, beat_starts: d.beat < steady_index + 4 })
            .collect(),
    };
    let steady = beats.confidence >= MIN_CONFIDENCE && !beats.beats_us.is_empty();
    let main = base.tracks.iter().find(|t| t.id == MAIN_TRACK).context("INVALID_PROJECT: no main track")?;
    let kinds: HashMap<&str, AssetKind> = main
        .clips
        .iter()
        .filter_map(|c| match &c.content {
            ClipContent::Media { asset_id, .. } => base.asset(asset_id).map(|a| (asset_id.as_str(), a.kind)),
            ClipContent::Text { .. } => None,
        })
        .collect();
    let mut rhythms: Vec<RhythmPlan> = Vec::new();
    if steady {
        for rhythm in Rhythm::ALL {
            let plan = match dry_run(base, &beats, clip_id, rhythm, false) {
                Ok((plan, after)) => {
                    let beat_from = on_timeline(beats.steady_from_us);
                    let same_as = rhythms
                        .iter()
                        .find(|r| r.error.is_none() && r.cut_us == plan.cut_us && r.end_us == plan.end_us)
                        .map(|r| r.rhythm);
                    RhythmPlan {
                        rhythm,
                        clips: plan.cut_us.len(),
                        intro_cut: plan.cut_us.iter().any(|&t| t > 0 && t < beat_from - 1),
                        captions_removed: captions_removed(base, &after),
                        project_end_us: after.duration_us(),
                        past_end: past_end(&after, plan.end_us),
                        continued: continued(&after),
                        beat_after_end: beat_from >= plan.end_us,
                        cut_us: plan.cut_us,
                        end_us: plan.end_us,
                        repeated: plan.repeated,
                        same_as,
                        error: None,
                    }
                }
                Err(error) => RhythmPlan {
                    rhythm,
                    clips: 0,
                    cut_us: Vec::new(),
                    end_us: 0,
                    project_end_us: 0,
                    past_end: PastEnd::default(),
                    repeated: 0,
                    continued: 0,
                    beat_after_end: false,
                    captions_removed: 0,
                    intro_cut: false,
                    same_as: None,
                    error: Some(explain(error)),
                },
            };
            rhythms.push(plan);
        }
    }
    let in_song = |s: i64| s >= source_in_us && on_timeline(s) < music.end_us();
    let builds_drop = beats
        .drops
        .iter()
        .find(|d| d.beat >= steady_index + 4)
        .filter(|d| in_song(d.at_us))
        .map(|d| on_timeline(d.at_us));
    Ok(Some(BeatPlans {
        clip_id: clip_id.into(),
        song,
        steady,
        bpm: (beats.bpm * speed as f64).round(),
        beat_from_us: in_song(beats.steady_from_us).then(|| on_timeline(beats.steady_from_us)),
        builds_drop_us: builds_drop,
        videos: kinds.values().filter(|&&k| k == AssetKind::Video).count(),
        pictures: kinds.values().filter(|&&k| k == AssetKind::Image).count(),
        footage_end_us: main.clips.iter().map(|c| c.end_us()).max().unwrap_or(0),
        song_end_us: music.end_us(),
        rhythms,
        cover_frame: base.thumbnails.iter().any(|t| t.image.is_none()),
        current: last.as_ref().map(|(l, newest)| CurrentCut { rhythm: l.rhythm, newest: *newest }),
    }))
}

/// Cuts the main track to the song in `rhythm` as one undo step. While this clip's last cut is the newest step the new
/// cut takes its place, so one undo still leads back to the video before both.
fn cut(cuts: &BeatCuts, host: &Host, clip_id: &str, rhythm: Rhythm, keep_sound: bool) -> Result<(usize, usize, usize)> {
    let session = &host.session;
    let state = session.state()?;
    let asset = song(&state.project, clip_id)?;
    let beats = cuts.beats(&host.cache_dir, &asset).context("BEATS_UNKNOWN: find the beat first")?;
    ensure!(beats.confidence >= MIN_CONFIDENCE, "NO_STEADY_BEAT: this song has no steady beat");
    let epoch = state.stamp.session_epoch.clone();
    let replace = cuts.last(host, &state.project, &epoch, clip_id).filter(|l| l.1).map(|l| l.0);
    let base = replace.as_ref().map_or(&state.project, |l| &l.before);
    let (plan, after) = dry_run(base, &beats, clip_id, rhythm, keep_sound)?;
    let expect = Expect { revision: Some(state.stamp.revision), speech_layout_key: None };
    match &replace {
        Some(last) => session.replace_newest(&last.before, plan.edits.clone(), expect)?,
        None => session.edit(plan.edits.clone(), None, expect)?,
    };
    let captions = captions_removed(base, &after);
    let main = after.tracks.iter().find(|t| t.id == MAIN_TRACK).context("INVALID_PROJECT: no main track")?;
    let made = main.clips.iter().map(|c| c.id.clone()).collect();
    let before = base.clone();
    *cuts.last.lock().unwrap() = Some(LastCut { epoch, clip_id: clip_id.into(), rhythm, before, after, made });
    Ok((plan.cut_us.len(), plan.repeated, captions))
}

/// The planner's codes, in words for the user.
fn explain(error: anyhow::Error) -> String {
    let text = format!("{error:#}");
    match text.split_once(':').map(|(code, _)| code) {
        Some("NO_FOOTAGE") => "Put your videos on the main track first, then cut them to the beat.".into(),
        Some("MUSIC_AFTER_END") => "The song starts after your videos end. Move it earlier to cut to it.".into(),
        Some("SHOT_TOO_SHORT") => {
            "Your videos are too short for this rhythm. Pick one that cuts more often, or add longer videos.".into()
        }
        Some("NO_STEADY_BEAT") => {
            "The part of the song on the timeline has no beat to cut to. Make the clip longer.".into()
        }
        Some("STALE_REVISION") => "The project changed meanwhile. Pick the rhythm again.".into(),
        Some("MEDIA_MISSING") => "The song's file is missing. Relink it, then try again.".into(),
        Some("INVALID_ARGUMENTS") => "This clip has no sound to find a beat in.".into(),
        _ => text,
    }
}

/// Finds the beat of the song under `clip_id` as a `beats:<asset>` job: prepares its sound if needed, then listens.
/// Returns the job, or null when the beat is already known.
#[tauri::command]
pub fn find_beats(app: AppHandle, clip_id: String, expected_epoch: Option<String>) -> CmdResult<Option<String>> {
    let state = app.state::<AppState>();
    let project = {
        let current = crate::lock_session(&state.session, expected_epoch.as_deref())?;
        current.host.session.state().map_err(crate::err)?.project
    };
    let asset = song(&project, &clip_id).map_err(explain)?;
    let cache = state.cache_dir.clone();
    if state.beat_cuts.beats(&cache, &asset).is_some() {
        return Ok(None);
    }
    let id = format!("beats:{}", asset.id);
    // Already listening to this song: the same job.
    let Some(cancel) = register(&app, &id) else { return Ok(Some(id)) };
    let (worker, job) = (app.clone(), id.clone());
    let spawned = std::thread::Builder::new().name("beats".into()).spawn(move || {
        let mut rep = Reporter::new(&worker, &job, "beats", format!("Finding the beat in {}", asset.name));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Preparing the sound is most of the wait; listening to it takes a fraction of a second.
            ensure_pcm(&cache, &asset, |p| {
                rep.progress(p * 0.95, None);
                check_cancelled(&cancel)
            })?;
            rep.progress(0.95, None);
            let beats = nuzky_analysis::beats_cancellable(&asset, &cache, || cancel.load(Ordering::Relaxed))?;
            check_cancelled(&cancel)?;
            let state = worker.state::<AppState>();
            state.beat_cuts.found.lock().unwrap().insert(pcm_path(&cache, &asset), Arc::new(beats));
            Ok(None)
        }))
        .unwrap_or_else(|p| Err(crashed(&p)));
        rep.finish(result, cancel.load(Ordering::Relaxed));
        unregister(&worker, &job);
    });
    if let Err(error) = spawned {
        unregister(&app, &id);
        return Err(format!("Finding the beat: {error}"));
    }
    Ok(Some(id))
}

/// What each rhythm would cut; null while the song's beat is not known yet.
#[tauri::command]
pub async fn beat_plans(
    app: AppHandle,
    clip_id: String,
    expected_epoch: Option<String>,
) -> CmdResult<Option<BeatPlans>> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let host = crate::lock_session(&state.session, expected_epoch.as_deref())?.host.clone();
        plans(&state.beat_cuts, &host, &clip_id).map_err(explain)
    })
    .await
    .map_err(crate::err)?
}

#[tauri::command]
pub async fn cut_to_beat(
    app: AppHandle,
    clip_id: String,
    rhythm: Rhythm,
    keep_video_sound: bool,
    expected_epoch: Option<String>,
) -> CmdResult<BeatCutDone> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let current = crate::lock_session(&state.session, expected_epoch.as_deref())?;
        let (clips, repeated, captions_removed) =
            cut(&state.beat_cuts, &current.host, &clip_id, rhythm, keep_video_sound).map_err(explain)?;
        Ok(BeatCutDone { snapshot: current.snapshot(vec![clip_id])?, clips, repeated, captions_removed })
    })
    .await
    .map_err(crate::err)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuzky_analysis::{BeatDrop, Section};
    use nuzky_engine::edit::EditCmd;
    use nuzky_engine::model::TextStyle;

    fn asset(id: &str, kind: AssetKind, duration_us: i64) -> Asset {
        Asset {
            id: id.into(),
            name: id.into(),
            path: format!("/{id}"),
            kind,
            duration_us,
            width: 1080,
            height: 1920,
            fps: 30.0,
            has_audio: true,
            rotation: 0,
            mirror: false,
            credit: None,
            part: None,
        }
    }

    /// The planner's own case: two 20 s takes, where the second goes on across the last three cuts, a 60 s song and
    /// titles at 2 s, 39 s and 45 s, the last two past where the cut ends.
    #[test]
    fn the_plan_counts_cuts_that_do_not_show_and_titles_that_keep_the_video_going() {
        let mut p = Project::new("beats");
        let assets = vec![
            asset("red", AssetKind::Video, 20_000_000),
            asset("blue", AssetKind::Video, 20_000_000),
            asset("song", AssetKind::Audio, 60_000_000),
        ];
        p.apply(EditCmd::AddAssets { assets }).unwrap();
        for id in ["red", "blue", "song"] {
            p.apply(EditCmd::AddClip { asset_id: id.into(), start_us: (id == "song").then_some(0), track_id: None })
                .unwrap();
        }
        let style: TextStyle =
            serde_json::from_value(serde_json::json!({"fontSize": 60.0, "color": "#ffffff"})).unwrap();
        for start_us in [2_000_000, 39_000_000, 45_000_000] {
            p.apply(EditCmd::AddText { start_us, text: "Hi".into(), style: style.clone() }).unwrap();
        }
        let song = p.tracks.iter().find(|t| t.kind == TrackKind::Audio).unwrap().clips[0].id.clone();
        let beats_us: Vec<i64> = (0..90).map(|i| 333_333 + i * 600_000).collect();
        let beats = Beats {
            bpm: 100.0,
            confidence: 0.9,
            steady_from_us: beats_us[4],
            first_downbeat: 0,
            drops: vec![BeatDrop { beat: 12, at_us: beats_us[12], rise_db: 9.0 }],
            sections: vec![Section { beat: 0, at_us: beats_us[0], level_db: -20.0 }],
            beats_us,
        };
        let (plan, after) = dry_run(&p, &beats, &song, Rhythm::Builds, false).unwrap();
        assert!(
            plan.warnings.iter().any(|w| w.starts_with("no other shot was long enough, so 3 of")),
            "{:?}",
            plan.warnings
        );
        assert_eq!(continued(&after), 3);
        let past = past_end(&after, plan.end_us);
        assert_eq!((past.titles, past.captions, past.overlays), (2, 0, 0));
        assert_eq!(after.duration_us(), 48_000_000);
        assert!(plan.end_us < 39_000_000, "{}", plan.end_us);
    }
}

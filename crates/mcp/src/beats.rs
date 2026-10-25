//! Cutting the main track to the beat of a song (`cut_to_beats`): where each piece starts, which footage fills
//! it, and the edits of existing commands that make it. Titles, overlays and the music keep their time.

use std::collections::HashSet;

use anyhow::{Context, Result, bail, ensure};
use nuzky_analysis::Beats;
use nuzky_engine::{
    Project,
    edit::{CopiedClip, EditCmd, MAIN_TRACK, TimeRange, media, stays_in_place},
    export::frame_time_us,
    model::{AssetKind, Clip, ClipContent, Track, TrackKind, Transform},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// How often the picture changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub enum Every {
    #[serde(rename = "beat")]
    Beat,
    #[serde(rename = "2_beats")]
    TwoBeats,
    #[serde(rename = "bar")]
    Bar,
    #[serde(rename = "2_bars")]
    TwoBars,
    #[serde(rename = "4_bars")]
    FourBars,
    /// One piece until the next change.
    #[serde(rename = "none")]
    None,
}

/// Where a change of rhythm starts: "drop", the song's first drop, or a beat index of analyze(kind: "beats").
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ChangeAt {
    Beat(usize),
    Named(Named),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Named {
    Drop,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub from: ChangeAt,
    pub every: Every,
}

/// Footage to cut from: a span of a video, in source microseconds, or a picture.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Shot {
    pub asset_id: String,
    pub source_in_us: i64,
    /// The end of the span; the end of the file, or of the part the user chose, when left out.
    pub source_out_us: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub music_clip_id: Option<String>,
    pub every: Option<Every>,
    pub changes: Vec<Change>,
    pub shots: Option<Vec<Shot>>,
    pub end_us: Option<i64>,
    pub keep_video_sound: bool,
}

/// A cut planned against the project as it is.
#[derive(Clone, Debug)]
pub struct Plan {
    pub edits: Vec<EditCmd>,
    /// Where each new main-track clip starts, the first at 0.
    pub cut_us: Vec<i64>,
    pub end_us: i64,
    pub music_clip_id: String,
    pub trimmed_to_us: Option<i64>,
    pub fade_out_us: i64,
    pub shots_used: usize,
    pub repeated: usize,
    pub warnings: Vec<String>,
    /// The song's beat on the timeline, for the answer.
    pub song: Value,
}

/// Under this the sound has no beat worth cutting to.
pub const MIN_CONFIDENCE: f64 = 0.3;
/// A piece is at least this many frames long; a cut closer to another is left out.
const MIN_FRAMES: i64 = 2;
/// The music fades out over its last bar, at most this long.
const MAX_FADE_US: i64 = 2_000_000;
/// A last section this much quieter than the one before is the song's quiet ending, which the video does not run into.
const QUIET_END_DB: f64 = 12.0;

struct Source {
    from: i64,
    /// None for a picture, which never runs out.
    to: Option<i64>,
    speed: f64,
    content: ClipContent,
}

/// The song to cut to: the clip named, else the only clip on an audio track that keeps its place.
pub fn music_clip<'a>(project: &'a Project, id: Option<&str>) -> Result<(&'a Track, &'a Clip)> {
    let clips = |keep: fn(&Track) -> bool| {
        project.tracks.iter().filter(move |t| keep(t)).flat_map(|t| t.clips.iter().map(move |c| (t, c)))
    };
    let name = |c: &Clip| match &c.content {
        ClipContent::Media { asset_id, .. } => project.asset(asset_id).map_or(asset_id.clone(), |a| a.name.clone()),
        ClipContent::Text { .. } => String::new(),
    };
    let list = |found: Vec<(&Track, &Clip)>| {
        found
            .iter()
            .map(|(_, c)| format!("{} ({}, at {} us)", c.id, name(c), c.start_us))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let (track, clip) = match id {
        Some(id) => clips(|_| true).find(|(_, c)| c.id == id).with_context(|| format!("UNKNOWN_CLIP: {id}"))?,
        None => {
            let kept: Vec<_> = clips(stays_in_place).collect();
            match kept.as_slice() {
                [one] => *one,
                [] => {
                    let audio: Vec<_> = clips(|t| t.kind == TrackKind::Audio).collect();
                    ensure!(
                        audio.is_empty(),
                        "MUSIC_MISSING: no song on a music track; pass music_clip_id of one of these audio clips: {}",
                        list(audio)
                    );
                    bail!("MUSIC_MISSING: add the song first with add_sound or import_media")
                }
                _ => bail!("MUSIC_AMBIGUOUS: several songs; pass music_clip_id, one of: {}", list(kept)),
            }
        }
    };
    let sound = match &clip.content {
        ClipContent::Media { asset_id, .. } => project.asset(asset_id).is_some_and(nuzky_engine::audio::has_audio),
        ClipContent::Text { .. } => false,
    };
    ensure!(
        track.kind == TrackKind::Audio && sound,
        "INVALID_ARGUMENTS: music_clip_id must name a clip with sound on an audio track"
    );
    Ok((track, clip))
}

/// The cut of the main track to `beats`, the analysis of the music clip's file.
pub fn plan(project: &Project, beats: &Beats, options: &Options) -> Result<Plan> {
    let (music_track, music) = music_clip(project, options.music_clip_id.as_deref())?;
    let ClipContent::Media { source_in_us: music_in, speed: music_speed, .. } = music.content else {
        unreachable!("music_clip returns media")
    };
    let main = project.tracks.iter().find(|t| t.id == MAIN_TRACK).context("INVALID_PROJECT: no main track")?;
    let fps = project.canvas.fps.max(1);
    let frame = |t: i64| ((2 * t.max(0) as i128 * fps as i128 + 1_000_000) / 2_000_000) as i64;
    let time = |f: i64| frame_time_us(f.max(0) as u64, fps);
    let speed = music_speed as f64;
    let on_timeline = |s: i64| music.start_us + ((s - music_in) as f64 / speed).round() as i64;
    let mapped: Vec<(usize, i64)> = beats
        .beats_us
        .iter()
        .enumerate()
        .filter(|&(_, &s)| s >= music_in)
        .map(|(i, &s)| (i, on_timeline(s)))
        .filter(|&(_, t)| t < music.end_us())
        .collect();
    ensure!(!mapped.is_empty(), "NO_STEADY_BEAT: the part of the song on the timeline has no beat");
    let steady = beats.beats_us.iter().position(|&b| b == beats.steady_from_us).unwrap_or(0);
    let down = beats.first_downbeat;
    let is_down = |i: usize| i >= down && (i - down).is_multiple_of(4);

    let first_drop = beats.drops.first().map(|d| d.beat);
    let mut rhythm: Vec<(usize, Every, bool)> = vec![(0, Every::None, false)];
    if options.every.is_none() && options.changes.is_empty() {
        rhythm.push((steady, Every::Bar, true));
        // A drop where the beat comes in only ends the intro.
        let drop = beats.drops.iter().map(|d| d.beat).find(|&beat| beat >= steady + 4);
        rhythm.extend(drop.map(|beat| (beat, Every::Beat, true)));
    } else {
        let mut changes = Vec::new();
        for change in &options.changes {
            let beat = match change.from {
                ChangeAt::Beat(beat) => {
                    ensure!(beat < beats.beats_us.len(), "INVALID_ARGUMENTS: beat {beat} is past the song's last beat");
                    beat
                }
                ChangeAt::Named(Named::Drop) => {
                    first_drop.context("NO_DROP: the song has no drop; start the change at a beat index")?
                }
            };
            changes.push((beat, change.every, true));
        }
        // A change holds until the next one, so the rhythm from where the beat is steady only leads up to the first.
        if changes.iter().all(|c| c.0 > steady) {
            rhythm.push((steady, options.every.unwrap_or(Every::Bar), true));
        }
        rhythm.extend(changes);
    }
    rhythm.sort_by_key(|r| r.0);
    let cuts_at = |i: usize| {
        let &(from, every, cut) = rhythm.iter().rev().find(|r| r.0 <= i).unwrap();
        let first_bar = from + (down as isize - from as isize).rem_euclid(4) as usize;
        let bar = |n: usize| is_down(i) && i >= first_bar && ((i - first_bar) / 4).is_multiple_of(n);
        (i == from && cut)
            || match every {
                Every::Beat => true,
                Every::TwoBeats => (i - from).is_multiple_of(2),
                Every::Bar => bar(1),
                Every::TwoBars => bar(2),
                Every::FourBars => bar(4),
                Every::None => false,
            }
    };

    let sources = sources(project, main, options.shots.as_deref())?;
    let footage = sources.iter().map(|s| s.to.map(|to| ((to - s.from) as f64 / s.speed) as i64)).sum::<Option<i64>>();
    let music_end = music.end_us();
    let (target, cap) = match options.end_us {
        Some(end) => {
            ensure!(
                end > 0 && end <= music_end,
                "INVALID_ARGUMENTS: end_us must be inside the song, which ends at {music_end} us"
            );
            (end, music_end)
        }
        None => {
            let quiet_end = match beats.sections.as_slice() {
                [.., before, last] if last.level_db <= before.level_db - QUIET_END_DB && last.at_us > music_in => {
                    on_timeline(last.at_us)
                }
                _ => i64::MAX,
            };
            let limit = footage.map_or(music_end, |f| f.min(music_end)).min(quiet_end);
            let last_bar = mapped.iter().rev().find(|&&(i, t)| is_down(i) && t > 0 && t <= limit).map(|m| m.1);
            (last_bar.unwrap_or(limit), limit)
        }
    };
    let mut end = frame(target);
    while end > 0 && time(end) > cap {
        end -= 1;
    }
    ensure!(
        time(end) > music.start_us,
        "MUSIC_AFTER_END: the song starts at {} us, after the footage ends at {} us; give end_us or more shots",
        music.start_us,
        time(end)
    );
    ensure!(end >= MIN_FRAMES, "INVALID_ARGUMENTS: the cut would be shorter than {MIN_FRAMES} frames");

    let mut frames = vec![0];
    for &(i, t) in &mapped {
        let f = frame(t);
        if cuts_at(i) && f < end && f - frames.last().unwrap() >= MIN_FRAMES {
            frames.push(f);
        }
    }
    if frames.len() > 1 && end - frames.last().unwrap() < MIN_FRAMES {
        frames.pop();
    }
    let mut warnings = Vec::new();
    let fits = |from: i64, to: i64| {
        let duration = time(to) - time(from);
        sources.iter().any(|s| s.to.is_none_or(|to| (duration as f64 * s.speed).ceil() as i64 <= to - s.from))
    };
    let bar_lines: Vec<i64> = mapped.iter().filter(|m| is_down(m.0)).map(|m| frame(m.1)).collect();
    let steady_frame = mapped.iter().find(|m| m.0 == steady).map(|m| frame(m.1));
    let planned = std::mem::take(&mut frames);
    for (n, &from) in planned.iter().enumerate() {
        let to = planned.get(n + 1).copied().unwrap_or(end);
        frames.push(from);
        if fits(from, to) {
            continue;
        }
        // No shot is that long: the piece changes every 2^n bars instead, the longest pieces the shots can fill,
        // counted back from where it ends.
        let inside: Vec<i64> =
            bar_lines.iter().copied().filter(|&f| f - from >= MIN_FRAMES && to - f >= MIN_FRAMES).collect();
        let mut bars = inside.len().next_power_of_two();
        let cuts = loop {
            let mut cuts: Vec<i64> = inside.iter().rev().skip(bars - 1).step_by(bars).copied().collect();
            cuts.reverse();
            let bounds: Vec<i64> = std::iter::once(from).chain(cuts.iter().copied()).chain([to]).collect();
            if bars == 1 || bounds.windows(2).all(|w| fits(w[0], w[1])) {
                break cuts;
            }
            bars /= 2;
        };
        if cuts.is_empty() {
            continue;
        }
        let piece = if from == 0 && Some(to) == steady_frame {
            "the intro".to_string()
        } else {
            format!("the piece from {} us", time(from))
        };
        warnings.push(format!(
            "no shot is as long as {piece} until {} us, so it changes every {bars} bar{} there; a shot that long keeps it one piece",
            time(to),
            if bars == 1 { "" } else { "s" }
        ));
        frames.extend(cuts);
    }
    let cut_us: Vec<i64> = frames.iter().map(|&f| time(f)).collect();
    let end_us = time(end);
    let pieces: Vec<(i64, i64)> = cut_us
        .iter()
        .zip(cut_us.iter().skip(1).chain([&end_us]))
        .map(|(&start, &next)| (start, next - start))
        .collect();

    let (clips, used, repeated, continued) = fill(&sources, &pieces, options.keep_video_sound)?;
    if repeated > 0 {
        warnings.push(format!(
            "footage repeats: {repeated} of {} pieces show footage shown before; give more shots or an earlier end_us",
            pieces.len()
        ));
    }
    if sources.len() == 1 && pieces.len() > 1 {
        warnings.push(
            "only one shot: each piece continues where the last stopped, so the cuts do not show; give more shots"
                .into(),
        );
    } else if continued > 0 {
        warnings.push(format!(
            "no other shot was long enough, so {continued} of {} pieces continue the shot before them and those cuts do not show; give more or longer shots",
            pieces.len()
        ));
    }
    if let Some(&(_, t)) = mapped.iter().find(|m| m.0 == steady)
        && t >= end_us
    {
        warnings.push(format!(
            "the beat comes in at {t} us, after the end at {end_us} us, so nothing is cut to it; give end_us or more shots"
        ));
    }
    if project.thumbnails.iter().any(|t| t.image.is_none()) {
        warnings.push(
            "a cover shows a frame of the timeline, which now shows other footage there; choose its frame again".into(),
        );
    }

    let old_end = main.clips.iter().map(Clip::end_us).max().unwrap_or(0);
    let length = end_us;
    let mut edits = Vec::new();
    if !music_track.keep_in_place {
        edits.push(EditCmd::UpdateTrack {
            track_id: music_track.id.clone(),
            muted: None,
            hidden: None,
            keep_in_place: Some(true),
        });
    }
    edits.push(EditCmd::PasteClips { clips, at_us: old_end });
    if old_end > 0 {
        edits.push(EditCmd::RippleDeleteRanges {
            ranges: vec![TimeRange { start_us: 0, end_us: old_end }],
            keep_track_ids: Some(
                project
                    .tracks
                    .iter()
                    .filter(|t| t.id != MAIN_TRACK && !t.is_captions())
                    .map(|t| t.id.clone())
                    .collect(),
            ),
        });
    }
    // Pasting opened the new footage's time at the old end; what was there or after it goes back where it was.
    let mut moved = Vec::new();
    let mut split = Vec::new();
    for track in &project.tracks {
        if track.id == MAIN_TRACK
            || track.id == music_track.id
            || stays_in_place(track)
            || old_end > 0 && track.is_captions()
        {
            continue;
        }
        for clip in &track.clips {
            if clip.start_us >= old_end {
                moved.push(clip.id.clone());
            } else if clip.end_us() > old_end {
                let still = match &clip.content {
                    ClipContent::Text { .. } => true,
                    ClipContent::Media { asset_id, .. } => {
                        project.asset(asset_id).is_some_and(|a| a.kind == AssetKind::Image)
                    }
                };
                if still {
                    // A title or picture across the old end grew instead.
                    edits.push(EditCmd::TrimClip {
                        clip_id: clip.id.clone(),
                        start_us: clip.start_us,
                        duration_us: clip.duration_us,
                        source_in_us: None,
                    });
                } else {
                    split.push(clip.id.clone());
                }
            }
        }
    }
    if !moved.is_empty() {
        edits.push(EditCmd::MoveClips { clip_ids: moved, offset_us: -length });
    }
    if !split.is_empty() {
        warnings.push(format!(
            "these clips ran across the end of the old main track and were split there; their second part now starts {length} us later: {}",
            split.join(", ")
        ));
    }
    let beat_us = 60e6 / beats.bpm.max(1.0) / speed;
    let (trimmed_to_us, fade_out_us) = if music_end > end_us {
        let fade = ((4.0 * beat_us) as i64).min(MAX_FADE_US);
        edits.push(EditCmd::TrimClip {
            clip_id: music.id.clone(),
            start_us: music.start_us,
            duration_us: end_us - music.start_us,
            source_in_us: None,
        });
        edits.push(fade_out(&music.id, fade));
        (Some(end_us), fade.min((end_us - music.start_us) / 2))
    } else {
        (None, 0)
    };
    let in_range = |s: i64| s >= music_in && on_timeline(s) < music_end;
    let song = json!({
        "bpm": (beats.bpm * speed * 100.0).round() / 100.0,
        "confidence": beats.confidence,
        "steady_from_us": in_range(beats.steady_from_us).then(|| on_timeline(beats.steady_from_us)),
        "drops": beats.drops.iter().filter(|d| in_range(d.at_us))
            .map(|d| json!({"beat": d.beat, "at_us": on_timeline(d.at_us), "rise_db": d.rise_db})).collect::<Vec<_>>(),
        // Each section heard on the timeline, the first from where the clip starts.
        "sections": beats.sections.iter().enumerate()
            .filter(|(n, s)| on_timeline(s.at_us) < music_end && beats.sections.get(n + 1).is_none_or(|next| next.at_us > music_in))
            .map(|(_, s)| json!({"beat": s.beat, "at_us": on_timeline(s.at_us).max(music.start_us), "level_db": s.level_db}))
            .collect::<Vec<_>>(),
    });
    Ok(Plan {
        edits,
        cut_us,
        end_us,
        music_clip_id: music.id.clone(),
        trimmed_to_us,
        fade_out_us,
        shots_used: used,
        repeated,
        warnings,
        song,
    })
}

fn fade_out(clip_id: &str, fade_out_us: i64) -> EditCmd {
    EditCmd::UpdateClip {
        clip_id: clip_id.into(),
        transform: None,
        volume: None,
        text: None,
        style: None,
        speed: None,
        keep_pitch: None,
        adjust: None,
        fade_in_us: None,
        fade_out_us: Some(fade_out_us),
        clean_voice: None,
        dereverb: None,
        normalize: None,
        leveler: None,
        shape: None,
        duck_db: None,
        background: None,
    }
}

/// The footage: the shots given, else the main track's clips in order, each keeping its look and speed.
fn sources(project: &Project, main: &Track, shots: Option<&[Shot]>) -> Result<Vec<Source>> {
    let look = |asset_id: &str| {
        main.clips.iter().find_map(|c| match &c.content {
            ClipContent::Media { asset_id: id, .. } if id == asset_id => Some(c.content.clone()),
            _ => None,
        })
    };
    let source = |asset_id: &str, from: i64, to: Option<i64>, content: ClipContent| {
        let ClipContent::Media { speed, .. } = content else { unreachable!("footage is media") };
        let asset = project.asset(asset_id).with_context(|| format!("UNKNOWN_ASSET: {asset_id}"))?;
        let to = (asset.kind != AssetKind::Image).then(|| to.unwrap_or(asset.duration_us).min(asset.duration_us));
        Ok(Source { from, to, speed: speed as f64, content })
    };
    let found: Vec<Source> = match shots {
        Some(shots) => {
            ensure!(!shots.is_empty(), "INVALID_ARGUMENTS: give at least one shot, or leave shots out");
            shots
                .iter()
                .map(|shot| {
                    let asset =
                        project.asset(&shot.asset_id).with_context(|| format!("UNKNOWN_ASSET: {}", shot.asset_id))?;
                    ensure!(
                        asset.kind != AssetKind::Audio,
                        "INVALID_ARGUMENTS: shot {} is a sound, not footage",
                        shot.asset_id
                    );
                    let end = asset.part.map_or(asset.duration_us, |p| p.end_us);
                    let to = shot.source_out_us.unwrap_or(end);
                    ensure!(
                        asset.kind == AssetKind::Image
                            || (0 <= shot.source_in_us && shot.source_in_us < to && to <= asset.duration_us),
                        "INVALID_ARGUMENTS: shot {} must lie inside its file, 0..{} us",
                        shot.asset_id,
                        asset.duration_us
                    );
                    let content = look(&shot.asset_id).unwrap_or_else(|| media(asset, Transform::default(), None));
                    source(&shot.asset_id, shot.source_in_us, Some(to), content)
                })
                .collect::<Result<_>>()?
        }
        None => main
            .clips
            .iter()
            .filter_map(|c| match &c.content {
                ClipContent::Media { asset_id, source_in_us, speed, .. } => Some(source(
                    asset_id,
                    *source_in_us,
                    Some(source_in_us + (c.duration_us as f64 * *speed as f64).round() as i64),
                    c.content.clone(),
                )),
                ClipContent::Text { .. } => None,
            })
            .collect::<Result<_>>()?,
    };
    ensure!(!found.is_empty(), "NO_FOOTAGE: the main track is empty; pass shots");
    Ok(found)
}

/// Main-track clips for the pieces: the shots in turn, each continuing where it stopped, never the same shot twice
/// in a row while another can fill the piece. When none can, the same shot goes on; when every shot has run out they
/// all start again.
fn fill(sources: &[Source], pieces: &[(i64, i64)], keep_sound: bool) -> Result<(Vec<CopiedClip>, usize, usize, usize)> {
    let mut cursor: Vec<i64> = sources.iter().map(|s| s.from).collect();
    let (mut next, mut previous, mut wrapped, mut repeated, mut continued) = (0, None, false, 0, 0);
    let mut used = HashSet::new();
    let mut clips = Vec::with_capacity(pieces.len());
    for &(start, duration) in pieces {
        let need = |s: &Source| if s.to.is_none() { 0 } else { (duration as f64 * s.speed).ceil() as i64 };
        let fits = |k: usize, cursor: &[i64]| sources[k].to.is_none_or(|to| cursor[k] + need(&sources[k]) <= to);
        let pick = |cursor: &[i64], other: bool| {
            (0..sources.len())
                .map(|o| (next + o) % sources.len())
                .find(|&k| (!other || sources.len() == 1 || Some(k) != previous) && fits(k, cursor))
        };
        let k = match pick(&cursor, true).or_else(|| pick(&cursor, false)) {
            Some(k) => k,
            None => {
                cursor = sources.iter().map(|s| s.from).collect();
                wrapped = true;
                pick(&cursor, true).or_else(|| pick(&cursor, false)).with_context(|| {
                    format!("SHOT_TOO_SHORT: no shot is long enough for a piece of {duration} us; give longer shots or cut more often")
                })?
            }
        };
        let mut content = sources[k].content.clone();
        if let ClipContent::Media { source_in_us, volume, fade_in_us, fade_out_us, .. } = &mut content {
            *source_in_us = cursor[k];
            if !keep_sound {
                *volume = 0.0;
            }
            (*fade_in_us, *fade_out_us) = (0, 0);
        }
        cursor[k] += need(&sources[k]);
        repeated += usize::from(wrapped);
        continued += usize::from(Some(k) == previous);
        used.insert(k);
        (next, previous) = (k + 1, Some(k));
        clips.push(CopiedClip {
            clip: Clip::new(String::new(), start, duration, content),
            track_id: MAIN_TRACK.into(),
            track_kind: TrackKind::Video,
            keep_in_place: false,
        });
    }
    Ok((clips, used.len(), repeated, continued))
}

/// The answer, from the project as the plan left it: checks that every piece starts where planned.
pub fn report(before: &Project, after: &Project, plan: &Plan) -> Result<Value> {
    let main = after.tracks.iter().find(|t| t.id == MAIN_TRACK).context("Main track is missing")?;
    let starts: Vec<i64> = main.clips.iter().map(|c| c.start_us).collect();
    ensure!(
        starts == plan.cut_us && main.clips.last().map(Clip::end_us) == Some(plan.end_us),
        "EDIT_REJECTED: the main track did not take the planned cuts"
    );
    let ids = |project: &Project| -> HashSet<String> {
        project
            .tracks
            .iter()
            .filter(|t| t.id != MAIN_TRACK)
            .flat_map(|t| t.clips.iter().map(|c| c.id.clone()))
            .collect()
    };
    let kept = ids(after);
    let mut removed = Vec::new();
    let mut captions = 0;
    for track in before.tracks.iter().filter(|t| t.id != MAIN_TRACK) {
        for clip in track.clips.iter().filter(|c| !kept.contains(&c.id)) {
            removed.push(clip.id.clone());
            captions += usize::from(track.is_captions());
        }
    }
    let mut warnings = plan.warnings.clone();
    if captions > 0 {
        warnings.push(format!(
            "{captions} captions of the old footage's speech were removed with it; build_captions makes captions again if the new sound has speech"
        ));
    }
    let past: Vec<&str> = after
        .tracks
        .iter()
        .filter(|t| t.id != MAIN_TRACK)
        .flat_map(|t| &t.clips)
        .filter(|c| c.id != plan.music_clip_id && c.end_us() > plan.end_us)
        .map(|c| c.id.as_str())
        .collect();
    if !past.is_empty() {
        warnings.push(format!("these clips run past the new end at {} us: {}", plan.end_us, past.join(", ")));
    }
    let mut answer = plan.song.clone();
    let object = answer.as_object_mut().unwrap();
    for (key, value) in [
        ("time_basis", json!("timeline")),
        ("cut_us", json!(plan.cut_us)),
        ("end_us", json!(plan.end_us)),
        ("duration_us", json!({"before": before.duration_us(), "after": after.duration_us()})),
        ("shots_used", json!(plan.shots_used)),
        ("repeated", json!(plan.repeated)),
        ("removed", json!(removed)),
        (
            "music",
            json!({"clip_id": plan.music_clip_id, "trimmed_to_us": plan.trimmed_to_us, "fade_out_us": plan.fade_out_us}),
        ),
        ("warnings", json!(warnings)),
    ] {
        object.insert(key.into(), value);
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuzky_analysis::{BeatDrop, Section};
    use nuzky_engine::model::{Asset, TextStyle};

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

    /// Two 20 s takes on the main track, a 60 s song from 0 s on a music track and three titles.
    fn project() -> (Project, Vec<String>) {
        let mut p = Project::new("beat test");
        let assets = vec![
            asset("red", AssetKind::Video, 20_000_000),
            asset("blue", AssetKind::Video, 20_000_000),
            asset("song", AssetKind::Audio, 60_000_000),
        ];
        p.apply(EditCmd::AddAssets { assets }).unwrap();
        for id in ["red", "blue", "song"] {
            let start_us = (id == "song").then_some(0);
            p.apply(EditCmd::AddClip { asset_id: id.into(), start_us, track_id: None }).unwrap();
        }
        let style: TextStyle = serde_json::from_value(json!({"fontSize": 60.0, "color": "#ffffff"})).unwrap();
        // The second title runs across the end of the main track at 40 s, the third starts after it.
        let titles = [2_000_000, 39_000_000, 45_000_000]
            .map(|start_us| {
                p.apply(EditCmd::AddText { start_us, text: "Hi".into(), style: style.clone() }).unwrap().select[0]
                    .clone()
            })
            .to_vec();
        (p, titles)
    }

    /// 100 BPM from 0.333 s; steady from beat 4, bars on beats 0, 4, 8..., a drop at beat 12.
    fn beats() -> Beats {
        let beats_us: Vec<i64> = (0..90).map(|i| 333_333 + i * 600_000).collect();
        Beats {
            bpm: 100.0,
            confidence: 0.9,
            steady_from_us: beats_us[4],
            first_downbeat: 0,
            drops: vec![BeatDrop { beat: 12, at_us: beats_us[12], rise_db: 9.0 }],
            sections: vec![Section { beat: 0, at_us: beats_us[0], level_db: -20.0 }],
            beats_us,
        }
    }

    fn at_frame(t: i64) -> i64 {
        ((t as f64 * 30.0 / 1e6).round() * 1e6 / 30.0).round() as i64
    }

    #[test]
    fn the_default_cut_holds_the_intro_then_cuts_bars_then_beats_on_frames() {
        let (mut p, titles) = project();
        let before = p.clone();
        let plan = plan(&p, &beats(), &Options::default()).unwrap();
        let b = beats().beats_us;
        // The footage ends at 40 s, so the video ends on the last bar line before it, beat 64.
        let expected: Vec<i64> =
            std::iter::once(0).chain([4, 8].into_iter().chain(12..64).map(|i| at_frame(b[i]))).collect();
        assert_eq!(plan.cut_us, expected);
        assert_eq!(plan.end_us, at_frame(b[64]));
        for edit in plan.edits.clone() {
            p.apply(edit).unwrap();
        }
        let main = &p.tracks[0].clips;
        assert_eq!(main.iter().map(|c| c.start_us).collect::<Vec<_>>(), plan.cut_us);
        assert_eq!(main.last().unwrap().end_us(), plan.end_us);
        let looks: Vec<(&str, f32)> = main
            .iter()
            .map(|c| match &c.content {
                ClipContent::Media { asset_id, volume, .. } => (asset_id.as_str(), *volume),
                ClipContent::Text { .. } => unreachable!(),
            })
            .collect();
        // Taking turns, the first take fills more of the pieces and runs out three pieces before the end, where the
        // second goes on where it stopped rather than showing footage again.
        let (turns, last) = looks.split_at(looks.len() - 4);
        assert!(turns.windows(2).all(|w| w[0].0 != w[1].0) && looks.iter().all(|l| l.1 == 0.0), "{looks:?}");
        assert!(last.iter().all(|l| l.0 == "blue"), "{looks:?}");
        assert_eq!(plan.repeated, 0);
        assert!(plan.warnings[0].starts_with("no other shot was long enough, so 3 of"), "{:?}", plan.warnings);
        let clip =
            |p: &Project, id: &str| p.tracks.iter().flat_map(|t| &t.clips).find(|c| c.id == id).cloned().unwrap();
        for title in &titles {
            let (was, is) = (clip(&before, title), clip(&p, title));
            assert_eq!((is.start_us, is.duration_us), (was.start_us, was.duration_us), "title {title}");
        }
        let song = clip(&p, &plan.music_clip_id);
        assert_eq!((song.start_us, song.end_us()), (0, plan.end_us));
        assert!(matches!(song.content, ClipContent::Media { fade_out_us: 2_000_000, .. }));
        let report = report(&before, &p, &plan).unwrap();
        assert_eq!(report["music"]["trimmed_to_us"], json!(plan.end_us));
        assert_eq!(report["steady_from_us"], json!(b[4]));
    }

    #[test]
    fn shots_take_turns_and_start_again_when_they_run_out() {
        let (p, _) = project();
        let shot = |asset_id: &str, from: i64, to: i64| Shot {
            asset_id: asset_id.into(),
            source_in_us: from,
            source_out_us: Some(to),
        };
        let options = |shots| Options {
            every: Some(Every::Beat),
            changes: vec![Change { from: ChangeAt::Beat(0), every: Every::Beat }],
            shots: Some(shots),
            end_us: Some(9_000_000),
            ..Options::default()
        };
        let turns =
            plan(&p, &beats(), &options(vec![shot("red", 0, 1_000_000), shot("blue", 5_000_000, 6_000_000)])).unwrap();
        let pieces = sources_of(&turns);
        // 0.33 s before the first beat, then 0.6 s pieces. Blue has 0.4 s left for the fourth, so both start again.
        assert_eq!(
            pieces[..5],
            [
                ("red".into(), 0),
                ("blue".into(), 5_000_000),
                ("red".into(), 333_333),
                ("blue".into(), 5_000_000),
                ("red".into(), 0)
            ]
        );
        assert!(pieces.windows(2).all(|w| w[0].0 != w[1].0));
        assert_eq!((turns.shots_used, turns.repeated), (2, pieces.len() - 3));
        assert!(turns.warnings[0].starts_with("footage repeats"));
        let short = plan_error(&p, options(vec![shot("red", 0, 300_000)]));
        assert!(short.starts_with("SHOT_TOO_SHORT"), "{short}");
        // Footage from files not on the timeline fills an empty main track; the titles stay where they are.
        let mut empty = p.clone();
        let main: Vec<String> = empty.tracks[0].clips.iter().map(|c| c.id.clone()).collect();
        empty.apply(EditCmd::DeleteClips { clip_ids: main }).unwrap();
        let titles: Vec<(i64, i64)> = empty
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Text)
            .flat_map(|t| &t.clips)
            .map(|c| (c.start_us, c.duration_us))
            .collect();
        let plan =
            plan(&empty, &beats(), &options(vec![shot("red", 0, 20_000_000), shot("blue", 0, 20_000_000)])).unwrap();
        for edit in plan.edits.clone() {
            empty.apply(edit).unwrap();
        }
        assert_eq!(empty.tracks[0].clips.iter().map(|c| c.start_us).collect::<Vec<_>>(), plan.cut_us);
        let after: Vec<(i64, i64)> = empty
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Text)
            .flat_map(|t| &t.clips)
            .map(|c| (c.start_us, c.duration_us))
            .collect();
        assert_eq!(after, titles);
    }

    /// 120 BPM, a bar every 2 s from 0.
    fn steady_beats() -> Beats {
        let beats_us: Vec<i64> = (0..120).map(|i| i * 500_000).collect();
        Beats {
            bpm: 120.0,
            confidence: 0.9,
            steady_from_us: 0,
            first_downbeat: 0,
            drops: vec![],
            sections: vec![Section { beat: 0, at_us: 0, level_db: -20.0 }],
            beats_us,
        }
    }

    fn sources_of(plan: &Plan) -> Vec<(String, i64)> {
        plan.edits
            .iter()
            .find_map(|e| match e {
                EditCmd::PasteClips { clips, .. } => Some(
                    clips
                        .iter()
                        .map(|c| match &c.clip.content {
                            ClipContent::Media { asset_id, source_in_us, .. } => (asset_id.clone(), *source_in_us),
                            ClipContent::Text { .. } => unreachable!(),
                        })
                        .collect(),
                ),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn a_long_shot_goes_on_where_it_stopped_when_the_other_is_too_short() {
        let (p, _) = project();
        let shot =
            |asset_id: &str, to: i64| Shot { asset_id: asset_id.into(), source_in_us: 0, source_out_us: Some(to) };
        let options = Options {
            every: Some(Every::Bar),
            shots: Some(vec![shot("red", 20_000_000), shot("blue", 1_000_000)]),
            end_us: Some(8_000_000),
            ..Options::default()
        };
        let plan = plan(&p, &steady_beats(), &options).unwrap();
        assert_eq!(plan.cut_us, [0, 2_000_000, 4_000_000, 6_000_000]);
        let red = |t: i64| ("red".to_string(), t);
        assert_eq!(sources_of(&plan), [red(0), red(2_000_000), red(4_000_000), red(6_000_000)]);
        assert_eq!(plan.repeated, 0);
    }

    #[test]
    fn a_change_from_the_first_beat_holds_past_where_the_beat_is_steady() {
        let (p, _) = project();
        let b = beats().beats_us;
        let options = Options {
            changes: vec![Change { from: ChangeAt::Beat(0), every: Every::Beat }],
            end_us: Some(9_000_000),
            ..Options::default()
        };
        let cut = plan(&p, &beats(), &options).unwrap();
        let end = at_frame(9_000_000);
        let every_beat: Vec<i64> =
            std::iter::once(0).chain(b.iter().map(|&t| at_frame(t)).filter(|&t| t < end)).collect();
        assert_eq!(cut.cut_us, every_beat);
        // A later change still takes over from where it starts.
        let options = Options {
            changes: vec![
                Change { from: ChangeAt::Beat(0), every: Every::Beat },
                Change { from: ChangeAt::Beat(8), every: Every::Bar },
            ],
            end_us: Some(9_000_000),
            ..Options::default()
        };
        let cut = plan(&p, &beats(), &options).unwrap();
        let bars_from_8: Vec<i64> = std::iter::once(0)
            .chain((0..8).chain((8..90).step_by(4)).map(|i| at_frame(b[i])).filter(|&t| t < end))
            .collect();
        assert_eq!(cut.cut_us, bars_from_8);
    }

    fn plan_error(p: &Project, options: Options) -> String {
        plan(p, &beats(), &options).unwrap_err().to_string()
    }
}

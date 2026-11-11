//! What an agent learns about one clip or file beyond the timeline: when and where it was recorded, its frames, its
//! steady tones, what it shows and where a thing in it goes, and the quietest place near a cut.
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use nuzky_engine::{
    Project,
    audio::{Mixer, us_to_samples},
    model::{Asset, AssetKind, CHANNELS, ClipContent},
    speech::is_heard,
};
use serde_json::{Value, json};

/// A file of the project and the part of it a tool reads, in the file's own microseconds.
pub struct Part {
    pub asset: Asset,
    pub range: (i64, i64),
    pub clip: Option<OnTimeline>,
}

/// Where a clip shows its file on the timeline.
#[derive(Clone)]
pub struct OnTimeline {
    pub id: String,
    start_us: i64,
    duration_us: i64,
    source_in_us: i64,
    speed: f64,
}

impl OnTimeline {
    /// The timeline time the clip shows file time `t` at, if it shows it at all.
    pub fn timeline(&self, t: i64) -> Option<i64> {
        if self.speed <= 0.0 {
            return (t == self.source_in_us).then_some(self.start_us);
        }
        let at = self.start_us + ((t - self.source_in_us) as f64 / self.speed).round() as i64;
        (at >= self.start_us && at < self.start_us + self.duration_us && t >= self.source_in_us).then_some(at)
    }
}

impl Part {
    pub fn is_image(&self) -> bool {
        self.asset.kind == AssetKind::Image
    }

    /// Each of `times` with the timeline time the clip shows it at, null when it does not or there is no clip.
    pub fn timeline(&self, t: i64) -> Value {
        json!(self.clip.as_ref().and_then(|clip| clip.timeline(t)))
    }

    pub fn describe(&self) -> Value {
        json!({"asset_id": self.asset.id, "clip_id": self.clip.as_ref().map(|c| &c.id), "range_us": [self.range.0, self.range.1],
            "time_basis": "file"})
    }
}

/// The clip `clip_id` or the file `asset_id` of `project`, and `range` of its file, by default the part the clip
/// shows, or the part of the file the user chose, or all of it.
pub fn part(project: &Project, clip_id: Option<&str>, asset_id: Option<&str>, range: Option<[i64; 2]>) -> Result<Part> {
    let (asset, clip) = match (clip_id, asset_id) {
        (Some(id), None) => {
            let clip = project
                .tracks
                .iter()
                .flat_map(|t| &t.clips)
                .find(|c| c.id == id)
                .with_context(|| format!("UNKNOWN_CLIP: {id}"))?;
            let ClipContent::Media { asset_id, source_in_us, speed, .. } = &clip.content else {
                bail!("INVALID_ARGUMENTS: {id} is a text clip; give a video, image or sound clip");
            };
            let asset = project.asset(asset_id).with_context(|| format!("UNKNOWN_ASSET: {asset_id}"))?;
            let on = OnTimeline {
                id: clip.id.clone(),
                start_us: clip.start_us,
                duration_us: clip.duration_us,
                source_in_us: *source_in_us,
                speed: f64::from(*speed),
            };
            (asset, Some(on))
        }
        (None, Some(id)) => (project.asset(id).with_context(|| format!("UNKNOWN_ASSET: {id}"))?, None),
        _ => bail!("INVALID_ARGUMENTS: give clip_id or asset_id"),
    };
    ensure!(Path::new(&asset.path).is_file(), "MEDIA_MISSING: {}", asset.path);
    let length = if asset.kind == AssetKind::Image { 1 } else { asset.duration_us.max(1) };
    let range = match (range, &clip) {
        (Some([start, end]), _) => {
            ensure!(
                0 <= start && start < end && end <= length,
                "INVALID_RANGE: range_us is a half-open range of the file's own microseconds inside 0..{length}"
            );
            (start, end)
        }
        (None, _) if asset.kind == AssetKind::Image => (0, 1),
        (None, Some(clip)) if clip.speed <= 0.0 => fit(clip.source_in_us, clip.source_in_us + 1, length),
        (None, Some(clip)) => {
            fit(clip.source_in_us, clip.source_in_us + (clip.duration_us as f64 * clip.speed).round() as i64, length)
        }
        (None, None) => asset.part.map_or((0, length), |p| fit(p.start_us, p.end_us, length)),
    };
    Ok(Part { asset: asset.clone(), range, clip })
}

/// `start..end` inside a file of `length`, at least a microsecond long.
fn fit(start: i64, end: i64, length: i64) -> (i64, i64) {
    let start = start.clamp(0, length - 1);
    (start, end.clamp(start + 1, length))
}

/// `count` times spread evenly over `range`, each in the middle of its share.
pub fn spread((start, end): (i64, i64), count: usize) -> Vec<i64> {
    let count = count.max(1) as i64;
    let mut times: Vec<i64> = (0..count).map(|i| start + (2 * i + 1) * (end - start) / (2 * count)).collect();
    times.dedup();
    times
}

/// When, where and on what each file was recorded, as the file says.
pub fn metadata(assets: &[&Asset]) -> Value {
    let rows: Vec<Value> = assets
        .iter()
        .map(|asset| {
            let mut row = json!({"asset_id": asset.id, "name": asset.name, "kind": asset.kind});
            if !Path::new(&asset.path).is_file() {
                row["missing"] = json!(true);
                return row;
            }
            match nuzky_engine::capture::capture(Path::new(&asset.path)) {
                Ok(capture) => {
                    for (key, value) in
                        serde_json::to_value(capture).unwrap_or_default().as_object().into_iter().flatten()
                    {
                        row[key] = value.clone();
                    }
                }
                Err(error) => row["error"] = json!(format!("{error:#}")),
            }
            row
        })
        .collect();
    json!({"assets": rows})
}

/// The quietest stretch of the speech as heard, `within_us` either side of `time_us`, and a frame time in its middle.
pub fn quiet_point(project: &Project, cache: &Path, time_us: i64, within_us: i64) -> Result<Value> {
    const BLOCK: i64 = 240;
    const SMOOTH: usize = 4;
    let duration = project.duration_us();
    ensure!((0..duration).contains(&time_us), "INVALID_RANGE: time_us must be inside the timeline, 0..{duration}");
    let (start, end) = ((time_us - within_us).max(0), (time_us + within_us).min(duration));
    let mut speech = project.clone();
    for (track, original) in speech.tracks.iter_mut().zip(&project.tracks) {
        track.clips.retain(|clip| is_heard(project, original, clip));
    }
    let first = us_to_samples(start);
    let frames = us_to_samples(end) - first;
    ensure!(frames >= BLOCK * SMOOTH as i64, "INVALID_RANGE: look at least 20 ms around time_us");
    let mut mix = vec![0f32; frames as usize * CHANNELS];
    Mixer::new(cache.to_path_buf()).mix(&speech, first, &mut mix);
    let power: Vec<f64> = mix
        .as_chunks::<{ BLOCK as usize * CHANNELS }>()
        .0
        .iter()
        .map(|block| block.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / block.len() as f64)
        .collect();
    // 20 ms of sound around each 5 ms block.
    let level: Vec<f64> = (0..power.len())
        .map(|i| {
            let window = &power[i.saturating_sub(SMOOTH / 2)..(i + SMOOTH / 2).min(power.len())];
            (10.0 * (window.iter().sum::<f64>() / window.len() as f64).max(1e-12).log10()).max(-120.0)
        })
        .collect();
    let block_us = |i: usize| start + nuzky_engine::audio::samples_to_us(i as i64 * BLOCK);
    let quietest = level.iter().copied().fold(f64::INFINITY, f64::min);
    // Runs of blocks within 3 dB of the quietest: the longest, then the one nearest the time asked for.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (i, &l) in level.iter().enumerate() {
        if l > quietest + 3.0 {
            continue;
        }
        match runs.last_mut() {
            Some(run) if run.1 == i => run.1 = i + 1,
            _ => runs.push((i, i + 1)),
        }
    }
    let middle = |(a, b): (usize, usize)| (block_us(a) + block_us(b)) / 2;
    let &(a, b) = runs
        .iter()
        .max_by_key(|run| (run.1 - run.0, -(middle(**run) - time_us).abs()))
        .context("INVALID_RANGE: nothing to listen to there")?;
    let (from, to) = (block_us(a), block_us(b));
    let fps = f64::from(project.canvas.fps.max(1));
    let on_frame = ((middle((a, b)) as f64 * fps / 1e6).round() * 1e6 / fps).round() as i64;
    let quiet_us = if (from..to).contains(&on_frame) { on_frame } else { middle((a, b)) };
    let mut sorted = level.clone();
    sorted.sort_by(f64::total_cmp);
    let loudest = sorted[(sorted.len() - 1) * 95 / 100];
    let at = ((time_us - start) * 48 / 1000 / BLOCK) as usize;
    let round = |v: f64| (v * 10.0).round() / 10.0;
    Ok(json!({
        "time_us": time_us,
        "quiet_us": quiet_us,
        "quiet_range_us": [from, to],
        "level_db": round(quietest),
        "at_time_db": round(level[at.min(level.len() - 1)]),
        "loudest_db": round(loudest),
        "pause": loudest - quietest >= 20.0 && to - from >= 60_000,
    }))
}

/// Mains hum and a whine in the sound of `part`, read in place from its prepared sound.
pub fn spectrum(cache: &Path, part: &Part) -> Result<Option<nuzky_analysis::Spectrum>> {
    let pcm = nuzky_engine::audio::Pcm::open(&nuzky_engine::audio::pcm_path(cache, &part.asset))
        .context("AUDIO_NOT_READY: the sound of the file is not prepared")?;
    let samples = pcm.samples();
    let (from, to) = (us_to_samples(part.range.0) as usize * CHANNELS, us_to_samples(part.range.1) as usize * CHANNELS);
    Ok(nuzky_analysis::spectrum(&samples[from.min(samples.len())..to.min(samples.len())], CHANNELS))
}

/// The frames of `part` at `times` of its file, as one labelled sheet, each `width` pixels wide.
pub fn frame_sheet(part: &Part, times: &[i64], width: u32) -> Result<Vec<u8>> {
    let asset = &part.asset;
    let long = (u64::from(width) * u64::from(asset.width.max(asset.height)) / u64::from(asset.width.max(1))) as u32;
    let mut source = nuzky_vision::source::Source::new(asset, long.max(16))?;
    let (w, h) = source.size();
    crate::media::sheet(times, w, h, |t| Ok(source.frame(t)?.rgba))
}

/// Up to `max` times of `part` that look unlike each other, the first of each shot, spread over the whole part.
pub fn shots(part: &Part, max: usize, cancelled: &dyn Fn() -> bool) -> Result<Vec<i64>> {
    if part.is_image() || part.range.1 - part.range.0 <= 1 {
        return Ok(vec![part.range.0]);
    }
    let source = nuzky_vision::source::Source::new(&part.asset, 64)?;
    let cursor = crate::activity::Cursor::new("", 0, part.range, crate::activity::DEFAULT_MIN_CHANGE);
    let found = crate::activity::changes(source.project(), cursor, usize::MAX, cancelled)?;
    let times = found.times;
    if times.len() <= max {
        return Ok(times);
    }
    let mut picked: Vec<i64> = (0..max).map(|i| times[i * (times.len() - 1) / (max - 1).max(1)]).collect();
    picked.dedup();
    Ok(picked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_maps_its_file_times_to_the_timeline_only_where_it_shows_them() {
        let clip = OnTimeline {
            id: "c".into(),
            start_us: 10_000_000,
            duration_us: 2_000_000,
            source_in_us: 5_000_000,
            speed: 2.0,
        };
        assert_eq!(clip.timeline(5_000_000), Some(10_000_000));
        assert_eq!(clip.timeline(7_000_000), Some(11_000_000));
        assert_eq!(clip.timeline(9_000_000), None);
        assert_eq!(clip.timeline(4_999_999), None);
        let freeze = OnTimeline { speed: 0.0, ..clip };
        assert_eq!((freeze.timeline(5_000_000), freeze.timeline(5_000_001)), (Some(10_000_000), None));
    }

    #[test]
    fn spread_times_sit_in_the_middle_of_their_share() {
        assert_eq!(spread((0, 1_000), 4), [125, 375, 625, 875]);
        assert_eq!(spread((0, 1), 3), [0]);
    }
}

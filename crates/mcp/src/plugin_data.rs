//! What a plugin's `needs` asks Nuzky to find in the sound: silences, loudness and beats, found by the same
//! analysis as the `analyze` tool, from the same prepared sound, and placed on the timeline through the clips that
//! play it. Every media clip whose file has sound counts, on every track, muted or not.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Result, bail, ensure};
use nuzky_analysis::{Beats, Range, SilenceParams};
use nuzky_engine::{
    Project,
    audio::{ensure_pcm, has_audio, pcm_path},
    model::{Asset, Clip, ClipContent},
};
use serde_json::{Map, Value, json};

use crate::plugins::Need;

/// Every quiet stretch of 0.1 s or more, unpadded, so a plugin chooses its own length and margin.
const SILENCES: SilenceParams = SilenceParams { threshold_db: None, min_silence_us: 100_000, pad_us: 0 };
/// Results of a few dozen files; each is small next to the sound it was found in.
const KEPT: usize = 64;

enum Found {
    Silences(Vec<Range>),
    Loudness { window_us: i64, dbfs: Vec<f32> },
    Beats(Beats),
}

/// By kind and prepared sound, whose name changes with the file (path, size, time), so a preview that runs again
/// after every change of its settings finds the sound once.
type Kept = HashMap<(Need, PathBuf), Arc<Found>>;
static FOUND: Mutex<Option<Kept>> = Mutex::new(None);

fn kept(need: Need, pcm: &Path) -> Option<Arc<Found>> {
    FOUND.lock().unwrap().as_ref()?.get(&(need, pcm.to_path_buf())).cloned()
}

fn keep(need: Need, pcm: PathBuf, found: Found) -> Arc<Found> {
    let found = Arc::new(found);
    let mut all = FOUND.lock().unwrap();
    let all = all.get_or_insert_default();
    if all.len() >= KEPT {
        all.clear();
    }
    all.insert((need, pcm), found.clone());
    found
}

/// Each kind of `needs` beyond the words, by its name in `ctx`. `project` has the media's full paths. With `prepare`
/// it prepares sound that is not yet, reporting 0..1 to `progress`; without, it refuses with AUDIO_NOT_READY, as the
/// app's own job is still preparing it.
pub(crate) fn gather(
    project: &Project,
    cache: &Path,
    needs: &[Need],
    prepare: bool,
    cancel: &AtomicBool,
    progress: &dyn Fn(f32),
) -> Result<Map<String, Value>> {
    let needs: Vec<Need> = needs.iter().copied().filter(|need| *need != Need::Words).collect();
    let mut out = Map::new();
    if needs.is_empty() {
        return Ok(out);
    }
    let clips: Vec<(&str, &Clip, &Asset)> = project
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter().map(move |clip| (track.id.as_str(), clip)))
        .filter_map(|(track, clip)| {
            let ClipContent::Media { asset_id, .. } = &clip.content else { return None };
            Some((track, clip, project.asset(asset_id).filter(|asset| has_audio(asset))?))
        })
        .collect();
    let mut assets: Vec<&Asset> = Vec::new();
    for (_, _, asset) in &clips {
        if !assets.iter().any(|a| a.id == asset.id) {
            assets.push(asset);
        }
    }
    let stopped = || cancel.load(Ordering::Relaxed);
    let mut found: HashMap<(Need, &str), Arc<Found>> = HashMap::new();
    for (i, asset) in assets.iter().enumerate() {
        ensure!(Path::new(&asset.path).is_file(), "MEDIA_MISSING: {} is missing; relink it", asset.name);
        let pcm = pcm_path(cache, asset);
        let part = |done: f32| progress((i as f32 + done) / assets.len() as f32);
        if !pcm.exists() {
            ensure!(
                prepare,
                "AUDIO_NOT_READY: Nuzky is still preparing the sound of {}. The preview follows once it is ready.",
                asset.name
            );
            ensure_pcm(cache, asset, |done| {
                ensure!(!stopped(), "CANCELLED: job cancelled");
                part(done * 0.9);
                Ok(())
            })?;
        }
        for &need in &needs {
            let result = match kept(need, &pcm) {
                Some(result) => result,
                None => {
                    let result = match need {
                        Need::Silences => {
                            Found::Silences(nuzky_analysis::silences_cancellable(asset, cache, SILENCES, stopped)?)
                        }
                        Need::Loudness => {
                            let window_us = crate::tools::loudness_window(asset.duration_us, None)?;
                            let dbfs = nuzky_analysis::loudness_cancellable(asset, cache, window_us, stopped)?;
                            Found::Loudness { window_us, dbfs }
                        }
                        Need::Beats => Found::Beats(nuzky_analysis::beats_cancellable(asset, cache, stopped)?),
                        Need::Words => unreachable!("words come from the transcript"),
                    };
                    ensure!(!stopped(), "CANCELLED: job cancelled");
                    keep(need, pcm.clone(), result)
                }
            };
            found.insert((need, asset.id.as_str()), result);
        }
        part(1.0);
    }
    for need in needs {
        let mut entries: Vec<Value> = Vec::new();
        for &(track, clip, asset) in &clips {
            let place = Place::of(track, clip, asset);
            match found[&(need, asset.id.as_str())].as_ref() {
                Found::Silences(ranges) => entries.extend(place.silences(ranges)),
                Found::Loudness { window_us, dbfs } => entries.push(place.loudness(*window_us, dbfs)),
                Found::Beats(beats) => entries.push(place.beats(beats)),
            }
        }
        if need == Need::Silences {
            entries.sort_by_key(|range| (range["start_us"].as_i64(), range["end_us"].as_i64()));
        }
        let name = match need {
            Need::Silences => "silences",
            Need::Loudness => "loudness",
            Need::Beats => "beats",
            Need::Words => bail!("words come from the transcript"),
        };
        out.insert(name.into(), Value::Array(entries));
    }
    Ok(out)
}

/// Where a clip plays its file on the timeline.
struct Place<'a> {
    track: &'a str,
    clip: &'a Clip,
    asset: &'a str,
    source_in: i64,
    source_end: i64,
    speed: f64,
}

impl<'a> Place<'a> {
    fn of(track: &'a str, clip: &'a Clip, asset: &'a Asset) -> Self {
        let ClipContent::Media { source_in_us, speed, .. } = &clip.content else { unreachable!("a media clip") };
        let speed = *speed as f64;
        let source_end = *source_in_us + (clip.duration_us as f64 * speed).round() as i64;
        Place { track, clip, asset: &asset.id, source_in: *source_in_us, source_end, speed }
    }

    fn holds(&self, source_us: i64) -> bool {
        (self.source_in..self.source_end).contains(&source_us)
    }

    /// A time of the file on the timeline, kept within the clip.
    fn at(&self, source_us: i64) -> i64 {
        let at = self.clip.start_us + ((source_us - self.source_in) as f64 / self.speed).round() as i64;
        at.clamp(self.clip.start_us, self.clip.end_us())
    }

    fn named(&self, mut value: Value) -> Value {
        value["clip_id"] = json!(self.clip.id);
        value["track_id"] = json!(self.track);
        value["asset_id"] = json!(self.asset);
        value
    }

    fn silences(&self, ranges: &[Range]) -> Vec<Value> {
        ranges
            .iter()
            .filter(|range| range.end_us > self.source_in && range.start_us < self.source_end)
            .map(|range| (self.at(range.start_us), self.at(range.end_us)))
            .filter(|(start, end)| end > start)
            .map(|(start, end)| self.named(json!({"start_us": start, "end_us": end})))
            .collect()
    }

    /// The windows the clip plays, each from where it starts on the timeline; the first from the clip's start.
    fn loudness(&self, window_us: i64, dbfs: &[f32]) -> Value {
        let first = (self.source_in / window_us) as usize;
        let last = ((self.source_end + window_us - 1) / window_us) as usize;
        let windows = first.min(dbfs.len())..last.min(dbfs.len());
        let times: Vec<i64> = windows.clone().map(|i| self.at(i as i64 * window_us)).collect();
        self.named(json!({"start_us": self.clip.start_us, "end_us": self.clip.end_us(), "times_us": times,
            "dbfs": dbfs[windows]}))
    }

    fn beats(&self, beats: &Beats) -> Value {
        let heard: Vec<usize> = (0..beats.beats_us.len()).filter(|&i| self.holds(beats.beats_us[i])).collect();
        let times = |beats: &mut dyn Iterator<Item = i64>| beats.map(|t| self.at(t)).collect::<Vec<_>>();
        let beats_us = times(&mut heard.iter().map(|&i| beats.beats_us[i]));
        let downbeats_us = times(
            &mut heard
                .iter()
                .filter(|&&i| (i as i64 - beats.first_downbeat as i64).rem_euclid(4) == 0)
                .map(|&i| beats.beats_us[i]),
        );
        let drops: Vec<Value> = beats
            .drops
            .iter()
            .filter(|drop| self.holds(drop.at_us))
            .map(|drop| json!({"at_us": self.at(drop.at_us), "rise_db": drop.rise_db}))
            .collect();
        // The section already playing where the clip starts counts from there.
        let playing = beats.sections.iter().rposition(|section| section.at_us <= self.source_in);
        let sections: Vec<Value> = beats
            .sections
            .iter()
            .enumerate()
            .filter(|(i, section)| Some(*i) == playing || self.holds(section.at_us))
            .map(|(_, section)| json!({"at_us": self.at(section.at_us), "level_db": section.level_db}))
            .collect();
        self.named(json!({
            "start_us": self.clip.start_us, "end_us": self.clip.end_us(),
            "bpm": beats.bpm * self.speed, "confidence": beats.confidence,
            "beats_us": beats_us, "downbeats_us": downbeats_us, "steady_from_us": self.at(beats.steady_from_us),
            "drops": drops, "sections": sections,
        }))
    }
}

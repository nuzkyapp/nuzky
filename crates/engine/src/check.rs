//! The check of an exported video: black frames, a frozen picture and silence, measured on the frames and the
//! sound as they are written, then held against what the project shows and plays there. What the project explains
//! as intended is left out; what it explains otherwise is named; what nothing explains is a likely defect.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use ffmpeg_next as ff;
use serde::Serialize;

use crate::audio::{has_audio, samples_to_us, us_to_samples};
use crate::edit::MAIN_TRACK;
use crate::effects::{source_time, transform_at, transition_at, transition_window};
use crate::export::{frame_count, frame_time_us};
use crate::media::{VideoDecoder, open_input, origin_us};
use crate::model::{Asset, AssetKind, CHANNELS, Clip, ClipContent, Project, TrackKind, TransitionKind};
use crate::render::visible_clips;

/// Cells of the picture grid along its long and short side, like the frames scene detection compares.
const GRID_LONG: usize = 64;
const GRID_SHORT: usize = 36;
/// Pixels read along each side of a cell: 16 of a cell's thousand at 1080p keep the cost the same at any size.
const CELL_SAMPLES: usize = 4;
/// A cell this dark or darker counts as black, in luma from 0 to 1.
const DARK_LUMA: f32 = 0.10;
/// A frame is black with this share of its cells black: a caption or a logo on black still is.
const BLACK_SHARE: f32 = 0.98;
/// Frames this close to the first of a run show the same picture (`picture_difference`). Compared with the first
/// frame, not the one before, so a slow zoom or drift does not pass for a frozen picture.
const FREEZE_DIFFERENCE: f32 = 0.002;
const FREEZE_MIN_US: i64 = 500_000;
/// Frame rate conversion repeats a frame two or three times; that is no frozen picture.
const FREEZE_MIN_FRAMES: u64 = 3;
/// Digital silence: room tone in a pause stays around -60 to -45 dBFS.
const SILENCE_DB: f64 = -70.0;
const SILENCE_MIN_US: i64 = 300_000;
/// Sample frames of each sound level measured.
const BLOCK: usize = 1024;

/// A source frame counts as black with this share of black cells. Less than the export needs, since a clip
/// can be cropped or zoomed into the dark part of its frame.
const SOURCE_BLACK_SHARE: f32 = 0.9;
/// A source still shows one picture with its frames this close to the first: a moving picture held in the export
/// differs by more, while camera noise and the encoding of a still scene stay under it.
const SOURCE_STILL_DIFFERENCE: f32 = 0.006;
/// Sound in a source this quiet or quieter, after the clip's volume, is a pause or room tone that Clean voice,
/// Reduce echo or a low volume can bring under the silence: speech and music are louder.
const QUIET_SOURCE_DB: f64 = -45.0;
/// How often a source's picture is compared while it is decoded.
const LOOK_EVERY_US: i64 = 100_000;
/// Ranges of source files decoded to explain the findings of one export, so a long video with many still shots keeps
/// the check short. Sound decodes far faster than pictures, and a recording with its pauses edited to digital silence
/// has many.
const MAX_PICTURE_LOOKUPS: usize = 8;
const MAX_SOUND_LOOKUPS: usize = 32;
/// Findings listed; the count of the rest is given.
const MAX_FINDINGS: usize = 20;
/// Frame times in each page to look at, as many as `inspect_frames` shows at once.
pub const LOOK_PAGE: usize = 16;

/// Mean absolute RGB difference of two RGBA pictures of one size, 0..=1: the measure a scene cut crosses at
/// `SceneParams::threshold` and a frozen picture stays under.
pub fn picture_difference(a: &[u8], b: &[u8]) -> f32 {
    let sum: u64 = a
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .map(|(a, b)| (0..3).map(|ch| a[ch].abs_diff(b[ch]) as u64).sum::<u64>())
        .sum();
    sum as f32 / ((a.len().min(b.len()) / 4).max(1) as f32 * 3.0 * 255.0)
}

fn grid_size(w: u32, h: u32) -> (usize, usize) {
    if w >= h { (GRID_LONG, GRID_SHORT) } else { (GRID_SHORT, GRID_LONG) }
}

/// The average colour of each cell of `w` × `h` straight RGBA, read at a few points of every cell, as RGBA.
fn grid(pixels: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (gw, gh) = grid_size(w, h);
    let (w, h) = (w as usize, h as usize);
    let at = |cell: usize, sample: usize, size: usize, cells: usize| {
        ((cell * CELL_SAMPLES + sample) * 2 + 1) * size / (cells * CELL_SAMPLES * 2)
    };
    let mut out = Vec::with_capacity(gw * gh * 4);
    for cy in 0..gh {
        for cx in 0..gw {
            let mut sum = [0u32; 3];
            for sy in 0..CELL_SAMPLES {
                let row = at(cy, sy, h, gh) * w;
                for sx in 0..CELL_SAMPLES {
                    let pixel = &pixels[(row + at(cx, sx, w, gw)) * 4..][..3];
                    for (sum, value) in sum.iter_mut().zip(pixel) {
                        *sum += u32::from(*value);
                    }
                }
            }
            let n = (CELL_SAMPLES * CELL_SAMPLES) as u32;
            out.extend([(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, 255]);
        }
    }
    out
}

/// The share of cells at most `DARK_LUMA` bright (BT.709 luma of the encoded values).
fn dark_share(grid: &[u8]) -> f32 {
    let cells = grid.as_chunks::<4>().0;
    let dark = cells
        .iter()
        .filter(|c| {
            (0.2126 * f32::from(c[0]) + 0.7152 * f32::from(c[1]) + 0.0722 * f32::from(c[2])) / 255.0 <= DARK_LUMA
        })
        .count();
    dark as f32 / cells.len().max(1) as f32
}

/// What the check measured in an export: frame ranges that are black or show one picture, and ranges of 48 kHz
/// timeline samples that are silent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measured {
    pub fps: u32,
    pub frames: u64,
    pub duration_us: i64,
    pub black: Vec<Range<u64>>,
    pub frozen: Vec<Range<u64>>,
    pub silent: Vec<Range<i64>>,
}

/// Measures the frames and the sound of an export as the writer takes them, in order.
pub(crate) struct Watch {
    measured: Measured,
    total_samples: i64,
    black_from: Option<u64>,
    /// The first frame of the picture shown since `still_from`.
    anchor: Option<Vec<u8>>,
    still_from: u64,
    energy: f64,
    count: usize,
    block_start: i64,
    silent_from: Option<i64>,
    silence: f64,
}

impl Watch {
    pub(crate) fn new(fps: u32, duration_us: i64) -> Self {
        let silence = 10f64.powf(SILENCE_DB / 20.0);
        Self {
            measured: Measured { fps, duration_us, ..Measured::default() },
            total_samples: us_to_samples(duration_us),
            black_from: None,
            anchor: None,
            still_from: 0,
            energy: 0.0,
            count: 0,
            block_start: 0,
            silent_from: None,
            silence: silence * silence,
        }
    }

    /// Frame `index`, straight RGBA `w` × `h`.
    pub(crate) fn frame(&mut self, index: u64, pixels: &[u8], w: u32, h: u32) {
        let cells = grid(pixels, w, h);
        let black = dark_share(&cells) >= BLACK_SHARE;
        match (black, self.black_from) {
            (true, None) => self.black_from = Some(index),
            (false, Some(from)) => {
                self.measured.black.push(from..index);
                self.black_from = None;
            }
            _ => {}
        }
        let same = self.anchor.as_ref().is_some_and(|anchor| picture_difference(anchor, &cells) <= FREEZE_DIFFERENCE);
        if !same {
            self.end_still(index);
            self.anchor = Some(cells);
            self.still_from = index;
        }
        self.measured.frames = index + 1;
    }

    fn end_still(&mut self, end: u64) {
        let (from, fps) = (self.still_from, self.measured.fps);
        if self.anchor.is_some()
            && end - from >= FREEZE_MIN_FRAMES
            && frame_time_us(end, fps) - frame_time_us(from, fps) >= FREEZE_MIN_US
        {
            self.measured.frozen.push(from..end);
        }
    }

    /// Interleaved stereo sound starting at timeline sample `pos`, each call continuing the one before.
    pub(crate) fn sound(&mut self, pos: i64, mix: &[f32]) {
        let frames = mix.as_chunks::<CHANNELS>().0;
        let heard = (self.total_samples - pos).clamp(0, frames.len() as i64) as usize;
        for frame in &frames[..heard] {
            self.energy += frame.iter().map(|s| f64::from(*s) * f64::from(*s)).sum::<f64>();
            self.count += 1;
            if self.count == BLOCK {
                self.end_block();
            }
        }
    }

    fn end_block(&mut self) {
        let end = self.block_start + self.count as i64;
        let silent = self.energy / (self.count * CHANNELS) as f64 <= self.silence;
        match (silent, self.silent_from) {
            (true, None) => self.silent_from = Some(self.block_start),
            (false, Some(from)) => {
                self.end_silence(from, self.block_start);
            }
            _ => {}
        }
        (self.block_start, self.energy, self.count) = (end, 0.0, 0);
    }

    fn end_silence(&mut self, from: i64, to: i64) {
        if to - from >= us_to_samples(SILENCE_MIN_US) {
            self.measured.silent.push(from..to);
        }
        self.silent_from = None;
    }

    pub(crate) fn finish(mut self) -> Measured {
        if self.count > 0 {
            self.end_block();
        }
        if let Some(from) = self.silent_from {
            self.end_silence(from, self.block_start);
        }
        let frames = self.measured.frames;
        if let Some(from) = self.black_from.take() {
            self.measured.black.push(from..frames);
        }
        self.end_still(frames);
        self.measured
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Black,
    Frozen,
    Silent,
}

/// A stretch of the export the project does not explain as intended.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Finding {
    pub kind: Kind,
    pub start_us: i64,
    pub end_us: i64,
    /// What the project shows or plays there, or `None` when nothing in it explains the stretch.
    pub cause: Option<&'static str>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Review {
    /// In timeline order, at most `MAX_FINDINGS`.
    pub findings: Vec<Finding>,
    pub more_findings: usize,
    /// Stretches left out because the project makes them so on purpose, by reason.
    pub explained: BTreeMap<&'static str, usize>,
    /// Frames to look at in the file, at most `LOOK_PAGE` a page: the last frame before and the first after every
    /// cut of a video track and the first and last frame of each finding.
    pub frame_time_us: Vec<Vec<i64>>,
}

/// Why a time of a stretch looks the way it does.
#[derive(Clone, Debug, PartialEq)]
enum Why {
    /// The project makes it so on purpose.
    Intended(&'static str),
    /// The project shows or plays this there.
    Shows(&'static str),
    /// Clips play there as usual; their files tell whether they look or sound the same.
    Plays(Vec<String>),
}

/// Explains the stretches `measured` found in an export of `project`. Decodes a limited number of ranges of the
/// project's original files, never its caches, so a broken cache comes out as a finding without a cause.
/// `cancelled` is checked on every decoded frame.
pub fn review(
    project: &Project,
    measured: &Measured,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(f32),
) -> Result<Review> {
    let (fps, duration) = (measured.fps, measured.duration_us);
    let frame_us = |i: u64| frame_time_us(i, fps).min(duration);
    let frames = |ranges: Vec<Range<u64>>| -> Vec<(Range<i64>, Vec<i64>)> {
        ranges
            .into_iter()
            .map(|r| (frame_us(r.start)..frame_us(r.end), r.map(|i| frame_time_us(i, fps)).collect()))
            .collect()
    };
    let block_us = samples_to_us(BLOCK as i64);
    let silent = measured
        .silent
        .iter()
        .map(|r| {
            let range = samples_to_us(r.start)..samples_to_us(r.end).min(duration);
            let times = (0..).map(|k| range.start + k * block_us).take_while(|t| *t < range.end).collect();
            (range, times)
        })
        .collect();
    let frozen = without(&measured.frozen, &measured.black);
    let cuts = cut_times(project);
    let mut pieces: Vec<(Kind, Range<i64>, Why)> = Vec::new();
    for (kind, stretches) in
        [(Kind::Black, frames(measured.black.clone())), (Kind::Silent, silent), (Kind::Frozen, frames(frozen))]
    {
        for (range, times) in stretches {
            for (piece, why) in split(&range, &times, &cuts, |t| match kind {
                Kind::Black => black_at(project, t),
                Kind::Frozen => frozen_at(project, t),
                Kind::Silent => silent_at(project, t),
            }) {
                // Runs of one picture can follow each other within a clip; a file is looked at once for them.
                match pieces.last_mut() {
                    Some((k, last, w)) if *k == kind && last.end == piece.start && *w == why => last.end = piece.end,
                    _ => pieces.push((kind, piece, why)),
                }
            }
        }
    }

    // The longest stretches are looked up first: a short pause matters less than minutes without sound.
    let mut longest = Vec::new();
    for (sound, most) in [(false, MAX_PICTURE_LOOKUPS), (true, MAX_SOUND_LOOKUPS)] {
        let mut these: Vec<usize> = (0..pieces.len())
            .filter(|&i| matches!(pieces[i].2, Why::Plays(_)) && (pieces[i].0 == Kind::Silent) == sound)
            .collect();
        these.sort_by_key(|&i| (std::cmp::Reverse(pieces[i].1.end - pieces[i].1.start), i));
        longest.extend(these.into_iter().take(most));
    }
    let lookups = longest.len();
    let (mut looked, mut explained, mut found) = (0, BTreeMap::new(), Vec::new());
    let mut last_intended: Option<(Kind, &'static str, i64)> = None;
    for (index, (kind, range, why)) in pieces.into_iter().enumerate() {
        if cancelled() {
            bail!("CANCELLED: the check was stopped");
        }
        let why = match why {
            Why::Plays(clips) if longest.contains(&index) => {
                looked += 1;
                let why = look(project, kind, &range, &clips, cancelled)?;
                progress(looked as f32 / lookups.max(1) as f32);
                why
            }
            Why::Plays(_) => Why::Shows("unchecked"),
            why => why,
        };
        match why {
            Why::Intended(reason) => {
                // A transition or animation split at a cut is still one.
                if last_intended != Some((kind, reason, range.start)) {
                    *explained.entry(reason).or_insert(0) += 1;
                }
                last_intended = Some((kind, reason, range.end));
            }
            Why::Shows(cause) => {
                found.push(Finding { kind, start_us: range.start, end_us: range.end, cause: Some(cause) })
            }
            Why::Plays(_) => found.push(Finding { kind, start_us: range.start, end_us: range.end, cause: None }),
        }
    }
    found.sort_by_key(|f| (f.start_us, f.kind));
    let more_findings = found.len().saturating_sub(MAX_FINDINGS);
    found.truncate(MAX_FINDINGS);
    let frame_time_us = look_times(project, measured, &found);
    progress(1.0);
    Ok(Review { findings: found, more_findings, explained, frame_time_us })
}

/// `ranges` without the frames in `minus`, both sorted.
fn without(ranges: &[Range<u64>], minus: &[Range<u64>]) -> Vec<Range<u64>> {
    let mut out = Vec::new();
    for range in ranges {
        let mut start = range.start;
        for cut in minus.iter().filter(|m| m.start < range.end && m.end > range.start) {
            if cut.start > start {
                out.push(start..cut.start);
            }
            start = start.max(cut.end);
        }
        if start < range.end {
            out.push(start..range.end);
        }
    }
    out
}

/// Every edge of a clip or a transition, where what the timeline shows changes.
fn cut_times(project: &Project) -> Vec<i64> {
    let mut cuts = BTreeSet::new();
    for clip in project.tracks.iter().flat_map(|t| &t.clips) {
        cuts.extend([clip.start_us, clip.end_us()]);
        if let Some((a, b)) = transition_window(clip) {
            cuts.extend([a, b]);
        }
    }
    cuts.into_iter().collect()
}

/// Splits `range` where a cut passes or the reason changes; `times` are the times measured in it.
fn split(range: &Range<i64>, times: &[i64], cuts: &[i64], why: impl Fn(i64) -> Why) -> Vec<(Range<i64>, Why)> {
    let mut pieces: Vec<(Range<i64>, Why, usize)> = Vec::new();
    for &t in times {
        let (segment, why) = (cuts.partition_point(|c| *c <= t), why(t));
        match pieces.last_mut() {
            Some((piece, last, at)) if *at == segment && *last == why => piece.end = t,
            Some((piece, _, _)) => {
                piece.end = t;
                pieces.push((t..t, why, segment));
            }
            None => pieces.push((range.start..t, why, segment)),
        }
    }
    if let Some((piece, _, _)) = pieces.last_mut() {
        piece.end = range.end;
    }
    pieces.into_iter().filter(|(r, _, _)| r.start < r.end).map(|(r, why, _)| (r, why)).collect()
}

/// A picture of a clip visible at a time.
struct Layer<'a> {
    clip: &'a Clip,
    asset: &'a Asset,
    opacity: f32,
    animating: bool,
}

/// Below this opacity a layer over black looks black.
const SHOWN_OPACITY: f32 = 0.1;

fn layers(project: &Project, t: i64) -> Vec<Layer<'_>> {
    let mut layers = Vec::new();
    for visible in project.tracks.iter().flat_map(|track| visible_clips(track, t)) {
        let clip = visible.clip;
        let ClipContent::Media { asset_id, .. } = &clip.content else { continue };
        let Some(asset) = project.asset(asset_id) else { continue };
        if asset.kind == AssetKind::Audio || asset.width == 0 || asset.height == 0 {
            continue;
        }
        let (transform, _) = transform_at(clip, t);
        let u = t - clip.start_us;
        let animating = [(clip.anim_in, false), (clip.anim_out, true)].into_iter().any(|(animation, exiting)| {
            animation.is_some_and(|a| {
                let d = a.duration_us.min(clip.duration_us);
                d > 0 && if exiting { u >= clip.duration_us - d } else { u < d }
            })
        });
        let opacity = if transform.scale > 0.0 { transform.opacity } else { 0.0 };
        layers.push(Layer { clip, asset, opacity, animating });
    }
    layers
}

fn transition(project: &Project, t: i64) -> Option<TransitionKind> {
    let main = project.tracks.iter().find(|track| track.id == MAIN_TRACK)?;
    transition_at(main, t).map(|(_, _, transition, _)| transition.kind)
}

fn black_at(project: &Project, t: i64) -> Why {
    if matches!(transition(project, t), Some(TransitionKind::FadeBlack | TransitionKind::FadeWhite)) {
        return Why::Intended("fade_transition");
    }
    let layers = layers(project, t);
    if layers.is_empty() {
        return Why::Shows("uncovered");
    }
    if layers.iter().any(|l| l.animating) {
        return Why::Intended("animation");
    }
    let shown: Vec<String> = layers.iter().filter(|l| l.opacity > SHOWN_OPACITY).map(|l| l.clip.id.clone()).collect();
    if shown.is_empty() {
        return Why::Shows("transparent");
    }
    Why::Plays(shown)
}

fn frozen_at(project: &Project, t: i64) -> Why {
    if transition(project, t).is_some() {
        return Why::Intended("transition");
    }
    let layers = layers(project, t);
    let shown: Vec<_> = layers.iter().filter(|l| l.opacity > SHOWN_OPACITY).collect();
    if shown.is_empty() {
        return Why::Shows(if layers.is_empty() { "uncovered" } else { "transparent" });
    }
    let videos: Vec<String> =
        shown.iter().filter(|l| l.asset.kind == AssetKind::Video).map(|l| l.clip.id.clone()).collect();
    if videos.is_empty() {
        return Why::Intended("image");
    }
    Why::Plays(videos)
}

fn silent_at(project: &Project, t: i64) -> Why {
    let mut heard = Vec::new();
    let mut fading = true;
    for track in project.tracks.iter().filter(|track| !track.muted && track.kind != TrackKind::Text) {
        let main = track.id == MAIN_TRACK;
        for (index, clip) in track.clips.iter().enumerate() {
            let ClipContent::Media { asset_id, volume, fade_in_us, fade_out_us, .. } = &clip.content else { continue };
            if *volume <= 0.0 || !project.asset(asset_id).is_some_and(has_audio) {
                continue;
            }
            // A transition plays the clips on either side over the whole of it.
            let from = transition_window(clip).filter(|_| main && index > 0).map_or(clip.start_us, |w| w.0);
            let to =
                track.clips.get(index + 1).and_then(transition_window).filter(|_| main).map_or(clip.end_us(), |w| w.1);
            if t < from.min(clip.start_us) || t >= to.max(clip.end_us()) {
                continue;
            }
            fading &= t < clip.start_us + fade_in_us || t >= clip.end_us() - fade_out_us;
            heard.push(clip.id.clone());
        }
    }
    if heard.is_empty() {
        Why::Shows("nothing_heard")
    } else if fading {
        Why::Intended("audio_fade")
    } else {
        Why::Plays(heard)
    }
}

/// Decodes the files of `clips` over the source of `range` and says whether they explain the stretch.
fn look(
    project: &Project,
    kind: Kind,
    range: &Range<i64>,
    clips: &[String],
    cancelled: &dyn Fn() -> bool,
) -> Result<Why> {
    let mut held = false;
    let mut loudest = None::<f64>;
    let mut ended = false;
    for id in clips {
        let clip = project.tracks.iter().flat_map(|t| &t.clips).find(|c| &c.id == id).context("The clip is gone")?;
        let ClipContent::Media { asset_id, volume, .. } = &clip.content else { continue };
        let asset = project.asset(asset_id).context("The clip's file is gone")?;
        let from = source_time(clip, range.start);
        let to = source_time(clip, range.end).max(from + 1);
        let path = Path::new(&asset.path);
        match kind {
            Kind::Black => {
                if !look_at_picture(path, from, to, Need::Dark, cancelled)?.dark {
                    return Ok(Why::Plays(clips.to_vec()));
                }
            }
            Kind::Frozen => {
                let seen = look_at_picture(path, from, to, Need::Still, cancelled)?;
                if !seen.still {
                    return Ok(Why::Plays(clips.to_vec()));
                }
                held |= seen.ended;
            }
            Kind::Silent => {
                let heard = hear(path, from, to, cancelled)?;
                ended |= heard.ended;
                if let Some(db) = heard.loudest_db {
                    let db = db + 20.0 * f64::from(*volume).log10();
                    loudest = Some(loudest.map_or(db, |l| l.max(db)));
                }
            }
        }
    }
    Ok(match kind {
        Kind::Black => Why::Intended("dark_source"),
        Kind::Frozen if held => Why::Shows("held_last_frame"),
        Kind::Frozen => Why::Intended("still_source"),
        // Sound the file has there and the export lost.
        Kind::Silent if loudest.is_some_and(|db| db > QUIET_SOURCE_DB) => Why::Plays(clips.to_vec()),
        Kind::Silent if ended => Why::Shows("past_source_end"),
        Kind::Silent => Why::Intended("quiet_source"),
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Need {
    Dark,
    Still,
}

struct Seen {
    dark: bool,
    still: bool,
    /// The video ends before `to`, so a clip holds its last frame.
    ended: bool,
}

/// How the picture of the file at `path` looks from `from` to `to` in its own time.
fn look_at_picture(path: &Path, from: i64, to: i64, need: Need, cancelled: &dyn Fn() -> bool) -> Result<Seen> {
    let mut decoder = VideoDecoder::open_for_seeking(path)?;
    decoder.seek(from)?;
    let mut seen = Seen { dark: true, still: true, ended: false };
    let mut anchor: Option<Vec<u8>> = None;
    let mut see = |decoder: &mut VideoDecoder, t: i64, frame: &ff::util::frame::Video| -> Result<bool> {
        let (gw, gh) = grid_size(frame.width(), frame.height());
        let (w, h) = ((gw * CELL_SAMPLES) as u32, (gh * CELL_SAMPLES) as u32);
        let rgba = decoder.convert(frame, t, w, h)?;
        let cells = grid(&rgba.data, rgba.width, rgba.height);
        seen.dark &= dark_share(&cells) >= SOURCE_BLACK_SHARE;
        match &anchor {
            Some(anchor) => seen.still &= picture_difference(anchor, &cells) <= SOURCE_STILL_DIFFERENCE,
            None => anchor = Some(cells),
        }
        // Nothing more to learn once the answer is no.
        Ok(match need {
            Need::Dark => seen.dark,
            Need::Still => seen.still,
        })
    };
    let (mut covering, mut seen_at, mut last) = (None, None::<i64>, None::<i64>);
    loop {
        if cancelled() {
            bail!("CANCELLED: the check was stopped");
        }
        let Some((t, frame)) = decoder.next_frame()? else { break };
        if decoder.is_image() {
            see(&mut decoder, t, &frame)?;
            return Ok(seen);
        }
        if t <= from {
            covering = Some((t, frame));
            continue;
        }
        if let Some((ct, cf)) = covering.take() {
            seen_at = Some(ct);
            if !see(&mut decoder, ct, &cf)? {
                return Ok(seen);
            }
        }
        if t >= to {
            return Ok(seen);
        }
        last = Some(t);
        // Every frame is decoded, a few a second are compared: a moving picture keeps moving between them.
        if seen_at.is_some_and(|at| t - at < LOOK_EVERY_US) {
            continue;
        }
        seen_at = Some(t);
        if !see(&mut decoder, t, &frame)? {
            return Ok(seen);
        }
    }
    if let Some((ct, cf)) = covering.take() {
        last = Some(ct);
        see(&mut decoder, ct, &cf)?;
    }
    // The last frame shows until the next would start. A clip runs a little past its picture when its sound is
    // longer; held that briefly, the frozen picture is the file's own.
    seen.ended = last.is_none_or(|t| to - (t + decoder.frame_duration_us) >= FREEZE_MIN_US);
    Ok(seen)
}

struct Heard {
    /// The loudest block of about 21 ms in dBFS, `None` without any sound in the range.
    loudest_db: Option<f64>,
    /// The sound ends at least `SILENCE_MIN_US` before the range does.
    ended: bool,
}

/// The sound of the file at `path` from `from` to `to` in its own time.
fn hear(path: &Path, from: i64, to: i64, cancelled: &dyn Fn() -> bool) -> Result<Heard> {
    use ff::util::mathematics::{Rescale, Rounding};
    let mut input = open_input(path)?;
    let Some(stream) = input.streams().best(ff::media::Type::Audio) else {
        return Ok(Heard { loudest_db: None, ended: true });
    };
    let (index, time_base) = (stream.index(), stream.time_base());
    let mut decoder = ff::codec::context::Context::from_parameters(stream.parameters())?.decoder().audio()?;
    let origin = origin_us(&input);
    // A little early, since a seek in sound can land on the packet after.
    let ts = (from - 100_000).max(0).saturating_add(origin).rescale_with((1, 1_000_000), time_base, Rounding::Down);
    if unsafe { ff::ffi::avformat_seek_file(input.as_mut_ptr(), index as i32, i64::MIN, ts, ts, 0) } < 0 {
        input.seek(0, ..).ok();
    }
    let tb = f64::from(time_base);
    let mut loudest: Option<f64> = None;
    let (mut energy, mut count, mut channels, mut last) = (0.0f64, 0usize, 1usize, None::<i64>);
    let mut frame = ff::util::frame::Audio::empty();
    let mut done = false;
    let mut take = |frame: &ff::util::frame::Audio, loudest: &mut Option<f64>| -> bool {
        let rate = frame.rate().max(1) as i64;
        let Some(pts) = frame.timestamp().or(frame.pts()) else { return false };
        let start = (pts as f64 * tb * 1e6).round() as i64 - origin;
        let block = (rate as usize * BLOCK / 48_000).max(1);
        channels = frame.channels().max(1) as usize;
        for (k, power) in powers(frame).into_iter().enumerate() {
            let t = start + k as i64 * 1_000_000 / rate;
            if t < from {
                continue;
            }
            if t >= to {
                return true;
            }
            last = Some(t);
            energy += power;
            count += 1;
            if count == block {
                let db = 10.0 * (energy / (count * channels) as f64).max(1e-20).log10();
                *loudest = Some(loudest.map_or(db, |l| l.max(db)));
                (energy, count) = (0.0, 0);
            }
        }
        false
    };
    for (s, packet) in input.packets() {
        if cancelled() {
            bail!("CANCELLED: the check was stopped");
        }
        if s.index() != index || decoder.send_packet(&packet).is_err() {
            continue;
        }
        while decoder.receive_frame(&mut frame).is_ok() {
            done |= take(&frame, &mut loudest);
        }
        if done {
            break;
        }
    }
    if !done {
        decoder.send_eof().ok();
        while decoder.receive_frame(&mut frame).is_ok() {
            take(&frame, &mut loudest);
        }
    }
    if count > 0 {
        let db = 10.0 * (energy / (count * channels) as f64).max(1e-20).log10();
        loudest = Some(loudest.map_or(db, |l| l.max(db)));
    }
    Ok(Heard { loudest_db: loudest, ended: last.is_none_or(|t| to - t >= SILENCE_MIN_US) })
}

/// The sum over channels of each sample's square, for every sample of `frame`, in -1..1 full scale.
fn powers(frame: &ff::util::frame::Audio) -> Vec<f64> {
    use ff::format::Sample;
    use ff::format::sample::Type;
    let (n, channels) = (frame.samples(), frame.channels().max(1) as usize);
    let format = frame.format();
    let width = format.bytes();
    let value = |bytes: &[u8]| -> f64 {
        match format {
            Sample::U8(_) => (f64::from(bytes[0]) - 128.0) / 128.0,
            Sample::I16(_) => f64::from(i16::from_ne_bytes([bytes[0], bytes[1]])) / 32_768.0,
            Sample::I32(_) => f64::from(i32::from_ne_bytes(bytes[..4].try_into().unwrap())) / 2_147_483_648.0,
            Sample::I64(_) => i64::from_ne_bytes(bytes[..8].try_into().unwrap()) as f64 / 9_223_372_036_854_775_808.0,
            Sample::F32(_) => f64::from(f32::from_ne_bytes(bytes[..4].try_into().unwrap())),
            Sample::F64(_) => f64::from_ne_bytes(bytes[..8].try_into().unwrap()),
            Sample::None => 0.0,
        }
    };
    let mut powers = vec![0.0; n];
    let planar = matches!(
        format,
        Sample::U8(Type::Planar)
            | Sample::I16(Type::Planar)
            | Sample::I32(Type::Planar)
            | Sample::I64(Type::Planar)
            | Sample::F32(Type::Planar)
            | Sample::F64(Type::Planar)
    );
    for (k, power) in powers.iter_mut().enumerate() {
        for ch in 0..channels {
            let (plane, at) = if planar { (ch, k * width) } else { (0, (k * channels + ch) * width) };
            let Some(bytes) = frame.data(plane).get(at..at + width) else { continue };
            let v = value(bytes);
            *power += v * v;
        }
    }
    powers
}

/// Frame times worth a look in the file: around every cut of a visible video track and at each finding's ends.
fn look_times(project: &Project, measured: &Measured, found: &[Finding]) -> Vec<Vec<i64>> {
    let (fps, frames) = (measured.fps, measured.frames);
    let mut indices = BTreeSet::new();
    let mut around = |t: i64| {
        let at = frame_count(t, fps);
        if at > 0 {
            indices.insert(at - 1);
        }
        if at < frames {
            indices.insert(at);
        }
    };
    for track in project.tracks.iter().filter(|t| t.kind == TrackKind::Video && !t.hidden) {
        for clip in track.clips.iter().filter(|c| matches!(c.content, ClipContent::Media { .. })) {
            for edge in [clip.start_us, clip.end_us()] {
                if edge > 0 && edge < measured.duration_us {
                    around(edge);
                }
            }
        }
    }
    for finding in found {
        // The frames showing the first and the last moment of it.
        for t in [finding.start_us, finding.end_us - 1] {
            let at = frame_count(t + 1, fps);
            if at > 0 && at <= frames {
                indices.insert(at - 1);
            }
        }
    }
    let times: Vec<i64> = indices.into_iter().map(|i| frame_time_us(i, fps)).collect();
    times.chunks(LOOK_PAGE).map(<[i64]>::to_vec).collect()
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        (0..w * h).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect()
    }

    #[test]
    fn a_frame_with_a_caption_on_black_is_black_and_a_dim_one_is_not() {
        let (w, h) = (1080, 1920);
        let mut frame = solid(w, h, [0; 3]);
        // A white caption over 1 % of the picture.
        for y in 1500..1540 {
            for x in 300..780 {
                frame[(y * w as usize + x) * 4..][..3].fill(255);
            }
        }
        let mut watch = Watch::new(30, 1_000_000);
        watch.frame(0, &frame, w, h);
        watch.frame(1, &solid(w, h, [40; 3]), w, h);
        assert_eq!(watch.finish().black, [0..1]);
    }

    #[test]
    fn a_frozen_picture_needs_half_a_second_and_three_frames() {
        let (w, h) = (64, 36);
        let measure = |fps: u32, frames: &[u8]| {
            let mut watch = Watch::new(fps, frame_time_us(frames.len() as u64, fps));
            for (i, value) in frames.iter().enumerate() {
                watch.frame(i as u64, &solid(w, h, [*value, 100, 100]), w, h);
            }
            watch.finish().frozen
        };
        // 15 frames at 30 fps hold 0.5 s; 14 do not.
        let mut held: Vec<u8> = (0..10).map(|i| i * 20).collect();
        held.extend([200; 14]);
        held.push(10);
        assert_eq!(measure(30, &held), []);
        held.insert(10, 200);
        assert_eq!(measure(30, &held), [10..25]);
        // At 4 fps two frames last half a second and are still not a frozen picture; three are.
        assert_eq!(measure(4, &[0, 50, 50, 100]), []);
        assert_eq!(measure(4, &[0, 50, 50, 50]), [1..4]);
    }

    #[test]
    fn silence_is_measured_in_blocks_up_to_the_end_of_the_timeline() {
        let duration = 1_000_000;
        let mut watch = Watch::new(30, duration);
        let loud = vec![0.1f32; 4800 * CHANNELS];
        let quiet = vec![0.0001f32; 24_000 * CHANNELS];
        watch.sound(0, &loud);
        watch.sound(4800, &quiet);
        // Past the end of the timeline, as the last AAC frame of an export reaches.
        watch.sound(28_800, &vec![0.0f32; 48_000 * CHANNELS]);
        let silent = watch.finish().silent;
        // Blocks of 1024 from 0: the first wholly quiet one starts at 5120; quiet runs to the end at 48 000.
        assert_eq!(silent, [5120..48_000]);
    }

    #[test]
    fn pieces_split_at_cuts_and_where_the_reason_changes() {
        let times: Vec<i64> = (0..10).map(|i| i * 100).collect();
        let why = |t: i64| if t < 500 { Why::Shows("a") } else { Why::Shows("b") };
        let pieces = split(&(0..1000), &times, &[250], why);
        let ranges: Vec<_> = pieces.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, [0..300, 300..500, 500..1000]);
        assert_eq!(without(&[0..10, 20..30], &[2..4, 8..22]), [0..2, 4..8, 22..30]);
    }
}

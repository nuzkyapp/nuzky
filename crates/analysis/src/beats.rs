//! The beat of music, for cutting video to a song: tempo, every beat, the bars, the drops and how
//! loud each section is. Times are source microseconds of the file. Same samples, same result.

use std::path::Path;

use anyhow::{Result, bail};
use nuzky_engine::model::{Asset, CHANNELS, SAMPLE_RATE};
use realfft::{RealFftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};

use crate::audio::open_pcm;

const HOP: usize = 480;
const WINDOW: usize = 2048;
/// Envelope frames per second: one every 10 ms.
const RATE: f64 = SAMPLE_RATE as f64 / HOP as f64;
const FRAME_US: f64 = 1e6 / RATE;
/// Bins 1-6 of 23.4 Hz each: the kick drum and the bass.
const LOW_BINS: std::ops::Range<usize> = 1..7;
/// The spectral flux of a 2048-sample Hann window peaks this much before the attack it marks, as
/// measured on generated clicks and kicks.
const ONSET_LATENCY_US: f64 = 8_000.0;
/// About ten seconds of audio between cancel checks.
const CANCEL_FRAMES: usize = 1_000;
/// How strongly the beat tracker holds the tempo against the onsets (Ellis 2007).
const TIGHTNESS: f64 = 100.0;
/// Tempi are reported in [78, 160) BPM, so half and double time always resolve the same way.
const SLOWEST: f64 = 78.0;
const FASTEST: f64 = 160.0;
const STEADY_RUN: usize = 8;
const CHANGE_DB: f64 = 6.0;
const SPAN_BEATS: usize = 16;
const MIN_GAP_BEATS: usize = 32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Beats {
    pub bpm: f64,
    /// 0..1: how regular the pulse is. Under 0.3 there is no beat worth cutting to.
    pub confidence: f64,
    /// Every beat. Those before `steady_from_us` are not heard but continue its grid back to the start.
    pub beats_us: Vec<i64>,
    /// Index of the first beat that starts a 4/4 bar; every fourth beat from it does too.
    pub first_downbeat: usize,
    /// Where the first run of eight strong beats starts.
    pub steady_from_us: i64,
    pub drops: Vec<BeatDrop>,
    pub sections: Vec<Section>,
}

/// A bar where the music gets at least 6 dB louder over four bars than over the four before.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BeatDrop {
    pub beat: usize,
    pub at_us: i64,
    pub rise_db: f64,
}

/// A part of the song as loud as itself: it starts where the level changes by 6 dB or more.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub beat: usize,
    pub at_us: i64,
    pub level_db: f64,
}

pub fn beats_cancellable(asset: &Asset, cache: &Path, cancelled: impl Fn() -> bool) -> Result<Beats> {
    let pcm = open_pcm(asset, cache, &cancelled)?;
    beats_in(pcm.samples(), &cancelled)
}

/// Beats of interleaved stereo 48 kHz samples.
pub fn beats_in(samples: &[f32], cancelled: &dyn Fn() -> bool) -> Result<Beats> {
    let none = Beats {
        bpm: 0.0,
        confidence: 0.0,
        beats_us: vec![],
        first_downbeat: 0,
        steady_from_us: 0,
        drops: vec![],
        sections: vec![],
    };
    let frames = Envelope::of(samples, cancelled)?;
    let onset = combined_onsets(&frames.flux, &frames.low);
    let lags = lag(FASTEST_SEARCH)..=lag(SLOWEST_SEARCH);
    if onset.len() < 3 * *lags.end() || onset.iter().all(|&o| o == 0.0) {
        return Ok(none);
    }
    let period = fold(tempo(&onset, lags));
    let confidence = (2.0 * periodicity(&onset, period)).clamp(0.0, 1.0);
    let std = deviation(&onset);
    let onset: Vec<f64> = onset.iter().map(|o| o / std).collect();
    let tracked = track(&onset, period, cancelled)?;
    let mut beats: Vec<(f64, f64)> = tracked
        .iter()
        .map(|&b| {
            let strength = onset[b.saturating_sub(3)..(b + 4).min(onset.len())].iter().sum::<f64>();
            (refine(&onset, b), strength)
        })
        .collect();
    let typical = percentile(beats.iter().map(|b| b.1).collect(), 0.75);
    // Past the last real beat the tracker keeps the grid going over silence or a fade.
    while beats.len() > 1 && beats.last().is_some_and(|b| b.1 < 0.1 * typical) {
        beats.pop();
    }
    if beats.len() < STEADY_RUN {
        return Ok(Beats { confidence, ..none });
    }
    let strong: Vec<bool> = beats.iter().map(|b| b.1 >= 0.4 * typical).collect();
    let steady = (0..=beats.len() - STEADY_RUN).find(|&i| strong[i..i + STEADY_RUN].iter().all(|&s| s)).unwrap_or(0);
    let times: Vec<f64> = beats[steady..].iter().map(|b| b.0).collect();
    let beat_us = grid_period(&times, period * FRAME_US);
    let mut beats_us: Vec<i64> = Vec::new();
    let mut k = 1.0;
    while times[0] - k * beat_us >= 0.0 {
        beats_us.push((times[0] - k * beat_us).round() as i64);
        k += 1.0;
    }
    beats_us.reverse();
    let steady = beats_us.len();
    beats_us.extend(times.iter().map(|t| t.round() as i64));
    let level = Levels { energy: &frames.energy };
    let span = SPAN_BEATS as f64 * beat_us;
    let change = |at: f64| {
        let before = (at - span).max(0.0);
        (at - before >= 4.0 * beat_us).then(|| level.db(at, at + span) - level.db(before, at))
    };
    // Music that comes in after a quiet intro comes in on the first beat of a bar.
    let first_downbeat = match change(beats_us[steady] as f64) {
        Some(rise) if rise >= CHANGE_DB => steady % 4,
        _ => downbeat_phase(&beats_us[steady..], beat_us, &frames.low, &level, steady),
    };
    let mut changes: Vec<(usize, f64)> = (first_downbeat..beats_us.len())
        .step_by(4)
        .filter_map(|i| change(beats_us[i] as f64).filter(|c| c.abs() >= CHANGE_DB).map(|c| (i, c)))
        .collect();
    // The biggest changes first, those of the steady beat before any in an intro fading in.
    changes
        .sort_by(|a, b| (a.0 < steady).cmp(&(b.0 < steady)).then(b.1.abs().total_cmp(&a.1.abs())).then(a.0.cmp(&b.0)));
    let mut picked: Vec<(usize, f64)> = Vec::new();
    for (i, c) in changes {
        if picked.iter().all(|p| p.0.abs_diff(i) >= MIN_GAP_BEATS) {
            picked.push((i, c));
        }
    }
    picked.sort_by_key(|p| p.0);
    // A song fades out in steps, and the window contrast peaks at the last step: the quieter part starts at the
    // first bar already nearer its own level than the one before.
    let end = (frames.energy.len() as f64 * FRAME_US).round();
    let bar_db = |i: usize| level.db(beats_us[i] as f64, beats_us.get(i + 4).map_or(end, |&t| t as f64));
    for n in 0..picked.len() {
        let (i, c) = picked[n];
        if c >= 0.0 {
            continue;
        }
        let at = beats_us[i] as f64;
        let middle = (level.db(at - span, at) + level.db(at, at + span)) / 2.0;
        let floor = if n > 0 { picked[n - 1].0 } else { 0 } + 8;
        let mut start = i;
        while start >= floor && i - start < SPAN_BEATS && bar_db(start - 4) < middle {
            start -= 4;
        }
        picked[n].0 = start;
    }
    let drops = picked
        .iter()
        .filter(|p| p.1 > 0.0 && p.0 >= steady)
        .map(|&(beat, rise)| BeatDrop { beat, at_us: beats_us[beat], rise_db: round(rise, 1) })
        .collect();
    let starts: Vec<usize> = std::iter::once(0).chain(picked.iter().map(|p| p.0).filter(|&i| i > 0)).collect();
    let sections = starts
        .iter()
        .enumerate()
        .map(|(n, &beat)| {
            let to = starts.get(n + 1).map_or(end, |&next| beats_us[next] as f64);
            Section { beat, at_us: beats_us[beat], level_db: round(level.db(beats_us[beat] as f64, to), 1) }
        })
        .collect();
    Ok(Beats {
        bpm: round(60e6 / beat_us, 2),
        confidence: round(confidence, 2),
        steady_from_us: beats_us[steady],
        first_downbeat,
        beats_us,
        drops,
        sections,
    })
}

/// The tempo is searched in 45-220 BPM before folding.
const SLOWEST_SEARCH: f64 = 45.0;
const FASTEST_SEARCH: f64 = 220.0;

fn lag(bpm: f64) -> usize {
    (60.0 * RATE / bpm) as usize
}

struct Envelope {
    /// Log-compressed spectral flux of all bins and of the low bins, one value per hop.
    flux: Vec<f64>,
    low: Vec<f64>,
    /// Mean square of the mono samples of each hop.
    energy: Vec<f64>,
}

impl Envelope {
    fn of(samples: &[f32], cancelled: &dyn Fn() -> bool) -> Result<Self> {
        let count = samples.len() / CHANNELS;
        let mono = |i: usize| (samples[i * CHANNELS] + samples[i * CHANNELS + 1]) as f64 / 2.0;
        let n = count / HOP + 1;
        let window: Vec<f64> =
            (0..WINDOW).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / WINDOW as f64).cos()).collect();
        // A sine at full scale reads 0.5.
        let norm = window.iter().sum::<f64>() / 2.0;
        let fft = RealFftPlanner::<f64>::new().plan_fft_forward(WINDOW);
        let (mut input, mut spectrum, mut scratch) =
            (fft.make_input_vec(), fft.make_output_vec(), fft.make_scratch_vec());
        let mut previous = vec![0.0; WINDOW / 2 + 1];
        let (mut flux, mut low) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for k in 0..n {
            if k % CANCEL_FRAMES == 0 && cancelled() {
                bail!("CANCELLED: analysis cancelled");
            }
            // Each window is centred on its frame time, k hops from the start.
            let start = (k * HOP) as isize - (WINDOW / 2) as isize;
            for (i, value) in input.iter_mut().enumerate() {
                let at = start + i as isize;
                *value = if (0..count as isize).contains(&at) { mono(at as usize) * window[i] } else { 0.0 };
            }
            fft.process_with_scratch(&mut input, &mut spectrum, &mut scratch)?;
            let (mut all, mut bass) = (0.0, 0.0);
            for (bin, (value, before)) in spectrum.iter().zip(previous.iter_mut()).enumerate() {
                let level = (1000.0 * value.norm() / norm).ln_1p();
                let rise = (level - *before).max(0.0);
                *before = level;
                if bin >= 1 {
                    all += rise;
                }
                if LOW_BINS.contains(&bin) {
                    bass += rise;
                }
            }
            flux.push(all);
            low.push(bass);
        }
        let energy = (0..count.div_ceil(HOP))
            .map(|k| {
                let block = k * HOP..((k + 1) * HOP).min(count);
                block.clone().map(|i| mono(i).powi(2)).sum::<f64>() / block.len() as f64
            })
            .collect();
        Ok(Self { flux, low, energy })
    }
}

struct Levels<'a> {
    energy: &'a [f64],
}

impl Levels<'_> {
    /// Mean level in dBFS between two times in microseconds, -120 for silence.
    fn db(&self, from_us: f64, to_us: f64) -> f64 {
        let block = |t: f64| ((t / FRAME_US).round().max(0.0) as usize).min(self.energy.len());
        let (a, b) = (block(from_us), block(to_us));
        let power = if b > a { self.energy[a..b].iter().sum::<f64>() / (b - a) as f64 } else { 0.0 };
        (10.0 * power.max(1e-12).log10()).max(-120.0)
    }
}

/// Onsets of all bins and of the bass above their local mean, each scaled to a mean of one.
fn combined_onsets(flux: &[f64], low: &[f64]) -> Vec<f64> {
    let (all, bass) = (above_local_mean(flux), above_local_mean(low));
    let scale = |x: &[f64]| {
        let mean = x.iter().sum::<f64>() / x.len().max(1) as f64;
        if mean > 0.0 { 1.0 / mean } else { 0.0 }
    };
    let (a, b) = (scale(&all), scale(&bass));
    all.iter().zip(&bass).map(|(x, y)| x * a + y * b).collect()
}

/// Bass onsets above their local mean, scaled to a mean of one: what the downbeats are found by.
fn bass_onsets(low: &[f64]) -> Vec<f64> {
    let bass = above_local_mean(low);
    let mean = bass.iter().sum::<f64>() / bass.len().max(1) as f64;
    bass.iter().map(|b| if mean > 0.0 { b / mean } else { 0.0 }).collect()
}

/// Half a second either side.
fn above_local_mean(x: &[f64]) -> Vec<f64> {
    let half = (RATE * 0.5) as usize;
    let mut sums = vec![0.0; x.len() + 1];
    for (i, v) in x.iter().enumerate() {
        sums[i + 1] = sums[i] + v;
    }
    (0..x.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(half), (i + half + 1).min(x.len()));
            (x[i] - (sums[b] - sums[a]) / (b - a) as f64).max(0.0)
        })
        .collect()
}

fn autocorrelation(x: &[f64], max_lag: usize) -> Vec<f64> {
    let size = (2 * x.len()).next_power_of_two();
    let mut planner = RealFftPlanner::<f64>::new();
    let (forward, inverse) = (planner.plan_fft_forward(size), planner.plan_fft_inverse(size));
    let mut input = forward.make_input_vec();
    input[..x.len()].copy_from_slice(x);
    let mut spectrum = forward.make_output_vec();
    forward.process(&mut input, &mut spectrum).expect("buffers sized by their plan");
    for value in &mut spectrum {
        *value = Complex::new(value.norm_sqr(), 0.0);
    }
    let mut output = inverse.make_output_vec();
    inverse.process(&mut spectrum, &mut output).expect("buffers sized by their plan");
    output.truncate(max_lag + 1);
    output
}

fn at(r: &[f64], x: f64) -> f64 {
    let i = x.floor() as usize;
    let f = x - i as f64;
    r[i] * (1.0 - f) + r[(i + 1).min(r.len() - 1)] * f
}

/// The beat period in frames: the lag where the onsets repeat best, weighed toward 120 BPM, then
/// refined over all its multiples in twelve seconds, since a 0.05 BPM error drifts 57 ms in two minutes.
fn tempo(onset: &[f64], lags: std::ops::RangeInclusive<usize>) -> f64 {
    let max_lag = (onset.len() - 1).min((RATE * 12.0) as usize);
    let r = autocorrelation(onset, max_lag);
    let prior = |lag: usize| (-0.5 * ((60.0 * RATE / lag as f64) / 120.0).log2().powi(2)).exp();
    let best = lags.max_by(|&a, &b| (r[a] * prior(a)).total_cmp(&(r[b] * prior(b))).then(b.cmp(&a))).unwrap();
    let comb = |tau: f64| {
        let multiples = (max_lag as f64 / tau) as usize;
        (1..multiples).map(|k| at(&r, k as f64 * tau)).sum::<f64>() / multiples.saturating_sub(1).max(1) as f64
    };
    (0..1500)
        .map(|step| best as f64 - 1.5 + step as f64 * 0.002)
        .max_by(|&a, &b| comb(a).total_cmp(&comb(b)).then(b.total_cmp(&a)))
        .unwrap()
}

fn fold(period: f64) -> f64 {
    let mut bpm = 60.0 * RATE / period;
    while bpm < SLOWEST {
        bpm *= 2.0;
    }
    while bpm >= FASTEST {
        bpm /= 2.0;
    }
    60.0 * RATE / bpm
}

/// How much the onsets repeat at the first eight multiples of the period, 0 for noise.
fn periodicity(onset: &[f64], period: f64) -> f64 {
    let mean = onset.iter().sum::<f64>() / onset.len() as f64;
    let centred: Vec<f64> = onset.iter().map(|o| o - mean).collect();
    let max_lag = (onset.len() - 1).min((9.0 * period) as usize + 1);
    let r = autocorrelation(&centred, max_lag);
    let multiples: Vec<f64> = (1..=8).map(|k| k as f64 * period).filter(|&l| l < max_lag as f64).collect();
    if r[0] <= 0.0 || multiples.is_empty() {
        return 0.0;
    }
    multiples.iter().map(|&l| at(&r, l)).sum::<f64>() / multiples.len() as f64 / r[0]
}

fn deviation(x: &[f64]) -> f64 {
    let mean = x.iter().sum::<f64>() / x.len() as f64;
    let std = (x.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
    if std > 0.0 { std } else { 1.0 }
}

/// Dynamic-programming beat tracking (Ellis 2007): the frames that land on strong onsets while the
/// gaps between them stay near the period.
fn track(onset: &[f64], period: f64, cancelled: &dyn Fn() -> bool) -> Result<Vec<usize>> {
    let n = onset.len();
    let (near, far) = ((period / 2.0).round() as usize, (2.0 * period).round() as usize);
    let penalty: Vec<f64> =
        (0..=far).map(|gap| if gap < near { 0.0 } else { -TIGHTNESS * (gap as f64 / period).ln().powi(2) }).collect();
    let mut score = vec![0.0; n];
    let mut from = vec![usize::MAX; n];
    for i in 0..n {
        if i % CANCEL_FRAMES == 0 && cancelled() {
            bail!("CANCELLED: analysis cancelled");
        }
        score[i] = onset[i];
        if i < near {
            continue;
        }
        let (mut best, mut best_at) = (f64::NEG_INFINITY, 0);
        for p in i.saturating_sub(far)..=i - near {
            let value = score[p] + penalty[i - p];
            if value > best {
                (best, best_at) = (value, p);
            }
        }
        if best > 0.0 {
            score[i] += best;
            from[i] = best_at;
        }
    }
    let tail = (period.round() as usize).min(n);
    let mut beat = (n - tail..n).max_by(|&a, &b| score[a].total_cmp(&score[b]).then(b.cmp(&a))).unwrap();
    let mut beats = vec![beat];
    while from[beat] != usize::MAX {
        beat = from[beat];
        beats.push(beat);
    }
    beats.reverse();
    Ok(beats)
}

/// The beat's own onset peak within 30 ms, between frames, in microseconds.
fn refine(onset: &[f64], beat: usize) -> f64 {
    let (a, b) = (beat.saturating_sub(3), (beat + 3).min(onset.len() - 1));
    let k = (a..=b).max_by(|&x, &y| onset[x].total_cmp(&onset[y]).then(y.cmp(&x))).unwrap();
    if k == 0 || k + 1 == onset.len() {
        return k as f64 * FRAME_US + ONSET_LATENCY_US;
    }
    let (y0, y1, y2) = (onset[k - 1], onset[k], onset[k + 1]);
    let curve = y0 - 2.0 * y1 + y2;
    let offset = if curve < 0.0 { (0.5 * (y0 - y2) / curve).clamp(-0.5, 0.5) } else { 0.0 };
    (k as f64 + offset) * FRAME_US + ONSET_LATENCY_US
}

/// The beat length in microseconds from a line through the beats, each numbered by the grid so a
/// missed beat does not bend it.
fn grid_period(times: &[f64], guess_us: f64) -> f64 {
    if times.len() < 2 {
        return guess_us;
    }
    let numbers: Vec<f64> = times.iter().map(|t| ((t - times[0]) / guess_us).round()).collect();
    let count = times.len() as f64;
    let (mx, my) = (numbers.iter().sum::<f64>() / count, times.iter().sum::<f64>() / count);
    let sxx: f64 = numbers.iter().map(|x| (x - mx).powi(2)).sum();
    let sxy: f64 = numbers.iter().zip(times).map(|(x, y)| (x - mx) * (y - my)).sum();
    if sxx > 0.0 { sxy / sxx } else { guess_us }
}

/// Which beat of four starts a bar: where the bass hits hardest and the music gets louder.
fn downbeat_phase(beats_us: &[i64], beat_us: f64, low: &[f64], level: &Levels, offset: usize) -> usize {
    let bass = bass_onsets(low);
    let features: Vec<(f64, f64)> = beats_us
        .iter()
        .map(|&t| {
            let t = t as f64;
            let frame = (t / FRAME_US).round() as usize;
            let hit = bass[frame.saturating_sub(3).min(bass.len())..(frame + 4).min(bass.len())].iter().sum::<f64>();
            let guard = 20_000.0;
            let jump = if t - beat_us - guard >= 0.0 {
                level.db(t - guard, t + beat_us - guard) - level.db(t - beat_us - guard, t - guard)
            } else {
                0.0
            };
            (hit, jump.max(0.0))
        })
        .collect();
    let mean = |pick: fn(&(f64, f64)) -> f64| {
        let m = features.iter().map(pick).sum::<f64>() / features.len().max(1) as f64;
        if m > 0.0 { 1.0 / m } else { 0.0 }
    };
    let (a, b) = (mean(|f| f.0), mean(|f| f.1));
    let score = |phase: usize| {
        let picked: Vec<f64> = features
            .iter()
            .enumerate()
            .filter(|(i, _)| (i + offset) % 4 == phase)
            .map(|(_, f)| f.0 * a + f.1 * b)
            .collect();
        picked.iter().sum::<f64>() / picked.len().max(1) as f64
    };
    (0..4).max_by(|&x, &y| score(x).total_cmp(&score(y)).then(y.cmp(&x))).unwrap()
}

fn percentile(mut values: Vec<f64>, q: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let rank = q * (values.len() - 1) as f64;
    let (i, f) = (rank.floor() as usize, rank.fract());
    values[i] * (1.0 - f) + values[(i + 1).min(values.len() - 1)] * f
}

fn round(value: f64, decimals: i32) -> f64 {
    let scale = 10f64.powi(decimals);
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A beat length found from beat times numbered by the grid keeps its precision across a beat
    /// the tracker missed.
    #[test]
    fn a_missed_beat_does_not_bend_the_tempo() {
        let beat = 600_000.0;
        let mut times: Vec<f64> = (0..40).map(|i| 1_000_000.0 + i as f64 * beat).collect();
        times.remove(17);
        assert!((grid_period(&times, 600_500.0) - beat).abs() < 1e-6);
    }

    #[test]
    fn tempi_fold_into_one_octave() {
        let bpm = |period: f64| 60.0 * RATE / fold(period);
        assert!((bpm(60.0 * RATE / 180.0) - 90.0).abs() < 1e-9);
        assert!((bpm(60.0 * RATE / 64.0) - 128.0).abs() < 1e-9);
        assert!((bpm(60.0 * RATE / 160.0) - 80.0).abs() < 1e-9);
        assert!((bpm(60.0 * RATE / 78.0) - 78.0).abs() < 1e-9);
    }
}

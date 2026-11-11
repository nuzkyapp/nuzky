//! Steady tones that are not the voice: mains hum at 50 or 60 Hz with its harmonics, and a whine, one high tone
//! such as a monitor's, a charger's or a fan's. A voice keeps changing pitch and pauses, so in the level every
//! frequency keeps for most of the recording, its quieter fifth, the voice fades and a steady tone stands out of
//! the noise around it as one narrow peak.
use realfft::RealFftPlanner;
use serde::Serialize;

pub const SAMPLE_RATE: f64 = 48_000.0;
/// 2.9 Hz between bins: 50 and 60 Hz and their harmonics fall on separate bins.
const SIZE: usize = 16_384;
/// Frames looked at; a longer recording is sampled evenly, about three minutes of it.
const MAX_FRAMES: usize = 600;
/// The level a frequency keeps in this share of the frames, the quieter end, where the voice is mostly absent.
const FLOOR_SHARE: f64 = 0.2;
/// The noise around a bin: this many bins either side, without the bins of the peak itself.
const AROUND: usize = 24;
const PEAK_HALF_WIDTH: usize = 3;
/// How far above the noise around it a tone stands to count.
const HUM_DB: f64 = 10.0;
const WHINE_DB: f64 = 12.0;
const WHINE_FROM_HZ: f64 = 1_000.0;
const MAX_WHINES: usize = 5;
const HARMONICS: usize = 10;
/// 50 ms at 48 kHz.
const NOISE_WINDOW: usize = 2_400;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Spectrum {
    /// Mains hum: its measured fundamental and each harmonic that stands out.
    pub hum: Option<Hum>,
    /// High steady tones, strongest first.
    pub whine: Vec<Tone>,
    /// RMS dBFS of the quietest tenth of the recording in 50 ms windows: the room and the microphone's noise.
    pub noise_floor_db: f64,
    /// Microseconds of sound looked at.
    pub analysed_us: i64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Hum {
    /// 50 or 60, as the mains of the place.
    pub mains_hz: u32,
    pub harmonics: Vec<Tone>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Tone {
    pub hz: f64,
    /// Peak level of the tone, dBFS.
    pub level_db: f64,
    /// How far it stands above the noise beside it.
    pub above_noise_db: f64,
}

/// Interleaved `samples` of `channels` at 48 kHz, read in place, the channels averaged; at least one frame of 0.34 s.
/// Only the frames looked at are read, so a recording of hours costs no more than three minutes of it.
pub fn spectrum(samples: &[f32], channels: usize) -> Option<Spectrum> {
    let channels = channels.max(1);
    let length = samples.len() / channels;
    if length < SIZE {
        return None;
    }
    let frames = length / SIZE;
    let take = frames.min(MAX_FRAMES);
    let starts: Vec<usize> = (0..take).map(|i| i * frames / take * SIZE).collect();
    let mono = |start: usize| {
        samples[start * channels..(start + SIZE) * channels]
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
    };
    let window: Vec<f32> =
        (0..SIZE).map(|n| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / SIZE as f64).cos()) as f32).collect();
    let fft = RealFftPlanner::<f32>::new().plan_fft_forward(SIZE);
    let (mut input, mut output) = (fft.make_input_vec(), fft.make_output_vec());
    let bins = output.len();
    let mut levels: Vec<Vec<f32>> = vec![Vec::with_capacity(take); bins];
    // RMS of each 50 ms of the frames looked at, for the noise floor.
    let mut quiet = Vec::with_capacity(take * SIZE / NOISE_WINDOW);
    for &start in &starts {
        for ((x, s), w) in input.iter_mut().zip(mono(start)).zip(&window) {
            *x = s * w;
        }
        let frame: Vec<f32> = mono(start).collect();
        quiet.extend(
            frame
                .as_chunks::<NOISE_WINDOW>()
                .0
                .iter()
                .map(|w| w.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / NOISE_WINDOW as f64),
        );
        if fft.process(&mut input, &mut output).is_err() {
            return None;
        }
        for (level, c) in levels.iter_mut().zip(&output) {
            level.push(c.norm_sqr());
        }
    }
    // A sine of amplitude A peaks at A·N/4 through a Hann window.
    let to_db = |power: f64| 20.0 * (4.0 * power.sqrt() / SIZE as f64).max(1e-9).log10();
    let floor: Vec<f64> = levels
        .iter_mut()
        .map(|level| {
            level.sort_by(f32::total_cmp);
            to_db(f64::from(level[((level.len() - 1) as f64 * FLOOR_SHARE).round() as usize]))
        })
        .collect();
    let noise: Vec<f64> = (0..bins).map(|k| around(&floor, k)).collect();
    let hz = |k: f64| k * SAMPLE_RATE / SIZE as f64;
    // The strongest bin near `k` and its frequency and level between bins, from a parabola through three bins.
    let peak = |k: usize| -> Option<Tone> {
        if k == 0 || k + 1 >= bins {
            return None;
        }
        let (a, b, c) = (floor[k - 1], floor[k], floor[k + 1]);
        let denominator = a - 2.0 * b + c;
        let offset = if denominator.abs() > 1e-9 { (0.5 * (a - c) / denominator).clamp(-0.5, 0.5) } else { 0.0 };
        let level = b - 0.25 * (a - c) * offset;
        Some(Tone {
            hz: round(hz(k as f64 + offset), 1),
            level_db: round(level, 1),
            above_noise_db: round(level - noise[k], 1),
        })
    };
    let local_max = |k: usize| k > 0 && k + 1 < bins && floor[k] >= floor[k - 1] && floor[k] >= floor[k + 1];
    let near = |target: f64, tolerance: f64| -> Option<usize> {
        let (lo, hi) = (
            ((target - tolerance) * SIZE as f64 / SAMPLE_RATE).floor() as usize,
            ((target + tolerance) * SIZE as f64 / SAMPLE_RATE).ceil() as usize,
        );
        (lo.max(1)..=hi.min(bins - 2)).max_by(|a, b| floor[*a].total_cmp(&floor[*b]))
    };
    let mut hum: Option<(Hum, Vec<usize>)> = None;
    for mains in [50u32, 60] {
        let mut found = Vec::new();
        let mut at = Vec::new();
        for h in 1..=HARMONICS {
            let target = f64::from(mains) * h as f64;
            let Some(k) = near(target, 3.0 + target * 0.004) else { continue };
            if let Some(tone) = peak(k).filter(|t| local_max(k) && t.above_noise_db >= HUM_DB) {
                found.push(tone);
                at.push(k);
            }
        }
        let strong = found.iter().any(|t| t.above_noise_db >= HUM_DB + 5.0);
        if (found.len() >= 2 || strong) && hum.as_ref().is_none_or(|(h, _)| found.len() > h.harmonics.len()) {
            hum = Some((Hum { mains_hz: mains, harmonics: found }, at));
        }
    }
    let hum_bins = hum.as_ref().map(|(_, at)| at.clone()).unwrap_or_default();
    let first = (WHINE_FROM_HZ * SIZE as f64 / SAMPLE_RATE) as usize;
    let mut whine: Vec<Tone> = (first..bins - 1)
        .filter(|&k| {
            local_max(k)
                && floor[k] - noise[k] >= WHINE_DB
                && !hum_bins.iter().any(|h| h.abs_diff(k) <= PEAK_HALF_WIDTH)
        })
        .filter_map(peak)
        .collect();
    whine.sort_by(|a, b| b.above_noise_db.total_cmp(&a.above_noise_db));
    // One tone spreads over neighbouring bins; keep the strongest of each.
    let mut kept: Vec<Tone> = Vec::new();
    for tone in whine {
        if kept.iter().all(|k| (k.hz - tone.hz).abs() > 20.0) && kept.len() < MAX_WHINES {
            kept.push(tone);
        }
    }
    Some(Spectrum {
        hum: hum.map(|(h, _)| h),
        whine: kept,
        noise_floor_db: round(noise_floor(quiet), 1),
        analysed_us: (take * SIZE) as i64 * 1_000_000 / SAMPLE_RATE as i64,
    })
}

/// The median level of the bins around `k`, without the peak's own bins.
fn around(floor: &[f64], k: usize) -> f64 {
    let (lo, hi) = (k.saturating_sub(AROUND), (k + AROUND).min(floor.len() - 1));
    let mut values: Vec<f64> = (lo..=hi).filter(|i| i.abs_diff(k) > PEAK_HALF_WIDTH).map(|i| floor[i]).collect();
    if values.is_empty() {
        return floor[k];
    }
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// The level of the quietest tenth of `power`, mean squares of 50 ms windows, in dBFS.
fn noise_floor(mut power: Vec<f64>) -> f64 {
    if power.is_empty() {
        return -120.0;
    }
    power.sort_by(f64::total_cmp);
    let quiet = &power[..(power.len() / 10).max(1)];
    (10.0 * (quiet.iter().sum::<f64>() / quiet.len() as f64).max(1e-12).log10()).max(-120.0)
}

fn round(v: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    (v * k).round() / k
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speech-like bursts: a voice gliding between 120 and 220 Hz with harmonics, on for 0.4 s, off for 0.3 s,
    /// over white noise at -60 dBFS.
    fn speech(seconds: f64, seed: u64) -> Vec<f32> {
        let n = (seconds * SAMPLE_RATE) as usize;
        let mut state = seed;
        let mut noise = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) as f64 / (1u64 << 31) as f64 - 1.0) * 0.001 * 3f64.sqrt()
        };
        let mut phase = 0.0;
        (0..n)
            .map(|i| {
                let t = i as f64 / SAMPLE_RATE;
                let on = (t % 0.7) < 0.4;
                let pitch = 170.0 + 50.0 * (t * 1.3).sin();
                phase += 2.0 * std::f64::consts::PI * pitch / SAMPLE_RATE;
                let voice =
                    if on { (1..=8).map(|h| 0.15 / h as f64 * (phase * h as f64).sin()).sum::<f64>() } else { 0.0 };
                (voice + noise()) as f32
            })
            .collect()
    }

    fn add_tone(samples: &mut [f32], hz: f64, amplitude: f64) {
        for (i, s) in samples.iter_mut().enumerate() {
            *s += (amplitude * (2.0 * std::f64::consts::PI * hz * i as f64 / SAMPLE_RATE).sin()) as f32;
        }
    }

    #[test]
    fn mains_hum_and_its_harmonics_are_found_and_told_50_from_60() {
        for mains in [50.0, 60.0] {
            let mut sound = speech(12.0, 7);
            for (h, amplitude) in [(1.0, 0.01), (2.0, 0.006), (3.0, 0.004)] {
                add_tone(&mut sound, mains * h, amplitude);
            }
            let found = spectrum(&sound, 1).unwrap();
            let hum = found.hum.as_ref().unwrap_or_else(|| panic!("no hum at {mains}: {found:?}"));
            assert_eq!(f64::from(hum.mains_hz), mains, "{found:?}");
            assert!(hum.harmonics.len() >= 3, "{found:?}");
            let first = &hum.harmonics[0];
            assert!((first.hz - mains).abs() < 0.5 && (first.level_db - -40.0).abs() < 1.5, "{first:?}");
            assert!(found.whine.is_empty(), "{found:?}");
        }
    }

    #[test]
    fn a_high_whine_is_found_and_the_voice_is_not() {
        let mut sound = speech(12.0, 3);
        add_tone(&mut sound, 15_734.0, 0.003);
        let found = spectrum(&sound, 1).unwrap();
        assert!(found.hum.is_none(), "{found:?}");
        assert_eq!(found.whine.len(), 1, "{found:?}");
        let whine = &found.whine[0];
        assert!((whine.hz - 15_734.0).abs() < 1.0 && (whine.level_db - -50.5).abs() < 1.5, "{whine:?}");
    }

    #[test]
    fn a_clean_voice_has_neither() {
        let found = spectrum(&speech(12.0, 11), 1).unwrap();
        assert!(found.hum.is_none() && found.whine.is_empty(), "{found:?}");
        // The noise between the bursts: RMS 0.001.
        assert!((found.noise_floor_db - -60.0).abs() < 1.0, "{found:?}");
        assert!(spectrum(&[0.0; 100], 1).is_none());
        // Two channels read as their average: a tone on one side only is half as loud.
        let mut left = speech(12.0, 5);
        add_tone(&mut left, 15_734.0, 0.006);
        let stereo: Vec<f32> = left.iter().flat_map(|s| [*s, 0.0]).collect();
        let whine = &spectrum(&stereo, 2).unwrap().whine[0];
        assert!((whine.level_db - -50.5).abs() < 1.5, "{whine:?}");
    }
}

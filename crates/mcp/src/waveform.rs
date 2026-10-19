//! The sound of a stretch of the timeline as one picture, with what each cut threw away shaded in
//! place, so an agent sees a syllable it cut off or a breath it kept.
use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use nuzky_engine::{
    Project,
    audio::{Mixer, Pcm, pcm_path, samples_to_us, us_to_samples},
    gpu::Image,
    model::{Asset, CHANNELS, Clip, ClipContent, TextStyle},
    speech::{Word, is_heard},
    text::TextRenderer,
};
use serde_json::{Value, json};

pub const MAX_RANGE_US: i64 = 60_000_000;
pub const DEFAULT_WIDTH: u32 = 1200;
/// How much of what a cut threw away shows on each side of it.
const SHOWN_US: i64 = 500_000;
/// Rows of the picture: times, the waveform, then word brackets and up to three rows of words.
const TIMES: u32 = 16;
const WAVE: u32 = 140;
const WORDS: u32 = TIMES + WAVE + 6;
const WORD_ROW: u32 = 18;
const WORD_ROWS: usize = 3;
const HEIGHT: u32 = WORDS + WORD_ROWS as u32 * WORD_ROW;
/// Peaks show from -60 dBFS, so a breath or room noise is visible beside speech.
const FLOOR_DB: f32 = -60.0;
/// A word is split when at least this much of it is heard and this much thrown away.
const SPLIT_US: i64 = 20_000;

const PANEL: [u8; 3] = [0x1c, 0x1c, 0x1f];
const SHADE: [u8; 3] = [0x0c, 0x0c, 0x0e];
const HATCH: [u8; 3] = [0x2c, 0x2c, 0x30];
const HEARD: [u8; 3] = [0x29, 0x97, 0xff];
const THROWN: [u8; 3] = [0x6e, 0x6e, 0x75];
const CUT: [u8; 3] = [0xff, 0x9f, 0x0a];
const WORD: [u8; 3] = [0xf2, 0xf2, 0xf5];
const WORD_THROWN: [u8; 3] = [0xa1, 0xa1, 0xa9];
const WORD_SPLIT: [u8; 3] = [0xff, 0x45, 0x3a];

/// A stretch of the picture: timeline as heard, or source a cut threw away.
enum Part<'a> {
    Heard { from: i64, to: i64 },
    Thrown { asset: &'a Asset, from: i64, to: i64, speed: f64, volume: f32, cut: usize },
}

impl Part<'_> {
    /// How long it plays, in microseconds.
    fn length(&self) -> f64 {
        match self {
            Part::Heard { from, to } => (to - from) as f64,
            Part::Thrown { from, to, speed, .. } => (to - from) as f64 / speed,
        }
    }
}

/// A word, or the part of it one stretch shows, with how much of it is heard and how much each cut
/// threw away.
struct Span<'a> {
    asset: &'a str,
    start: i64,
    text: &'a str,
    x0: f64,
    x1: f64,
    heard: i64,
    thrown: Vec<(usize, i64)>,
}

impl Span<'_> {
    /// Whether `cut`, or any cut when `None`, split the word into a heard and a thrown-away part.
    fn split(&self, cut: Option<usize>) -> bool {
        self.heard >= SPLIT_US && self.thrown.iter().any(|&(by, us)| cut.is_none_or(|cut| cut == by) && us >= SPLIT_US)
    }
}

/// A PNG of `start..end` and its cuts, `{t_us, split_words}`. `words` are the source words of each
/// heard file.
pub fn draw(
    project: &Project,
    words: &HashMap<String, Vec<Word>>,
    cache: &Path,
    (start, end): (i64, i64),
    width: u32,
) -> Result<(Vec<u8>, Vec<Value>)> {
    // Speech alone: music under it would hide the gaps between words.
    let mut speech = project.clone();
    for (track, original) in speech.tracks.iter_mut().zip(&project.tracks) {
        track.clips.retain(|clip| is_heard(project, original, clip));
    }
    let (parts, cuts) = parts(&speech, start, end);
    let per_px = parts.iter().map(Part::length).sum::<f64>() / f64::from(width);
    let mut starts = Vec::with_capacity(parts.len());
    parts.iter().fold(0.0, |at, part| {
        starts.push(at);
        at + part.length()
    });
    let x_of = |k: usize| ((starts[k] / per_px).round() as u32).min(width - 1);

    let first = us_to_samples(start);
    let mut mix = vec![0f32; (us_to_samples(end) - first) as usize * CHANNELS];
    Mixer::new(cache.to_path_buf()).mix(&speech, first, &mut mix);
    let mut files: HashMap<&str, Option<Pcm>> = HashMap::new();
    let mut pixels = vec![0u8; (width * HEIGHT * 4) as usize];
    pixels.as_chunks_mut::<4>().0.fill([PANEL[0], PANEL[1], PANEL[2], 255]);
    for x in 0..width {
        let (a, b) = (f64::from(x) * per_px, f64::from(x + 1) * per_px);
        let k = starts.partition_point(|s| *s <= (a + b) / 2.0).max(1) - 1;
        let (lo, hi) = (a.max(starts[k]) - starts[k], b.min(starts[k] + parts[k].length()) - starts[k]);
        let (peak, thrown) = match &parts[k] {
            Part::Heard { from, .. } => {
                let s0 = (us_to_samples(from + lo as i64) - first) as usize;
                let s1 = (us_to_samples(from + hi.ceil() as i64) - first) as usize;
                (loudest(&mix, s0, s1.max(s0 + 1)), false)
            }
            Part::Thrown { asset, from, speed, volume, .. } => {
                let pcm = files.entry(&asset.id).or_insert_with(|| Pcm::open(&pcm_path(cache, asset)).ok());
                let pcm = pcm.as_ref().context("AUDIO_NOT_READY: the sound of a file is not prepared")?;
                let s0 = us_to_samples(from + (lo * speed) as i64) as usize;
                let s1 = us_to_samples(from + (hi * speed).ceil() as i64) as usize;
                (loudest(pcm.samples(), s0, s1.max(s0 + 1)) * volume, true)
            }
        };
        let db = 20.0 * peak.max(1e-6).log10();
        let half = (((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0) * (WAVE / 2 - 2) as f32).max(0.5);
        let middle = (TIMES + WAVE / 2) as f32;
        for y in TIMES..TIMES + WAVE {
            let colour = match (thrown, (y as f32 - middle).abs() <= half) {
                (false, true) => HEARD,
                (false, false) => PANEL,
                (true, true) => THROWN,
                (true, false) if (x + y) % 6 == 0 => HATCH,
                (true, false) => SHADE,
            };
            put(&mut pixels, width, x, y, colour);
        }
    }
    for k in 1..parts.len() {
        let x = x_of(k);
        if matches!(parts[k - 1], Part::Heard { .. }) != matches!(parts[k], Part::Heard { .. }) {
            for y in 0..TIMES + WAVE {
                put(&mut pixels, width, x, y, CUT);
                put(&mut pixels, width, x.saturating_sub(1), y, CUT);
            }
        } else {
            for y in (TIMES..TIMES + WAVE).filter(|y| y % 6 < 3) {
                put(&mut pixels, width, x, y, CUT);
            }
        }
    }

    let cut_x = cuts.iter().enumerate().map(|(i, t)| {
        let k = parts.iter().position(|part| matches!(part, Part::Thrown { cut, .. } if *cut == i));
        (k.map_or(0, x_of) + 4, *t)
    });
    let mut free = 0;
    for (x, t) in std::iter::once((4, start)).chain(cut_x) {
        let text = seconds(t);
        if x >= free && x + text.len() as u32 * 8 <= width {
            crate::media::label(&mut pixels, width, x, 3, &text, width - x);
            free = x + text.len() as u32 * 8 + 8;
        }
    }
    let text = seconds(end);
    let x = width.saturating_sub(text.len() as u32 * 8 + 4);
    if x >= free {
        crate::media::label(&mut pixels, width, x, 3, &text, width - x);
    }

    let placed = place_words(&speech, words, &parts, &starts, per_px);
    let mut fonts = TextRenderer::new();
    let mut rows = [0f64; WORD_ROWS];
    for span in &placed {
        let colour = match (span.split(None), span.heard) {
            (true, _) => WORD_SPLIT,
            (false, 0) => WORD_THROWN,
            _ => WORD,
        };
        // A pixel short at the end, so words said without a pause between them stay apart.
        let (x0, x1) = (span.x0.max(0.0), span.x1.min(f64::from(width)) - 1.0);
        for x in x0 as u32..(x1.ceil() as u32).max(x0 as u32 + 1).min(width) {
            for y in WORDS - 4..WORDS - 2 {
                put(&mut pixels, width, x, y, colour);
            }
        }
        let Some(row) = rows.iter().position(|free| x0 >= *free) else { continue };
        let style = TextStyle {
            font_family: None,
            font_size: 12.0,
            color: format!("#{:02x}{:02x}{:02x}", colour[0], colour[1], colour[2]),
            bold: span.split(None),
            stroke_width: 0.0,
            stroke_color: "#000000".into(),
            background: None,
            max_width: None,
            highlight: None,
            keywords: None,
            shadow: None,
            letter_case: Default::default(),
            letter_spacing: 0.0,
            line_spacing: 0.0,
        };
        let image = fonts.render(span.text.trim(), &style, 1.0, 10_000.0).image;
        rows[row] = x0 + f64::from(image.width) + 2.0;
        blend(&mut pixels, width, x0 as u32, WORDS + row as u32 * WORD_ROW, &image);
    }

    let cuts = cuts
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let split: Vec<&str> = placed.iter().filter(|s| s.split(Some(i))).map(|s| s.text.trim()).collect();
            json!({"t_us": t, "split_words": split})
        })
        .collect();
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.write_header().context("Writing PNG header")?.write_image_data(&pixels).context("Encoding waveform")?;
    Ok((png, cuts))
}

/// The stretches of the picture in order, and the time of each cut. At every edge of a heard clip in
/// `start..=end` where its file goes on, up to `SHOWN_US` of what was thrown away shows on each side:
/// the rest of the clip that stops there, then the lead-in of the clip that starts there. Where the
/// next clip skips less than twice that of the same file, the whole skipped part shows once.
fn parts(speech: &Project, start: i64, end: i64) -> (Vec<Part<'_>>, Vec<i64>) {
    let mut pieces = Vec::new();
    for track in &speech.tracks {
        for (i, clip) in track.clips.iter().enumerate() {
            let ClipContent::Media { asset_id, source_in_us, speed, volume, .. } = &clip.content else { continue };
            let Some(asset) = speech.asset(asset_id) else { continue };
            let speed = f64::from(*speed);
            let shown = (SHOWN_US as f64 * speed) as i64;
            let out = source_in_us + (clip.duration_us as f64 * speed).round() as i64;
            let piece = |from, to| Part::Thrown { asset, from, to, speed, volume: *volume, cut: 0 };
            if (start..=end).contains(&clip.end_us())
                && out < asset.duration_us
                && !track.clips.get(i + 1).is_some_and(|next| plays_on(clip, next))
            {
                pieces.push((clip.end_us(), false, piece(out, (out + shown).min(asset.duration_us))));
            }
            if (start..=end).contains(&clip.start_us)
                && *source_in_us > 0
                && !i.checked_sub(1).is_some_and(|i| plays_on(&track.clips[i], clip))
            {
                pieces.push((clip.start_us, true, piece((source_in_us - shown).max(0), *source_in_us)));
            }
        }
    }
    pieces.sort_by_key(|(t, starts, _)| (*t, *starts));
    let (mut parts, mut cuts) = (Vec::new(), Vec::new());
    let mut at = start;
    let mut pieces = pieces.into_iter().peekable();
    while let Some((t, starts, mut piece)) = pieces.next() {
        if t > at {
            parts.push(Part::Heard { from: at, to: t });
            at = t;
        }
        if cuts.last() != Some(&t) {
            cuts.push(t);
        }
        if let Part::Thrown { asset, from, to, speed, cut, .. } = &mut piece {
            *cut = cuts.len() - 1;
            if let Some((next_t, true, Part::Thrown { asset: next, to: skipped_to, speed: next_speed, .. })) =
                pieces.peek()
                && !starts
                && *next_t == t
                && next.id == asset.id
                && next_speed == speed
                && (0..=2 * (SHOWN_US as f64 * *speed) as i64).contains(&(skipped_to - *from))
            {
                *to = *skipped_to;
                pieces.next();
            }
        }
        parts.push(piece);
    }
    if at < end {
        parts.push(Part::Heard { from: at, to: end });
    }
    (parts, cuts)
}

/// Whether `next` plays on where `clip` stops in the same file, as the halves of a split do, also at
/// another speed: nothing of the file is thrown away between them.
fn plays_on(clip: &Clip, next: &Clip) -> bool {
    let (
        ClipContent::Media { asset_id: a, source_in_us: in_a, speed, .. },
        ClipContent::Media { asset_id: b, source_in_us: in_b, .. },
    ) = (&clip.content, &next.content)
    else {
        return false;
    };
    let out = in_a + (clip.duration_us as f64 * f64::from(*speed)).round() as i64;
    a == b && clip.end_us() == next.start_us && (in_b - out).abs() <= samples_to_us(1)
}

/// The words each stretch shows, at their places in the picture, in order. A word that goes on from
/// heard sound into the sound its cut threw away is one span with both.
fn place_words<'a>(
    speech: &Project,
    words: &'a HashMap<String, Vec<Word>>,
    parts: &[Part],
    starts: &[f64],
    per_px: f64,
) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    for (part, at) in parts.iter().zip(starts) {
        match part {
            Part::Heard { from, to } => {
                for clip in speech.tracks.iter().flat_map(|track| &track.clips) {
                    let ClipContent::Media { asset_id, source_in_us, speed, .. } = &clip.content else { continue };
                    let (a, b) = ((*from).max(clip.start_us), (*to).min(clip.end_us()));
                    let Some((asset, words)) = words.get_key_value(asset_id).filter(|_| a < b) else { continue };
                    let speed = f64::from(*speed);
                    let source = |t: i64| source_in_us + ((t - clip.start_us) as f64 * speed).round() as i64;
                    let x = |s: i64| (at + (clip.start_us - from) as f64 + (s - source_in_us) as f64 / speed) / per_px;
                    let (s0, s1) = (source(a), source(b));
                    for word in words.iter().filter(|w| w.start_us < s1 && w.end_us > s0) {
                        let (ws, we) = (word.start_us.max(s0), word.end_us.min(s1));
                        let (start, text) = (word.start_us, word.text.as_str());
                        let (x0, x1) = (x(ws), x(we));
                        spans.push(Span { asset, start, text, x0, x1, heard: we - ws, thrown: Vec::new() });
                    }
                }
            }
            Part::Thrown { asset, from, to, speed, cut, .. } => {
                let Some((asset, words)) = words.get_key_value(&asset.id) else { continue };
                let x = |s: i64| (at + (s - from) as f64 / speed) / per_px;
                for word in words.iter().filter(|w| w.start_us < *to && w.end_us > *from) {
                    let (ws, we) = (word.start_us.max(*from), word.end_us.min(*to));
                    let (start, text) = (word.start_us, word.text.as_str());
                    let (x0, x1) = (x(ws), x(we));
                    spans.push(Span { asset, start, text, x0, x1, heard: 0, thrown: vec![(*cut, we - ws)] });
                }
            }
        }
    }
    spans.sort_by(|a, b| (a.asset, a.start).cmp(&(b.asset, b.start)).then(a.x0.total_cmp(&b.x0)));
    let mut placed: Vec<Span> = Vec::new();
    for span in spans {
        match placed.last_mut() {
            Some(last) if (last.asset, last.start) == (span.asset, span.start) && span.x0 - last.x1 <= 1.0 => {
                last.x1 = last.x1.max(span.x1);
                last.heard += span.heard;
                last.thrown.extend(span.thrown);
            }
            _ => placed.push(span),
        }
    }
    placed.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    placed
}

fn seconds(t: i64) -> String {
    format!("{}.{:03}", t / 1_000_000, t % 1_000_000 / 1000)
}

/// The loudest sample of frames `from..to` of interleaved `samples`.
fn loudest(samples: &[f32], from: usize, to: usize) -> f32 {
    let (from, to) = ((from * CHANNELS).min(samples.len()), (to * CHANNELS).min(samples.len()));
    samples[from..to].iter().fold(0.0, |m, s| m.max(s.abs()))
}

fn put(pixels: &mut [u8], width: u32, x: u32, y: u32, colour: [u8; 3]) {
    let i = ((y * width + x) * 4) as usize;
    pixels[i..i + 3].copy_from_slice(&colour);
}

/// Draws straight-alpha `image` over the picture with its top left at `x, y`, cut at the picture's edges.
fn blend(pixels: &mut [u8], width: u32, x: u32, y: u32, image: &Image) {
    for row in 0..image.height.min(HEIGHT.saturating_sub(y)) {
        for col in 0..image.width.min(width.saturating_sub(x)) {
            let src = &image.data[((row * image.width + col) * 4) as usize..][..4];
            let dst = (((y + row) * width + x + col) * 4) as usize;
            let alpha = u16::from(src[3]);
            for c in 0..3 {
                pixels[dst + c] =
                    ((u16::from(src[c]) * alpha + u16::from(pixels[dst + c]) * (255 - alpha)) / 255) as u8;
            }
        }
    }
}

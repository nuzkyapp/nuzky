//! A recap: a short vertical video of how a video was cut. The recording plays fast with what was cut
//! marked, the transcript strikes the cut words, a counter falls from the length before to the length
//! after, and a sample of the finished cut closes it. Its pictures come from the renderer of preview
//! and export, so the recording and the cut look as they do in the app.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use ffmpeg_next as ff;

use crate::audio::{Mixer, ensure_pcm, us_to_samples};
use crate::edit::MAIN_TRACK;
use crate::export::{
    Mp4Writer, Quality, check_cancel, check_source_path, frame_count, frame_time_us, prepare_sound, write_beside,
    write_credits,
};
use crate::gpu::{Draw, Image, Layer, Mask};
use crate::media::{Transfer, init};
use crate::model::{
    Adjust, Asset, Background, CHANNELS, Canvas, Clip, ClipContent, Project, TextStyle, Track, TrackKind, Transform,
    parse_color,
};
use crate::render::{Pending, Renderer, WHOLE, Wait};

/// What a recap shows. `cuts` and `words` are on the timeline of `before`.
#[derive(Clone, Debug)]
pub struct Recap {
    pub before: Project,
    pub after: Project,
    /// Time of `before` that `after` no longer plays, in order, apart and not touching.
    pub cuts: Vec<RecapCut>,
    /// The transcript of `before`, in order; empty when `transcribed` is false.
    pub words: Vec<RecapWord>,
    /// Every file a cut touches has a transcript, so words, retakes and pauses are known.
    pub transcribed: bool,
    /// Built-in sounds the recap plays: a tick as each cut passes and a whoosh between its parts.
    pub tick: Option<Asset>,
    pub whoosh: Option<Asset>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecapCut {
    pub start_us: i64,
    pub end_us: i64,
    /// Transcript words whose middle falls in the cut.
    pub words: usize,
    /// Retakes the cut removed: attempts at a sentence said again later, and whole takes.
    pub retakes: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecapWord {
    pub start_us: i64,
    pub end_us: i64,
    pub text: String,
    /// Its middle falls in a cut.
    pub removed: bool,
}

/// What a recap counts, the same numbers the run summary gives.
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecapTotals {
    pub before_us: i64,
    pub after_us: i64,
    /// `None` while a file a cut touches has no transcript.
    pub words: Option<usize>,
    pub retakes: Option<usize>,
    /// Cuts without a word.
    pub pauses: Option<usize>,
}

impl Recap {
    pub fn totals(&self) -> RecapTotals {
        let known = |n: usize| self.transcribed.then_some(n);
        RecapTotals {
            before_us: self.before.duration_us(),
            after_us: self.after.duration_us(),
            words: known(self.cuts.iter().map(|c| c.words).sum()),
            retakes: known(self.cuts.iter().map(|c| c.retakes).sum()),
            pauses: known(self.cuts.iter().filter(|c| c.words == 0).count()),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RecapOptions {
    /// A short card with nuzky.app at the end.
    pub end_card: bool,
    pub replace_existing: bool,
}

const HOOK_US: i64 = 1_500_000;
/// The recording plays at least this fast and takes at most this long.
const FAST_MIN_US: i64 = 3_000_000;
const FAST_MAX_US: i64 = 17_000_000;
const HOLD_US: i64 = 600_000;
const SAMPLE_MAX_US: i64 = 8_000_000;
const END_CARD_US: i64 = 1_500_000;

/// The parts of a recap, as lengths: the opening numbers, the recording played fast, the counter held,
/// the finished cut and the end card (0 when off).
pub fn sections(recap: &Recap, end_card: bool) -> [i64; 5] {
    let before = recap.before.duration_us();
    let fast = (before / 4).clamp(FAST_MIN_US, FAST_MAX_US).min(before);
    let sample = recap.after.duration_us().min(SAMPLE_MAX_US);
    [HOOK_US, fast, HOLD_US, sample, if end_card { END_CARD_US } else { 0 }]
}

pub fn duration_us(recap: &Recap, end_card: bool) -> i64 {
    sections(recap, end_card).iter().sum()
}

/// A length as the run summary writes it: "45.2 s" under a minute, "3:42" from a minute up.
pub fn duration_text(us: i64) -> String {
    let tenths = (us.max(0) + 50_000) / 100_000;
    if tenths < 600 {
        return format!("{}.{} s", tenths / 10, tenths % 10);
    }
    let secs = (us + 500_000) / 1_000_000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Writes the recap to `out` through a temporary `.nuzky-part-*` beside it, as export does.
pub fn render(
    recap: &Recap,
    cache_dir: &Path,
    out: &Path,
    options: &RecapOptions,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<()> {
    init();
    ensure!(
        recap.before.duration_us() > 0 && recap.after.duration_us() > 0,
        "There is nothing to recap: the video is empty"
    );
    check_source_path(&recap.before, out)?;
    check_source_path(&recap.after, out)?;
    // The finished cut plays its CC BY sounds, so they are credited beside the recap as beside an export.
    let credits = crate::credits::credits(&recap.after);
    let credits_out = crate::credits::credits_path(out);
    if credits.is_some() && credits_out.symlink_metadata().is_ok() {
        bail!("OUTPUT_EXISTS: {} already exists; choose a new file name", credits_out.display());
    }
    prepare_sound(&recap.after, cache_dir, cancel)?;
    for asset in [&recap.tick, &recap.whoosh].into_iter().flatten() {
        ensure_pcm(cache_dir, asset, |_| check_cancel(cancel))?;
    }
    crate::matte::check(cache_dir, &recap.after)?;
    write_beside(out, options.replace_existing, cancel, |tmp| {
        encode(recap, cache_dir, tmp, options.end_card, cancel, &mut progress)
    })?;
    match credits {
        Some(text) => write_credits(&text, &credits_out, false).with_context(|| {
            format!("The recap is saved, but its credits could not be written to {}", credits_out.display())
        }),
        None => Ok(()),
    }
}

const WIDTH: u32 = 1080;
const HEIGHT: u32 = 1920;
const FPS: u32 = 30;
/// What Reels and TikTok leave free on a 1080 × 1920 video (`Canvas::safe_area`): everything that says
/// something stays inside.
const LEFT: f32 = 60.0;
const RIGHT: f32 = 900.0;
const TOP: f32 = 250.0;
const CAPTION_BOTTOM: f32 = 1405.0;

const FG: &str = "#f2f2f5";
/// Labels over the picture: 6:1 on the chip even over white footage.
const LABEL: &str = "#d1d1d6";
const RED: &str = "#ff453a";
/// Black at 72 %, behind text over the picture.
const CHIP: &str = "#000000b8";
/// Under the bar and its playhead, so they hold on bright footage.
const SHADE: &str = "#00000073";
const BAR: &str = "#ffffff73";
/// Fill of a struck caption word and of words not said yet; over the black outline they show as #595959 and #bfbfbf.
const STRUCK: &str = "#ffffff59";
const UPCOMING: &str = "#ffffffbf";

/// Where the baseline sits below the top of a line of Inter, as a share of the font size.
const BASELINE: f32 = 0.963;
/// A caption page holds at most this many characters, about two lines, and ends at a pause this long.
const PAGE_CHARS: usize = 34;
const SENTENCE_GAP_US: i64 = 600_000;
/// A page stays at least this long in the recap, so it can be read even when the recording races.
const PAGE_MIN_US: i64 = 500_000;
/// Above this speed the recording shows ten pictures a second, a time-lapse, instead of thirty.
const SAMPLED_SPEED: f64 = 8.0;
/// The finished cut is labelled this long.
const AFTER_LABEL_US: i64 = 4_000_000;
/// Ticks closer than this to the one before are left out.
const TICK_GAP_US: i64 = 120_000;

fn encode(
    recap: &Recap,
    cache_dir: &Path,
    out: &Path,
    end_card: bool,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(f32),
) -> Result<()> {
    let sections = sections(recap, end_card);
    let total: i64 = sections.iter().sum();
    let frames = frame_count(total, FPS);
    let vcodec = ff::encoder::find_by_name("libx264")
        .or_else(|| ff::encoder::find(ff::codec::Id::H264))
        .ok_or_else(|| anyhow!("No H.264 encoder available in this FFmpeg build"))?;
    let mut mp4 =
        Mp4Writer::create(out, vcodec, WIDTH, HEIGHT, FPS, Quality::Recommended.crf(), &[("preset", "veryfast")])?;
    let mut painter = Painter::new(recap, cache_dir, sections)?;
    let mut sound = Sound::new(recap, cache_dir, sections);
    for i in 0..frames {
        check_cancel(cancel)?;
        let pixels = painter.frame(frame_time_us(i, FPS))?;
        mp4.frame(i, &pixels)?;
        let until = us_to_samples(frame_time_us(i + 1, FPS)).min(us_to_samples(total));
        mp4.sound_until(until, |pos, mix| sound.fill(pos, mix))?;
        progress((i + 1) as f32 / frames.max(1) as f32);
    }
    mp4.finish()
}

/// The recording as the recap plays it fast: the main track as it was, without the person cut out, the
/// overlays or the text, which the recap draws itself.
fn recording(before: &Project) -> Project {
    let mut recording = before.clone();
    recording.tracks.retain(|track| track.id == MAIN_TRACK);
    for clip in recording.tracks.iter_mut().flat_map(|track| &mut track.clips) {
        if let ClipContent::Media { background, .. } = &mut clip.content {
            *background = Background::None;
        }
    }
    recording
}

/// The size a canvas is drawn at, whole inside the recap.
fn fit(canvas: &Canvas) -> (u32, u32) {
    let scale = (WIDTH as f32 / canvas.width.max(1) as f32).min(HEIGHT as f32 / canvas.height.max(1) as f32);
    let even = |v: u32| ((v as f32 * scale / 2.0).round().max(1.0) * 2.0) as u32;
    (even(canvas.width), even(canvas.height))
}

fn speed_text(speed: f64) -> String {
    if speed >= 9.95 { format!("{speed:.0}×") } else { format!("{speed:.1}×").replace(".0×", "×") }
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// Words of the transcript in pages of a caption each, as ranges of word indices.
fn pages(words: &[RecapWord]) -> Vec<Range<usize>> {
    let mut pages = Vec::new();
    let (mut start, mut chars) = (0, 0);
    for (i, word) in words.iter().enumerate() {
        let len = word.text.chars().count();
        if i > start && (chars + 1 + len > PAGE_CHARS || word.start_us - words[i - 1].end_us >= SENTENCE_GAP_US) {
            pages.push(start..i);
            (start, chars) = (i, 0);
        }
        chars += if chars == 0 { len } else { len + 1 };
    }
    if start < words.len() {
        pages.push(start..words.len());
    }
    pages
}

fn style(size: f32, color: &str, bold: bool, stroke: f32) -> TextStyle {
    TextStyle {
        font_family: None,
        font_size: size,
        color: color.into(),
        bold,
        stroke_width: stroke,
        stroke_color: "#000000".into(),
        background: None,
        max_width: None,
        highlight: None,
        keywords: None,
        shadow: None,
        letter_case: Default::default(),
        letter_spacing: 0.0,
        line_spacing: 0.0,
    }
}

fn layer(image: Image, [x, y, w, h]: [f32; 4]) -> Layer {
    Layer {
        image,
        corners: [[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
        uv_rotation: 0,
        mirror: false,
        opacity: 1.0,
        adjust: Adjust::default(),
        blur: 0.0,
        clip: None,
        transfer: Transfer::Sdr,
        mask: None,
        matte: None,
    }
}

/// A box of `color`, its corners rounded by `radius` pixels.
fn rect(x: f32, y: f32, w: f32, h: f32, color: &str, radius: f32) -> Draw {
    let pixel = parse_color(color).map(|c| (c * 255.0).round() as u8).to_vec();
    let mut rect = layer(Image { width: 1, height: 1, data: Arc::new(pixel) }, [x, y, w, h]);
    if radius > 0.0 {
        rect.mask = Some(Mask {
            rect: WHOLE,
            radius: (2.0 * radius / w.min(h)).min(1.0),
            border: 0.0,
            border_color: [0.0; 4],
            shadow: 0.0,
            shadow_blur: 0.0,
        });
    }
    Draw::Layer(rect)
}

const CHIP_HEIGHT: f32 = 64.0;

/// `text` in a chip of `fill` with its top left corner at `x`, `y`: the counter's speed, the finished cut's label and
/// a cut's label share its size.
fn chip(draws: &mut Vec<Draw>, text: &Text, x: f32, y: f32, fill: &str) {
    draws.push(rect(x, y, text.ink() + 44.0, CHIP_HEIGHT, fill, 14.0));
    draws.push(text.at(x + 22.0, y + 46.0));
}

/// Words of these widths in lines at most `max` wide, as ranges. Two lines break where they come out most even, so
/// no word is left alone on the second.
fn lines(widths: &[f32], gap: f32, max: f32) -> Vec<Range<usize>> {
    let span = |r: Range<usize>| widths[r.clone()].iter().sum::<f32>() + gap * r.len().saturating_sub(1) as f32;
    let n = widths.len();
    if span(0..n) <= max {
        return std::iter::once(0..n).collect();
    }
    let longer = |k: usize| span(0..k).max(span(k..n));
    if let Some(k) = (1..n).filter(|&k| longer(k) <= max).min_by(|&a, &b| longer(a).total_cmp(&longer(b))) {
        return vec![0..k, k..n];
    }
    let mut lines: Vec<Range<usize>> = Vec::new();
    for i in 0..n {
        match lines.last_mut() {
            Some(line) if span(line.start..i + 1) <= max => line.end = i + 1,
            _ => lines.push(i..i + 1),
        }
    }
    lines
}

/// A rendered piece of text: its image, the size it is drawn at, and the padding around its glyphs.
struct Text {
    image: Image,
    w: f32,
    h: f32,
    pad: f32,
    size: f32,
}

impl Text {
    /// Width of the glyphs alone.
    fn ink(&self) -> f32 {
        self.w - 2.0 * self.pad
    }

    /// Drawn with the start of its glyphs at `x` and its baseline at `baseline`.
    fn at(&self, x: f32, baseline: f32) -> Draw {
        let top = baseline - BASELINE * self.size - self.pad;
        Draw::Layer(layer(self.image.clone(), [x - self.pad, top, self.w, self.h]))
    }
}

struct Painter<'a> {
    recap: &'a Recap,
    renderer: Renderer,
    recording: Project,
    sections: [i64; 5],
    totals: RecapTotals,
    pages: Vec<Range<usize>>,
    /// The caption page on screen and when it came up.
    shown: Option<(usize, i64)>,
    /// The last picture drawn: of the finished cut or not, at which time, and its pixels.
    last: Option<(bool, i64, Image)>,
}

impl<'a> Painter<'a> {
    fn new(recap: &'a Recap, cache_dir: &Path, sections: [i64; 5]) -> Result<Self> {
        let mut renderer = Renderer::new()?;
        renderer.use_mattes(cache_dir.to_path_buf(), Pending::Fail);
        Ok(Self {
            recap,
            renderer,
            recording: recording(&recap.before),
            sections,
            totals: recap.totals(),
            pages: pages(&recap.words),
            shown: None,
            last: None,
        })
    }

    fn text(&mut self, text: &str, style: &TextStyle) -> Text {
        let image = self.renderer.text.render(text, style, 1.0, 4.0 * WIDTH as f32);
        let pad = style.stroke_width.ceil() + 2.0;
        let (w, h) = (image.image.width as f32 / image.scale, image.image.height as f32 / image.scale);
        Text { image: image.image, w, h, pad, size: style.font_size }
    }

    /// Pieces of text in a row on one baseline from `x`, `gap` pixels apart.
    fn row(&mut self, draws: &mut Vec<Draw>, x: f32, baseline: f32, pieces: &[(String, TextStyle)], gap: f32) {
        let mut at = x;
        for (i, (text, style)) in pieces.iter().enumerate() {
            let piece = self.text(text, style);
            if i > 0 {
                at += gap;
            }
            draws.push(piece.at(at, baseline));
            at += piece.ink();
        }
    }

    fn row_width(&mut self, pieces: &[(String, TextStyle)], gap: f32) -> f32 {
        let inks: Vec<f32> = pieces.iter().map(|(text, style)| self.text(text, style).ink()).collect();
        inks.iter().sum::<f32>() + gap * inks.len().saturating_sub(1) as f32
    }

    /// The numbers the recap counts, as far as `at` of the recording, as pieces of a row.
    fn stats(&self, at: i64, size: f32, numbers: &str, labels: &str) -> Vec<(String, TextStyle)> {
        let RecapTotals { words, retakes, pauses, .. } = self.totals;
        let passed = |cut: &&RecapCut| cut.start_us <= at;
        let counts = [
            (words, self.recap.cuts.iter().filter(passed).map(|c| c.words).sum(), "word", "words"),
            (retakes, self.recap.cuts.iter().filter(passed).map(|c| c.retakes).sum(), "retake", "retakes"),
            (pauses, self.recap.cuts.iter().filter(passed).filter(|c| c.words == 0).count(), "pause", "pauses"),
        ];
        let mut pieces = Vec::new();
        for (total, now, one, many) in counts {
            if total.unwrap_or(0) == 0 {
                continue;
            }
            if !pieces.is_empty() {
                pieces.push(("·".to_owned(), style(size, labels, false, 0.0)));
            }
            pieces.push((now.to_string(), style(size, numbers, true, 0.0)));
            pieces.push((if now == 1 { one } else { many }.to_owned(), style(size, labels, false, 0.0)));
        }
        pieces
    }

    /// The recording or the finished cut at `time`, fitted into the recap.
    fn picture(&mut self, after: bool, time: i64, adjust: Adjust) -> Result<Draw> {
        let project = if after { &self.recap.after } else { &self.recording };
        let (w, h) = fit(&project.canvas);
        let image = match &self.last {
            Some((was_after, at, image)) if *was_after == after && *at == time => image.clone(),
            _ => {
                let rgba = self.renderer.render(project, time, w, h, Wait::Exact, after)?;
                let image = Image { width: w, height: h, data: Arc::new(rgba) };
                self.last = Some((after, time, image.clone()));
                image
            }
        };
        let x = (WIDTH - w) as f32 / 2.0;
        let y = (HEIGHT - h) as f32 / 2.0;
        let mut picture = layer(image, [x, y, w as f32, h as f32]);
        picture.adjust = adjust;
        Ok(Draw::Layer(picture))
    }

    fn frame(&mut self, t: i64) -> Result<Vec<u8>> {
        let [hook, fast, hold, sample, _] = self.sections;
        let before = self.recap.before.duration_us();
        let mut draws = Vec::new();
        if t < hook {
            draws.push(self.picture(false, 0, Adjust { exposure: -1.5, ..Adjust::default() })?);
            self.opening(&mut draws);
        } else if t < hook + fast + hold {
            let speed = before as f64 / fast as f64;
            let mut into = (t - hook).min(fast);
            if speed > SAMPLED_SPEED {
                into -= into % 100_000;
            }
            let at = ((into as f64 * speed) as i64).min(before - 1);
            let cut = self.recap.cuts.iter().find(|c| c.start_us <= at && at < c.end_us).cloned();
            let adjust = match cut {
                Some(_) => Adjust { saturation: -1.0, exposure: -1.25, ..Adjust::default() },
                None => Adjust::default(),
            };
            draws.push(self.picture(false, at, adjust)?);
            if cut.is_some() {
                let edge = 11.0;
                let (w, h) = (WIDTH as f32, HEIGHT as f32);
                draws.extend([
                    rect(0.0, 0.0, w, edge, RED, 0.0),
                    rect(0.0, h - edge, w, edge, RED, 0.0),
                    rect(0.0, 0.0, edge, h, RED, 0.0),
                    rect(w - edge, 0.0, edge, h, RED, 0.0),
                ]);
            }
            let left = before - self.recap.cuts.iter().map(|c| (at.min(c.end_us) - c.start_us).max(0)).sum::<i64>();
            self.counter(&mut draws, at, left, speed);
            self.bar(&mut draws, 488.0, Some(at));
            let lines = self.caption(&mut draws, at, t);
            if let Some(cut) = cut {
                self.label(&mut draws, &cut, lines);
            }
        } else if t < hook + fast + hold + sample {
            let into = t - (hook + fast + hold);
            draws.push(self.picture(true, into, Adjust::default())?);
            if into < AFTER_LABEL_US {
                let label =
                    self.text(&format!("After · {}", duration_text(self.totals.after_us)), &style(40.0, FG, true, 0.0));
                chip(&mut draws, &label, LEFT, TOP, CHIP);
            }
        } else {
            let last = (sample - frame_time_us(1, FPS)).max(0);
            draws.push(self.picture(true, last, Adjust { exposure: -2.0, ..Adjust::default() })?);
            let centre = (LEFT + RIGHT) / 2.0;
            for (text, style, baseline) in [
                ("Cut in Nuzky", style(54.0, FG, true, 0.0), 840.0),
                ("nuzky.app", style(41.0, "#c7c7cc", false, 0.0), 905.0),
            ] {
                let text = self.text(text, &style);
                draws.push(text.at(centre - text.ink() / 2.0, baseline));
            }
        }
        self.renderer.gpu.render(WIDTH, HEIGHT, [0.0, 0.0, 0.0, 1.0], &draws)
    }

    /// The opening: the length before struck through, the length after, what went and the whole cut on the bar.
    fn opening(&mut self, draws: &mut Vec<Draw>) {
        let centre = (LEFT + RIGHT) / 2.0;
        let was = self.text(&duration_text(self.totals.before_us), &style(90.0, "#c9c9cf", true, 0.0));
        let x = centre - was.ink() / 2.0;
        draws.push(was.at(x, 600.0));
        draws.push(rect(x - 6.0, 600.0 - 0.3 * 90.0 - 4.0, was.ink() + 12.0, 9.0, RED, 4.5));
        let now = self.text(&duration_text(self.totals.after_us), &style(260.0, "#ffffff", true, 9.0));
        draws.push(now.at(centre - now.ink() / 2.0, 870.0));
        let pieces = self.stats(i64::MAX, 45.0, "#ffffff", "#ffffff");
        let width = self.row_width(&pieces, 14.0);
        self.row(draws, centre - width / 2.0, 965.0, &pieces, 14.0);
        self.bar(draws, 1037.0, None);
    }

    /// The length left at `at` of the recording, over what it started at, and what has gone so far.
    fn counter(&mut self, draws: &mut Vec<Draw>, at: i64, left: i64, speed: f64) {
        let length = vec![
            (duration_text(left), style(90.0, FG, true, 0.0)),
            (format!("from {}", duration_text(self.totals.before_us)), style(40.0, LABEL, false, 0.0)),
        ];
        let stats = self.stats(at, 40.0, FG, LABEL);
        let width = self.row_width(&length, 16.0).max(self.row_width(&stats, 12.0));
        let height = if stats.is_empty() { 126.0 } else { 184.0 };
        draws.push(rect(LEFT, TOP, width + 60.0, height, CHIP, 26.0));
        self.row(draws, LEFT + 30.0, TOP + 100.0, &length, 16.0);
        self.row(draws, LEFT + 30.0, TOP + 158.0, &stats, 12.0);
        let speed = self.text(&speed_text(speed), &style(40.0, FG, true, 0.0));
        chip(draws, &speed, RIGHT - speed.ink() - 44.0, TOP, CHIP);
    }

    /// The whole recording as a bar, what was cut on it in red, and with `at` its playhead.
    fn bar(&self, draws: &mut Vec<Draw>, y: f32, at: Option<i64>) {
        let before = self.recap.before.duration_us().max(1) as f32;
        let (w, h) = (RIGHT - LEFT, 18.0);
        draws.push(rect(LEFT - 3.0, y - 3.0, w + 6.0, h + 6.0, SHADE, 12.0));
        draws.push(rect(LEFT, y, w, h, BAR, h / 2.0));
        for cut in &self.recap.cuts {
            let x = LEFT + w * cut.start_us as f32 / before;
            let width = (w * (cut.end_us - cut.start_us) as f32 / before).max(3.0);
            draws.push(rect(x, y, width.min(RIGHT - x), h, RED, 0.0));
        }
        if let Some(at) = at {
            let x = LEFT + w * at as f32 / before;
            draws.push(rect(x - 5.0, y - 12.0, 10.0, h + 24.0, SHADE, 5.0));
            draws.push(rect(x - 3.0, y - 10.0, 6.0, h + 20.0, FG, 3.0));
        }
    }

    /// The caption page around `at` of the recording, its cut words struck once passed. Returns its lines.
    fn caption(&mut self, draws: &mut Vec<Draw>, at: i64, t: i64) -> usize {
        let words = &self.recap.words;
        let Some(here) = self.pages.partition_point(|p| words[p.start].start_us <= at).checked_sub(1) else {
            return 0;
        };
        let page = match self.shown {
            Some((shown, since)) if shown >= here || t - since < PAGE_MIN_US => shown,
            shown => {
                // Of the pages passed while the last one stayed, the latest with a cut word, so a cut is seen.
                let from = shown.map_or(0, |(page, _)| page + 1).min(here);
                let next =
                    (from..=here).rev().find(|&p| self.pages[p].clone().any(|i| words[i].removed)).unwrap_or(here);
                self.shown = Some((next, t));
                next
            }
        };
        let size = 71.0;
        let (gap, line_height) = (0.26 * size, 1.2 * size);
        let mut items = Vec::new();
        for i in self.pages[page].clone() {
            let word = &self.recap.words[i];
            let said = word.start_us <= at;
            let fill = match (said, word.removed) {
                (true, true) => STRUCK,
                (true, false) => "#ffffff",
                (false, _) => UPCOMING,
            };
            items.push((self.text(&word.text, &style(size, fill, true, 6.0)), said && word.removed));
        }
        let lines = lines(&items.iter().map(|(t, _)| t.ink()).collect::<Vec<_>>(), gap, RIGHT - LEFT);
        for (n, line) in lines.iter().enumerate() {
            let line = &items[line.clone()];
            let width: f32 = line.iter().map(|(t, _)| t.ink()).sum::<f32>() + gap * (line.len() - 1) as f32;
            let baseline = CAPTION_BOTTOM - 0.25 * size - (lines.len() - 1 - n) as f32 * line_height;
            let mut x = (LEFT + RIGHT) / 2.0 - width / 2.0;
            for (text, struck) in line {
                draws.push(text.at(x, baseline));
                if *struck {
                    draws.push(rect(x - 4.0, baseline - 0.3 * size - 4.0, text.ink() + 8.0, 8.0, RED, 4.0));
                }
                x += text.ink() + gap;
            }
        }
        lines.len()
    }

    /// What the cut on screen removed, above the caption.
    fn label(&mut self, draws: &mut Vec<Draw>, cut: &RecapCut, lines: usize) {
        let length = (cut.end_us - cut.start_us) as f64 / 1e6;
        let text = match (cut.retakes, cut.words, self.recap.transcribed) {
            (1.., _, _) => "Retake".to_owned(),
            (0, 1.., _) => count(cut.words, "word", "words"),
            (_, _, true) => format!("Pause {length:.1} s"),
            _ => format!("Cut {length:.1} s"),
        };
        let text = self.text(&text, &style(40.0, "#000000", true, 0.0));
        let bottom = CAPTION_BOTTOM - 0.25 * 71.0 - 71.0 - lines.saturating_sub(1) as f32 * 1.2 * 71.0 - 36.0;
        chip(draws, &text, (LEFT + RIGHT) / 2.0 - (text.ink() + 44.0) / 2.0, bottom - CHIP_HEIGHT, RED);
    }
}

/// The recap's sound: a whoosh into each part and a tick as each cut passes, then the finished cut as it plays.
struct Sound<'a> {
    after: &'a Project,
    cues: Project,
    cue_mixer: Mixer,
    after_mixer: Mixer,
    /// Samples of the recap the finished cut plays in.
    sample: Range<i64>,
    buffer: Vec<f32>,
}

impl<'a> Sound<'a> {
    fn new(recap: &'a Recap, cache_dir: &Path, [hook, fast, hold, sample, _]: [i64; 5]) -> Self {
        let before = recap.before.duration_us().max(1);
        let start = hook + fast + hold;
        let mut cues = Project::new("Recap sounds");
        cues.tracks.clear();
        let clip = |asset: &Asset, at: i64, volume: f32| {
            let content = ClipContent::Media {
                asset_id: asset.id.clone(),
                source_in_us: 0,
                volume,
                transform: Transform::default(),
                speed: 1.0,
                keep_pitch: false,
                adjust: Adjust::default(),
                fade_in_us: 0,
                fade_out_us: 0,
                clean_voice: false,
                dereverb: 0.0,
                normalize: false,
                leveler: 0.0,
                shape: None,
                duck_db: 0.0,
                background: Background::None,
            };
            Clip::new(format!("{}-{at}", asset.id), at.max(0), asset.duration_us, content)
        };
        let track = |id: &str, clips: Vec<Clip>| Track {
            id: id.into(),
            kind: TrackKind::Audio,
            name: id.into(),
            muted: false,
            hidden: false,
            keep_in_place: true,
            clips,
        };
        if let Some(tick) = &recap.tick {
            let mut clips: Vec<Clip> = Vec::new();
            for cut in &recap.cuts {
                let at = hook + (cut.start_us as f64 / before as f64 * fast as f64) as i64;
                if clips.last().is_none_or(|last| at - last.start_us >= TICK_GAP_US.max(tick.duration_us)) {
                    clips.push(clip(tick, at, 0.5));
                }
            }
            cues.assets.push(tick.clone());
            cues.tracks.push(track("ticks", clips));
        }
        if let Some(whoosh) = &recap.whoosh {
            let lead = whoosh.duration_us / 2;
            let clips = vec![clip(whoosh, hook - lead, 0.6), clip(whoosh, start - lead, 0.6)];
            cues.assets.push(whoosh.clone());
            cues.tracks.push(track("whooshes", clips));
        }
        Self {
            after: &recap.after,
            cues,
            cue_mixer: Mixer::new(cache_dir.to_path_buf()),
            after_mixer: Mixer::new(cache_dir.to_path_buf()),
            sample: us_to_samples(start)..us_to_samples(start + sample),
            buffer: Vec::new(),
        }
    }

    fn fill(&mut self, pos: i64, out: &mut [f32]) {
        self.cue_mixer.mix(&self.cues, pos, out);
        let frames = (out.len() / CHANNELS) as i64;
        let Range { start, end } = self.sample;
        if pos + frames <= start || pos >= end {
            return;
        }
        self.buffer.resize(out.len(), 0.0);
        self.after_mixer.mix(self.after, pos - start, &mut self.buffer);
        let (fade_in, fade_out) = (us_to_samples(100_000) as f32, us_to_samples(300_000) as f32);
        for (n, (out, sound)) in out.chunks_mut(CHANNELS).zip(self.buffer.chunks(CHANNELS)).enumerate() {
            let at = pos + n as i64;
            if !(start..end).contains(&at) {
                continue;
            }
            let gain = ((at - start) as f32 / fade_in).min((end - at) as f32 / fade_out).min(1.0);
            for (out, sound) in out.iter_mut().zip(sound) {
                *out += sound * gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recap(before_s: i64, after_s: i64) -> Recap {
        let project = |secs: i64| {
            let mut project = Project::new("p");
            let content = ClipContent::Text {
                text: "x".into(),
                style: style(10.0, FG, false, 0.0),
                transform: Transform::default(),
                words: Vec::new(),
            };
            project.tracks[0].clips.push(Clip::new("c".into(), 0, secs * 1_000_000, content));
            project
        };
        Recap {
            before: project(before_s),
            after: project(after_s),
            cuts: Vec::new(),
            words: Vec::new(),
            transcribed: false,
            tick: None,
            whoosh: None,
        }
    }

    #[test]
    fn a_recap_fits_in_thirty_seconds_and_never_plays_slower_than_recorded() {
        for (before, after) in [(2, 1), (10, 6), (222, 68), (1200, 300), (36_000, 20_000)] {
            let recap = recap(before, after);
            let [_, fast, _, sample, _] = sections(&recap, true);
            assert!(duration_us(&recap, true) <= 30_000_000, "{before} s");
            assert!(fast <= before * 1_000_000 && sample <= after * 1_000_000, "{before} s");
            assert_eq!(duration_us(&recap, true) - duration_us(&recap, false), END_CARD_US);
        }
        // A short recording plays at a few times its speed, not a frame a second.
        let [_, fast, ..] = sections(&recap(10, 6), false);
        assert_eq!(fast, FAST_MIN_US);
    }

    #[test]
    fn lengths_read_as_the_summary_writes_them() {
        assert_eq!(duration_text(20_000_000), "20.0 s");
        assert_eq!(duration_text(4_450_000), "4.5 s");
        assert_eq!(duration_text(59_940_000), "59.9 s");
        assert_eq!(duration_text(59_960_000), "1:00");
        assert_eq!(duration_text(222_000_000), "3:42");
        assert_eq!(duration_text(68_040_000), "1:08");
        assert_eq!(duration_text(3_725_000_000), "62:05");
    }

    #[test]
    fn caption_pages_end_at_a_pause_and_stay_short() {
        let word = |text: &str, start_ms: i64| RecapWord {
            start_us: start_ms * 1000,
            end_us: start_ms * 1000 + 200_000,
            text: text.into(),
            removed: false,
        };
        let words = [
            word("Dneska", 0),
            word("vám", 300),
            word("ukážu,", 600),
            word("jak", 900),
            word("natočit", 1200),
            word("video", 1500),
            word("na", 1800),
            word("mobil.", 2100),
            word("Potřebujete", 3500),
            word("telefon.", 3800),
        ];
        let pages = pages(&words);
        assert_eq!(pages, [0..5, 5..8, 8..10]);
        for page in pages {
            let text: Vec<&str> = words[page].iter().map(|w| w.text.as_str()).collect();
            assert!(text.join(" ").chars().count() <= PAGE_CHARS, "{text:?}");
        }
    }

    #[test]
    fn two_caption_lines_break_where_they_are_most_even() {
        // Greedy wrapping would leave the last word alone: 300 + 300 + 300 fit, 100 does not.
        assert_eq!(lines(&[300.0, 300.0, 300.0, 100.0], 0.0, 900.0), [0..2, 2..4]);
        assert_eq!(lines(&[200.0, 200.0], 10.0, 900.0).len(), 1);
        // A page too long for two lines wraps as it fits.
        assert_eq!(lines(&[800.0, 800.0, 800.0], 10.0, 900.0), [0..1, 1..2, 2..3]);
    }

    #[test]
    fn totals_count_words_retakes_and_pauses_only_with_a_transcript() {
        let mut recap = recap(20, 10);
        recap.cuts = vec![
            RecapCut { start_us: 0, end_us: 1_000_000, words: 0, retakes: 0 },
            RecapCut { start_us: 2_000_000, end_us: 5_000_000, words: 7, retakes: 1 },
            RecapCut { start_us: 6_000_000, end_us: 7_000_000, words: 2, retakes: 0 },
        ];
        let unknown = recap.totals();
        assert_eq!((unknown.words, unknown.retakes, unknown.pauses), (None, None, None));
        recap.transcribed = true;
        let known = recap.totals();
        assert_eq!((known.words, known.retakes, known.pauses), (Some(9), Some(1), Some(1)));
        assert_eq!((known.before_us, known.after_us), (20_000_000, 10_000_000));
    }
}

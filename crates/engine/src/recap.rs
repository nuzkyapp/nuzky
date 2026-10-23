//! A recap: a short vertical video of how a video was cut. The recording races over a bar of its whole
//! length; each cut slows down on screen, strikes its words and then snaps shut on the bar with a quick
//! swish, so the bar and the number at its end shrink to the new length, which lands big before a
//! sample of the finished cut closes it. Its pictures come from the renderer of preview
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
    LIMITER_CEILING_DB, Mp4Writer, Quality, check_cancel, check_source_path, frame_count, frame_time_us, prepare_sound,
    write_beside, write_credits,
};
use crate::gpu::{Draw, Image, Layer, Mask};
use crate::loudness::{Limiter, db_to_gain};
use crate::media::{Transfer, init};
use crate::model::{
    Adjust, Asset, BOX_RADIUS, Background, CHANNELS, Canvas, Clip, ClipContent, Project, TextAlign, TextStyle, Track,
    TrackKind, Transform, parse_color,
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
    /// Built-in sounds the recap plays: a short swish as each cut snaps shut, a whoosh between its parts and a soft
    /// hit as the new length lands.
    pub swish: Option<Asset>,
    pub whoosh: Option<Asset>,
    pub land: Option<Asset>,
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

const HOOK_US: i64 = 1_000_000;
/// The recording plays at least this fast and takes at most this long.
const FAST_MIN_US: i64 = 2_500_000;
const FAST_MAX_US: i64 = 5_000_000;
const LANDING_US: i64 = 1_600_000;
const SAMPLE_MAX_US: i64 = 4_000_000;
const END_CARD_US: i64 = 1_500_000;
/// A cut stays on screen this long when there is room, so it is seen while the recording races past it.
const CUT_SCREEN_US: i64 = 400_000;
/// Cuts at most this many pixels of the bar apart go as one piece, which is drawn at least this wide.
const MERGE_PX: f32 = 10.0;
const PIECE_MIN_PX: f32 = 6.0;

/// One cut as the recap shows it, or several so close on the bar that they go as one.
#[derive(Clone, Debug, PartialEq)]
struct Piece {
    start_us: i64,
    end_us: i64,
    /// What its cuts removed; less than its span when it holds several, with kept footage between them.
    removed_us: i64,
    words: usize,
    retakes: usize,
    cuts: usize,
}

impl Piece {
    fn len(&self) -> i64 {
        self.end_us - self.start_us
    }
}

/// How the recording races: each piece slows down to stay on screen and the kept footage takes the rest.
struct Race {
    pieces: Vec<Piece>,
    /// When each piece is on screen, from the start of the race.
    shown: Vec<Range<i64>>,
    /// Kept recording over the race time it takes.
    kept_us: i64,
    kept_screen_us: i64,
    length: i64,
}

impl Race {
    fn new(recap: &Recap) -> Self {
        let before = recap.before.duration_us().max(1);
        let merge = (MERGE_PX as f64 / BAR_W as f64 * before as f64) as i64;
        let mut pieces: Vec<Piece> = Vec::new();
        for cut in &recap.cuts {
            match pieces.last_mut() {
                Some(piece) if cut.start_us - piece.end_us <= merge => {
                    piece.end_us = cut.end_us;
                    piece.removed_us += cut.end_us - cut.start_us;
                    piece.words += cut.words;
                    piece.retakes += cut.retakes;
                    piece.cuts += 1;
                }
                _ => pieces.push(Piece {
                    start_us: cut.start_us,
                    end_us: cut.end_us,
                    removed_us: cut.end_us - cut.start_us,
                    words: cut.words,
                    retakes: cut.retakes,
                    cuts: 1,
                }),
            }
        }
        let fast = (before / 6).clamp(FAST_MIN_US, FAST_MAX_US).min(before);
        // The pieces take at most half the race, so with many cuts the kept recording still reads.
        let budget = fast / 2 / (pieces.len() as i64).max(1);
        let screens: Vec<i64> = pieces.iter().map(|p| p.len().min(CUT_SCREEN_US).min(budget)).collect();
        let kept_us = (before - pieces.iter().map(Piece::len).sum::<i64>()).max(1);
        // Never slower than it was recorded.
        let kept_screen_us = (fast - screens.iter().sum::<i64>()).min(kept_us).max(1);
        let mut shown = Vec::with_capacity(pieces.len());
        let (mut t, mut at) = (0, 0);
        for (piece, screen) in pieces.iter().zip(&screens) {
            t += scale(piece.start_us - at, kept_screen_us, kept_us);
            shown.push(t..t + screen);
            t += screen;
            at = piece.end_us;
        }
        // The last piece snaps shut and the picture springs back before the race ends.
        let length =
            (t + scale(before - at, kept_screen_us, kept_us)).max(shown.last().map_or(0, |s| s.end + RETURN_US));
        Self { pieces, shown, kept_us, kept_screen_us, length }
    }

    /// The recording's time at `t` of the race, its speed there and the piece on screen.
    fn at(&self, t: i64) -> (i64, f64, Option<usize>) {
        let (mut from_t, mut from_at) = (0, 0);
        for (i, (piece, shown)) in self.pieces.iter().zip(&self.shown).enumerate() {
            if t < shown.start {
                break;
            }
            if t < shown.end {
                let screen = (shown.end - shown.start).max(1);
                let at = piece.start_us + scale(t - shown.start, piece.len(), screen);
                return (at, piece.len() as f64 / screen as f64, Some(i));
            }
            (from_t, from_at) = (shown.end, piece.end_us);
        }
        let speed = self.kept_us as f64 / self.kept_screen_us as f64;
        (from_at + scale(t - from_t, self.kept_us, self.kept_screen_us), speed, None)
    }

    /// How far each piece has snapped shut at `t` of the race, 0 to 1.
    fn snapped(&self, t: i64) -> Vec<f32> {
        self.shown.iter().map(|shown| ease_in_out(progress(t - shown.end, SNAP_US))).collect()
    }

    /// How far the picture has stepped back at `t`: in while a piece plays, back out after it, or staying back
    /// when the next piece comes soon, so many cuts close together do not flicker.
    fn recede(&self, t: i64) -> f32 {
        self.shown
            .iter()
            .enumerate()
            .map(|(i, shown)| {
                let next = self.shown.get(i + 1);
                if shown.contains(&t) {
                    ease_out(progress(t - shown.start, RECEDE_US))
                } else if let Some(next) = next.filter(|n| n.start - shown.end < STAY_BACK_US && t < n.end) {
                    if t >= shown.end && t < next.end { 1.0 } else { 0.0 }
                } else if t >= shown.end {
                    1.0 - ease_out(progress(t - shown.end, RETURN_US))
                } else {
                    0.0
                }
            })
            .fold(0.0, f32::max)
    }
}

fn scale(us: i64, to: i64, from: i64) -> i64 {
    (us as i128 * to as i128 / from.max(1) as i128) as i64
}

fn progress(us: i64, over: i64) -> f32 {
    (us as f32 / over as f32).clamp(0.0, 1.0)
}

fn ease_out(x: f32) -> f32 {
    1.0 - (1.0 - x).powi(3)
}

fn ease_in_out(x: f32) -> f32 {
    if x < 0.5 { 4.0 * x * x * x } else { 1.0 - (-2.0 * x + 2.0).powi(3) / 2.0 }
}

fn lerp(a: f32, b: f32, x: f32) -> f32 {
    a + (b - a) * x
}

/// The parts of a recap, as lengths: the opening, the recording racing, the landing of the new length,
/// the finished cut and the end card (0 when off).
pub fn sections(recap: &Recap, end_card: bool) -> [i64; 5] {
    let sample = recap.after.duration_us().min(SAMPLE_MAX_US);
    [HOOK_US, Race::new(recap).length, LANDING_US, sample, if end_card { END_CARD_US } else { 0 }]
}

pub fn duration_us(recap: &Recap, end_card: bool) -> i64 {
    sections(recap, end_card).iter().sum()
}

/// When each piece of the bar shows in the recap; a piece holds one cut or several close together.
pub fn cut_times(recap: &Recap) -> Vec<Range<i64>> {
    Race::new(recap).shown.into_iter().map(|r| r.start + HOOK_US..r.end + HOOK_US).collect()
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
    // The finished cut plays its CC BY sounds and footage, so they are credited beside the recap as beside an export.
    let credits = crate::credits::credits(&recap.after);
    let credits_out = crate::credits::credits_path(out);
    if credits.is_some() && credits_out.symlink_metadata().is_ok() {
        bail!("OUTPUT_EXISTS: {} already exists; choose a new file name", credits_out.display());
    }
    prepare_sound(&recap.after, cache_dir, cancel)?;
    for asset in recap.swish.iter().chain(&recap.whoosh).chain(&recap.land) {
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
const BAR_W: f32 = RIGHT - LEFT;
/// The bar while the recording races, and the caption's last line above it.
const BAR_Y: f32 = 1240.0;
const BAR_H: f32 = 40.0;
const CAPTION_BOTTOM: f32 = 1060.0;
/// The speed and the length sit on this baseline above the bar's top, clear of its playhead.
const LABEL_ABOVE: f32 = 42.0;
const CAPTION_SIZE: f32 = 62.0;

/// Behind everything, and the dark seam where a cut snapped shut.
const GRAPHITE: &str = "#0c0c0e";
const FG: &str = "#f2f2f5";
const LABEL: &str = "#d1d1d6";
const MUTED: &str = "#a1a1a9";
const RED: &str = "#ff453a";
/// The bar not played yet; the played part is `FG`.
const AHEAD: &str = "#ffffff6b";
/// Behind the playhead, so it holds on bright footage.
const SHADE: &str = "#0000008c";
/// Fill of a struck caption word and of words not said yet; over the black outline they show as #9e9e9e and #bfbfbf.
const STRUCK: &str = "#ffffff9e";
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
/// A cut steps the picture back this fast, it springs back this fast after, and its piece of the bar snaps shut
/// this fast once it has played.
const RECEDE_US: i64 = 220_000;
const RETURN_US: i64 = 320_000;
const SNAP_US: i64 = 280_000;
/// The new length has grown to its size this far into the landing, where its hit sounds.
const LAND_HIT_US: i64 = 860_000;
/// The picture stays back between pieces closer together than this, so it never comes back for less than 200 ms.
const STAY_BACK_US: i64 = RETURN_US + 200_000;
/// What is written over the picture fades this fast where it goes.
const TEXT_OUT_US: i64 = 200_000;
/// The finished cut shows its bar this long, then the bar fades.
const AFTER_BAR_US: i64 = 2_000_000;
const FADE_US: i64 = 400_000;
/// Swishes closer than this to the one before are left out, so none plays over the last.
const SWISH_GAP_US: i64 = 150_000;
/// A swish starts this long after its piece starts to snap shut, so it peaks as the piece closes fastest.
const SWISH_DELAY_US: i64 = 90_000;
/// The music under the racing recording, so the swishes stand out.
const BED_DB: f64 = -9.0;
/// The finished cut's own sound fades out this long before its sample ends, leaving its music.
const HANDOVER_US: i64 = 300_000;
/// A sound on an audio track at least this long is music, even when it does not duck under speech.
const MUSIC_US: i64 = 10_000_000;

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
        align: TextAlign::Center,
        box_radius: BOX_RADIUS,
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
        self.scaled(x, baseline, 1.0)
    }

    /// Drawn `scale` times its size, as the landing grows the new length.
    fn scaled(&self, x: f32, baseline: f32, scale: f32) -> Draw {
        let top = baseline - (BASELINE * self.size + self.pad) * scale;
        Draw::Layer(layer(self.image.clone(), [x - self.pad * scale, top, self.w * scale, self.h * scale]))
    }
}

fn faded(mut draw: Draw, opacity: f32) -> Draw {
    if let Draw::Layer(layer) = &mut draw {
        layer.opacity *= opacity;
    }
    draw
}

/// A band of soft shadow across the whole width from `top` to `bottom`, from nothing at its edges to 70 % in its
/// middle, so what is written over the picture reads on bright footage.
fn scrim(top: f32, bottom: f32, opacity: f32) -> Draw {
    const STEPS: usize = 32;
    let data = (0..STEPS)
        .flat_map(|i| {
            let alpha = (std::f32::consts::PI * i as f32 / (STEPS - 1) as f32).sin().powi(2) * 0.7 * opacity;
            [0, 0, 0, (alpha * 255.0).round() as u8]
        })
        .collect();
    let image = Image { width: 1, height: STEPS as u32, data: Arc::new(data) };
    Draw::Layer(layer(image, [0.0, top, WIDTH as f32, bottom - top]))
}

/// The edge of a box drawn `width` pixels wide around it and nothing inside, as the length a bar had.
fn outline(x: f32, y: f32, w: f32, h: f32, width: f32, color: &str) -> Draw {
    let mut draw = rect(x + width, y + width, w - 2.0 * width, h - 2.0 * width, "#00000000", 0.0);
    if let Draw::Layer(layer) = &mut draw {
        layer.mask = Some(Mask {
            rect: WHOLE,
            radius: 1.0,
            border: width,
            border_color: parse_color(color),
            shadow: 0.0,
            shadow_blur: 0.0,
        });
    }
    draw
}

struct Painter<'a> {
    recap: &'a Recap,
    renderer: Renderer,
    recording: Project,
    sections: [i64; 5],
    totals: RecapTotals,
    race: Race,
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
            race: Race::new(recap),
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

    /// `text` with a red line through it, its glyphs ending at `right`.
    fn struck(&mut self, draws: &mut Vec<Draw>, text: &str, size: f32, right: f32, baseline: f32) {
        let text = self.text(text, &style(size, MUTED, true, 0.0));
        let x = right - text.ink();
        draws.push(text.at(x, baseline));
        draws.push(rect(x - 4.0, baseline - 0.33 * size - 2.0, text.ink() + 8.0, 4.0, RED, 2.0));
    }

    /// What the cuts removed, as pieces of a row: the counts with a transcript, else the number of cuts.
    fn stats(&self, size: f32, numbers: &str, labels: &str) -> Vec<(String, TextStyle)> {
        let RecapTotals { words, retakes, pauses, .. } = self.totals;
        let counts = [(words, "word", "words"), (retakes, "retake", "retakes"), (pauses, "pause", "pauses")];
        let mut pieces = Vec::new();
        for (n, one, many) in counts {
            let Some(n @ 1..) = n else { continue };
            if !pieces.is_empty() {
                pieces.push(("·".to_owned(), style(size, labels, false, 0.0)));
            }
            pieces.push((n.to_string(), style(size, numbers, true, 0.0)));
            pieces.push((if n == 1 { one } else { many }.to_owned(), style(size, labels, false, 0.0)));
        }
        if pieces.is_empty() {
            let cuts = self.recap.cuts.len();
            pieces.push((cuts.to_string(), style(size, numbers, true, 0.0)));
            pieces.push((if cuts == 1 { "cut" } else { "cuts" }.to_owned(), style(size, labels, false, 0.0)));
        }
        pieces
    }

    /// The recording or the finished cut at `time`, fitted into the recap. `back` steps it back into a dim card, as
    /// a cut plays; `zoom` pushes it in.
    fn picture(&mut self, after: bool, time: i64, back: f32, zoom: f32, exposure: f32) -> Result<Draw> {
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
        let scale = zoom * (1.0 - 0.1 * back);
        let (dw, dh) = (w as f32 * scale, h as f32 * scale);
        let mut picture = layer(image, [(WIDTH as f32 - dw) / 2.0, (HEIGHT as f32 - dh) / 2.0, dw, dh]);
        picture.adjust = Adjust { exposure: exposure - 0.8 * back, saturation: -0.4 * back, ..Adjust::default() };
        if back > 0.0 {
            picture.mask = Some(Mask {
                rect: WHOLE,
                radius: (56.0 * back / (dw.min(dh) / 2.0)).min(1.0),
                border: 0.0,
                border_color: [0.0; 4],
                shadow: 0.0,
                shadow_blur: 0.0,
            });
        }
        Ok(Draw::Layer(picture))
    }

    fn frame(&mut self, t: i64) -> Result<Vec<u8>> {
        let [hook, fast, landing, sample, _] = self.sections;
        let mut draws = Vec::new();
        if t < hook {
            self.opening(&mut draws, t)?;
        } else if t < hook + fast {
            self.racing(&mut draws, t, t - hook)?;
        } else if t < hook + fast + landing {
            self.landing(&mut draws, t - hook - fast)?;
        } else if t < hook + fast + landing + sample {
            self.finished(&mut draws, t - hook - fast - landing)?;
        } else {
            self.end_card(&mut draws, t - hook - fast - landing - sample)?;
        }
        self.renderer.gpu.render(WIDTH, HEIGHT, parse_color(GRAPHITE), &draws)
    }

    /// The recording's first picture under the whole bar, drawn left to right, its cuts popping in red.
    fn opening(&mut self, draws: &mut Vec<Draw>, t: i64) -> Result<()> {
        draws.push(self.picture(false, 0, 0.0, 1.0, -0.3)?);
        draws.push(scrim(860.0, 1480.0, 1.0));
        // A third of the bar is there from the first frame, which a platform may show as the video's picture.
        let drawn = lerp(0.35, 1.0, ease_out(progress(t, 450_000)));
        let n = self.race.pieces.len() as i64;
        let stagger = (300_000 / n.max(1)).min(60_000);
        let shown: Vec<f32> = (0..n).map(|i| progress(t - 450_000 - i * stagger, 120_000)).collect();
        let snapped = vec![0.0; self.race.pieces.len()];
        self.bar(draws, BAR_Y, BAR_H, &snapped, None, drawn, &shown);
        let cuts = self.text(&count(self.recap.cuts.len(), "cut", "cuts"), &style(46.0, LABEL, false, 0.0));
        draws.push(cuts.at(LEFT, BAR_Y - LABEL_ABOVE));
        let length = self.text(&duration_text(self.totals.before_us), &style(112.0, FG, true, 0.0));
        draws.push(length.at(RIGHT - length.ink(), BAR_Y - LABEL_ABOVE));
        Ok(())
    }

    /// The recording racing: each cut steps the picture back and strikes its words, and once played its piece of
    /// the bar snaps shut, taking its length off the number at the bar's end.
    fn racing(&mut self, draws: &mut Vec<Draw>, t: i64, into: i64) -> Result<()> {
        let before = self.recap.before.duration_us();
        let (at, speed, piece) = self.race.at(into);
        let at = at.min(before - 1);
        // Fast kept footage shows ten pictures a second, never one from the cut it has just left.
        let mut shot = at;
        if piece.is_none() && speed > SAMPLED_SPEED {
            let kept_from = self.race.shown.iter().zip(&self.race.pieces).rev().find(|(s, _)| s.end <= into);
            shot = self.race.at(into - into % 100_000).0.max(kept_from.map_or(0, |(_, p)| p.end_us)).min(at);
        }
        let back = self.race.recede(into);
        draws.push(self.picture(false, shot, back, 1.0, 0.0)?);
        draws.push(scrim(860.0, 1480.0, 1.0));
        // The caption leaves while a pause plays, so the chip alone says what goes, and with the speed as the race ends.
        let ending = 1.0 - progress(into - (self.race.length - TEXT_OUT_US), TEXT_OUT_US);
        let pause = piece.is_some_and(|i| self.recap.transcribed && self.race.pieces[i].words == 0);
        let mut caption = Vec::new();
        self.caption(&mut caption, at, t);
        let shown = ending * if pause { 1.0 - back } else { 1.0 };
        draws.extend(caption.into_iter().map(|d| faded(d, shown)));
        let snapped = self.race.snapped(into);
        let ones = vec![1.0; snapped.len()];
        let end = self.bar(draws, BAR_Y, BAR_H, &snapped, Some(at), 1.0, &ones);
        // The speed of the kept footage: the cuts' own speeds would flicker past too fast to read.
        let kept = self.race.kept_us as f64 / self.race.kept_screen_us as f64;
        let speed = self.text(&speed_text(kept), &style(46.0, LABEL, false, 0.0));
        draws.push(faded(speed.at(LEFT, BAR_Y - LABEL_ABOVE), ending * (1.0 - back)));
        let gone: f64 = self.race.pieces.iter().zip(&snapped).map(|(p, s)| p.removed_us as f64 * *s as f64).sum();
        let length = self.text(&duration_text(before - gone as i64), &style(112.0, FG, true, 0.0));
        let right = end.max(LEFT + speed.ink() + 40.0 + length.ink());
        draws.push(length.at(right - length.ink(), BAR_Y - LABEL_ABOVE));
        let chip = self.race.shown.iter().rposition(|s| s.start <= into && into < s.end + SNAP_US);
        if let Some(i) = chip {
            let piece = self.race.pieces[i].clone();
            let text = self.text(&self.piece_text(&piece), &style(44.0, "#ffffff", true, 0.0));
            let w = text.ink() + 56.0;
            let x = self.x_at(piece.start_us, &snapped).clamp(LEFT, RIGHT - w);
            let y = BAR_Y + BAR_H + 32.0;
            let opacity = 1.0 - snapped[i];
            draws.push(faded(rect(x, y, w, 68.0, RED, 34.0), opacity));
            draws.push(faded(text.at(x + 28.0, y + 50.0), opacity));
        }
        Ok(())
    }

    /// The new length lands: the bar rises with the outline of the length it had, the number rides on it and then
    /// grows above it, and the old length and what went show under it, until it all fades into the finished cut.
    fn landing(&mut self, draws: &mut Vec<Draw>, u: i64) -> Result<()> {
        let before = self.recap.before.duration_us();
        let dark = ease_out(progress(u, 300_000)) * (1.0 - ease_in_out(progress(u - (LANDING_US - 300_000), 300_000)));
        draws.push(self.picture(false, before - 1, 0.0, 1.0, -1.4 * dark)?);
        let leaving = 1.0 - progress(u - (LANDING_US - TEXT_OUT_US), TEXT_OUT_US);
        let rise = ease_in_out(progress(u, 500_000));
        let (y, h) = (lerp(BAR_Y, 1000.0, rise), lerp(BAR_H, 56.0, rise));
        let mut over =
            vec![scrim(860.0, 1480.0, 1.0 - rise), faded(outline(LEFT, y, BAR_W, h, 3.0, "#ffffff52"), rise)];
        let snapped = self.race.snapped(self.race.length + u);
        let ones = vec![1.0; snapped.len()];
        let end = self.bar(&mut over, y, h, &snapped, Some(before), 1.0, &ones);
        let length = self.text(&duration_text(self.totals.after_us), &style(250.0, FG, true, 0.0));
        // Once the bar has arrived, the length grows from its end to the left edge.
        let grow = ease_out(progress(u - 500_000, 360_000));
        let pop = 1.0 + 0.03 * (std::f32::consts::PI * progress(u - LAND_HIT_US, 240_000)).sin();
        let small = 112.0 / 250.0;
        let x = lerp((end - length.ink() * small).max(LEFT), LEFT, grow);
        over.push(length.scaled(x, y - LABEL_ABOVE, lerp(small, 1.0, grow) * pop));
        let under = ease_out(progress(u - 500_000, 300_000));
        let mut lines = Vec::new();
        self.struck(&mut lines, &duration_text(self.totals.before_us), 64.0, RIGHT, y + h + 86.0);
        let mut stats = self.stats(52.0, FG, LABEL);
        if self.recap.transcribed {
            stats.insert(0, ("Cut".to_owned(), style(52.0, LABEL, false, 0.0)));
        }
        let size = 52.0 * (BAR_W / self.row_width(&stats, 16.0)).min(1.0);
        for (_, style) in &mut stats {
            style.font_size = size;
        }
        self.row(&mut lines, LEFT, y + h + 176.0, &stats, 16.0);
        over.extend(lines.into_iter().map(|d| faded(d, under)));
        draws.extend(over.into_iter().map(|d| faded(d, leaving)));
        Ok(())
    }

    /// The finished cut, with a thin bar of its length playing along under it for a few seconds.
    fn finished(&mut self, draws: &mut Vec<Draw>, into: i64) -> Result<()> {
        draws.push(self.picture(true, into, 0.0, 1.0, 0.0)?);
        let opacity = progress(into, TEXT_OUT_US) * (1.0 - progress(into - AFTER_BAR_US, FADE_US));
        if opacity <= 0.0 {
            return Ok(());
        }
        let mut bar = vec![scrim(1100.0, 1500.0, 0.57)];
        let ones = vec![1.0; self.race.pieces.len()];
        let end = self.bar(&mut bar, 1352.0, 14.0, &ones, Some(self.recorded(into)), 1.0, &ones);
        let length = self.text(&duration_text(self.totals.after_us), &style(60.0, FG, true, 0.0));
        bar.push(length.at((end - length.ink()).max(LEFT), 1352.0 - 26.0));
        draws.extend(bar.into_iter().map(|d| faded(d, opacity)));
        Ok(())
    }

    /// The last picture of the finished cut, dark and pushing in, under the name and the bar as it ended.
    fn end_card(&mut self, draws: &mut Vec<Draw>, u: i64) -> Result<()> {
        let last = (self.sections[3] - frame_time_us(1, FPS)).max(0);
        let zoom = 1.0 + 0.04 * progress(u, END_CARD_US);
        let shown = ease_out(progress(u, 300_000));
        draws.push(self.picture(true, last, 0.0, zoom, -1.8 * shown)?);
        let mut card = Vec::new();
        let name = self.text("Cut in Nuzky", &style(100.0, FG, true, 0.0));
        card.push(name.at(LEFT, 876.0));
        let ones = vec![1.0; self.race.pieces.len()];
        card.push(outline(LEFT, 930.0, BAR_W, 32.0, 3.0, "#ffffff47"));
        self.bar(&mut card, 930.0, 32.0, &ones, Some(self.recap.before.duration_us()), 1.0, &ones);
        let after = self.text(&duration_text(self.totals.after_us), &style(112.0, FG, true, 0.0));
        card.push(after.at(LEFT, 1090.0));
        self.struck(&mut card, &duration_text(self.totals.before_us), 64.0, RIGHT, 1090.0);
        let site = self.text("nuzky.app", &style(46.0, LABEL, false, 0.0));
        card.push(site.at(LEFT, 1180.0));
        draws.extend(card.into_iter().map(|d| faded(d, shown)));
        Ok(())
    }

    /// Where `at` of the recording sits on the bar once each piece has snapped shut by `snapped`.
    fn x_at(&self, at: i64, snapped: &[f32]) -> f32 {
        let gone: f64 = self
            .race
            .pieces
            .iter()
            .zip(snapped)
            .filter(|(p, _)| p.end_us <= at)
            .map(|(p, s)| p.removed_us as f64 * *s as f64)
            .sum();
        LEFT + BAR_W * ((at as f64 - gone) / self.recap.before.duration_us().max(1) as f64) as f32
    }

    /// The recording's time `into` the finished cut shows.
    fn recorded(&self, into: i64) -> i64 {
        self.recap.cuts.iter().fold(into, |at, c| if c.start_us <= at { at + c.end_us - c.start_us } else { at })
    }

    /// The whole recording as a bar at `y`, `h` high: its cuts red until they snap shut, then a seam, and white as
    /// far as `played`, where its playhead is until the end. `drawn` is the share of it drawn and `shown` how far each piece has come in.
    /// Returns where it ends.
    #[allow(clippy::too_many_arguments)]
    fn bar(
        &self,
        draws: &mut Vec<Draw>,
        y: f32,
        h: f32,
        snapped: &[f32],
        played: Option<i64>,
        drawn: f32,
        shown: &[f32],
    ) -> f32 {
        let before = self.recap.before.duration_us().max(1) as f32;
        let end = self.x_at(self.recap.before.duration_us(), snapped);
        let reach = LEFT + (end - LEFT) * drawn;
        draws.push(rect(LEFT, y, reach - LEFT, h, AHEAD, h / 2.0));
        if let Some(at) = played {
            let x = self.x_at(at, snapped).min(reach);
            if x - LEFT >= 1.0 {
                draws.push(rect(LEFT, y, x - LEFT, h, FG, h / 2.0));
            }
        }
        for ((piece, &s), &come) in self.race.pieces.iter().zip(snapped).zip(shown) {
            let x = self.x_at(piece.start_us, snapped);
            let w = ((BAR_W * piece.removed_us as f32 / before).max(PIECE_MIN_PX) * (1.0 - s)).min(reach - x);
            if come > 0.0 && w > 0.5 {
                let round = if x <= LEFT + h / 2.0 || x + w >= end - h / 2.0 { h / 2.0 } else { 3.0 };
                draws.push(faded(rect(x, y, w, h, RED, round), come));
            }
            if s > 0.0 {
                draws.push(faded(rect(x - 3.0, y - 6.0, 6.0, h + 12.0, GRAPHITE, 3.0), s));
            }
        }
        if let Some(at) = played.filter(|&at| at < self.recap.before.duration_us()) {
            let x = self.x_at(at, snapped).min(reach);
            let (reach_y, tall) = (0.55 * h, 2.1 * h);
            draws.push(rect(x - 7.0, y - reach_y - 2.0, 14.0, tall + 4.0, SHADE, 7.0));
            draws.push(rect(x - 5.0, y - reach_y, 10.0, tall, "#ffffff", 5.0));
        }
        end
    }

    /// What a piece of the bar removed, as its chip says it.
    fn piece_text(&self, piece: &Piece) -> String {
        let length = piece.len() as f64 / 1e6;
        if piece.cuts > 1 {
            return match (self.recap.transcribed, piece.words) {
                (true, 1..) => format!("{} cuts · {}", piece.cuts, count(piece.words, "word", "words")),
                _ => format!("{} cuts", piece.cuts),
            };
        }
        match (piece.retakes, piece.words, self.recap.transcribed) {
            (1.., _, _) => "Retake".to_owned(),
            (0, 1.., _) => count(piece.words, "word", "words"),
            (_, _, true) => format!("Pause {length:.1} s"),
            _ => format!("Cut {length:.1} s"),
        }
    }

    /// The caption page around `at` of the recording, its cut words struck once passed.
    fn caption(&mut self, draws: &mut Vec<Draw>, at: i64, t: i64) {
        let words = &self.recap.words;
        let Some(here) = self.pages.partition_point(|p| words[p.start].start_us <= at).checked_sub(1) else {
            return;
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
        let size = CAPTION_SIZE;
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
            let baseline = CAPTION_BOTTOM - (lines.len() - 1 - n) as f32 * line_height;
            let mut x = (LEFT + RIGHT) / 2.0 - width / 2.0;
            for (text, struck) in line {
                draws.push(text.at(x, baseline));
                if *struck {
                    draws.push(rect(x - 4.0, baseline - 0.3 * size - 4.0, text.ink() + 8.0, 8.0, RED, 4.0));
                }
                x += text.ink() + gap;
            }
        }
    }
}

/// The recap's sound: a whoosh into each part and a short swish as each piece of the bar snaps shut, then the
/// finished cut as it plays.
/// The finished cut's music plays from the start of the recap, so the recording races to it.
struct Sound {
    cues: Project,
    soundtrack: Project,
    /// The soundtrack's music alone, which takes over from it as the finished cut ends.
    music: Project,
    /// Recap frames over which the soundtrack gives way to its music.
    handover: Range<i64>,
    cue_mixer: Mixer,
    mixer: Mixer,
    music_mixer: Mixer,
    /// Holds the peaks where the swishes land on the music, as export does.
    limiter: Limiter,
    /// Recap frame the mixers read next, `Limiter::latency()` ahead of what comes out.
    read: i64,
    mix: Vec<f32>,
    buffer: Vec<f32>,
    music_buffer: Vec<f32>,
}

impl Sound {
    fn new(recap: &Recap, cache_dir: &Path, [hook, fast, landing, sample, end_card]: [i64; 5]) -> Self {
        let start = hook + fast + landing;
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
        if let Some(swish) = &recap.swish {
            let mut clips: Vec<Clip> = Vec::new();
            for shown in Race::new(recap).shown {
                let at = hook + shown.end + SWISH_DELAY_US;
                if clips.last().is_none_or(|last| at - last.start_us >= SWISH_GAP_US) {
                    clips.push(clip(swish, at, 0.8));
                }
            }
            cues.assets.push(swish.clone());
            cues.tracks.push(track("swishes", clips));
        }
        if let Some(whoosh) = &recap.whoosh {
            let lead = whoosh.duration_us / 2;
            let clips = vec![clip(whoosh, hook - lead, 0.6), clip(whoosh, start - lead, 0.6)];
            cues.assets.push(whoosh.clone());
            cues.tracks.push(track("whooshes", clips));
        }
        if let Some(land) = &recap.land {
            cues.assets.push(land.clone());
            cues.tracks.push(track("landing", vec![clip(land, hook + fast + LAND_HIT_US, 0.5)]));
        }
        let stop = start + sample;
        let (soundtrack, music) = soundtrack(&recap.after, start, stop + end_card, end_card);
        let mut sound = Self {
            cues,
            soundtrack,
            music,
            handover: us_to_samples(stop - HANDOVER_US)..us_to_samples(stop),
            cue_mixer: Mixer::new(cache_dir.to_path_buf()),
            mixer: Mixer::new(cache_dir.to_path_buf()),
            music_mixer: Mixer::new(cache_dir.to_path_buf()),
            limiter: Limiter::new(db_to_gain(LIMITER_CEILING_DB)),
            read: 0,
            mix: Vec::new(),
            buffer: Vec::new(),
            music_buffer: Vec::new(),
        };
        let early = sound.read_ahead(Limiter::latency()).count();
        debug_assert_eq!(early, 0, "the limiter lets nothing out before its look-ahead is read");
        sound
    }

    /// Mixes the next `frames` of the recap into the limiter, returning what it lets out.
    fn read_ahead(&mut self, frames: usize) -> impl Iterator<Item = [f32; CHANNELS]> + '_ {
        let (from, to) = (self.read, self.read + frames as i64);
        self.read = to;
        for buffer in [&mut self.mix, &mut self.buffer, &mut self.music_buffer] {
            buffer.clear();
            buffer.resize(frames * CHANNELS, 0.0);
        }
        self.cue_mixer.mix(&self.cues, from, &mut self.mix);
        let Range { start: begin, end } = self.handover.clone();
        if from < end {
            self.mixer.mix(&self.soundtrack, from, &mut self.buffer);
        }
        if to > begin {
            self.music_mixer.mix(&self.music, from, &mut self.music_buffer);
        }
        let limiter = &mut self.limiter;
        let frames = self.mix.as_chunks::<CHANNELS>().0.iter().zip(self.buffer.as_chunks::<CHANNELS>().0);
        frames.zip(self.music_buffer.as_chunks::<CHANNELS>().0).enumerate().filter_map(
            move |(i, ((cue, all), music))| {
                let gain = ((from + i as i64 - begin) as f32 / (end - begin).max(1) as f32).clamp(0.0, 1.0);
                limiter.push(std::array::from_fn(|c| cue[c] + all[c] * (1.0 - gain) + music[c] * gain))
            },
        )
    }

    /// `pos` must continue where the previous call ended.
    fn fill(&mut self, pos: i64, out: &mut [f32]) {
        debug_assert_eq!(pos, self.read - Limiter::latency() as i64);
        let frames = out.len() / CHANNELS;
        for (out, frame) in out.as_chunks_mut::<CHANNELS>().0.iter_mut().zip(self.read_ahead(frames)) {
            out.copy_from_slice(&frame);
        }
    }
}

/// The finished cut's sound on the recap's timeline, and its music alone: the music from the start of the recap to
/// `end`, quieter while the recording races so the swishes stand out, and everything else from `start`, as the
/// picture plays it, so it keeps its transitions and the music still ducks under the speech. The music fades out
/// over `fade`, the end card.
fn soundtrack(after: &Project, start: i64, end: i64, fade: i64) -> (Project, Project) {
    const FADE_IN_US: i64 = 100_000;
    const FADE_OUT_US: i64 = 300_000;
    let bed_gain = db_to_gain(BED_DB);
    let length = after.duration_us();
    let assets: std::collections::HashMap<&str, &Asset> = after.assets.iter().map(|a| (a.id.as_str(), a)).collect();
    let is_music = |track: &Track, clip: &Clip| {
        matches!(&clip.content, ClipContent::Media { duck_db, .. }
            if track.kind == TrackKind::Audio && (*duck_db > 0.0 || clip.duration_us >= MUSIC_US))
    };
    // The music starts with the recap even when the finished cut brings it in later.
    let lead = after.tracks.iter().flat_map(|t| t.clips.iter().filter(|c| is_music(t, c))).map(|c| c.start_us).min();
    let lead = lead.unwrap_or(0);
    let mut project = after.clone();
    let mut music = after.clone();
    for (track, music) in project.tracks.iter_mut().zip(&mut music.tracks) {
        music.clips.clear();
        for mut clip in std::mem::take(&mut track.clips) {
            let is_music = is_music(track, &clip);
            // Music that plays to the end of the finished cut plays on past it while its file lasts.
            let to_the_end = clip.start_us + clip.duration_us >= length;
            if is_music {
                clip.start_us -= lead;
            }
            let mut bed = clip.clone();
            let ClipContent::Media { asset_id, source_in_us, volume, speed, fade_in_us, fade_out_us, .. } =
                &mut clip.content
            else {
                clip.start_us += start;
                track.clips.push(clip);
                continue;
            };
            if !is_music {
                if clip.start_us == 0 {
                    *fade_in_us = (*fade_in_us).max(FADE_IN_US);
                }
                clip.start_us += start;
                track.clips.push(clip);
                continue;
            }
            if to_the_end {
                let left = assets.get(asset_id.as_str()).map_or(0, |a| a.duration_us - *source_in_us);
                let longest = (left.max(0) as f64 / *speed as f64) as i64;
                clip.duration_us = clip.duration_us.max(longest.min(end - clip.start_us));
            }
            if clip.start_us >= end {
                continue;
            }
            if clip.start_us + clip.duration_us >= end {
                clip.duration_us = end - clip.start_us;
                *fade_out_us = (*fade_out_us).max(fade.max(FADE_OUT_US)).min(clip.duration_us);
            }
            if clip.start_us + clip.duration_us <= start {
                *volume *= bed_gain;
            } else if clip.start_us < start {
                let into = start - clip.start_us;
                *source_in_us += (into as f64 * *speed as f64) as i64;
                *fade_in_us = 0;
                clip.id = format!("{}-finished", clip.id);
                clip.start_us = start;
                clip.duration_us -= into;
                bed.duration_us = into;
                if let ClipContent::Media { volume, fade_out_us, .. } = &mut bed.content {
                    *volume *= bed_gain;
                    *fade_out_us = 0;
                }
                music.clips.push(bed.clone());
                track.clips.push(bed);
            }
            music.clips.push(clip.clone());
            track.clips.push(clip);
        }
        track.clips.sort_by_key(|c| c.start_us);
        music.clips.sort_by_key(|c| c.start_us);
    }
    (project, music)
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
            swish: None,
            whoosh: None,
            land: None,
        }
    }

    #[test]
    fn a_recap_fits_in_fourteen_seconds_and_never_plays_slower_than_recorded() {
        for (before, after) in [(2, 1), (10, 6), (222, 68), (1200, 300), (36_000, 20_000)] {
            let recap = recap(before, after);
            let [_, fast, _, sample, _] = sections(&recap, true);
            assert!(duration_us(&recap, true) <= 14_000_000, "{before} s");
            assert!(fast <= before * 1_000_000 && sample <= after * 1_000_000, "{before} s");
            assert_eq!(duration_us(&recap, true) - duration_us(&recap, false), END_CARD_US);
        }
        // A short recording plays at a few times its speed, not a frame a second.
        let [_, fast, ..] = sections(&recap(10, 6), false);
        assert_eq!(fast, FAST_MIN_US);
    }

    #[test]
    fn each_cut_stays_on_screen_and_the_race_still_fits_and_never_plays_slower_than_recorded() {
        let cut =
            |a: f64, b: f64| RecapCut { start_us: (a * 1e6) as i64, end_us: (b * 1e6) as i64, words: 1, retakes: 0 };
        let mut long = recap(600, 300);
        long.cuts = (0..60).map(|i| cut(i as f64 * 10.0 + 2.0, i as f64 * 10.0 + 7.0)).collect();
        let mut short = recap(20, 15);
        short.cuts = vec![cut(2.0, 4.0), cut(10.0, 12.5), cut(12.6, 13.0)];
        let mut most = recap(30, 3);
        most.cuts = vec![cut(0.5, 14.0), cut(15.0, 29.5)];
        for recap in [&long, &short, &most] {
            let race = Race::new(recap);
            assert!(duration_us(recap, true) <= 14_000_000);
            // The bar shrinks by what the cuts removed, not by the kept footage between cuts that go as one.
            let removed: i64 = recap.cuts.iter().map(|c| c.end_us - c.start_us).sum();
            assert_eq!(race.pieces.iter().map(|p| p.removed_us).sum::<i64>(), removed);
            assert!(race.kept_us >= race.kept_screen_us, "kept footage plays at least as fast as recorded");
            let budget = race.length / 2 / race.pieces.len() as i64;
            for (piece, shown) in race.pieces.iter().zip(&race.shown) {
                assert!(shown.end - shown.start >= piece.len().min(CUT_SCREEN_US).min(budget) - 1, "{piece:?}");
                assert!(shown.end - shown.start <= piece.len());
                // The recording runs on without a jump into and out of each piece.
                assert!((race.at(shown.start).0 - piece.start_us).abs() < 2_000);
                assert!((race.at(shown.end).0 - piece.end_us).abs() < 2_000);
            }
            let times: Vec<i64> = (0..race.length).step_by(33_333).map(|t| race.at(t).0).collect();
            assert!(times.windows(2).all(|w| w[0] <= w[1]));
        }
        // Cuts a few pixels of the bar apart go as one piece with one chip.
        let race = Race::new(&short);
        assert_eq!(race.pieces.len(), 2);
        assert_eq!((race.pieces[1].start_us, race.pieces[1].end_us, race.pieces[1].cuts), (10_000_000, 13_000_000, 2));
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

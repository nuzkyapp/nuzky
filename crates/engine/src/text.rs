//! CPU text rasterisation with outline and background box, cached by text content, style and output size.
//! Karaoke captions also keep their layout, so the spoken word changes colour without laying out again.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use cosmic_text::{
    Align, Attrs, Buffer, CacheKey, Color, Family, FeatureTag, FontFeatures, FontSystem, LineIter, Metrics, Shaping,
    SwashCache, Weight,
};

use crate::gpu::Image;
use crate::model::{LETTER_SPACING, LINE_SPACING, LetterCase, TextAlign, TextStyle, max_stroke_width, parse_color};

// Physical raster limits also bound legacy files, zoomed text, many lines and cache retention.
const MAX_GLYPH_PX: f32 = 2048.0;
const MAX_RASTER_SIDE: usize = 8192;
const MAX_RASTER_PIXELS: usize = 16 * 1024 * 1024;
const MAX_TEXT_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_GLYPH_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_LAYOUT_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_RASTER_SCALE: f32 = 8.0;
const MIN_RASTER_SCALE: f32 = 0.01;
/// Layout bounds in canvas pixels; tall text may also use up to MAX_TEXT_LINES lines.
const MAX_LAYOUT_SIDE: f32 = 4.0 * 7680.0;
/// Bounds layout work for pasted walls of text at any font size.
const MAX_TEXT_LINES: f32 = 1000.0;

fn finite_clamp(value: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() { value.clamp(min, max) } else { min }
}

/// Rasterised text and the output pixels per canvas pixel it was drawn at. Large text is drawn at
/// a lower scale than requested, so callers size the layer with this one.
#[derive(Clone)]
pub struct TextImage {
    pub image: Image,
    pub scale: f32,
}

#[derive(serde::Deserialize)]
pub struct FontFace {
    pub family: String,
    pub file: String,
    pub weight: String,
}

pub static FONT_MANIFEST: std::sync::LazyLock<Vec<FontFace>> = std::sync::LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../assets/fonts/manifest.json")).expect("valid bundled font manifest")
});

pub static BUNDLED_FONT_FAMILIES: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| {
    FONT_MANIFEST.iter().map(|face| face.family.as_str()).collect::<BTreeSet<_>>().into_iter().collect()
});

macro_rules! embed_fonts {
    ($($file:literal),* $(,)?) => {
        &[$(($file, include_bytes!(concat!("../../../assets/fonts/", $file)) as &[u8])),*]
    };
}
const FONT_DATA: &[(&str, &[u8])] = embed_fonts![
    "anton/Anton-Regular.ttf",
    "bebasneue/BebasNeue-Regular.ttf",
    "inter/Inter[opsz,wght].ttf",
    "lexend/Lexend[wght].ttf",
    "montserrat/Montserrat[wght].ttf",
    "oswald/Oswald[wght].ttf",
    "poppins/Poppins-Regular.ttf",
    "poppins/Poppins-Bold.ttf",
    "roboto/Roboto[wdth,wght].ttf",
];

static BUNDLED_FONTS: std::sync::LazyLock<Vec<&'static [u8]>> = std::sync::LazyLock::new(|| {
    FONT_MANIFEST
        .iter()
        .map(|face| FONT_DATA.iter().find(|(file, _)| *file == face.file).expect("manifest font is embedded").1)
        .collect()
});

/// Text without a chosen font, or with one this computer lacks, uses this bundled family, so a project
/// looks the same on every machine.
pub const DEFAULT_FAMILY: &str = "Inter";

pub struct TextRenderer {
    fonts: FontSystem,
    families: HashSet<String>,
    /// Whether a family draws bold itself, by family name.
    bold: HashMap<String, bool>,
    swash: SwashCache,
    cache: HashMap<u64, TextImage>,
    /// Layouts of karaoke captions, by text, style, scale and wrap width.
    layouts: HashMap<u64, Arc<Layout>>,
    stats: TextStats,
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRenderer {
    pub fn new() -> Self {
        let mut fonts = FontSystem::new();
        // An installed copy of a bundled family, often another version, would draw instead of ours.
        let installed: Vec<_> = fonts
            .db()
            .faces()
            .filter(|face| face.families.iter().any(|(name, _)| BUNDLED_FONT_FAMILIES.contains(&name.as_str())))
            .map(|face| face.id)
            .collect();
        for id in installed {
            fonts.db_mut().remove_face(id);
        }
        for data in BUNDLED_FONTS.iter() {
            fonts.db_mut().load_font_data(data.to_vec());
        }
        fonts.db_mut().set_sans_serif_family(DEFAULT_FAMILY);
        let families = fonts.db().faces().flat_map(|face| face.families.iter().map(|(name, _)| name.clone())).collect();
        Self {
            fonts,
            families,
            bold: HashMap::new(),
            swash: SwashCache::new(),
            cache: HashMap::new(),
            layouts: HashMap::new(),
            stats: TextStats::default(),
        }
    }

    /// Bundled families first, then installed families, each group sorted and unique.
    pub fn font_families(&self) -> Vec<String> {
        let system: BTreeSet<&str> =
            self.families.iter().map(String::as_str).filter(|name| !BUNDLED_FONT_FAMILIES.contains(name)).collect();
        BUNDLED_FONT_FAMILIES.iter().copied().chain(system).map(str::to_owned).collect()
    }

    /// Whether text in `family` is drawn in it: bundled and installed families match by exact name.
    /// Text in any other family is drawn in `DEFAULT_FAMILY`.
    pub fn has_family(&self, family: &str) -> bool {
        self.families.contains(family)
    }

    fn family<'a>(&self, name: Option<&'a str>) -> Family<'a> {
        match name {
            Some(name) if self.has_family(name) => Family::Name(name),
            _ => Family::SansSerif,
        }
    }

    /// Single-weight families such as Anton have no bold face; asking for one would make
    /// cosmic-text draw another family's bold, so they stay regular instead.
    fn draws_bold(&mut self, family: Family) -> bool {
        let name = match family {
            Family::Name(name) => name,
            _ => DEFAULT_FAMILY,
        };
        if let Some(&known) = self.bold.get(name) {
            return known;
        }
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(16.0, 20.0));
        buffer.set_text("A", &Attrs::new().family(family).weight(Weight::BOLD), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let font = buffer.layout_runs().flat_map(|run| run.glyphs.iter()).map(|glyph| glyph.font_id).next();
        let own = font
            .and_then(|id| self.fonts.db().face(id))
            .is_some_and(|face| face.families.iter().any(|(n, _)| n == name));
        self.bold.insert(name.to_owned(), own);
        own
    }

    /// Renders `text` with `style` scaled by `scale` (output pixels per canvas pixel).
    /// `max_width` is the wrap width in output pixels at the requested scale.
    pub fn render(&mut self, text: &str, style: &TextStyle, scale: f32, max_width: f32) -> TextImage {
        self.render_spoken(text, style, scale, max_width, None, &[])
    }

    /// Like `render`, with the words at byte ranges `keys` of `text` filled in `style.keywords` and the
    /// word at `spoken` in `style.highlight`, over a key word too. Every variant uses the same layout, so
    /// the size and wrapping never change between words. The layout is kept, so a new spoken word only
    /// repaints colours; shaping, glyph rasterising and the outline happen once per text, never per frame.
    pub fn render_spoken(
        &mut self,
        text: &str,
        style: &TextStyle,
        scale: f32,
        max_width: f32,
        spoken: Option<Range<usize>>,
        keys: &[Range<usize>],
    ) -> TextImage {
        let karaoke = style.highlight.is_some() || style.keywords.is_some();
        let spoken = spoken.filter(|range| style.highlight.is_some() && range.start < range.end);
        let keys = if style.keywords.is_some() { keys } else { &[] };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        format!("{style:?}").hash(&mut h);
        scale.to_bits().hash(&mut h);
        max_width.to_bits().hash(&mut h);
        let layout_key = h.finish();
        spoken.hash(&mut h);
        keys.hash(&mut h);
        let key = h.finish();
        if let Some(img) = self.cache.get(&key) {
            return img.clone();
        }
        let img = if karaoke {
            let layout = match self.layouts.get(&layout_key) {
                Some(layout) => layout.clone(),
                None => {
                    let layout = Arc::new(self.lay_out(text, style, scale, max_width));
                    if self.layouts.len() >= 64
                        || self.layouts.values().map(|l| l.bytes()).sum::<usize>() + layout.bytes()
                            > MAX_LAYOUT_CACHE_BYTES
                    {
                        self.layouts.clear();
                    }
                    self.layouts.insert(layout_key, layout.clone());
                    layout
                }
            };
            let spoken = spoken.map(|range| self.word_mask(&layout, &[range]));
            let keys = (!keys.is_empty()).then(|| self.word_mask(&layout, keys));
            self.paint(&layout, style, keys.as_deref(), spoken.as_deref())
        } else {
            let layout = self.lay_out(text, style, scale, max_width);
            self.paint(&layout, style, None, None)
        };
        if self.cache.len() >= 512
            || self.cache.values().map(|text| text.image.data.len()).sum::<usize>() + img.image.data.len()
                > MAX_TEXT_CACHE_BYTES
        {
            self.cache.clear();
        }
        self.cache.insert(key, img.clone());
        img
    }

    /// How often text was laid out and painted since the renderer started, for benchmarks.
    pub fn stats(&self) -> TextStats {
        self.stats
    }

    /// Coverage of the glyphs whose characters start inside `ranges`, words of the text.
    fn word_mask(&mut self, layout: &Layout, ranges: &[Range<usize>]) -> Vec<u8> {
        let (w, h) = (layout.w, layout.h);
        let mut mask = vec![0u8; w * h];
        for glyph in layout.glyphs.iter().filter(|glyph| ranges.iter().any(|range| range.contains(&glyph.start))) {
            stamp(&mut self.swash, &mut self.fonts, glyph, &mut mask, w, h);
        }
        mask
    }

    /// Shapes `text`, rasterises its glyphs into one coverage mask and dilates it for the outline.
    fn lay_out(&mut self, text: &str, style: &TextStyle, scale: f32, max_width: f32) -> Layout {
        self.stats.layouts += 1;
        // Layout stays in canvas pixels; only glyph rasterisation uses the output scale.
        let wanted = finite_clamp(scale, MIN_RASTER_SCALE, MAX_RASTER_SCALE);
        let size = finite_clamp(style.font_size, 1.0, MAX_GLYPH_PX / MIN_RASTER_SCALE);
        let stroke = finite_clamp(style.stroke_width, 0.0, max_stroke_width(size));
        let max_width = finite_clamp(max_width / wanted, 1.0, MAX_LAYOUT_SIDE);
        let pad_box = if style.background.is_some() { size * 0.3 } else { 0.0 };
        let letter_spacing = finite_clamp(style.letter_spacing, *LETTER_SPACING.start(), *LETTER_SPACING.end());
        let line_spacing = finite_clamp(style.line_spacing, *LINE_SPACING.start(), *LINE_SPACING.end()) * size;
        let shadow = style.shadow.as_ref().map(|shadow| shadow.in_pixels(size));
        let text_pad = stroke.ceil() + pad_box.ceil() + 2.0;
        // The image grows by the shadow's reach on every side, so the text stays at the layer's centre.
        let room = shadow.map_or(0.0, |((dx, dy), blur)| (dx.abs().max(dy.abs()) + blur).ceil());
        let pad = (text_pad + room) as i32;

        let line_height = size * 1.2 + line_spacing;
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, line_height));
        // Text wraps where it would without a shadow.
        let wrap = (max_width - 2.0 * text_pad).max(size);
        buffer.set_size(Some(wrap), Some((MAX_TEXT_LINES * line_height).max(MAX_LAYOUT_SIDE)));
        let family = self.family(style.font_family.as_deref());
        let weight = if style.bold && self.draws_bold(family) { Weight::BOLD } else { Weight::NORMAL };
        let mut attrs = Attrs::new().family(family).weight(weight);
        if letter_spacing != 0.0 {
            // As in CSS: spaced letters do not join into ligatures, so "ffi" spreads like the letters around it.
            let mut features = FontFeatures::new();
            features.disable(FeatureTag::STANDARD_LIGATURES).disable(FeatureTag::CONTEXTUAL_LIGATURES);
            attrs = attrs.letter_spacing(letter_spacing).font_features(features);
        }
        let text = if text.trim().is_empty() { " " } else { text };
        let (text, shown_to_typed) = cased(text, style.letter_case);
        let align = match style.align {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        };
        buffer.set_text(&text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.fonts, false);

        let mut line_w: f32 = 0.0;
        let (mut left, mut right) = (f32::MAX, f32::MIN);
        let mut text_h: f32 = 0.0;
        for run in buffer.layout_runs() {
            line_w = line_w.max(run.line_w);
            for glyph in run.glyphs {
                left = left.min(glyph.x);
                right = right.max(glyph.x + glyph.w - letter_spacing * size);
            }
            text_h = text_h.max(run.line_top + size * 1.2);
        }
        // Glyphs line up inside `wrap`, so crop to the widest line. Every line ends in the letter spacing
        // after its last letter, which is not drawn. Pulled closer than their own width, letters reach past the
        // line, so the crop follows the letters. Line spacing goes only between lines: cosmic-text centres each
        // line in its height, so the glyphs move up by half the spacing and the last line ends as without.
        let (x0, line_w) = if letter_spacing < 0.0 && left <= right {
            (left, right - left)
        } else {
            let start = match style.align {
                TextAlign::Left => 0.0,
                TextAlign::Center => (wrap - line_w) / 2.0,
                TextAlign::Right => wrap - line_w,
            };
            (start, (line_w - letter_spacing * size).max(0.0))
        };
        let x0 = x0.floor() as i32;
        let canvas_w = (line_w.ceil() + 2.0 * pad as f32).max(1.0);
        let canvas_h = (text_h.ceil() + 2.0 * pad as f32).max(1.0);
        // Large text gets a coarser raster rather than a smaller font or a cropped bitmap.
        let mut scale = wanted
            .min(MAX_GLYPH_PX / size)
            .min(MAX_RASTER_SIDE as f32 / canvas_w.max(canvas_h))
            .min((MAX_RASTER_PIXELS as f32 / (canvas_w * canvas_h)).sqrt())
            .max(MIN_RASTER_SCALE);
        let pixels = |scale: f32| ((canvas_w * scale).ceil() as usize, (canvas_h * scale).ceil() as usize);
        // Rounding up can overshoot a limit by a pixel.
        while scale > MIN_RASTER_SCALE
            && let (w, h) = pixels(scale)
            && (w.max(h) > MAX_RASTER_SIDE || w * h > MAX_RASTER_PIXELS)
        {
            scale = (scale * 0.999).max(MIN_RASTER_SCALE);
        }
        // Pathological text below the minimum scale is clipped before any allocation.
        let (w, h) = pixels(scale);
        let w = w.clamp(1, MAX_RASTER_SIDE);
        let h = h.clamp(1, MAX_RASTER_SIDE.min(MAX_RASTER_PIXELS / w));

        let mut fill = vec![0u8; w * h];
        // Glyph positions name the byte of their line; lines start where cosmic-text split the text.
        let line_starts: Vec<usize> = LineIter::new(&text).map(|(range, _)| range.start).collect();
        let mut glyphs = Vec::new();
        for run in buffer.layout_runs() {
            let line_start = line_starts.get(run.line_i).copied().unwrap_or(usize::MAX / 2);
            for glyph in run.glyphs {
                let offset = ((pad - x0) as f32 * scale, (run.line_y - line_spacing / 2.0 + pad as f32) * scale);
                let physical = glyph.physical(offset, scale);
                let shown = line_start + glyph.start;
                let start = shown_to_typed.as_ref().map_or(shown, |typed| typed.get(shown).copied().unwrap_or(shown));
                let placed = Placed { start, key: physical.cache_key, x: physical.x, y: physical.y };
                stamp(&mut self.swash, &mut self.fonts, &placed, &mut fill, w, h);
                glyphs.push(placed);
            }
        }

        let outline = if stroke > 0.0 { Some(dilate(&fill, w, h, stroke * scale)) } else { None };
        let shadow = shadow.map(|((dx, dy), blur)| {
            let mut mask = shifted(outline.as_ref().unwrap_or(&fill), w, h, (dx * scale).round(), (dy * scale).round());
            box_blur(&mut mask, w, h, blur * scale);
            mask
        });
        let inset = room * scale;
        let radius =
            (finite_clamp(style.box_radius, 0.0, 1.0) * size * scale).min((w.min(h) as f32 - 2.0 * inset) / 2.0);
        Layout { w, h, scale, radius: radius.max(0.0), inset, fill, outline, shadow, glyphs }
    }

    /// Colours a layout: box, outline, then the fill, with the key words' coverage in the keyword colour and
    /// the spoken word's in the highlight.
    fn paint(&mut self, layout: &Layout, style: &TextStyle, keys: Option<&[u8]>, spoken: Option<&[u8]>) -> TextImage {
        self.stats.paints += 1;
        let Layout { w, h, scale, radius, inset, ref fill, ref outline, ref shadow, .. } = *layout;
        let fill_c = parse_color(&style.color);
        let shadow_c = style.shadow.as_ref().map(|shadow| {
            let [r, g, b, a] = parse_color(&shadow.color);
            [r, g, b, a * finite_clamp(shadow.opacity, 0.0, 1.0)]
        });
        let stroke_c = parse_color(&style.stroke_color);
        let bg_c = style.background.as_deref().map(parse_color);
        let highlight_c = style.highlight.as_deref().map(parse_color).unwrap_or(fill_c);
        let key_c = style.keywords.as_ref().map(|k| parse_color(&k.color)).unwrap_or(fill_c);
        // The share of a fill pixel a mask's glyphs cover.
        let share =
            |mask: Option<&[u8]>, i: usize| mask.map_or(0.0, |m| (m[i] as f32 / fill[i].max(1) as f32).min(1.0));

        // Premultiplied "over" compositing, converted back to straight alpha at the end.
        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let mut acc = [0f32; 4];
                let mut over = |c: [f32; 4], cov: f32| {
                    let a = c[3] * cov;
                    for k in 0..3 {
                        acc[k] = c[k] * a + acc[k] * (1.0 - a);
                    }
                    acc[3] = a + acc[3] * (1.0 - a);
                };
                if let Some(bg) = bg_c {
                    let (x, y) = (x as f32 + 0.5 - inset, y as f32 + 0.5 - inset);
                    over(bg, rounded_rect_coverage(x, y, w as f32 - 2.0 * inset, h as f32 - 2.0 * inset, radius));
                }
                if let (Some(s), Some(c)) = (shadow, shadow_c) {
                    over(c, s[i] as f32 / 255.0);
                }
                if let Some(o) = outline {
                    over(stroke_c, o[i] as f32 / 255.0);
                }
                // A word's glyphs cover the same pixels as in the fill mask; where a neighbour's edge
                // overlaps them, the colours mix by coverage.
                if fill[i] > 0 {
                    let (key, lit) = (share(keys, i), share(spoken, i));
                    let c: [f32; 4] = std::array::from_fn(|k| {
                        let base = fill_c[k] + (key_c[k] - fill_c[k]) * key;
                        base + (highlight_c[k] - base) * lit
                    });
                    over(c, fill[i] as f32 / 255.0);
                }
                if acc[3] > 0.0 {
                    let o = &mut out[i * 4..i * 4 + 4];
                    for k in 0..3 {
                        o[k] = ((acc[k] / acc[3]).clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                    o[3] = (acc[3] * 255.0).round() as u8;
                }
            }
        }
        TextImage { image: Image { width: w as u32, height: h as u32, data: Arc::new(out) }, scale }
    }
}

/// Text laid out at one output scale: what every colour variant of it shares.
struct Layout {
    w: usize,
    h: usize,
    scale: f32,
    /// Corner radius of the background box in output pixels.
    radius: f32,
    /// Room for the shadow around the background box, in output pixels.
    inset: f32,
    /// Glyph coverage.
    fill: Vec<u8>,
    /// Glyph coverage dilated by the outline width.
    outline: Option<Vec<u8>>,
    /// The outline's coverage, or the glyphs' without one, moved and blurred.
    shadow: Option<Vec<u8>>,
    glyphs: Vec<Placed>,
}

impl Layout {
    fn bytes(&self) -> usize {
        let masks = 1 + usize::from(self.outline.is_some()) + usize::from(self.shadow.is_some());
        self.fill.len() * masks + self.glyphs.len() * size_of::<Placed>()
    }
}

/// `text` as `case` draws it and, when that changed it, the byte of `text` each of its bytes comes from, so
/// karaoke and key words still find their words.
fn cased(text: &str, case: LetterCase) -> (std::borrow::Cow<'_, str>, Option<Vec<usize>>) {
    if case == LetterCase::None {
        return (text.into(), None);
    }
    let mut shown = String::with_capacity(text.len());
    let mut typed = Vec::with_capacity(text.len() + 1);
    let mut word_start = true;
    for (at, c) in text.char_indices() {
        let before = shown.len();
        match case {
            LetterCase::Upper => shown.extend(c.to_uppercase()),
            LetterCase::Lower => shown.extend(c.to_lowercase()),
            _ if word_start && c.is_alphabetic() => shown.extend(c.to_uppercase()),
            _ => shown.push(c),
        }
        word_start = c.is_whitespace() || (word_start && !c.is_alphanumeric());
        typed.resize(typed.len() + shown.len() - before, at);
    }
    typed.push(text.len());
    (shown.into(), Some(typed))
}

/// `mask` moved by whole pixels, dropping what moves past an edge.
fn shifted(mask: &[u8], w: usize, h: usize, dx: f32, dy: f32) -> Vec<u8> {
    let (dx, dy) = (dx as isize, dy as isize);
    let mut out = vec![0u8; w * h];
    let (from, to) = (dx.min(0).unsigned_abs(), dx.max(0) as usize);
    let len = w.saturating_sub(dx.unsigned_abs());
    for y in 0..h {
        let sy = y as isize - dy;
        if len == 0 || sy < 0 || sy >= h as isize {
            continue;
        }
        let src = sy as usize * w + from;
        out[y * w + to..y * w + to + len].copy_from_slice(&mask[src..src + len]);
    }
    out
}

/// Softens `mask` by three box blurs in each direction, close to a Gaussian, reaching `radius` pixels in
/// all. A pass costs the same for any radius.
fn box_blur(mask: &mut [u8], w: usize, h: usize, radius: f32) {
    let k = (radius / 3.0).round() as usize;
    if k == 0 {
        return;
    }
    let mut line = Vec::new();
    for _ in 0..3 {
        for y in 0..h {
            blur_line(mask, y * w, 1, w, k, &mut line);
        }
        for x in 0..w {
            blur_line(mask, x, w, h, k, &mut line);
        }
    }
}

/// Averages each of `n` values `step` apart from `start` with the `k` on either side, counting those past
/// the ends as empty.
fn blur_line(mask: &mut [u8], start: usize, step: usize, n: usize, k: usize, line: &mut Vec<u8>) {
    line.clear();
    line.extend((0..n).map(|i| mask[start + i * step]));
    let window = (2 * k + 1) as u32;
    let mut sum: u32 = line[..k.min(n)].iter().map(|&v| u32::from(v)).sum();
    for i in 0..n {
        if i + k < n {
            sum += u32::from(line[i + k]);
        }
        mask[start + i * step] = ((sum + window / 2) / window) as u8;
        if i >= k {
            sum -= u32::from(line[i - k]);
        }
    }
}

/// A glyph in the raster, with the byte in the text where its characters start.
struct Placed {
    start: usize,
    key: CacheKey,
    x: i32,
    y: i32,
}

/// Counts of the expensive steps, so a benchmark can show they do not happen per frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStats {
    /// Shaping, glyph rasterising and outline dilation of one text at one size.
    pub layouts: u64,
    /// Colouring a layout into an image: once per text, and once per spoken word of a karaoke caption.
    pub paints: u64,
}

/// Draws one glyph's coverage into `mask`, keeping the higher value where glyphs overlap.
fn stamp(swash: &mut SwashCache, fonts: &mut FontSystem, glyph: &Placed, mask: &mut [u8], w: usize, h: usize) {
    if swash.image_cache.values().flatten().map(|image| image.data.len()).sum::<usize>() > MAX_GLYPH_CACHE_BYTES {
        swash.image_cache.clear();
    }
    swash.with_pixels(fonts, glyph.key, Color::rgb(255, 255, 255), |x, y, color| {
        let (px, py) = (x + glyph.x, y + glyph.y);
        if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
            let i = py as usize * w + px as usize;
            mask[i] = mask[i].max(color.a());
        }
    });
}

/// Approximates a round brush outline by stamping the glyph mask on three rings. An outline under half a pixel,
/// such as a thin one in a small preview, is the half-pixel ring at its share of coverage, so it fades instead of
/// vanishing.
fn dilate(mask: &[u8], w: usize, h: usize, radius: f32) -> Vec<u8> {
    let share = (radius * 512.0).min(256.0) as u32;
    let radius = radius.max(0.5);
    let mut out = mask.to_vec();
    let mut offsets = Vec::new();
    for ring in [1.0, 0.66, 0.33] {
        let r = radius * ring;
        let steps = ((r * 2.0) as usize).clamp(8, 32);
        for k in 0..steps {
            let a = k as f32 / steps as f32 * std::f32::consts::TAU;
            offsets.push(((a.cos() * r).round() as i32, (a.sin() * r).round() as i32));
        }
    }
    offsets.sort_unstable();
    offsets.dedup();
    for (dx, dy) in offsets {
        for y in 0..h as i32 {
            let sy = y - dy;
            if sy < 0 || sy >= h as i32 {
                continue;
            }
            let (row, src) = (y as usize * w, sy as usize * w);
            for x in 0..w as i32 {
                let sx = x - dx;
                if sx >= 0 && sx < w as i32 {
                    let v = ((mask[src + sx as usize] as u32 * share) >> 8) as u8;
                    let o = &mut out[row + x as usize];
                    if v > *o {
                        *o = v;
                    }
                }
            }
        }
    }
    out
}

fn rounded_rect_coverage(x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let qx = (x - w / 2.0).abs() - (w / 2.0 - r);
    let qy = (y - h / 2.0).abs() - (h / 2.0 - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
    (0.5 - outside).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BOX_RADIUS, TextShadow};
    use cosmic_text::skrifa::{FontRef, MetadataProvider};

    const CZECH: &str = "Příliš žluťoučký kůň ěščřžýáíéúůťďň ĚŠČŘŽÝÁÍÉÚŮŤĎŇ";

    #[test]
    fn outline_under_half_a_pixel_fades_instead_of_vanishing() {
        let mut dot = vec![0u8; 25];
        dot[12] = 255;
        let beside = |radius| dilate(&dot, 5, 5, radius)[13];
        // A 1.5 px outline drawn at a quarter of its size, as a small preview does.
        assert_eq!(beside(0.375), 191);
        assert_eq!(beside(0.5), 255);
        assert_eq!(beside(1.5), 255);
    }

    fn style(family: &str) -> TextStyle {
        TextStyle {
            font_family: Some(family.into()),
            font_size: 95.0,
            color: "#ffffff".into(),
            bold: false,
            stroke_width: 7.5,
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

    #[test]
    fn large_text_gets_a_coarser_raster_instead_of_a_crop() {
        let mut renderer = bundled_renderer();
        let canvas_size =
            |text: &TextImage| (text.image.width as f32 / text.scale, text.image.height as f32 / text.scale);
        // Captions at 2160p: the pixel limit lowers the scale, rounding must not cut the last row.
        let mut title = style("Inter");
        title.font_size = 300.0;
        title.stroke_width = 0.0;
        title.background = Some("#ffffffff".into());
        let exact = canvas_size(&renderer.render("BIG\nTITLE", &title, 1.0, 972.0));
        let export = renderer.render("BIG\nTITLE", &title, 8.0, 972.0 * 8.0);
        assert!(export.scale < 8.0);
        let coarse = canvas_size(&export);
        assert!(coarse.0 >= exact.0 && coarse.1 >= exact.1, "{coarse:?} vs {exact:?}");
        assert!(coarse.0 - exact.0 < 1.0 && coarse.1 - exact.1 < 1.0, "{coarse:?} vs {exact:?}");
        // A valid 8K title keeps all three lines.
        title.font_size = 15360.0;
        title.background = None;
        let lines = renderer.render("A\nA\nA", &title, 1.0, 6912.0);
        assert!(canvas_size(&lines).1 > 3.0 * 15360.0 * 1.2, "{:?}", canvas_size(&lines));
        // So do 1100 lines of tiny text that fit the same canvas.
        title.font_size = 5.0;
        let lines = renderer.render(&"A\n".repeat(1100), &title, 1.0, 6912.0);
        assert!(canvas_size(&lines).1 > 1100.0 * 5.0 * 1.2, "{:?}", canvas_size(&lines));
    }

    #[test]
    fn legacy_text_styles_render_with_bounded_allocations() {
        let mut renderer = bundled_renderer();
        let mut canvas = crate::Project::new("old project").canvas;
        canvas.width = 64;
        canvas.height = 64;
        for value in [f32::MAX, f32::INFINITY, f32::NAN, -1.0] {
            let mut old = style("Inter");
            old.font_size = value;
            old.stroke_width = value;
            old.max_width = Some(value);
            old.letter_spacing = value;
            old.line_spacing = value;
            old.shadow =
                Some(TextShadow { color: "#€".into(), opacity: value, blur: value, distance: value, angle: value });
            let bounded = old.bounded(&canvas);
            let image = renderer.render("Old title", &bounded, 1.0, bounded.max_width.unwrap()).image;
            assert!(image.width > 0 && image.height > 0);
            assert_eq!(image.data.len(), image.width as usize * image.height as usize * 4);
            assert!(image.data.len() <= MAX_RASTER_PIXELS * 4);
        }
        // Direct renderer callers are protected too, including malformed scales and many lines.
        let mut old = style("Inter");
        old.font_size = f32::MAX;
        old.stroke_width = f32::MAX;
        let image = renderer.render("A", &old, 1.0, f32::MAX).image;
        assert!(image.data.len() <= MAX_RASTER_PIXELS * 4);
        let image = renderer.render(&"line\n".repeat(1000), &style("Inter"), f32::NAN, f32::INFINITY).image;
        assert!(image.data.len() <= MAX_RASTER_PIXELS * 4);
        old.shadow =
            Some(TextShadow { color: "#000".into(), opacity: 1.0, blur: f32::MAX, distance: f32::MAX, angle: 1e30 });
        old.letter_spacing = f32::MAX;
        old.line_spacing = f32::MIN;
        let image = renderer.render(&"line\n".repeat(1000), &old, 8.0, f32::MAX).image;
        assert!(image.data.len() <= MAX_RASTER_PIXELS * 4);
    }

    /// The creator's CapCut caption look as the importer maps it: size 12, #fff059, a black shadow at 41 %,
    /// Blur 23 %, Distance 5, -45°.
    fn reel_look() -> TextStyle {
        let size = 12.0 * 5.1;
        TextStyle {
            font_size: size,
            color: "#fff059".into(),
            stroke_width: 0.0,
            shadow: Some(TextShadow {
                color: "#000000".into(),
                opacity: 0.41,
                blur: 23.0,
                distance: 5.0,
                angle: -45.0,
            }),
            ..style("Inter")
        }
    }

    /// Where the pixels `keep` picks lie on average, weighted by their alpha, in canvas pixels from the
    /// image's centre, and how much of them there is.
    fn centre(text: &TextImage, keep: impl Fn(&[u8; 4]) -> bool) -> (f32, f32, f32) {
        let w = text.image.width as usize;
        let (mut x, mut y, mut total) = (0.0, 0.0, 0.0);
        for (i, px) in text.image.data.as_chunks::<4>().0.iter().enumerate().filter(|(_, px)| keep(px)) {
            let a = px[3] as f32;
            x += (i % w) as f32 * a;
            y += (i / w) as f32 * a;
            total += a;
        }
        let at = |sum: f32, side: u32| (sum / total.max(1.0) - side as f32 / 2.0) / text.scale;
        (at(x, text.image.width), at(y, text.image.height), total)
    }

    fn dark(px: &[u8; 4]) -> bool {
        px[3] > 0 && u32::from(px[0]) + u32::from(px[1]) + u32::from(px[2]) < 120
    }

    fn bright(px: &[u8; 4]) -> bool {
        px[3] > 200 && px[0] > 200 && px[1] > 200
    }

    #[test]
    fn the_reel_shadow_falls_down_and_right_under_the_text_without_moving_it() {
        let mut renderer = bundled_renderer();
        let look = reel_look();
        let plain = TextStyle { shadow: None, ..look.clone() };
        // As a layer scaled 152 % is drawn.
        let (with, without) =
            (renderer.render("Ahoj světe", &look, 2.0, 2000.0), renderer.render("Ahoj světe", &plain, 2.0, 2000.0));
        assert_eq!(centre(&without, dark).2, 0.0, "no shadow without one");
        assert!(centre(&with, dark).2 > 0.0, "the shadow shows beside the letters");
        // Letters filled with nothing leave the shadow alone. CapCut's -45° is down and to the right.
        let ghost =
            renderer.render("Ahoj světe", &TextStyle { color: "#fff05900".into(), ..look.clone() }, 2.0, 2000.0);
        let (sx, sy, _) = centre(&ghost, |p| p[3] > 0);
        let (tx, ty, _) = centre(&without, |p| p[3] > 0);
        let away = 0.05 * look.font_size * std::f32::consts::FRAC_1_SQRT_2;
        assert!((sx - tx - away).abs() < 0.6 && (sy - ty - away).abs() < 0.6, "{:?}, not {away}", (sx - tx, sy - ty));
        // The letters stay where they were: the image grows by the shadow's reach on every side.
        let (lx, ly, _) = centre(&with, bright);
        let (px, py, _) = centre(&without, bright);
        assert!((px - lx).abs() < 0.5 && (py - ly).abs() < 0.5, "{:?} vs {:?}", (px, py), (lx, ly));
        // Not clipped: the image's edges stay clear.
        let (w, h) = (with.image.width as usize, with.image.height as usize);
        let px = with.image.data.as_chunks::<4>().0;
        let edges = (0..w).flat_map(|x| [x, (h - 1) * w + x]).chain((0..h).flat_map(|y| [y * w, y * w + w - 1]));
        assert!(edges.into_iter().all(|i| px[i][3] == 0), "the shadow reaches the edge");
        // At 41 % black, the shadow alone is never darker than that.
        assert!(px.iter().filter(|p| dark(p)).all(|p| p[3] <= 105), "too dark");
    }

    #[test]
    fn a_sharp_shadow_is_the_letters_moved_by_its_distance_in_its_direction() {
        let mut renderer = bundled_renderer();
        let mut look = style("Inter");
        look.stroke_width = 0.0;
        let bounds = |text: &TextImage, keep: fn(&[u8; 4]) -> bool| {
            let w = text.image.width as usize;
            let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
            for (i, _) in text.image.data.as_chunks::<4>().0.iter().enumerate().filter(|(_, px)| keep(px)) {
                (x0, y0, x1, y1) = (x0.min(i % w), y0.min(i / w), x1.max(i % w), y1.max(i / w));
            }
            [x0, y0, x1, y1].map(|v| v as f32)
        };
        let red = |px: &[u8; 4]| px[3] > 200 && px[0] > 200 && px[1] < 60;
        // Distance 20 is a fifth of the font size: 19 pixels at 95 and twice that when the text doubles.
        for size in [95.0, 190.0] {
            look.font_size = size;
            let far = 0.2 * size;
            let diagonal = far * std::f32::consts::FRAC_1_SQRT_2;
            for (angle, (dx, dy)) in [(90.0, (0.0, -far)), (180.0, (-far, 0.0)), (-45.0, (diagonal, diagonal))] {
                look.shadow =
                    Some(TextShadow { color: "#ff0000".into(), opacity: 1.0, blur: 0.0, distance: 20.0, angle });
                let text = renderer.render("H", &look, 1.0, 1000.0);
                let (letters, shadow) = (bounds(&text, bright), bounds(&text, red));
                // Where the letters cover it, the shadow does not show: compare the far edges.
                let far_x = if dx < 0.0 { shadow[0] - letters[0] } else { shadow[2] - letters[2] };
                let far_y = if dy < 0.0 { shadow[1] - letters[1] } else { shadow[3] - letters[3] };
                assert!(
                    (far_x - dx).abs() <= 1.5 && (far_y - dy).abs() <= 1.5,
                    "{size} {angle}°: {:?}",
                    (far_x, far_y)
                );
            }
        }
    }

    #[test]
    fn a_blurred_shadow_softens_over_a_share_of_the_font_size() {
        let mut renderer = bundled_renderer();
        let mut look = style("Inter");
        look.stroke_width = 0.0;
        // How far the shadow reaches beyond the letters' left edge, at no distance: its blur alone.
        let reach = |renderer: &mut TextRenderer, look: &TextStyle| {
            let text = renderer.render("H", look, 1.0, 4000.0);
            let w = text.image.width as usize;
            let px = text.image.data.as_chunks::<4>().0;
            let first =
                |keep: &dyn Fn(&[u8; 4]) -> bool| (0..w).find(|&x| px.iter().skip(x).step_by(w).any(keep)).unwrap();
            (first(&bright) - first(&|p: &[u8; 4]| p[3] > 0)) as f32
        };
        let mut reaches = Vec::new();
        for size in [95.0, 190.0] {
            look.font_size = size;
            look.shadow =
                Some(TextShadow { color: "#000000".into(), opacity: 1.0, blur: 40.0, distance: 0.0, angle: 0.0 });
            reaches.push(reach(&mut renderer, &look));
        }
        // Blur 40 % softens over a fifth of the font size: about 19 pixels at 95, 38 at 190.
        assert!((reaches[0] - 19.0).abs() <= 3.0 && (reaches[1] - 38.0).abs() <= 4.0, "{reaches:?}");
    }

    #[test]
    fn spaced_letters_do_not_join_into_ligatures() {
        let mut renderer = bundled_renderer();
        let mut look = style("Roboto");
        let glyphs =
            |renderer: &mut TextRenderer, look: &TextStyle| renderer.lay_out("office", look, 1.0, 4000.0).glyphs.len();
        // Roboto joins "ffi" into one glyph, which letter spacing would leave glued.
        assert!(glyphs(&mut renderer, &look) < 6);
        for spacing in [0.2, -0.1] {
            look.letter_spacing = spacing;
            assert_eq!(glyphs(&mut renderer, &look), 6, "{spacing}");
        }
    }

    #[test]
    fn letters_pulled_together_stay_inside_the_image() {
        let mut renderer = bundled_renderer();
        let mut look = style("Inter");
        for spacing in [-0.5, -0.3, -0.1] {
            look.letter_spacing = spacing;
            let text = renderer.render("Hello", &look, 1.0, 4000.0);
            let (w, h) = (text.image.width as usize, text.image.height as usize);
            let px = text.image.data.as_chunks::<4>().0;
            let edges = (0..w).flat_map(|x| [x, (h - 1) * w + x]).chain((0..h).flat_map(|y| [y * w, y * w + w - 1]));
            assert!(edges.into_iter().all(|i| px[i][3] == 0), "{spacing}: the letters reach the edge");
        }
    }

    #[test]
    fn letter_spacing_widens_the_text_and_line_spacing_moves_its_lines_apart() {
        let mut renderer = bundled_renderer();
        let mut look = style("Inter");
        let size = |renderer: &mut TextRenderer, text: &str, look: &TextStyle| {
            let image = renderer.render(text, look, 1.0, 4000.0);
            (image.image.width as f32, image.image.height as f32)
        };
        let tight = size(&mut renderer, "Hello", &look);
        look.letter_spacing = 0.2;
        let wide = size(&mut renderer, "Hello", &look);
        // Four gaps between five letters, none after the last.
        assert!((wide.0 - tight.0 - 4.0 * 0.2 * 95.0).abs() <= 2.0, "{tight:?} -> {wide:?}");
        assert_eq!(wide.1, tight.1);
        look.letter_spacing = 0.0;
        let lines = size(&mut renderer, "A\nA", &look);
        let line = size(&mut renderer, "A", &look);
        look.line_spacing = 0.5;
        let apart = size(&mut renderer, "A\nA", &look);
        assert!((apart.1 - lines.1 - 0.5 * 95.0).abs() <= 1.0, "{lines:?} -> {apart:?}");
        // One line has no line to keep apart from.
        assert_eq!(size(&mut renderer, "A", &look), line);
    }

    /// The leftmost and rightmost drawn column of each band of rows that has ink, top to bottom.
    fn line_edges(text: &TextImage) -> Vec<(usize, usize)> {
        let (w, h) = (text.image.width as usize, text.image.height as usize);
        let ink = |x: usize, y: usize| text.image.data[(y * w + x) * 4 + 3] > 128;
        let mut lines = Vec::new();
        let mut current: Option<(usize, usize)> = None;
        for y in 0..h {
            let row = (0..w).filter(|&x| ink(x, y));
            match (row.clone().min().zip(row.max()), &mut current) {
                (Some((l, r)), Some((cl, cr))) => (*cl, *cr) = ((*cl).min(l), (*cr).max(r)),
                (Some(edges), None) => current = Some(edges),
                (None, Some(_)) => lines.extend(current.take()),
                (None, None) => {}
            }
        }
        lines.extend(current);
        lines
    }

    #[test]
    fn alignment_lines_the_shorter_line_up_with_the_longer_one() {
        let mut renderer = bundled_renderer();
        let text = "Hi\nHello world";
        let edges = |renderer: &mut TextRenderer, align| {
            let look = TextStyle { align, ..style("Inter") };
            line_edges(&renderer.render(text, &look, 1.0, 4000.0))
        };
        let [short, long] = edges(&mut renderer, TextAlign::Left)[..] else { panic!("two lines") };
        assert!(short.0.abs_diff(long.0) <= 6 && long.1 - short.1 > 200, "left: {short:?} {long:?}");
        let [short, long] = edges(&mut renderer, TextAlign::Right)[..] else { panic!("two lines") };
        assert!(short.1.abs_diff(long.1) <= 6 && short.0 - long.0 > 200, "right: {short:?} {long:?}");
        let [short, long] = edges(&mut renderer, TextAlign::Center)[..] else { panic!("two lines") };
        let middle = |(l, r): (usize, usize)| (l + r) as f32 / 2.0;
        assert!((middle(short) - middle(long)).abs() <= 6.0, "centre: {short:?} {long:?}");
        // The widest line sets the image, so alignment never changes its size.
        let size = |renderer: &mut TextRenderer, align| {
            let image = renderer.render(text, &TextStyle { align, ..style("Inter") }, 1.0, 4000.0).image;
            (image.width, image.height)
        };
        assert_eq!(size(&mut renderer, TextAlign::Left), size(&mut renderer, TextAlign::Center));
        assert_eq!(size(&mut renderer, TextAlign::Right), size(&mut renderer, TextAlign::Center));
    }

    #[test]
    fn box_corners_go_from_square_to_fully_round() {
        let mut renderer = bundled_renderer();
        let boxed = |radius| TextStyle { background: Some("#0000ffff".into()), box_radius: radius, ..style("Inter") };
        let corner = |renderer: &mut TextRenderer, radius| {
            let image = renderer.render("Ahoj", &boxed(radius), 1.0, 4000.0).image;
            // Alpha of the top left pixel and of one a quarter of the box's height in along its top edge.
            let in_from_corner = image.height as usize / 4;
            (image.data[3], image.data[in_from_corner * 4 + 3], image.width, image.height)
        };
        let square = corner(&mut renderer, 0.0);
        let usual = corner(&mut renderer, BOX_RADIUS);
        let round = corner(&mut renderer, 1.0);
        assert_eq!((square.0, square.1), (255, 255), "a square box fills its corner");
        assert!(usual.0 < 64 && usual.1 == 255, "the usual radius rounds only the corner: {usual:?}");
        assert!(round.0 == 0 && round.1 < 128, "a full radius rounds a line's box into a pill: {round:?}");
        assert_eq!((square.2, square.3), (round.2, round.3), "corners never change the box's size");
    }

    #[test]
    fn letter_case_changes_what_is_drawn_and_karaoke_still_finds_its_words() {
        let mut renderer = bundled_renderer();
        let mut look = style("Inter");
        let drawn = |renderer: &mut TextRenderer, text: &str, look: &TextStyle| {
            renderer.render(text, look, 1.0, 4000.0).image.data
        };
        let typed = drawn(&mut renderer, "Ahoj světe, (ahoj)", &look);
        for (case, shown) in [
            (LetterCase::Upper, "AHOJ SVĚTE, (AHOJ)"),
            (LetterCase::Lower, "ahoj světe, (ahoj)"),
            (LetterCase::Title, "Ahoj Světe, (Ahoj)"),
        ] {
            look.letter_case = case;
            let cased = drawn(&mut renderer, "Ahoj světe, (ahoj)", &look);
            look.letter_case = LetterCase::None;
            assert_eq!(cased, drawn(&mut renderer, shown, &look), "{case:?}");
            assert_ne!(cased, typed, "{case:?}");
        }
        // "ﬁ" is one character of three bytes and "FI" two of one: the spoken word "it" sits at other bytes
        // once drawn, and still lights up.
        look.highlight = Some("#00ff00".into());
        look.letter_case = LetterCase::Upper;
        let lit = renderer.render_spoken("ﬁx it", &look, 1.0, 4000.0, Some(5..7), &[]).image.data;
        look.letter_case = LetterCase::None;
        let wanted = renderer.render_spoken("FIX IT", &look, 1.0, 4000.0, Some(4..6), &[]).image.data;
        assert_eq!(lit, wanted);
    }

    #[test]
    fn box_outline_and_karaoke_keep_working_over_a_shadow() {
        let mut renderer = bundled_renderer();
        let mut look = reel_look();
        look.stroke_width = 6.0;
        look.background = Some("#0000ffff".into());
        look.highlight = Some("#00ff00".into());
        let text = renderer.render_spoken("Ahoj světe", &look, 1.0, 4000.0, Some(0..4), &[]);
        let (w, h) = (text.image.width as usize, text.image.height as usize);
        let px = text.image.data.as_chunks::<4>().0;
        let green = |p: &[u8; 4]| p[1] > 200 && p[0] < 60 && p[2] < 60;
        let lit: Vec<usize> = (0..px.len()).filter(|&i| green(&px[i])).map(|i| i % w).collect();
        assert!(!lit.is_empty() && lit.iter().all(|&x| x < w / 2), "only the spoken first word lights up");
        // The box sits inside the room the shadow keeps around it, under the text's middle row.
        let room = (0.05 * look.font_size / 2.0_f32.sqrt() + 0.23 * look.font_size / 2.0).ceil() as usize;
        assert_eq!(px[h / 2 * w][3], 0, "the shadow's room stays clear");
        assert_eq!(px[h / 2 * w + room + 3], [0, 0, 255, 255], "the box");
    }

    #[test]
    fn manifest_matches_embedded_fonts() {
        let files: BTreeSet<_> = FONT_MANIFEST.iter().map(|face| face.file.as_str()).collect();
        assert_eq!(files.len(), FONT_MANIFEST.len());
        assert_eq!(files, FONT_DATA.iter().map(|(file, _)| *file).collect());
        assert!(FONT_MANIFEST.iter().all(|face| !face.weight.is_empty()));
    }

    fn bundled_renderer() -> TextRenderer {
        let mut db = cosmic_text::fontdb::Database::new();
        for data in BUNDLED_FONTS.iter() {
            db.load_font_data(data.to_vec());
        }
        db.set_sans_serif_family("Inter");
        let fonts = FontSystem::new_with_locale_and_db("cs-CZ".into(), db);
        let families = fonts.db().faces().flat_map(|face| face.families.iter().map(|(name, _)| name.clone())).collect();
        TextRenderer {
            fonts,
            families,
            bold: HashMap::new(),
            swash: SwashCache::new(),
            cache: HashMap::new(),
            layouts: HashMap::new(),
            stats: TextStats::default(),
        }
    }

    #[test]
    fn bundled_fonts_cover_and_render_czech_without_fallback() {
        let mut renderer = bundled_renderer();
        for face in renderer.fonts.db().faces() {
            let covered = renderer.fonts.db().with_face_data(face.id, |data, index| {
                let font = FontRef::from_index(data, index).unwrap();
                let cmap = font.charmap();
                CZECH.chars().all(|c| cmap.map(c).is_some_and(|id| id.to_u32() != 0))
            });
            assert_eq!(covered, Some(true), "Missing Czech glyphs in {:?}", face.families);
        }
        for family in BUNDLED_FONT_FAMILIES.iter().copied() {
            let image = renderer.render("Příliš žluťoučký kůň", &style(family), 1.0, 1080.0).image;
            assert!(image.data.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0), "Empty {family}");
            let mut buffer = Buffer::new(&mut renderer.fonts, Metrics::new(95.0, 114.0));
            buffer.set_size(Some(4000.0), None);
            buffer.set_text(CZECH, &Attrs::new().family(Family::Name(family)), Shaping::Advanced, None);
            buffer.shape_until_scroll(&mut renderer.fonts, false);
            for run in buffer.layout_runs() {
                for glyph in run.glyphs {
                    assert_ne!(glyph.glyph_id, 0, "Missing glyph in {family}");
                    let face = renderer.fonts.db().face(glyph.font_id).unwrap();
                    assert!(face.families.iter().any(|(name, _)| name == family), "Fallback in {family}");
                }
            }
        }
    }

    #[test]
    fn bundled_variable_and_static_bold_weights_render_differently() {
        let mut renderer = bundled_renderer();
        for family in ["Inter", "Lexend", "Montserrat", "Oswald", "Poppins", "Roboto"] {
            let mut style = style(family);
            style.stroke_width = 0.0;
            let regular = renderer.render("Příliš žluťoučký kůň", &style, 1.0, 2000.0).image;
            style.bold = true;
            let bold = renderer.render("Příliš žluťoučký kůň", &style, 1.0, 2000.0).image;
            let coverage = |image: &Image| image.data.as_chunks::<4>().0.iter().map(|p| u64::from(p[3])).sum::<u64>();
            assert!(coverage(&bold) > coverage(&regular), "Bold did not increase ink coverage in {family}");
        }
    }

    #[test]
    fn single_weight_families_stay_themselves_when_bold() {
        // With installed fonts present, cosmic-text would otherwise pick another family's bold.
        let mut renderer = TextRenderer::new();
        for family in ["Anton", "Bebas Neue"] {
            let mut style = style(family);
            let regular = renderer.render("Příliš žluťoučký kůň", &style, 1.0, 2000.0).image;
            style.bold = true;
            assert_eq!(
                renderer.render("Příliš žluťoučký kůň", &style, 1.0, 2000.0).image.data,
                regular.data,
                "{family}"
            );
        }
    }

    #[test]
    fn family_selection_cache_fallback_and_listing() {
        let mut renderer = TextRenderer::new();
        let families = renderer.font_families();
        assert_eq!(&families[..BUNDLED_FONT_FAMILIES.len()], BUNDLED_FONT_FAMILIES.as_slice());
        assert_eq!(families.iter().collect::<BTreeSet<_>>().len(), families.len());
        assert!(families[BUNDLED_FONT_FAMILIES.len()..].windows(2).all(|pair| pair[0] < pair[1]));
        for family in &families {
            assert_eq!(renderer.family(Some(family)), Family::Name(family));
        }
        let inter = renderer.render("Příliš žluťoučký kůň", &style("Inter"), 1.0, 2000.0).image;
        let anton = renderer.render("Příliš žluťoučký kůň", &style("Anton"), 1.0, 2000.0).image;
        assert_ne!(inter.data, anton.data);
        let missing = renderer.render("Ahoj", &style("Nuzky nonexistent font"), 1.0, 1000.0).image;
        let mut default = style("Inter");
        default.font_family = None;
        let fallback = renderer.render("Ahoj", &default, 1.0, 1000.0).image;
        assert_eq!(missing.data, fallback.data);
    }
}

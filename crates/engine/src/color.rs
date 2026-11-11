//! Camera log profiles turned into normal colour, and 3D LUTs read from `.cube` files.
//!
//! The compositor draws both (`gpu.rs`: `log_to_display`, `lut3d`) as `Conversion::apply` and `Lut3d::sample` do
//! here, which the Filters tab's tiles use, so a tile shows what the preview and the export do.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::model::{ClipContent, LogProfile, Lut, Project};

/// `size`³ RGB entries over 0..1 in, red changing fastest, then green, then blue, as `.cube` files list them.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut3d {
    pub size: usize,
    pub data: Vec<[f32; 3]>,
}

impl Lut3d {
    pub fn from_fn(size: usize, f: impl Fn([f32; 3]) -> [f32; 3]) -> Self {
        let step = 1.0 / (size - 1) as f32;
        let mut data = Vec::with_capacity(size * size * size);
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    data.push(f([r as f32 * step, g as f32 * step, b as f32 * step]));
                }
            }
        }
        Self { size, data }
    }

    /// Trilinear, the way the compositor's `lut3d` samples it.
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        let n = self.size;
        let mut i = [0usize; 3];
        let mut f = [0f32; 3];
        for c in 0..3 {
            let p = rgb[c].clamp(0.0, 1.0) * (n - 1) as f32;
            i[c] = (p.floor() as usize).min(n - 2);
            f[c] = p - i[c] as f32;
        }
        let at = |r: usize, g: usize, b: usize| self.data[r + n * (g + n * b)];
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| -> [f32; 3] { std::array::from_fn(|c| a[c] + (b[c] - a[c]) * t) };
        let [r, g, b] = i;
        let row = |g: usize, b: usize| lerp(at(r, g, b), at(r + 1, g, b), f[0]);
        let plane = |b: usize| lerp(row(g, b), row(g + 1, b), f[1]);
        lerp(plane(b), plane(b + 1), f[2])
    }

    /// The table over inputs 0..1 on a grid of `size`, where this one's grid spans each channel's `domain` (min, max).
    fn resampled(&self, size: usize, [min, max]: [[f32; 3]; 2]) -> Self {
        Self::from_fn(size, |rgb| self.sample(std::array::from_fn(|c| (rgb[c] - min[c]) / (max[c] - min[c]))))
    }
}

/// The largest `.cube` file Nuzky reads; a 129-point LUT, the finest common size, takes about 55 MB.
pub const MAX_CUBE_BYTES: u64 = 64 << 20;
/// The most points per side a LUT is drawn with; a finer one is resampled to this.
pub const MAX_DRAWN_SIZE: usize = 65;
const MAX_CUBE_SIZE: usize = 256;

/// Reads a `.cube` file's 3D LUT (Adobe's Cube LUT Specification 1.0, as Resolve, Premiere and CapCut write it).
/// Errors say in plain words why the file cannot be used.
pub fn parse_cube(text: &str) -> Result<Lut3d, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut size = None;
    let mut domain = [[0f32; 3], [1f32; 3]];
    let mut data: Vec<[f32; 3]> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let number = index + 1;
        let unreadable = || {
            format!(
                "Line {number} isn't three numbers (“{}”), so the file is damaged or isn't a .cube LUT.",
                clip(line)
            )
        };
        let numbers = |words: std::str::SplitAsciiWhitespace<'_>| -> Result<Vec<f32>, String> {
            words.map(|w| w.parse::<f32>().ok().filter(|v| v.is_finite())).collect::<Option<_>>().ok_or_else(unreadable)
        };
        let mut words = line.split_ascii_whitespace();
        let keyword = words.next().unwrap_or_default();
        if keyword.parse::<f32>().is_ok() {
            let [r, g, b] = numbers(line.split_ascii_whitespace())?[..] else { return Err(unreadable()) };
            data.push([r, g, b]);
            continue;
        }
        if !data.is_empty() {
            return Err(unreadable());
        }
        match keyword {
            "LUT_1D_SIZE" => {
                return Err(if text.contains("LUT_3D_SIZE") {
                    "It has a 1D shaper before its 3D table, which Nuzky can't apply. Export it again as a plain 3D LUT.".into()
                } else {
                    "It's a 1D LUT, and Nuzky applies 3D LUTs. Export it again as a 3D LUT.".into()
                });
            }
            "LUT_3D_SIZE" => {
                let n = words.next().and_then(|w| w.parse::<usize>().ok()).unwrap_or(0);
                if !(2..=MAX_CUBE_SIZE).contains(&n) {
                    return Err(format!(
                        "Its size, “{}”, isn't one Nuzky reads: from 2 to {MAX_CUBE_SIZE} points per side.",
                        clip(line)
                    ));
                }
                size = Some(n);
            }
            "DOMAIN_MIN" | "DOMAIN_MAX" => {
                let [r, g, b] = numbers(words)?[..] else {
                    return Err(format!("Its {keyword} isn't three numbers, so the file is damaged."));
                };
                domain[(keyword == "DOMAIN_MAX") as usize] = [r, g, b];
            }
            "LUT_3D_INPUT_RANGE" => {
                let [lo, hi] = numbers(words)?[..] else {
                    return Err("Its LUT_3D_INPUT_RANGE isn't two numbers, so the file is damaged.".into());
                };
                domain = [[lo; 3], [hi; 3]];
            }
            // TITLE and keywords of other programs, such as LUT_IN_VIDEO_RANGE, come before the table; the
            // specification lets a reader pass over them.
            _ if keyword.starts_with(|c: char| c.is_ascii_uppercase()) => {}
            _ => return Err(unreadable()),
        }
    }
    let Some(size) = size else {
        return Err(if data.is_empty() {
            "The file holds no LUT. Export the LUT again as a 3D .cube file.".into()
        } else {
            "It doesn't say its size, so it isn't a 3D .cube LUT. Export the LUT again as a 3D .cube file.".into()
        });
    };
    if data.len() != size.pow(3) {
        return Err(format!(
            "The file is cut short or damaged: it has {} of the {} colours its size needs. Export or download it again.",
            data.len(),
            size.pow(3)
        ));
    }
    if (0..3).any(|c| domain[0][c] >= domain[1][c]) {
        return Err("Its input range runs backwards, so the file is damaged.".into());
    }
    let lut = Lut3d { size, data };
    let unit = domain == [[0.0; 3], [1.0; 3]];
    // A coarse table over another domain is drawn on the finest grid, so its corners keep their place.
    Ok(if unit && size <= MAX_DRAWN_SIZE { lut } else { lut.resampled(MAX_DRAWN_SIZE, domain) })
}

fn clip(line: &str) -> String {
    let short: String = line.chars().take(40).collect();
    if short.len() < line.len() { format!("{short}…") } else { short }
}

/// Reads a `.cube` file of at most `MAX_CUBE_BYTES`.
pub fn read_cube(path: &Path) -> Result<Lut3d, String> {
    parse_cube(&read_cube_text(path)?)
}

/// The text of a `.cube` file of at most `MAX_CUBE_BYTES`.
pub fn read_cube_text(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("Couldn't open the LUT: {e}."))?;
    let mut bytes = Vec::new();
    file.take(MAX_CUBE_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("Couldn't read the LUT: {e}."))?;
    if bytes.len() as u64 > MAX_CUBE_BYTES {
        return Err(format!("The file is larger than {} MB, more than any .cube LUT.", MAX_CUBE_BYTES >> 20));
    }
    String::from_utf8(bytes).map_err(|_| "This file isn't text, so it isn't a .cube LUT.".to_owned())
}

/// Where Nuzky keeps the LUTs the user imported, each as `<id>.cube`.
pub fn luts_dir() -> Option<PathBuf> {
    crate::data_dir().map(|dir| dir.join("luts"))
}

/// A LUT's id: the first 16 hex digits of its file's SHA-256, so it never names a path.
pub fn is_lut_id(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn lut_file(dir: &Path, id: &str) -> Option<PathBuf> {
    is_lut_id(id).then(|| dir.join(format!("{id}.cube")))
}

/// The names of the project's LUTs that are not in `dir`, which it plays and exports without.
pub fn missing_luts(project: &Project, dir: Option<&Path>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for clip in project.tracks.iter().flat_map(|t| &t.clips) {
        if let ClipContent::Media { lut: Lut::File { id, name, .. }, .. } = &clip.content {
            let there = dir.and_then(|dir| lut_file(dir, id)).is_some_and(|file| file.is_file());
            if !there && !names.contains(name) {
                names.push(name.clone());
            }
        }
    }
    names
}

/// What an export says when it went without LUTs `missing_luts` named.
pub fn exported_without_luts(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("“{n}”")).collect();
    let listed = match quoted.as_slice() {
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        [] => String::new(),
    };
    let (lut, it) = if names.len() == 1 { ("LUT", "it isn't") } else { ("LUTs", "they aren't") };
    format!("Exported without the {lut} {listed}: {it} on this computer. Import the .cube file in Filters to use it.")
}

type Matrix = [[f64; 3]; 3];
type Primaries = [[f64; 2]; 3];

const D65: [f64; 2] = [0.3127, 0.3290];
const BT709: Primaries = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]];
const BT2020: Primaries = [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]];
const LUMA_709: [f64; 3] = [0.2126, 0.7152, 0.0722];

/// The display rendering: scene light x, 0.18 being 18 % grey, becomes display light m·x^p / (x^p + c), encoded for
/// a BT.1886 screen like other SDR video. It puts 18 % grey at 0.44 and 90 % white at 0.88, about where camera
/// makers' Log to Rec.709 conversions put them, follows the BT.709 camera curve in the shadows and rolls light up
/// to 16 times grey's into white instead of clipping it.
const TONE_POWER: f64 = 1.763696;
const TONE_KNEE: f64 = 0.300731;
const TONE_GAIN: f64 = 1.002262;

/// How a profile's signal becomes scene light, and the primaries of its colours, all with a D65 white.
struct Profile {
    decode: fn(f64) -> f64,
    /// The curve is published in 10-bit code values, which cameras record in limited range, so the signal FFmpeg
    /// decodes (0 at code 64, 1 at code 940) becomes code / 1023 before the curve.
    legal: bool,
    primaries: Primaries,
}

fn profile(log: LogProfile) -> Option<Profile> {
    use LogProfile::*;
    const S_GAMUT: Primaries = [[0.730, 0.280], [0.140, 0.855], [0.100, -0.050]];
    const CINEMA_GAMUT: Primaries = [[0.740, 0.270], [0.170, 1.140], [0.080, -0.100]];
    let (decode, legal, primaries): (fn(f64) -> f64, bool, Primaries) = match log {
        None => return Option::None,
        AppleLog => (apple_log, false, BT2020),
        AppleLog2 => (apple_log, false, [[0.725, 0.301], [0.221, 0.814], [0.068, -0.076]]),
        XiaomiLog => (xiaomi_log, false, BT2020),
        SLog3Cine => (s_log3, true, [[0.766, 0.275], [0.225, 0.800], [0.089, -0.087]]),
        SLog3 => (s_log3, true, S_GAMUT),
        SLog2 => (s_log2, false, S_GAMUT),
        CanonLog3 => (canon_log3, true, CINEMA_GAMUT),
        CanonLog2 => (canon_log2, true, CINEMA_GAMUT),
        VLog => (v_log, true, [[0.730, 0.280], [0.165, 0.840], [0.100, -0.030]]),
        FLog => (f_log, true, BT2020),
        FLog2 => (f_log2, true, BT2020),
        NLog => (n_log, true, BT2020),
        LLog => (l_log, true, BT2020),
        DLog => (d_log, false, [[0.71, 0.31], [0.21, 0.88], [0.09, -0.08]]),
        BlackmagicFilm5 => {
            (blackmagic_film5, false, [[0.7177215, 0.3171181], [0.2280410, 0.8615690], [0.1005841, -0.0820452]])
        }
        LogC3 => (log_c3, true, [[0.6840, 0.3130], [0.2210, 0.8480], [0.0861, -0.1020]]),
        LogC4 => (log_c4, true, [[0.7347, 0.2653], [0.1424, 0.8576], [0.0991, -0.0308]]),
    };
    Some(Profile { decode, legal, primaries })
}

// The makers' published curves: Apple's Apple Log Profile (2023) and Apple Log 2 (2025) white papers, Xiaomi's Log
// Profile white paper (2024), Sony's S-Log3 and S-Log2 technical summaries, Canon Log 2 and 3 v1.2, Panasonic
// V-Log/V-Gamut, Fujifilm's F-Log and F-Log2 data sheets, Nikon N-Log, Leica's L-Log reference v1.4, DJI's
// D-Log/D-Gamut white paper, Blackmagic Film Generation 5 and ARRI LogC3 and LogC4, as colour-science implements
// them. Each takes the curve's own code value and gives scene light, 0.18 for 18 % grey.

fn apple_family(p: f64, [r0, rt, sigma, beta, gamma, delta]: [f64; 6]) -> f64 {
    if p >= sigma * (rt - r0).powi(2) {
        2f64.powf((p - delta) / gamma) - beta
    } else if p >= 0.0 {
        (p / sigma).sqrt() + r0
    } else {
        r0
    }
}

fn apple_log(p: f64) -> f64 {
    apple_family(p, [-0.05641088, 0.01, 47.28711236, 0.00964052, 0.08550479, 0.69336945])
}

fn xiaomi_log(p: f64) -> f64 {
    apple_family(p, [-0.09023729, 0.01974185, 18.10531998, 0.01384578, 0.09271529, 0.6729185])
}

fn s_log3(y: f64) -> f64 {
    if y >= 171.2102946929 / 1023.0 {
        10f64.powf((y * 1023.0 - 420.0) / 261.5) * 0.19 - 0.01
    } else {
        (y * 1023.0 - 95.0) * 0.01125 / (171.2102946929 - 95.0)
    }
}

/// S-Log2's constants are published on the 0..1 signal itself.
fn s_log2(x: f64) -> f64 {
    let black = 0.030001222851889303;
    let reflection = if x >= black { 10f64.powf((x - 0.646596) / 0.432699) - 0.037584 } else { (x - black) / 5.0 };
    reflection * 0.9 * 219.0 / 155.0
}

fn canon_log2(v: f64) -> f64 {
    let x = if v < 0.092864125 {
        -(10f64.powf((0.092864125 - v) / 0.24136077) - 1.0) / 87.09937546
    } else {
        (10f64.powf((v - 0.092864125) / 0.24136077) - 1.0) / 87.09937546
    };
    x * 0.9
}

fn canon_log3(v: f64) -> f64 {
    let x = if v < 0.097465473 {
        -(10f64.powf((0.12783901 - v) / 0.36726845) - 1.0) / 14.98325
    } else if v <= 0.15277891 {
        (v - 0.12512219) / 1.9754798
    } else {
        (10f64.powf((v - 0.12240537) / 0.36726845) - 1.0) / 14.98325
    };
    x * 0.9
}

fn v_log(v: f64) -> f64 {
    if v < 0.181 { (v - 0.125) / 5.6 } else { 10f64.powf((v - 0.598206) / 0.241514) - 0.00873 }
}

fn f_log_family(v: f64, [cut, a, b, c, d, e, f]: [f64; 7]) -> f64 {
    if v < cut { (v - f) / e } else { 10f64.powf((v - d) / c) / a - b / a }
}

fn f_log(v: f64) -> f64 {
    f_log_family(v, [0.100537775223865, 0.555556, 0.009468, 0.344676, 0.790453, 8.735631, 0.092864])
}

fn f_log2(v: f64) -> f64 {
    f_log_family(v, [0.100686685370811, 5.555556, 0.064829, 0.245281, 0.384316, 8.799461, 0.092864])
}

fn n_log(v: f64) -> f64 {
    if v < 452.0 / 1023.0 {
        (v / (650.0 / 1023.0)).powi(3) - 0.0075
    } else {
        ((v - 619.0 / 1023.0) / (150.0 / 1023.0)).exp()
    }
}

fn l_log(v: f64) -> f64 {
    if v <= 0.1380 { (v - 0.09) / 8.0 } else { (10f64.powf((v - 0.6) / 0.27) - 0.0115) / 1.3 }
}

fn d_log(y: f64) -> f64 {
    if y <= 0.14 { (y - 0.0929) / 6.025 } else { (10f64.powf(3.89616 * y - 2.27752) - 0.0108) / 0.9892 }
}

fn blackmagic_film5(y: f64) -> f64 {
    const A: f64 = 0.08692876065491224;
    const B: f64 = 0.005494072432257808;
    const C: f64 = 0.5300133392291939;
    const D: f64 = 8.283605932402494;
    const E: f64 = 0.09246575342465753;
    if y < D * 0.005 + E { (y - E) / D } else { ((y - C) / A).exp() - B }
}

fn log_c3(t: f64) -> f64 {
    let (cut, a, b, c, d, e, f) = (0.010591, 5.555556, 0.052272, 0.247190, 0.385537, 5.367655, 0.092809);
    if t > e * cut + f { (10f64.powf((t - d) / c) - b) / a } else { (t - f) / e }
}

fn log_c4(e: f64) -> f64 {
    let a = (2f64.powi(18) - 16.0) / 117.45;
    let b = (1023.0 - 95.0) / 1023.0;
    let c = 95.0 / 1023.0;
    let s = (7.0 * std::f64::consts::LN_2 * 2f64.powf(7.0 - 14.0 * c / b)) / (a * b);
    let t = (2f64.powf(14.0 * (-c / b) + 6.0) - 64.0) / a;
    if e >= 0.0 { (2f64.powf(14.0 * (e - c) / b + 6.0) - 64.0) / a } else { e * s + t }
}

/// RGB to CIE XYZ for primaries with a D65 white.
fn to_xyz(primaries: Primaries) -> Matrix {
    let xyz = |[x, y]: [f64; 2]| [x / y, 1.0, (1.0 - x - y) / y];
    let columns = primaries.map(xyz);
    let m: Matrix = std::array::from_fn(|row| std::array::from_fn(|col| columns[col][row]));
    let s = mul(invert(m), xyz(D65));
    std::array::from_fn(|row| std::array::from_fn(|col| m[row][col] * s[col]))
}

fn invert(m: Matrix) -> Matrix {
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    [
        [(e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det],
        [(f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det],
        [(d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det],
    ]
}

fn mul(m: Matrix, v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|row| m[row][0] * v[0] + m[row][1] * v[1] + m[row][2] * v[2])
}

fn rgb_to_rgb(from: Primaries, to: Primaries) -> Matrix {
    let (a, b) = (invert(to_xyz(to)), to_xyz(from));
    std::array::from_fn(|row| std::array::from_fn(|col| (0..3).map(|k| a[row][k] * b[k][col]).sum()))
}

/// Moves a colour with no channel below 0 towards its own BT.709 luma until no channel is above 1, keeping its hue
/// and luma, so a bright colour turns white instead of shifting hue; luma above 1 becomes white. The compositor's
/// `fit` does the same.
fn fit_bright(rgb: [f64; 3]) -> [f64; 3] {
    let y = (rgb[0] * LUMA_709[0] + rgb[1] * LUMA_709[1] + rgb[2] * LUMA_709[2]).min(1.0);
    let hi = rgb[0].max(rgb[1]).max(rgb[2]);
    let k = if hi > 1.0 { (1.0 - y) / (hi - y).max(1e-12) } else { 1.0 };
    rgb.map(|c| y + (c - y) * k)
}

/// The profile's conversion with its matrices worked out.
struct Conversion {
    profile: Profile,
    to_2020: Matrix,
    to_709: Matrix,
}

impl Conversion {
    fn new(log: LogProfile) -> Option<Self> {
        let profile = profile(log)?;
        Some(Self { to_2020: rgb_to_rgb(profile.primaries, BT2020), to_709: rgb_to_rgb(BT2020, BT709), profile })
    }

    /// Scene light of one channel's 0..1 signal.
    fn scene(&self, s: f32) -> f64 {
        let p = &self.profile;
        (p.decode)(if p.legal { (64.0 + 876.0 * s as f64) / 1023.0 } else { s as f64 })
    }

    /// The tone curve works on each channel in BT.2020, as ACES renders in its own wide primaries, so a colour the
    /// camera saw beyond BT.709 keeps changing smoothly as it brightens. Only then does it become BT.709: a colour
    /// the screen cannot show loses what lies below 0 in each channel, so a deep red stays red where pulling it
    /// towards its luma would add blue, and one above 1 turns white.
    fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let scene = mul(self.to_2020, rgb.map(|s| self.scene(s)));
        let display = scene.map(|x| {
            let x = x.max(0.0).powf(TONE_POWER);
            TONE_GAIN * x / (x + TONE_KNEE)
        });
        fit_bright(mul(self.to_709, display).map(|c| c.max(0.0))).map(|c| c.clamp(0.0, 1.0).powf(1.0 / 2.4) as f32)
    }
}

/// A picture recorded in `log`, 0..1 RGB as decoded, in normal colour for an SDR screen.
pub fn convert(log: LogProfile, rgb: [f32; 3]) -> [f32; 3] {
    Conversion::new(log).map_or(rgb, |c| c.apply(rgb))
}

/// What the compositor's `log_to_display` needs of a profile: its curve sampled over the 0..1 signal, and the
/// matrix from its primaries to BT.2020, as rows. A 3D LUT of the whole conversion would bend saturated blues by
/// up to 14 levels between its points at 65 per side; the curve alone, a smooth function of one channel, keeps
/// within a hundredth of a level at this many points.
pub struct LogConversion {
    pub curve: Vec<f32>,
    pub to_2020: [[f32; 3]; 3],
}

const CURVE_POINTS: usize = 4096;

/// The profile's `LogConversion`, made once per process.
pub fn log_conversion(log: LogProfile) -> Option<Arc<LogConversion>> {
    static MADE: OnceLock<Mutex<HashMap<LogProfile, Arc<LogConversion>>>> = OnceLock::new();
    let conversion = Conversion::new(log)?;
    let mut made = MADE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    let made = made.entry(log).or_insert_with(|| {
        let curve = (0..CURVE_POINTS).map(|i| conversion.scene(i as f32 / (CURVE_POINTS - 1) as f32) as f32).collect();
        Arc::new(LogConversion { curve, to_2020: conversion.to_2020.map(|row| row.map(|v| v as f32)) })
    });
    Some(made.clone())
}

/// A profile and then a LUT at its strength over straight RGBA, as the compositor applies them before the
/// adjustments; for the Filters tab's tiles.
pub fn apply_to_rgba(rgba: &mut [u8], log: LogProfile, lut: Option<(&Lut3d, f32)>) {
    let conversion = Conversion::new(log);
    for px in rgba.as_chunks_mut::<4>().0 {
        let mut rgb = [px[0], px[1], px[2]].map(|v| v as f32 / 255.0);
        if let Some(conversion) = &conversion {
            rgb = conversion.apply(rgb);
        }
        if let Some((lut, strength)) = lut {
            let graded = lut.sample(rgb);
            rgb = std::array::from_fn(|c| rgb[c] + (graded[c] - rgb[c]) * strength);
        }
        for c in 0..3 {
            px[c] = (rgb[c].clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use LogProfile::*;

    const ALL: [LogProfile; 17] = [
        AppleLog,
        AppleLog2,
        XiaomiLog,
        SLog3Cine,
        SLog3,
        SLog2,
        CanonLog3,
        CanonLog2,
        VLog,
        FLog,
        FLog2,
        NLog,
        LLog,
        DLog,
        BlackmagicFilm5,
        LogC3,
        LogC4,
    ];

    /// The signal a profile records for scene light `x`, found by bisection on its decoding, which rises.
    fn encode(log: LogProfile, x: f64) -> f32 {
        let (mut lo, mut hi) = (0f32, 1f32);
        let p = profile(log).unwrap();
        let decode = |s: f32| (p.decode)(if p.legal { (64.0 + 876.0 * s as f64) / 1023.0 } else { s as f64 });
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if decode(mid) < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo + hi) / 2.0
    }

    /// Scene light colour-science 0.4.7 decodes from the 0..1 signals 0.05, 0.2, 0.4, 0.6 and 0.9: Apple Log Profile,
    /// Mi-Log, D-Log and Blackmagic Film Generation 5 from the signal, ARRI LogC3 (EI 800) and LogC4 from
    /// `full_to_legal` of it, the others with `in_normalised_code_value=False` (Canon Log v1.2).
    #[test]
    fn curves_decode_as_colour_science_does() {
        #[rustfmt::skip]
        let cases: [(LogProfile, [f64; 5]); 15] = [
            (AppleLog, [-0.023893645340279858, 0.00862358931944028, 0.08307602266464743, 0.4594768499747651, 5.329458082811693]),
            (XiaomiLog, [-0.03768617926171559, 0.014864931476568824, 0.11613596116389324, 0.5659120829598417, 5.44741724246513]),
            (SLog3, [0.0018895085051208397, 0.02866854290595098, 0.1708584010601357, 0.8359010548596323, 8.546396193613893]),
            (SLog2, [0.005366676672291775, 0.07030444130835939, 0.2945440263203573, 0.944564976711075, 4.84989065625535]),
            (CanonLog3, [-0.00899592155919824, 0.0607149699872551, 0.2933703251278973, 0.9741760185577133, 5.117047993735095]),
            (CanonLog2, [0.0013100527311745765, 0.029316779433863873, 0.1928116271014416, 1.0304728617701768, 12.059916820096552]),
            (VLog, [-0.0035042242703533003, 0.022262128584880497, 0.14989278275686924, 0.803127344377689, 9.39177019514155]),
            (FLog, [0.0014791116010589196, 0.026642639450967717, 0.12011143569942431, 0.41356674239934105, 2.3784578017934455]),
            (FLog2, [0.0014542179072420877, 0.03215511579750174, 0.20707621237417603, 1.0801799538800674, 12.164148465156645]),
            (NLog, [-0.002938410370505232, 0.042336031999999996, 0.25163088023304514, 0.8220122346781868, 4.739823980017223]),
            (LLog, [0.0019220430107526904, 0.025025950081749165, 0.13708303295232974, 0.6198519513850407, 5.6131705960570315]),
            (DLog, [-0.007120331950207467, 0.02117638538656266, 0.18212767982843486, 1.1502416202587098, 17.118393000420788]),
            (BlackmagicFilm5, [-0.005126481603687439, 0.016958156169356923, 0.21861310183789934, 2.2314342276716577, 70.5363031782989]),
            (LogC3, [0.002341309954909829, 0.034395083578948875, 0.20653726166860586, 1.0551678998532175, 11.643226001655659]),
            (LogC4, [0.004106995068396062, 0.10085966308918284, 0.7804999378852904, 5.026035923764938, 78.88941008001092]),
        ];
        for (log, expected) in cases {
            let p = profile(log).unwrap();
            for (s, want) in [0.05, 0.2, 0.4, 0.6, 0.9].into_iter().zip(expected) {
                let got = (p.decode)(if p.legal { (64.0 + 876.0 * s) / 1023.0 } else { s });
                assert!((got - want).abs() <= 1e-6 * want.abs().max(1.0), "{log:?} at {s}: {got} != {want}");
            }
        }
    }

    /// colour-science's `matrix_RGB_to_RGB` without chromatic adaptation.
    #[test]
    fn gamut_matrices_match_colour_science() {
        #[rustfmt::skip]
        let cases = [
            (BT2020, BT709, [[1.6604910021084343, -0.5876411387885497, -0.07284986331988469], [-0.1245504745215907, 1.1328998971259605, -0.008349422604369489], [-0.01815076335490521, -0.10057889800800743, 1.1187296613629123]]),
            (profile(SLog3Cine).unwrap().primaries, BT2020, [[0.9600463438763903, 0.11953297883442855, -0.079579322602454], [-0.05223947960319518, 1.2643057471780599, -0.21206626776810023], [-0.02884050666093021, -0.059520968181194546, 1.088361474885053]]),
        ];
        for (from, to, want) in cases {
            let got = rgb_to_rgb(from, to);
            for (g, w) in got.iter().flatten().zip(want.iter().flatten()) {
                assert!((g - w).abs() < 1e-4, "{got:?} != {want:?}");
            }
        }
    }

    /// Every profile shows black as black, 18 % grey at 0.44 and 90 % white at 0.88, grey without a tint, and light
    /// past the brightest it records as almost white rather than a clipped jump.
    #[test]
    fn every_profile_puts_grey_and_white_where_rec709_does() {
        for log in ALL {
            let at = |x: f64| convert(log, [encode(log, x); 3]);
            for (x, want) in [(0.0, 0.0), (0.18, 0.44), (0.9, 0.88)] {
                let got = at(x);
                assert!(got.iter().all(|c| (c - want).abs() < 0.006), "{log:?}: {x} became {got:?}, not {want}");
            }
            let bright = at(8.0);
            assert!(bright.iter().all(|&c| c > 0.97 && c < 1.0), "{log:?}: 8 became {bright:?}");
        }
    }

    /// The tone curve rises over every channel, so a brighter recording never shows darker.
    #[test]
    fn brighter_signals_never_show_darker() {
        for log in ALL {
            let mut last = 0.0;
            for i in 0..=255 {
                let [r, ..] = convert(log, [i as f32 / 255.0, 0.3, 0.3]);
                assert!(r >= last - 1e-6, "{log:?}: red fell at {i}");
                last = r;
            }
        }
    }

    /// The sampled curve the GPU interpolates stays within a hundredth of a level of the exact one: 1e-4 of grey's
    /// light in the shadows, where the kinks between two pieces of a curve are, and 1e-4 of the light above.
    #[test]
    fn sampled_curves_follow_the_published_ones() {
        for log in ALL {
            let table = log_conversion(log).unwrap();
            let exact = Conversion::new(log).unwrap();
            let step = (CURVE_POINTS - 1) as f32;
            for i in 0..=1000 {
                let s = i as f32 / 1000.0;
                let p = s * step;
                let at = (p.floor() as usize).min(CURVE_POINTS - 2);
                let t = p - at as f32;
                let lerp = table.curve[at] + (table.curve[at + 1] - table.curve[at]) * t;
                let want = exact.scene(s) as f32;
                assert!((lerp - want).abs() <= 1e-4 * (1.0 + want.abs()), "{log:?} at {s}: {lerp} != {want}");
            }
        }
    }

    #[test]
    fn cube_files_read_with_red_changing_fastest() {
        let text =
            "TITLE \"Swap\"\n# a comment\nLUT_3D_SIZE 2\n\n0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n";
        let lut = parse_cube(text).unwrap();
        // Red in comes out as blue: the table swaps red and blue.
        assert_eq!(lut.sample([1.0, 0.0, 0.0]), [0.0, 0.0, 1.0]);
        assert_eq!(lut.sample([0.0, 0.0, 1.0]), [1.0, 0.0, 0.0]);
        let half = lut.sample([0.5, 0.25, 0.0]);
        assert!((half[2] - 0.5).abs() < 1e-6 && (half[1] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn a_domain_and_a_fine_grid_are_drawn_on_a_unit_grid() {
        let mut text = String::from("LUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 2 2 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    text += &format!("{r} {g} {b}\n");
                }
            }
        }
        // Over 0..2 the identity table halves 0..1 inputs.
        let lut = parse_cube(&text).unwrap();
        assert!((lut.sample([1.0, 0.5, 0.0])[0] - 0.5).abs() < 1e-6);
        // Over 0..0.5 it reaches white at 0.5 and stays there.
        let half = parse_cube(&text.replace("DOMAIN_MAX 2 2 2", "DOMAIN_MAX 0.5 0.5 0.5")).unwrap();
        assert!((half.sample([0.5, 0.25, 1.0])[0] - 1.0).abs() < 1e-6);
        assert!((half.sample([0.5, 0.25, 1.0])[1] - 0.5).abs() < 1e-6);
        assert!((half.sample([0.5, 0.25, 1.0])[2] - 1.0).abs() < 1e-6);
        let fine = Lut3d::from_fn(129, |rgb| rgb);
        let mut text = String::from("LUT_3D_SIZE 129\n");
        for [r, g, b] in &fine.data {
            text += &format!("{r} {g} {b}\n");
        }
        let lut = parse_cube(&text).unwrap();
        assert_eq!(lut.size, MAX_DRAWN_SIZE);
        assert!((lut.sample([0.3, 0.6, 0.9])[1] - 0.6).abs() < 1e-5);
    }

    #[test]
    fn broken_cube_files_say_why() {
        let cases = [
            ("", "holds no LUT"),
            ("0 0 0\n1 1 1\n", "doesn't say its size"),
            ("LUT_1D_SIZE 4\n0 0 0\n", "1D LUT"),
            ("LUT_1D_SIZE 2\nLUT_3D_SIZE 2\n", "1D shaper"),
            ("LUT_3D_SIZE 1\n", "from 2 to 256"),
            ("LUT_3D_SIZE 300\n", "from 2 to 256"),
            ("LUT_3D_SIZE 2\n0 0 0\n1 1 1\n", "cut short"),
            ("LUT_3D_SIZE 2\n0 0 nan\n", "Line 2"),
            ("LUT_3D_SIZE 2\n0 0\n", "Line 2"),
            ("LUT_3D_SIZE 2\n0 0 0\nLUT_3D_SIZE 2\n", "Line 3"),
            (
                "LUT_3D_SIZE 2\nDOMAIN_MIN 1 0 0\nDOMAIN_MAX 1 1 1\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n",
                "runs backwards",
            ),
            ("<html>not a lut</html>\n", "Line 1"),
        ];
        for (text, says) in cases {
            let error = parse_cube(text).unwrap_err();
            assert!(error.contains(says), "{text:?}: {error}");
        }
    }

    #[test]
    fn lut_ids_never_name_a_path() {
        assert!(is_lut_id(&"0a".repeat(8)));
        for name in
            ["", "../../etc/passwd", &"0A".repeat(8), &"a".repeat(15), &"a".repeat(17), &format!("{}/", "a".repeat(15))]
        {
            assert!(!is_lut_id(name), "{name}");
        }
    }
}

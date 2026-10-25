//! Offscreen wgpu compositor shared by preview and export.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use wgpu::util::DeviceExt;

use crate::media::Transfer;
use crate::model::Adjust;

/// HDR to SDR conversion, shared by the compositor and plugin effects, whose own names never start with `nuzky_`.
pub(crate) const HDR_WGSL: &str = r#"
const nuzky_bt2020_luma = vec3<f32>(0.2627, 0.678, 0.0593);
// Linear BT.2020 to BT.709 primaries; columns are the BT.2020 red, green and blue.
const nuzky_bt2020_to_bt709 = mat3x3<f32>(
    vec3<f32>(1.6605, -0.1246, -0.0182),
    vec3<f32>(-0.5876, 1.1329, -0.1006),
    vec3<f32>(-0.0728, -0.0083, 1.1187),
);
// HDR scene light to SDR: luminance to this power, with a toe that keeps near-black deep.
const nuzky_hdr_power = 0.93;
const nuzky_hdr_toe = 0.0014;

fn nuzky_pq_to_nits(e: vec3<f32>) -> vec3<f32> {
    let p = pow(e, vec3<f32>(1.0 / 78.84375));
    return 10000.0 * pow(max(p - 0.8359375, vec3<f32>(0.0)) / (18.8515625 - 18.6875 * p), vec3<f32>(1.0 / 0.1593017578125));
}

// PQ or HLG BT.2020 colour to SDR BT.709, encoded for a BT.1886 display like SDR video.
//
// iPhones record HLG, so HDR shows the way Apple's own SDR conversion (AVFoundation) shows HLG, from scene
// light. The power and the toe are fitted to its output, 2/255 off on average on skin tones. Tone mapping
// the light of a 1000 nit display instead shows midtones about 17 levels brighter and less saturated than
// Apple's, so footage looks flat next to the phone. PQ (HDR10 from Android phones) first becomes the scene
// light it shows on a 1000 nit display, as BT.2100 converts PQ to HLG, so one scene shot on either phone
// looks the same; it clips above 1000 nits. One gain from luminance scales the colour, so hue and
// saturation stay.
fn nuzky_hdr_to_sdr(rgb: vec3<f32>, transfer: f32) -> vec3<f32> {
    let e = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    var scene: vec3<f32>;
    if transfer < 1.5 {
        // The inverse of HLG's OOTF (system gamma 1.2) on display light relative to 1000 nits.
        let display = nuzky_pq_to_nits(e) / 1000.0;
        scene = display * pow(max(dot(display, nuzky_bt2020_luma), 1e-6), -1.0 / 6.0);
    } else {
        scene = select((exp((e - 0.55991073) / 0.17883277) + 0.28466892) / 12.0, e * e / 3.0, e <= vec3<f32>(0.5));
    }
    let y = max(dot(scene, nuzky_bt2020_luma), 1e-6);
    let lum = pow(y, nuzky_hdr_power);
    let sdr = clamp(nuzky_bt2020_to_bt709 * (scene * (lum / y * lum / (lum + nuzky_hdr_toe))), vec3<f32>(0.0), vec3<f32>(1.0));
    return pow(sdr, vec3<f32>(1.0 / 2.4));
}
"#;

/// One step of shrinking what effects drew to the size the layer is shown at, by at most `SHRINK_STEP` per side. Each
/// output pixel averages, in premultiplied colour, every texel of the area it covers weighted by how much of it lies
/// inside, so lines finer than the output blend instead of being skipped. The source is the top-left `size` of its
/// texture, as earlier steps leave it.
const SHRINK: &str = r#"
struct Shrink {
    ratio: vec2<f32>, // texels per output pixel
    size: vec2<f32>,
};
@group(0) @binding(0) var<uniform> shrink: Shrink;
@group(0) @binding(1) var src: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let lo = floor(pos.xy) * shrink.ratio;
    let hi = min(lo + shrink.ratio, shrink.size);
    let first = vec2<i32>(floor(lo));
    let last = min(vec2<i32>(ceil(hi)) - 1, vec2<i32>(shrink.size) - 1);
    var sum = vec4<f32>(0.0);
    var weight = 0.0;
    for (var y = first.y; y <= last.y; y += 1) {
        let wy = min(hi.y, f32(y + 1)) - max(lo.y, f32(y));
        for (var x = first.x; x <= last.x; x += 1) {
            let w = (min(hi.x, f32(x + 1)) - max(lo.x, f32(x))) * wy;
            let c = textureLoad(src, vec2<i32>(x, y), 0);
            sum += vec4<f32>(c.rgb * c.a, c.a) * w;
            weight += w;
        }
    }
    let c = sum / max(weight, 1e-6);
    return vec4<f32>(select(vec3<f32>(0.0), c.rgb / c.a, c.a > 0.0), c.a);
}
"#;

/// The most one shrink step divides a side by: at most 25 texels per output pixel, and a 4K picture shown at 60 pixels
/// wide takes three steps.
const SHRINK_STEP: u32 = 4;

const SHADER: &str = r#"
struct Layer {
    corners: array<vec4<f32>, 4>, // xy = clip-space position, zw = uv
    local: array<vec4<f32>, 4>, // xy = output pixels from the centre of the visible part
    opacity: vec4<f32>, // opacity, transfer (0 SDR, 1 PQ, 2 HLG)
    adjust: vec4<f32>,
    effects: vec4<f32>,
    grading: vec4<f32>, // exposure, tint, highlights, shadows
    clip: vec4<f32>,
    mask: vec4<f32>, // half width and height of the visible part, corner radius, border width; no mask at 0 width
    border: vec4<f32>, // premultiplied border colour
    shadow: vec4<f32>, // opacity, blur, offset down
    matte: vec4<f32>, // fill behind the person: 0 none, 1 footage, 2 colour, 3 picture; footage: its own transfer, or picture scale
    fill: vec4<f32>, // premultiplied fill colour
};
@group(0) @binding(0) var<uniform> layer: Layer;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var matte_tex: texture_2d<f32>; // the person in alpha, over the same uv as tex
@group(0) @binding(3) var fill_tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) local: vec2<f32>,
    @location(2) layer_uv: vec2<f32>, // where in the whole layer, upright, 0..1
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    let c = layer.corners[i];
    var out: VsOut;
    out.pos = vec4<f32>(c.xy, 0.0, 1.0);
    out.uv = c.zw;
    out.local = layer.local[i].xy;
    out.layer_uv = layer.local[i].zw;
    return out;
}

// Signed distance from a box with rounded corners centred on 0, negative inside.
fn rounded_box(p: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half + radius;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

const BT709_LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

// g * x / (1 + (g - 1) * x) keeps 0 and 1, has slope g at 0 and 1 / g at 1, and rises for every g > 0.
fn bend(x: vec3<f32>, g: f32) -> vec3<f32> {
    return g * x / (1.0 + (g - 1.0) * x);
}

// Brings colour into 0..1 by moving it towards its own luma, keeping hue and luma where clipping each
// channel apart would shift the hue; light that cannot fit becomes white.
fn fit(rgb: vec3<f32>) -> vec3<f32> {
    let y = clamp(dot(rgb, BT709_LUMA), 0.0, 1.0);
    let hi = max(max(rgb.r, rgb.g), rgb.b);
    let lo = min(min(rgb.r, rgb.g), rgb.b);
    let k = min(1.0, min((1.0 - y) / max(hi - y, 1e-6), y / max(y - lo, 1e-6)));
    return y + (rgb - y) * k;
}

fn premultiplied_texel(p: vec2<i32>, size: vec2<i32>) -> vec4<f32> {
    let c = textureLoad(tex, clamp(p, vec2<i32>(0), size - 1), 0);
    return select(vec4<f32>(c.rgb * c.a, c.a), c, layer.effects.z > 0.0);
}

// Bilinear filtering of premultiplied colour, so the colour of transparent texels cannot darken
// soft edges. Textures hold straight alpha except the premultiplied transition intermediate.
fn sample_premultiplied(uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(tex));
    let p = uv * vec2<f32>(size) - 0.5;
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let top = mix(premultiplied_texel(i, size), premultiplied_texel(i + vec2<i32>(1, 0), size), f.x);
    let bottom = mix(premultiplied_texel(i + vec2<i32>(0, 1), size), premultiplied_texel(i + 1, size), f.x);
    return mix(top, bottom, f.y);
}

// Bilinear premultiplied colour of a texture holding straight alpha.
fn sample_straight(t: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(t));
    let p = uv * vec2<f32>(size) - 0.5;
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    var texels: array<vec4<f32>, 4>;
    for (var k = 0; k < 4; k += 1) {
        let c = textureLoad(t, clamp(i + vec2<i32>(k % 2, k / 2), vec2<i32>(0), size - 1), 0);
        texels[k] = vec4<f32>(c.rgb * c.a, c.a);
    }
    return mix(mix(texels[0], texels[1], f.x), mix(texels[2], texels[3], f.x), f.y);
}

// Cubic B-spline weights of the four texels around a point `t` past the second one.
fn bspline(t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4<f32>(1.0 - 3.0 * t + 3.0 * t2 - t3, 4.0 - 6.0 * t2 + 3.0 * t3, 1.0 + 3.0 * t + 3.0 * t2 - 3.0 * t3, t3) / 6.0;
}

// How much of the person covers `uv`. The matte is a coarse grid of probabilities: a cubic B-spline over it
// gives a smooth outline with no corners at the cells, and the smoothstep makes it a clean edge a few pixels wide.
fn person(uv: vec2<f32>, smooth_outline: bool) -> f32 {
    let size = vec2<i32>(textureDimensions(matte_tex));
    let p = uv * vec2<f32>(size) - 0.5;
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    var a = 0.0;
    if smooth_outline {
        let wx = bspline(f.x);
        let wy = bspline(f.y);
        for (var y = 0; y < 4; y += 1) {
            for (var x = 0; x < 4; x += 1) {
                a += wx[x] * wy[y] * textureLoad(matte_tex, clamp(i + vec2<i32>(x - 1, y - 1), vec2<i32>(0), size - 1), 0).a;
            }
        }
    } else {
        a = sample_straight(matte_tex, uv).a;
    }
    // The model is unsure about hair and a bun behind the head; leaning towards the person keeps them.
    return smoothstep(0.15, 0.6, a);
}

// The fill behind the person at a point, premultiplied.
fn fill_at(uv: vec2<f32>, layer_uv: vec2<f32>) -> vec4<f32> {
    if layer.matte.x < 1.5 {
        let f = sample_straight(fill_tex, uv);
        if layer.matte.y > 0.0 {
            // HDR footage behind a picture its effects already made SDR is tone mapped on its own.
            let rgb = select(vec3<f32>(0.0), f.rgb / f.a, f.a > 0.0);
            return vec4<f32>(nuzky_hdr_to_sdr(rgb, layer.matte.y) * f.a, f.a);
        }
        return f;
    } else if layer.matte.x < 2.5 {
        return layer.fill;
    }
    return sample_straight(fill_tex, 0.5 + (layer_uv - 0.5) * layer.matte.yz);
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    if in.pos.x < layer.clip.x || in.pos.y < layer.clip.y || in.pos.x >= layer.clip.z || in.pos.y >= layer.clip.w {
        discard;
    }
    let matte = layer.matte.x > 0.5;
    // A colour or picture shows as picked, after the clip's own adjustments.
    let picked = layer.matte.x > 1.5;
    var c = sample_premultiplied(in.uv);
    var person_a = 1.0;
    // What a blur transition takes of a picked fill, added after the adjustments.
    var picked_fill = vec4<f32>(0.0);
    if layer.effects.y > 0.0 {
        let step = vec2<f32>(layer.effects.y) / vec2<f32>(textureDimensions(tex));
        c = vec4<f32>(0.0);
        for (var y = -2; y <= 2; y += 1) {
            for (var x = -2; x <= 2; x += 1) {
                let offset = vec2<f32>(f32(x), f32(y)) * step;
                var tap = sample_premultiplied(in.uv + offset);
                if matte {
                    // A blur transition blurs the finished picture, so no part of the room shows around the person.
                    let a = person(in.uv + offset, false);
                    let fill = fill_at(in.uv + offset, in.layer_uv + offset) * (1.0 - a);
                    tap *= a;
                    if picked {
                        picked_fill += fill / 25.0;
                    } else {
                        tap += fill;
                    }
                }
                c += tap / 25.0;
            }
        }
    } else if matte {
        person_a = person(in.uv, true);
        if layer.matte.x < 1.5 {
            // The blurred footage is the clip's own picture: tone mapped and adjusted with the person.
            c = c * person_a + fill_at(in.uv, in.layer_uv) * (1.0 - person_a);
        }
    }
    var rgb = c.rgb;
    let adjusting = any(layer.adjust != vec4<f32>(0.0)) || any(layer.grading != vec4<f32>(0.0)) || layer.effects.x != 0.0 || layer.effects.w != 0.0;
    let hdr = layer.opacity.y > 0.0;
    if adjusting || hdr {
        // Tone mapping and adjustments work on straight colour.
        rgb = select(vec3<f32>(0.0), rgb / c.a, c.a > 0.0);
        if hdr {
            rgb = nuzky_hdr_to_sdr(rgb, layer.opacity.y);
        }
        if adjusting {
            // Every control scales or bends the picture instead of adding a constant to it, so black stays black
            // apart from fade, and each is the identity at 0. `fit` keeps colour in 0..1 where clipping each
            // channel would shift its hue.
            const STOPS = 2.0;
            // Pulled down, contrast and brightness bend by fewer stops: at 2 they stretched the tones next to
            // clipped white into a hard-edged patch with contour rings.
            const CONTRAST_DOWN_STOPS = 1.0;
            const BRIGHTNESS_DOWN_STOPS = 1.25;
            const TEMPERATURE = vec3<f32>(1.15, 0.15, -1.15);
            const TINT = vec3<f32>(0.9, -0.9, 0.9);
            const TONE_STOPS = 1.25;
            const HIGHLIGHTS_DIM = 0.2;
            const FADE_BLACK = 0.25;
            const FADE_WHITE = 0.05;
            // White balance and exposure are gains on linear light, the 2.4 power of the colour here. The white
            // balance gains keep luma, so warming does not brighten. Keeping luma alone takes green down when
            // warming and up when cooling, which turned skin red or olive; green following red a little keeps the
            // hue of skin. At 1.15 and 0.9 they move red against blue on 18 % grey as far as the additive shift
            // they replaced.
            let wb = exp2(layer.adjust.w * TEMPERATURE + layer.grading.y * TINT);
            rgb = fit(rgb * pow(wb / dot(wb, BT709_LUMA) * exp2(layer.grading.x * STOPS), vec3<f32>(1.0 / 2.4)));
            // Contrast is an S-curve around mid grey, slope 4^c there and 4^-c at black and white (2^c and 2^-c
            // pulled down), so nothing clips. It moves only the brightest and darkest channel and puts the middle
            // one at the same place between them, which keeps the hue; saturation follows contrast as on film.
            let hi = max(max(rgb.r, rgb.g), rgb.b);
            let lo = min(min(rgb.r, rgb.g), rgb.b);
            let d = 2.0 * vec3<f32>(hi, lo, 0.0) - 1.0;
            let c = layer.adjust.y;
            let s = 0.5 + 0.5 * sign(d) * bend(abs(d), exp2(c * select(STOPS, CONTRAST_DOWN_STOPS, c < 0.0)));
            rgb = s.y + (rgb - lo) * (s.x - s.y) / max(hi - lo, 1e-6);
            // Shadows expose tones below mid grey by up to 1.25 stops and highlights the distance to white above
            // it, both keeping black and mid grey. Highlights pulled down also dim the tones above 70 % by up to a
            // fifth, white included, so a bright sky gets its tone back. Brightness bends luma between black and
            // white, which stay, and moves mid grey. All of them work on luma and scale the colour with it, so a
            // lighter shadow keeps its colour instead of turning grey.
            let y = dot(rgb, BT709_LUMA);
            let h = layer.grading.z;
            let toned = select(
                y * exp2(TONE_STOPS * layer.grading.w * (1.0 - smoothstep(0.0, 0.5, y))),
                (1.0 - (1.0 - y) * exp2(-TONE_STOPS * h * smoothstep(0.5, 1.0, y)))
                    * (1.0 + HIGHLIGHTS_DIM * min(h, 0.0) * smoothstep(0.7, 1.0, y)),
                y > 0.5
            );
            let b = layer.adjust.x;
            rgb *= bend(vec3<f32>(toned), exp2(b * select(STOPS, BRIGHTNESS_DOWN_STOPS, b < 0.0))).x / max(y, 1e-6);
            let luma = dot(rgb, BT709_LUMA);
            rgb = fit(mix(vec3<f32>(luma), rgb, 1.0 + layer.adjust.z));
            // Fade maps black to 0.25 and white to 0.95 at full strength, without changing hue.
            rgb = rgb * (1.0 - layer.effects.w * (FADE_BLACK + FADE_WHITE)) + layer.effects.w * FADE_BLACK;
            let edge = smoothstep(0.2, 0.72, distance(in.uv, vec2<f32>(0.5)));
            rgb *= 1.0 - edge * layer.effects.x * 0.85;
        }
        rgb = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * c.a;
    }
    var color = vec4<f32>(rgb, c.a);
    if picked && layer.effects.y <= 0.0 {
        color = color * person_a + fill_at(in.uv, in.layer_uv) * (1.0 - person_a);
    } else if picked {
        color += picked_fill;
    }
    if layer.mask.x > 0.0 {
        // Coverage of the visible part and of it with the border, smoothed over one pixel.
        let d = rounded_box(in.local, layer.mask.xy, layer.mask.z);
        let inside = clamp(0.5 - d, 0.0, 1.0);
        let bordered = clamp(0.5 - d + layer.mask.w, 0.0, 1.0);
        color = color * inside + layer.border * (bordered - inside);
        if layer.shadow.x > 0.0 {
            let outer = layer.mask.xy + layer.mask.w;
            let s = rounded_box(in.local - vec2<f32>(0.0, layer.shadow.z), outer, layer.mask.z + layer.mask.w);
            color += vec4<f32>(0.0, 0.0, 0.0, layer.shadow.x * (1.0 - smoothstep(-layer.shadow.y, layer.shadow.y, s))) * (1.0 - color.a);
        }
    }
    return color * layer.opacity.x;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LayerUniform {
    corners: [[f32; 4]; 4],
    local: [[f32; 4]; 4],
    opacity: [f32; 4],
    adjust: [f32; 4],
    effects: [f32; 4],
    grading: [f32; 4],
    clip: [f32; 4],
    mask: [f32; 4],
    border: [f32; 4],
    shadow: [f32; 4],
    matte: [f32; 4],
    fill: [f32; 4],
}

/// Straight-alpha RGBA image shared between frames without copying.
#[derive(Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub data: Arc<Vec<u8>>,
}

#[derive(Clone)]
pub struct Layer {
    pub image: Image,
    /// Output pixel positions of the top-left, top-right, bottom-right, bottom-left corners.
    pub corners: [[f32; 2]; 4],
    /// Clockwise rotation of the image inside the quad (0, 90, 180, 270).
    pub uv_rotation: u32,
    /// Mirrors the rotated image left to right.
    pub mirror: bool,
    pub opacity: f32,
    pub adjust: Adjust,
    pub blur: f32,
    pub clip: Option<[f32; 4]>,
    pub transfer: Transfer,
    pub mask: Option<Mask>,
    pub matte: Option<Box<Matte>>,
    /// Plugin effects the image goes through first, in order; each must be prepared (`Gpu::prepare_effect`).
    pub effects: Vec<EffectPass>,
}

/// One plugin effect drawn over a layer's image (`crate::plugin_effects`).
#[derive(Clone)]
pub struct EffectPass {
    pub key: u64,
    /// The layer's upright size in canvas pixels, the same in the preview and the export.
    pub size: [f32; 2],
    /// The upright texels the effects draw, the same in the preview and the export, so a pattern of canvas pixels
    /// comes out alike; the same for each effect of a layer.
    pub grid: [u32; 2],
    /// The upright pixels the layer takes in the output; a smaller output gets the effects' picture shrunk to it.
    pub shown: [u32; 2],
    /// Seconds since the clip started, on the timeline.
    pub time: f32,
    pub seed: u32,
    pub params: [f32; 16],
}

/// Why a frame failed on the GPU itself, so what it drew must not be drawn again on this device.
#[derive(Debug)]
pub enum GpuFault {
    Timeout(Duration),
    Lost(String),
    Error(String),
    /// The wait for the frame ended at the caller's cancel.
    Stopped,
}

impl std::fmt::Display for GpuFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout(limit) => write!(f, "the GPU took longer than {} s to draw it", limit.as_secs_f32()),
            Self::Lost(why) => write!(f, "the GPU stopped while drawing it ({why})"),
            Self::Error(why) => write!(f, "the GPU refused to draw it ({why})"),
            Self::Stopped => write!(f, "CANCELLED: stopped while the GPU drew the frame"),
        }
    }
}

impl std::error::Error for GpuFault {}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EffectUniform {
    to_texture: [f32; 4],
    to_upright: [f32; 4],
    offsets: [f32; 4],
    /// Upright size in canvas pixels, time, transfer of the image read.
    info: [f32; 4],
    seed: [u32; 4],
    params: [[f32; 4]; 4],
}

/// The affine map from the layer's upright (u, v) to its texture and back, as columns and offsets.
fn orientation(rotation: u32, mirror: bool) -> ([f32; 4], [f32; 4], [f32; 4]) {
    let o = texture_uv(0.0, 0.0, rotation, mirror);
    let x = texture_uv(1.0, 0.0, rotation, mirror);
    let y = texture_uv(0.0, 1.0, rotation, mirror);
    let (ax, ay) = ([x[0] - o[0], x[1] - o[1]], [y[0] - o[0], y[1] - o[1]]);
    // A rotation with a mirror is orthonormal: its inverse is its transpose.
    let back = [ax[0], ay[0], ax[1], ay[1]];
    let back_offset = [-(ax[0] * o[0] + ax[1] * o[1]), -(ay[0] * o[0] + ay[1] * o[1])];
    ([ax[0], ax[1], ay[0], ay[1]], back, [o[0], o[1], back_offset[0], back_offset[1]])
}

/// The person cut out of a layer by a matte, over a fill.
#[derive(Clone)]
pub struct Matte {
    /// The person in alpha, covering the layer image in its orientation.
    pub alpha: Image,
    pub fill: Fill,
}

#[derive(Clone)]
pub enum Fill {
    /// The layer's own picture blurred, in the layer image's orientation; graded with the person.
    Footage(Image),
    /// Straight RGBA, shown as picked.
    Color([f32; 4]),
    /// An upright picture shown as picked; `scale` maps the layer's upright (u, v) about the centre into it.
    Picture { image: Image, scale: [f32; 2] },
}

/// The visible part of a layer and how its edge is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mask {
    /// Left, top, right and bottom edges of the visible part, in 0..1 of the quad.
    pub rect: [f32; 4],
    /// Share of half the shorter visible side, 0 to 1; it scales with the quad, as in a zoom transition.
    pub radius: f32,
    /// Output pixels, outside the edge.
    pub border: f32,
    /// Straight RGBA.
    pub border_color: [f32; 4],
    /// Opacity of the shadow, 0 for none.
    pub shadow: f32,
    /// Output pixels.
    pub shadow_blur: f32,
}

impl Mask {
    /// How far the border and shadow reach past the visible part, plus a pixel of edge smoothing.
    fn reach(&self) -> f32 {
        let shadow = if self.shadow > 0.0 { self.shadow_blur * SHADOW_OFFSET + self.shadow_blur } else { 0.0 };
        self.border + shadow + 1.0
    }
}

/// How far a shadow falls below the layer, as a share of its blur.
const SHADOW_OFFSET: f32 = 0.4;

/// Where the image point shown at `(u, v)` of the quad lies in the texture, for an image rotated clockwise by
/// `rotation` and then mirrored.
pub(crate) fn texture_uv(u: f32, v: f32, rotation: u32, mirror: bool) -> [f32; 2] {
    let u = if mirror { 1.0 - u } else { u };
    match rotation % 360 {
        90 => [v, 1.0 - u],
        180 => [1.0 - u, 1.0 - v],
        270 => [1.0 - v, u],
        _ => [u, v],
    }
}

// A frame has a few draws, built anew each frame; boxing a transition's pair would only add an allocation.
#[allow(clippy::large_enum_variant)]
pub enum Draw {
    Layer(Layer),
    /// Both layers contribute premultiplied colour to a transparent intermediate.
    Transition([Layer; 2]),
}

struct Target {
    size: (u32, u32),
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_row: u32,
}

struct TransitionTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    additive: wgpu::RenderPipeline,
    transitions: Vec<TransitionTarget>,
    layout: wgpu::BindGroupLayout,
    effect_layout: wgpu::BindGroupLayout,
    effect_pipelines: HashMap<u64, wgpu::RenderPipeline>,
    shrink: wgpu::RenderPipeline,
    /// The textures of each layer with effects in a frame.
    effect_targets: Vec<EffectTargets>,
    /// Frames whose wait acts as if the GPU never answered, for tests of a hung driver.
    #[doc(hidden)]
    pub hung_frames: u32,
    /// Errors wgpu reported outside an error scope during the frame, which by default would panic.
    errors: Arc<Mutex<Vec<String>>>,
    lost: Arc<Mutex<Option<String>>>,
    target: Option<Target>,
    /// Textures from the previous frame, keyed by the image allocation they hold.
    textures: HashMap<usize, (Arc<Vec<u8>>, wgpu::Texture)>,
    /// Bound where a layer has no matte or fill.
    blank: wgpu::Texture,
    pub adapter_name: String,
}

/// Two textures a layer's effects draw into in turn, and the one their picture is shrunk into for a smaller output.
struct EffectTargets {
    passes: [wgpu::Texture; 2],
    shrunk: Option<wgpu::Texture>,
}

/// How often a wait for the GPU looks at its cancel.
const WAIT_SLICE: Duration = Duration::from_millis(25);

/// A pipeline drawing a whole texture through `module`'s `vs` and `fs`, replacing what was there.
fn whole_texture_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    module: &wgpu::ShaderModule,
    [vs, fs]: [&str; 2],
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(fs),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(fs),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vs),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

impl Gpu {
    pub fn new() -> Result<Self> {
        // Debug builds name every GPU object by default. Vulkan loaders before 1.4.345 look the device
        // up for each name without the lock that guards another renderer's new instance freeing unused
        // drivers, so preview and export starting together crashed. WGPU_DEBUG=1 still names them.
        let mut options = wgpu::InstanceDescriptor::new_without_display_handle();
        options.flags.remove(wgpu::InstanceFlags::DEBUG);
        let instance = wgpu::Instance::new(options.with_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .context("No GPU adapter found")?;
        let info = adapter.get_info();
        let adapter_name = format!("{} ({})", info.name, info.backend);
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor { label: Some("nuzky"), ..Default::default() }),
        )
        .context("Cannot open GPU device")?;
        let errors = Arc::new(Mutex::new(Vec::new()));
        let reported = errors.clone();
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            log::error!("GPU error: {error}");
            reported.lock().unwrap().push(error.to_string());
        }));
        let lost = Arc::new(Mutex::new(None));
        let gone = lost.clone();
        device.set_device_lost_callback(move |reason, message| {
            *gone.lock().unwrap() = Some(format!("{reason:?}: {message}"));
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compositor"),
            source: wgpu::ShaderSource::Wgsl(format!("{SHADER}{HDR_WGSL}").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("layer"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("compositor"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make_pipeline = |blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("compositor"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make_pipeline(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let component = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let additive = make_pipeline(wgpu::BlendState { color: component, alpha: component });
        let effect_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effect"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(1),
            ],
        });
        let shrink_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shrink"),
            source: wgpu::ShaderSource::Wgsl(SHRINK.into()),
        });
        let shrink = whole_texture_pipeline(&device, &effect_layout, &shrink_module, ["vs", "fs"]);
        let blank = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("blank"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[0, 0, 0, 0],
        );
        Ok(Self {
            device,
            queue,
            pipeline,
            additive,
            transitions: Vec::new(),
            layout,
            effect_layout,
            effect_pipelines: HashMap::new(),
            shrink,
            effect_targets: Vec::new(),
            hung_frames: 0,
            errors,
            lost,
            target: None,
            textures: HashMap::new(),
            blank,
            adapter_name,
        })
    }

    pub fn max_texture_dimension(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    fn target(&mut self, w: u32, h: u32) -> &Target {
        if self.target.as_ref().map(|t| t.size) != Some((w, h)) {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("frame"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let padded_row = (w * 4).div_ceil(256) * 256;
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: padded_row as u64 * h as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.target = Some(Target { size: (w, h), texture, view, readback, padded_row });
        }
        self.target.as_ref().unwrap()
    }

    fn texture_for(&mut self, image: &Image) -> Result<wgpu::Texture> {
        let key = Arc::as_ptr(&image.data) as usize;
        if let Some((_, tex)) = self.textures.get(&key) {
            return Ok(tex.clone());
        }
        // wgpu panics on invalid textures; callers size layers to the limit, so this is a bug guard.
        let max = self.max_texture_dimension();
        if image.width == 0
            || image.height == 0
            || image.width.max(image.height) > max
            || image.data.len() < image.width as usize * image.height as usize * 4
        {
            anyhow::bail!("Cannot draw a {}×{} layer (GPU limit {max} px)", image.width, image.height);
        }
        let size = wgpu::Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("layer"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &image.data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(image.width * 4), rows_per_image: None },
            size,
        );
        self.textures.insert(key, (image.data.clone(), tex.clone()));
        Ok(tex)
    }

    /// `textures` are the layer's image, its matte and its fill.
    fn bind(&self, layer: &Layer, textures: [&wgpu::Texture; 3], w: u32, h: u32, premult: bool) -> wgpu::BindGroup {
        let base = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let shift = (layer.uv_rotation / 90) as usize % 4;
        let mut corners = [[0.0; 4]; 4];
        let mut local = [[0.0; 4]; 4];
        let clip_space = |p: [f32; 2]| [p[0] / w as f32 * 2.0 - 1.0, 1.0 - p[1] / h as f32 * 2.0];
        // Triangle strip order: top-left, top-right, bottom-left, bottom-right.
        let strip = [0usize, 1, 3, 2];
        let (mask, border, shadow) = match layer.mask {
            None => {
                for (slot, cyclic) in strip.into_iter().enumerate() {
                    let p = clip_space(layer.corners[cyclic]);
                    // Mirrored, each corner shows what its left-right partner would.
                    let source = if layer.mirror { [1, 0, 3, 2][cyclic] } else { cyclic };
                    let uv = base[(source + 4 - shift) % 4];
                    corners[slot] = [p[0], p[1], uv[0], uv[1]];
                    local[slot] = [0.0, 0.0, base[cyclic][0], base[cyclic][1]];
                }
                ([0.0; 4], [0.0; 4], [0.0; 4])
            }
            Some(mask) => {
                // The quad is a parallelogram: a point at (u, v) of it is the top-left corner plus u of
                // the top edge and v of the left one. The drawn quad is the visible part plus the reach
                // of the border and shadow.
                let [tl, tr, _, bl] = layer.corners;
                let across = [tr[0] - tl[0], tr[1] - tl[1]];
                let down = [bl[0] - tl[0], bl[1] - tl[1]];
                let size = [across[0].hypot(across[1]).max(1e-3), down[0].hypot(down[1]).max(1e-3)];
                let [left, top, right, bottom] = mask.rect;
                let centre = [(left + right) / 2.0, (top + bottom) / 2.0];
                let reach = [mask.reach() / size[0], mask.reach() / size[1]];
                let drawn = [
                    [left - reach[0], top - reach[1]],
                    [right + reach[0], top - reach[1]],
                    [right + reach[0], bottom + reach[1]],
                    [left - reach[0], bottom + reach[1]],
                ];
                for (slot, cyclic) in strip.into_iter().enumerate() {
                    let [u, v] = drawn[cyclic];
                    let p = clip_space([tl[0] + u * across[0] + v * down[0], tl[1] + u * across[1] + v * down[1]]);
                    let uv = texture_uv(u, v, layer.uv_rotation, layer.mirror);
                    corners[slot] = [p[0], p[1], uv[0], uv[1]];
                    local[slot] = [(u - centre[0]) * size[0], (v - centre[1]) * size[1], u, v];
                }
                // min and max rather than clamp, which panics on the NaN of a broken project.
                let half = [((right - left) * size[0] / 2.0).max(1e-3), ((bottom - top) * size[1] / 2.0).max(1e-3)];
                let [r, g, b, a] = mask.border_color;
                (
                    [half[0], half[1], mask.radius.clamp(0.0, 1.0) * half[0].min(half[1]), mask.border],
                    [r * a, g * a, b * a, a],
                    [mask.shadow, mask.shadow_blur.max(1e-3), mask.shadow_blur * SHADOW_OFFSET, 0.0],
                )
            }
        };
        // Effects draw SDR, so the picture is not tone mapped again; HDR footage behind it is, on its own.
        let transfer = layer.transfer as u8 as f32;
        let (picture_transfer, footage_transfer) =
            if layer.effects.is_empty() { (transfer, 0.0) } else { (0.0, transfer) };
        let (matte, fill) = match layer.matte.as_ref().map(|m| &m.fill) {
            None => ([0.0; 4], [0.0; 4]),
            Some(Fill::Footage(_)) => ([1.0, footage_transfer, 0.0, 0.0], [0.0; 4]),
            Some(Fill::Color([r, g, b, a])) => ([2.0, 0.0, 0.0, 0.0], [r * a, g * a, b * a, *a]),
            Some(Fill::Picture { scale, .. }) => ([3.0, scale[0], scale[1], 0.0], [0.0; 4]),
        };
        let a = layer.adjust;
        let uniform = LayerUniform {
            corners,
            local,
            opacity: [layer.opacity.clamp(0.0, 1.0), picture_transfer, 0.0, 0.0],
            adjust: [a.brightness, a.contrast, a.saturation, a.temperature],
            effects: [a.vignette, layer.blur, premult as u8 as f32, a.fade],
            grading: [a.exposure, a.tint, a.highlights, a.shadows],
            clip: layer.clip.unwrap_or([0.0, 0.0, w as f32, h as f32]),
            mask,
            border,
            shadow,
            matte,
            fill,
        };
        let buffer = self.uniform_buffer("layer", bytemuck::bytes_of(&uniform));
        self.bind_group(&buffer, textures.map(|t| t.create_view(&Default::default())))
    }

    /// Written through the queue: a buffer mapped at creation panics in wgpu on a lost device.
    fn uniform_buffer(&self, label: &str, contents: &[u8]) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: contents.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, contents);
        buffer
    }

    fn bind_group(&self, buffer: &wgpu::Buffer, views: [wgpu::TextureView; 3]) -> wgpu::BindGroup {
        let [image, matte, fill] = views.each_ref().map(wgpu::BindingResource::TextureView);
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: image },
                wgpu::BindGroupEntry { binding: 2, resource: matte },
                wgpu::BindGroupEntry { binding: 3, resource: fill },
            ],
        })
    }

    /// The textures a layer draws from, keeping each one's image for the next frame. Its effects draw into the
    /// next pair of effect textures, whose last one it then draws from.
    fn layer_textures(
        &mut self,
        layer: &Layer,
        used: &mut Vec<usize>,
        encoder: &mut wgpu::CommandEncoder,
        effect_slots: &mut usize,
    ) -> Result<[wgpu::Texture; 3]> {
        let mut images = [Some(&layer.image), None, None];
        if let Some(matte) = &layer.matte {
            images[1] = Some(&matte.alpha);
            images[2] = match &matte.fill {
                Fill::Footage(image) | Fill::Picture { image, .. } => Some(image),
                Fill::Color(_) => None,
            };
        }
        let mut textures = [self.blank.clone(), self.blank.clone(), self.blank.clone()];
        for (texture, image) in textures.iter_mut().zip(images) {
            if let Some(image) = image {
                *texture = self.texture_for(image)?;
                used.push(Arc::as_ptr(&image.data) as usize);
            }
        }
        if !layer.effects.is_empty() {
            textures[0] = self.draw_effects(encoder, layer, textures[0].clone(), *effect_slots)?;
            *effect_slots += 1;
        }
        Ok(textures)
    }

    /// Compiles an effect's shader once per device. A shader the GPU refuses fails here, inside error scopes, so
    /// wgpu never panics on it.
    pub fn prepare_effect(&mut self, key: u64, shader: &str) -> std::result::Result<(), String> {
        if self.effect_pipelines.contains_key(&key) {
            return Ok(());
        }
        let internal = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("effect"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = whole_texture_pipeline(&self.device, &self.effect_layout, &module, ["nuzky_vs", "nuzky_fs"]);
        let errors = [validation.pop(), memory.pop(), internal.pop()].map(pollster::block_on);
        if let Some(error) = errors.into_iter().flatten().next() {
            return Err(format!("the GPU cannot build it: {error}"));
        }
        self.effect_pipelines.insert(key, pipeline);
        Ok(())
    }

    /// Draws the layer's effects over `image` on their own grid, then shrinks the result to the size the layer is
    /// shown at when that is smaller, and returns the texture the layer then draws from.
    fn draw_effects(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        layer: &Layer,
        image: wgpu::Texture,
        slot: usize,
    ) -> Result<wgpu::Texture> {
        let first = &layer.effects[0];
        // The grid and output sizes are upright; the textures keep the image's own orientation.
        let texels =
            |[x, y]: [u32; 2]| if layer.uv_rotation % 180 == 90 { (y.max(1), x.max(1)) } else { (x.max(1), y.max(1)) };
        let (w, h) = texels(first.grid);
        let shown = texels(first.shown);
        let shown = (shown.0.min(w), shown.1.min(h));
        let shrunk = (shown != (w, h)).then_some(shown);
        let make = |(width, height): (u32, u32)| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("effect"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let size = |texture: &wgpu::Texture| (texture.width(), texture.height());
        let targets = match self.effect_targets.get(slot) {
            Some(targets) if size(&targets.passes[0]) == (w, h) => EffectTargets {
                passes: targets.passes.clone(),
                shrunk: targets.shrunk.clone().filter(|t| Some(size(t)) == shrunk).or_else(|| shrunk.map(make)),
            },
            _ => EffectTargets { passes: [make((w, h)), make((w, h))], shrunk: shrunk.map(make) },
        };
        if slot == self.effect_targets.len() {
            self.effect_targets.push(targets);
        } else {
            self.effect_targets[slot] = targets;
        }
        let EffectTargets { passes, shrunk } = &self.effect_targets[slot];
        let (passes, shrunk) = (passes.clone(), shrunk.clone());
        let (to_texture, to_upright, offsets) = orientation(layer.uv_rotation, layer.mirror);
        let mut source = image;
        let mut transfer = layer.transfer as u8 as f32;
        for (i, effect) in layer.effects.iter().enumerate() {
            let pipeline =
                self.effect_pipelines.get(&effect.key).context("An effect was drawn before it was prepared")?;
            let p = effect.params;
            let uniform = EffectUniform {
                to_texture,
                to_upright,
                offsets,
                info: [effect.size[0], effect.size[1], effect.time, transfer],
                seed: [effect.seed, 0, 0, 0],
                params: std::array::from_fn(|row| std::array::from_fn(|column| p[row * 4 + column])),
            };
            let target = &passes[i % 2];
            self.whole_texture_pass(encoder, pipeline, bytemuck::bytes_of(&uniform), &source, target, (w, h));
            source = target.clone();
            transfer = 0.0;
        }
        let Some(shrunk) = shrunk else { return Ok(source) };
        // Steps before the last draw into the top-left of the pass texture the effects no longer need, in turn.
        let mut spare = passes[layer.effects.len() % 2].clone();
        let mut from = (w, h);
        loop {
            let to = (shrink_step(from.0, shown.0), shrink_step(from.1, shown.1));
            let target = if to == shown { &shrunk } else { &spare };
            let uniform: [f32; 4] =
                [from.0 as f32 / to.0 as f32, from.1 as f32 / to.1 as f32, from.0 as f32, from.1 as f32];
            self.whole_texture_pass(encoder, &self.shrink, bytemuck::bytes_of(&uniform), &source, target, to);
            if to == shown {
                return Ok(shrunk);
            }
            (source, spare) = (spare, source);
            from = to;
        }
    }

    /// Draws the top-left `width` × `height` of `target` with `pipeline` from `input` and a uniform of `contents`,
    /// clearing the rest.
    fn whole_texture_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        contents: &[u8],
        input: &wgpu::Texture,
        target: &wgpu::Texture,
        (width, height): (u32, u32),
    ) {
        let buffer = self.uniform_buffer("effect", contents);
        let input = input.create_view(&Default::default());
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effect"),
            layout: &self.effect_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&input) },
            ],
        });
        let view = target.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("effect"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..4, 0..1);
    }

    /// Ends the device as a crashed driver would, for tests of what happens then.
    #[doc(hidden)]
    pub fn lose_device(&self) {
        self.device.destroy();
    }

    fn draw_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        background: [f32; 4],
        groups: &[wgpu::BindGroup],
        additive: bool,
    ) {
        let [r, g, b, a] = background;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: r as f64, g: g as f64, b: b as f64, a: a as f64 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(if additive { &self.additive } else { &self.pipeline });
        for bg in groups {
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..4, 0..1);
        }
    }

    /// Composites the layers bottom to top over `background` and returns tight RGBA rows.
    pub fn render(&mut self, w: u32, h: u32, background: [f32; 4], layers: &[Draw]) -> Result<Vec<u8>> {
        self.render_within(w, h, background, layers, None, &AtomicBool::new(false))
    }

    /// `render` that fails with a `GpuFault` when the GPU takes longer than `limit`, or when `stop` is set while it
    /// draws; the device is then still busy with the frame and must not draw again.
    pub fn render_within(
        &mut self,
        w: u32,
        h: u32,
        background: [f32; 4],
        layers: &[Draw],
        limit: Option<Duration>,
        stop: &AtomicBool,
    ) -> Result<Vec<u8>> {
        let max = self.max_texture_dimension();
        if w > max || h > max {
            anyhow::bail!("This resolution is larger than your GPU supports (max {max} px)");
        }
        let mut used = Vec::new();
        let mut bind_groups = Vec::new();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let mut transition_index = 0;
        let mut effect_slots = 0;
        let hung = self.hung_frames > 0;
        self.hung_frames = self.hung_frames.saturating_sub(1);
        for draw in layers {
            match draw {
                Draw::Layer(layer) => {
                    let textures = self.layer_textures(layer, &mut used, &mut encoder, &mut effect_slots)?;
                    bind_groups.push(self.bind(layer, textures.each_ref(), w, h, false));
                }
                Draw::Transition(pair) => {
                    let mut groups = Vec::new();
                    for layer in pair {
                        let textures = self.layer_textures(layer, &mut used, &mut encoder, &mut effect_slots)?;
                        groups.push(self.bind(layer, textures.each_ref(), w, h, false));
                    }
                    if self.transitions.get(transition_index).map(|t| (t.texture.width(), t.texture.height()))
                        != Some((w, h))
                    {
                        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("transition"),
                            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        });
                        let view = texture.create_view(&Default::default());
                        let uniform = self.device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("transition"),
                            size: std::mem::size_of::<LayerUniform>() as u64,
                            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        });
                        let blank = || self.blank.create_view(&Default::default());
                        let bind_group = self.bind_group(&uniform, [view.clone(), blank(), blank()]);
                        let target = TransitionTarget { texture, view, uniform, bind_group };
                        if transition_index == self.transitions.len() {
                            self.transitions.push(target);
                        } else {
                            self.transitions[transition_index] = target;
                        }
                    }
                    let target = &self.transitions[transition_index];
                    transition_index += 1;
                    self.draw_pass(&mut encoder, &target.view, [0.0; 4], &groups, true);
                    let uniform = LayerUniform {
                        corners: [
                            [-1.0, 1.0, 0.0, 0.0],
                            [1.0, 1.0, 1.0, 0.0],
                            [-1.0, -1.0, 0.0, 1.0],
                            [1.0, -1.0, 1.0, 1.0],
                        ],
                        local: [[0.0; 4]; 4],
                        opacity: [1.0, 0.0, 0.0, 0.0],
                        adjust: [0.0; 4],
                        effects: [0.0, 0.0, 1.0, 0.0],
                        grading: [0.0; 4],
                        clip: [0.0, 0.0, w as f32, h as f32],
                        mask: [0.0; 4],
                        border: [0.0; 4],
                        shadow: [0.0; 4],
                        matte: [0.0; 4],
                        fill: [0.0; 4],
                    };
                    self.queue.write_buffer(&target.uniform, 0, bytemuck::bytes_of(&uniform));
                    bind_groups.push(target.bind_group.clone());
                }
            }
        }
        self.textures.retain(|k, _| used.contains(k));

        self.target(w, h);
        let target = self.target.as_ref().unwrap();
        self.draw_pass(&mut encoder, &target.view, background, &bind_groups, false);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &target.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(target.padded_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = target.readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).ok();
        });
        wait(&self.device, hung, limit, stop)?;
        if let Some(why) = self.lost.lock().unwrap().clone() {
            return Err(GpuFault::Lost(why).into());
        }
        let errors = std::mem::take(&mut *self.errors.lock().unwrap());
        if !errors.is_empty() {
            return Err(GpuFault::Error(errors.join("; ")).into());
        }
        let mapped = rx.recv_timeout(limit.unwrap_or(Duration::from_secs(60)));
        let mapped = mapped.map_err(|_| GpuFault::Lost("no answer from the GPU".into()))?;
        if mapped.is_err() && effect_slots > 0 {
            return Err(GpuFault::Lost("the frame could not be read back".into()).into());
        }
        mapped.context("Mapping readback buffer failed")?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        {
            let data = slice.get_mapped_range().context("Reading back the frame failed")?;
            let row = (w * 4) as usize;
            for y in 0..h as usize {
                let start = y * target.padded_row as usize;
                out.extend_from_slice(&data[start..start + row]);
            }
        }
        target.readback.unmap();
        Ok(out)
    }
}

/// Waits for the frame submitted to `device`, looking at `stop` every `WAIT_SLICE`.
fn wait(device: &wgpu::Device, hung: bool, limit: Option<Duration>, stop: &AtomicBool) -> Result<()> {
    let started = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(GpuFault::Stopped.into());
        }
        let left = limit.map(|limit| limit.saturating_sub(started.elapsed()));
        let slice = left.map_or(WAIT_SLICE, |left| left.min(WAIT_SLICE));
        if hung {
            std::thread::sleep(slice);
        } else {
            match device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(slice) }) {
                Err(wgpu::PollError::Timeout) => {}
                polled => return polled.map(drop).context("GPU poll failed"),
            }
        }
        if let (Some(limit), Some(left)) = (limit, left)
            && left <= slice
        {
            return Err(GpuFault::Timeout(limit).into());
        }
    }
}

/// The next size on the way from `from` down to `shown`: the shown size times a power of the step, so every pixel of a
/// step lies within one shown pixel and the averages of the steps are the average a single reduction would take.
fn shrink_step(from: u32, shown: u32) -> u32 {
    let mut to = shown;
    while to < from.div_ceil(SHRINK_STEP) {
        to *= SHRINK_STEP;
    }
    to
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_resources_reuse_and_resize() {
        let mut gpu = Gpu::new().unwrap();
        let layer = Layer {
            image: Image { width: 1, height: 1, data: Arc::new(vec![80, 100, 120, 255]) },
            corners: [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
            uv_rotation: 0,
            mirror: false,
            opacity: 0.5,
            adjust: Adjust::default(),
            blur: 0.0,
            clip: None,
            transfer: Transfer::Sdr,
            mask: None,
            matte: None,
            effects: Vec::new(),
        };
        let draws = [Draw::Transition([layer.clone(), layer])];
        let first = gpu.render(2, 2, [0.0; 4], &draws).unwrap();
        let group = gpu.transitions[0].bind_group.clone();
        assert_eq!(gpu.render(2, 2, [0.0; 4], &draws).unwrap(), first);
        assert_eq!(gpu.transitions[0].bind_group, group);
        let resized = gpu.render(4, 4, [0.0; 4], &draws).unwrap();
        assert_ne!(gpu.transitions[0].bind_group, group);
        assert!(resized.as_chunks::<4>().0.iter().all(|p| p == &first[..4]));
        let max = gpu.max_texture_dimension();
        assert_eq!(
            gpu.render(max + 1, 2, [0.0; 4], &[]).unwrap_err().to_string(),
            format!("This resolution is larger than your GPU supports (max {max} px)")
        );
    }

    /// Text and PNG edges store transparent texels as black; filtering them unpremultiplied
    /// drew a dark fringe around scaled white text over a light background.
    #[test]
    fn scaled_soft_edges_have_no_dark_fringe() {
        let mut gpu = Gpu::new().unwrap();
        let mut layer = Layer {
            image: Image { width: 2, height: 1, data: Arc::new(vec![255, 255, 255, 255, 0, 0, 0, 0]) },
            corners: [[0.0, 0.0], [64.0, 0.0], [64.0, 2.0], [0.0, 2.0]],
            uv_rotation: 0,
            mirror: false,
            opacity: 1.0,
            adjust: Adjust::default(),
            blur: 0.0,
            clip: None,
            transfer: Transfer::Sdr,
            mask: None,
            matte: None,
            effects: Vec::new(),
        };
        for adjust in [false, true] {
            layer.adjust.contrast = if adjust { 0.2 } else { 0.0 };
            let out = gpu.render(64, 2, [1.0; 4], &[Draw::Layer(layer.clone())]).unwrap();
            let darkest = out.as_chunks::<4>().0.iter().map(|p| p[0].min(p[1]).min(p[2])).min().unwrap();
            assert!(darkest >= 254, "adjust={adjust}: darkest edge pixel {darkest}");
        }
        // Half-covered texels keep their straight colour through adjustments.
        layer.image = Image { width: 1, height: 1, data: Arc::new(vec![200, 100, 50, 128]) };
        layer.adjust = Adjust { saturation: -1.0, ..Adjust::default() };
        let out = gpu.render(2, 2, [0.0, 0.0, 0.0, 1.0], &[Draw::Layer(layer)]).unwrap();
        let grey = (0.2126 * 200.0 + 0.7152 * 100.0 + 0.0722 * 50.0) * 128.0 / 255.0;
        assert!(out[..3].iter().all(|&v| (v as f32 - grey).abs() <= 1.5), "{:?} vs {grey}", &out[..4]);
    }

    /// Preview keeps drawing while an export or an agent's contact sheet starts its own renderer.
    /// The Vulkan loader freed driver records under the drawing threads and crashed the process.
    /// With an effect the picture comes out of it SDR, while the blurred footage behind the person is still HLG: it is
    /// tone mapped on its own, as it is without the effect.
    #[test]
    fn hdr_footage_behind_an_effect_is_tone_mapped_as_without_it() {
        let mut gpu = Gpu::new().unwrap();
        let shader = crate::plugin_effects::shader("fn effect(p: Pixel) -> vec4<f32> { return p.color; }");
        gpu.prepare_effect(1, &shader).unwrap();
        let grey = |v: u8, a: u8| Image { width: 1, height: 1, data: Arc::new(vec![v, v, v, a]) };
        // Nobody in the matte, so all that shows is the footage.
        let matte = Matte { alpha: grey(0, 0), fill: Fill::Footage(grey(150, 255)) };
        let mut layer = Layer {
            image: grey(200, 255),
            corners: [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]],
            uv_rotation: 0,
            mirror: false,
            opacity: 1.0,
            adjust: Adjust::default(),
            blur: 0.0,
            clip: None,
            transfer: Transfer::Hlg,
            mask: None,
            matte: Some(Box::new(matte)),
            effects: Vec::new(),
        };
        let without = gpu.render(2, 2, [0.0, 0.0, 0.0, 1.0], &[Draw::Layer(layer.clone())]).unwrap();
        assert!(without[0] < 140, "the footage is tone mapped: {:?}", &without[..4]);
        layer.effects = vec![EffectPass {
            key: 1,
            size: [2.0; 2],
            grid: [2; 2],
            shown: [2; 2],
            time: 0.0,
            seed: 0,
            params: [0.0; 16],
        }];
        let with = gpu.render(2, 2, [0.0, 0.0, 0.0, 1.0], &[Draw::Layer(layer)]).unwrap();
        assert!(with.iter().zip(&without).all(|(a, b)| a.abs_diff(*b) <= 1), "{:?} vs {:?}", &with[..4], &without[..4]);
    }

    #[test]
    fn renderers_start_and_stop_while_others_draw() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let draw = |gpu: &mut Gpu| {
            let layer = Layer {
                image: Image { width: 1, height: 1, data: Arc::new(vec![80, 100, 120, 255]) },
                corners: [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
                uv_rotation: 0,
                mirror: false,
                opacity: 1.0,
                adjust: Adjust::default(),
                blur: 0.0,
                clip: None,
                transfer: Transfer::Sdr,
                mask: None,
                matte: None,
                effects: Vec::new(),
            };
            assert_eq!(gpu.render(1, 1, [0.0; 4], &[Draw::Layer(layer)]).unwrap(), [80, 100, 120, 255]);
        };
        let done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    let mut gpu = Gpu::new().unwrap();
                    while !done.load(Ordering::Relaxed) {
                        draw(&mut gpu);
                    }
                });
            }
            let starters: Vec<_> =
                (0..4).map(|_| scope.spawn(|| (0..8).for_each(|_| draw(&mut Gpu::new().unwrap())))).collect();
            let started: Vec<_> = starters.into_iter().map(|starter| starter.join()).collect();
            done.store(true, Ordering::Relaxed);
            for result in started {
                if let Err(panic) = result {
                    std::panic::resume_unwind(panic);
                }
            }
        });
    }

    #[test]
    fn zero_adjust_preserves_pixels_and_crossfade_preserves_colour() {
        let mut gpu = Gpu::new().unwrap();
        let pixels = vec![30, 80, 170, 255, 250, 130, 20, 255, 0, 255, 90, 255, 128, 128, 128, 255];
        let layer = Layer {
            image: Image { width: 2, height: 2, data: Arc::new(pixels.clone()) },
            corners: [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]],
            uv_rotation: 0,
            mirror: false,
            opacity: 1.0,
            adjust: Adjust::default(),
            blur: 0.0,
            clip: None,
            transfer: Transfer::Sdr,
            mask: None,
            matte: None,
            effects: Vec::new(),
        };
        assert_eq!(gpu.render(2, 2, [0.0; 4], &[Draw::Layer(layer.clone())]).unwrap(), pixels);
        let mut half = layer.clone();
        half.opacity = 0.5;
        let mixed = gpu.render(2, 2, [1.0, 0.0, 1.0, 1.0], &[Draw::Transition([half.clone(), half])]).unwrap();
        for (a, b) in mixed.iter().zip(&pixels) {
            assert!((*a as i16 - *b as i16).abs() <= 1);
        }
        let mut grey = layer;
        grey.adjust.saturation = -1.0;
        let output = gpu.render(2, 2, [0.0; 4], &[Draw::Layer(grey)]).unwrap();
        for pixel in output.as_chunks::<4>().0 {
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(pixel[1], pixel[2]);
        }
    }
}

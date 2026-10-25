//! Effects from plugins: one WGSL function each, `fn effect(p: Pixel) -> vec4<f32>` in
//! `plugins/<id>/effects/<effect>.wgsl`. Nuzky checks the function with naga, wraps it in a shader of its own that
//! owns the only texture and uniform, and draws a clip's picture through it before the compositor places the layer,
//! so the preview, export, saved frames and covers show the same thing. The project keeps only the effect's name and
//! values; a computer without the plugin opens and exports the project without the effect and says so.

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;

use naga::{AddressSpace, Expression, Handle, Statement, TypeInner};
use serde::{Deserialize, Serialize};

use crate::model::{Effect, EffectValue, Project};

/// The plugin API whose manifests may hold effects; the same as `nuzky_mcp::plugins::API`.
pub const API: u64 = 2;
pub const MAX_SOURCE_BYTES: u64 = 16 << 10;
/// Floats the params of one effect take together: a number or a toggle takes one, a colour three.
pub const MAX_PARAM_FLOATS: usize = 16;
pub const MAX_PLUGIN_EFFECTS: usize = 8;
/// What one pixel of an effect may cost: expressions over everything it calls, and reads of the picture.
pub const MAX_EXPRESSIONS: usize = 4096;
pub const MAX_SOURCE_READS: usize = 16;
/// The largest local or private variable, so no shader asks the GPU for megabytes per pixel.
const MAX_VARIABLE_BYTES: u32 = 1024;
const MAX_MANIFEST_BYTES: u64 = 64 << 10;

/// An effect's setting, the same shapes as a plugin's number, toggle and colour inputs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum EffectParam {
    Number {
        id: String,
        label: String,
        min: f64,
        max: f64,
        step: f64,
        default: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
    Toggle {
        id: String,
        label: String,
        default: bool,
    },
    Color {
        id: String,
        label: String,
        default: String,
    },
}

impl EffectParam {
    pub fn id(&self) -> &str {
        match self {
            Self::Number { id, .. } | Self::Toggle { id, .. } | Self::Color { id, .. } => id,
        }
    }

    fn label(&self) -> &str {
        match self {
            Self::Number { label, .. } | Self::Toggle { label, .. } | Self::Color { label, .. } => label,
        }
    }

    fn floats(&self) -> usize {
        if matches!(self, Self::Color { .. }) { 3 } else { 1 }
    }

    /// The value as drawn: `value` when it fits the param, else its default.
    fn bounded(&self, value: Option<&EffectValue>, out: &mut Vec<f32>) {
        match (self, value) {
            (Self::Number { min, max, .. }, Some(EffectValue::Number(v))) if v.is_finite() => {
                out.push(v.clamp(*min, *max) as f32)
            }
            (Self::Number { default, .. }, _) => out.push(*default as f32),
            (Self::Toggle { .. }, Some(EffectValue::Toggle(on))) => out.push(*on as u8 as f32),
            (Self::Toggle { default, .. }, _) => out.push(*default as u8 as f32),
            (Self::Color { default, .. }, value) => {
                let given = match value {
                    Some(EffectValue::Color(text)) if is_color(text) => text,
                    _ => default,
                };
                let [r, g, b, _] = crate::model::parse_color(given);
                out.extend([r, g, b]);
            }
        }
    }
}

/// One entry of a manifest's `effects`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub params: Vec<EffectParam>,
}

impl EffectDef {
    /// The params of `effect` bounded by this manifest, in the slots of `Pixel.params`.
    pub fn pack(&self, values: &BTreeMap<String, EffectValue>) -> [f32; MAX_PARAM_FLOATS] {
        let mut floats = Vec::with_capacity(MAX_PARAM_FLOATS);
        for param in &self.params {
            param.bounded(values.get(param.id()), &mut floats);
        }
        let mut out = [0.0; MAX_PARAM_FLOATS];
        for (slot, value) in out.iter_mut().zip(floats) {
            *slot = value;
        }
        out
    }
}

pub fn is_id(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn is_color(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn short(text: &str, max: usize) -> bool {
    !text.trim().is_empty() && text.chars().count() <= max && !text.contains('\n')
}

/// Checks a manifest's `effects`; the message names the field, as after "plugin.json: ".
pub fn check_defs(defs: &[EffectDef]) -> Result<(), String> {
    if defs.len() > MAX_PLUGIN_EFFECTS {
        return Err(format!("at most {MAX_PLUGIN_EFFECTS} effects"));
    }
    let mut ids = std::collections::HashSet::new();
    for def in defs {
        if !is_id(&def.id) || !ids.insert(def.id.as_str()) {
            return Err(format!(
                "effect id {:?} must be distinct, 1 to 40 lowercase letters, digits and dashes",
                def.id
            ));
        }
        if !short(&def.name, 40) {
            return Err(format!("the name of effect {} needs 1 to 40 characters", def.id));
        }
        let mut params = std::collections::HashSet::new();
        for param in &def.params {
            let id = param.id();
            if !(1..=40).contains(&id.len())
                || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                || !params.insert(id)
            {
                return Err(format!(
                    "param id {id:?} of effect {} must be distinct, 1 to 40 letters, digits and _",
                    def.id
                ));
            }
            if !short(param.label(), 40) {
                return Err(format!("the label of param {id} of effect {} needs 1 to 40 characters", def.id));
            }
            let fits = match param {
                EffectParam::Number { min, max, step, default, unit, .. } => {
                    min.is_finite()
                        && max.is_finite()
                        && min < max
                        && step.is_finite()
                        && *step > 0.0
                        && (min..=max).contains(&default)
                        && unit.as_ref().is_none_or(|unit| unit.chars().count() <= 12)
                }
                EffectParam::Toggle { .. } => true,
                EffectParam::Color { default, .. } => is_color(default),
            };
            if !fits {
                return Err(format!(
                    "param {id} of effect {} needs min below max, a positive step, a default between them, a unit of at most 12 characters and a colour written #RRGGBB",
                    def.id
                ));
            }
        }
        let floats: usize = def.params.iter().map(EffectParam::floats).sum();
        if floats > MAX_PARAM_FLOATS {
            return Err(format!(
                "the params of effect {} take {floats} floats; at most {MAX_PARAM_FLOATS} (a number or toggle takes 1, a colour 3)",
                def.id
            ));
        }
    }
    Ok(())
}

/// What the effect's function gets for each pixel, and the helpers it may call. Every name that starts with
/// `nuzky_` is Nuzky's.
const WRAPPER: &str = r#"
struct Pixel {
    uv: vec2<f32>,
    color: vec4<f32>,
    size: vec2<f32>,
    time: f32,
    seed: u32,
    params: array<f32, 16>,
};

struct nuzky_uniforms {
    to_texture: vec4<f32>,
    to_upright: vec4<f32>,
    offsets: vec4<f32>,
    info: vec4<f32>,
    seed: vec4<u32>,
    params: array<vec4<f32>, 4>,
};
@group(0) @binding(0) var<uniform> nuzky_effect: nuzky_uniforms;
@group(0) @binding(1) var nuzky_tex: texture_2d<f32>;

fn nuzky_texel(p: vec2<i32>, size: vec2<i32>) -> vec4<f32> {
    let c = textureLoad(nuzky_tex, clamp(p, vec2<i32>(0), size - 1), 0);
    return vec4<f32>(c.rgb * c.a, c.a);
}

fn nuzky_source(uv: vec2<f32>) -> vec4<f32> {
    let u = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let t = nuzky_effect.to_texture.xy * u.x + nuzky_effect.to_texture.zw * u.y + nuzky_effect.offsets.xy;
    let size = vec2<i32>(textureDimensions(nuzky_tex));
    let p = t * vec2<f32>(size) - 0.5;
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let top = mix(nuzky_texel(i, size), nuzky_texel(i + vec2<i32>(1, 0), size), f.x);
    let bottom = mix(nuzky_texel(i + vec2<i32>(0, 1), size), nuzky_texel(i + 1, size), f.x);
    let c = mix(top, bottom, f.y);
    var rgb = select(vec3<f32>(0.0), c.rgb / c.a, c.a > 0.0);
    if nuzky_effect.info.w > 0.0 {
        rgb = nuzky_hdr_to_sdr(rgb, nuzky_effect.info.w);
    }
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), c.a);
}

fn nuzky_mix3(v0: vec3<u32>) -> vec3<u32> {
    var v = v0 * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3<u32>(16u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

fn nuzky_hash(v: vec3<u32>) -> u32 {
    return nuzky_mix3(v).x;
}

fn nuzky_random(v: vec3<u32>) -> f32 {
    return f32(nuzky_hash(v) >> 8u) / 16777216.0;
}

fn nuzky_noise(p: vec2<f32>, seed: u32) -> f32 {
    let cell = floor(p);
    let f = p - cell;
    let c = bitcast<vec2<u32>>(vec2<i32>(cell));
    let a = nuzky_random(vec3<u32>(c, seed));
    let b = nuzky_random(vec3<u32>(c + vec2<u32>(1u, 0u), seed));
    let d = nuzky_random(vec3<u32>(c + vec2<u32>(0u, 1u), seed));
    let e = nuzky_random(vec3<u32>(c + vec2<u32>(1u, 1u), seed));
    let s = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, s.x), mix(d, e, s.x), s.y);
}

struct nuzky_vertex {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn nuzky_vs(@builtin(vertex_index) i: u32) -> nuzky_vertex {
    let corner = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    var out: nuzky_vertex;
    out.pos = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = corner;
    return out;
}

@fragment
fn nuzky_fs(in: nuzky_vertex) -> @location(0) vec4<f32> {
    var p: Pixel;
    p.uv = nuzky_effect.to_upright.xy * in.uv.x + nuzky_effect.to_upright.zw * in.uv.y + nuzky_effect.offsets.zw;
    p.color = nuzky_source(p.uv);
    p.size = nuzky_effect.info.xy;
    p.time = nuzky_effect.info.z;
    p.seed = nuzky_effect.seed.x;
    for (var k = 0; k < 16; k += 1) {
        p.params[k] = nuzky_effect.params[k / 4][k % 4];
    }
    let c = effect(p);
    // NaN and infinity have every exponent bit set; a compiler may fold `c == c` away.
    let finite = (bitcast<vec4<u32>>(c) & vec4<u32>(0x7f800000u)) != vec4<u32>(0x7f800000u);
    return clamp(select(vec4<f32>(0.0), c, finite), vec4<f32>(0.0), vec4<f32>(1.0));
}
"#;

/// The whole shader of an effect: its function first, so naga's lines are the file's.
pub fn shader(source: &str) -> String {
    format!("{source}\n{WRAPPER}{}", crate::gpu::HDR_WGSL)
}

/// Names the wrapper declares, so whatever else a shader declares is the plugin's.
static NUZKY_NAMES: LazyLock<std::collections::HashSet<String>> = LazyLock::new(|| {
    let module = naga::front::wgsl::parse_str(&shader("fn effect(p: Pixel) -> vec4<f32> { return p.color; }"))
        .expect("the effect wrapper parses");
    let mut names: std::collections::HashSet<String> = declared(&module).map(|(name, _)| name.to_owned()).collect();
    names.remove("effect");
    names
});

enum Item {
    Function(Handle<naga::Function>),
    Other(naga::Span),
}

fn declared(module: &naga::Module) -> impl Iterator<Item = (&str, Item)> {
    let functions = module.functions.iter().filter_map(|(h, f)| Some((f.name.as_deref()?, Item::Function(h))));
    let constants = module
        .constants
        .iter()
        .filter_map(|(h, c)| Some((c.name.as_deref()?, Item::Other(module.constants.get_span(h)))));
    let globals = module
        .global_variables
        .iter()
        .filter_map(|(h, g)| Some((g.name.as_deref()?, Item::Other(module.global_variables.get_span(h)))));
    let types =
        module.types.iter().filter_map(|(h, t)| Some((t.name.as_deref()?, Item::Other(module.types.get_span(h)))));
    functions.chain(constants).chain(globals).chain(types)
}

/// Where in the effect's file, as `line:column: `, when the span lies in it.
fn at(full: &str, source_len: usize, span: naga::Span) -> String {
    match span.to_range() {
        Some(range) if range.start < source_len => {
            let location = span.location(full);
            format!("{}:{}: ", location.line_number, location.line_position)
        }
        _ => String::new(),
    }
}

fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(inner) = source {
        let more = inner.to_string();
        if !text.contains(&more) {
            text = format!("{text}: {more}");
        }
        source = inner.source();
    }
    text
}

/// Checks an effect's WGSL as `save_plugin` and the renderer do. The error starts with `line:column: ` when it
/// points into the file.
pub fn check(source: &str) -> Result<(), String> {
    if source.len() as u64 > MAX_SOURCE_BYTES {
        return Err(format!("larger than {} KB", MAX_SOURCE_BYTES >> 10));
    }
    reserved_names(source)?;
    let full = shader(source);
    let module = naga::front::wgsl::parse_str(&full)
        .map_err(|e| located(&full, source.len(), e.labels().map(|(span, _)| span), e.message()))?;
    rules(&module, &full, source.len())?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
        .validate(&module)
        .map_err(|e| located(&full, source.len(), e.spans().map(|(span, _)| *span), &chain(e.as_inner())))?;
    Ok(())
}

/// The message at the first of its places in the effect's file. One only in Nuzky's part of the shader means the
/// effect is missing or takes a name of Nuzky's.
fn located(full: &str, source_len: usize, spans: impl Iterator<Item = naga::Span>, message: &str) -> String {
    match spans.map(|span| at(full, source_len, span)).find(|place| !place.is_empty()) {
        Some(place) => format!("{place}{message}"),
        None => format!(
            "{message} (an effect is fn effect(p: Pixel) -> vec4<f32> and the functions it calls; Pixel and names starting with nuzky_ are Nuzky's)"
        ),
    }
}

/// Refuses a constant or variable the file names with `nuzky_`, which naga may fold away before `rules` sees it.
fn reserved_names(source: &str) -> Result<(), String> {
    let mut code = source.as_bytes().to_vec();
    // Comments become spaces, so offsets and lines stay the file's.
    let mut i = 0;
    let mut depth = 0;
    while i < code.len() {
        if depth == 0 && code[i..].starts_with(b"//") {
            while i < code.len() && code[i] != b'\n' {
                code[i] = b' ';
                i += 1;
            }
        } else if code[i..].starts_with(b"/*") {
            depth += 1;
            code[i..i + 2].fill(b' ');
            i += 2;
        } else if depth > 0 && code[i..].starts_with(b"*/") {
            depth -= 1;
            code[i..i + 2].fill(b' ');
            i += 2;
        } else {
            if depth > 0 && code[i] != b'\n' {
                code[i] = b' ';
            }
            i += 1;
        }
    }
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < code.len() {
        if word(code[i]) {
            let start = i;
            while i < code.len() && word(code[i]) {
                i += 1;
            }
            tokens.push((start, &code[start..i]));
        } else {
            if !code[i].is_ascii_whitespace() {
                tokens.push((i, &code[i..i + 1]));
            }
            i += 1;
        }
    }
    let mut k = 0;
    while k + 1 < tokens.len() {
        let mut next = k + 1;
        if tokens[k].1 == b"var" && tokens[next].1 == b"<" {
            while next < tokens.len() && tokens[next].1 != b">" {
                next += 1;
            }
            next += 1;
        }
        let declares = matches!(tokens[k].1, b"const" | b"override" | b"alias" | b"let" | b"var");
        if let Some(&(at, name)) = tokens.get(next).filter(|(_, name)| declares && name.starts_with(b"nuzky_")) {
            let line = source[..at].matches('\n').count() + 1;
            let column = at - source[..at].rfind('\n').map_or(0, |n| n + 1) + 1;
            return Err(format!(
                "{line}:{column}: names starting with nuzky_ are Nuzky's; rename {}",
                String::from_utf8_lossy(name)
            ));
        }
        k += 1;
    }
    Ok(())
}

fn rules(module: &naga::Module, full: &str, source_len: usize) -> Result<(), String> {
    let at = |span| at(full, source_len, span);
    let mut entry_points: Vec<&str> = module.entry_points.iter().map(|e| e.name.as_str()).collect();
    entry_points.sort_unstable();
    if entry_points != ["nuzky_fs", "nuzky_vs"] {
        return Err("an effect has no entry points of its own: write fn effect(p: Pixel) -> vec4<f32>".into());
    }
    if let Some((h, _)) = module.overrides.iter().next() {
        return Err(format!("{}override constants are not allowed", at(module.overrides.get_span(h))));
    }
    for (name, item) in declared(module) {
        if name.starts_with("nuzky_") && !NUZKY_NAMES.contains(name) {
            let span = match item {
                Item::Function(h) => module.functions.get_span(h),
                Item::Other(span) => span,
            };
            return Err(format!("{}names starting with nuzky_ are Nuzky's; rename {name}", at(span)));
        }
    }
    let ctx = module.to_ctx();
    for (h, global) in module.global_variables.iter() {
        if NUZKY_NAMES.contains(global.name.as_deref().unwrap_or_default()) {
            continue;
        }
        let span = module.global_variables.get_span(h);
        if global.space != AddressSpace::Private {
            return Err(format!("{}an effect reads no buffers or textures; only var<private> globals", at(span)));
        }
        if module.types[global.ty].inner.size(ctx) > MAX_VARIABLE_BYTES {
            return Err(format!("{}a variable takes at most {MAX_VARIABLE_BYTES} bytes", at(span)));
        }
    }
    let source = module.functions.iter().find(|(_, f)| f.name.as_deref() == Some("nuzky_source")).map(|(h, _)| h);
    let pixel = module.types.iter().find(|(_, t)| t.name.as_deref() == Some("Pixel")).map(|(h, _)| h);
    let mut effect = None;
    // Callees come before their callers in the arena, so each function's cost is known when a caller needs it.
    let mut cost: HashMap<Handle<naga::Function>, (usize, usize)> = HashMap::new();
    for (h, function) in module.functions.iter() {
        let name = function.name.as_deref().unwrap_or_default();
        let ours = NUZKY_NAMES.contains(name);
        if !ours {
            for (_, local) in function.local_variables.iter() {
                if module.types[local.ty].inner.size(ctx) > MAX_VARIABLE_BYTES {
                    return Err(format!(
                        "{}a variable takes at most {MAX_VARIABLE_BYTES} bytes",
                        at(module.functions.get_span(h))
                    ));
                }
            }
            for (e, expression) in function.expressions.iter() {
                let forbidden = match expression {
                    Expression::ImageSample { .. } | Expression::ImageLoad { .. } | Expression::ImageQuery { .. } => {
                        Some("textures; read the picture with nuzky_source(uv)")
                    }
                    Expression::GlobalVariable(g) if module.global_variables[*g].space != AddressSpace::Private => {
                        Some("Nuzky's texture or settings; read the picture with nuzky_source(uv)")
                    }
                    Expression::Derivative { .. } => Some("derivatives, which differ between GPUs"),
                    Expression::Literal(_)
                    | Expression::Constant(_)
                    | Expression::ZeroValue(_)
                    | Expression::Compose { .. }
                    | Expression::Access { .. }
                    | Expression::AccessIndex { .. }
                    | Expression::Splat { .. }
                    | Expression::Swizzle { .. }
                    | Expression::FunctionArgument(_)
                    | Expression::GlobalVariable(_)
                    | Expression::LocalVariable(_)
                    | Expression::Load { .. }
                    | Expression::Unary { .. }
                    | Expression::Binary { .. }
                    | Expression::Select { .. }
                    | Expression::Relational { .. }
                    | Expression::Math { .. }
                    | Expression::As { .. }
                    | Expression::CallResult(_) => None,
                    _ => Some("this operation"),
                };
                if let Some(what) = forbidden {
                    return Err(format!("{}an effect cannot use {what}", at(function.expressions.get_span(e))));
                }
            }
            statements(&function.body, &at)?;
            if name == "effect" {
                effect = Some((h, function));
            }
        }
        let mut total = (function.expressions.len(), usize::from(Some(h) == source));
        calls(&function.body, &mut |callee| {
            let (expressions, reads) = cost.get(&callee).copied().unwrap_or_default();
            total = (total.0.saturating_add(expressions), total.1.saturating_add(reads));
        });
        cost.insert(h, total);
    }
    let (h, function) = effect.ok_or("needs fn effect(p: Pixel) -> vec4<f32>")?;
    let span = module.functions.get_span(h);
    let vec4 = |ty: Handle<naga::Type>| {
        matches!(module.types[ty].inner, TypeInner::Vector { size: naga::VectorSize::Quad, scalar: naga::Scalar::F32 })
    };
    if function.arguments.len() != 1
        || Some(function.arguments[0].ty) != pixel
        || !function.result.as_ref().is_some_and(|r| vec4(r.ty))
    {
        return Err(format!("{}effect must be fn effect(p: Pixel) -> vec4<f32>", at(span)));
    }
    let (expressions, reads) = cost[&h];
    if reads > MAX_SOURCE_READS {
        return Err(format!(
            "{}effect reads the picture {reads} times per pixel through nuzky_source; at most {MAX_SOURCE_READS}",
            at(span)
        ));
    }
    if expressions > MAX_EXPRESSIONS {
        return Err(format!(
            "{}effect takes {expressions} operations per pixel with what it calls; at most {MAX_EXPRESSIONS}",
            at(span)
        ));
    }
    Ok(())
}

fn statements(block: &naga::Block, at: &impl Fn(naga::Span) -> String) -> Result<(), String> {
    for (statement, span) in block.span_iter() {
        let forbidden = match statement {
            Statement::Block(inner) => {
                statements(inner, at)?;
                None
            }
            Statement::If { accept, reject, .. } => {
                statements(accept, at)?;
                statements(reject, at)?;
                None
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    statements(&case.body, at)?;
                }
                None
            }
            Statement::Loop { .. } | Statement::Continue => {
                Some("loops; write each step out, or call a function for it")
            }
            Statement::Kill => Some("discard; return a transparent colour instead"),
            Statement::Emit(_)
            | Statement::Break
            | Statement::Return { .. }
            | Statement::Store { .. }
            | Statement::Call { .. } => None,
            _ => Some("barriers, atomics or writes to textures"),
        };
        if let Some(what) = forbidden {
            return Err(format!("{}an effect cannot use {what}", at(*span)));
        }
    }
    Ok(())
}

fn calls(block: &naga::Block, each: &mut impl FnMut(Handle<naga::Function>)) {
    for statement in block.iter() {
        match statement {
            Statement::Call { function, .. } => each(*function),
            Statement::Block(inner) => calls(inner, each),
            Statement::If { accept, reject, .. } => {
                calls(accept, each);
                calls(reject, each);
            }
            Statement::Switch { cases, .. } => cases.iter().for_each(|case| calls(&case.body, each)),
            Statement::Loop { body, continuing, .. } => {
                calls(body, each);
                calls(continuing, each);
            }
            _ => {}
        }
    }
}

/// An installed effect ready to draw.
#[derive(Debug)]
pub struct Loaded {
    pub def: EffectDef,
    /// The plugin's version.
    pub version: String,
    /// The hash of its shader: the key of its pipeline and of its place on the broken list.
    pub key: u64,
    pub shader: Arc<str>,
}

#[derive(Clone, Debug)]
pub enum Lookup {
    Ready(Arc<Loaded>),
    /// The plugin, the effect in its manifest, or its file is not there.
    Missing,
    /// It is there but does not load or draw; why, naming the file.
    Broken(String),
}

type Stamp = Vec<Option<(SystemTime, u64)>>;

/// Effects as last read, with the times and sizes of their files, so a change to a plugin is read again.
type Library = HashMap<(PathBuf, String, String), (Stamp, Lookup)>;
static LIBRARY: LazyLock<Mutex<Library>> = LazyLock::new(Mutex::default);
/// Effects that timed out, lost the GPU or failed to draw in this process, by key, with why.
static BROKEN: LazyLock<Mutex<HashMap<u64, String>>> = LazyLock::new(Mutex::default);

/// The folder of the user's plugins in Nuzky's data.
pub fn plugins_dir() -> Option<PathBuf> {
    crate::data_dir().map(|dir| dir.join("plugins"))
}

/// The effect as installed under `dir` now, and whether it drew in this process.
pub fn lookup(dir: &Path, plugin: &str, effect: &str) -> Lookup {
    if !is_id(plugin) || !is_id(effect) {
        return Lookup::Missing;
    }
    let folder = dir.join(plugin);
    let files = [folder.clone(), folder.join("plugin.json"), folder.join("effects"), effect_file(&folder, effect)];
    let stamp: Stamp = files
        .iter()
        .map(|file| {
            let meta = std::fs::symlink_metadata(file).ok()?;
            Some((meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len()))
        })
        .collect();
    let key = (dir.to_path_buf(), plugin.to_owned(), effect.to_owned());
    let cached = LIBRARY.lock().unwrap().get(&key).filter(|(s, _)| *s == stamp).map(|(_, lookup)| lookup.clone());
    let lookup = cached.unwrap_or_else(|| {
        let lookup = load(&folder, plugin, effect);
        LIBRARY.lock().unwrap().insert(key, (stamp, lookup.clone()));
        lookup
    });
    match lookup {
        Lookup::Ready(loaded) => match BROKEN.lock().unwrap().get(&loaded.key) {
            Some(why) => Lookup::Broken(why.clone()),
            None => Lookup::Ready(loaded),
        },
        other => other,
    }
}

fn effect_file(folder: &Path, effect: &str) -> PathBuf {
    folder.join("effects").join(format!("{effect}.wgsl"))
}

/// A plugin's own file; a link is refused, as for its script.
fn read(path: &Path, max: u64) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| "missing".to_owned())?;
    if !meta.is_file() {
        return Err("not a plain file".into());
    }
    if meta.len() > max {
        return Err(format!("larger than {} KB", max >> 10));
    }
    std::fs::read_to_string(path).map_err(|_| "unreadable".into())
}

/// The effects a plugin's manifest declares, checked, and the plugin's version.
fn manifest_effects(folder: &Path) -> Result<(Vec<EffectDef>, String), String> {
    let text = read(&folder.join("plugin.json"), MAX_MANIFEST_BYTES).map_err(|e| format!("plugin.json: {e}"))?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("plugin.json: {e}"))?;
    if value.get("nuzkyPlugin").and_then(serde_json::Value::as_u64) != Some(API) {
        return Err("plugin.json: made for another version of Nuzky".into());
    }
    let effects = match value.get("effects") {
        None => Vec::new(),
        Some(effects) => Vec::<EffectDef>::deserialize(effects).map_err(|e| format!("plugin.json: effects: {e}"))?,
    };
    check_defs(&effects).map_err(|e| format!("plugin.json: {e}"))?;
    let version = value.get("version").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
    Ok((effects, version))
}

fn load(folder: &Path, plugin: &str, effect: &str) -> Lookup {
    if !std::fs::symlink_metadata(folder).is_ok_and(|meta| meta.is_dir()) {
        return Lookup::Missing;
    }
    let (defs, version) = match manifest_effects(folder) {
        Ok(found) => found,
        Err(why) => return Lookup::Broken(why),
    };
    let Some(def) = defs.into_iter().find(|def| def.id == effect) else { return Lookup::Missing };
    let file = format!("effects/{effect}.wgsl");
    if !std::fs::symlink_metadata(folder.join("effects")).is_ok_and(|meta| meta.is_dir()) {
        return Lookup::Broken(format!("{file}: missing"));
    }
    let source = match read(&effect_file(folder, effect), MAX_SOURCE_BYTES) {
        Ok(source) => source,
        Err(why) => return Lookup::Broken(format!("{file}: {why}")),
    };
    if let Err(why) = check(&source) {
        let separator = if why.starts_with(|c: char| c.is_ascii_digit()) { ":" } else { ": " };
        return Lookup::Broken(format!("{file}{separator}{why}"));
    }
    let shader: Arc<str> = shader(&source).into();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (plugin, effect, &*shader).hash(&mut hasher);
    Lookup::Ready(Arc::new(Loaded { def, version, key: hasher.finish(), shader }))
}

/// Keeps an effect from drawing again in this process, in every renderer.
pub fn mark_broken(key: u64, why: &str) {
    if let std::collections::hash_map::Entry::Vacant(entry) = BROKEN.lock().unwrap().entry(key) {
        log::error!("A plugin effect stopped drawing: {why}");
        entry.insert(why.to_owned());
    }
}

/// An effect on the project's clips that this computer draws without, and why.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectIssue {
    pub plugin: String,
    pub effect: String,
    pub name: String,
    /// What is wrong with an installed effect; `None` when it is not installed.
    pub broken: Option<String>,
    pub clip_ids: Vec<String>,
    /// `sentence`, for the app.
    pub message: String,
}

impl EffectIssue {
    /// One sentence for the user.
    pub fn sentence(&self) -> String {
        match &self.broken {
            None => format!("{} is not installed on this computer, so the clip is drawn without it.", self.name),
            Some(why) => format!("{} has an error, so the clip is drawn without it: {why}", self.name),
        }
    }
}

/// Every effect on the project's clips that is not installed or does not draw, in the order the clips come.
pub fn issues(project: &Project) -> Vec<EffectIssue> {
    issues_in(project, plugins_dir().as_deref())
}

pub fn issues_in(project: &Project, dir: Option<&Path>) -> Vec<EffectIssue> {
    let mut out: Vec<EffectIssue> = Vec::new();
    for clip in project.tracks.iter().flat_map(|t| &t.clips) {
        for effect in clip.effects.iter().take(crate::model::MAX_CLIP_EFFECTS) {
            let broken = match dir.map(|dir| lookup(dir, &effect.plugin, &effect.effect)) {
                Some(Lookup::Ready(_)) => continue,
                None | Some(Lookup::Missing) => None,
                Some(Lookup::Broken(why)) => Some(why),
            };
            match out.iter_mut().find(|i| i.plugin == effect.plugin && i.effect == effect.effect) {
                Some(issue) => {
                    if !issue.clip_ids.contains(&clip.id) {
                        issue.clip_ids.push(clip.id.clone());
                    }
                }
                None => out.push(EffectIssue {
                    plugin: effect.plugin.clone(),
                    effect: effect.effect.clone(),
                    name: display_name(effect),
                    broken,
                    clip_ids: vec![clip.id.clone()],
                    message: String::new(),
                }),
            }
        }
    }
    for issue in &mut out {
        issue.message = issue.sentence();
    }
    out
}

/// "Exported without VHS: it isn't installed on this computer.", from the names an export was drawn without and
/// the project's `issues`; a name they no longer list failed while it drew.
pub fn exported_without(names: &[String], issues: &[EffectIssue]) -> String {
    if names.is_empty() {
        return String::new();
    }
    let (missing, broken): (Vec<&String>, Vec<&String>) =
        names.iter().partition(|name| issues.iter().any(|i| &i.name == *name && i.broken.is_none()));
    let all: Vec<&String> = names.iter().collect();
    let why = match (missing.len(), broken.len()) {
        (1, 0) => "it isn't installed on this computer".to_owned(),
        (_, 0) => "they aren't installed on this computer".to_owned(),
        (0, 1) => "it has an error".to_owned(),
        (0, _) => "they have errors".to_owned(),
        (m, b) => format!(
            "{} {} installed on this computer, and {} {}",
            listed(&missing),
            if m == 1 { "isn't" } else { "aren't" },
            listed(&broken),
            if b == 1 { "has an error" } else { "have errors" }
        ),
    };
    format!("Exported without {}: {why}.", listed(&all))
}

/// "VHS, Glitch and Grain".
fn listed(names: &[&String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", ")),
    }
}

/// What to call an effect in a message.
pub fn display_name(effect: &Effect) -> String {
    if short(&effect.name, 40) { effect.name.clone() } else { effect.effect.clone() }
}

/// A clip's seed: the same for the clip wherever it is drawn, different for another clip.
pub fn seed(clip_id: &str) -> u32 {
    // FNV-1a, fixed so a seed never changes between versions or computers.
    clip_id.bytes().fold(0x811c_9dc5_u32, |hash, byte| (hash ^ byte as u32).wrapping_mul(0x0100_0193))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = "fn effect(p: Pixel) -> vec4<f32> {\n    return vec4<f32>(1.0 - p.color.rgb, p.color.a);\n}\n";

    fn error(source: &str) -> String {
        check(source).expect_err("the effect should be refused")
    }

    #[test]
    fn the_guide_example_passes() {
        check(include_str!("../../../skills/nuzky-plugin/examples/vhs/effects/vhs.wgsl")).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../skills/nuzky-plugin/examples/vhs/plugin.json")).unwrap();
        check_defs(&Vec::<EffectDef>::deserialize(&manifest["effects"]).unwrap()).unwrap();
    }

    #[test]
    fn a_plain_effect_with_helpers_passes() {
        check(OK).unwrap();
        check(
            "var<private> shift: f32 = 0.5;
const LINES = 3.0;
fn wave(y: f32) -> f32 { return sin(y * LINES); }
fn effect(p: Pixel) -> vec4<f32> {
    let n = nuzky_noise(p.uv * p.size / 8.0, p.seed) + nuzky_random(vec3<u32>(1u, 2u, p.seed));
    var c = nuzky_source(p.uv + vec2<f32>(wave(p.uv.y) * shift * p.params[0], 0.0));
    if p.time > 1.0 { c = vec4<f32>(c.rgb * n, c.a); }
    switch nuzky_hash(vec3<u32>(p.seed)) % 3u { case 0u: { return c; } default: { return p.color; } }
}",
        )
        .unwrap();
    }

    #[test]
    fn syntax_errors_point_at_the_line_of_the_file() {
        let e = error("fn effect(p: Pixel) -> vec4<f32> {\n    return p.color\n}\n");
        assert!(e.starts_with("3:1: ") || e.starts_with("2:"), "{e}");
    }

    #[test]
    fn loops_are_refused() {
        let e = error(
            "fn effect(p: Pixel) -> vec4<f32> {\n    var c = p.color;\n    for (var i = 0; i < 4; i++) { c *= 0.9; }\n    return c;\n}\n",
        );
        assert!(e.starts_with("3:") && e.contains("loops"), "{e}");
        assert!(error("fn effect(p: Pixel) -> vec4<f32> {\n  loop { break; }\n  return p.color;\n}").contains("loops"));
        assert!(
            error("fn effect(p: Pixel) -> vec4<f32> {\n  var i = 0;\n  while i < 3 { i++; }\n  return p.color;\n}")
                .contains("loops")
        );
    }

    #[test]
    fn globals_other_than_private_and_resources_are_refused() {
        for global in [
            "@group(0) @binding(2) var<uniform> mine: vec4<f32>;",
            "@group(1) @binding(0) var<storage, read> data: array<f32>;",
            "var<workgroup> shared_data: array<f32, 4>;",
            "@group(0) @binding(3) var other: texture_2d<f32>;",
        ] {
            let e = error(&format!("{global}\n{OK}"));
            assert!(
                e.contains("only var<private>") || e.contains("Capabilit") || e.contains("workgroup"),
                "{global}: {e}"
            );
        }
        let e = error(
            "@group(0) @binding(2) var<uniform> mine: vec4<f32>;\nfn effect(p: Pixel) -> vec4<f32> { return mine; }",
        );
        assert!(e.starts_with("1:"), "{e}");
    }

    #[test]
    fn overrides_and_entry_points_are_refused() {
        assert!(error(&format!("override strength: f32 = 1.0;\n{OK}")).contains("override"));
        let e = error(&format!("@fragment fn main() -> @location(0) vec4<f32> {{ return vec4<f32>(1.0); }}\n{OK}"));
        assert!(e.contains("entry points"), "{e}");
    }

    #[test]
    fn nuzky_names_are_refused() {
        let e = error(&format!("fn nuzky_blur(x: f32) -> f32 {{ return x; }}\n{OK}"));
        assert!(e.starts_with("1:") && e.contains("nuzky_"), "{e}");
        let constant = error("const nuzky_k = 0.5;\nfn effect(p: Pixel) -> vec4<f32> { return p.color * nuzky_k; }");
        assert!(constant.starts_with("1:7: ") && constant.contains("rename nuzky_k"), "{constant}");
        let local =
            error("fn effect(p: Pixel) -> vec4<f32> {\n  var<function> nuzky_c = p.color;\n  return nuzky_c;\n}");
        assert!(local.starts_with("2:17: "), "{local}");
        // Names in comments and calls of Nuzky's helpers are fine.
        check("// let nuzky_x = 1;\n/* const nuzky_y = 2; */\nfn effect(p: Pixel) -> vec4<f32> { let n = nuzky_noise(p.uv, 1u); return p.color * n; }")
            .unwrap();
        // Nuzky's own names are taken already.
        let taken =
            error(&format!("fn nuzky_source(uv: vec2<f32>) -> vec4<f32> {{ return vec4<f32>(uv, 0.0, 1.0); }}\n{OK}"));
        assert!(taken.starts_with("1:4: redefinition"), "{taken}");
    }

    #[test]
    fn textures_barriers_atomics_and_derivatives_are_refused() {
        let texture =
            error("fn effect(p: Pixel) -> vec4<f32> {\n    return textureLoad(nuzky_tex, vec2<i32>(0), 0);\n}\n");
        assert!(texture.starts_with("2:") && texture.contains("nuzky_source(uv)"), "{texture}");
        let uniform = error("fn effect(p: Pixel) -> vec4<f32> {\n    return nuzky_effect.info;\n}\n");
        assert!(uniform.starts_with("2:") && uniform.contains("Nuzky's texture or settings"), "{uniform}");
        assert!(
            error("fn effect(p: Pixel) -> vec4<f32> {\n    return p.color * dpdx(p.uv.x);\n}\n")
                .contains("derivatives")
        );
        assert!(
            !error("fn effect(p: Pixel) -> vec4<f32> {\n    workgroupBarrier();\n    return p.color;\n}\n").is_empty()
        );
        assert!(
            error("fn effect(p: Pixel) -> vec4<f32> {\n    if p.uv.x > 0.5 { discard; }\n    return p.color;\n}\n")
                .contains("discard")
        );
    }

    #[test]
    fn the_effect_function_must_exist_with_its_signature() {
        assert!(error("fn look(p: Pixel) -> vec4<f32> { return p.color; }").contains("fn effect"));
        assert!(error("fn effect(p: Pixel) -> vec3<f32> { return p.color.rgb; }").contains("fn effect(p: Pixel)"));
        assert!(
            error("fn effect(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(uv, 0.0, 1.0); }")
                .contains("fn effect(p: Pixel)")
        );
    }

    #[test]
    fn cost_bounds_cover_what_the_effect_calls() {
        let reads = |n: usize| {
            let taps: Vec<String> =
                (0..n).map(|i| format!("nuzky_source(p.uv + vec2<f32>({i}.0 * 0.001, 0.0))")).collect();
            format!("fn effect(p: Pixel) -> vec4<f32> {{\n    return ({}) / {n}.0;\n}}\n", taps.join(" + "))
        };
        check(&reads(16)).unwrap();
        assert!(error(&reads(17)).contains("17 times"), "{}", error(&reads(17)));
        // Reads hidden in a helper count once per call.
        let helper = "fn tap(uv: vec2<f32>) -> vec4<f32> { return nuzky_source(uv) + nuzky_source(uv.yx); }\n\
            fn effect(p: Pixel) -> vec4<f32> {\n    return tap(p.uv) + tap(p.uv * 0.5) + tap(p.uv * 0.25) + tap(p.uv * 0.1) + tap(p.uv * 0.2) + tap(p.uv * 0.3) + tap(p.uv * 0.4) + tap(p.uv * 0.6) + tap(p.uv * 0.7);\n}\n";
        assert!(error(helper).contains("18 times"), "{}", error(helper));
        let mut heavy = String::from("fn step(x: f32) -> f32 { return ");
        heavy += &(0..200).map(|i| format!("sin(x * {i}.0)")).collect::<Vec<_>>().join(" + ");
        heavy += "; }\nfn effect(p: Pixel) -> vec4<f32> {\n    return vec4<f32>(";
        heavy += &(0..12).map(|_| "step(p.uv.x)").collect::<Vec<_>>().join(" + ");
        heavy += ");\n}\n";
        assert!(error(&heavy).contains("operations per pixel"), "{}", error(&heavy));
    }

    #[test]
    fn big_variables_and_files_are_refused() {
        let e =
            error("fn effect(p: Pixel) -> vec4<f32> {\n    var big: array<vec4<f32>, 512>;\n    return p.color;\n}\n");
        assert!(e.contains("bytes"), "{e}");
        assert!(error(&format!("{OK}//{}", "x".repeat(17 << 10))).contains("16 KB"));
    }

    #[test]
    fn params_pack_in_manifest_order_bounded_by_it() {
        let def: EffectDef = serde_json::from_value(serde_json::json!({
            "id": "vhs", "name": "VHS", "params": [
                {"type": "number", "id": "amount", "label": "Amount", "min": 0, "max": 1, "step": 0.1, "default": 0.5},
                {"type": "color", "id": "tint", "label": "Tint", "default": "#FF0000"},
                {"type": "toggle", "id": "lines", "label": "Lines", "default": true}
            ]
        }))
        .unwrap();
        check_defs(std::slice::from_ref(&def)).unwrap();
        let values = BTreeMap::from([
            ("amount".to_owned(), EffectValue::Number(7.0)),
            ("tint".to_owned(), EffectValue::Toggle(false)),
            ("gone".to_owned(), EffectValue::Number(1.0)),
        ]);
        assert_eq!(&def.pack(&values)[..6], &[1.0, 1.0, 0.0, 0.0, 1.0, 0.0]);
        let too_many = EffectDef {
            params: (0..6)
                .map(|i| EffectParam::Color { id: format!("c{i}"), label: "C".into(), default: "#000000".into() })
                .collect(),
            ..def
        };
        assert!(check_defs(&[too_many]).unwrap_err().contains("18 floats"));
    }

    #[test]
    fn an_export_says_which_effects_are_missing_and_which_have_errors() {
        let issue = |name: &str, broken: Option<&str>| EffectIssue {
            plugin: name.to_lowercase(),
            effect: name.to_lowercase(),
            name: name.into(),
            broken: broken.map(Into::into),
            clip_ids: vec![],
            message: String::new(),
        };
        let issues = [issue("VHS", None), issue("Grain", None), issue("Glitch", Some("effects/glitch.wgsl: time-out"))];
        let said = |names: &[&str]| exported_without(&names.iter().map(|n| n.to_string()).collect::<Vec<_>>(), &issues);
        assert_eq!(said(&["VHS"]), "Exported without VHS: it isn't installed on this computer.");
        assert_eq!(said(&["VHS", "Grain"]), "Exported without VHS and Grain: they aren't installed on this computer.");
        assert_eq!(said(&["Glitch"]), "Exported without Glitch: it has an error.");
        // A name the issues no longer list stopped drawing during the export.
        assert_eq!(said(&["Glitch", "Fixed"]), "Exported without Glitch and Fixed: they have errors.");
        assert_eq!(
            said(&["VHS", "Grain", "Glitch"]),
            "Exported without VHS, Grain and Glitch: VHS and Grain aren't installed on this computer, and Glitch has an error."
        );
        assert_eq!(said(&[]), "");
    }
}

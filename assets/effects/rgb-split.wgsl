// Red and blue drift apart from green, up to 24 canvas pixels each way, along the angle.
fn effect(p: Pixel) -> vec4<f32> {
    let a = radians(p.params[1]);
    let d = vec2<f32>(cos(a), sin(a)) * p.params[0] * 24.0 / p.size;
    let r = nuzky_source(p.uv + d);
    let b = nuzky_source(p.uv - d);
    return vec4<f32>(r.r, p.color.g, b.b, max(p.color.a, max(r.a, b.a)));
}

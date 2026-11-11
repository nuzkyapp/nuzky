// A handheld camera shake: the picture drifts on smooth noise of the clip's time, zoomed in just enough that its
// edges never show. Speed 1 moves about six times a second.
fn wobble(t: f32, row: f32, seed: u32) -> f32 {
    return nuzky_noise(vec2<f32>(t, row), seed) + 0.5 * nuzky_noise(vec2<f32>(t * 2.3, row + 5.0), seed) - 0.75;
}

fn effect(p: Pixel) -> vec4<f32> {
    let amount = p.params[0];
    let t = p.time * p.params[1] * 6.0;
    let offset = vec2<f32>(wobble(t, 0.5, p.seed), wobble(t, 20.5, p.seed)) * 0.04 * amount;
    let zoom = 1.0 + 0.13 * amount;
    return nuzky_source(vec2<f32>(0.5) + (p.uv - vec2<f32>(0.5)) / zoom + offset);
}

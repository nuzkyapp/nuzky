// Film grain in grains of size canvas pixels that change 24 times a second, lighter in the middle tones.
fn effect(p: Pixel) -> vec4<f32> {
    let frame = u32(max(p.time, 0.0) * 24.0);
    let cell = vec2<u32>(p.uv * p.size / p.params[1]);
    let n = nuzky_random(vec3<u32>(cell, frame ^ p.seed)) - 0.5;
    let luma = dot(p.color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let weight = 0.35 + 0.65 * (1.0 - abs(luma * 2.0 - 1.0));
    return vec4<f32>(p.color.rgb + n * 0.3 * p.params[0] * weight, p.color.a);
}

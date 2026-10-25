// A worn video tape: red and blue drift apart, every other row is darker, and grain changes every frame.
// params: 0 amount, 1 scanlines (0 or 1), 2..4 tint (red, green, blue).
fn effect(p: Pixel) -> vec4<f32> {
    let amount = p.params[0];
    let tint = vec3<f32>(p.params[2], p.params[3], p.params[4]);
    // Up to 4 canvas pixels sideways, so the preview and the export drift alike.
    let shift = vec2<f32>(4.0 * amount / p.size.x, 0.0);
    var rgb = vec3<f32>(nuzky_source(p.uv + shift).r, p.color.g, nuzky_source(p.uv - shift).b);
    // Bands 6 canvas pixels high across the picture as shown.
    let row = u32(floor(p.uv.y * p.size.y / 6.0));
    rgb *= 1.0 - p.params[1] * amount * 0.45 * f32(row % 2u);
    // Grain from the clip's seed and the frame, the same on every GPU.
    let frame = u32(p.time * 30.0);
    let cell = vec2<u32>(p.uv * p.size / 2.0);
    rgb += (nuzky_random(vec3<u32>(cell, frame ^ p.seed)) - 0.5) * 0.15 * amount;
    rgb = mix(rgb, rgb * tint, 0.4 * amount);
    return vec4<f32>(rgb, p.color.a);
}

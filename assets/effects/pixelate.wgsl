// Square blocks of size canvas pixels, each the colour at its middle.
fn effect(p: Pixel) -> vec4<f32> {
    let cells = max(p.size / p.params[0], vec2<f32>(1.0));
    return nuzky_source((floor(p.uv * cells) + 0.5) / cells);
}

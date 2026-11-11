// A digital glitch: about 12 times a second some bands of rows jump sideways with their red and blue apart.
fn effect(p: Pixel) -> vec4<f32> {
    let amount = p.params[0];
    let frame = u32(max(p.time, 0.0) * 12.0);
    let bands = 6.0 + floor(nuzky_random(vec3<u32>(frame, 1u, p.seed)) * 26.0);
    let band = u32(floor(p.uv.y * bands));
    let hit = nuzky_random(vec3<u32>(band, frame, p.seed)) < 0.35 * amount;
    let jump = (nuzky_random(vec3<u32>(band, frame, p.seed ^ 77u)) - 0.5) * 0.2 * amount;
    let shift = select(0.0, jump, hit);
    let split = select(1.5, 6.0 + 18.0 * amount, hit) * amount / p.size.x;
    let uv = p.uv + vec2<f32>(shift, 0.0);
    let g = nuzky_source(uv);
    let r = nuzky_source(uv + vec2<f32>(split, 0.0)).r;
    let b = nuzky_source(uv - vec2<f32>(split, 0.0)).b;
    let row = u32(floor(p.uv.y * p.size.y / 2.0));
    let lines = 1.0 - 0.12 * amount * f32(row % 2u);
    return vec4<f32>(vec3<f32>(r, g.g, b) * lines, g.a);
}

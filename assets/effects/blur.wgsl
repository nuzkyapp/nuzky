// A soft blur: the pixel and 14 samples on a spiral up to 30 canvas pixels away, turned a little per pixel so the
// samples blend instead of showing as copies.
fn tap(p: Pixel, i: f32, turn: f32, radius: f32) -> vec4<f32> {
    let a = i * 2.39996 + turn;
    let d = sqrt((i + 0.5) / 14.0) * radius;
    let c = nuzky_source(p.uv + vec2<f32>(cos(a), sin(a)) * d / p.size);
    return vec4<f32>(c.rgb * c.a, c.a);
}

fn effect(p: Pixel) -> vec4<f32> {
    let radius = p.params[0] * 30.0;
    let turn = nuzky_random(vec3<u32>(vec2<u32>(p.uv * p.size), 7u)) * 6.28318;
    var sum = vec4<f32>(p.color.rgb * p.color.a, p.color.a);
    sum += tap(p, 0.0, turn, radius) + tap(p, 1.0, turn, radius) + tap(p, 2.0, turn, radius) + tap(p, 3.0, turn, radius);
    sum += tap(p, 4.0, turn, radius) + tap(p, 5.0, turn, radius) + tap(p, 6.0, turn, radius) + tap(p, 7.0, turn, radius);
    sum += tap(p, 8.0, turn, radius) + tap(p, 9.0, turn, radius) + tap(p, 10.0, turn, radius) + tap(p, 11.0, turn, radius);
    sum += tap(p, 12.0, turn, radius) + tap(p, 13.0, turn, radius);
    sum /= 15.0;
    let rgb = select(vec3<f32>(0.0), sum.rgb / sum.a, sum.a > 0.0);
    return vec4<f32>(rgb, sum.a);
}

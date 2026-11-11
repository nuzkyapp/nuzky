// Streaks out from the middle, as if the camera zoomed during the exposure: 14 samples on the line to the middle.
fn tap(p: Pixel, i: f32, jitter: f32, reach: f32) -> vec4<f32> {
    let s = 1.0 - reach * (i + jitter) / 14.0;
    let c = nuzky_source(vec2<f32>(0.5) + (p.uv - vec2<f32>(0.5)) * s);
    return vec4<f32>(c.rgb * c.a, c.a);
}

fn effect(p: Pixel) -> vec4<f32> {
    let reach = p.params[0] * 0.25;
    let jitter = nuzky_random(vec3<u32>(vec2<u32>(p.uv * p.size), 11u));
    var sum = vec4<f32>(p.color.rgb * p.color.a, p.color.a);
    sum += tap(p, 0.0, jitter, reach) + tap(p, 1.0, jitter, reach) + tap(p, 2.0, jitter, reach) + tap(p, 3.0, jitter, reach);
    sum += tap(p, 4.0, jitter, reach) + tap(p, 5.0, jitter, reach) + tap(p, 6.0, jitter, reach) + tap(p, 7.0, jitter, reach);
    sum += tap(p, 8.0, jitter, reach) + tap(p, 9.0, jitter, reach) + tap(p, 10.0, jitter, reach) + tap(p, 11.0, jitter, reach);
    sum += tap(p, 12.0, jitter, reach) + tap(p, 13.0, jitter, reach);
    sum /= 15.0;
    let rgb = select(vec3<f32>(0.0), sum.rgb / sum.a, sum.a > 0.0);
    return vec4<f32>(rgb, sum.a);
}

// Temporary measurement for refitting HDR tone mapping; not committed.
mod qa_support;
use nuzky_engine::{Renderer, Wait, media::VideoDecoder};
use qa_support::*;

const P: usize = 16;
const COLS: usize = 68;

fn patches() -> Vec<[f64; 3]> {
    let mut out = Vec::new();
    for r in 0..17 {
        for g in 0..17 {
            for b in 0..17 {
                out.push([r as f64 / 16.0, g as f64 / 16.0, b as f64 / 16.0]);
            }
        }
    }
    out.extend([
        [0.25; 3],
        [0.5; 3],
        [0.75; 3],
        [0.85; 3],
        [0.95; 3],
        [0.62, 0.55, 0.50],
        [0.85, 0.83, 0.76],
        [0.70, 0.30, 0.25],
    ]);
    let (median, lit) = ([0.71, 0.60, 0.51], [0.94, 0.89, 0.82]);
    for i in 0..8 {
        let t = i as f64 / 7.0;
        out.push(std::array::from_fn(|c| median[c] + (lit[c] - median[c]) * t));
    }
    out
}

#[test]
#[ignore]
fn hdr_fit_dump() {
    let label = std::env::var("HDR_FIT_LABEL").unwrap_or("run".into());
    let list = patches();
    let rows = list.len().div_ceil(COLS);
    let (w, h) = (COLS * P, rows * P);
    let d = dir("hdr-fit");
    let mut out = serde_json::Map::new();
    for transfer in ["arib-std-b67", "smpte2084"] {
        let path = d.join(format!("{transfer}.mp4"));
        if !path.exists() {
            let source = d.join("grid.ppm");
            let mut ppm = format!("P6\n{w} {h}\n65535\n").into_bytes();
            for y in 0..h {
                for x in 0..w {
                    let i = (y / P) * COLS + x / P;
                    let v = list.get(i).copied().unwrap_or([0.0; 3]);
                    ppm.extend(v.iter().flat_map(|v| ((v * 65535.0).round() as u16).to_be_bytes()));
                }
            }
            std::fs::write(&source, ppm).unwrap();
            // The signal is the same for both; only the tag differs, so PQ shows how the same code values map.
            let x265 = format!(
                "pools=1:frame-threads=1:log-level=error:lossless=1:colorprim=bt2020:transfer={transfer}:colormatrix=bt2020nc:range=limited"
            );
            let yuv = "scale=in_range=pc:out_color_matrix=bt2020nc:out_range=tv,format=yuv420p10le";
            ff(&["-i", source.to_str().unwrap(), "-vf", yuv, "-c:v", "libx265", "-x265-params", &x265, "-frames:v", "1"], &path);
        }
        let mut decoder = VideoDecoder::open_for_seeking(&path).unwrap();
        let (t, f) = decoder.next_frame().unwrap().unwrap();
        assert_eq!(f.format(), ffmpeg_next::format::Pixel::YUV420P10LE);
        let at = |plane: usize, x: usize, y: usize| {
            let row = &f.data(plane)[y * f.stride(plane)..];
            u16::from_le_bytes([row[2 * x], row[2 * x + 1]]) as f64
        };
        let centre = |i: usize| ((i % COLS) * P + P / 2, (i / COLS) * P + P / 2);
        let exact: Vec<[f64; 3]> = (0..list.len())
            .map(|i| {
                let (x, y) = centre(i);
                let (yy, cb, cr) = ((at(0, x, y) - 64.0) / 876.0, (at(1, x / 2, y / 2) - 512.0) / 896.0, (at(2, x / 2, y / 2) - 512.0) / 896.0);
                let (r, b) = (yy + 1.4746 * cr, yy + 1.8814 * cb);
                let g = (yy - 0.2627 * r - 0.0593 * b) / 0.678;
                [r, g, b].map(|c| (c * 255.0).clamp(0.0, 255.0))
            })
            .collect();
        let mean = |rgba: &[u8], width: usize, i: usize, scale: f64| -> [f64; 3] {
            let (cx, cy) = (((i % COLS) * P) as f64 * scale, ((i / COLS) * P) as f64 * scale);
            let (x0, x1) = ((cx + 4.0 * scale).ceil() as usize, (cx + 12.0 * scale).floor() as usize);
            let (y0, y1) = ((cy + 4.0 * scale).ceil() as usize, (cy + 12.0 * scale).floor() as usize);
            let mut sum = [0.0; 3];
            let mut n = 0.0;
            for y in y0..y1 {
                for x in x0..x1 {
                    for c in 0..3 {
                        sum[c] += rgba[(y * width + x) * 4 + c] as f64;
                    }
                    n += 1.0;
                }
            }
            sum.map(|s| s / n)
        };
        let mut entry = serde_json::Map::new();
        entry.insert("signal".into(), serde_json::json!(list));
        entry.insert("exact".into(), serde_json::json!(exact));
        for (name, scale) in [("own", 1.0), ("scaled", 2.0 / 3.0)] {
            let (sw, sh) = ((w as f64 * scale) as u32, (h as f64 * scale) as u32);
            let rgba = decoder.convert(&f, t, sw, sh).unwrap();
            let conv: Vec<_> = (0..list.len()).map(|i| mean(&rgba.data, sw as usize, i, scale)).collect();
            entry.insert(format!("conv_{name}"), serde_json::json!(conv));
        }
        let mut renderer = Renderer::new().unwrap();
        let frame = renderer.render(&project(&path, 200_000), 0, w as u32, h as u32, Wait::Exact, false).unwrap();
        let drawn: Vec<_> = (0..list.len()).map(|i| mean(&frame, w, i, 1.0)).collect();
        entry.insert("drawn".into(), serde_json::json!(drawn));
        out.insert(transfer.into(), serde_json::Value::Object(entry));
    }
    std::fs::write(d.join(format!("{label}.json")), serde_json::to_string(&out).unwrap()).unwrap();
}

#[test]
#[ignore]
fn hdr_convert_speed() {
    use ffmpeg_next::{format::Pixel, software::scaling};
    let d = dir("hdr-fit");
    let flags = [("bilinear", scaling::Flags::BILINEAR), ("full_chroma", scaling::Flags::BILINEAR | scaling::Flags::FULL_CHR_H_INT)];
    for (name, size) in [("1080p", "1920x1080"), ("4k", "3840x2160")] {
        let path = d.join(format!("speed-{name}.mp4"));
        if !path.exists() {
            let x265 = "pools=1:frame-threads=1:log-level=error:crf=18:colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc:range=limited";
            let src = format!("testsrc2=size={size}:rate=30,noise=alls=20:allf=t");
            ff(&["-f", "lavfi", "-i", &src, "-vf", "format=yuv420p10le", "-c:v", "libx265", "-x265-params", x265, "-frames:v", "1"], &path);
        }
        let mut decoder = VideoDecoder::open(&path).unwrap();
        let (_, f) = decoder.next_frame().unwrap().unwrap();
        let (fw, fh) = (f.width(), f.height());
        let targets: Vec<(u32, u32)> = if name == "1080p" { vec![(1920, 1080), (1280, 720), (640, 360), (214, 120)] } else { vec![(3840, 2160), (1920, 1080), (1280, 720)] };
        for (w, h) in targets {
            let mut ctxs: Vec<_> = flags
                .iter()
                .map(|(_, fl)| {
                    let mut c = scaling::Context::get(f.format(), fw, fh, Pixel::RGBA, w, h, *fl).unwrap();
                    unsafe {
                        let coef = ffmpeg_next::ffi::sws_getCoefficients(ffmpeg_next::ffi::SWS_CS_BT2020);
                        let def = ffmpeg_next::ffi::sws_getCoefficients(ffmpeg_next::ffi::SWS_CS_DEFAULT);
                        ffmpeg_next::ffi::sws_setColorspaceDetails(c.as_mut_ptr(), coef, 0, def, 1, 0, 1 << 16, 1 << 16);
                    }
                    c
                })
                .collect();
            let mut medians = [Vec::new(), Vec::new()];
            for _round in 0..7 {
                for (k, c) in ctxs.iter_mut().enumerate() {
                    let mut out = ffmpeg_next::frame::Video::new(Pixel::RGBA, w, h);
                    let mut times = Vec::new();
                    for _ in 0..15 {
                        let s = std::time::Instant::now();
                        c.run(&f, &mut out).unwrap();
                        times.push(s.elapsed().as_secs_f64() * 1000.0);
                    }
                    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    medians[k].push(times[7]);
                }
            }
            let summary: Vec<String> = medians
                .iter_mut()
                .zip(flags.iter())
                .map(|(m, (n, _))| {
                    m.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    format!("{n} {:.2} ms (rounds {:.2}..{:.2})", m[3], m[0], m[6])
                })
                .collect();
            eprintln!("SPEED {name} -> {w}x{h}: {}", summary.join(", "));
        }
    }
}

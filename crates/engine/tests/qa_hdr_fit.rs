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
    // The ten Monk skin tones (sRGB), as HLG with SDR white at HDR reference white (BT.2408), then with twice the
    // light, as a face lit by a window.
    const MONK: [u32; 10] =
        [0xf6ede4, 0xf3e7db, 0xf7ead0, 0xeadaba, 0xd7bd96, 0xa07e56, 0x825c43, 0x604134, 0x3a312a, 0x292420];
    let to_2020 = [[0.6274, 0.3293, 0.0433], [0.0691, 0.9195, 0.0114], [0.0164, 0.0880, 0.8956]];
    let oetf = |l: f64| {
        if l <= 1.0 / 12.0 { (3.0 * l).sqrt() } else { 0.17883277 * (12.0 * l - 0.28466892).ln() + 0.55991073 }
    };
    let srgb = |v: f64| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    let reference = (((0.75f64 - 0.55991073) / 0.17883277).exp() + 0.28466892) / 12.0;
    for gain in [1.0, 2.0] {
        for hex in MONK {
            let lin = [16, 8, 0].map(|s| srgb(((hex >> s) & 0xff) as f64 / 255.0));
            out.push(std::array::from_fn(|c| {
                let l: f64 = (0..3).map(|k| to_2020[c][k] * lin[k]).sum();
                oetf((l * reference * gain).min(1.0))
            }));
        }
    }
    out
}

#[test]
#[ignore]
fn hdr_fit_dump() {
    let list = patches();
    let rows = list.len().div_ceil(COLS);
    let (w, h) = (COLS * P, rows * P);
    let d = dir("hdr-fit");
    let mut out = serde_json::Map::new();
    for transfer in ["arib-std-b67", "smpte2084"] {
        let path = d.join(format!("{transfer}-grid.mov"));
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
                "pools=1:frame-threads=1:log-level=error:crf=4:colorprim=bt2020:transfer={transfer}:colormatrix=bt2020nc:range=limited"
            );
            let yuv = "scale=in_range=pc:out_color_matrix=bt2020nc:out_range=tv,format=yuv420p10le";
            ff(
                &[
                    "-i",
                    source.to_str().unwrap(),
                    "-vf",
                    yuv,
                    "-c:v",
                    "libx265",
                    "-x265-params",
                    &x265,
                    "-frames:v",
                    "1",
                    "-tag:v",
                    "hvc1",
                ],
                &path,
            );
        }
        let mut entry = serde_json::Map::new();
        entry.insert("signal".into(), serde_json::json!(list));
        for (label, old) in [("old", true), ("new", false)] {
            unsafe {
                if old {
                    std::env::set_var("NUZKY_HDR_OLD_FLAGS", "1")
                } else {
                    std::env::remove_var("NUZKY_HDR_OLD_FLAGS")
                }
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
                    let (yy, cb, cr) = (
                        (at(0, x, y) - 64.0) / 876.0,
                        (at(1, x / 2, y / 2) - 512.0) / 896.0,
                        (at(2, x / 2, y / 2) - 512.0) / 896.0,
                    );
                    let (r, b) = (yy + 1.4746 * cr, yy + 1.8814 * cb);
                    let g = (yy - 0.2627 * r - 0.0593 * b) / 0.678;
                    [r, g, b].map(|c| (c * 255.0).clamp(0.0, 255.0))
                })
                .collect();
            entry.insert("exact".into(), serde_json::json!(exact));
            let mean = |rgba: &[u8], width: usize, i: usize, sx: f64, sy: f64| -> [f64; 3] {
                let (cx, cy) = (((i % COLS) * P) as f64 * sx, ((i / COLS) * P) as f64 * sy);
                let (x0, x1) = ((cx + 4.0 * sx).ceil() as usize, (cx + 12.0 * sx).floor() as usize);
                let (y0, y1) = ((cy + 4.0 * sy).ceil() as usize, (cy + 12.0 * sy).floor() as usize);
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
            // The file's own size, then widths the preview and thumbnails could ask for.
            let mut sizes = vec![("own".to_string(), w as u32, h as u32)];
            for sw in [725u32, 724, 726, 720, 736, 768, 544] {
                sizes.push((format!("w{sw}"), sw, (h as u64 * sw as u64 / w as u64) as u32 & !1));
            }
            for (name, sw, sh) in sizes {
                let rgba = decoder.convert(&f, t, sw, sh).unwrap();
                let (sx, sy) = (sw as f64 / w as f64, sh as f64 / h as f64);
                let conv: Vec<_> = (0..list.len()).map(|i| mean(&rgba.data, sw as usize, i, sx, sy)).collect();
                entry.insert(format!("conv_{name}_{label}"), serde_json::json!(conv));
            }
            let mut renderer = Renderer::new().unwrap();
            let frame = renderer.render(&project(&path, 200_000), 0, w as u32, h as u32, Wait::Exact, false).unwrap();
            let drawn: Vec<_> = (0..list.len()).map(|i| mean(&frame, w, i, 1.0, 1.0)).collect();
            entry.insert(format!("drawn_{label}"), serde_json::json!(drawn));
        }
        unsafe { std::env::remove_var("NUZKY_HDR_OLD_FLAGS") }
        out.insert(transfer.into(), serde_json::Value::Object(entry));
    }
    std::fs::write(d.join("dump.json"), serde_json::to_string(&out).unwrap()).unwrap();
}

#[test]
#[ignore]
fn hdr_convert_speed() {
    let d = dir("hdr-fit");
    let mut sources = Vec::new();
    for (name, size) in [("1080p", "1920x1080"), ("4k", "3840x2160")] {
        let path = d.join(format!("speed-{name}.mov"));
        if !path.exists() {
            let x265 = "pools=1:frame-threads=1:log-level=error:crf=18:colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc:range=limited";
            let src = format!("testsrc2=size={size}:rate=30,noise=alls=20:allf=t");
            ff(
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    &src,
                    "-vf",
                    "format=yuv420p10le",
                    "-c:v",
                    "libx265",
                    "-x265-params",
                    x265,
                    "-frames:v",
                    "2",
                    "-tag:v",
                    "hvc1",
                ],
                &path,
            );
        }
        sources.push((name, path));
    }
    let proxy = nuzky_engine::proxy::ensure_proxy(&d.join("cache"), &sources[0].1, |_| Ok(())).unwrap();
    let cases: Vec<(&str, std::path::PathBuf, bool, Vec<(u32, u32)>)> = vec![
        ("1080p 10-bit", sources[0].1.clone(), false, vec![(1920, 1080), (1280, 720), (640, 360), (214, 120)]),
        ("4k 10-bit", sources[1].1.clone(), false, vec![(3840, 2160), (1920, 1080), (1280, 720)]),
        ("1080p 8-bit proxy", proxy, true, vec![(1280, 720), (640, 360), (214, 120)]),
    ];
    for (name, path, is_proxy, targets) in cases {
        let open =
            || if is_proxy { VideoDecoder::open_proxy(&path).unwrap() } else { VideoDecoder::open(&path).unwrap() };
        let mut probe = open();
        let s = std::time::Instant::now();
        let (t, f) = probe.next_frame().unwrap().unwrap();
        eprintln!("SPEED {name}: {:?} first frame decoded in {:.1} ms", f.format(), s.elapsed().as_secs_f64() * 1000.0);
        for (w, h) in targets {
            // Each decoder keeps the converter made on its first frame, so one is made the old way and one the new.
            let mut decoders = [open(), open()];
            for (k, decoder) in decoders.iter_mut().enumerate() {
                if k == 0 {
                    unsafe { std::env::set_var("NUZKY_HDR_OLD_FLAGS", "1") }
                } else {
                    unsafe { std::env::remove_var("NUZKY_HDR_OLD_FLAGS") }
                }
                decoder.convert(&f, t, w, h).unwrap();
            }
            unsafe { std::env::remove_var("NUZKY_HDR_OLD_FLAGS") }
            let mut medians = [Vec::new(), Vec::new()];
            for _round in 0..9 {
                for (k, decoder) in decoders.iter_mut().enumerate() {
                    let mut times = Vec::new();
                    for _ in 0..15 {
                        let s = std::time::Instant::now();
                        std::hint::black_box(decoder.convert(&f, t, w, h).unwrap());
                        times.push(s.elapsed().as_secs_f64() * 1000.0);
                    }
                    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    medians[k].push(times[7]);
                }
            }
            let summary: Vec<String> = medians
                .iter_mut()
                .zip(["old", "full_chroma"])
                .map(|(m, n)| {
                    m.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    format!("{n} {:.2} ms (rounds {:.2}..{:.2})", m[4], m[0], m[8])
                })
                .collect();
            eprintln!("SPEED {name} -> {w}x{h}: {}", summary.join(", "));
        }
    }
}

#[test]
#[ignore]
fn hdr_pipeline_speed() {
    let d = dir("hdr-fit");
    let path = d.join("pipeline-4k.mov");
    if !path.exists() {
        let x265 =
            "pools=4:log-level=error:crf=20:colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc:range=limited";
        let src = "testsrc2=size=3840x2160:rate=30,noise=alls=12:allf=t";
        ff(
            &[
                "-f",
                "lavfi",
                "-i",
                src,
                "-vf",
                "format=yuv420p10le",
                "-c:v",
                "libx265",
                "-preset",
                "ultrafast",
                "-x265-params",
                x265,
                "-frames:v",
                "90",
                "-tag:v",
                "hvc1",
            ],
            &path,
        );
    }
    let set = |old: bool| unsafe {
        if old { std::env::set_var("NUZKY_HDR_OLD_FLAGS", "1") } else { std::env::remove_var("NUZKY_HDR_OLD_FLAGS") }
    };
    let median = |mut v: Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        format!("{:.3} (range {:.3}..{:.3})", v[v.len() / 2], v[0], v[v.len() - 1])
    };
    // Playing 4K HLG before its proxy exists, at the preview's size.
    let mut play = [Vec::new(), Vec::new()];
    for _round in 0..5 {
        for (k, old) in [true, false].into_iter().enumerate() {
            set(old);
            let p = project(&path, 3_000_000);
            let mut renderer = Renderer::new().unwrap();
            renderer.render(&p, 0, 1280, 720, Wait::Exact, true).unwrap();
            let s = std::time::Instant::now();
            for i in 1..90 {
                std::hint::black_box(renderer.render(&p, i * 33_333, 1280, 720, Wait::Exact, true).unwrap());
            }
            play[k].push(s.elapsed().as_secs_f64() * 1000.0 / 89.0);
        }
    }
    let [old, new] = play;
    eprintln!("SPEED play 4K HLG at 1280x720, ms a frame: old {}, full_chroma {}", median(old), median(new));
    for (preset, canvas, rounds) in
        [("ultrafast", (3840, 2160), 5), ("ultrafast", (1920, 1080), 5), ("medium", (3840, 2160), 3)]
    {
        let mut times = [Vec::new(), Vec::new()];
        for _round in 0..rounds {
            for (k, old) in [true, false].into_iter().enumerate() {
                set(old);
                let mut p = project(&path, 3_000_000);
                (p.canvas.width, p.canvas.height) = canvas;
                let options = nuzky_engine::export::ExportOptions {
                    preset: preset.into(),
                    replace_existing: true,
                    ..Default::default()
                };
                let cancel = std::sync::atomic::AtomicBool::new(false);
                let s = std::time::Instant::now();
                nuzky_engine::export::export(&p, &d.join("cache"), &d.join("out.mp4"), &options, &cancel, |_| {})
                    .unwrap();
                times[k].push(s.elapsed().as_secs_f64());
            }
        }
        let [old, new] = times;
        eprintln!(
            "SPEED export 3 s of 4K HLG at {}x{} {preset}, s: old {}, full_chroma {}",
            canvas.0,
            canvas.1,
            median(old),
            median(new)
        );
    }
    set(false);
}

#[test]
#[ignore]
fn hdr_decode_speed() {
    let path = dir("hdr-fit").join("pipeline-4k.mov");
    for (name, seeking) in [("open (export)", false), ("open_for_seeking (preview)", true)] {
        let mut runs = Vec::new();
        for _ in 0..5 {
            let mut decoder = if seeking { VideoDecoder::open_for_seeking(&path).unwrap() } else { VideoDecoder::open(&path).unwrap() };
            decoder.next_frame().unwrap().unwrap();
            let s = std::time::Instant::now();
            let mut n = 0;
            while let Some(f) = decoder.next_frame().unwrap() {
                std::hint::black_box(f);
                n += 1;
            }
            runs.push(s.elapsed().as_secs_f64() * 1000.0 / n as f64);
        }
        runs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        eprintln!("SPEED decode 4K HLG {name}: {:.2} ms a frame (range {:.2}..{:.2})", runs[2], runs[0], runs[4]);
    }
}

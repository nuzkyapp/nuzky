mod qa_support;
use nuzky_engine::{
    Renderer, Wait,
    media::{VideoDecoder, decode_size, probe},
    model::*,
    worker::VideoWorker,
};
use qa_support::*;
use std::{path::Path, process::Command, time::Instant};

fn check_seek(path: &Path, extra_times: &[i64]) {
    let timestamps = pts(path);
    let a = probe(path, "seek".into()).unwrap();
    let truth = raw(path, &[]);
    let frame_len = (a.width * a.height * 4) as usize;
    assert_eq!(truth.len() / frame_len, timestamps.len());
    let mut decoder = VideoDecoder::open_for_seeking(path).unwrap();
    let mut worker = VideoWorker::spawn_for_seeking(path.into());
    let mut rng = Rng(0xCAFE_1234);
    let mut times = vec![0, 1, *timestamps.last().unwrap()];
    times.extend(extra_times);
    for _ in 0..12 {
        let index = (rng.next() as usize) % (timestamps.len() - 1);
        times.extend([timestamps[index], timestamps[index + 1] - 1]);
    }
    let mut failures = Vec::new();
    for t in times {
        let expected = timestamps.iter().rposition(|&p| p <= t).unwrap();
        decoder.seek(t).unwrap();
        let mut candidate = None;
        while let Some((ts, frame)) = decoder.next_frame().unwrap() {
            if ts > t {
                break;
            }
            candidate = Some((ts, frame));
        }
        if let Some((ts, frame)) = candidate {
            let rgba = decoder.convert(&frame, ts, a.width, a.height).unwrap();
            let error = mae(&rgba.data, &truth[expected * frame_len..(expected + 1) * frame_len]);
            if ts != timestamps[expected] || error > 1.0 {
                failures.push(format!("decoder t={t}: pts={ts} expected={} MAE={error}", timestamps[expected]));
            }
        } else {
            failures.push(format!("decoder t={t}: no candidate"));
        }
        let rgba = worker.get(t, (a.width, a.height), false, true).expect("worker frame");
        let error = mae(&rgba.data, &truth[expected * frame_len..(expected + 1) * frame_len]);
        if !worker.exact || rgba.t_us != timestamps[expected] || error > 1.0 {
            failures.push(format!(
                "worker t={t}: pts={} expected={} exact={} MAE={error}",
                rgba.t_us, timestamps[expected], worker.exact
            ));
        }
    }
    assert!(failures.is_empty(), "{}\n{}", path.display(), failures.join("\n"));
}

#[test]
fn cfr_h264_long_gop_b_frames_seek_matches_ffmpeg_pts_and_pixels() {
    if !available() {
        return;
    }
    let d = dir("seek-cfr");
    let mut failures = Vec::new();
    for rate in ["25", "30000/1001", "30", "60"] {
        let path = d.join(format!("counter-{}.mp4", rate.replace('/', "_")));
        ff(
            &[
                "-f",
                "lavfi",
                "-i",
                &format!("testsrc2=size=96x64:rate={rate}:duration={}", if rate == "25" { 10.4 } else { 3.0 }),
                "-vf",
                "drawtext=text='%{n}':x=4:y=4:fontsize=22:fontcolor=white:box=1:boxcolor=black",
                "-c:v",
                "libx264",
                "-threads",
                "2",
                "-g",
                "250",
                "-bf",
                "3",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
            ],
            &path,
        );
        if std::panic::catch_unwind(|| check_seek(&path, &[])).is_err() {
            failures.push(rate);
        }
    }
    assert!(failures.is_empty(), "Failed rates: {failures:?}");
}
#[test]
fn vfr_and_hevc_10_bit_seek_matches_ffmpeg() {
    if !available() {
        return;
    }
    let d = dir("seek-vfr-hevc");
    let encoders = run(Command::new("ffmpeg").args(["-hide_banner", "-encoders"]));
    let has_x265 = String::from_utf8_lossy(&encoders.stdout).contains("libx265");
    let mut failures = Vec::new();
    for hevc in [false, true] {
        let path = d.join(if hevc { "hevc10.mp4" } else { "vfr.mp4" });
        let filter = "drawtext=text='%{n}':x=4:y=4:fontsize=22:fontcolor=white:box=1:boxcolor=black,setpts='if(lt(N,20),N/(30*TB),(20/30+(N-20)/12)/TB)'";
        let vaapi_filter = format!("{filter},format=p010le,hwupload");
        let mut args =
            vec!["-f", "lavfi", "-i", "testsrc2=size=96x64:rate=30:duration=2", "-vf", filter, "-fps_mode", "vfr"];
        if hevc && has_x265 {
            args.extend([
                "-c:v",
                "libx265",
                "-x265-params",
                "pools=1:frame-threads=1:keyint=250:bframes=3:log-level=error",
                "-pix_fmt",
                "yuv420p10le",
            ]);
        } else if hevc {
            args[5] = &vaapi_filter;
            args.splice(0..0, ["-vaapi_device", "/dev/dri/renderD128"]);
            args.extend(["-c:v", "hevc_vaapi", "-profile:v", "main10"]);
        } else {
            args.extend(["-c:v", "libx264", "-threads", "2", "-g", "250", "-bf", "3"]);
        }
        ff(&args, &path);
        if std::panic::catch_unwind(|| check_seek(&path, &[])).is_err() {
            failures.push(hevc);
        }
    }
    assert!(failures.is_empty(), "Failed HEVC flags {failures:?}");
}
/// HEVC whose pictures decode row by row on several threads (wavefronts, as Apple's encoder and x265 write them) lands
/// on the same frames as FFmpeg after every seek, also with B-frames.
#[test]
fn hevc_with_wavefronts_seeks_match_ffmpeg() {
    if !available() {
        return;
    }
    let encoders =
        String::from_utf8_lossy(&run(Command::new("ffmpeg").args(["-hide_banner", "-encoders"])).stdout).into_owned();
    let d = dir("seek-hevc-wavefronts");
    let source = ["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=2"];
    let mut checked = 0;
    if encoders.contains("libx265") {
        let path = d.join("x265.mp4");
        ff(&[&source[..], &["-c:v", "libx265", "-x265-params", "keyint=30:bframes=3:log-level=error"]].concat(), &path);
        check_seek(&path, &[]);
        checked += 1;
    }
    if encoders.contains("hevc_videotoolbox") {
        let path = d.join("apple.mov");
        ff(&[&source[..], &["-c:v", "hevc_videotoolbox", "-b:v", "4M", "-g", "30", "-tag:v", "hvc1"]].concat(), &path);
        check_seek(&path, &[]);
        checked += 1;
    }
    if checked == 0 {
        eprintln!("SKIP: ffmpeg without an HEVC encoder");
    }
}
#[test]
fn rotation_display_matrices_match_ffmpeg_autorotate() {
    if !available() {
        return;
    }
    let d = dir("rotation");
    let source = d.join("base.mp4");
    ff(&["-f", "lavfi", "-i", "testsrc2=size=96x64:rate=25:duration=0.2", "-c:v", "libx264", "-threads", "2"], &source);
    let mut renderer = Renderer::new().unwrap();
    for angle in [90, 180, 270] {
        let path = d.join(format!("rotate-{angle}.mp4"));
        ff(&["-display_rotation", &angle.to_string(), "-i", source.to_str().unwrap(), "-c", "copy"], &path);
        assert!(
            info(&path)["streams"][0]["side_data_list"].as_array().is_some_and(|a| !a.is_empty()),
            "fixture has no display matrix"
        );
        let p = project(&path, 200_000);
        let (w, h) = if angle == 180 { (96, 64) } else { (64, 96) };
        assert_eq!((p.assets[0].width, p.assets[0].height), (w, h));
        assert_eq!(p.assets[0].rotation, (360 - angle) % 360);
        let got = renderer.render(&p, 0, w, h, Wait::Exact, false).unwrap();
        let expected = raw(&path, &["-frames:v", "1"]);
        let error = mae(&got, &expected);
        assert!(error < 3.0, "rotation {angle}: pixel MAE={error}");
    }
}
#[test]
fn odd_tiny_and_4k_frames_decode_without_corruption() {
    if !available() {
        return;
    }
    let d = dir("sizes");
    for (w, h) in [(1, 1), (3, 5), (1080, 1918), (3840, 2160)] {
        let p = d.join(format!("{w}x{h}.{}", if w == 3840 { "mp4" } else { "png" }));
        if w == 3840 {
            ff(
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    "color=white:s=3840x2160:r=25",
                    "-frames:v",
                    "1",
                    "-c:v",
                    "libx264",
                    "-threads",
                    "2",
                    "-preset",
                    "ultrafast",
                    "-pix_fmt",
                    "yuv420p",
                ],
                &p,
            );
        } else {
            ff(
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("color=white:s=2x2,scale={w}:{h}"),
                    "-frames:v",
                    "1",
                    "-pix_fmt",
                    "rgb24",
                ],
                &p,
            );
        }
        let start = Instant::now();
        let mut decoder = VideoDecoder::open(&p).unwrap();
        assert_eq!(decoder.source_size(), (w, h));
        let (t, f) = decoder.next_frame().unwrap().unwrap();
        let size = decode_size((w, h), 0, (w as f32, h as f32), 8192);
        let rgba = decoder.convert(&f, t, size.0, size.1).unwrap();
        assert_eq!(rgba.data.len(), (size.0 * size.1 * 4) as usize);
        assert!(rgba.data.iter().all(|&v| v >= 253));
        eprintln!("QA decode {w}x{h}: {:?}", start.elapsed());
    }
}
#[test]
fn full_range_444_rgb_alpha_and_gif() {
    if !available() {
        return;
    }
    let d = dir("pixels");
    let mut failures = Vec::new();
    for (name, codec, pix) in [
        ("full.mkv", "mjpeg", "yuvj420p"),
        ("444.mkv", "libx264", "yuv444p"),
        ("rgb.png", "png", "rgb24"),
        ("alpha.png", "png", "rgba"),
        ("anim.gif", "gif", "rgb8"),
    ] {
        let p = d.join(name);
        ff(
            &[
                "-f",
                "lavfi",
                "-i",
                "nullsrc=s=64x64:r=10:d=0.3,format=rgba,geq=r='16+X*3':g='16+X*3':b='16+X*3':a=128",
                "-frames:v",
                if name.ends_with("gif") { "3" } else { "1" },
                "-c:v",
                codec,
                "-threads",
                "2",
                "-pix_fmt",
                pix,
            ],
            &p,
        );
        let mut decoder = VideoDecoder::open(&p).unwrap();
        let (t, f) = decoder.next_frame().unwrap().unwrap();
        let rgba = decoder.convert(&f, t, 64, 64).unwrap();
        let expected = raw(&p, &["-frames:v", "1"]);
        let error = mae(&rgba.data, &expected);
        if error > 1.0 {
            failures.push(format!("{name} MAE={error}"));
        }
        if name == "full.mkv" && ((rgba.data[0] as i16 - 16).abs() > 4 || (rgba.data[63 * 4] as i16 - 205).abs() > 4) {
            failures.push(format!("range {}..{}", rgba.data[0], rgba.data[63 * 4]));
        }
        if name == "alpha.png" {
            assert_eq!(rgba.data[3], 128);
        }
        if name == "anim.gif" {
            assert_eq!(probe(&p, "gif".into()).unwrap().kind, AssetKind::Video);
            assert_eq!(pts(&p).len(), 3);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
// An RGB image has no YUV matrix. One 720 px or taller was once converted through BT.601 and back through
// BT.709, which drew a screenshot's purple (254, 0, 253) as (255, 39, 255).
#[test]
fn saturated_png_colours_match_ffmpeg_in_preview_thumbnails_and_export() {
    if !available() {
        return;
    }
    const COLOURS: [[u8; 3]; 4] = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [254, 0, 253]];
    let d = dir("png-colours");
    let middles = |frame: &[u8], w: u32, h: u32| -> Vec<[u8; 4]> {
        let (w, y) = (w as usize, h as usize / 2);
        (0..4).map(|i| frame[(y * w + (2 * i + 1) * w / 8) * 4..][..4].try_into().unwrap()).collect()
    };
    let mut failures = Vec::new();
    let mut compare = |what: String, ours: Vec<[u8; 4]>, theirs: &[[u8; 4]], tolerance: u8| {
        if ours.iter().flatten().zip(theirs.iter().flatten()).any(|(a, b)| a.abs_diff(*b) > tolerance) {
            failures.push(format!("{what}: {ours:?}, FFmpeg {theirs:?}"));
        }
    };
    for (w, h) in [(320, 180), (1280, 720)] {
        for (pix, alpha) in [("rgb24", 255), ("rgba", 160)] {
            let p = d.join(format!("{w}x{h}-{pix}.png"));
            let band = |c: usize| {
                let terms = COLOURS.iter().enumerate();
                let terms = terms.map(|(i, v)| format!("{}*between(X,{},{})", v[c], i * w / 4, (i + 1) * w / 4 - 1));
                terms.collect::<Vec<_>>().join("+")
            };
            let source =
                format!("nullsrc=s={w}x{h},format=rgba,geq=r='{}':g='{}':b='{}':a={alpha}", band(0), band(1), band(2));
            ff(&["-f", "lavfi", "-i", &source, "-frames:v", "1", "-pix_fmt", pix], &p);
            let (w, h) = (w as u32, h as u32);
            let reference = middles(&raw(&p, &[]), w, h);

            // Thumbnails and the preview decode at the size they show, often smaller than the file.
            let mut decoder = VideoDecoder::open(&p).unwrap();
            let (t, f) = decoder.next_frame().unwrap().unwrap();
            for (cw, ch) in [(w, h), (w / 2, h / 2), (w * 120 / h, 120)] {
                let rgba = decoder.convert(&f, t, cw, ch).unwrap();
                compare(format!("{w}x{h} {pix} decoded at {cw}x{ch}"), middles(&rgba.data, cw, ch), &reference, 1);
            }

            // On a canvas half its size the preview, `nuzky frame` and the export scale it, as in most projects.
            let mut timeline = project(&p, 200_000);
            (timeline.canvas.width, timeline.canvas.height) = (w / 2, h / 2);
            timeline.canvas.background = "#000000".into();
            let over_black = format!("color=black:s={w}x{h},format=rgba[bg];[bg][0:v]overlay=format=rgb:shortest=1");
            let theirs = middles(&raw(&p, &["-filter_complex", &over_black, "-frames:v", "1"]), w, h);
            let frame = Renderer::new().unwrap().render(&timeline, 0, w / 2, h / 2, Wait::Exact, false).unwrap();
            compare(format!("{w}x{h} {pix} drawn"), middles(&frame, w / 2, h / 2), &theirs, 1);

            // The export is BT.709 4:2:0 in limited range.
            let out = d.join(format!("{w}x{h}-{pix}.mp4"));
            let options = nuzky_engine::export::ExportOptions {
                preset: "ultrafast".into(),
                replace_existing: true,
                ..Default::default()
            };
            let cancel = std::sync::atomic::AtomicBool::new(false);
            nuzky_engine::export::export(&timeline, &d.join("cache"), &out, &options, &cancel, |_| {}).unwrap();
            let through_yuv = format!(
                "{over_black},scale=out_color_matrix=bt709:out_range=tv,format=yuv420p,scale=in_color_matrix=bt709:in_range=tv"
            );
            let theirs = middles(&raw(&p, &["-filter_complex", &through_yuv, "-frames:v", "1"]), w, h);
            let exported = middles(&raw(&out, &["-frames:v", "1"]), w / 2, h / 2);
            compare(format!("{w}x{h} {pix} exported"), exported, &theirs, 3);
        }
    }
    // Shown tiny, swscale scales in two steps and sets the matrix only on the first. FFmpeg 6 scales the same way,
    // so the reference is the frame at full size, which the two steps round a few values off.
    let video =
        ["-vf", "scale=out_color_matrix=bt709", "-pix_fmt", "yuv420p", "-colorspace", "bt709", "-c:v", "libx264"];
    for (name, args) in [("tiny.png", &["-pix_fmt", "rgb24"][..]), ("tiny.mp4", &video[..])] {
        let p = d.join(name);
        ff(&[&["-f", "lavfi", "-i", "color=0xFE00FD:s=3840x2160", "-frames:v", "1"], args].concat(), &p);
        let mut decoder = VideoDecoder::open(&p).unwrap();
        let (t, f) = decoder.next_frame().unwrap().unwrap();
        let rgba = decoder.convert(&f, t, 28, 16).unwrap();
        let theirs = middles(&raw(&p, &[]), 3840, 2160);
        compare(format!("3840x2160 {name} decoded at 28x16"), middles(&rgba.data, 28, 16), &theirs, 5);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn png_alpha_composites_over_video() {
    if !available() {
        return;
    }
    let d = dir("alpha-composite");
    let video = d.join("blue.mp4");
    let png = d.join("red-half.png");
    ff(&["-f", "lavfi", "-i", "color=blue:s=64x64:r=25:d=0.2", "-c:v", "libx264", "-threads", "2"], &video);
    ff(&["-f", "lavfi", "-i", "color=red@0.5:s=64x64,format=rgba", "-frames:v", "1", "-pix_fmt", "rgba"], &png);
    let mut p = project(&video, 200_000);
    let asset = probe(&png, "overlay".into()).unwrap();
    p.tracks.push(Track {
        id: "overlay-track".into(),
        kind: TrackKind::Video,
        name: String::new(),
        muted: false,
        hidden: false,
        keep_in_place: false,
        clips: vec![clip("over", &asset.id, 0, 200_000)],
    });
    p.assets.push(asset);
    let frame = Renderer::new().unwrap().render(&p, 0, 64, 64, Wait::Exact, false).unwrap();
    let pixel = &frame[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 4];
    assert!(
        (pixel[0] as i16 - 127).abs() < 5 && pixel[1] < 5 && (pixel[2] as i16 - 127).abs() < 5 && pixel[3] == 255,
        "half-red over blue: {pixel:?}"
    );
}

#[test]
fn preview_decodes_new_media_after_a_clip_is_relinked() {
    if !available() {
        return;
    }
    let d = dir("relink");
    let (blue, red) = (d.join("blue.mp4"), d.join("red.mp4"));
    ff(&["-f", "lavfi", "-i", "color=blue:s=64x64:r=25:d=0.2", "-c:v", "libx264", "-threads", "2"], &blue);
    ff(&["-f", "lavfi", "-i", "color=red:s=64x64:r=25:d=0.2", "-c:v", "libx264", "-threads", "2"], &red);
    // The preview keeps one renderer across projects; a copied project keeps its clip ids.
    let mut renderer = Renderer::new().unwrap();
    let mut p = project(&blue, 200_000);
    let centre = |frame: &[u8]| frame[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 3].to_vec();
    let before = centre(&renderer.render(&p, 0, 64, 64, Wait::Exact, false).unwrap());
    assert!(before[2] > 200 && before[0] < 50, "blue: {before:?}");
    p.assets[0].path = red.to_string_lossy().into_owned();
    let after = centre(&renderer.render(&p, 0, 64, 64, Wait::Exact, false).unwrap());
    assert!(after[0] > 200 && after[2] < 50, "red: {after:?}");
    // The decoder of the old file goes at once, not after idling, even when the new file fails.
    assert_eq!(renderer.decoders(), 1);
    let broken = d.join("broken.mp4");
    std::fs::write(&broken, b"not a video").unwrap();
    p.assets[0].path = broken.to_string_lossy().into_owned();
    assert!(renderer.render(&p, 0, 64, 64, Wait::Exact, false).is_err());
    assert_eq!(renderer.decoders(), 1);
}

#[test]
fn cold_seek_immediately_before_keyframe_uses_previous_frame() {
    if !available() {
        return;
    }
    let d = dir("seek-keyframe-boundary");
    let path = d.join("gop10.mp4");
    ff(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=96x64:r=25:d=0.8",
            "-c:v",
            "libx264",
            "-threads",
            "2",
            "-g",
            "10",
            "-keyint_min",
            "10",
            "-sc_threshold",
            "0",
            "-bf",
            "3",
        ],
        &path,
    );
    let truth = raw(&path, &[]);
    let timestamps = pts(&path);
    let mut failures = Vec::new();
    for t in [399_900, 399_990, 399_999, 400_000, 400_001] {
        let expected = timestamps.iter().rposition(|&p| p <= t).unwrap();
        let mut decoder = VideoDecoder::open(&path).unwrap();
        decoder.seek(t).unwrap();
        let first = decoder.next_frame().unwrap().unwrap().0;
        let mut worker = VideoWorker::spawn(path.clone());
        let frame = worker.get(t, (96, 64), false, true).unwrap();
        let error = mae(&frame.data, &truth[expected * 96 * 64 * 4..(expected + 1) * 96 * 64 * 4]);
        eprintln!(
            "QA keyframe boundary want={t} first_after_seek={first} worker={} expected={} exact={} MAE={error}",
            frame.t_us, timestamps[expected], worker.exact
        );
        if frame.t_us != timestamps[expected] || error > 1.0 {
            failures.push(format!(
                "want={t}: returned {}, expected {}, exact={}",
                frame.t_us, timestamps[expected], worker.exact
            ));
        }
    }
    // Diagnose the proposed repair without changing VideoDecoder: seek in stream ticks,
    // explicitly rounding the upper bound down. A 1/12800 tick must not round up to .4s.
    let mut input = ffmpeg_next::format::input(&path).unwrap();
    let stream = input.streams().best(ffmpeg_next::media::Type::Video).unwrap();
    let index = stream.index();
    let tb = stream.time_base();
    let tick = 399_999_i64 * tb.denominator() as i64 / (1_000_000 * tb.numerator() as i64);
    let result =
        unsafe { ffmpeg_next::ffi::avformat_seek_file(input.as_mut_ptr(), index as i32, i64::MIN, tick, tick, 0) };
    assert_eq!(result, 0);
    let first = input.packets().find(|(s, _)| s.index() == index).unwrap().1.pts().unwrap();
    eprintln!("QA stream-tick diagnostic: time_base={tb}, floor_tick={tick}, first_packet_pts={first}");
    assert!(first <= tick);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Inserts a big-endian EXIF APP1 segment with only an Orientation tag after the JPEG SOI marker.
fn with_exif_orientation(jpeg: &[u8], orientation: u8) -> Vec<u8> {
    assert_eq!(&jpeg[..2], [0xFF, 0xD8]);
    let tiff = [b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, orientation, 0, 0, 0, 0, 0, 0];
    let mut out = jpeg[..2].to_vec();
    out.extend([0xFF, 0xE1, 0, (2 + 6 + tiff.len()) as u8]);
    out.extend(b"Exif\0\0");
    out.extend(tiff);
    out.extend(&jpeg[2..]);
    out
}
#[test]
fn exif_orientation_of_still_images_matches_ffmpeg_autorotate() {
    if !available() {
        return;
    }
    let d = dir("exif-orientation");
    let source = d.join("source.jpg");
    ff(&["-f", "lavfi", "-i", "testsrc2=size=96x64", "-frames:v", "1", "-q:v", "2"], &source);
    let jpeg = std::fs::read(&source).unwrap();
    let mut renderer = Renderer::new().unwrap();
    // Orientations 2, 4, 5 and 7 mirror, as front cameras ask for.
    let all = [
        (1, 0, false),
        (2, 0, true),
        (3, 180, false),
        (4, 180, true),
        (5, 90, true),
        (6, 90, false),
        (7, 270, true),
        (8, 270, false),
    ];
    for (orientation, rotation, mirror) in all {
        let path = d.join(format!("orientation-{orientation}.jpg"));
        std::fs::write(&path, with_exif_orientation(&jpeg, orientation)).unwrap();
        let p = project(&path, 200_000);
        let (w, h) = if rotation % 180 == 90 { (64, 96) } else { (96, 64) };
        let a = &p.assets[0];
        assert_eq!((a.width, a.height, a.rotation, a.mirror), (w, h, rotation, mirror), "{orientation}");
        let got = renderer.render(&p, 0, w, h, Wait::Exact, false).unwrap();
        let expected = raw(&path, &["-frames:v", "1"]);
        let error = mae(&got, &expected);
        assert!(error < 3.0, "orientation {orientation}: pixel MAE={error}");
    }
}

#[test]
fn panorama_zoomed_past_the_gpu_texture_limit_renders_downscaled() {
    if !available() {
        return;
    }
    let d = dir("panorama");
    let path = d.join("panorama.jpg");
    ff(&["-f", "lavfi", "-i", "color=red:s=20000x3000", "-frames:v", "1"], &path);
    let mut p = project(&path, 200_000);
    (p.canvas.width, p.canvas.height) = (1080, 1920);
    // Filling the portrait canvas shows the panorama 12 800 px wide, past the 8192 px default limit.
    let ClipContent::Media { transform, .. } = &mut p.tracks[0].clips[0].content else { unreachable!() };
    transform.scale = (1920.0 / 3000.0) / (1080.0 / 20000.0);
    let mut renderer = Renderer::new().unwrap();
    assert!(renderer.max_texture_dimension() < 12_800);
    let frame = renderer.render(&p, 0, 1080, 1920, Wait::Exact, false).unwrap();
    assert!(frame.as_chunks::<4>().0.iter().all(|px| px[0] > 250 && px[1] < 5 && px[2] < 5), "not filled with red");
}

#[test]
fn mpeg_ts_seek_matches_ffmpeg_pts_and_pixels() {
    if !available() {
        return;
    }
    let d = dir("seek-mpegts");
    let path = d.join("counter.ts");
    ff(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=96x64:rate=25:duration=10",
            "-vf",
            "drawtext=text='%{n}':x=4:y=4:fontsize=22:fontcolor=white:box=1:boxcolor=black",
            "-c:v",
            "libx264",
            "-threads",
            "2",
            "-g",
            "50",
            "-bf",
            "3",
            "-crf",
            "18",
            "-pix_fmt",
            "yuv420p",
        ],
        &path,
    );
    // Keyframes every 2 s. Without an index the demuxer lands after the keyframe, so the decoder
    // used to start at the next one: up to a GOP late, or nothing at all in the last GOP.
    check_seek(&path, &[1_999_999, 3_960_000, 3_999_999, 4_000_000, 7_999_999, 9_000_000, 2_000_000]);
}

#[test]
fn hlg_and_pq_sources_are_tone_mapped_like_libplacebo() {
    if !available() {
        return;
    }
    let tools = |kind: &str| {
        String::from_utf8_lossy(&run(Command::new("ffmpeg").args(["-hide_banner", kind])).stdout).into_owned()
    };
    if !tools("-encoders").contains("libx265") || !tools("-filters").contains("libplacebo") {
        eprintln!("SKIP: ffmpeg without libx265 or libplacebo");
        return;
    }
    // libplacebo makes the reference on a Vulkan GPU; a machine without one, like a CI runner, cannot.
    // Any other failure of this probe fails the test, so the comparison never goes quiet by accident.
    let probe = Command::new("ffmpeg")
        .args(["-v", "error", "-threads", "2", "-filter_threads", "1", "-f", "lavfi", "-i", "color=s=16x16:d=0.04"])
        .args(["-vf", "libplacebo", "-f", "null", "-"])
        .output()
        .unwrap();
    if !probe.status.success() {
        let error = String::from_utf8_lossy(&probe.stderr);
        assert!(error.contains("Vulkan device"), "libplacebo probe failed:\n{error}");
        eprintln!("SKIP: libplacebo has no Vulkan device");
        return;
    }
    // HLG signal of a phone shot: greys from shadows to highlights, skin, a lit wall and a red that only
    // looks saturated once converted from BT.2020. Phones put faces and walls around and above HDR
    // reference white (75 %), which the old curve kept at full strength and clipped to white.
    const PATCHES: [[f64; 3]; 8] = [
        [0.25; 3],
        [0.5; 3],
        [0.75; 3],
        [0.85; 3],
        [0.95; 3],
        [0.62, 0.55, 0.50],
        [0.85, 0.83, 0.76],
        [0.70, 0.30, 0.25],
    ];
    // The same light as PQ: HLG on its nominal 1000 nit display, then the PQ curve.
    let hlg_to_pq = |e: [f64; 3]| -> [f64; 3] {
        let scene =
            e.map(|v| if v <= 0.5 { v * v / 3.0 } else { (((v - 0.55991073) / 0.17883277).exp() + 0.28466892) / 12.0 });
        let luma = 0.2627 * scene[0] + 0.678 * scene[1] + 0.0593 * scene[2];
        scene.map(|s| {
            let y = (luma.powf(0.2) * s / 10.0).powf(0.1593017578125);
            ((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y)).powf(78.84375)
        })
    };
    let d = dir("hdr");
    let mut renderer = Renderer::new().unwrap();
    for transfer in ["arib-std-b67", "smpte2084"] {
        let pq = transfer == "smpte2084";
        let source = d.join(format!("{transfer}.ppm"));
        let mut ppm = b"P6\n640 80\n65535\n".to_vec();
        for _ in 0..80 {
            for x in 0..640 {
                let patch = if pq { hlg_to_pq(PATCHES[x / 80]) } else { PATCHES[x / 80] };
                ppm.extend(patch.iter().flat_map(|v| ((v * 65535.0).round() as u16).to_be_bytes()));
            }
        }
        std::fs::write(&source, ppm).unwrap();
        let path = d.join(format!("{transfer}.mp4"));
        // PQ declares a 1000 nit peak like phone HDR10, so libplacebo assumes the peak Nuzky does.
        let metadata = if pq {
            ":master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400"
        } else {
            ""
        };
        let x265 = format!(
            "pools=1:frame-threads=1:log-level=error:crf=4:colorprim=bt2020:transfer={transfer}:colormatrix=bt2020nc:range=limited{metadata}"
        );
        let input = source.to_str().unwrap();
        let yuv = "scale=in_range=pc:out_color_matrix=bt2020nc:out_range=tv,format=yuv420p10le";
        ff(&["-i", input, "-vf", yuv, "-c:v", "libx265", "-x265-params", &x265, "-frames:v", "1"], &path);
        // libplacebo's default tone mapping without per-frame peak detection, which a static curve cannot follow.
        let placebo = "libplacebo=colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=pc:peak_detect=0:apply_dolbyvision=0:format=rgba";
        let reference = raw(&path, &["-vf", placebo]);
        let frame = renderer.render(&project(&path, 200_000), 0, 640, 80, Wait::Exact, false).unwrap();
        let patch = |rgba: &[u8], i: usize| -> [f64; 3] {
            let pixels: Vec<&[u8]> = (20..60)
                .flat_map(|y| (i * 80 + 20..i * 80 + 60).map(move |x| (y * 640 + x) * 4))
                .map(|p| &rgba[p..p + 3])
                .collect();
            std::array::from_fn(|c| pixels.iter().map(|p| p[c] as f64).sum::<f64>() / pixels.len() as f64)
        };
        let ours: Vec<_> = (0..PATCHES.len()).map(|i| patch(&frame, i)).collect();
        let theirs: Vec<_> = (0..PATCHES.len()).map(|i| patch(&reference, i)).collect();
        eprintln!("QA HDR {transfer}: Nuzky {ours:.0?}\n  libplacebo {theirs:.0?}");
        // The old curve showed the 75 % grey at 246 and the wall at (255, 235, 198) against libplacebo's
        // 179 and (221, 203, 172).
        for i in 0..5 {
            let off = (0..3).map(|c| (ours[i][c] - theirs[i][c]).abs()).fold(0.0, f64::max);
            assert!(off <= 12.0, "{transfer} patch {i}: Nuzky {:?}, libplacebo {:?}", ours[i], theirs[i]);
        }
        // Skin and the wall keep libplacebo's hue but come out darker. libplacebo maps luminance, which
        // clipped the red of 12 % of a lit face in an iPhone clip; Nuzky maps the brightest channel. Apple's
        // AVFoundation shows these patches darker still, at (132, 103, 94) and (198, 180, 150).
        for i in [5, 6] {
            let (o, t) = (ours[i], theirs[i]);
            let hue = (1..3).map(|c| (o[c] / o[0] - t[c] / t[0]).abs()).fold(0.0, f64::max);
            assert!(
                o[0] <= t[0] + 4.0 && o[0] >= t[0] - 20.0 && hue <= 0.04,
                "{transfer} patch {i}: Nuzky {o:?}, libplacebo {t:?}"
            );
        }
        // Highlights above reference white keep their steps instead of all clipping to white.
        assert!(ours[4][1] - ours[3][1] > 20.0, "{transfer}: highlights {:?} {:?}", ours[3], ours[4]);
        // Shown untouched, the red is (179, 77, 64); Apple shows it at (178, 26, 41). It is left out of the
        // comparison because libplacebo also remaps the gamut.
        let red = ours[7];
        assert!(red[0] > 160.0 && red[1] < 60.0, "{transfer}: red {red:?}");
    }
}

#[test]
fn lit_skin_from_a_phone_keeps_its_shades() {
    if !available() {
        return;
    }
    let encoders =
        String::from_utf8_lossy(&run(Command::new("ffmpeg").args(["-hide_banner", "-encoders"])).stdout).into_owned();
    if !encoders.contains("libx265") {
        eprintln!("SKIP: ffmpeg without libx265");
        return;
    }
    // HLG skin of an iPhone clip lit by a window, from the face's median to the brightest spot on the
    // forehead. Its red used to reach SDR white from the middle of this ramp on, so foreheads and cheeks
    // showed as flat orange patches.
    const MEDIAN: [f64; 3] = [0.71, 0.60, 0.51];
    const LIT: [f64; 3] = [0.94, 0.89, 0.82];
    const STEPS: usize = 8;
    let d = dir("hdr-skin");
    let source = d.join("skin.ppm");
    let mut ppm = format!("P6\n{} 40\n65535\n", STEPS * 40).into_bytes();
    for _ in 0..40 {
        for x in 0..STEPS * 40 {
            let t = (x / 40) as f64 / (STEPS - 1) as f64;
            ppm.extend(
                (0..3).flat_map(|c| (((MEDIAN[c] + (LIT[c] - MEDIAN[c]) * t) * 65535.0).round() as u16).to_be_bytes()),
            );
        }
    }
    std::fs::write(&source, ppm).unwrap();
    let path = d.join("skin.mp4");
    let yuv = "scale=in_range=pc:out_color_matrix=bt2020nc:out_range=tv,format=yuv420p10le";
    let x265 = "pools=1:frame-threads=1:log-level=error:crf=4:colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc:range=limited";
    ff(&["-i", source.to_str().unwrap(), "-vf", yuv, "-c:v", "libx265", "-x265-params", x265, "-frames:v", "1"], &path);
    let width = STEPS * 40;
    let frame =
        Renderer::new().unwrap().render(&project(&path, 200_000), 0, width as u32, 40, Wait::Exact, false).unwrap();
    let frame = &frame[..];
    let reds: Vec<f64> = (0..STEPS)
        .map(|i| {
            let pixels: Vec<f64> = (10..30)
                .flat_map(|y| (i * 40 + 10..i * 40 + 30).map(move |x| frame[(y * width + x) * 4] as f64))
                .collect();
            pixels.iter().sum::<f64>() / pixels.len() as f64
        })
        .collect();
    eprintln!("QA HDR lit skin red: {reds:.0?}");
    assert!(reds.windows(2).all(|w| w[1] - w[0] >= 2.0) && reds[STEPS - 1] < 248.0, "red of lit skin: {reds:?}");
}

/// Black where the frame shows nothing; the gradient below is never black.
fn black(frame: &[u8], width: usize, x: usize, y: usize) -> bool {
    frame[(y * width + x) * 4..][..3].iter().all(|&v| v < 12)
}

fn crop_and_shape(p: &mut Project, crop: Crop, shape: Option<Shape>) {
    let ClipContent::Media { transform, shape: current, .. } = &mut p.tracks[0].clips[0].content else {
        unreachable!()
    };
    transform.crop = Some(crop);
    *current = shape;
}

#[test]
fn crop_circle_border_and_shadow_match_ffmpeg() {
    if !available() {
        return;
    }
    let d = dir("crop-shape");
    // A gradient with blue everywhere, so nothing of the picture is black and a shifted crop shows.
    let gradient = "color=black:s=320x180,format=rgb24,geq=r='X*255/W':g='Y*255/H':b=128";
    let source = d.join("gradient.png");
    ff(&["-f", "lavfi", "-i", gradient, "-frames:v", "1"], &source);
    let mut p = project(&source, 200_000);
    let mut renderer = Renderer::new().unwrap();
    let mut frame = |p: &Project, w, h| renderer.render(p, 0, w, h, Wait::Exact, false).unwrap();

    // Cropped edges are cut off to the pixel and the rest stays where it was.
    crop_and_shape(&mut p, Crop { left: 0.25, top: 0.1, right: 0.15, bottom: 0.2 }, None);
    let cropped = frame(&p, 320, 180);
    let error = mae(&cropped, &raw(&source, &["-vf", "crop=192:126:80:18,pad=320:180:80:18:black"]));
    assert!(error < 0.5, "crop: pixel MAE={error}");
    for y in 18..144 {
        assert!(black(&cropped, 320, 79, y) && !black(&cropped, 320, 80, y), "left edge at row {y}");
        assert!(!black(&cropped, 320, 271, y) && black(&cropped, 320, 272, y), "right edge at row {y}");
    }
    assert!(black(&cropped, 320, 150, 17) && !black(&cropped, 320, 150, 18) && black(&cropped, 320, 150, 144));

    // A border sits outside the crop: 6 px of red, then the background again.
    let border = Shape { border_width: 6.0, border_color: "#ff0000".into(), ..Shape::default() };
    crop_and_shape(&mut p, Crop { left: 0.25, top: 0.1, right: 0.15, bottom: 0.2 }, Some(border));
    let bordered = frame(&p, 320, 180);
    let px = |x: usize, y: usize| &bordered[(y * 320 + x) * 4..][..3];
    assert!([74, 79].iter().all(|&x| px(x, 80) == [255, 0, 0]) && black(&bordered, 320, 73, 80), "{:?}", px(74, 80));
    assert!(px(150, 12) == [255, 0, 0] && black(&bordered, 320, 150, 11) && px(150, 149) == [255, 0, 0]);
    assert_eq!(&bordered[(80 * 320 + 150) * 4..][..4], &cropped[(80 * 320 + 150) * 4..][..4], "inside is unchanged");

    // Fully rounded corners on a square crop: a circle, as FFmpeg cuts it, at export and preview sizes.
    let round = Shape { radius: 1.0, ..Shape::default() };
    crop_and_shape(&mut p, Crop { left: 0.21875, right: 0.21875, ..Crop::default() }, Some(round));
    let circle = frame(&p, 320, 180);
    let reference = raw(
        &source,
        &[
            "-filter_complex",
            "color=black:s=320x180[bg];[0]format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='255*lte(hypot(X+0.5-160,Y+0.5-90),90)'[fg];[bg][fg]overlay=format=auto",
            "-frames:v",
            "1",
        ],
    );
    let error = mae(&circle, &reference);
    assert!(error < 1.0, "circle: pixel MAE={error}");
    assert!(black(&circle, 320, 72, 2) && black(&circle, 320, 247, 177) && !black(&circle, 320, 160, 2));
    let half = frame(&p, 160, 90);
    let shrunk: Vec<u8> = (0..90 * 160 * 4)
        .map(|i| {
            let (c, x, y) = (i % 4, i / 4 % 160, i / 640);
            let at = |dx: usize, dy: usize| circle[((2 * y + dy) * 320 + 2 * x + dx) * 4 + c] as u32;
            ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8
        })
        .collect();
    let error = mae(&half, &shrunk);
    assert!(error < 2.0, "circle at half size: pixel MAE={error}");

    // A shadow darkens the white canvas below the layer, less above it, and nothing far away.
    p.canvas.background = "#ffffff".into();
    crop_and_shape(&mut p, Crop::default(), Some(Shape { shadow: 1.0, ..Shape::default() }));
    let ClipContent::Media { transform, .. } = &mut p.tracks[0].clips[0].content else { unreachable!() };
    transform.scale = 0.5;
    let shadowed = frame(&p, 320, 180);
    let grey = |x: usize, y: usize| shadowed[(y * 320 + x) * 4];
    assert!(
        grey(160, 136) < 200 && grey(160, 43) > grey(160, 136) && grey(160, 170) == 255,
        "{:?}",
        [grey(160, 136), grey(160, 43)]
    );

    // The blurred background shows only what the crop keeps: the darker red left half, not the red right one.
    p.canvas.background_blur = 1.0;
    crop_and_shape(&mut p, Crop { right: 0.5, ..Crop::default() }, None);
    let ClipContent::Media { transform, .. } = &mut p.tracks[0].clips[0].content else { unreachable!() };
    transform.scale = 1.0;
    let blurred = frame(&p, 320, 180);
    let red = blurred[(90 * 320 + 310) * 4];
    assert!(red < 140, "red {red} of the cut-off half shows in the background");
}

#[test]
fn crop_follows_the_picture_as_shown_after_exif_rotation_and_mirroring() {
    if !available() {
        return;
    }
    let d = dir("crop-orientation");
    let source = d.join("gradient.jpg");
    let gradient = "color=black:s=96x64,format=rgb24,geq=r='X*255/W':g='Y*255/H':b=128";
    ff(&["-f", "lavfi", "-i", gradient, "-frames:v", "1", "-q:v", "2"], &source);
    let jpeg = std::fs::read(&source).unwrap();
    let mut renderer = Renderer::new().unwrap();
    // Upright, turned 90° and turned 90° with a mirror, as a front camera saves selfies.
    for orientation in [1, 6, 5] {
        let path = d.join(format!("orientation-{orientation}.jpg"));
        std::fs::write(&path, with_exif_orientation(&jpeg, orientation)).unwrap();
        let mut p = project(&path, 200_000);
        let (w, h) = (p.canvas.width, p.canvas.height);
        crop_and_shape(&mut p, Crop { left: 0.25, top: 0.125, right: 0.125, bottom: 0.25 }, None);
        let got = renderer.render(&p, 0, w, h, Wait::Exact, false).unwrap();
        let (left, top) = (w / 4, h / 8);
        let filter = format!("crop={}:{}:{left}:{top},pad={w}:{h}:{left}:{top}:black", w * 5 / 8, h * 5 / 8);
        let error = mae(&got, &raw(&path, &["-frames:v", "1", "-vf", &filter]));
        assert!(error < 3.0, "orientation {orientation}: pixel MAE={error}");
    }
}

#[test]
fn captions_in_a_small_preview_match_the_export_scaled_down_by_ffmpeg() {
    use nuzky_engine::edit::{CaptionSegment, EditCmd};
    if !available() {
        return;
    }
    let d = dir("preview-captions");
    // Grey, so the black outline shows like it does over a video.
    let grey = d.join("grey.png");
    ff(&["-f", "lavfi", "-i", "color=0x808080:s=1080x1920", "-frames:v", "1"], &grey);
    let mut p = project(&grey, 1_000_000);
    let style = TextStyle {
        font_family: None,
        font_size: 110.0,
        color: "#ffffff".into(),
        bold: true,
        stroke_width: 11.0,
        stroke_color: "#000000".into(),
        background: None,
        max_width: None,
        highlight: None,
        keywords: None,
    };
    let segment = CaptionSegment { start_us: 0, end_us: 1_000_000, text: "Poslední dva".into(), words: Vec::new() };
    p.apply(EditCmd::AddCaptions { segments: vec![segment], style, anim_in: None, anim_out: None }).unwrap();
    let mut renderer = Renderer::new().unwrap();
    let export = d.join("export.rgba");
    std::fs::write(&export, renderer.render(&p, 500_000, 1080, 1920, Wait::Exact, false).unwrap()).unwrap();
    // The preview of a vertical video beside the library, the inspector and the AI panel in a 1080p window.
    let (w, h) = (296, 526);
    let preview = renderer.render(&p, 500_000, w, h, Wait::Exact, false).unwrap();
    let reference = d.join("reference.rgba");
    let scale = format!("scale={w}:{h}:flags=area");
    let input = ["-f", "rawvideo", "-pix_fmt", "rgba", "-s", "1080x1920", "-i", export.to_str().unwrap()];
    ff(&[&input[..], &["-vf", &scale, "-f", "rawvideo", "-pix_fmt", "rgba"]].concat(), &reference);
    let reference = std::fs::read(&reference).unwrap();
    // Only the rows of the caption: the plain frame around it would hide any difference.
    let row = w as usize * 4;
    let lit: Vec<usize> = (0..h as usize)
        .filter(|y| reference[y * row..(y + 1) * row].as_chunks::<4>().0.iter().any(|px| px[0].abs_diff(128) > 40))
        .collect();
    let band = lit[0] * row..(lit[lit.len() - 1] + 1) * row;
    let error = mae(&preview[band.clone()], &reference[band]);
    assert!(error < 4.0, "caption pixel MAE={error}");
}

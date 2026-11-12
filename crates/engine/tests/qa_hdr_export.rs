mod qa_support;
use nuzky_engine::{
    edit::EditCmd,
    export::{ExportOptions, export, hdr_footage},
    media::probe,
    model::*,
};
use qa_support::*;
use std::{path::Path, process::Command, sync::atomic::AtomicBool};

/// HLG signal of a phone shot: greys from shadows to highlights, skin, a lit wall and a red that only BT.2020 holds.
/// Phones put faces and walls around and above HDR reference white (75 %).
const PATCHES: [[f64; 3]; 8] =
    [[0.25; 3], [0.5; 3], [0.75; 3], [0.85; 3], [0.95; 3], [0.62, 0.55, 0.50], [0.85, 0.83, 0.76], [0.70, 0.30, 0.25]];
/// SDR BT.709 colours of a screen recording or a title: white, mid grey, black, red, skin.
const SDR: [[u8; 3]; 5] = [[255; 3], [128; 3], [0; 3], [255, 0, 0], [200, 150, 120]];
const W: u32 = 640;
const H: u32 = 80;

/// The same light as PQ: HLG on its nominal 1000 nit display, then the PQ curve (BT.2100).
fn hlg_to_pq(e: [f64; 3]) -> [f64; 3] {
    let scene =
        e.map(|v| if v <= 0.5 { v * v / 3.0 } else { (((v - 0.55991073) / 0.17883277).exp() + 0.28466892) / 12.0 });
    let luma = 0.2627 * scene[0] + 0.678 * scene[1] + 0.0593 * scene[2];
    scene.map(|s| {
        let y = (luma.powf(0.2) * s / 10.0).powf(0.1593017578125);
        ((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y)).powf(78.84375)
    })
}

/// SDR BT.709 shown at HDR reference white in HLG BT.2020 (BT.2408): BT.1886 display light, 1 at 203 nits of HLG's
/// 1000, without HLG's OOTF, then its OETF.
fn sdr_to_hlg(rgb: [u8; 3]) -> [f64; 3] {
    let linear = rgb.map(|v| (v as f64 / 255.0).powf(2.4));
    let to_2020 = [[0.6274, 0.3293, 0.0433], [0.0691, 0.9195, 0.0114], [0.0164, 0.0880, 0.8956]];
    let display = to_2020.map(|row: [f64; 3]| (0..3).map(|c| row[c] * linear[c]).sum::<f64>() * 0.203);
    let y = (0.2627 * display[0] + 0.678 * display[1] + 0.0593 * display[2]).max(1e-6);
    display.map(|d| {
        let e = d * y.powf(-1.0 / 6.0);
        if e <= 1.0 / 12.0 { (3.0 * e).sqrt() } else { 0.17883277 * (12.0 * e - 0.28466892).ln() + 0.55991073 }
    })
}

fn has_x265() -> bool {
    let encoders = run(Command::new("ffmpeg").args(["-hide_banner", "-encoders"])).stdout;
    let found = String::from_utf8_lossy(&encoders).contains("libx265");
    if !found {
        eprintln!("SKIP: ffmpeg without libx265");
    }
    found
}

/// A phone's HDR clip of the patches side by side, 10-bit HEVC with the given transfer.
fn phone_clip(d: &Path, transfer: &str) -> std::path::PathBuf {
    let pq = transfer == "smpte2084";
    let source = d.join(format!("{transfer}.ppm"));
    let mut ppm = format!("P6\n{W} {H}\n65535\n").into_bytes();
    for _ in 0..H {
        for x in 0..W as usize {
            let patch = if pq { hlg_to_pq(PATCHES[x / 80]) } else { PATCHES[x / 80] };
            ppm.extend(patch.iter().flat_map(|v| ((v * 65535.0).round() as u16).to_be_bytes()));
        }
    }
    std::fs::write(&source, ppm).unwrap();
    let path = d.join(format!("{transfer}.mov"));
    let x265 = format!(
        "pools=1:frame-threads=1:log-level=error:crf=4:colorprim=bt2020:transfer={transfer}:colormatrix=bt2020nc:range=limited"
    );
    let yuv = "scale=in_range=pc:out_color_matrix=bt2020nc:out_range=tv,format=yuv420p10le";
    let input = source.to_str().unwrap();
    let args = ["-loop", "1", "-i", input, "-t", "1", "-r", "30", "-vf", yuv, "-c:v", "libx265", "-x265-params", &x265];
    ff(&[&args[..], &["-tag:v", "hvc1"]].concat(), &path);
    path
}

/// An SDR screen recording of the `SDR` colours side by side, 8-bit H.264 BT.709.
fn sdr_clip(d: &Path) -> std::path::PathBuf {
    let source = d.join("sdr.ppm");
    let mut ppm = format!("P6\n{W} {H}\n255\n").into_bytes();
    for _ in 0..H {
        for x in 0..W as usize {
            ppm.extend(SDR[x * SDR.len() / W as usize]);
        }
    }
    std::fs::write(&source, ppm).unwrap();
    let path = d.join("sdr.mp4");
    let yuv = "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p";
    let input = source.to_str().unwrap();
    let tags = ["-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709"];
    let args = ["-loop", "1", "-i", input, "-t", "1", "-r", "30", "-vf", yuv, "-c:v", "libx264", "-qp", "0"];
    ff(&[&args[..], &tags].concat(), &path);
    path
}

/// The frame of `video` at `at` seconds as RGB in 0..1 of its own signal, read through its BT.2020 matrix.
fn frame(video: &Path, at: f64) -> Vec<[f64; 3]> {
    let filter = "scale=in_color_matrix=bt2020:in_range=tv:flags=accurate_rnd+full_chroma_int,format=rgb48le";
    let out = run(Command::new("ffmpeg").args(["-v", "error", "-ss", &at.to_string(), "-i"]).arg(video).args([
        "-frames:v",
        "1",
        "-vf",
        filter,
        "-f",
        "rawvideo",
        "-",
    ]))
    .stdout;
    assert_eq!(out.len(), (W * H * 6) as usize, "one {W}×{H} frame of 16-bit RGB");
    out.as_chunks::<6>()
        .0
        .iter()
        .map(|p| std::array::from_fn(|c| u16::from_le_bytes([p[c * 2], p[c * 2 + 1]]) as f64 / 65535.0))
        .collect()
}

/// The mean colour of a 20 × 20 square around (x, y).
fn around(frame: &[[f64; 3]], x: u32, y: u32) -> [f64; 3] {
    let pixels: Vec<_> =
        (y - 10..y + 10).flat_map(|py| (x - 10..x + 10).map(move |px| frame[(py * W + px) as usize])).collect();
    std::array::from_fn(|c| pixels.iter().map(|p| p[c]).sum::<f64>() / pixels.len() as f64)
}

fn off(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f64::max)
}

fn hdr_options() -> ExportOptions {
    ExportOptions { preset: "ultrafast".into(), crf: 4, replace_existing: true, hdr: true, ..ExportOptions::default() }
}

#[test]
fn hdr_export_keeps_the_light_of_hlg_and_pq_and_puts_sdr_at_reference_white() {
    if !available() || !has_x265() {
        return;
    }
    let d = dir("hdr-export");
    let mut p = Project::new("HDR QA");
    (p.canvas.width, p.canvas.height, p.canvas.fps) = (W, H, 30);
    // Mid grey behind the SDR clip, which shows at half size.
    p.canvas.background = "#808080".into();
    for (id, path, start) in [("hlg", phone_clip(&d, "arib-std-b67"), 0), ("pq", phone_clip(&d, "smpte2084"), 1)] {
        p.assets.push(probe(&path, id.into()).unwrap());
        p.tracks[0].clips.push(clip(id, id, start * 1_000_000, 1_000_000));
    }
    p.assets.push(probe(&sdr_clip(&d), "sdr".into()).unwrap());
    let mut sdr = clip("sdr", "sdr", 2_000_000, 1_000_000);
    if let ClipContent::Media { transform, .. } = &mut sdr.content {
        transform.scale = 0.5;
    }
    p.tracks[0].clips.push(sdr);
    assert_eq!(hdr_footage(&p), [p.assets[0].name.clone(), p.assets[1].name.clone()], "the HLG and PQ clips are HDR");

    let out = d.join("hdr.mp4");
    export(&p, &d.join("cache"), &out, &hdr_options(), &AtomicBool::new(false), |_| {}).unwrap();

    let j = info(&out);
    let video = j["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == "video").unwrap().clone();
    let field = |key: &str| video[key].as_str().unwrap_or_default().to_owned();
    assert_eq!(
        ["codec_name", "codec_tag_string", "profile", "pix_fmt"].map(field),
        ["hevc", "hvc1", "Main 10", "yuv420p10le"],
        "10-bit HEVC with the sample entry Apple's players read"
    );
    assert_eq!(
        ["color_transfer", "color_primaries", "color_space", "color_range"].map(field),
        ["arib-std-b67", "bt2020", "bt2020nc", "tv"],
        "tagged as HLG BT.2020, as iPhones record HDR"
    );

    for (what, at, tolerance) in [("HLG", 0.5, 0.012), ("PQ", 1.5, 0.02)] {
        let f = frame(&out, at);
        for (i, want) in PATCHES.iter().enumerate() {
            let got = around(&f, i as u32 * 80 + 40, H / 2);
            eprintln!("QA HDR export {what} patch {i}: {got:.3?}, want {want:.3?}");
            assert!(off(got, *want) <= tolerance, "{what} patch {i}: {got:.3?}, want {want:.3?}");
        }
        // Highlights above reference white stay above it instead of being tone mapped to SDR.
        assert!(around(&f, 4 * 80 + 40, H / 2)[1] > 0.9, "{what} highlights stay HDR");
    }

    let f = frame(&out, 2.5);
    // The SDR clip covers the middle half of the canvas: 320 × 40 pixels from (160, 20), 64 for each colour.
    for (i, colour) in SDR.iter().enumerate() {
        let (got, want) = (around(&f, 160 + i as u32 * 64 + 32, H / 2), sdr_to_hlg(*colour));
        eprintln!("QA HDR export SDR {colour:?}: {got:.3?}, want {want:.3?}");
        assert!(off(got, want) <= 0.012, "SDR {colour:?}: {got:.3?}, want {want:.3?}");
    }
    let white = sdr_to_hlg([255; 3]);
    assert!((white[1] - 0.75).abs() < 0.002, "SDR white is HDR reference white, 75 % HLG: {white:?}");
    let (got, want) = (around(&f, 60, H / 2), sdr_to_hlg([128; 3]));
    assert!(off(got, want) <= 0.012, "the canvas colour: {got:.3?}, want {want:.3?}");
}

#[test]
fn hdr_export_refuses_a_video_without_hdr_footage() {
    if !available() {
        return;
    }
    let d = dir("hdr-export-sdr");
    let p = project(&sdr_clip(&d), 500_000);
    assert!(hdr_footage(&p).is_empty());
    let out = d.join("refused.mp4");
    std::fs::remove_file(&out).ok();
    let error = export(&p, &d.join("cache"), &out, &hdr_options(), &AtomicBool::new(false), |_| {}).unwrap_err();
    let error = format!("{error:#}");
    assert_eq!(error, "HDR_UNAVAILABLE: No clip in this video was filmed in HDR. Export in SDR.");
    assert!(!out.exists(), "nothing is written");
}

#[test]
fn hdr_export_draws_dissolves_blurred_backgrounds_crops_and_text() {
    if !available() || !has_x265() {
        return;
    }
    let d = dir("hdr-export-layers");
    let hlg = phone_clip(&d, "arib-std-b67");
    let mut p = project(&hlg, 1_000_000);
    p.canvas.fps = 30;
    // The phone clip at half size and cropped, over its own blurred picture, which is HDR too.
    p.canvas.background_blur = 0.5;
    if let ClipContent::Media { transform, .. } = &mut p.tracks[0].clips[0].content {
        transform.scale = 0.5;
        transform.crop = Some(Crop { left: 0.0, top: 0.0, right: 0.625, bottom: 0.0 });
    }
    p.assets.push(probe(&sdr_clip(&d), "sdr".into()).unwrap());
    let mut sdr = clip("sdr", "sdr", 1_000_000, 1_000_000);
    sdr.transition_in = Some(Transition { kind: TransitionKind::Dissolve, duration_us: 400_000 });
    p.tracks[0].clips.push(sdr);
    let style = serde_json::from_value(serde_json::json!({"fontSize": 60.0, "color": "#ffffff"})).unwrap();
    p.apply(EditCmd::AddText { start_us: 0, text: "HDR".into(), style }).unwrap();

    let out = d.join("layers.mp4");
    export(&p, &d.join("cache"), &out, &hdr_options(), &AtomicBool::new(false), |_| {}).unwrap();

    let f = frame(&out, 0.5);
    let brightest = f.iter().map(|p| p[1]).fold(0.0, f64::max);
    // The crop leaves the phone's greys up to reference white, so the white text is the brightest: reference white too.
    assert!((brightest - 0.75).abs() <= 0.02, "white text at HDR reference white: brightest {brightest:.3}");
    // The blurred picture shows left of the clip, which takes the middle half.
    let behind = around(&f, 60, H / 2);
    assert!(behind.iter().all(|&v| (0.15..0.75).contains(&v)), "the blurred phone clip behind it: {behind:.3?}");
    // Midway through the dissolve the phone's 25 % grey blends with the SDR clip's mid grey there, 0.447 in HLG.
    let f = frame(&out, 1.0);
    let mid = around(&f, 180, H / 2)[1];
    assert!((0.29..0.41).contains(&mid), "the dissolve blends HDR and SDR in HLG: {mid:.3}");
}

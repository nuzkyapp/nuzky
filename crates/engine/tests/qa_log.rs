//! Log footage and LUTs as the renderer the preview and the export share draws them, against the CPU conversion
//! (`color::apply_to_rgba`, which the Filters tab's tiles use) of the picture FFmpeg decodes from the same file.

mod qa_support;

use std::path::{Path, PathBuf};

use nuzky_engine::color::{self, Lut3d};
use nuzky_engine::model::{ClipContent, LogProfile, Lut, Project};
use nuzky_engine::{Renderer, Wait};
use qa_support::*;

const W: usize = 160;
const H: usize = 96;

/// Patches of the chart, left to right in two rows of four: black, 18 % grey, 90 % white, four times white; skin, a
/// blue sky, a saturated red and a saturated green, all scene light in BT.2020.
const PATCHES: [[f64; 3]; 8] = [
    [0.0, 0.0, 0.0],
    [0.18, 0.18, 0.18],
    [0.9, 0.9, 0.9],
    [3.6, 3.6, 3.6],
    [0.42, 0.25, 0.17],
    [0.16, 0.28, 0.55],
    [0.6, 0.04, 0.03],
    [0.05, 0.5, 0.04],
];

/// The middle of a patch, in pixels.
fn centre(patch: usize) -> (usize, usize) {
    ((patch % 4) * W / 4 + W / 8, (patch / 4) * (H * 3 / 8) + H * 3 / 16)
}

/// The chart: the patches over the top three quarters, a grey ramp from black to eight times white below.
fn scene(x: usize, y: usize) -> [f64; 3] {
    if y < H * 3 / 4 {
        PATCHES[(y / (H * 3 / 8)) * 4 + x * 4 / W]
    } else {
        [7.2 * (x as f64 / (W - 1) as f64).powi(3); 3]
    }
}

/// Apple Log's encoding of scene light, from Apple's Apple Log Profile white paper.
fn apple_log(r: f64) -> f64 {
    let (r0, rt, sigma, beta, gamma, delta) = (-0.05641088, 0.01, 47.28711236, 0.00964052, 0.08550479, 0.69336945);
    if r >= rt {
        gamma * (r + beta).log2() + delta
    } else if r >= r0 {
        sigma * (r - r0).powi(2)
    } else {
        0.0
    }
}

/// The chart in Apple Log as an iPhone records it: ProRes 422 HQ, 10-bit, BT.2020 primaries and matrix, limited
/// range and no transfer named, so nothing but the user's choice says it is log.
fn log_video(dir: &Path) -> PathBuf {
    let png = dir.join("apple-log-chart.png");
    let mut samples = Vec::with_capacity(W * H * 6);
    for y in 0..H {
        for x in 0..W {
            for c in scene(x, y) {
                samples.extend_from_slice(&((apple_log(c).clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes());
            }
        }
    }
    let mut encoder = png::Encoder::new(std::fs::File::create(&png).unwrap(), W as u32, H as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Sixteen);
    encoder.write_header().unwrap().write_image_data(&samples).unwrap();
    let out = dir.join("apple-log.mov");
    let png = png.to_str().unwrap();
    #[rustfmt::skip]
    ff(&[
        "-loop", "1", "-framerate", "30", "-i", png, "-t", "1",
        "-vf", "scale=out_color_matrix=bt2020:out_range=tv:flags=accurate_rnd+full_chroma_int",
        "-c:v", "prores_ks", "-profile:v", "3", "-pix_fmt", "yuv422p10le",
        "-colorspace", "bt2020nc", "-color_primaries", "bt2020", "-color_range", "tv",
    ], &out);
    out
}

fn with_color(mut project: Project, log: LogProfile, lut: Lut) -> Project {
    let ClipContent::Media { log: l, lut: t, .. } = &mut project.tracks[0].clips[0].content else { panic!() };
    (*l, *t) = (log, lut);
    project
}

fn render(renderer: &mut Renderer, project: &Project) -> Vec<u8> {
    renderer.render(project, 500_000, W as u32, H as u32, Wait::Exact, false).unwrap()
}

/// The frame FFmpeg decodes, converted on the CPU.
fn expected(video: &Path, log: LogProfile, lut: Option<(&Lut3d, f32)>) -> Vec<u8> {
    let decoded = raw(video, &[]);
    let mut frame = decoded[..W * H * 4].to_vec();
    color::apply_to_rgba(&mut frame, log, lut);
    frame
}

fn at(frame: &[u8], (x, y): (usize, usize)) -> [i32; 3] {
    let p = &frame[(y * W + x) * 4..];
    [p[0], p[1], p[2]].map(i32::from)
}

fn worst(a: &[u8], b: &[u8]) -> i32 {
    a.iter().zip(b).map(|(&x, &y)| (x as i32 - y as i32).abs()).max().unwrap()
}

/// A warm grade with a curve in it, so the LUT is not a linear map the trilinear lookup gets right by chance.
fn warm() -> Lut3d {
    Lut3d::from_fn(17, |[r, g, b]| [r.powf(0.8), g * 0.95, b * b])
}

fn cube(lut: &Lut3d) -> String {
    let mut text = format!("TITLE \"Warm\"\nLUT_3D_SIZE {}\n", lut.size);
    for [r, g, b] in &lut.data {
        text += &format!("{r:.6} {g:.6} {b:.6}\n");
    }
    text
}

#[test]
fn apple_log_shows_normal_colour_in_the_preview_and_export_renderer() {
    if !available() {
        return;
    }
    let dir = dir("log-apple");
    let video = log_video(&dir);
    let project = project(&video, 1_000_000);
    let mut renderer = Renderer::new().unwrap();

    let flat = render(&mut renderer, &project);
    let converted = render(&mut renderer, &with_color(project.clone(), LogProfile::AppleLog, Lut::None));
    let want = expected(&video, LogProfile::AppleLog, None);
    assert!(
        mae(&converted, &want) < 0.5 && worst(&converted, &want) <= 3,
        "{} {}",
        mae(&converted, &want),
        worst(&converted, &want)
    );

    // As recorded, grey and white sit 50 levels apart and black is lifted; converted they sit where Rec.709 video
    // has them, and skin, sky, red and green keep their hues.
    let [black, grey, white, bright] = [0, 1, 2, 3].map(|p| at(&converted, centre(p)));
    let [flat_black, flat_grey, flat_white] = [0, 1, 2].map(|p| at(&flat, centre(p))[1]);
    assert!(flat_white - flat_grey < 60 && flat_black > 30, "flat {flat_black} {flat_grey} {flat_white}");
    for (patch, want) in [(black, 0), (grey, 112), (white, 224)] {
        assert!(patch.iter().all(|c| (c - want).abs() <= 4), "{patch:?} not {want}");
    }
    assert!(bright.iter().all(|&c| (240..255).contains(&c)), "four times white: {bright:?}");
    let [skin, sky, red, green] = [4, 5, 6, 7].map(|p| at(&converted, centre(p)));
    assert!(skin[0] > skin[1] && skin[1] > skin[2], "skin {skin:?}");
    assert!(sky[2] > sky[1] && sky[1] > sky[0], "sky {sky:?}");
    assert!(red[0] > 150 && red[1] < 60 && red[2] < 60, "red {red:?}");
    assert!(green[1] > 150 && green[0] < 90 && green[2] < 90, "green {green:?}");
    // The ramp rises without steps or bands.
    for x in 0..W - 1 {
        let (a, b) = (at(&converted, (x, H - 4))[1], at(&converted, (x + 1, H - 4))[1]);
        assert!((0..=6).contains(&(b - a)), "ramp at {x}: {a} then {b}");
    }

    // Compare draws the left of the frame as recorded and the right converted, and an export never sets it.
    renderer.set_compare(Some(0.5));
    let split = render(&mut renderer, &with_color(project.clone(), LogProfile::AppleLog, Lut::None));
    renderer.set_compare(None);
    for y in [centre(1).1, H - 4] {
        assert_eq!(at(&split, (W / 4, y)), at(&flat, (W / 4, y)));
        assert_eq!(at(&split, (W * 3 / 4, y)), at(&converted, (W * 3 / 4, y)));
    }
}

#[test]
fn a_lut_mixes_over_the_log_conversion_at_its_strength() {
    if !available() {
        return;
    }
    let dir = dir("log-lut");
    let video = log_video(&dir);
    let project = project(&video, 1_000_000);
    let luts = dir.join("luts");
    std::fs::create_dir_all(&luts).unwrap();
    let id = "00112233aabbccdd";
    std::fs::write(luts.join(format!("{id}.cube")), cube(&warm())).unwrap();
    let lut = color::read_cube(&luts.join(format!("{id}.cube"))).unwrap();
    let mut renderer = Renderer::new().unwrap();
    renderer.use_luts(Some(luts.clone()));

    for (log, strength) in [(LogProfile::None, 1.0), (LogProfile::AppleLog, 0.5), (LogProfile::AppleLog, 0.0)] {
        let graded = with_color(project.clone(), log, Lut::File { id: id.into(), name: "Warm".into(), strength });
        let got = render(&mut renderer, &graded);
        let want = expected(&video, log, Some((&lut, strength)));
        assert!(mae(&got, &want) < 0.5 && worst(&got, &want) <= 3, "{log:?} {strength}: {}", worst(&got, &want));
    }

    // Compare draws the left of the frame as recorded, without the profile or the LUT.
    let flat = render(&mut renderer, &project);
    renderer.set_compare(Some(0.5));
    let graded = with_color(
        project.clone(),
        LogProfile::AppleLog,
        Lut::File { id: id.into(), name: "Warm".into(), strength: 0.5 },
    );
    let split = render(&mut renderer, &graded);
    renderer.set_compare(None);
    let whole = render(&mut renderer, &graded);
    for y in [centre(4).1, H - 4] {
        assert_eq!(at(&split, (W / 4, y)), at(&flat, (W / 4, y)));
        assert_eq!(at(&split, (W * 3 / 4, y)), at(&whole, (W * 3 / 4, y)));
    }

    // A LUT that is not on this computer leaves the clip as its profile makes it, and says so by name.
    let elsewhere = Lut::File { id: "ffffffffffffffff".into(), name: "Teal and orange".into(), strength: 1.0 };
    let missing = with_color(project.clone(), LogProfile::AppleLog, elsewhere);
    assert_eq!(
        render(&mut renderer, &missing),
        render(&mut renderer, &with_color(project, LogProfile::AppleLog, Lut::None))
    );
    assert_eq!(color::missing_luts(&missing, Some(&luts)), ["Teal and orange"]);
}

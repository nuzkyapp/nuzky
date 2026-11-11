//! Split screens and adjustment layers on real files: a split screen shows each clip filling its own cell, as FFmpeg
//! scales and crops the same picture, without a seam between cells; an adjustment layer grades everything below it as
//! the clips' own adjustments would, leaves text above it and the time outside it alone, and the export shows what the
//! preview does.
mod qa_support;
use nuzky_engine::{
    Pending, Renderer, Wait,
    edit::{EditCmd, SplitLayout},
    export::{ExportOptions, export},
    media::probe,
    model::*,
};
use qa_support::*;
use std::{path::Path, sync::atomic::AtomicBool};

const FPS: i64 = 25;
const LENGTH_US: i64 = 400_000;

/// A phone video turned a quarter by its display matrix, upright 64 × 96.
fn phone(d: &Path) -> std::path::PathBuf {
    let base = d.join("base.mp4");
    ff(&["-f", "lavfi", "-i", "testsrc2=size=96x64:rate=25:duration=0.4", "-c:v", "libx264", "-crf", "8"], &base);
    let path = d.join("phone.mp4");
    ff(&["-display_rotation", "90", "-i", base.to_str().unwrap(), "-c", "copy"], &path);
    path
}

fn solid(d: &Path, name: &str, colour: &str, size: &str) -> std::path::PathBuf {
    let path = d.join(format!("{name}.mp4"));
    let source = format!("color=c={colour}:size={size}:rate=25:duration=0.4");
    ff(&["-f", "lavfi", "-i", &source, "-c:v", "libx264", "-crf", "8", "-pix_fmt", "yuv420p"], &path);
    path
}

/// The clip of `asset` added over the whole picture on a track of its own, from the start.
fn overlay(p: &mut Project, asset: Asset) -> String {
    let id = asset.id.clone();
    p.assets.push(asset);
    let out = p.apply(EditCmd::AddOverlay { asset_id: id, start_us: 0, duration_us: Some(LENGTH_US) }).unwrap();
    out.select[0].clone()
}

fn frame_at(renderer: &mut Renderer, p: &Project, t: i64) -> Vec<u8> {
    renderer.render(p, t, p.canvas.width, p.canvas.height, Wait::Exact, false).unwrap()
}

fn exported(p: &Project, d: &Path, t: i64) -> Vec<u8> {
    let out = d.join("export.mp4");
    let options = ExportOptions { preset: "ultrafast".into(), crf: 4, replace_existing: true, ..Default::default() };
    export(p, &d.join("cache"), &out, &options, &AtomicBool::new(false), |_| {}).unwrap();
    let decoded = raw(&out, &[]);
    let size = (p.canvas.width * p.canvas.height * 4) as usize;
    let index = (t * FPS / 1_000_000) as usize;
    decoded[index * size..(index + 1) * size].to_vec()
}

fn pixel(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * w + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

#[test]
fn a_split_screen_fills_each_cell_with_its_clip_without_a_seam() {
    if !available() {
        return;
    }
    let d = dir("layout-split");
    let mut p = project(&phone(&d), LENGTH_US);
    (p.canvas.width, p.canvas.height, p.canvas.fps) = (108, 192, FPS as u32);
    p.canvas.background = "#000000".into();
    let red = overlay(&mut p, probe(&solid(&d, "red", "red", "160x90"), "red".into()).unwrap());
    let blue = overlay(&mut p, probe(&solid(&d, "blue", "blue", "90x160"), "blue".into()).unwrap());
    let ids = vec!["qa-clip".to_owned(), red, blue];
    p.apply(EditCmd::SplitScreen { clip_ids: ids, layout: SplitLayout::OneOverTwo }).unwrap();

    let t = 120_000;
    let mut renderer = Renderer::new().unwrap();
    renderer.use_mattes(d.join("cache"), Pending::Fail);
    let shown = frame_at(&mut renderer, &p, t);

    // The phone video covers the top half as FFmpeg scales it to the width and crops its middle.
    let phone_path = d.join("phone.mp4");
    let reference = raw(&phone_path, &["-vf", &format!("scale=108:162:flags={SWS_FLAGS},crop=108:96")]);
    let reference = &reference[3 * 108 * 96 * 4..4 * 108 * 96 * 4];
    let top = &shown[..108 * 96 * 4];
    let error = mae(top, reference);
    assert!(error < 6.0, "the top cell differs from FFmpeg's scaled and cropped picture: MAE {error}");

    // Every pixel of the bottom half is the red or the blue clip, the line between them included.
    for y in 96..192 {
        for x in 0..108 {
            let [r, g, b] = pixel(&shown, 108, x, y);
            let expected = if x < 54 { [r, b] } else { [b, r] };
            assert!(
                expected[0] > 230 && expected[1] < 30 && g < 30,
                "({x}, {y}) is {:?}, not the {} clip",
                [r, g, b],
                if x < 54 { "red" } else { "blue" }
            );
        }
    }
    // Nothing of the canvas shows between the top cell and the bottom ones.
    for x in 0..108 {
        let [r, g, b] = pixel(&shown, 108, x, 95);
        assert!(r as u32 + g as u32 + b as u32 > 60, "({x}, 95) is dark: {:?}", [r, g, b]);
    }

    let error = mae(&exported(&p, &d, t), &shown);
    assert!(error < 4.0, "the export differs from the preview: MAE {error}");
}

fn title(start: i64, duration: i64) -> Clip {
    let style = TextStyle {
        font_family: None,
        font_size: 28.0,
        color: "#ff2020".into(),
        bold: true,
        font_weight: None,
        stroke_width: 0.0,
        stroke_color: "#000000".into(),
        background: Some("#20ff20ff".into()),
        max_width: None,
        highlight: None,
        keywords: None,
        shadow: None,
        letter_case: Default::default(),
        letter_spacing: 0.0,
        line_spacing: 0.0,
        align: TextAlign::Center,
        box_radius: BOX_RADIUS,
    };
    let transform = Transform { y: 0.3, ..Transform::default() };
    Clip::new(
        "title".into(),
        start,
        duration,
        ClipContent::Text { text: "Hi".into(), style, transform, words: Vec::new() },
    )
}

#[test]
fn an_adjustment_layer_grades_every_clip_below_it_and_nothing_above_it_or_after_it() {
    if !available() {
        return;
    }
    let d = dir("layout-adjustment");
    let mut plain = project(&phone(&d), LENGTH_US);
    (plain.canvas.width, plain.canvas.height, plain.canvas.fps) = (64, 96, FPS as u32);
    let small = probe(&solid(&d, "orange", "orange", "32x32"), "orange".into()).unwrap();
    plain.assets.push(small);
    let pip = plain
        .apply(EditCmd::AddPictureInPicture { asset_id: "orange".into(), start_us: 0, duration_us: Some(LENGTH_US) })
        .unwrap()
        .select[0]
        .clone();
    let grade = Adjust { saturation: -1.0, contrast: 0.4, temperature: 0.5, vignette: 0.3, ..Adjust::default() };
    // Over the first 200 ms only.
    let mut graded = plain.clone();
    let layer = graded
        .apply(EditCmd::AddAdjustment { start_us: 0, duration_us: Some(200_000), adjust: Some(grade) })
        .unwrap()
        .select[0]
        .clone();
    let track = graded.tracks.iter().find(|t| t.clips.iter().any(|c| c.id == layer)).unwrap();
    assert_eq!((track.kind, track.name.as_str()), (TrackKind::Video, "Adjustment"));
    assert_eq!(graded.tracks.iter().position(|t| t.id == track.id), Some(graded.tracks.len() - 1));
    let bare = plain.clone();
    // A title added afterwards goes on a text track above the adjustment layer.
    for p in [&mut plain, &mut graded] {
        p.tracks.push(Track {
            id: "titles".into(),
            kind: TrackKind::Text,
            name: "Text".into(),
            muted: false,
            hidden: false,
            keep_in_place: false,
            clips: vec![title(0, LENGTH_US)],
            translation: None,
        });
    }

    let mut renderer = Renderer::new().unwrap();
    renderer.use_mattes(d.join("cache"), Pending::Fail);
    let t = 120_000;
    let without = frame_at(&mut renderer, &plain, t);
    let untitled = frame_at(&mut renderer, &bare, t);
    let with = frame_at(&mut renderer, &graded, t);
    let changed = mae(&with, &without);
    assert!(changed > 15.0, "the adjustment layer changes the picture: MAE {changed}");

    // Desaturating and warming are per pixel, so grading each clip on its own gives the same picture, except the
    // vignette, which on the layer darkens the corners of the whole frame rather than of each clip.
    let mut each = plain.clone();
    for clip in each.tracks.iter_mut().flat_map(|t| &mut t.clips) {
        if let ClipContent::Media { adjust, .. } = &mut clip.content {
            *adjust = Adjust { vignette: 0.0, ..grade };
        }
    }
    let mut graded_flat = graded.clone();
    for clip in graded_flat.tracks.iter_mut().flat_map(|t| &mut t.clips) {
        if let ClipContent::Adjustment { adjust, .. } = &mut clip.content {
            adjust.vignette = 0.0;
        }
    }
    let error = mae(&frame_at(&mut renderer, &graded_flat, t), &frame_at(&mut renderer, &each, t));
    assert!(error < 1.5, "the layer grades as the clips' own adjustments do: MAE {error}");

    // The title above keeps its red and green; the picture below is grey.
    let (w, h) = (graded.canvas.width, graded.canvas.height);
    let title_pixels: Vec<_> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let ([r, g, b], [r0, g0, b0]) = (pixel(&without, w, x, y), pixel(&untitled, w, x, y));
            r.abs_diff(r0) as u32 + g.abs_diff(g0) as u32 + b.abs_diff(b0) as u32 > 300
        })
        .collect();
    assert!(title_pixels.len() > 50, "the title shows: {} pixels", title_pixels.len());
    for &(x, y) in &title_pixels {
        let (graded, plain) = (pixel(&with, w, x, y), pixel(&without, w, x, y));
        let off = (0..3).map(|i| graded[i].abs_diff(plain[i])).max().unwrap();
        // A pixel the title covers only partly takes a little of the grey picture behind it.
        assert!(off <= 3, "the title at ({x}, {y}) is graded: {graded:?} instead of {plain:?}");
    }
    let pip_quad = &nuzky_engine::render::layer_bounds(&graded, t, &mut Default::default());
    assert!(pip_quad.iter().any(|(id, _, _)| *id == pip), "the picture in picture shows");
    assert!(
        !pip_quad.iter().any(|(id, _, _)| *id == layer),
        "the adjustment layer has no box to select in the preview"
    );

    // After the layer ends the picture is as without it.
    let later = 280_000;
    assert_eq!(frame_at(&mut renderer, &graded, later), frame_at(&mut renderer, &plain, later));

    let error = mae(&exported(&graded, &d, t), &with);
    assert!(error < 4.0, "the export differs from the preview: MAE {error}");

    // A layer that changes nothing draws nothing.
    let mut neutral = plain.clone();
    neutral.apply(EditCmd::AddAdjustment { start_us: 0, duration_us: None, adjust: None }).unwrap();
    assert_eq!(frame_at(&mut renderer, &neutral, t), without);
}

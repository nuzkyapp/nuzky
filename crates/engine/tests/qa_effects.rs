//! Plugin effects on real files: the preview, the export as FFmpeg decodes it, a saved frame and a cover show the
//! same picture, also when the preview is smaller; an effect keeps to the picture as shown on rotated phone video; a
//! missing or failing effect leaves the clip drawn without it, and a GPU that never finishes holds no frame for long.
mod qa_support;
use nuzky_engine::{
    Pending, Renderer, Wait,
    export::{ExportOptions, export},
    model::*,
    plugin_effects::{self, Lookup},
};
use qa_support::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Dark bands `spacing` canvas pixels high across the picture as shown.
const SCANLINES: &str = "fn effect(p: Pixel) -> vec4<f32> {
    let band = u32(floor(p.uv.y * p.size.y / p.params[0]));
    if band % 2u == 1u {
        return vec4<f32>(p.color.rgb * p.params[1], p.color.a);
    }
    return p.color;
}
";

/// Nuzky's data folder for this test binary, with its plugins; export and every renderer read it.
fn data() -> &'static Path {
    static DATA: OnceLock<PathBuf> = OnceLock::new();
    DATA.get_or_init(|| {
        let data = dir("effects-data");
        // Set once, before any renderer of this binary starts.
        unsafe { std::env::set_var("NUZKY_DATA_DIR", &data) };
        data
    })
}

fn install(plugin: &str, effect: &str, source: &str) {
    let folder = data().join("plugins").join(plugin);
    std::fs::create_dir_all(folder.join("effects")).unwrap();
    let manifest = serde_json::json!({
        "nuzkyPlugin": plugin_effects::API, "id": plugin, "name": "Lines", "description": "Scanlines.",
        "version": "1.0.0", "effects": [{"id": effect, "name": "Scanlines", "params": [
            {"type": "number", "id": "spacing", "label": "Spacing", "min": 1, "max": 64, "step": 1, "default": 8},
            {"type": "number", "id": "dim", "label": "Dim", "min": 0, "max": 1, "step": 0.05, "default": 0.25}
        ]}]
    });
    std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
    std::fs::write(folder.join("effects").join(format!("{effect}.wgsl")), source).unwrap();
}

fn with_effect(mut project: Project, plugin: &str) -> Project {
    project.tracks[0].clips[0].effects = vec![Effect {
        plugin: plugin.into(),
        effect: "scanlines".into(),
        name: "Scanlines".into(),
        version: "1.0.0".into(),
        params: BTreeMap::from([("spacing".into(), EffectValue::Number(8.0))]),
    }];
    project
}

/// Phone video turned a quarter by its display matrix, upright 64 × 96.
fn rotated(d: &Path) -> PathBuf {
    let base = d.join("base.mp4");
    ff(&["-f", "lavfi", "-i", "testsrc2=size=96x64:rate=25:duration=0.4", "-c:v", "libx264", "-crf", "8"], &base);
    let path = d.join("rotated.mp4");
    ff(&["-display_rotation", "90", "-i", base.to_str().unwrap(), "-c", "copy"], &path);
    path
}

#[test]
fn preview_export_frame_and_cover_show_the_same_effect() {
    if !available() {
        return;
    }
    install("lines", "scanlines", SCANLINES);
    let d = dir("effects-surfaces");
    let mut plain = project(&rotated(&d), 400_000);
    plain.canvas.fps = 25;
    assert_eq!((plain.assets[0].width, plain.assets[0].height, plain.assets[0].rotation), (64, 96, 270));
    let (w, h) = (plain.canvas.width, plain.canvas.height);
    let p = with_effect(plain.clone(), "lines");
    let t = 120_000;

    let mut frame = Renderer::new().unwrap();
    frame.use_mattes(d.join("cache"), Pending::Fail);
    let saved = frame.render(&p, t, w, h, Wait::Exact, false).unwrap();
    assert!(frame.skipped_effects().is_empty(), "{:?}", frame.skipped_effects());
    let without = frame.render(&plain, t, w, h, Wait::Exact, false).unwrap();
    let changed = mae(&saved, &without);
    assert!(changed > 20.0, "the effect changes the picture: MAE {changed}");

    // The bands run across the picture as shown, not across the file's own rows: every pixel of a dark row is dimmed
    // to a quarter, every other pixel is as it was.
    let row = (w * 4) as usize;
    for y in 0..h as usize {
        let dim = if (y / 8) % 2 == 1 { 0.25 } else { 1.0 };
        let off = (y * row..(y + 1) * row)
            .filter(|i| i % 4 != 3)
            .map(|i| (saved[i] as f32 - without[i] as f32 * dim).abs())
            .fold(0.0, f32::max);
        assert!(off <= 1.5, "row {y}: a pixel is {off} levels from {dim} of the picture");
    }

    let mut preview = Renderer::new().unwrap();
    preview.use_proxies(d.join("cache"));
    preview.use_mattes(d.join("cache"), Pending::Original);
    preview.set_gpu_timeout(Duration::from_secs(2));
    let shown = preview.render(&p, t, w, h, Wait::Exact, false).unwrap();
    assert!(mae(&shown, &saved) < 0.5, "preview and saved frame differ: MAE {}", mae(&shown, &saved));

    let mut cover = p.clone();
    cover.thumbnails = vec![serde_json::from_value(serde_json::json!({"format": "cover_9x16", "timeUs": t})).unwrap()];
    let thumbnail = cover.thumbnails[0].clone();
    let picture = frame.thumbnail_frame(&cover, &thumbnail).unwrap();
    assert!(mae(&picture.data, &saved) < 0.5, "the cover's frame differs: MAE {}", mae(&picture.data, &saved));

    let out = d.join("export.mp4");
    let options = ExportOptions { preset: "ultrafast".into(), crf: 4, replace_existing: true, ..Default::default() };
    let measured = export(&p, &d.join("cache"), &out, &options, &AtomicBool::new(false), |_| {}).unwrap();
    assert!(measured.drawn_without.is_empty(), "{:?}", measured.drawn_without);
    let decoded = raw(&out, &[]);
    let size = (w * h * 4) as usize;
    let index = (t * 25 / 1_000_000) as usize;
    let exported = &decoded[index * size..(index + 1) * size];
    let error = mae(exported, &saved);
    eprintln!("QA effects: changed {changed:.1}, export MAE {error:.2}");
    assert!(error < 4.0, "the export differs from the preview: MAE {error}");
}

#[test]
fn a_missing_plugin_draws_the_clip_without_its_effect_and_says_so() {
    if !available() {
        return;
    }
    data();
    let d = dir("effects-missing");
    let plain = project(&rotated(&d), 400_000);
    let p = with_effect(plain.clone(), "not-installed");
    let (w, h) = (plain.canvas.width, plain.canvas.height);
    let mut renderer = Renderer::new().unwrap();
    let drawn = renderer.render(&p, 0, w, h, Wait::Exact, false).unwrap();
    assert_eq!(renderer.skipped_effects(), ["Scanlines"]);
    assert_eq!(drawn, renderer.render(&plain, 0, w, h, Wait::Exact, false).unwrap());
    assert!(renderer.skipped_effects().is_empty());
    assert_eq!(renderer.effects_drawn_without(), ["Scanlines"]);
    let issues = plugin_effects::issues(&p);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].sentence(), "Scanlines is not installed on this computer, so the clip is drawn without it.");
}

/// A driver that hangs on a shader or crashes ends the device; the frame is drawn again without the effect, and no
/// renderer in the process draws that effect again.
#[test]
fn a_lost_device_or_a_timeout_marks_the_effect_broken_and_draws_without_it() {
    if !available() {
        return;
    }
    let d = dir("effects-broken");
    let plain = project(&rotated(&d), 400_000);
    let (w, h) = (plain.canvas.width, plain.canvas.height);
    for (plugin, lose) in [("lose-device", true), ("time-out", false)] {
        // Each case its own shader, so marking one broken leaves the other alone.
        install(plugin, "scanlines", &format!("{SCANLINES}// {plugin}\n"));
        let p = with_effect(plain.clone(), plugin);
        let mut renderer = Renderer::new().unwrap();
        let without = renderer.render(&plain, 0, w, h, Wait::Exact, false).unwrap();
        let drawn = renderer.render(&p, 0, w, h, Wait::Exact, false).unwrap();
        assert!(mae(&drawn, &without) > 20.0, "{plugin}: the effect draws first");
        if lose {
            renderer.lose_gpu();
        } else {
            renderer.set_gpu_timeout(Duration::ZERO);
        }
        let after = renderer.render(&p, 0, w, h, Wait::Exact, false).unwrap();
        assert_eq!(after, without, "{plugin}: the frame is drawn without the effect");
        assert_eq!(renderer.skipped_effects(), ["Scanlines"], "{plugin}");
        let Lookup::Broken(why) = plugin_effects::lookup(&data().join("plugins"), plugin, "scanlines") else {
            panic!("{plugin}: the effect is not marked broken")
        };
        assert!(why.contains(if lose { "stopped" } else { "longer than" }), "{plugin}: {why}");
        renderer.set_gpu_timeout(Duration::from_secs(10));
        assert_eq!(renderer.render(&p, 0, w, h, Wait::Exact, false).unwrap(), without, "{plugin}: the next frame");
        let mut other = Renderer::new().unwrap();
        assert_eq!(other.render(&p, 0, w, h, Wait::Exact, false).unwrap(), without, "{plugin}: another renderer");
        assert!(plugin_effects::issues(&p)[0].sentence().starts_with("Scanlines has an error"), "{plugin}");
    }
}

/// Every other canvas row dimmed to a quarter: finer than a preview at half size can show row by row.
const ROWS: &str = "fn effect(p: Pixel) -> vec4<f32> {
    if u32(floor(p.uv.y * p.size.y)) % 2u == 1u {
        return vec4<f32>(p.color.rgb * 0.25, p.color.a);
    }
    return p.color;
}
";

/// RGBA `w`×`h` shrunk to half by averaging each 2×2 block.
fn half(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h);
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            for c in 0..4 {
                let at = |x: usize, y: usize| rgba[(y * w + x) * 4 + c] as u32;
                out.push(((at(x, y) + at(x + 1, y) + at(x, y + 1) + at(x + 1, y + 1) + 2) / 4) as u8);
            }
        }
    }
    out
}

/// The effect draws on the clip's own grid in a preview at half size too, and the preview shrinks its picture, so
/// stripes one canvas pixel high come out as the export's stripes seen from further away, not as one dark field.
#[test]
fn a_smaller_preview_shows_what_the_export_shows() {
    if !available() {
        return;
    }
    install("rows", "scanlines", ROWS);
    let d = dir("effects-smaller");
    let grey = d.join("grey.mp4");
    ff(
        &["-f", "lavfi", "-i", "color=c=0x909090:size=64x64:rate=25:duration=0.4", "-c:v", "libx264", "-crf", "0"],
        &grey,
    );
    let mut p = with_effect(project(&grey, 400_000), "rows");
    p.canvas.fps = 25;
    let t = 120_000;

    let out = d.join("export.mp4");
    let options = ExportOptions { preset: "ultrafast".into(), crf: 0, replace_existing: true, ..Default::default() };
    export(&p, &d.join("cache"), &out, &options, &AtomicBool::new(false), |_| {}).unwrap();
    let size = 64 * 64 * 4;
    let index = (t * 25 / 1_000_000) as usize;
    let exported = raw(&out, &[])[index * size..(index + 1) * size].to_vec();
    let row = |y: usize| exported[y * 64 * 4] as i32;
    assert!(row(0) - row(1) > 60, "the export has the stripes: rows {} and {}", row(0), row(1));

    let mut preview = Renderer::new().unwrap();
    preview.use_proxies(d.join("cache"));
    preview.use_mattes(d.join("cache"), Pending::Original);
    preview.set_gpu_timeout(Duration::from_secs(2));
    let shown = preview.render(&p, t, 32, 32, Wait::Exact, false).unwrap();
    let error = mae(&shown, &half(&exported, 64, 64));
    eprintln!("QA effects: half-size preview MAE {error:.2}");
    assert!(error < 4.0, "the half-size preview differs from the export seen at that size: MAE {error}");
}

/// A frame that times out with an effect is drawn again without it, and that try is bounded too, so a GPU that
/// never finishes either try ends the frame with an error within seconds instead of holding the preview or export.
#[test]
fn a_gpu_that_never_finishes_ends_the_frame_within_seconds() {
    if !available() {
        return;
    }
    install("never-ends", "scanlines", &format!("{SCANLINES}// never ends\n"));
    let d = dir("effects-hung");
    let p = with_effect(project(&rotated(&d), 400_000), "never-ends");
    let (tx, rx) = std::sync::mpsc::channel();
    let started = Instant::now();
    let shown = p.clone();
    std::thread::spawn(move || {
        let mut renderer = Renderer::new().unwrap();
        renderer.set_gpu_timeout(Duration::from_millis(200));
        renderer.hang_gpu(2);
        let (w, h) = (shown.canvas.width, shown.canvas.height);
        tx.send(renderer.render(&shown, 0, w, h, Wait::Exact, false).map(drop).map_err(|e| format!("{e:#}"))).ok();
    });
    let result = rx.recv_timeout(Duration::from_secs(30)).expect("the frame never ended");
    let error = result.expect_err("a GPU that never finishes draws no frame");
    assert!(error.contains("longer than"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(30), "{:?}", started.elapsed());
    assert!(matches!(plugin_effects::lookup(&data().join("plugins"), "never-ends", "scanlines"), Lookup::Broken(_)));
}

/// An export's cancel ends the wait for a GPU that does not finish, and does not count against the effect.
#[test]
fn cancel_ends_the_wait_for_a_stuck_gpu() {
    if !available() {
        return;
    }
    install("cancelled", "scanlines", &format!("{SCANLINES}// cancelled\n"));
    let d = dir("effects-cancel");
    let p = with_effect(project(&rotated(&d), 400_000), "cancelled");
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();
    let (shown, stopped) = (p.clone(), stop.clone());
    let drawing = std::thread::spawn(move || {
        let mut renderer = Renderer::new().unwrap();
        renderer.set_gpu_timeout(Duration::from_secs(60));
        renderer.hang_gpu(1);
        let (w, h) = (shown.canvas.width, shown.canvas.height);
        tx.send(Instant::now()).ok();
        let result = renderer.render_until(&shown, 0, w, h, Wait::Exact, true, &stopped);
        tx.send(Instant::now()).ok();
        result.map(drop).map_err(|e| format!("{e:#}")).expect_err("a stopped frame draws nothing");
        renderer.render(&shown, 0, w, h, Wait::Exact, true).expect("the next frame draws on a new device");
    });
    rx.recv_timeout(Duration::from_secs(30)).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let cancelled = Instant::now();
    stop.store(true, Ordering::Relaxed);
    let ended = rx.recv_timeout(Duration::from_secs(30)).expect("cancel did not end the frame");
    assert!(ended.duration_since(cancelled) < Duration::from_secs(3), "{:?}", ended.duration_since(cancelled));
    assert!(matches!(plugin_effects::lookup(&data().join("plugins"), "cancelled", "scanlines"), Lookup::Ready(_)));
    drawing.join().unwrap();
}

/// Posters and covers kept for later go stale when a plugin effect of the project changes, breaks or goes away.
#[test]
fn the_effects_stamp_changes_with_the_plugin() {
    install("stamped", "scanlines", SCANLINES);
    let mut p = Project::new("Stamp");
    p.tracks[0].clips.push(clip("still", "a", 0, 1_000_000));
    let p = with_effect(p, "stamped");
    let installed = plugin_effects::stamp(&p);
    assert!(!installed.is_empty() && !installed.contains("missing"), "{installed}");
    let file = data().join("plugins/stamped/effects/scanlines.wgsl");
    std::fs::write(&file, format!("{SCANLINES}// changed\n")).unwrap();
    let changed = plugin_effects::stamp(&p);
    assert_ne!(changed, installed);
    std::fs::write(&file, "fn effect(p: Pixel) -> vec4<f32> { loop { } }\n").unwrap();
    let broken = plugin_effects::stamp(&p);
    assert!(broken.contains("broken"), "{broken}");
    std::fs::remove_dir_all(data().join("plugins/stamped")).unwrap();
    assert!(plugin_effects::stamp(&p).contains("missing"));
    assert_eq!(plugin_effects::stamp(&Project::new("None")), "");
}

/// Two-pixel columns a quarter as bright, one pixel off the grid: a reduction that skips texels lands on the dark ones.
const COLUMNS: &str = "fn effect(p: Pixel) -> vec4<f32> {
    if ((u32(floor(p.uv.x * p.size.x)) + 1u) / 2u) % 2u == 1u {
        return vec4<f32>(p.color.rgb * 0.25, p.color.a);
    }
    return p.color;
}
";

/// A 4K clip shown at an eighth draws its effect on its own 4K grid, so a preview at an eighth of the canvas shrinks
/// that grid 64 times. Every texel counts there as it does in the export's eightfold reduction, so fine columns blend
/// to the same grey in both instead of the preview landing on the dark ones.
#[test]
fn a_much_smaller_preview_blends_what_the_export_blends() {
    if !available() {
        return;
    }
    install("columns", "scanlines", COLUMNS);
    let d = dir("effects-much-smaller");
    let grey = d.join("grey.mp4");
    ff(
        &["-f", "lavfi", "-i", "color=c=0x909090:size=3840x2160:rate=25:duration=0.12", "-c:v", "libx264", "-crf", "0"],
        &grey,
    );
    let mut p = with_effect(project(&grey, 120_000), "columns");
    p.canvas.fps = 25;
    let ClipContent::Media { transform, .. } = &mut p.tracks[0].clips[0].content else { unreachable!() };
    transform.scale = 0.125;
    let t = 40_000;
    // The middle of the clip, which covers canvas pixels 1680..2160 × 945..1215.
    let mean = |rgba: &[u8], w: usize, k: usize| {
        let (xs, ys) = (1720 / k..2120 / k, 984 / k..1176 / k);
        let n = xs.len() * ys.len();
        ys.flat_map(|y| xs.clone().map(move |x| (y * w + x) * 4)).map(|i| rgba[i] as f64).sum::<f64>() / n as f64
    };

    let out = d.join("export.mp4");
    let options = ExportOptions { preset: "ultrafast".into(), crf: 0, replace_existing: true, ..Default::default() };
    export(&p, &d.join("cache"), &out, &options, &AtomicBool::new(false), |_| {}).unwrap();
    let size = 3840 * 2160 * 4;
    let index = (t * 25 / 1_000_000) as usize;
    let exported = mean(&raw(&out, &[])[index * size..(index + 1) * size], 3840, 1);

    let mut preview = Renderer::new().unwrap();
    preview.use_proxies(d.join("cache"));
    preview.use_mattes(d.join("cache"), Pending::Original);
    preview.set_gpu_timeout(Duration::from_secs(2));
    let shown = mean(&preview.render(&p, t, 480, 270, Wait::Exact, false).unwrap(), 480, 8);
    eprintln!("QA effects: an eighth of a 4K clip, export {exported:.1}, preview at an eighth {shown:.1}");
    let blended = 144.0 * 0.625;
    assert!((exported - blended).abs() < 3.0, "the export blends the columns: {exported}");
    assert!((shown - exported).abs() < 3.0, "the preview shows {shown} where the export shows {exported}");
}

const ROW_TRIPLES: &str = "fn effect(p: Pixel) -> vec4<f32> {
    if (u32(floor(p.uv.y * p.size.y)) / 3u) % 2u == 1u {
        return vec4<f32>(p.color.rgb * 0.25, p.color.a);
    }
    return p.color;
}
";

/// A clip at half the canvas, its effect drawn on its full-size grid in threes of bright and dark rows, shrinks ten
/// times for a fifth-size preview. Each preview row then covers one and two thirds of a bright and dark six, so the rows
/// come out lighter and darker; the steps of the shrink keep that, as the export reduced to the preview's size shows.
#[test]
fn a_preview_reduction_in_steps_keeps_what_one_reduction_shows() {
    if !available() {
        return;
    }
    install("row-triples", "scanlines", ROW_TRIPLES);
    let d = dir("effects-steps");
    let grey = d.join("grey.mp4");
    ff(
        &["-f", "lavfi", "-i", "color=c=0x909090:size=1920x1080:rate=25:duration=0.12", "-c:v", "libx264", "-crf", "0"],
        &grey,
    );
    let mut p = with_effect(project(&grey, 120_000), "row-triples");
    p.canvas.fps = 25;
    let ClipContent::Media { transform, .. } = &mut p.tracks[0].clips[0].content else { unreachable!() };
    transform.scale = 0.5;
    let t = 40_000;
    let out = d.join("export.mp4");
    let options = ExportOptions { preset: "ultrafast".into(), crf: 0, replace_existing: true, ..Default::default() };
    export(&p, &d.join("cache"), &out, &options, &AtomicBool::new(false), |_| {}).unwrap();
    let size = 1920 * 1080 * 4;
    let index = (t * 25 / 1_000_000) as usize;
    let frame = &raw(&out, &[])[index * size..(index + 1) * size];
    // The export's red channel reduced five times by area, as the preview's 384 × 216.
    let reduced = |x: usize, y: usize| {
        let pixels = (0..5).flat_map(|dy| (0..5).map(move |dx| ((y * 5 + dy) * 1920 + x * 5 + dx) * 4));
        pixels.map(|i| frame[i] as f64).sum::<f64>() / 25.0
    };
    let mut preview = Renderer::new().unwrap();
    preview.use_proxies(d.join("cache"));
    preview.use_mattes(d.join("cache"), Pending::Original);
    preview.set_gpu_timeout(Duration::from_secs(2));
    let shown = preview.render(&p, t, 384, 216, Wait::Exact, false).unwrap();
    // Inside the clip, which covers preview pixels 96..288 × 54..162.
    let (xs, ys) = (110..270, 60..156);
    let n = (xs.len() * ys.len()) as f64;
    let error = ys
        .flat_map(|y| xs.clone().map(move |x| (x, y)))
        .map(|(x, y)| (shown[(y * 384 + x) * 4] as f64 - reduced(x, y)).abs())
        .sum::<f64>()
        / n;
    let rows: Vec<f64> = (60..66).map(|y| reduced(150, y)).collect();
    eprintln!("QA effects: rows reduced from the export {rows:.1?}, preview off by {error:.2} on average");
    assert!(
        rows.iter().any(|v| *v > 95.0) && rows.iter().any(|v| *v < 85.0),
        "the export keeps lighter and darker rows: {rows:?}"
    );
    assert!(error < 3.0, "the preview is {error:.2} levels off the export reduced to its size");
}

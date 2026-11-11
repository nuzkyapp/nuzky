//! Nuzky's own effects, transitions and keyframe curves on real files: each draws with no plugin installed, changes
//! the picture, and the export as FFmpeg decodes it shows what the preview does.
mod qa_support;
use nuzky_engine::{
    Renderer, Wait,
    export::{ExportOptions, export},
    media::probe,
    model::*,
    plugin_effects,
};
use qa_support::*;
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};

const FPS: u32 = 25;

/// An empty data folder, so no plugin of anyone's can draw what the test expects of Nuzky's own effects.
fn no_plugins() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe { std::env::set_var("NUZKY_DATA_DIR", dir("looks-data")) });
}

/// One second of an FFmpeg test pattern, 96 × 64 unless `pattern` names its size.
fn source(d: &Path, name: &str, pattern: &str) -> std::path::PathBuf {
    let path = d.join(format!("{name}.mp4"));
    let size = if pattern.contains("size=") { "" } else { "=size=96x64" };
    let input = format!("{pattern}{size}:rate={FPS}:duration=1");
    ff(&["-f", "lavfi", "-i", &input, "-c:v", "libx264", "-crf", "8", "-pix_fmt", "yuv420p"], &path);
    path
}

fn built_in(effect: &str) -> Effect {
    let def = plugin_effects::built_in().find(|def| def.id == effect).unwrap();
    Effect {
        plugin: plugin_effects::BUILT_IN.into(),
        effect: effect.into(),
        name: def.name.clone(),
        version: "1".into(),
        params: BTreeMap::new(),
    }
}

/// The export's frames at `times`, each beside the preview's frame there.
fn exported_and_previewed(p: &Project, d: &Path, times: &[i64]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let out = d.join("export.mp4");
    let options = ExportOptions { preset: "ultrafast".into(), crf: 4, replace_existing: true, ..Default::default() };
    let measured = export(p, &d.join("cache"), &out, &options, &AtomicBool::new(false), |_| {}).unwrap();
    assert!(measured.drawn_without.is_empty(), "{:?}", measured.drawn_without);
    let decoded = raw(&out, &[]);
    let (w, h) = (p.canvas.width, p.canvas.height);
    let size = (w * h * 4) as usize;
    let mut preview = Renderer::new().unwrap();
    times
        .iter()
        .map(|&t| {
            let index = (t * FPS as i64 / 1_000_000) as usize;
            let shown = preview.render(p, t, w, h, Wait::Exact, false).unwrap();

            assert!(preview.skipped_effects().is_empty(), "{:?}", preview.skipped_effects());
            (decoded[index * size..(index + 1) * size].to_vec(), shown)
        })
        .collect()
}

/// RGBA `w` × `h` shrunk to half by averaging each 2 × 2 block.
fn half(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let at = |x: usize, y: usize, c: usize| rgba[(y * w + x) * 4 + c] as u32;
    (0..h / 2)
        .flat_map(|y| (0..w / 2).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            (0..4).map(move |c| {
                ((at(2 * x, 2 * y, c)
                    + at(2 * x + 1, 2 * y, c)
                    + at(2 * x, 2 * y + 1, c)
                    + at(2 * x + 1, 2 * y + 1, c)
                    + 2)
                    / 4) as u8
            })
        })
        .collect()
}

#[test]
fn nuzkys_own_effects_draw_without_plugins_and_export_as_previewed() {
    if !available() {
        return;
    }
    no_plugins();
    let d = dir("looks-effects");
    let mut plain = project(&source(&d, "a", "testsrc2"), 1_000_000);
    plain.canvas.fps = FPS;
    let (w, h) = (plain.canvas.width, plain.canvas.height);
    let mut renderer = Renderer::new().unwrap();
    let t = 360_000;
    let without = renderer.render(&plain, t, w, h, Wait::Exact, false).unwrap();
    for def in plugin_effects::built_in() {
        let mut p = plain.clone();
        p.tracks[0].clips[0].effects = vec![built_in(&def.id)];
        let drawn = renderer.render(&p, t, w, h, Wait::Exact, false).unwrap();
        assert!(renderer.skipped_effects().is_empty(), "{}: {:?}", def.id, renderer.skipped_effects());
        let changed = mae(&drawn, &without);
        eprintln!("QA looks: {} changes the picture by MAE {changed:.1}", def.id);
        assert!(changed > 1.0, "{} changes the picture: MAE {changed}", def.id);
        assert!(plugin_effects::issues(&p).is_empty(), "{}", def.id);
    }

    // The ones that move with time, together, exported.
    let mut p = plain.clone();
    p.tracks[0].clips[0].effects = ["shake", "glitch", "grain", "blur"].map(built_in).to_vec();
    p.tracks[0].clips[0].effects[0].params.insert("amount".into(), EffectValue::Number(1.0));
    for (exported, shown) in exported_and_previewed(&p, &d, &[200_000, 600_000]) {
        let error = mae(&exported, &shown);
        eprintln!("QA looks: effects export MAE {error:.2}");
        assert!(error < 4.0, "the export differs from the preview: MAE {error}");
    }
}

#[test]
fn new_transitions_and_keyframe_curves_export_as_previewed() {
    if !available() {
        return;
    }
    no_plugins();
    let d = dir("looks-motion");
    let mut p = project(&source(&d, "a", "testsrc2"), 1_000_000);
    p.canvas.fps = FPS;
    let b = probe(&source(&d, "b", "smptebars"), "b".into()).unwrap();
    let mut second = clip("second", &b.id, 1_000_000, 1_000_000);
    p.assets.push(b);
    let small = Transform { scale: 0.4, ..Transform::default() };
    second.keyframes = vec![
        Keyframe { t_us: 200_000, transform: small, ease: Ease::EaseIn },
        Keyframe { t_us: 800_000, transform: Transform { x: 0.3, ..Transform::default() }, ease: Ease::Linear },
    ];
    second.transition_in = Some(Transition { kind: TransitionKind::Spin, duration_us: 400_000 });
    p.tracks[0].clips.push(second);
    let (w, h) = (p.canvas.width, p.canvas.height);
    let (spin_at, curve_at) = (1_080_000, 1_520_000);

    // Each kind draws its own picture midway, and the curve moves the clip elsewhere than an even pace.
    let mut renderer = Renderer::new().unwrap();
    let mut frame = |p: &Project, t| renderer.render(p, t, w, h, Wait::Exact, false).unwrap();
    let spin = frame(&p, spin_at);
    let mut kinds = Vec::new();
    for kind in
        [TransitionKind::Dissolve, TransitionKind::SlideRight, TransitionKind::WipeDown, TransitionKind::ZoomOut]
    {
        let mut other = p.clone();
        other.tracks[0].clips[1].transition_in = Some(Transition { kind, duration_us: 400_000 });
        kinds.push(frame(&other, spin_at));
    }
    kinds.push(spin.clone());
    for (i, a) in kinds.iter().enumerate() {
        for b in &kinds[i + 1..] {
            assert!(mae(a, b) > 2.0, "two transitions draw alike midway: MAE {}", mae(a, b));
        }
    }
    // A preview half the size shows what the export does, only smaller, on wide flat bars where shrinking adds little.
    let mut bars = Project::new("bars");
    let a = probe(&source(&d, "bars", "smptebars=size=384x256"), "bars".into()).unwrap();
    (bars.canvas.width, bars.canvas.height, bars.canvas.fps) = (384, 256, FPS);
    bars.tracks[0].clips = vec![clip("a", &a.id, 0, 1_000_000), clip("b", &a.id, 1_000_000, 1_000_000)];
    bars.assets.push(a);
    let mut full = Renderer::new().unwrap();
    let mut small = Renderer::new().unwrap();
    for kind in [TransitionKind::Spin, TransitionKind::ZoomOut, TransitionKind::SlideDown, TransitionKind::WipeUp] {
        bars.tracks[0].clips[1].transition_in = Some(Transition { kind, duration_us: 400_000 });
        let whole = full.render(&bars, spin_at, 384, 256, Wait::Exact, false).unwrap();
        let shrunk = small.render(&bars, spin_at, 192, 128, Wait::Exact, false).unwrap();
        let error = mae(&shrunk, &half(&whole, 384, 256));
        eprintln!("QA looks: {kind:?} at half size MAE {error:.2}");
        assert!(error < 2.0, "{kind:?} draws otherwise in a smaller preview: MAE {error}");
    }
    let eased = frame(&p, curve_at);
    let mut even = p.clone();
    even.tracks[0].clips[1].keyframes[0].ease = Ease::Linear;
    let differs = mae(&eased, &frame(&even, curve_at));
    assert!(differs > 2.0, "ease in moves the clip elsewhere than linear: MAE {differs}");

    for (exported, shown) in exported_and_previewed(&p, &d, &[spin_at, curve_at]) {
        let error = mae(&exported, &shown);
        eprintln!("QA looks: transition and curve export MAE {error:.2}");
        assert!(error < 4.0, "the export differs from the preview: MAE {error}");
    }
}

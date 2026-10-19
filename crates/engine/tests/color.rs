use nuzky_engine::model::{Adjust, ClipContent};
use nuzky_engine::{Project, Renderer, Wait};

const RAMP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/color-ramp.ppm");

/// One still image filling a canvas of its own size, drawn as CLI frames draw it.
fn render(renderer: &mut Renderer, path: &str, (width, height): (u32, u32), adjust: Adjust) -> Vec<u8> {
    let project: Project = serde_json::from_value(serde_json::json!({
        "version": 1, "name": "Color",
        "canvas": { "width": width, "height": height, "fps": 30, "background": "#000000" },
        "assets": [{
            "id": "still", "name": "Still", "path": path,
            "kind": "image", "durationUs": 0, "width": width, "height": height,
            "fps": 0, "hasAudio": false
        }],
        "tracks": [{ "id": "main", "kind": "video", "clips": [{
            "id": "still", "startUs": 0, "durationUs": 1_000_000,
            "content": {
                "type": "media", "assetId": "still", "sourceInUs": 0,
                "speed": 1, "volume": 1,
                "transform": { "x": 0, "y": 0, "scale": 1, "rotation": 0, "opacity": 1 },
                "adjust": adjust
            }
        }] }]
    }))
    .unwrap();
    let ClipContent::Media { adjust: loaded, .. } = &project.tracks[0].clips[0].content else { panic!() };
    assert_eq!(*loaded, adjust);
    renderer.render(&project, 0, width, height, Wait::Exact, false).unwrap()
}

/// 256 x 16: greys from black to white in the upper half, colours in the lower half.
fn ramp(renderer: &mut Renderer, adjust: Adjust) -> Vec<u8> {
    render(renderer, RAMP, (256, 16), adjust)
}

fn rgb(frame: &[u8], x: usize) -> [i32; 3] {
    let p = &frame[(4 * 256 + x) * 4..];
    [p[0], p[1], p[2]].map(i32::from)
}

fn grey(frame: &[u8], x: usize) -> i32 {
    rgb(frame, x)[0]
}

fn smooth_grey(frame: &[u8]) {
    for x in 0..255 {
        let step = grey(frame, x + 1) - grey(frame, x);
        assert!((0..=3).contains(&step), "non-monotonic or abrupt ramp at {x}: {step}");
    }
    assert!(frame.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
}

fn each_control(amount: f32) -> [(&'static str, Adjust); 9] {
    let d = Adjust::default();
    [
        ("exposure", Adjust { exposure: amount, ..d }),
        ("brightness", Adjust { brightness: amount, ..d }),
        ("contrast", Adjust { contrast: amount, ..d }),
        ("highlights", Adjust { highlights: amount, ..d }),
        ("shadows", Adjust { shadows: amount, ..d }),
        ("saturation", Adjust { saturation: amount, ..d }),
        ("temperature", Adjust { temperature: amount, ..d }),
        ("tint", Adjust { tint: amount, ..d }),
        ("fade", Adjust { fade: amount.abs(), ..d }),
    ]
}

// Golden RGBA captured with the unmodified a18d535 CLI: frame ramp.json 0 ramp-head.png 256.
// 256x16 image layer, default transform/Adjust, RADV NAVI21; upper half grey, lower half RGB.
#[test]
fn zero_adjust_matches_head_a18d535_frame() {
    let mut renderer = Renderer::new().unwrap();
    let frame = ramp(&mut renderer, Adjust::default());
    assert_eq!(frame.as_slice(), include_bytes!("fixtures/color-ramp-head-a18d535.rgba"));
    let old: Adjust =
        serde_json::from_str(r#"{"brightness":0,"contrast":0,"saturation":0,"temperature":0,"vignette":0}"#).unwrap();
    assert_eq!(old, Adjust::default());
    assert_eq!(ramp(&mut renderer, old), frame);
}

#[test]
fn exposure_uses_linear_stops_and_preserves_black() {
    let mut renderer = Renderer::new().unwrap();
    let base = ramp(&mut renderer, Adjust::default());
    for amount in [-1.0, 1.0] {
        let frame = ramp(&mut renderer, Adjust { exposure: amount, ..Adjust::default() });
        let mean = |f: &[u8]| (0..256).map(|x| grey(f, x)).sum::<i32>();
        assert_eq!(grey(&frame, 0), 0);
        assert_eq!(mean(&frame) > mean(&base), amount > 0.0);
        // 64 is 0.251 in the 2.4 power encoding; +/-2 stops of light scale it by 4^(+/-1/2.4) to 0.447 / 0.141.
        let expected = if amount > 0.0 { 114 } else { 36 };
        assert!((grey(&frame, 64) - expected).abs() <= 2, "{amount}: {}", grey(&frame, 64));
    }
}

#[test]
fn tint_moves_green_against_red_and_blue() {
    let mut renderer = Renderer::new().unwrap();
    for amount in [-1.0, 1.0] {
        let frame = ramp(&mut renderer, Adjust { tint: amount, ..Adjust::default() });
        let p = &frame[(4 * 256 + 128) * 4..][..4];
        assert_eq!(p[0] > p[1] && p[2] > p[1], amount > 0.0);
        assert_eq!(p[0] < p[1] && p[2] < p[1], amount < 0.0);
    }
}

#[test]
fn highlights_target_brights_and_preserve_midgrey() {
    let mut renderer = Renderer::new().unwrap();
    let base = ramp(&mut renderer, Adjust::default());
    for amount in [-1.0, 1.0] {
        let frame = ramp(&mut renderer, Adjust { highlights: amount, ..Adjust::default() });
        let delta = grey(&frame, 230) - grey(&base, 230);
        assert_eq!(delta > 0, amount > 0.0);
        assert!(delta.abs() > 4 * (grey(&frame, 128) - grey(&base, 128)).abs() + 4);
        assert!((grey(&frame, 128) - grey(&base, 128)).abs() <= 1);
        assert_eq!(grey(&frame, 32), grey(&base, 32));
        smooth_grey(&frame);
    }
}

#[test]
fn shadows_target_darks_and_preserve_midgrey() {
    let mut renderer = Renderer::new().unwrap();
    let base = ramp(&mut renderer, Adjust::default());
    for amount in [-1.0, 1.0] {
        let frame = ramp(&mut renderer, Adjust { shadows: amount, ..Adjust::default() });
        let delta = grey(&frame, 25) - grey(&base, 25);
        assert_eq!(delta > 0, amount > 0.0);
        assert!(delta.abs() > 4 * (grey(&frame, 128) - grey(&base, 128)).abs() + 4);
        assert!((grey(&frame, 128) - grey(&base, 128)).abs() <= 1);
        assert_eq!(grey(&frame, 230), grey(&base, 230));
        smooth_grey(&frame);
    }
}

#[test]
fn fade_lifts_black_and_gently_compresses_white() {
    let mut renderer = Renderer::new().unwrap();
    let base = ramp(&mut renderer, Adjust::default());
    for amount in [0.5, 1.0] {
        let frame = ramp(&mut renderer, Adjust { fade: amount, ..Adjust::default() });
        assert!(grey(&frame, 0) > grey(&base, 0));
        assert!((grey(&frame, 0) as f32 - 255.0 * 0.25 * amount).abs() <= 1.0);
        assert!(grey(&frame, 255) < 255 && grey(&frame, 255) >= 242);
        smooth_grey(&frame);
    }
}

/// Brightness, shadows and white balance once added a constant or mixed towards one: +50 brightness drew black
/// as 51 grey, -50 drew white as 204, and warming drew black as brown.
#[test]
fn light_and_colour_controls_keep_black_and_white() {
    let mut renderer = Renderer::new().unwrap();
    let d = Adjust::default();
    for amount in [-0.5, 0.5] {
        for (name, adjust) in [
            ("temperature", Adjust { temperature: amount, ..d }),
            ("tint", Adjust { tint: amount, ..d }),
            ("brightness", Adjust { brightness: amount, ..d }),
            ("shadows", Adjust { shadows: amount, ..d }),
        ] {
            let frame = ramp(&mut renderer, adjust);
            assert_eq!(rgb(&frame, 0), [0; 3], "{name} {amount}: black");
            // A white balance leaves white faintly tinted, but as light as before.
            let [r, g, b] = rgb(&frame, 255).map(|v| v as f32);
            let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            assert!(luma >= 250.0, "{name} {amount}: white {:?}", rgb(&frame, 255));
        }
    }
}

/// Hue in degrees of a colour whose red is highest and blue lowest, such as skin.
fn warm_hue([r, g, b]: [i32; 3]) -> f32 {
    assert!(r >= g && g >= b, "not a warm colour: {:?}", [r, g, b]);
    60.0 * (g - b) as f32 / (r - b) as f32
}

/// Contrast and saturation once pushed each channel of bright skin apart and clipped red alone, which turned it
/// yellow: contrast +50 moved its hue by 8 degrees.
#[test]
fn contrast_and_saturation_keep_the_hue_of_bright_skin() {
    const SKIN: [u8; 3] = [242, 184, 148];
    let path = format!("{}/bright-skin.ppm", env!("CARGO_TARGET_TMPDIR"));
    let mut ppm = b"P6\n8 8\n255\n".to_vec();
    ppm.extend(SKIN.repeat(64));
    std::fs::write(&path, ppm).unwrap();
    let mut renderer = Renderer::new().unwrap();
    let pixel = |frame: &[u8]| [frame[0], frame[1], frame[2]].map(i32::from);
    let skin = SKIN.map(i32::from);
    for adjust in [Adjust { contrast: 0.5, ..Adjust::default() }, Adjust { saturation: 0.5, ..Adjust::default() }] {
        let shown = pixel(&render(&mut renderer, &path, (8, 8), adjust));
        assert!((0..3).any(|c| (shown[c] - skin[c]).abs() >= 5), "{adjust:?} left the skin as it was: {shown:?}");
        let moved = (warm_hue(shown) - warm_hue(skin)).abs();
        assert!(moved <= 1.0, "{adjust:?}: {shown:?} moved the hue by {moved:.1} degrees");
    }
}

/// At full strength every control still draws darker greys darker and keeps neighbouring greys apart. Contrast and
/// brightness once clipped: contrast +100 drew a quarter of the greys as black, so dark hair and clothes went flat.
/// Exposure is left out of the second part, as more light than white holds is white; vignette darkens by place.
#[test]
fn every_control_keeps_a_grey_ramp_in_order() {
    let mut renderer = Renderer::new().unwrap();
    for amount in [-1.0, 1.0] {
        for (name, adjust) in each_control(amount) {
            let frame = ramp(&mut renderer, adjust);
            let mut flat = 1;
            for x in 1..256 {
                let (was, now) = (rgb(&frame, x - 1), rgb(&frame, x));
                assert!((0..3).all(|c| now[c] >= was[c]), "{name} {amount}: {was:?} then {now:?} at {x}");
                flat = if now == was { flat + 1 } else { 1 };
                assert!(name == "exposure" || flat <= 8, "{name} {amount}: {flat} greys up to {x} all draw as {now:?}");
            }
        }
    }
}

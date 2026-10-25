//! A project whose clip has a plugin effect, through `nuzky frame` and `nuzky render`: on a computer without the
//! plugin it still saves frames and exports, says what it drew without, and looks as the project without the effect.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use nuzky_engine::{
    Project,
    edit::{EditCmd, new_id},
    media::probe,
    model::{Effect, EffectValue},
};

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

/// The CLI with Nuzky's data, cache and home of its own under `home`.
fn nuzky(home: &Path, args: &[&Path]) -> Output {
    let mut cli = Command::new(env!("CARGO_BIN_EXE_nuzky"));
    cli.env("HOME", home)
        .env("NUZKY_DATA_DIR", home.join("data"))
        .env("XDG_DATA_HOME", home.join("xdg-data"))
        .env("XDG_CACHE_HOME", home.join("xdg-cache"))
        .env("XDG_RUNTIME_DIR", home.join("run"));
    let out = cli.args(args).output().unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    out
}

fn decoded(path: &Path) -> Vec<u8> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

fn mae(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(&x, &y)| (x as f64 - y as f64).abs()).sum::<f64>() / a.len() as f64
}

fn save(project: &Project, path: &Path) -> PathBuf {
    std::fs::write(path, serde_json::to_vec(project).unwrap()).unwrap();
    path.to_path_buf()
}

#[test]
fn a_project_whose_plugin_is_missing_saves_frames_and_exports_without_the_effect() {
    if !ffmpeg_available() {
        eprintln!("SKIP: ffmpeg CLI is unavailable");
        return;
    }
    let dir = std::env::temp_dir().join(format!("cli-effects-{}", new_id()));
    let home = dir.join("home");
    std::fs::create_dir_all(home.join("run")).unwrap();
    let video = dir.join("take.mp4");
    let made = Command::new("ffmpeg")
        .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=64x96:rate=25:duration=0.4", "-c:v", "libx264"])
        .arg(&video)
        .output()
        .unwrap();
    assert!(made.status.success());
    let mut plain = Project::new("effects");
    plain.canvas.width = 64;
    plain.canvas.height = 96;
    plain.canvas.fps = 25;
    let asset = probe(&video, "take".into()).unwrap();
    plain.apply(EditCmd::AddAssets { assets: vec![asset] }).unwrap();
    plain.apply(EditCmd::AddClip { asset_id: "take".into(), start_us: None, track_id: None }).unwrap();
    let mut with = plain.clone();
    let clip_id = with.tracks[0].clips[0].id.clone();
    let effects = vec![Effect {
        plugin: "lines".into(),
        effect: "scanlines".into(),
        name: "Scanlines".into(),
        version: "1.0.0".into(),
        params: [("dim".to_owned(), EffectValue::Number(0.25))].into(),
    }];
    with.apply(EditCmd::SetEffects { clip_id, effects }).unwrap();
    let (plain, with) = (save(&plain, &dir.join("plain.nuzky")), save(&with, &dir.join("with.nuzky")));

    let time = Path::new("0.2");
    let frame = |project: &Path, name: &str| {
        let out = dir.join(name);
        let run = nuzky(&home, &[Path::new("frame"), project, time, &out]);
        (decoded(&out), String::from_utf8_lossy(&run.stderr).into_owned())
    };
    let (expected, quiet) = frame(&plain, "plain.png");
    assert!(!quiet.contains("Warning"), "{quiet}");
    let (missing, said) = frame(&with, "missing.png");
    assert!(
        said.contains("Warning: Scanlines is not installed on this computer, so the clip is drawn without it."),
        "{said}"
    );
    assert_eq!(missing, expected, "the frame without the plugin is the frame without the effect");

    let export = |project: &Path, name: &str| {
        let out = dir.join(name);
        let run = nuzky(&home, &[Path::new("render"), project, &out]);
        (decoded(&out), String::from_utf8_lossy(&run.stderr).into_owned())
    };
    let (exported_plain, _) = export(&plain, "plain.mp4");
    let (exported, said) = export(&with, "missing.mp4");
    assert!(said.contains("Warning: Exported without Scanlines: it isn't installed on this computer."), "{said}");
    let error = mae(&exported, &exported_plain);
    assert!(error < 0.5, "the export without the plugin is the export without the effect: MAE {error}");

    // Installed, the same project draws the effect and says nothing.
    let folder = home.join("data/plugins/lines");
    std::fs::create_dir_all(folder.join("effects")).unwrap();
    let manifest = serde_json::json!({"nuzkyPlugin": nuzky_engine::plugin_effects::API, "id": "lines", "name": "Lines",
        "description": "Scanlines.", "version": "1.0.0", "effects": [{"id": "scanlines", "name": "Scanlines", "params": [
            {"type": "number", "id": "dim", "label": "Dim", "min": 0, "max": 1, "step": 0.05, "default": 0.5}]}]});
    std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
    std::fs::write(
        folder.join("effects/scanlines.wgsl"),
        "fn effect(p: Pixel) -> vec4<f32> {\n    let band = u32(floor(p.uv.y * p.size.y / 8.0)) % 2u;\n    return vec4<f32>(p.color.rgb * select(1.0, p.params[0], band == 1u), p.color.a);\n}\n",
    )
    .unwrap();
    let (drawn, said) = frame(&with, "drawn.png");
    assert!(!said.contains("Warning"), "{said}");
    let changed = mae(&drawn, &expected);
    assert!(changed > 10.0, "the installed effect changes the frame: MAE {changed}");
    std::fs::remove_dir_all(dir).unwrap();
}

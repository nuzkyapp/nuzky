//! Following a boxed thing through a video made by FFmpeg: a 160 px test pattern, changing every frame, crossing a
//! textured wide picture. Needs only the `ffmpeg` CLI and the renderer, as the engine's QA tests.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use nuzky_engine::{Project, edit::EditCmd, effects::framing_at, media::probe, model::Canvas};
use nuzky_vision::follow::follow;

const SIDE: f32 = 160.0;
const WIDE: [f32; 2] = [1280.0, 720.0];

/// The pattern's top-left corner at `t` seconds of the file.
fn pattern_at(t: f32) -> [f32; 2] {
    [100.0 + 120.0 * t, 280.0 + 120.0 * (0.8 * t).sin()]
}

fn crossing() -> Option<PathBuf> {
    if !Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success()) {
        eprintln!("SKIP: ffmpeg CLI is unavailable");
        return None;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp-test/qa/follow");
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("crossing.mp4");
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-threads", "2", "-f", "lavfi", "-i"])
        .arg("color=c=gray:s=1280x720:r=30:d=8,geq=lum='128+50*sin(X/9)*cos(Y/13)+30*sin((X-2*Y)/31)':cb=128:cr=128")
        .args(["-f", "lavfi", "-i", "testsrc2=s=160x160:r=30:d=8", "-filter_complex"])
        .arg("[0][1]overlay=x='100+120*t':y='280+120*sin(0.8*t)'")
        .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    Some(dunce::canonicalize(out).unwrap())
}

#[test]
fn a_boxed_thing_crossing_a_wide_picture_stays_whole_in_a_vertical_frame() {
    let Some(file) = crossing() else { return };
    let mut project = Project::new("follow");
    project.canvas = Canvas { width: 1080, height: 1920, ..project.canvas };
    project.apply(EditCmd::AddAssets { assets: vec![probe(&file, "wide".into()).unwrap()] }).unwrap();
    project.apply(EditCmd::AddClip { asset_id: "wide".into(), start_us: None, track_id: None }).unwrap();
    let clip = project.tracks[0].clips[0].id.clone();
    // Boxed at 3 s, where the pattern is a third of the way across.
    let [x, y] = pattern_at(3.0);
    let region = [x / WIDE[0], y / WIDE[1], SIDE / WIDE[0], SIDE / WIDE[1]];
    let followed = follow(&project, &clip, 3_000_000, region, &AtomicBool::new(false), &mut |_| {}).unwrap();
    assert_eq!(followed.lost_us, 0, "lost for {} µs", followed.lost_us);
    assert!(followed.keyframes >= 2, "the picture moves with it: {}", followed.keyframes);
    for cmd in followed.commands {
        project.apply(cmd).unwrap();
    }
    let clip = &project.tracks[0].clips[0];
    let area = project.canvas.safe_area().unwrap();
    for t in (0..32).map(|i| 125_000 + i * 250_000) {
        let at = framing_at(clip, t);
        // At scale 1 the wide picture fits the 1080 px width of the vertical canvas.
        let width = 1080.0 * at.scale;
        let [left, _] = pattern_at(t as f32 / 1e6);
        let centre = 1080.0 * (0.5 + at.x) + ((left + SIDE / 2.0) / WIDE[0] - 0.5) * width;
        let half = SIDE / 2.0 / WIDE[0] * width;
        assert!(centre - half >= 0.0 && centre + half <= 1080.0, "cut at {t} µs: {centre} ± {half}");
        assert!(area.left < centre && centre < area.right, "outside the safe area at {t} µs: {centre}");
    }
}

//! A frame whose decoding takes longer than it used to be allowed, because the computer slept or the app did not run in
//! the middle of it, still comes out of `nuzky frame` exact, as FFmpeg decodes it, instead of "Timed out waiting for
//! frame". Covers, exports and the paused preview wait the same way.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nuzky_engine::edit::new_id;

const PAUSE: Duration = Duration::from_secs(10);

fn ffmpeg(args: &[&str]) {
    let status = Command::new("ffmpeg").args(["-v", "error", "-y"]).args(args).status().unwrap();
    assert!(status.success(), "ffmpeg {args:?}");
}

fn can_encode_hevc() -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains("libx265"))
}

/// HEVC as a phone records it, stored landscape and shown portrait at a jittered frame rate, with its only keyframe at
/// the start, so a frame near the end decodes the whole clip first.
fn phone_clip(dir: &Path) -> PathBuf {
    let stored = dir.join("stored.mov");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=s=2560x1440:r=60:d=8,noise=alls=6:allf=t",
        "-vf",
        "settb=1/600,setpts='(N*10+eq(mod(N\\,4)\\,1)*4)/600/TB'",
        "-fps_mode",
        "vfr",
        "-enc_time_base",
        "1/600",
        "-c:v",
        "libx265",
        "-preset",
        "ultrafast",
        "-x265-params",
        "log-level=error:keyint=600",
        "-tag:v",
        "hvc1",
        "-video_track_timescale",
        "600",
        "-pix_fmt",
        "yuv420p",
        stored.to_str().unwrap(),
    ]);
    let phone = dir.join("phone.mov");
    ffmpeg(&["-display_rotation", "-90", "-i", stored.to_str().unwrap(), "-c", "copy", phone.to_str().unwrap()]);
    phone
}

fn frame_times(path: &Path) -> Vec<i64> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v", "-show_entries", "frame=pts_time", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap();
    let mut times: Vec<i64> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.split(',').next()?.trim().parse::<f64>().ok())
        .map(|t| (t * 1e6).round() as i64)
        .collect();
    times.sort_unstable();
    times
}

/// The upright frame at `t_us`, one of the file's frame times, as FFmpeg decodes it, at 360×640 in RGBA.
fn ffmpeg_frame(path: &Path, t_us: i64) -> Vec<u8> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-ss", &format!("{:.6}", t_us as f64 / 1e6), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf", "scale=360:640:flags=area", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .unwrap();
    assert!(out.status.success());
    out.stdout
}

fn png_rgba(path: &Path) -> Vec<u8> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .unwrap();
    assert!(out.status.success());
    out.stdout
}

fn mean_difference(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as f64).sum::<f64>() / a.len() as f64
}

/// Whether process `pid` has `path` open: the CLI opens the clip when it starts decoding it.
fn has_open(pid: u32, path: &Path) -> bool {
    if cfg!(target_os = "linux") {
        std::fs::read_dir(format!("/proc/{pid}/fd"))
            .map(|fds| fds.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|link| link == path)))
            .unwrap_or(false)
    } else {
        let out = Command::new("lsof").args(["-p", &pid.to_string(), "-Fn"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).lines().any(|line| line.strip_prefix('n') == Some(path.to_str().unwrap()))
    }
}

fn signal(pid: u32, which: &str) {
    assert!(Command::new("kill").args([which, &pid.to_string()]).status().unwrap().success());
}

fn scratch() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap();
    let dir = root.join(format!("tmp-test/slow-decoding-{}", new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The CLI with Nuzky's data, cache and home of its own under `dir`.
fn nuzky(dir: &Path, args: &[&str]) -> Command {
    let mut cli = Command::new(env!("CARGO_BIN_EXE_nuzky"));
    cli.env("HOME", dir.join("home"))
        .env("NUZKY_DATA_DIR", dir.join("data"))
        .env("XDG_DATA_HOME", dir.join("xdg-data"))
        .env("XDG_CACHE_HOME", dir.join("xdg-cache"))
        .env("XDG_RUNTIME_DIR", dir.join("run"))
        .args(args);
    cli
}

fn new_project(dir: &Path, clip: &Path) -> PathBuf {
    let project = dir.join("project.nuzky");
    let made = nuzky(dir, &["new", project.to_str().unwrap(), clip.to_str().unwrap()]).output().unwrap();
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    project
}

#[test]
fn a_frame_decoding_while_the_computer_sleeps_comes_out_exact() {
    if Command::new("ffmpeg").arg("-version").output().is_err() || !can_encode_hevc() {
        eprintln!("SKIP: ffmpeg with libx265 is required");
        return;
    }
    let dir = scratch();
    let clip = phone_clip(&dir);
    let project = new_project(&dir, &clip);
    let times = frame_times(&clip);
    let t = times[times.len() - 2];
    let out = dir.join("frame.png");

    let started = Instant::now();
    let mut child = nuzky(
        &dir,
        &["frame", project.to_str().unwrap(), &format!("{:.6}", t as f64 / 1e6), out.to_str().unwrap(), "360"],
    )
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
    while !has_open(child.id(), &clip) && child.try_wait().unwrap().is_none() {
        assert!(started.elapsed() < Duration::from_secs(60), "the CLI never opened the clip");
        std::thread::sleep(Duration::from_millis(5));
    }
    // In the middle of decoding up to the frame, nothing of the CLI runs while its clock goes on.
    let paused = child.try_wait().unwrap().is_none();
    if paused {
        signal(child.id(), "-STOP");
        std::thread::sleep(PAUSE);
        signal(child.id(), "-CONT");
    }
    let finished = child.wait_with_output().unwrap();
    assert!(paused, "the frame was drawn before the CLI could be paused");
    assert!(finished.status.success(), "{}", String::from_utf8_lossy(&finished.stderr));

    // Each frame has grain of its own, so of the frames around it the one at `t` looks most like what was drawn.
    let drawn = png_rgba(&out);
    let near: Vec<(i64, f64)> =
        times[times.len() - 4..].iter().map(|&at| (at, mean_difference(&drawn, &ffmpeg_frame(&clip, at)))).collect();
    let closest = near.iter().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    assert!(closest.0 == t && closest.1 < 10.0, "the frame at {t} µs drew as: {near:?}");
    std::fs::remove_dir_all(&dir).ok();
}

/// A video that stops coming, as from a drive that no longer answers (here a pipe nobody writes to), ends the frame
/// after `STALL` with what happened and what to do, instead of waiting for it forever.
#[test]
fn a_video_that_stops_coming_ends_with_what_to_do() {
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("SKIP: ffmpeg is required");
        return;
    }
    let dir = scratch();
    let clip = dir.join("clip.ts");
    ffmpeg(&["-f", "lavfi", "-i", "testsrc2=s=320x180:r=30:d=1", "-c:v", "libx264", clip.to_str().unwrap()]);
    let project = new_project(&dir, &clip);
    let stuck = dir.join("stuck.ts");
    assert!(Command::new("mkfifo").arg(&stuck).status().unwrap().success());
    let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&project).unwrap()).unwrap();
    json["assets"][0]["path"] = serde_json::json!(stuck);
    std::fs::write(&project, serde_json::to_vec(&json).unwrap()).unwrap();

    let started = Instant::now();
    let out = dir.join("frame.png");
    let mut child = nuzky(&dir, &["frame", project.to_str().unwrap(), "0.5", out.to_str().unwrap(), "160"])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    while child.try_wait().unwrap().is_none() && started.elapsed() < Duration::from_secs(90) {
        std::thread::sleep(Duration::from_millis(100));
    }
    let waited = started.elapsed();
    child.kill().ok();
    let finished = child.wait_with_output().unwrap();
    let said = String::from_utf8_lossy(&finished.stderr);
    assert!(waited < Duration::from_secs(60), "the frame was still waiting after {waited:?}: {said}");
    assert!(
        !finished.status.success() && said.contains("no picture came") && said.contains("copy it to this computer"),
        "{said}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

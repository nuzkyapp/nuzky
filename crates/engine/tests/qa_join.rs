mod qa_support;
use nuzky_engine::{
    join::join,
    media::{extract_pcm, probe},
    model::{CHANNELS, Part, SAMPLE_RATE},
};
use qa_support::*;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
};

const DURATION_US: i64 = 12_000_000;
const FRAME_US: i64 = 40_000;
/// When the sound beeps, at uneven gaps so a part shifted by any whole gap is caught.
const BEEPS: [f64; 8] = [0.5, 2.3, 4.1, 5.35, 6.9, 8.45, 10.2, 11.5];

/// YouTube's separate picture and sound: 25 fps with a keyframe every 2 s, and short beeps at `BEEPS`.
fn sources(d: &Path, video: &str, audio: &str, vcodec: &[&str], acodec: &[&str]) -> (PathBuf, PathBuf) {
    let (v, a) = (d.join(video), d.join(audio));
    let seconds = (DURATION_US / 1_000_000).to_string();
    let mut args = vec!["-f", "lavfi", "-i", "testsrc2=s=320x240:r=25", "-t", &seconds, "-an"];
    args.extend(vcodec);
    args.extend(["-g", "50", "-keyint_min", "50"]);
    ff(&args, &v);
    let beeps: Vec<String> = BEEPS.iter().map(|t| format!("between(t,{t},{})", t + 0.02)).collect();
    let tone = format!("aevalsrc='0.8*sin(2*PI*1000*t)*({})':s=48000:d={seconds}", beeps.join("+"));
    let mut args = vec!["-f", "lavfi", "-i", &tone];
    args.extend(acodec);
    ff(&args, &a);
    (v, a)
}

/// The picture FFmpeg shows `t_us` into `path`, as RGBA.
fn frame_at(path: &Path, t_us: i64) -> Vec<u8> {
    let at = format!("{:.6}", t_us as f64 / 1e6);
    // After the input, -ss decodes from the start and drops the frames before it.
    let picture = raw(path, &["-ss", &at, "-frames:v", "1"]);
    assert_eq!(picture.len(), 320 * 240 * 4, "a frame at {at} s of {}", path.display());
    picture
}

/// When each beep starts in the sound Nuzky hears from `path`, in microseconds from the file's start.
fn beeps(d: &Path, path: &Path) -> Vec<i64> {
    let pcm_path = d.join("beeps.pcm");
    extract_pcm(path, &pcm_path, |_| Ok(())).unwrap();
    let pcm = pcm(&pcm_path);
    let mut onsets = Vec::new();
    let mut last = -1i64;
    for (i, frame) in pcm.chunks(CHANNELS).enumerate() {
        let i = i as i64;
        if frame[0].abs() > 0.3 {
            if i - last > SAMPLE_RATE as i64 / 10 {
                onsets.push(i * 1_000_000 / SAMPLE_RATE as i64);
            }
            last = i;
        }
    }
    onsets
}

fn assert_valid(out: &Path, kept: Option<Part>) {
    let asset = probe(out, "joined".into()).unwrap();
    assert!(asset.has_audio && asset.width == 320, "{asset:?}");
    if let Some(part) = kept {
        assert!(
            0 <= part.start_us && part.start_us < part.end_us && part.end_us <= asset.duration_us,
            "{part:?} {asset:?}"
        );
    }
    // Every frame decodes, from a keyframe on.
    let decoded = run(Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(out).args(["-f", "null", "-"]));
    assert!(decoded.stderr.is_empty(), "{}", String::from_utf8_lossy(&decoded.stderr));
    let first = run(Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v", "-show_entries", "packet=flags", "-read_intervals", "%+#1"])
        .args(["-of", "csv=p=0"])
        .arg(out));
    assert!(String::from_utf8_lossy(&first.stdout).starts_with('K'), "the first picture is a keyframe");
}

fn check_pair(d: &Path, video: &Path, audio: &Path) {
    let outs = d.join("out");
    std::fs::remove_dir_all(&outs).ok();
    std::fs::create_dir_all(&outs).unwrap();
    let no = AtomicBool::new(false);
    let joined = |name: &str, part| {
        let out = outs.join(name);
        (join(video, Some(audio), &out, part, &no).unwrap(), out)
    };

    // The whole video, with each beep where it was.
    let (kept, whole) = joined("whole.mp4", None);
    assert_eq!(kept, None);
    assert_valid(&whole, None);
    let duration = probe(&whole, "whole".into()).unwrap().duration_us;
    assert!((duration - DURATION_US).abs() <= 50_000, "{duration}");
    for (heard, beep) in beeps(d, &whole).iter().zip(BEEPS) {
        assert!((heard - (beep * 1e6) as i64).abs() <= 1_000, "beep at {beep} s heard at {heard} us");
    }
    assert!(frame_at(&whole, 5_020_000) == frame_at(video, 5_020_000));

    // A part in the middle: the picture and the sound at its start in the new file are the source's at its start.
    let part = Part { start_us: 5_020_000, end_us: 9_020_000 };
    let (kept, cut) = joined("middle.mp4", Some(part));
    let kept = kept.unwrap();
    assert_valid(&cut, Some(kept));
    assert_eq!(kept.end_us - kept.start_us, part.end_us - part.start_us);
    // The last keyframe at or before 3 s ahead of the part is the one at 2 s.
    assert_eq!(kept.start_us, part.start_us - 2_000_000, "{kept:?}");
    let source = frame_at(video, part.start_us);
    let error = mae(&frame_at(&cut, kept.start_us), &source);
    assert!(error < 0.5, "the part starts on another picture than the source's: MAE {error:.2}");
    let neighbour = mae(&frame_at(video, part.start_us + FRAME_US), &source);
    assert!(neighbour > 2.0, "neighbouring frames must differ for the check to mean anything: MAE {neighbour:.2}");
    let error = mae(&frame_at(&cut, kept.end_us - FRAME_US), &frame_at(video, part.end_us - FRAME_US));
    assert!(error < 0.5, "the part ends on another picture than the source's: MAE {error:.2}");
    let shift = part.start_us - kept.start_us;
    let heard = beeps(d, &cut);
    let expected: Vec<i64> = BEEPS.iter().map(|b| (b * 1e6) as i64 - shift).filter(|&t| t > 100_000).collect();
    assert!(heard.len() >= 4, "{heard:?}");
    for (heard, expected) in heard.iter().zip(&expected) {
        assert!((heard - expected).abs() <= 1_000, "beep expected at {expected} us heard at {heard} us");
    }

    // Parts at the very start and end, one frame long, 1 s from either end and all of it.
    let end = DURATION_US;
    for (i, (start_us, end_us)) in
        [(0, FRAME_US), (1_000_000, 2_000_000), (end - 2_000_000, end - 1_000_000), (end - FRAME_US, end), (0, end)]
            .into_iter()
            .enumerate()
    {
        let part = Part { start_us, end_us };
        let (kept, out) = joined(&format!("part-{i}.mp4"), Some(part));
        let kept = kept.unwrap();
        assert_valid(&out, Some(kept));
        assert!(kept.end_us - kept.start_us > (end_us - start_us) - FRAME_US, "{part:?} became {kept:?}");
        let spare = start_us.min(3_000_000);
        assert!((spare..=spare + 2_000_000).contains(&kept.start_us), "{part:?} became {kept:?}");
        let error = mae(&frame_at(&out, kept.start_us), &frame_at(video, start_us));
        assert!(error < 0.5, "{part:?} starts on another picture: MAE {error:.2}");
    }

    // A stopped join leaves nothing behind, also once it has started writing.
    let stop = AtomicBool::new(true);
    for part in [None, Some(part)] {
        let error = join(video, Some(audio), &outs.join("stopped.mp4"), part, &stop).unwrap_err();
        assert!(error.to_string().starts_with("CANCELLED"), "{error:#}");
    }
    let left: Vec<_> = std::fs::read_dir(&outs).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(left.len(), 7, "{left:?}");
    assert!(left.iter().all(|n| !n.to_string_lossy().starts_with(".nuzky-part") && n != "stopped.mp4"), "{left:?}");
}

#[test]
fn join_copies_h264_and_aac_and_keeps_a_part_in_sync() {
    if !available() {
        return;
    }
    let d = dir("join-h264");
    let vcodec = ["-c:v", "libx264", "-preset", "veryfast", "-bf", "3", "-sc_threshold", "0", "-crf", "12"];
    let (video, audio) = sources(&d, "video.mp4", "audio.m4a", &vcodec, &["-c:a", "aac", "-b:a", "192k"]);
    assert!(info(&video)["streams"][0]["has_b_frames"].as_u64().unwrap() > 0, "the source has B-frames");
    check_pair(&d, &video, &audio);
}

#[test]
fn join_copies_vp9_and_opus_and_keeps_a_part_in_sync() {
    if !available() {
        return;
    }
    let encoders = run(Command::new("ffmpeg").args(["-hide_banner", "-encoders"]));
    let encoders = String::from_utf8_lossy(&encoders.stdout);
    if !encoders.contains("libvpx-vp9") || !encoders.contains("libopus") {
        eprintln!("SKIP: ffmpeg without libvpx-vp9 or libopus");
        return;
    }
    let d = dir("join-vp9");
    let vcodec = ["-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-crf", "20", "-b:v", "0"];
    let (video, audio) = sources(&d, "video.webm", "audio.webm", &vcodec, &["-c:a", "libopus", "-b:a", "128k"]);
    check_pair(&d, &video, &audio);
}

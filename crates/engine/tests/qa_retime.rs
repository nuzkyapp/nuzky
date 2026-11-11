//! Clips played backwards and on speed ramps show, in the preview and in the export, the frames FFmpeg decodes at
//! the source times a ramp or reverse puts there, worked out here apart from the engine, and their sound stays with
//! their picture.
mod qa_support;
use nuzky_engine::worker::VideoWorker;
use nuzky_engine::{
    Renderer, Wait,
    audio::{Mixer, ensure_pcm},
    edit::EditCmd,
    export::{ExportOptions, export, frame_time_us},
    model::*,
};
use qa_support::*;
use std::{path::Path, process::Command, sync::atomic::AtomicBool};

const FPS: u32 = 30;

fn apply(p: &mut Project, edit: serde_json::Value) {
    p.apply(serde_json::from_value::<EditCmd>(edit).unwrap()).unwrap();
}

fn update(p: &mut Project, mut edit: serde_json::Value) {
    edit["type"] = "updateClip".into();
    edit["clipId"] = "qa-clip".into();
    apply(p, edit);
}

/// A 4 s counter at 30 fps with two-second GOPs and B-frames, so playing backwards crosses keyframes.
fn counter(path: &Path) {
    ff(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=96x64:rate=30:duration=4",
            "-vf",
            "drawtext=text='%{n}':x=4:y=4:fontsize=22:fontcolor=white:box=1:boxcolor=black",
            "-c:v",
            "libx264",
            "-threads",
            "2",
            "-g",
            "60",
            "-bf",
            "3",
            "-crf",
            "16",
            "-pix_fmt",
            "yuv420p",
        ],
        path,
    );
}

/// The source frame `frame` looks most like.
fn closest(frame: &[u8], truth: &[u8], frame_len: usize) -> (usize, f64) {
    truth.chunks(frame_len).map(|f| mae(frame, f)).enumerate().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap()
}

/// How a case retimes a clip of 0.5 s to 3.5 s of the counter, and the source time it plays at each timeline offset,
/// worked out on paper: the speed moves evenly from the first to the second, so the source covered by offset `u` is
/// the area under that line.
struct Case {
    name: &'static str,
    edit: serde_json::Value,
    speeds: (f64, f64),
    reverse: bool,
}

const FROM: f64 = 500_000.0;
const LENGTH: f64 = 3_000_000.0;

impl Case {
    fn duration(&self) -> f64 {
        LENGTH / ((self.speeds.0 + self.speeds.1) / 2.0)
    }

    fn source(&self, u: f64) -> f64 {
        let d = self.duration();
        let x = u / d;
        let (v0, v1) = self.speeds;
        let covered = d * (v0 * x + (v1 - v0) * x * x / 2.0);
        if self.reverse { FROM + LENGTH - covered } else { FROM + covered }
    }

    /// The source frame shown at `u`: forwards the one at or before its time, backwards the one before it.
    fn frame(&self, pts: &[i64], u: f64) -> usize {
        let s = self.source(u).round() as i64;
        pts.iter().rposition(|&p| if self.reverse { p < s } else { p <= s }).unwrap()
    }
}

#[test]
fn reversed_and_ramped_clips_show_the_frames_ffmpeg_decodes_there_in_preview_and_export() {
    if !available() {
        return;
    }
    let d = dir("retime-frames");
    let source = d.join("counter.mp4");
    counter(&source);
    let pts = pts(&source);
    let truth = raw(&source, &[]);
    let frame_len = 96 * 64 * 4;
    assert_eq!(truth.len() / frame_len, pts.len());
    let cases = [
        Case { name: "backwards", edit: serde_json::json!({"reverse": true}), speeds: (1.0, 1.0), reverse: true },
        Case { name: "backwards-2x", edit: serde_json::json!({"speed": 2.0}), speeds: (2.0, 2.0), reverse: true },
        Case {
            name: "ramp-down",
            edit: serde_json::json!({"ramp": [{"at": 0.0, "speed": 2.0}, {"at": 1.0, "speed": 0.5}]}),
            speeds: (2.0, 0.5),
            reverse: false,
        },
        Case {
            name: "ramp-up-backwards",
            edit: serde_json::json!({"ramp": [{"at": 0.0, "speed": 0.5}, {"at": 1.0, "speed": 3.0}]}),
            speeds: (0.5, 3.0),
            reverse: true,
        },
    ];
    let cache = d.join("cache");
    let mut failures = Vec::new();
    for case in &cases {
        let mut p = project(&source, 3_000_000);
        p.canvas.fps = FPS;
        if let ClipContent::Media { source_in_us, .. } = &mut p.tracks[0].clips[0].content {
            *source_in_us = FROM as i64;
        }
        if case.reverse {
            update(&mut p, serde_json::json!({"reverse": true}));
        }
        update(&mut p, case.edit.clone());
        let clip = &p.tracks[0].clips[0];
        let expected_duration = case.duration();
        assert!(
            (clip.duration_us as f64 - expected_duration).abs() <= 1.0,
            "{}: the clip is {} µs long, not {expected_duration}",
            case.name,
            clip.duration_us
        );

        let out = d.join(format!("{}.mp4", case.name));
        let options =
            ExportOptions { preset: "ultrafast".into(), crf: 12, replace_existing: true, ..Default::default() };
        let started = std::time::Instant::now();
        export(&p, &cache, &out, &options, &AtomicBool::new(false), |_| {}).unwrap();
        eprintln!("QA retime {}: exported {:.2} s in {:?}", case.name, expected_duration / 1e6, started.elapsed());
        let exported = raw(&out, &[]);
        let frames = exported.len() / frame_len;
        assert!(frames + 1 >= (expected_duration / 1e6 * FPS as f64) as usize, "{}: {frames} frames", case.name);

        // The preview plays through the clip as playback does, then jumps to a few frames as a paused playhead does.
        let mut renderer = Renderer::new().unwrap();
        let mut times: Vec<u64> = (0..frames as u64).collect();
        times.extend([frames as u64 / 2, 3, frames as u64 - 2, 10]);
        for (n, i) in times.into_iter().enumerate() {
            let t = frame_time_us(i, FPS);
            let want = case.frame(&pts, t as f64);
            let playing = n < frames;
            let shown = renderer.render(&p, t, 96, 64, Wait::Exact, playing).unwrap();
            let (got, error) = closest(&shown, &truth, frame_len);
            if got != want {
                failures.push(format!("{} preview at {t} µs: frame {got} (MAE {error:.2}), not {want}", case.name));
            }
            if playing {
                let (got, error) = closest(&exported[i as usize * frame_len..][..frame_len], &truth, frame_len);
                if got != want {
                    failures.push(format!("{} export at {t} µs: frame {got} (MAE {error:.2}), not {want}", case.name));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} frames wrong:\n{}", failures.len(), failures.join("\n"));
}

/// Played backwards, the decoder serves frames from pieces it decoded forwards; a playhead that jumps back on the
/// timeline (later in the file) or pauses there still gets the exact frame, never the last one it had.
#[test]
fn backwards_playback_that_jumps_or_pauses_shows_the_frame_there() {
    if !available() {
        return;
    }
    let d = dir("retime-jump");
    let source = d.join("counter.mp4");
    counter(&source);
    let pts = pts(&source);
    let frame_of = |t: i64| pts[pts.iter().rposition(|&p| p <= t).unwrap()];
    let mut worker = VideoWorker::spawn(source);
    let mut failures = Vec::new();
    let mut ask = |t: i64, playing: bool| {
        let got = worker.get(t, (96, 64), playing, true, true).unwrap();
        if got.t_us != frame_of(t) || !worker.exact {
            failures.push(format!(
                "at {t} µs playing {playing}: frame {} exact {}, not {}",
                got.t_us,
                worker.exact,
                frame_of(t)
            ));
        }
    };
    for i in 0..40 {
        ask(3_000_000 - 1 - i * 33_333, true);
    }
    // Half a second back on the timeline, still playing, then paused a little further on.
    ask(2_200_000, true);
    ask(2_150_000, true);
    ask(2_600_000, false);
    ask(3_500_000, false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A flash and a click at the same moment of the file stay together played backwards and on a ramp, in the export,
/// where the ramp puts them, and a split of the clip plays on without a seam.
#[test]
fn sound_stays_with_the_picture_backwards_and_on_a_ramp() {
    if !available() {
        return;
    }
    let d = dir("retime-sync");
    let source = d.join("flash.mkv");
    // White frames and a 1 kHz tone from 1 s to 1.2 s of a 3 s file; nothing else moves or sounds.
    let tone = "if(between(t,1,1.2),0.8*sin(2*PI*1000*t),0)";
    ff(
        &[
            "-f",
            "lavfi",
            "-i",
            "nullsrc=s=64x64:r=30:d=3,format=gray,geq=lum='if(between(N,30,35),235,16)'",
            "-f",
            "lavfi",
            "-i",
            &format!("aevalsrc='{tone}|{tone}':s=48000:d=3"),
            "-c:v",
            "libx264",
            "-threads",
            "2",
            "-g",
            "60",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "pcm_f32le",
        ],
        &source,
    );
    let cache = d.join("cache");
    // Each case splits the clip while the tone plays.
    for (name, edit, onset, split_at) in [
        // 3 s backwards: the white frames and the tone, 1.2 s to 1 s of the file, play from 1.8 s to 2 s.
        ("backwards", serde_json::json!({"reverse": true}), 1_800_000.0, 1_900_000),
        // From 2x to 0.5x, an average of 1.25x over 2.4 s: 1 s of the file is behind once 2.4·(2x − 0.75x²) = 1, at
        // x = (2 − √2.75)/1.5 = 0.22779, 546.7 ms in; 1.2 s at x = (2 − √2.5)/1.5, 670.2 ms in.
        (
            "ramp",
            serde_json::json!({"ramp": [{"at": 0.0, "speed": 2.0}, {"at": 1.0, "speed": 0.5}]}),
            546_700.0,
            600_000,
        ),
    ] {
        let mut p = project(&source, 3_000_000);
        p.canvas.fps = FPS;
        ensure_pcm(&cache, &p.assets[0], |_| Ok(())).unwrap();
        update(&mut p, edit);

        let out = d.join(format!("{name}.mp4"));
        let options = ExportOptions { preset: "ultrafast".into(), replace_existing: true, ..Default::default() };
        export(&p, &cache, &out, &options, &AtomicBool::new(false), |_| {}).unwrap();
        let picture = raw(&out, &[]);
        let bright = picture
            .chunks(64 * 64 * 4)
            .position(|f| f.iter().map(|&b| b as u64).sum::<u64>() / f.len() as u64 > 128)
            .expect("no white frame");
        let flash = frame_time_us(bright as u64, FPS) as f64;
        let sound = run(Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&out)
            .args(["-vn", "-f", "f32le", "-ac", "2", "-ar", "48000", "-"]))
        .stdout;
        let sound: Vec<f32> = sound.as_chunks::<4>().0.iter().map(|&b| f32::from_le_bytes(b)).collect();
        let click = sound.iter().position(|s| s.abs() > 0.05).expect("no tone") / 2;
        let click = click as f64 / 48_000.0 * 1e6;
        eprintln!("QA retime sync {name}: white from {flash:.0} µs, tone from {click:.0} µs, expected {onset:.0} µs");
        let frame = 1e6 / FPS as f64;
        assert!((click - onset).abs() < 5_000.0, "{name}: the tone starts at {click:.0} µs, not {onset:.0}");
        // The picture turns white on the first frame from the tone on; the tone's first loud sample comes a few
        // samples into its sine.
        assert!(
            click - 1_000.0 <= flash && flash < click + frame,
            "{name}: the first white frame at {flash:.0} µs is not the first frame from the tone on, {click:.0} µs"
        );

        // Split anywhere, the two pieces sound exactly as the whole.
        let mix = |p: &Project| {
            let mut out = vec![0.0; 3 * 48_000 * CHANNELS];
            let mut mixer = Mixer::new(cache.clone());
            for (index, part) in out.chunks_mut(1024 * CHANNELS).enumerate() {
                mixer.mix(p, (index * 1024) as i64, part);
            }
            out
        };
        let whole = mix(&p);
        let mut halves = p.clone();
        apply(&mut halves, serde_json::json!({"type": "splitClip", "clipId": "qa-clip", "atUs": split_at}));
        let split = mix(&halves);
        // The pieces of a ramp keep their speeds as f32, about -75 dBFS here; a crossfade at the split would dip
        // the tone by tenths.
        let step = split.iter().zip(&whole).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(step < 1e-3, "{name}: a split changes the sound by {step}");
    }
}

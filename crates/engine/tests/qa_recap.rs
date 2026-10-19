mod qa_support;
use nuzky_engine::{
    Renderer, Wait,
    edit::{EditCmd, Editor, TimeRange},
    media::probe,
    model::*,
    recap::{Recap, RecapCut, RecapOptions, RecapWord, cut_times, duration_us, render, sections},
};
use qa_support::*;
use std::{
    path::Path,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
};

const S: i64 = 1_000_000;

fn sound(id: &str) -> Asset {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/sounds").join(format!("{id}.ogg"));
    probe(&path, format!("nuzky:{id}")).unwrap()
}

/// A 20 s vertical talking-head take with a 440 Hz tone, cut at 2–4 s and 10–12.5 s, with the built-in snips.
/// With `music`, an 880 Hz tone from 5 s on an audio track ducks under it, as music does.
fn recap(dir: &Path, music: bool) -> Recap {
    let take = dir.join("take.mp4");
    if !take.exists() {
        let face = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../site/public/media/ugc.jpg");
        ff(
            &[
                "-loop",
                "1",
                "-framerate",
                "30",
                "-i",
                face.to_str().unwrap(),
                "-f",
                "lavfi",
                "-i",
                "sine=f=440:r=48000:d=20",
                "-vf",
                "scale=1080:1920,drawtext=text='%{n}':x=40:y=1700:fontsize=60:fontcolor=white:box=1:boxcolor=black",
                "-t",
                "20",
                "-pix_fmt",
                "yuv420p",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac",
            ],
            &take,
        );
    }
    let mut before = Project::new("Recap QA");
    before.assets.push(probe(&take, "take".into()).unwrap());
    before.tracks[0].clips.push(clip("take-clip", "take", 0, 20 * S));
    if music {
        let song = dir.join("song.m4a");
        if !song.exists() {
            ff(&["-f", "lavfi", "-i", "sine=f=880:r=48000:d=40", "-c:a", "aac"], &song);
        }
        before.assets.push(probe(&song, "song".into()).unwrap());
        let mut song = clip("song-clip", "song", 5 * S, 15 * S);
        if let ClipContent::Media { duck_db, .. } = &mut song.content {
            *duck_db = 12.0;
        }
        before.tracks.push(Track {
            id: "music".into(),
            kind: TrackKind::Audio,
            name: "Music".into(),
            muted: false,
            hidden: false,
            keep_in_place: true,
            clips: vec![song],
        });
    }
    let mut editor = Editor::new(before.clone());
    let cut = |a: i64, b: i64| TimeRange { start_us: a, end_us: b };
    editor
        .apply(
            EditCmd::RippleDeleteRanges {
                ranges: vec![cut(2 * S, 4 * S), cut(10 * S, 12_500_000)],
                keep_track_ids: None,
            },
            None,
        )
        .unwrap();
    let text = "Dneska vám ukážu jak natočit video na mobil za deset minut potřebujete jen telefon a okno \
                Okno je vaše nejlepší světlo postavte se k němu čelem a kameru dejte do výšky očí";
    let cuts = [(2 * S, 4 * S), (10 * S, 12_500_000)];
    let words: Vec<RecapWord> = text
        .split(' ')
        .enumerate()
        .map(|(i, text)| {
            let start_us = i as i64 * 600_000;
            let middle = start_us + 200_000;
            RecapWord {
                start_us,
                end_us: start_us + 400_000,
                text: text.into(),
                removed: cuts.iter().any(|&(a, b)| a <= middle && middle < b),
            }
        })
        .collect();
    let in_cut = |(a, b): (i64, i64)| {
        words.iter().filter(|w| w.removed && a <= w.start_us + 200_000 && w.start_us + 200_000 < b).count()
    };
    Recap {
        cuts: vec![
            RecapCut { start_us: 2 * S, end_us: 4 * S, words: in_cut(cuts[0]), retakes: 1 },
            RecapCut { start_us: 10 * S, end_us: 12_500_000, words: in_cut(cuts[1]), retakes: 0 },
        ],
        before,
        after: editor.project,
        words,
        transcribed: true,
        snips: ["scissors-snip", "scissors-snip-2", "scissors-snip-3"].map(sound).to_vec(),
        whoosh: None,
        land: None,
    }
}

/// Frame `n` of the recap as RGBA, 1080 × 1920.
fn frame(path: &Path, n: u32) -> Vec<u8> {
    raw(path, &["-vf", &format!("select=eq(n\\,{n})"), "-frames:v", "1"])
}

fn pixel(rgba: &[u8], x: usize, y: usize) -> [u8; 3] {
    let i = (y * 1080 + x) * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

fn red([r, g, b]: [u8; 3]) -> bool {
    r > 200 && g < 110 && b < 110
}

/// Root mean square of the recap's sound from `a` to `b` seconds, through `filter`.
fn rms(path: &Path, a: f64, b: f64, filter: &str) -> f64 {
    let out = run(Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(path).args([
        "-ss",
        &a.to_string(),
        "-to",
        &b.to_string(),
        "-af",
        filter,
        "-ac",
        "1",
        "-f",
        "f32le",
        "-",
    ]));
    let samples: Vec<f32> = out.stdout.as_chunks::<4>().0.iter().map(|&b| f32::from_le_bytes(b)).collect();
    (samples.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / samples.len().max(1) as f64).sqrt()
}

#[test]
fn recap_snaps_each_cut_shut_on_the_bar_and_ends_on_the_finished_cut_as_the_renderer_draws_it() {
    if !available() {
        return;
    }
    let d = dir("recap");
    let recap = recap(&d, false);
    let out = d.join("recap.mp4");
    let options = RecapOptions { end_card: true, replace_existing: true };
    render(&recap, &d.join("cache"), &out, &options, &AtomicBool::new(false), |_| {}).unwrap();

    let j = info(&out);
    let streams = j["streams"].as_array().unwrap();
    let video = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
    assert_eq!((video["width"].as_u64(), video["height"].as_u64()), (Some(1080), Some(1920)));
    assert!(streams.iter().any(|s| s["codec_name"] == "aac"));
    let length = j["format"]["duration"].as_str().unwrap().parse::<f64>().unwrap();
    let expected = duration_us(&recap, true) as f64 / 1e6;
    assert!((length - expected).abs() < 0.05, "{length} s instead of {expected} s");
    assert!(length <= 30.0);

    // Once drawn, the opening bar is red where the recording was cut and not where it was kept.
    let [hook, fast, landing, ..] = sections(&recap, true);
    let opening = frame(&out, 33);
    let bar_x = |at_us: i64| (60.0 + 840.0 * at_us as f64 / (20 * S) as f64) as usize;
    assert!(red(pixel(&opening, bar_x(11 * S), 1260)), "{:?}", pixel(&opening, bar_x(11 * S), 1260));
    assert!(!red(pixel(&opening, bar_x(7 * S), 1260)), "{:?}", pixel(&opening, bar_x(7 * S), 1260));

    // Each cut stays on screen at least 0.7 s, its picture stepped back to the graphite around it, its chip red
    // under the bar and its words struck; kept footage between them fills the frame.
    let times = cut_times(&recap);
    assert_eq!(times.len(), 2);
    assert!(times.iter().all(|t| t.end - t.start >= 700_000), "{times:?}");
    let at = |us: i64| (us * 30 / S) as u32;
    let cut = frame(&out, at((times[0].start + times[0].end) / 2));
    let kept = frame(&out, at((times[0].end + times[1].start) / 2 + 300_000));
    let graphite = |rgba: &[u8]| pixel(rgba, 8, 8).iter().all(|&c| c < 24);
    assert!(graphite(&cut), "the cut steps the picture back: {:?}", pixel(&cut, 8, 8));
    assert!(!graphite(&kept), "kept footage fills the frame: {:?}", pixel(&kept, 8, 8));
    let reds = |rgba: &[u8], (top, bottom): (usize, usize)| {
        (top..bottom).flat_map(|y| (60..900).map(move |x| (x, y))).filter(|&(x, y)| red(pixel(rgba, x, y))).count()
    };
    assert!(reds(&cut, (1312, 1380)) > 2000, "the cut's chip hangs under the bar: {}", reds(&cut, (1312, 1380)));
    assert!(reds(&cut, (900, 1090)) > 300, "the cut words are struck through: {} red pixels", reds(&cut, (900, 1090)));
    assert_eq!(reds(&kept, (900, 1090)) + reds(&kept, (1312, 1380)), 0, "nothing kept is struck or labelled");

    // Once both pieces have snapped shut, the bar ends where 15.5 s of 20 s ends.
    let landed = frame(&out, at(hook + fast + landing - 500_000));
    let end = bar_x(15_500_000);
    let lit = |x: usize| pixel(&landed, x, 1028).iter().all(|&c| c > 170);
    assert!(
        lit(end - 12) && !lit(end + 12),
        "{:?} {:?}",
        pixel(&landed, end - 12, 1028),
        pixel(&landed, end + 12, 1028)
    );

    // Five seconds into the finished cut, past its bar, the recap shows the renderer's frame.
    let start = hook + fast + landing;
    let n = ((start + 5 * S) * 30 / S) as u32;
    let into = nuzky_engine::export::frame_time_us(n as u64, 30) - start;
    let mut renderer = Renderer::new().unwrap();
    let drawn = renderer.render(&recap.after, into, 1080, 1920, Wait::Exact, false).unwrap();
    let shown = frame(&out, n);
    let error = mae(&drawn, &shown);
    assert!(error < 4.0, "the finished cut differs from the renderer by {error:.2} per channel");

    // The racing recording is silent but for a snip as each piece snaps shut; the finished cut plays its sound.
    let secs = |us: i64| us as f64 / 1e6;
    let snip = |t: &std::ops::Range<i64>| rms(&out, secs(t.end) - 0.1, secs(t.end) + 0.3, "highpass=f=2000");
    let quiet = rms(&out, secs(hook) + 0.1, secs(times[0].end) - 0.2, "anull");
    let voice = rms(&out, secs(start) + 1.0, secs(start) + 4.0, "anull");
    assert!(times.iter().all(|t| snip(t) > 0.005), "snips {:.4} {:.4}", snip(&times[0]), snip(&times[1]));
    assert!(quiet < 0.001 && voice > 0.1, "race {quiet:.4}, finished cut {voice:.4}");
    assert!(
        !std::fs::read_dir(&d).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".nuzky-part-"))
    );
}

#[test]
fn the_finished_cuts_music_plays_from_the_start_of_the_recap_even_when_it_comes_in_later_and_its_speech_only_with_its_picture()
 {
    if !available() {
        return;
    }
    let d = dir("recap-music");
    let recap = recap(&d, true);
    let out = d.join("recap.mp4");
    let options = RecapOptions { end_card: true, replace_existing: true };
    render(&recap, &d.join("cache"), &out, &options, &AtomicBool::new(false), |_| {}).unwrap();
    let [hook, fast, landing, sample, _] = sections(&recap, true);
    let secs = |us: i64| us as f64 / 1e6;
    let (music, speech) = ("bandpass=f=880:width_type=q:w=8", "bandpass=f=440:width_type=q:w=8");
    // Before 5 s, where the music comes in on the finished cut's timeline.
    let race = (secs(hook) + 0.3, (secs(hook) + 3.0).min(secs(hook + fast) - 0.2));
    let finished = (secs(hook + fast + landing) + 1.0, secs(hook + fast + landing + sample) - 1.0);
    let (race_music, race_speech) = (rms(&out, race.0, race.1, music), rms(&out, race.0, race.1, speech));
    let (cut_music, cut_speech) = (rms(&out, finished.0, finished.1, music), rms(&out, finished.0, finished.1, speech));
    assert!(race_music > 0.02 && race_speech < 0.005, "race: music {race_music:.4}, speech {race_speech:.4}");
    assert!(cut_speech > 0.1, "the finished cut has its speech: {cut_speech:.4}");
    // Under the racing recording the music is lower, so the snips stand out, and under speech it ducks further.
    assert!(cut_music < race_music, "music under speech {cut_music:.4}, while racing {race_music:.4}");
    let end = rms(&out, secs(duration_us(&recap, true)) - 0.15, secs(duration_us(&recap, true)) - 0.02, music);
    assert!(end < race_music / 4.0, "the music fades out at the end: {end:.4}");
}

#[test]
fn recap_keeps_an_existing_file_and_stops_when_cancelled() {
    if !available() {
        return;
    }
    let d = dir("recap-refuse");
    let recap = recap(&d, false);
    let out = d.join("existing.mp4");
    std::fs::write(&out, b"someone's video").unwrap();
    let options = RecapOptions { end_card: false, replace_existing: false };
    let error = render(&recap, &d.join("cache"), &out, &options, &AtomicBool::new(false), |_| {}).unwrap_err();
    assert!(error.to_string().contains("OUTPUT_EXISTS"), "{error:#}");
    assert_eq!(std::fs::read(&out).unwrap(), b"someone's video");

    let cancel = AtomicBool::new(false);
    let fresh = d.join("cancelled.mp4");
    let error = render(&recap, &d.join("cache"), &fresh, &options, &cancel, |p| {
        if p > 0.1 {
            cancel.store(true, Ordering::Relaxed);
        }
    })
    .unwrap_err();
    assert!(error.to_string().starts_with("CANCELLED"), "{error:#}");
    assert!(!fresh.exists());
    assert!(
        !std::fs::read_dir(&d).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".nuzky-part-"))
    );
}

#[test]
fn a_transition_into_a_clip_past_the_finished_cuts_sample_still_plays_its_sound() {
    if !available() {
        return;
    }
    let d = dir("recap-transition");
    let face = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../site/public/media/ugc.jpg");
    let take = |name: &str, tone: u32| {
        let path = d.join(name);
        if !path.exists() {
            let sine = format!("sine=f={tone}:r=48000:d=10");
            let video = face.to_str().unwrap();
            ff(
                &[
                    "-loop",
                    "1",
                    "-framerate",
                    "30",
                    "-i",
                    video,
                    "-f",
                    "lavfi",
                    "-i",
                    &sine,
                    "-vf",
                    "scale=1080:1920",
                    "-t",
                    "10",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:v",
                    "libx264",
                    "-preset",
                    "ultrafast",
                    "-c:a",
                    "aac",
                ],
                &path,
            );
        }
        probe(&path, name.into()).unwrap()
    };
    // The second take, from 2 s into its file, comes in over a 2 s dissolve centred on 8.2 s, so its sound starts at
    // 7.2 s, inside the recap's 8 s sample of the finished cut, though the clip itself starts after it.
    let mut after = Project::new("Transition");
    after.assets.extend([take("first.mp4", 440), take("second.mp4", 1320)]);
    after.tracks[0].clips.push(clip("first-clip", "first.mp4", 0, 8_200_000));
    let mut second = clip("second-clip", "second.mp4", 8_200_000, 5 * S);
    if let ClipContent::Media { source_in_us, .. } = &mut second.content {
        *source_in_us = 2 * S;
    }
    second.transition_in = Some(Transition { kind: TransitionKind::Dissolve, duration_us: 2 * S });
    after.tracks[0].clips.push(second);
    let recap = Recap {
        before: after.clone(),
        after,
        cuts: Vec::new(),
        words: Vec::new(),
        transcribed: false,
        snips: Vec::new(),
        whoosh: None,
        land: None,
    };
    let out = d.join("recap.mp4");
    let options = RecapOptions { end_card: false, replace_existing: true };
    render(&recap, &d.join("cache"), &out, &options, &AtomicBool::new(false), |_| {}).unwrap();
    let [hook, fast, landing, ..] = sections(&recap, false);
    let start = (hook + fast + landing) as f64 / 1e6;
    let incoming = rms(&out, start + 7.5, start + 7.7, "bandpass=f=1320:width_type=q:w=8");
    assert!(incoming > 0.01, "the incoming take is heard across the dissolve: {incoming:.4}");
}

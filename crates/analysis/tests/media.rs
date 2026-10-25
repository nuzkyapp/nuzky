use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context, Result, ensure};
use nuzky_analysis::{
    AudioSource, Range, SceneParams, SilenceParams, beats_cancellable, loudness, scene_cuts, scene_cuts_in, silences,
    silences_cancellable, transcribe_words_cancellable,
};
use nuzky_engine::media::probe;

fn directory(name: &str) -> Result<PathBuf> {
    let src = dunce::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))?;
    let root = src.ancestors().nth(3).context("Missing workspace root")?;
    let dir = root.join("tmp-test/analysis").join(format!("tests-{}", std::process::id())).join(name);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn ffmpeg(args: &[&str], output: &Path) -> Result<()> {
    let result = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(output)
        .output()
        .context("Starting ffmpeg; required for scene integration tests")?;
    ensure!(result.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&result.stderr));
    Ok(())
}

#[test]
fn hard_cuts_detected_and_slow_fade_ignored() -> Result<()> {
    let dir = directory("cuts")?;
    let path = dir.join("cuts.mp4");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=red:s=160x90:r=25:d=1",
            "-f",
            "lavfi",
            "-i",
            "color=blue:s=160x90:r=25:d=1",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=160x90:r=25:d=3",
            "-filter_complex",
            "[2:v]fade=t=out:st=1:d=1[f];[0:v][1:v][f]concat=n=3:v=1:a=0[v]",
            "-map",
            "[v]",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-threads",
            "1",
        ],
        &path,
    )?;
    let asset = probe(&path, "cuts".into())?;
    let cuts = scene_cuts(&asset, SceneParams::default())?;
    assert_eq!(cuts.len(), 2, "{cuts:?}");
    assert_eq!(cuts[0].time_us, 1_000_000);
    assert_eq!(cuts[1].time_us, 2_000_000);
    assert!(cuts.iter().all(|c| c.score >= 0.18 && c.score <= 1.0));
    assert_eq!(scene_cuts(&asset, SceneParams { min_gap_us: 2_000_000, ..Default::default() })?.len(), 1);
    // Reading only around moments finds the same cuts there and none elsewhere.
    let around = |windows: &[[i64; 2]]| {
        let windows: Vec<_> = windows.iter().map(|&[start_us, end_us]| Range { start_us, end_us }).collect();
        scene_cuts_in(&asset, &windows, SceneParams::default(), || false)
    };
    assert_eq!(around(&[[1_500_000, 2_200_000], [800_000, 1_200_000]])?, cuts);
    assert!(around(&[[1_100_000, 1_900_000]])?.is_empty());
    Ok(())
}

/// A camera that starts moving fast at once, as a whip pan, changes every following frame a little: one cut where
/// it starts, not one after every gap while it moves.
#[test]
fn motion_that_sets_in_at_once_is_one_cut() -> Result<()> {
    let dir = directory("onset")?;
    let path = dir.join("onset.mp4");
    let filter = "color=gray:s=160x90:r=25:d=3,format=yuv420p,\
                  geq=lum='if(lt(N,25),40,40+16*abs(mod(N-25,22)-11))':cb=128:cr=128";
    ffmpeg(&["-f", "lavfi", "-i", filter, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-threads", "1"], &path)?;
    let asset = probe(&path, "onset".into())?;
    let cuts = scene_cuts(&asset, SceneParams { threshold: 0.05, ..Default::default() })?;
    assert_eq!(cuts.iter().map(|c| c.time_us).collect::<Vec<_>>(), [1_000_000]);
    Ok(())
}

#[test]
fn rotation_and_variable_frame_pts_are_preserved() -> Result<()> {
    let dir = directory("vfr")?;
    let source = dir.join("source.mp4");
    let path = dir.join("rotated.mp4");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=red:s=160x90:r=25:d=1",
            "-f",
            "lavfi",
            "-i",
            "color=blue:s=160x90:r=25:d=1",
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1:a=0,settb=1/1000000,setpts='if(lt(N,25),N*40000,1080000+(N-25)*80000)'[v]",
            "-map",
            "[v]",
            "-fps_mode",
            "vfr",
            "-enc_time_base",
            "1/1000000",
            "-c:v",
            "libx264",
            "-threads",
            "1",
        ],
        &source,
    )?;
    ffmpeg(&["-display_rotation", "90", "-i", source.to_str().context("UTF-8 path")?, "-c", "copy"], &path)?;
    let asset = probe(&path, "vfr".into())?;
    assert_eq!(asset.rotation % 180, 90);
    let cuts = scene_cuts(&asset, SceneParams::default())?;
    assert_eq!(cuts.len(), 1, "{cuts:?}");
    assert!((cuts[0].time_us - 1_080_000).abs() <= 1, "{cuts:?}");
    let window = Range { start_us: 1_000_000, end_us: 1_200_000 };
    assert_eq!(scene_cuts_in(&asset, &[window], SceneParams::default(), || false)?, cuts);
    Ok(())
}

#[test]
fn pcm_cache_audio_api_and_cli_are_consistent() -> Result<()> {
    let dir = directory("pcm")?;
    let path = dir.join("tone.wav");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1000:sample_rate=48000:duration=3",
            "-af",
            "volume=enable='between(t,1,2)':volume=0",
            "-c:a",
            "pcm_f32le",
        ],
        &path,
    )?;
    let asset = probe(&path, "tone".into())?;
    let levels = loudness(&asset, &dir, 100_000)?;
    assert_eq!(levels.len(), 30);
    assert!((levels[0] + 21.074).abs() < 0.02);
    let gaps = silences(&asset, &dir, SilenceParams::default())?;
    assert_eq!(gaps.len(), 1);
    assert!((gaps[0].start_us - 1_120_000).abs() <= 30_000);
    assert!((gaps[0].end_us - 1_880_000).abs() <= 30_000);
    let binary = env!("CARGO_BIN_EXE_nuzky-analyze");
    let result = Command::new(binary).arg(&path).arg("silences").arg("--cache").arg(&dir).output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(serde_json::from_slice::<Vec<nuzky_analysis::Range>>(&result.stdout)?, gaps);
    let other = dir.join("silence.wav");
    ffmpeg(&["-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo:d=1", "-c:a", "pcm_f32le"], &other)?;
    let result = Command::new(binary).arg(&other).arg("loudness").arg("--cache").arg(&dir).output()?;
    assert!(result.status.success());
    let value: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(value["rms_dbfs"], serde_json::json!(vec![-120.0; 10]));
    // Silence has no integrated loudness (nothing passes the -70 LUFS gate) and a finite peak.
    assert_eq!(
        (&value["integrated_lufs"], &value["true_peak_dbtp"]),
        (&serde_json::json!(null), &serde_json::json!(-120.0))
    );
    let result = Command::new(binary).arg(&path).arg("loudness").arg("--cache").arg(&dir).output()?;
    let value: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    // The tone peaks at 1/8 (-18.06 dBFS) on both channels. The gates drop the silent second but
    // keep blocks partly over it, so it reads -18.77 LUFS, as FFmpeg's ebur128 does.
    let (lufs, peak) = (value["integrated_lufs"].as_f64().unwrap(), value["true_peak_dbtp"].as_f64().unwrap());
    assert!((lufs + 18.77).abs() < 0.1 && (peak + 18.06).abs() < 0.1, "{value}");
    Ok(())
}

#[test]
fn cancelled_analysis_stops_preparing_audio_and_leaves_no_partial_cache() -> Result<()> {
    let dir = directory("cancel")?;
    let path = dir.join("long.wav");
    ffmpeg(&["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=600", "-c:a", "pcm_s16le"], &path)?;
    let asset = probe(&path, "long".into())?;
    let cache = dir.join("cache");
    let polls = AtomicUsize::new(0);
    let stop = || polls.fetch_add(1, Ordering::Relaxed) >= 2;
    let error = silences_cancellable(&asset, &cache, SilenceParams::default(), stop).unwrap_err();
    assert!(format!("{error:#}").contains("CANCELLED"), "{error:#}");
    // Progress is reported every 2 %, so the stop came long before the end of the file.
    assert_eq!(polls.load(Ordering::Relaxed), 3);
    let left: Vec<_> = std::fs::read_dir(cache.join("pcm"))?
        .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?;
    assert!(left.iter().all(|name| name.ends_with(".lock")), "partial cache left behind: {left:?}");
    // The next request prepares the audio from the start.
    assert!(silences(&asset, &cache, SilenceParams::default())?.is_empty());
    Ok(())
}

/// A song that ticks quietly for 30 s, then comes in loud with a kick on every beat and a louder first beat in
/// each bar: the grid an editor cuts to, its bars, where the beat is steady and the drop.
#[test]
fn a_click_track_gives_its_tempo_beats_bars_and_drop() -> Result<()> {
    let dir = directory("beats")?;
    let path = dir.join("click128.wav");
    // A beat every 0.46875 s (128 BPM); bars start at beat 0, 4, 8 and so on, and beat 64 is at 30 s.
    let beat = "mod(t,0.46875)";
    let source = format!(
        "aevalsrc='if(lt(t,30), 0.03*exp(-200*{beat})*sin(2*PI*2000*t), \
         if(eq(mod(floor(t/0.46875+1e-9),4),0),0.9,0.45)*(exp(-20*{beat})*sin(2*PI*60*{beat})+0.3*exp(-300*{beat})*sin(2*PI*3000*{beat})))':s=48000:d=150"
    );
    ffmpeg(&["-f", "lavfi", "-i", &source, "-ac", "2", "-c:a", "pcm_s16le"], &path)?;
    let asset = probe(&path, "click128".into())?;
    let found = beats_cancellable(&asset, &dir, || false)?;
    assert!((found.bpm - 128.0).abs() <= 0.1, "{}", found.bpm);
    let grid = 468_750.0;
    let numbers: Vec<i64> = found.beats_us.iter().map(|&t| (t as f64 / grid).round() as i64).collect();
    let worst = found.beats_us.iter().zip(&numbers).map(|(&t, &n)| (t - n * grid as i64).abs()).max().unwrap();
    assert!(worst <= 10_000, "a beat is {worst} us off the grid");
    // Every beat of the file but the one at 0 s, which the grid places a hair before the start.
    assert!(
        numbers.windows(2).all(|w| w[1] == w[0] + 1) && numbers[0] <= 1 && numbers.last() == Some(&319),
        "{numbers:?}"
    );
    assert_eq!(numbers[found.first_downbeat] % 4, 0, "bars start on beat {}", numbers[found.first_downbeat]);
    assert!((found.steady_from_us - 30_000_000).abs() <= 10_000, "{}", found.steady_from_us);
    assert_eq!(found.drops.len(), 1, "{:?}", found.drops);
    let drop = &found.drops[0];
    assert!((drop.at_us - 30_000_000).abs() <= 10_000 && drop.rise_db >= 6.0, "{drop:?}");
    assert_eq!(found.beats_us[drop.beat], drop.at_us);
    assert_eq!(found.sections.iter().map(|s| s.beat).collect::<Vec<_>>(), [0, drop.beat]);
    // The prepared sound is read every ten seconds of audio for a cancellation.
    let polls = AtomicUsize::new(0);
    let error = beats_cancellable(&asset, &dir, || polls.fetch_add(1, Ordering::Relaxed) >= 2).unwrap_err();
    assert!(format!("{error:#}").starts_with("CANCELLED"), "{error:#}");
    assert_eq!(polls.load(Ordering::Relaxed), 3);
    Ok(())
}

/// A 90 BPM kick is not reported at double time, and noise has no beat to cut to.
#[test]
fn a_slow_beat_keeps_its_tempo_and_noise_has_none() -> Result<()> {
    let dir = directory("beats-90")?;
    let path = dir.join("click90.wav");
    let beat = "mod(t,0.6666666667)";
    ffmpeg(
        &["-f", "lavfi", "-i", &format!("aevalsrc='0.6*exp(-25*{beat})*sin(2*PI*80*{beat})':s=48000:d=60"), "-ac", "2"],
        &path,
    )?;
    let found = beats_cancellable(&probe(&path, "click90".into())?, &dir, || false)?;
    assert!((found.bpm - 90.0).abs() <= 0.1 && found.confidence >= 0.9, "{} {}", found.bpm, found.confidence);
    let noise = dir.join("noise.wav");
    ffmpeg(&["-f", "lavfi", "-i", "anoisesrc=d=60:c=pink:r=48000:a=0.3:seed=7", "-ac", "2"], &noise)?;
    let found = beats_cancellable(&probe(&noise, "noise".into())?, &dir, || false)?;
    assert!(found.confidence < 0.3, "{}", found.confidence);
    Ok(())
}

/// The song built into Nuzky against an independent beat tracker. It fades in without a beat for 18 s.
#[test]
fn the_built_in_song_matches_an_independent_beat_tracker() -> Result<()> {
    let dir = directory("beats-song")?;
    let root = dir.ancestors().nth(4).context("Missing workspace root")?;
    let asset = probe(&root.join("assets/sounds/dancing.ogg"), "dancing".into())?;
    let found = beats_cancellable(&asset, &dir, || false)?;
    let reference: Vec<i64> =
        std::fs::read_to_string(root.join("crates/analysis/tests/data/dancing-beats-librosa.tsv"))?
            .lines()
            .filter(|line| !line.starts_with('#'))
            .map(|line| line.trim().parse::<f64>().map(|s| (s * 1e6).round() as i64))
            .collect::<Result<_, _>>()?;
    let offsets: Vec<i64> = reference
        .iter()
        .filter(|&&r| r > 18_400_000)
        .map(|&r| found.beats_us.iter().map(|&b| b - r).min_by_key(|d| d.abs()).unwrap())
        .collect();
    let matched = offsets.iter().filter(|d| d.abs() <= 50_000).count() as f64 / offsets.len() as f64;
    let mut sizes: Vec<i64> = offsets.iter().map(|d| d.abs()).collect();
    sizes.sort();
    eprintln!(
        "bpm {} confidence {}: {:.1} % of {} reference beats within 50 ms, median {} us, max {} us, steady from {} us, drops {:?}",
        found.bpm,
        found.confidence,
        100.0 * matched,
        offsets.len(),
        sizes[sizes.len() / 2],
        sizes.last().unwrap(),
        found.steady_from_us,
        found.drops
    );
    assert!((found.bpm - 99.0).abs() <= 0.99 && found.confidence >= 0.5, "{} {}", found.bpm, found.confidence);
    assert!(matched >= 0.9, "{matched}");
    let beat = (60e6 / found.bpm) as i64;
    assert!((found.steady_from_us - 18_300_000).abs() <= beat, "{}", found.steady_from_us);
    let drop = found.drops.first().context("no drop where the beat comes in")?;
    assert!((drop.at_us - 18_300_000).abs() <= beat, "{drop:?}");
    // It fades out in two steps: the bar from 127.03 s is at -36 dBFS after -18 before it, the next at -45. The quiet
    // ending starts with the first.
    let [.., before, ending] = found.sections.as_slice() else { panic!("{:?}", found.sections) };
    assert!(
        (ending.at_us - 127_030_000).abs() <= beat / 2 && ending.level_db <= before.level_db - 12.0,
        "{:?}",
        found.sections
    );
    Ok(())
}

#[test]
#[ignore = "requires tmp-test/speech.wav and installed small + Silero models in tmp-test/xdg/data"]
fn cancelling_speech_recognition_half_way_reports_a_cancellation() -> Result<()> {
    let dir = directory("speech-cancel")?;
    let root = dir.ancestors().nth(4).context("Missing workspace root")?;
    let models = root.join("tmp-test/xdg/data/nuzky/models");
    let path = root.join("tmp-test/speech.wav");
    let asset = probe(&path, "speech".into())?;
    let cache = dir.join("cache");
    let recognise = |cancelled: &dyn Fn() -> bool| {
        transcribe_words_cancellable(
            AudioSource::Asset { asset: &asset, cache: &cache },
            &models.join("ggml-small.bin"),
            &models.join("ggml-silero-v5.1.2.bin"),
            // A fixed language skips detection, whose encoder pass whisper.cpp cannot stop.
            "en",
            cancelled,
            |_| {},
        )
    };
    let polls = AtomicUsize::new(0);
    let words = recognise(&|| polls.fetch_add(1, Ordering::Relaxed) == usize::MAX)?.words.len();
    let total = polls.load(Ordering::Relaxed);
    assert!(words > 5, "{words} words");
    let polls = AtomicUsize::new(0);
    let error = recognise(&|| polls.fetch_add(1, Ordering::Relaxed) >= total / 2).unwrap_err();
    assert!(format!("{error:#}").starts_with("CANCELLED"), "{error:#}");
    Ok(())
}

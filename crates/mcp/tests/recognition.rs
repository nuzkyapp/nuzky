use std::{path::Path, process::Command, sync::atomic::AtomicBool};

use anyhow::Result;
use nuzky_mcp::transcript::{Stage, recognise};
use nuzky_session::transcripts::TranscriptStore;

#[test]
#[ignore = "needs krysar-cs.wav and speech models from scripts/fixtures.sh; run with scripts/isolated.sh"]
fn recognition_keeps_both_attempts_of_a_restarted_sentence() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root.join(format!("tmp-test/recognition-retakes-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let media = dir.join("retake.wav");
    let cuts = [(0.0, 8.93), (10.70, 10.85), (7.88, 12.52), (10.70, 10.85), (11.36, 16.0)];
    let mut filter = String::new();
    for (i, (start, end)) in cuts.iter().enumerate() {
        filter.push_str(&format!("[0:a]atrim=start={start}:end={end},asetpts=PTS-STARTPTS[a{i}];"));
    }
    filter.push_str("[a0][a1][a2][a3][a4]concat=n=5:v=0:a=1[out]");
    assert!(
        Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(root.join("tmp-test/krysar-cs.wav"))
            .args(["-filter_complex", &filter, "-map", "[out]", "-ar", "16000", "-ac", "1"])
            .arg(&media)
            .status()?
            .success()
    );
    let asset = nuzky_engine::media::probe(&media, "retake".into())?;
    let models = root.join("tmp-test/xdg/data/nuzky/models");
    let store = TranscriptStore::at(dir.join("transcripts"))?;
    let mut progress = 0.0;
    let record = recognise(
        &store,
        &asset,
        &dir.join("cache"),
        "large-v3-turbo-q5_0",
        &(models.join("ggml-large-v3-turbo-q5_0.bin"), models.join(nuzky_analysis::VAD_MODEL)),
        "cs",
        &AtomicBool::new(false),
        |stage| {
            if let Stage::Aligning(part) = stage {
                assert!(part >= progress && part <= 1.0, "progress went from {progress} to {part}");
                progress = part;
            }
        },
    )?;
    assert!(record.alignment.is_some());
    assert!(record.words.windows(2).all(|w| w[0].end_us <= w[1].start_us));
    assert_eq!(store.get(&asset)?, Some(record.clone()));
    for (start, end, expected) in
        [(7.85, 8.93, vec!["hled|led", "s", "důvěř"]), (12.50, 13.72, vec!["a", "krys|křes", "stále"])]
    {
        let words: Vec<_> = record
            .words
            .iter()
            .filter(|w| w.start_us >= (start * 1e6) as i64 - 40_000 && w.end_us <= (end * 1e6) as i64 + 40_000)
            .map(|w| w.text.to_lowercase())
            .collect();
        for prefix in expected {
            assert!(
                words.iter().any(|w| prefix.split('|').any(|p| w.starts_with(p))),
                "missing {prefix} in {start}-{end}: {words:?}"
            );
        }
    }
    for (prefix, reference) in [("hled|led", 9_105_000), ("a", 13_910_000)] {
        let word = record
            .words
            .iter()
            .filter(|w| prefix.split('|').any(|p| w.text.to_lowercase().starts_with(p)))
            .min_by_key(|w| (w.start_us - reference).abs())
            .expect("missing kept attempt");
        assert!((word.start_us - reference).abs() <= 40_000, "kept attempt {word:?}, reference {reference}");
    }
    assert_eq!(progress, 1.0);
    let cancel = AtomicBool::new(false);
    let mut asked = None;
    let stopped = recognise(
        &store,
        &asset,
        &dir.join("cache"),
        "large-v3-turbo-q5_0",
        &(models.join("ggml-large-v3-turbo-q5_0.bin"), models.join(nuzky_analysis::VAD_MODEL)),
        "cs",
        &cancel,
        |stage| {
            if let Stage::Aligning(p) = stage
                && p > 0.8
                && p < 0.9
                && asked.is_none()
            {
                asked = Some(std::time::Instant::now());
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        },
    )
    .expect_err("Stop during re-recognition was ignored");
    assert!(stopped.to_string().starts_with("CANCELLED"), "{stopped:#}");
    let elapsed = asked.expect("never reached re-recognition").elapsed();
    assert!(elapsed < std::time::Duration::from_secs(3), "Stop took {elapsed:?}");
    assert_eq!(store.get(&asset)?, Some(record));
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

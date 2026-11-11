//! The scene model on the sample video built into Nuzky (assets/sample, CC BY): a woman on a chair with a dog in
//! her lap, a laptop and a cup on the desk. Run as AGENTS.md says:
//! `scripts/isolated.sh cargo test --workspace --test scene -- --ignored`.
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use nuzky_engine::media::probe;
use nuzky_vision::florence::Tokenizer;
use nuzky_vision::models::FLORENCE_TOKENIZER;
use nuzky_vision::scene::{describe, track};

fn models() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME").expect("run with scripts/isolated.sh");
    PathBuf::from(data).join("nuzky/models")
}

fn sample() -> nuzky_engine::model::Asset {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/sample/spotlight-effect.mp4");
    probe(&dunce::canonicalize(path).unwrap(), "sample".into()).unwrap()
}

/// Token ids from Hugging Face's `tokenizers` 0.23.3 with the pinned tokenizer.json.
#[test]
#[ignore = "Requires the scene model from scripts/fixtures.sh; run with scripts/isolated.sh"]
fn prompts_encode_as_hugging_face_encodes_them() {
    let tokens = Tokenizer::load(&FLORENCE_TOKENIZER.path(&models())).unwrap();
    let cases: &[(&str, &[i64])] = &[
        (
            "Describe in detail what is shown in the image.",
            &[0, 47066, 21700, 11, 4617, 99, 16, 2343, 11, 5, 2274, 4, 2],
        ),
        ("Locate dog in the image.", &[0, 574, 22486, 2335, 11, 5, 2274, 4, 2]),
        ("Locate the red car  in the image.", &[0, 574, 22486, 5, 1275, 512, 1437, 11, 5, 2274, 4, 2]),
        (
            "Locate Žluťoučký kůň's 3D-printed   mug!! in the image.",
            &[
                0, 574, 22486, 2742, 10809, 6487, 1277, 8210, 1438, 4955, 330, 3849, 10809, 449, 1277, 10965, 1277,
                23133, 18, 155, 495, 12, 36818, 1437, 1437, 20148, 12846, 11, 5, 2274, 4, 2,
            ],
        ),
        (
            "Locate  dogs\tand\ncats  in the image.",
            &[0, 574, 22486, 1437, 3678, 50117, 463, 50118, 19295, 1437, 11, 5, 2274, 4, 2],
        ),
    ];
    for (text, ids) in cases {
        assert_eq!(tokens.encode(text), *ids, "{text:?}");
    }
    let decoded = tokens.decode(&[0, 2335, 50572, 50882, 50963, 51267, 2]);
    assert_eq!((decoded.text.trim(), decoded.locs.as_slice()), ("dog", &[303, 613, 694, 998][..]));
}

#[test]
#[ignore = "Requires the scene model from scripts/fixtures.sh; run with scripts/isolated.sh"]
fn the_sample_is_described_and_its_dog_followed_but_no_elephant() {
    let (asset, cancel) = (sample(), AtomicBool::new(false));
    let start = Instant::now();
    let text = describe(&asset, &[10_000_000], &models(), &cancel, &mut |_| {}).unwrap();
    eprintln!("described in {:?}: {text:?}", start.elapsed());
    let text = text[0].to_lowercase();
    assert!(text.contains("dog") && (text.contains("woman") || text.contains("person")), "{text}");
    assert!(text.contains("laptop"), "{text}");

    let times = [2_000_000, 10_000_000, 25_000_000, 40_000_000];
    let start = Instant::now();
    let dog = track(&asset, &times, "dog", &models(), &cancel, &mut |_| {}).unwrap();
    eprintln!("followed in {:?}: {dog:?}", start.elapsed());
    assert_eq!(dog.len(), times.len());
    for seen in &dog {
        assert!(seen.matches, "{seen:?}");
        // In her lap: the lower middle of the picture.
        let [x, y, w, h] = seen.region;
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        assert!((0.35..0.65).contains(&cx) && (0.6..0.95).contains(&cy), "{seen:?}");
    }
    assert!(dog.iter().filter(|seen| seen.seen == "dog").count() >= 3, "{dog:?}");
    // Asked for what is not there, the model still finds a region, and names it otherwise.
    for words in ["elephant", "red car", "a guitar"] {
        let absent = track(&asset, &times[..2], words, &models(), &cancel, &mut |_| {}).unwrap();
        eprintln!("{words}: {absent:?}");
        assert!(absent.iter().all(|seen| !seen.matches), "{words}: {absent:?}");
    }
}

#[test]
#[ignore = "Requires the scene model from scripts/fixtures.sh; run with scripts/isolated.sh"]
fn stop_ends_a_track_at_once() {
    let (asset, cancel) = (sample(), AtomicBool::new(true));
    let error = track(&asset, &[1_000_000], "dog", &models(), &cancel, &mut |_| {}).unwrap_err();
    assert!(error.to_string().starts_with("CANCELLED"), "{error:#}");
}

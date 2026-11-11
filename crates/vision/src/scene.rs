//! What a file shows, in words, and where in its picture the thing some words name is over time, with the scene
//! model (`florence`) on the file's own picture (`source`).
use std::path::Path;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use nuzky_engine::model::Asset;
use serde::Serialize;

use crate::florence::{Florence, Region};
use crate::models::{self, SCENE};
use crate::nets::iou;
use crate::source::Source;

/// How much less likely, in nats a token, the words asked for may be than the model's own name for the region it
/// found and still name it: "dog" for a dog it called "cat" is about 1 less likely, "elephant" about 7.
const NAME_MARGIN: f32 = 2.0;

/// The long side the file is drawn at for the model, which squeezes every picture to 768 × 768.
const LOOK_SIDE: u32 = 768;

/// A description of the picture at each of `times`, in the file's own microseconds.
pub fn describe(
    asset: &Asset,
    times: &[i64],
    models_dir: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<Vec<String>> {
    models::require(SCENE, models_dir)?;
    let mut source = Source::new(asset, LOOK_SIDE)?;
    let mut florence = Florence::load(models_dir)?;
    let mut out = Vec::with_capacity(times.len());
    for (i, &t) in times.iter().enumerate() {
        progress(i as f32 / times.len() as f32);
        let look = florence.look(&source.frame(t)?, cancel)?;
        out.push(florence.describe(&look, cancel)?);
    }
    Ok(out)
}

/// What the model saw at one time of a file when asked for the words.
#[derive(Clone, Debug, Serialize)]
pub struct Seen {
    pub t_us: i64,
    /// Left, top, width and height as fractions of the upright picture, rounded to 4 places.
    #[serde(rename = "box")]
    pub region: [f32; 4],
    /// What the model calls that region.
    pub seen: String,
    /// Whether the words name that region about as well as `seen` does, or `seen` says each of them.
    pub matches: bool,
    /// Other regions it found for the words at the same time.
    pub others: usize,
}

/// Where the thing `words` name is at each of `times` of the file. Of several regions the one nearest the region
/// followed before is taken, so the track stays on one thing.
pub fn track(
    asset: &Asset,
    times: &[i64],
    words: &str,
    models_dir: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<Vec<Seen>> {
    models::require(SCENE, models_dir)?;
    let mut source = Source::new(asset, LOOK_SIDE)?;
    let mut florence = Florence::load(models_dir)?;
    let mut out: Vec<Seen> = Vec::with_capacity(times.len());
    let mut followed: Option<Region> = None;
    for (i, &t) in times.iter().enumerate() {
        progress(i as f32 / times.len() as f32);
        let look = florence.look(&source.frame(t)?, cancel)?;
        let regions = florence.locate(&look, words, cancel)?;
        let Some(&region) = (match followed {
            Some(before) => regions.iter().max_by(|a, b| closeness(before, **a).total_cmp(&closeness(before, **b))),
            None => regions.first(),
        }) else {
            continue;
        };
        let (seen, close) = florence.name(&look, region, words, NAME_MARGIN, cancel)?;
        let matches = close || names_the_same(words, &seen);
        if matches {
            followed = Some(region);
        }
        out.push(Seen {
            t_us: t,
            region: region.map(|v| (v * 10_000.0).round() / 10_000.0),
            seen,
            matches,
            others: regions.len() - 1,
        });
    }
    progress(1.0);
    Ok(out)
}

/// Overlap first, then how near the centres are, so a region that moved off the last one still wins over one
/// across the picture.
fn closeness(a: Region, b: Region) -> f32 {
    let centre = |r: Region| [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0];
    let ([ax, ay], [bx, by]) = (centre(a), centre(b));
    iou(a, b) - ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt()
}

/// Words too common to tell what was asked for.
const COMMON: &[&str] = &["the", "a", "an", "of", "with", "and", "in", "on", "his", "her", "its", "their", "my"];

/// Whether the model's name for a region says every word asked for, as a whole word or its plural: "dog" in "dogs",
/// never "man" in "woman" or "red car" for a "blue car". Other names, such as "puppy" for "dog", count only when
/// the model finds the words about as likely; the agent reads `seen`.
pub fn names_the_same(words: &str, seen: &str) -> bool {
    let split = |text: &str| -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .map(str::to_lowercase)
            .filter(|w| !w.is_empty() && w != "s" && !COMMON.contains(&w.as_str()))
            .collect()
    };
    let plural = |one: &str, other: &str| other.strip_prefix(one).is_some_and(|end| end == "s" || end == "es");
    let named = split(seen);
    let asked = split(words);
    !asked.is_empty() && asked.iter().all(|a| named.iter().any(|n| a == n || plural(a, n) || plural(n, a)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_matches_words_it_shares_and_not_a_synonym() {
        assert!(names_the_same("dog", "dog"));
        assert!(names_the_same("the dogs", "dog"));
        assert!(names_the_same("red car", "a red car"));
        assert!(names_the_same("the man's face", "man face"));
        assert!(!names_the_same("elephant", "dog"));
        assert!(!names_the_same("puppy", "dog"));
        assert!(!names_the_same("man", "woman"));
        assert!(!names_the_same("cat", "cattle"));
        assert!(!names_the_same("red car", "blue car"));
        assert!(!names_the_same("a", "a"));
    }

    #[test]
    fn the_region_followed_before_wins_over_one_across_the_picture() {
        let before = [0.1, 0.1, 0.2, 0.2];
        let near = [0.15, 0.12, 0.2, 0.2];
        let far = [0.7, 0.7, 0.2, 0.2];
        assert!(closeness(before, near) > closeness(before, far));
    }
}

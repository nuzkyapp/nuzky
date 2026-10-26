use std::{ops::Range, path::Path};

use anyhow::Result;
use nuzky_engine::model::Asset;
use unicode_normalization::UnicodeNormalization;

use crate::{
    Aligner, Transcript, Word,
    audio::{open_pcm, speech_rate},
    boundaries::refine_words,
    speech,
};

/// Measures a fresh transcript, retrying only VAD regions with speech missing from Whisper's text.
#[allow(clippy::too_many_arguments)]
pub fn align_transcript(
    transcript: &mut Transcript,
    regions: &[(usize, usize)],
    asset: &Asset,
    cache: &Path,
    aligner: &Aligner,
    model: &Path,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(f32),
) -> Result<bool> {
    let pcm = open_pcm(asset, cache, cancelled)?;
    let audio = speech_rate(pcm.samples());
    let us = |sample: usize| sample as i64 * 1_000_000 / 16_000;
    let ranges: Vec<_> = regions
        .iter()
        .map(|&(start, end)| {
            let at = |t| transcript.words.partition_point(|w| (w.start_us + w.end_us) / 2 < t);
            at(us(start))..at(us(end))
        })
        .collect();
    let mut heard = Vec::new();
    let placed =
        aligner.align_heard(&mut transcript.words, &audio, cancelled, &mut |p| progress(p * 0.8), Some(&mut heard))?;
    anyhow::ensure!(!cancelled(), "CANCELLED: transcription cancelled");
    refine_words(&mut transcript.words, pcm.samples(), &placed);
    let mut measured = placed.contains(&true);
    let mut candidates = Vec::new();
    for (&region, range) in regions.iter().zip(ranges) {
        let heard: Vec<_> = heard
            .iter()
            .filter(|w| {
                let mid = (w.start_us + w.end_us) / 2;
                us(region.0) - 200_000 <= mid && mid < us(region.1) + 200_000
            })
            .cloned()
            .collect();
        // Forced alignment can move a missing restart's words into the next region. Compare where
        // they landed, but replace the words owned by the original recognition region.
        let at = |t| transcript.words.partition_point(|w| (w.start_us + w.end_us) / 2 < t);
        let (score, missing) =
            mismatch(&transcript.words[at(us(region.0) - 200_000)..at(us(region.1) + 200_000)], &heard);
        if !missing.is_empty() {
            candidates.push((region, range, heard, score, missing));
        }
    }
    let mut context = None;
    let count = candidates.len();
    for (i, (region, range, heard, score, missing)) in candidates.into_iter().rev().enumerate() {
        anyhow::ensure!(!cancelled(), "CANCELLED: transcription cancelled");
        progress(0.8 + 0.2 * i as f32 / count as f32);
        let mut cuts = vec![region.0];
        for pair in heard.windows(2) {
            let mid = (pair[0].end_us + pair[1].start_us) / 2;
            if pair[1].start_us - pair[0].end_us >= 250_000
                && missing
                    .iter()
                    .any(|r| heard[r.start].start_us - 1_500_000 <= mid && mid <= heard[r.end - 1].end_us + 1_500_000)
            {
                let sample = (mid * 16_000 / 1_000_000) as usize;
                if region.0 < sample && sample < region.1 {
                    cuts.push(sample);
                }
            }
        }
        if cuts.len() > 1 {
            cuts.push(region.1);
            let pieces: Vec<_> = cuts.windows(2).map(|p| (p[0], p[1])).collect();
            if context.is_none() {
                context = Some(speech::speech_context(model)?);
            }
            let mut replacement = speech::recognise_again(
                context.as_ref().unwrap(),
                &audio,
                &pieces,
                &transcript.language,
                cancelled,
                &mut |p| progress(0.8 + 0.2 * (i as f32 + p * 0.5) / count as f32),
            )?;
            let (new_score, _) = mismatch(&replacement.words, &heard);
            if new_score + 0.01 < score {
                let placed = aligner.align(&mut replacement.words, &audio, cancelled, &mut |p| {
                    progress(0.8 + 0.2 * (i as f32 + 0.5 + p * 0.5) / count as f32)
                })?;
                anyhow::ensure!(!cancelled(), "CANCELLED: transcription cancelled");
                refine_words(&mut replacement.words, pcm.samples(), &placed);
                measured |= placed.contains(&true);
                transcript.words.splice(range, replacement.words);
                transcript.segments.retain(|s| {
                    let mid = (s.start_us + s.end_us) / 2;
                    mid < us(region.0) || mid >= us(region.1)
                });
                transcript.segments.extend(replacement.segments);
                transcript.segments.sort_by_key(|s| s.start_us);
            }
        }
    }
    progress(1.0);
    Ok(measured)
}

fn normalise(text: &str) -> String {
    if text.chars().any(|c| c.is_ascii_digit()) {
        return "#".into();
    }
    text.nfd().flat_map(char::to_lowercase).filter(char::is_ascii_alphabetic).collect()
}

fn number(word: &str) -> bool {
    [
        "nul", "jedn", "jeden", "dva", "dve", "tri", "ctyr", "pet", "sest", "sedm", "osm", "devet", "deset", "nact",
        "cet", "desat", "sto", "set", "tisic", "milion", "miliard", "procent",
    ]
    .iter()
    .any(|part| word.contains(part))
}

fn distance(a: &str, b: &str) -> usize {
    let mut row: Vec<_> = (0..=b.len()).collect();
    for (i, a) in a.bytes().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, b) in b.bytes().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (old + 1).min(row[j] + 1).min(diagonal + usize::from(a != b));
            diagonal = old;
        }
    }
    row[b.len()]
}

/// A digit can cover several spoken number words; split and joined CTC spellings are not omissions.
fn mismatch(words: &[Word], heard: &[Word]) -> (f64, Vec<Range<usize>>) {
    let words: Vec<_> = words.iter().map(|w| normalise(&w.text)).filter(|w| !w.is_empty()).collect();
    let heard: Vec<_> = heard.iter().map(|w| normalise(&w.text)).collect();
    let (n, m) = (words.len(), heard.len());
    let mut costs = vec![vec![(f64::INFINITY, b' '); m + 1]; n + 1];
    costs[0][0].0 = 0.0;
    for i in 0..=n {
        for j in 0..=m {
            let (cost, previous) = costs[i][j];
            let mut offer = |i: usize, j: usize, extra: f64, op| {
                if cost + extra < costs[i][j].0 {
                    costs[i][j] = (cost + extra, op);
                }
            };
            if i < n && j < m {
                let (w, h) = (&words[i], &heard[j]);
                let sub = if w == "#" {
                    if number(h) { 0.2 } else { 1.0 }
                } else {
                    distance(w, h) as f64 / w.len().max(h.len()).max(1) as f64
                };
                offer(i + 1, j + 1, sub, b'M');
            }
            if j < m {
                let absorb = i > 0 && words[i - 1] == "#" && matches!(previous, b'M' | b'A') && number(&heard[j]);
                offer(i, j + 1, if absorb { 0.1 } else { 0.6 }, if absorb { b'A' } else { b'I' });
            }
            if i < n {
                offer(i + 1, j, 0.6, b'D');
            }
            // A piece that is the word itself is that word, not half of a split spelling: otherwise "tak" + "a" of a
            // restarted "a tak a tak" would pass for one "tak".
            if i < n && j + 1 < m && words[i] != "#" && heard[j] != words[i] && heard[j + 1] != words[i] {
                let joined = distance(&words[i], &(heard[j].clone() + &heard[j + 1]));
                offer(i + 1, j + 2, joined as f64 / words[i].len() as f64 + 0.05, b'S');
            }
            if i + 1 < n
                && j < m
                && words[i] != "#"
                && words[i + 1] != "#"
                && words[i] != heard[j]
                && words[i + 1] != heard[j]
            {
                let joined = distance(&(words[i].clone() + &words[i + 1]), &heard[j]);
                offer(i + 2, j + 1, joined as f64 / heard[j].len().max(1) as f64 + 0.05, b'J');
            }
        }
    }
    let mut missing = vec![false; m];
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        match costs[i][j].1 {
            b'M' => {
                i -= 1;
                j -= 1;
            }
            b'A' => j -= 1,
            b'I' => {
                missing[j - 1] = true;
                j -= 1;
            }
            b'S' => {
                i -= 1;
                j -= 2;
            }
            b'J' => {
                i -= 2;
                j -= 1;
            }
            _ => i -= 1,
        }
    }
    let mut runs = Vec::new();
    let mut j = 0;
    while j < m {
        if !missing[j] {
            j += 1;
            continue;
        }
        let start = j;
        while j < m && missing[j] {
            j += 1;
        }
        if j - start >= 2 || heard[start..j].iter().map(String::len).sum::<usize>() >= 5 {
            runs.push(start..j);
        }
    }
    (costs[n][m].0, runs)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn words(text: &str) -> Vec<Word> {
        text.split_whitespace()
            .map(|text| Word { start_us: 0, end_us: 0, text: text.into(), probability: 1.0 })
            .collect()
    }
    #[test]
    fn restarts_match_fuzzy_words_numbers_and_joined_spellings() {
        for (a, b) in [
            ("Stáhlo přes 13 000 lidí.", "stálo přes třináct tisíc lidí"),
            ("mobilní aplikaci", "mobil ní aplikacy"),
            ("A nebojte", "anebojte"),
            ("nůžky.app", "nůšky ap"),
        ] {
            assert!(mismatch(&words(a), &words(b)).1.is_empty(), "{a}: {b}");
        }
    }
    #[test]
    fn restarts_require_missing_words_and_a_smaller_mismatch() {
        let heard = words("hleděla s důvěřivým hleděla s důvěřivým úsměvem");
        let before = mismatch(&words("Hleděla s důvěřivým úsměvem."), &heard);
        assert!(!before.1.is_empty());
        assert!(mismatch(&heard, &heard).0 < before.0);
        assert!(mismatch(&words("Hleděla s úsměvem."), &heard).0 >= before.0);
        assert!(mismatch(&words("a krysař"), &words("a krysař a")).1.is_empty());
        assert!(!mismatch(&words("a krysař"), &words("a krysař stále")).1.is_empty());
        assert!(!mismatch(&words("A tak mám auto"), &words("a tak a tak mám auto")).1.is_empty());
    }
}

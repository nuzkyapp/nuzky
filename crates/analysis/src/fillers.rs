use crate::{Range, Transcript, Word};

/// Conservative lexical proposals, not semantic judgements. cs: ehm/hmm/eee;
/// en: um/uh/erm. Ambiguous cs jakoby/prostě and en like/you know require pauses
/// (>=150 ms) or comma boundaries on BOTH sides. Immediate repeated words within
/// 250 ms propose removing earlier copies, keeping the final one. Intentional
/// repetition is indistinguishable from stuttering here: review before applying.
/// Unsupported languages return no proposals.
pub fn filler_words(transcript: &Transcript, language: &str) -> Vec<Range> {
    let language = if language == "auto" { transcript.language.as_str() } else { language };
    if !matches!(language, "cs" | "en") {
        return Vec::new();
    }
    let words = &transcript.words;
    let normalized: Vec<String> = words.iter().map(|w| normalize(&w.text)).collect();
    let mut ranges = Vec::new();
    for (index, word) in words.iter().enumerate() {
        let token = normalized[index].as_str();
        let simple = match language {
            "cs" => matches!(token, "ehm" | "hmm" | "eee"),
            _ => matches!(token, "um" | "uh" | "erm"),
        };
        let ambiguous = matches!((language, token), ("cs", "jakoby" | "prostě") | ("en", "like"));
        if simple || (ambiguous && isolated(words, index, index)) {
            ranges.push(Range { start_us: word.start_us, end_us: word.end_us });
        }
        if language == "en"
            && token == "you"
            && normalized.get(index + 1).is_some_and(|w| w == "know")
            && words[index + 1].start_us.saturating_sub(word.end_us) <= 250_000
            && isolated(words, index, index + 1)
        {
            ranges.push(Range { start_us: word.start_us, end_us: words[index + 1].end_us });
        }
        if !token.is_empty()
            && !word.text.ends_with(['.', '?', '!', ',', ';', ':'])
            && let Some(next) = words.get(index + 1)
        {
            let gap = next.start_us.saturating_sub(word.end_us);
            if normalized[index + 1] == token && (0..=250_000).contains(&gap) {
                ranges.push(Range { start_us: word.start_us, end_us: word.end_us });
            }
        }
    }
    ranges.retain(|r| r.start_us >= 0 && r.end_us > r.start_us);
    ranges.sort_unstable_by_key(|r| (r.start_us, r.end_us));
    let mut merged: Vec<Range> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start_us <= last.end_us => last.end_us = last.end_us.max(range.end_us),
            _ => merged.push(range),
        }
    }
    merged
}

/// Waveform peaks are 20 ms each (`nuzky_engine::audio::PEAKS_PER_SECOND`).
const PEAK_US: i64 = 20_000;
/// Voice shorter than this between two words is left alone: a click, a breath or a word recognition missed.
pub const HESITATION_US: i64 = 150_000;
/// This much of a gap next to each word belongs to the word: measured word edges are off by a frame or so.
const WORD_EDGE_US: i64 = 30_000;
/// Speech must be this much louder than the room, or voice cannot be told from it (music, a loud room).
const MIN_RANGE_DB: f32 = 20.0;

/// Voice in the gaps between words that recognition did not write, such as an "eee": for each of
/// `gaps`, source ranges between two words, the sound from the first to the last stretch of at least
/// HESITATION_US louder than halfway between the room and speech, or None. Room and speech are the
/// 10th and 90th percentile of `peaks`, the waveform peaks of the whole file from its start. A
/// stretch that runs into either word is that word's own sound, so quiet must part it from both.
pub fn hesitations(peaks: &[u8], gaps: &[Range]) -> Vec<Option<Range>> {
    let db = |peak: u8| 20.0 * (f32::from(peak).max(0.5) / 255.0).log10();
    let mut counts = [0usize; 256];
    for &p in peaks {
        counts[usize::from(p)] += 1;
    }
    let percentile = |share: usize| {
        let mut seen = 0;
        (0..=255u8).find(|&p| {
            seen += counts[usize::from(p)];
            seen * 100 >= peaks.len() * share
        })
    };
    let (Some(room), Some(speech)) = (percentile(10), percentile(90)) else { return vec![None; gaps.len()] };
    if db(speech) - db(room) < MIN_RANGE_DB {
        return vec![None; gaps.len()];
    }
    let gate = (db(room) + db(speech)) / 2.0;
    let loud = |block: i64| peaks.get(block as usize).is_some_and(|&p| db(p) > gate);
    gaps.iter()
        .map(|gap| {
            // Whole peaks inside the gap, past the word edges.
            let first = (gap.start_us + WORD_EDGE_US + PEAK_US - 1).div_euclid(PEAK_US);
            let end = (gap.end_us - WORD_EDGE_US).div_euclid(PEAK_US);
            let mut runs: Vec<(i64, i64)> = Vec::new();
            for block in (first..end).filter(|&b| loud(b)) {
                match runs.last_mut() {
                    // One quiet peak inside voice does not split it.
                    Some(run) if block <= run.1 + 1 => run.1 = block + 1,
                    _ => runs.push((block, block + 1)),
                }
            }
            let voiced: Vec<_> =
                runs.into_iter().filter(|&(a, b)| a > first && b < end && (b - a) * PEAK_US >= HESITATION_US).collect();
            Some(Range { start_us: voiced.first()?.0 * PEAK_US, end_us: voiced.last()?.1 * PEAK_US })
        })
        .collect()
}

fn normalize(text: &str) -> String {
    text.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn isolated(words: &[Word], first: usize, last: usize) -> bool {
    let before = first == 0
        || words[first].start_us.saturating_sub(words[first - 1].end_us) >= 150_000
        || words[first - 1].text.ends_with(',');
    let after = last + 1 == words.len()
        || words[last + 1].start_us.saturating_sub(words[last].end_us) >= 150_000
        || words[last].text.ends_with(',');
    before && after
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transcript(text: &str) -> Transcript {
        Transcript {
            language: "cs".into(),
            segments: vec![],
            words: text
                .split_whitespace()
                .enumerate()
                .map(|(i, text)| Word {
                    start_us: i as i64 * 200_000,
                    end_us: i as i64 * 200_000 + 180_000,
                    text: text.into(),
                    probability: 1.0,
                })
                .collect(),
        }
    }

    #[test]
    fn conservative_cs_and_stutter_keep_last_copy() {
        let t = transcript("Ehm je je je to prostě jednoduché jakoby nic hmm");
        let ranges = filler_words(&t, "cs");
        assert_eq!(ranges.iter().map(|r| r.start_us).collect::<Vec<_>>(), vec![0, 200_000, 400_000, 1_800_000]);
        assert_eq!(
            filler_words(&transcript("To, prostě, funguje"), "cs"),
            vec![Range { start_us: 200_000, end_us: 380_000 }]
        );
        assert!(filler_words(&transcript("Je. Je to tak"), "cs").is_empty());
    }

    /// Peaks of a file whose room is at 2 and speech at 120, with voice loud at the 20 ms peaks `from..to`.
    fn peaks(voice: &[(usize, usize)]) -> Vec<u8> {
        let mut peaks = vec![2u8; 200];
        peaks[..30].fill(120);
        for &(from, to) in voice {
            peaks[from..to].fill(110);
        }
        peaks
    }

    fn gap(start_us: i64, end_us: i64) -> Range {
        Range { start_us, end_us }
    }

    #[test]
    fn a_hesitation_parted_from_both_words_by_quiet_is_found_whole() {
        // An "eee" at 1.0-1.3 s in a gap at 0.8-1.5 s.
        let found = hesitations(&peaks(&[(50, 65)]), &[gap(800_000, 1_500_000)]);
        assert_eq!(found, [Some(gap(1_000_000, 1_300_000))]);
        // One quiet peak inside it does not split it; two stretches in one gap are one hesitation.
        let found = hesitations(&peaks(&[(50, 57), (58, 65), (68, 76)]), &[gap(800_000, 1_700_000)]);
        assert_eq!(found, [Some(gap(1_000_000, 1_520_000))]);
    }

    #[test]
    fn sound_running_into_a_word_is_the_word() {
        // The previous word's tail goes on past its measured end into the gap.
        assert_eq!(hesitations(&peaks(&[(40, 60)]), &[gap(800_000, 1_500_000)]), [None]);
        // Its tail, quiet, then an "eee": only the "eee".
        let found = hesitations(&peaks(&[(40, 45), (50, 60)]), &[gap(800_000, 1_500_000)]);
        assert_eq!(found, [Some(gap(1_000_000, 1_200_000))]);
        // Voice up to the next word.
        assert_eq!(hesitations(&peaks(&[(60, 75)]), &[gap(800_000, 1_500_000)]), [None]);
    }

    /// The gap's edges: WORD_EDGE_US beside each word is the word's, and a peak must lie wholly inside.
    #[test]
    fn the_gap_edges_belong_to_the_words() {
        // In a gap at 0.80-1.20 s the words own 0.80-0.83 and 1.17-1.20 s, so the gap's peaks are
        // those wholly inside 0.83-1.17 s, 0.84-1.16 s, and its first and last must be quiet.
        let edge = gap(800_000, 1_200_000);
        assert_eq!(hesitations(&peaks(&[(42, 57)]), &[edge]), [None]);
        assert_eq!(hesitations(&peaks(&[(43, 58)]), &[edge]), [None]);
        assert_eq!(hesitations(&peaks(&[(43, 57)]), &[edge]), [Some(gap(860_000, 1_140_000))]);
        // A word ending at 0.81 s leaves the peak at 0.84 s to the gap; a microsecond later it is the word's.
        assert_eq!(hesitations(&peaks(&[(43, 57)]), &[gap(810_000, 1_200_000)]), [Some(gap(860_000, 1_140_000))]);
        assert_eq!(hesitations(&peaks(&[(43, 57)]), &[gap(810_001, 1_200_000)]), [None]);
        // Seven peaks of voice, 140 ms, are not HESITATION_US; eight, 160 ms, are.
        assert_eq!(hesitations(&peaks(&[(50, 57)]), &[gap(800_000, 1_500_000)]), [None]);
        assert_eq!(hesitations(&peaks(&[(50, 58)]), &[gap(800_000, 1_500_000)]), [Some(gap(1_000_000, 1_160_000))]);
    }

    #[test]
    fn without_a_quiet_room_nothing_is_a_hesitation() {
        let mut loud_room = peaks(&[(50, 65)]);
        for p in &mut loud_room {
            *p = (*p).max(30);
        }
        assert_eq!(hesitations(&loud_room, &[gap(800_000, 1_500_000)]), [None]);
        assert_eq!(hesitations(&[], &[gap(800_000, 1_500_000)]), [None]);
    }

    #[test]
    fn english_phrases_preserve_literal_usage() {
        assert!(filler_words(&transcript("I like you and you know this"), "en").is_empty());
        let ranges = filler_words(&transcript("Um I, like, think, you know, uh yes"), "en");
        assert_eq!(ranges.len(), 4);
        assert_eq!(ranges[2], Range { start_us: 800_000, end_us: 1_180_000 });
        assert!(filler_words(&transcript("um"), "de").is_empty());
    }
}

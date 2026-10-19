//! Groups transcript words into short on-screen captions, the way reels show them.

use nuzky_engine::edit::CaptionSegment;
use nuzky_engine::model::CaptionWord;
use serde::{Deserialize, Serialize};

use crate::Word;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptionGrouping {
    /// Most words on screen at once.
    pub max_words: usize,
    /// Most characters per caption, spaces included; a single longer word still gets its own caption.
    pub max_chars: usize,
    /// A pause at least this long between words starts a new caption.
    pub break_gap_us: i64,
}

impl Default for CaptionGrouping {
    /// Reel captions: one or two words, fifteen characters. A hand-made reel of 191 captions
    /// had 110 with two words, 67 with one and 14 with three.
    fn default() -> Self {
        Self { max_words: 2, max_chars: 15, break_gap_us: 300_000 }
    }
}

/// A caption may stay on screen this long after its last word if nothing follows.
const TAIL_US: i64 = 150_000;

/// Captions in time order. A caption ends where the next one starts when they are close,
/// so text does not flicker off between words, otherwise shortly after its last word.
/// Each caption keeps its words and their times, for karaoke styles. `max_words` counts the words
/// shown, so a corrected word with spaces ("v pohodě.") counts as each of them.
pub fn group_words(words: &[Word], grouping: CaptionGrouping, frame_us: i64) -> Vec<CaptionSegment> {
    let parts: Vec<Word> = words
        .iter()
        .filter(|w| !w.text.trim().is_empty())
        .flat_map(|w| parts(w, grouping.max_words.max(1), frame_us))
        .collect();
    let mut groups: Vec<Vec<&Word>> = Vec::new();
    for word in &parts {
        match groups.last_mut() {
            Some(group) if !hard_break(group, word, grouping) && fits(group, word, grouping) => group.push(word),
            // "iPhone 17": a number starts its caption together with the word it belongs to.
            Some(group)
                if !hard_break(group, word, grouping)
                    && starts_with_digit(word)
                    && group.len() > 1
                    && fits(&group[group.len() - 1..], word, grouping) =>
            {
                let carried = group.pop();
                groups.push(carried.into_iter().chain([word]).collect());
            }
            _ => groups.push(vec![word]),
        }
        if ends_phrase(&word.text) {
            balance_phrase_end(&mut groups, grouping);
        }
    }
    let starts: Vec<i64> = groups.iter().map(|g| g[0].start_us).collect();
    groups
        .iter()
        .enumerate()
        .map(|(i, group)| {
            let last_end = group[group.len() - 1].end_us;
            let end = match starts.get(i + 1) {
                Some(&next) if next - last_end < grouping.break_gap_us => next,
                Some(&next) => (last_end + TAIL_US).min(next),
                None => last_end + TAIL_US,
            };
            let text = group.iter().map(|w| w.text.trim()).collect::<Vec<_>>().join(" ");
            let words = group
                .iter()
                .map(|w| CaptionWord { text: w.text.trim().into(), start_us: w.start_us, end_us: w.end_us, key: false })
                .collect();
            CaptionSegment { start_us: group[0].start_us, end_us: end.max(group[0].start_us + 1), text, words }
        })
        .collect()
}

/// A phrase ends or the speaker pauses before `next`, so it never shares a caption with `group`.
fn hard_break(group: &[&Word], next: &Word, grouping: CaptionGrouping) -> bool {
    let last = group[group.len() - 1];
    ends_phrase(&last.text) || next.start_us - last.end_us >= grouping.break_gap_us
}

fn fits(group: &[&Word], next: &Word, grouping: CaptionGrouping) -> bool {
    let chars =
        group.iter().map(|w| w.text.trim().chars().count() + 1).sum::<usize>() + next.text.trim().chars().count();
    let shown = group.iter().map(|w| w.text.split_whitespace().count()).sum::<usize>();
    shown + next.text.split_whitespace().count() <= grouping.max_words.max(1) && chars <= grouping.max_chars
}

/// A word showing more than `max_words` words, such as the correction "Vzal bys ji?", goes on screen
/// in parts of up to `max_words`, which share its time by their length. A word too short to give each
/// part a frame stays whole, since captions starting within a frame of each other merge into one.
fn parts(word: &Word, max_words: usize, frame_us: i64) -> Vec<Word> {
    let shown: Vec<&str> = word.text.split_whitespace().collect();
    if shown.len() <= max_words {
        return vec![word.clone()];
    }
    let texts: Vec<String> = shown.chunks(max_words).map(|chunk| chunk.join(" ")).collect();
    let total = texts.iter().map(|t| t.chars().count() as i64).sum::<i64>();
    let span = word.end_us - word.start_us;
    let mut said = 0;
    let mut parts = Vec::with_capacity(texts.len());
    for text in texts {
        let start_us = word.start_us + span * said / total;
        said += text.chars().count() as i64;
        let end_us = word.start_us + span * said / total;
        if end_us - start_us < frame_us {
            return vec![word.clone()];
        }
        parts.push(Word { start_us, end_us, text, probability: word.probability });
    }
    parts
}

/// A phrase does not end on a lone word when the caption before can give it one:
/// "natáčí fakt | skvěle." becomes "natáčí | fakt skvěle.".
fn balance_phrase_end(groups: &mut [Vec<&Word>], grouping: CaptionGrouping) {
    let [.., before, last] = groups else { return };
    if last.len() != 1 || before.len() < 2 || hard_break(before, last[0], grouping) {
        return;
    }
    if let Some(&moved) = before.last()
        && fits(&[moved], last[0], grouping)
    {
        before.pop();
        last.insert(0, moved);
    }
}

fn starts_with_digit(word: &Word) -> bool {
    word.text.trim_start().starts_with(|c: char| c.is_ascii_digit())
}

fn ends_phrase(text: &str) -> bool {
    text.trim_end().ends_with(['.', ',', '!', '?', ';', ':', '…'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(start_ms: i64, end_ms: i64, text: &str) -> Word {
        Word { start_us: start_ms * 1000, end_us: end_ms * 1000, text: text.into(), probability: 1.0 }
    }

    /// One frame at 30 fps.
    const FRAME_US: i64 = 33_334;

    fn texts(words: &[Word], grouping: CaptionGrouping) -> Vec<String> {
        group_words(words, grouping, FRAME_US).into_iter().map(|c| c.text).collect()
    }

    #[test]
    fn breaks_on_word_count_characters_punctuation_and_pauses() {
        let words = [
            word(0, 200, "iPhone"),
            word(200, 400, "17"),
            word(420, 600, "mi"),
            word(600, 800, "dnes"),
            word(800, 1300, "natáčí"),
            word(1300, 1500, "fakt"),
            word(1500, 1900, "skvěle."),
            word(1950, 2300, "Největší"),
            word(2300, 2700, "rozdíl"),
            word(3200, 3500, "je"),
        ];
        assert_eq!(
            texts(&words, CaptionGrouping::default()),
            ["iPhone 17", "mi dnes", "natáčí", "fakt skvěle.", "Největší rozdíl", "je"]
        );
        let one = CaptionGrouping { max_words: 1, ..CaptionGrouping::default() };
        assert_eq!(texts(&words[..2], one), ["iPhone", "17"]);
        let three = CaptionGrouping { max_words: 3, ..CaptionGrouping::default() };
        assert_eq!(texts(&words[..5], three), ["iPhone 17 mi", "dnes natáčí"]);
    }

    #[test]
    fn numbers_stay_with_their_word_and_phrases_do_not_end_on_a_lone_word() {
        let words = [
            word(0, 300, "přišel"),
            word(300, 600, "iPhone"),
            word(600, 900, "17"),
            word(900, 1100, "a"),
            word(1100, 1500, "konec."),
        ];
        assert_eq!(texts(&words, CaptionGrouping::default()), ["přišel", "iPhone 17", "a konec."]);
        // Across a pause the lone word keeps its own caption.
        let paused = [word(0, 300, "fakt"), word(300, 600, "dobře"), word(1200, 1500, "jo.")];
        assert_eq!(texts(&paused, CaptionGrouping::default()), ["fakt dobře", "jo."]);
    }

    #[test]
    fn captions_bridge_short_gaps_and_end_after_long_ones() {
        let words = [word(0, 400, "Ahoj"), word(420, 800, "světe."), word(2000, 2400, "Konec")];
        let captions = group_words(&words, CaptionGrouping::default(), FRAME_US);
        // "Ahoj světe." ends at a full stop; the next caption is 1.2 s later, so it ends after a short tail.
        assert_eq!((captions[0].start_us, captions[0].end_us), (0, 950_000));
        assert_eq!((captions[1].start_us, captions[1].end_us), (2_000_000, 2_550_000));
        let close = [word(0, 400, "musím"), word(500, 900, "se"), word(950, 1300, "víc,"), word(1350, 1700, "snažit")];
        let captions = group_words(&close, CaptionGrouping::default(), FRAME_US);
        assert_eq!(captions[0].end_us, captions[1].start_us);
    }

    #[test]
    fn captions_keep_each_word_with_its_time_for_karaoke() {
        let words = [word(0, 400, " Ahoj"), word(420, 800, "světe. "), word(2000, 2400, "Konec")];
        let captions = group_words(&words, CaptionGrouping::default(), FRAME_US);
        let spoken: Vec<Vec<(&str, i64, i64)>> = captions
            .iter()
            .map(|c| c.words.iter().map(|w| (w.text.as_str(), w.start_us / 1000, w.end_us / 1000)).collect())
            .collect();
        assert_eq!(spoken, [vec![("Ahoj", 0, 400), ("světe.", 420, 800)], vec![("Konec", 2000, 2400)]]);
        for caption in &captions {
            assert_eq!(caption.text, caption.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "));
        }
    }

    #[test]
    fn a_correction_with_spaces_counts_every_word_it_shows() {
        let words = [word(0, 300, "fakt"), word(300, 900, "v pohodě"), word(900, 1200, "dneska")];
        assert_eq!(texts(&words, CaptionGrouping::default()), ["fakt", "v pohodě", "dneska"]);
        // "natáčí fakt | v pohodě." would balance into "natáčí | fakt v pohodě." by entries, not words.
        let phrase = [word(0, 300, "natáčí"), word(300, 600, "fakt"), word(600, 1200, "v pohodě.")];
        assert_eq!(texts(&phrase, CaptionGrouping::default()), ["natáčí fakt", "v pohodě."]);
    }

    #[test]
    fn a_correction_longer_than_a_caption_splits_over_consecutive_captions() {
        let words = [word(0, 900, "Vzal bys ji?"), word(1000, 1300, "Jo.")];
        let captions = group_words(&words, CaptionGrouping::default(), FRAME_US);
        let texts: Vec<_> = captions.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["Vzal bys", "ji?", "Jo."]);
        for pair in captions.windows(2) {
            assert!(pair[0].start_us < pair[1].start_us && pair[0].end_us <= pair[1].start_us, "{captions:?}");
        }
        // Each part highlights over its share of the word, by length: 8 of 11 characters, then 3.
        assert_eq!((captions[0].words[0].start_us, captions[0].words[0].end_us), (0, 654_545));
        assert_eq!((captions[1].words[0].start_us, captions[1].words[0].end_us), (654_545, 900_000));
        for caption in &captions {
            assert_eq!(caption.text, caption.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "));
        }
        let one = CaptionGrouping { max_words: 1, ..CaptionGrouping::default() };
        let texts: Vec<_> = group_words(&words, one, FRAME_US).into_iter().map(|c| c.text).collect();
        assert_eq!(texts, ["Vzal", "bys", "ji?", "Jo."]);
    }

    #[test]
    fn a_correction_too_short_for_a_frame_per_part_stays_whole() {
        let words = [word(0, 50, "Vzal bys ji?")];
        let captions = group_words(&words, CaptionGrouping::default(), FRAME_US);
        assert_eq!(captions.len(), 1);
        assert_eq!(captions[0].text, "Vzal bys ji?");
    }

    #[test]
    fn a_long_word_still_gets_its_own_caption() {
        let words = [word(0, 900, "nejneobhospodařovávatelnějšími"), word(900, 1200, "lidmi")];
        assert_eq!(texts(&words, CaptionGrouping::default()), ["nejneobhospodařovávatelnějšími", "lidmi"]);
    }
}

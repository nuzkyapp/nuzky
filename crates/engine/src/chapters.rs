//! YouTube chapters: where each chapter of the project starts in the exported video, and the list for the video's
//! description. YouTube shows chapters only when the first starts at 00:00, there are at least three in ascending
//! order and each lasts 10 s or more (https://support.google.com/youtube/answer/9884579). A chapter that would break
//! that is left out of the list with the reason, and no list is given when fewer than three would be left.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::model::{ClipContent, Project};
use crate::retime::Retime;
use crate::speech::{TimelineWord, is_heard};

/// YouTube's shortest chapter.
pub const MIN_CHAPTER_US: i64 = 10_000_000;
/// YouTube's fewest chapters.
pub const MIN_CHAPTERS: usize = 3;
/// The shortest video three chapters fit in.
pub const MIN_VIDEO_US: i64 = MIN_CHAPTERS as i64 * MIN_CHAPTER_US;
const SECOND: i64 = 1_000_000;
const HOUR: i64 = 3600 * SECOND;

pub const WORDS_CUT: &str = "Its words were cut or moved";
pub const TOO_CLOSE: &str = "Less than 10 s from the chapter next to it";
pub const TOO_LATE: &str = "Less than 10 s before the end";

/// A chapter of the project where it plays in the video.
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacedChapter {
    pub title: String,
    /// Where its first word is heard, 0 for the first chapter; none when cuts took all its words.
    pub start_us: Option<i64>,
    /// Its time as YouTube reads it, in whole seconds: "01:23", or "1:01:23" in a video over an hour; empty without
    /// a start.
    pub time: String,
    /// Why YouTube would not take it, when it is left out of the list.
    pub left_out: Option<String>,
}

/// The project's chapters and the list YouTube takes from them.
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterList {
    /// Every chapter of the project, in its order.
    pub chapters: Vec<PlacedChapter>,
    /// The list for the description, a "time title" line for each chapter not left out; empty while `problem` says
    /// why there is none.
    pub text: String,
    pub problem: Option<String>,
}

/// Places the project's chapters on the timeline, whose `words` were placed by `speech::map_words`. A chapter starts
/// where a clip plays its first word's start in the file now; when a cut took that moment, at the first of its words
/// still heard. Of those places, the list takes the ones that keep the chapters in order, 10 s or more apart and 10 s
/// or more before the end, placing the most chapters, then the most at their own first word, then the nearest to it,
/// then the earliest. A chapter it cannot take is left out with the reason. The list counts whole seconds down, so it
/// never jumps past a chapter's first word.
pub fn place(project: &Project, words: &[TimelineWord]) -> ChapterList {
    let duration = project.duration_us();
    let long = duration > HOUR;
    let found = candidates(project, words);
    let listed = in_order(&found, project.chapters.len(), duration);
    let mut chapters = Vec::with_capacity(project.chapters.len());
    for (i, chapter) in project.chapters.iter().enumerate() {
        let (start, left_out) = match (i, listed[i]) {
            (0, _) => (Some(0), None),
            (_, Some(us)) => (Some(us), None),
            _ => {
                let before = listed[..i].iter().rev().flatten().next().copied().unwrap_or(0);
                let after = listed[i + 1..].iter().flatten().next().copied();
                let between =
                    found.iter().find(|c| c.chapter == i && c.at_us > before && after.is_none_or(|a| c.at_us < a));
                match between {
                    None => (None, Some(WORDS_CUT)),
                    Some(c) if after.is_none() && !ends_in_time(c.at_us, duration) => (Some(c.at_us), Some(TOO_LATE)),
                    Some(c) => (Some(c.at_us), Some(TOO_CLOSE)),
                }
            }
        };
        chapters.push(PlacedChapter {
            title: chapter.title.trim().to_owned(),
            start_us: start,
            time: start.map(|us| time(us, long)).unwrap_or_default(),
            left_out: left_out.map(str::to_owned),
        });
    }
    let kept: Vec<_> = chapters.iter().filter(|c| c.left_out.is_none()).collect();
    let problem = if chapters.is_empty() {
        None
    } else if duration < MIN_VIDEO_US {
        Some(format!(
            "The video is {} s long. YouTube shows chapters only in a video of 30 s or more: 3 chapters of 10 s.",
            duration / SECOND
        ))
    } else if kept.len() < MIN_CHAPTERS {
        Some(format!(
            "YouTube needs 3 chapters, each 10 s or longer, and only {} {} left. Suggest chapters again.",
            kept.len(),
            if kept.len() == 1 { "is" } else { "are" }
        ))
    } else {
        None
    };
    let text = match problem {
        Some(_) => String::new(),
        None => kept.iter().map(|c| format!("{} {}", c.time, c.title)).collect::<Vec<_>>().join("\n"),
    };
    ChapterList { chapters, text, problem }
}

/// A chapter starting at `at_us` lasts 10 s or more before the end, counted from its whole second.
fn ends_in_time(at_us: i64, duration: i64) -> bool {
    duration - at_us / SECOND * SECOND >= MIN_CHAPTER_US
}

/// A place a chapter could start: its chapter, the time on the timeline, whether it is the chapter's own first word
/// and how far into the file after it, when not.
struct Candidate {
    chapter: usize,
    at_us: i64,
    exact: bool,
    drift_us: i64,
}

/// Every place each chapter after the first could start, in chapter order.
fn candidates(project: &Project, words: &[TimelineWord]) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (i, chapter) in project.chapters.iter().enumerate().skip(1) {
        let exact = heard_at(project, &chapter.asset_id, chapter.source_us);
        if !exact.is_empty() {
            out.extend(exact.into_iter().map(|at_us| Candidate { chapter: i, at_us, exact: true, drift_us: 0 }));
            continue;
        }
        let until = project
            .chapters
            .get(i + 1)
            .filter(|next| next.asset_id == chapter.asset_id && next.source_us > chapter.source_us);
        let own = |w: &&TimelineWord| {
            w.asset_id == chapter.asset_id
                && w.source_start_us >= chapter.source_us
                && until.is_none_or(|next| w.source_start_us < next.source_us)
        };
        let Some(first) = words.iter().filter(own).map(|w| w.source_start_us).min() else { continue };
        out.extend(words.iter().filter(own).filter(|w| w.source_start_us == first).map(|w| Candidate {
            chapter: i,
            at_us: w.start_us,
            exact: false,
            drift_us: first - chapter.source_us,
        }));
    }
    out
}

/// The start of each of `count` chapters from `candidates`, none for the first, which is at 0: the places in
/// ascending order 10 s or more apart, the last 10 s or more before the end, that place the most chapters, then the
/// most exactly, then with the least drift, then the earliest.
fn in_order(candidates: &[Candidate], count: usize, duration: i64) -> Vec<Option<i64>> {
    type Score = (usize, usize, i64, i64);
    let apart = |from: i64, to: i64| to / SECOND - from / SECOND >= MIN_CHAPTER_US / SECOND;
    let value = |c: &Candidate| (1, usize::from(c.exact), -c.drift_us, -c.at_us);
    let add = |a: Score, b: Score| (a.0 + b.0, a.1 + b.1, a.2 + b.2, a.3 + b.3);
    let mut best: Vec<Option<(Score, Option<usize>)>> = Vec::with_capacity(candidates.len());
    for (k, c) in candidates.iter().enumerate() {
        let entry = apart(0, c.at_us).then(|| {
            let before = (0..k)
                .filter_map(|p| best[p].map(|(score, _)| (p, score)))
                .filter(|&(p, _)| candidates[p].chapter < c.chapter && apart(candidates[p].at_us, c.at_us))
                .max_by_key(|&(_, score)| score);
            (add(before.map_or((0, 0, 0, 0), |(_, score)| score), value(c)), before.map(|(p, _)| p))
        });
        best.push(entry);
    }
    let mut starts = vec![None; count];
    let mut at = (0..candidates.len())
        .filter_map(|k| best[k].map(|(score, _)| (k, score)))
        .filter(|&(k, _)| ends_in_time(candidates[k].at_us, duration))
        .max_by_key(|&(_, score)| score)
        .map(|(k, _)| k);
    while let Some(k) = at {
        starts[candidates[k].chapter] = Some(candidates[k].at_us);
        at = best[k].and_then(|(_, before)| before);
    }
    starts
}

/// Every time a heard clip plays the moment `source_us` of the file, forwards.
fn heard_at(project: &Project, asset_id: &str, source_us: i64) -> Vec<i64> {
    project
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter().filter(move |clip| is_heard(project, track, clip)))
        .filter_map(|clip| {
            let ClipContent::Media { asset_id: id, source_in_us, speed, .. } = &clip.content else { return None };
            let map = Retime::of(clip).filter(|map| !map.reverse() && id == asset_id)?;
            let source_end = *source_in_us as f64 + clip.duration_us as f64 * f64::from(*speed);
            let at = source_us as f64;
            (at >= *source_in_us as f64 && at < source_end).then(|| clip.start_us + map.offset_of(at).round() as i64)
        })
        .collect()
}

/// A time on the timeline in whole seconds, counted down: "01:23", or "1:01:23" with `long`.
fn time(us: i64, long: bool) -> String {
    let s = us / SECOND;
    if long { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) }
}

/// The chapters file written beside an export: `reel.mp4` gets `reel.chapters.txt`.
pub fn file_path(video: &Path) -> PathBuf {
    video.with_extension("chapters.txt")
}

/// What the chapters file holds: the list, ending with a line break.
pub fn file_text(list: &ChapterList) -> Option<String> {
    (list.problem.is_none() && !list.text.is_empty()).then(|| format!("{}\n", list.text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::EditCmd;
    use crate::model::{Asset, AssetKind, Chapter};
    use crate::speech::{Word, map_words};
    use serde_json::json;

    const S: i64 = SECOND;

    fn edit(project: &mut Project, cmd: serde_json::Value) {
        project.apply(serde_json::from_value::<EditCmd>(cmd).unwrap()).unwrap();
    }

    /// A take of `duration_us` on the main track.
    fn take(duration_us: i64) -> Project {
        let mut project = Project::new("talk");
        project.apply(EditCmd::AddAssets { assets: vec![asset("take", duration_us)] }).unwrap();
        edit(&mut project, json!({"type": "addClip", "assetId": "take"}));
        project
    }

    fn asset(id: &str, duration_us: i64) -> Asset {
        Asset {
            id: id.into(),
            name: format!("{id}.mov"),
            path: format!("/{id}.mov"),
            kind: AssetKind::Video,
            duration_us,
            width: 1080,
            height: 1920,
            fps: 30.0,
            has_audio: true,
            rotation: 0,
            mirror: false,
            credit: None,
            part: None,
        }
    }

    fn chapter(title: &str, source_us: i64) -> Chapter {
        Chapter { title: title.into(), asset_id: "take".into(), source_us }
    }

    /// A word every second of each file, starting 0.4 s into each.
    fn words(project: &Project) -> Vec<TimelineWord> {
        words_from(project, 400_000)
    }

    fn words_from(project: &Project, offset_us: i64) -> Vec<TimelineWord> {
        let said = |duration: i64| {
            (0..duration / S)
                .map(|s| Word {
                    start_us: s * S + offset_us,
                    end_us: s * S + 800_000,
                    text: format!("w{s}"),
                    probability: 0.9,
                })
                .collect()
        };
        map_words(project, &project.assets.iter().map(|a| (a.id.clone(), said(a.duration_us))).collect())
    }

    #[test]
    fn the_list_starts_at_zero_and_counts_whole_seconds_down() {
        let mut project = take(40 * S);
        project.chapters =
            vec![chapter("Hello", 400_000), chapter("Why", 12 * S + 400_000), chapter(" How ", 25 * S + 400_000)];
        let list = place(&project, &words(&project));
        assert_eq!(list.text, "00:00 Hello\n00:12 Why\n00:25 How");
        assert_eq!(list.chapters[1].start_us, Some(12 * S + 400_000));
        assert_eq!(list.problem, None);
        assert_eq!(file_text(&list).unwrap(), "00:00 Hello\n00:12 Why\n00:25 How\n");
    }

    #[test]
    fn a_video_over_an_hour_counts_hours() {
        let mut project = take(2 * HOUR);
        project.chapters =
            vec![chapter("A", 0), chapter("B", HOUR + 61 * S + 400_000), chapter("C", HOUR + 2 * 60 * S)];
        assert_eq!(place(&project, &words(&project)).text, "0:00:00 A\n1:01:01 B\n1:02:00 C");
        let mut hour = take(HOUR);
        hour.chapters = vec![chapter("A", 0), chapter("B", 61 * S), chapter("C", 59 * 60 * S)];
        assert_eq!(place(&hour, &words(&hour)).text, "00:00 A\n01:01 B\n59:00 C", "an hour exactly is not over one");
    }

    #[test]
    fn chapters_shorter_than_ten_seconds_are_left_out_with_why() {
        let mut project = take(45 * S);
        // 10 s after the first counts; 9.4 s, in whole seconds 9, does not, and neither do 7.6 s before the end.
        project.chapters = vec![
            chapter("A", 0),
            chapter("B", 9 * S + 400_000),
            chapter("C", 10 * S + 400_000),
            chapter("D", 20 * S + 400_000),
            chapter("E", 37 * S + 400_000),
        ];
        let list = place(&project, &words(&project));
        let left: Vec<_> = list.chapters.iter().map(|c| c.left_out.as_deref()).collect();
        assert_eq!(left, [None, Some(TOO_CLOSE), None, None, Some(TOO_LATE)]);
        assert_eq!(list.text, "00:00 A\n00:10 C\n00:20 D");
    }

    #[test]
    fn fewer_than_three_or_a_short_video_give_no_list() {
        let mut project = take(40 * S);
        project.chapters = vec![chapter("A", 0), chapter("B", 5 * S), chapter("C", 20 * S)];
        let list = place(&project, &words(&project));
        assert_eq!(list.text, "");
        assert_eq!(
            list.problem.as_deref(),
            Some("YouTube needs 3 chapters, each 10 s or longer, and only 2 are left. Suggest chapters again.")
        );
        assert_eq!(file_text(&list), None);

        let mut short = take(29 * S);
        short.chapters = vec![chapter("A", 0), chapter("B", 10 * S), chapter("C", 19 * S)];
        let list = place(&short, &words(&short));
        assert!(list.problem.unwrap().starts_with("The video is 29 s long."), "three chapters of 10 s need 30 s");

        project.chapters.clear();
        assert_eq!(place(&project, &words(&project)), ChapterList::default());
    }

    #[test]
    fn chapters_follow_cuts_and_speed_and_lose_only_their_own_words() {
        let chapters = vec![chapter("A", 0), chapter("B", 20 * S + 400_000), chapter("C", 40 * S + 400_000)];
        let mut project = take(60 * S);
        project.chapters = chapters.clone();
        // The first 10 s of B go, and the rest plays twice as fast.
        edit(&mut project, json!({"type": "rippleDeleteRanges", "ranges": [{"startUs": 20 * S, "endUs": 30 * S}]}));
        let rest = project.tracks[0].clips[1].id.clone();
        edit(&mut project, json!({"type": "updateClip", "clipId": rest, "speed": 2.0}));
        let list = place(&project, &words(&project));
        // B's first word heard is 30.4 s into the file, 20.2 s into the video; C's is 40.4 s, 20 + 10.4 / 2.
        assert_eq!(list.chapters[1].start_us, Some(20 * S + 200_000));
        assert_eq!(list.chapters[2].start_us, Some(25 * S + 200_000));
        assert_eq!(
            list.chapters[1].left_out.as_deref(),
            Some(TOO_CLOSE),
            "C starts at its own first word, B no longer does"
        );
        assert_eq!(list.chapters[2].left_out, None);

        // Recognised again, every word starts 0.1 s earlier: the chapters stay where they were.
        let again = words_from(&project, 300_000);
        assert_eq!(place(&project, &again).chapters[2].start_us, Some(25 * S + 200_000));

        // With all of B's words cut, B is nowhere, rather than taking C's first word.
        let mut project = take(60 * S);
        project.chapters = chapters;
        edit(&mut project, json!({"type": "rippleDeleteRanges", "ranges": [{"startUs": 20 * S, "endUs": 40 * S}]}));
        let list = place(&project, &words(&project));
        assert_eq!((list.chapters[1].start_us, list.chapters[1].left_out.as_deref()), (None, Some(WORDS_CUT)));
        assert_eq!(list.chapters[2].start_us, Some(20 * S + 400_000));
    }

    #[test]
    fn a_chapter_whose_words_were_cut_never_jumps_past_the_next_one_in_another_file() {
        let mut project = Project::new("two files");
        project.apply(EditCmd::AddAssets { assets: vec![asset("a", 60 * S), asset("b", 30 * S)] }).unwrap();
        for id in ["a", "b", "a"] {
            edit(&mut project, json!({"type": "addClip", "assetId": id}));
        }
        // a 0-30 s, all of b, then a again from 40 s.
        let ranges = json!([{"startUs": 30 * S, "endUs": 60 * S}, {"startUs": 90 * S, "endUs": 130 * S}]);
        edit(&mut project, json!({"type": "rippleDeleteRanges", "ranges": ranges}));
        let chapter = |id: &str, source_us: i64| Chapter { title: id.into(), asset_id: id.into(), source_us };
        project.chapters = vec![
            chapter("a", 400_000),
            chapter("a", 20 * S + 400_000),
            chapter("b", 5 * S + 400_000),
            chapter("a", 50 * S + 400_000),
        ];
        edit(&mut project, json!({"type": "rippleDeleteRanges", "ranges": [{"startUs": 20 * S, "endUs": 30 * S}]}));
        let list = place(&project, &words(&project));
        let placed: Vec<_> = list.chapters.iter().map(|c| (c.start_us, c.left_out.as_deref())).collect();
        assert_eq!(
            placed,
            [(Some(0), None), (None, Some(WORDS_CUT)), (Some(25 * S + 400_000), None), (Some(60 * S + 400_000), None)]
        );

        // Cutting b's first word too: b starts at its next word, still before the return to a.
        edit(&mut project, json!({"type": "rippleDeleteRanges", "ranges": [{"startUs": 25 * S, "endUs": 26 * S}]}));
        let list = place(&project, &words(&project));
        let placed: Vec<_> = list.chapters.iter().map(|c| (c.start_us, c.left_out.as_deref())).collect();
        assert_eq!(
            placed,
            [(Some(0), None), (None, Some(WORDS_CUT)), (Some(25 * S + 400_000), None), (Some(59 * S + 400_000), None)]
        );
    }

    #[test]
    fn a_hook_copied_from_later_to_the_start_keeps_every_chapter() {
        let mut project = take(60 * S);
        project.chapters = vec![chapter("A", 400_000), chapter("B", 20 * S + 400_000), chapter("C", 40 * S + 400_000)];
        edit(&mut project, json!({"type": "addClip", "assetId": "take", "startUs": 0}));
        let hook = project.tracks[0].clips[0].id.clone();
        edit(
            &mut project,
            json!({"type": "trimClip", "clipId": hook, "startUs": 0, "durationUs": 6 * S, "sourceInUs": 40 * S}),
        );
        let list = place(&project, &words(&project));
        let starts: Vec<_> = list.chapters.iter().map(|c| c.start_us).collect();
        assert_eq!(starts, [Some(0), Some(26 * S + 400_000), Some(46 * S + 400_000)], "{list:?}");
        assert_eq!(list.text, "00:00 A\n00:26 B\n00:46 C");
    }

    #[test]
    fn a_repeat_at_the_end_keeps_the_chapter_where_it_was_said() {
        let mut project = take(60 * S);
        project.chapters = vec![chapter("A", 400_000), chapter("B", 20 * S + 400_000), chapter("C", 40 * S + 400_000)];
        edit(&mut project, json!({"type": "addClip", "assetId": "take"}));
        let repeat = project.tracks[0].clips[1].id.clone();
        let trim =
            json!({"type": "trimClip", "clipId": repeat, "startUs": 60 * S, "durationUs": 6 * S, "sourceInUs": 40 * S});
        edit(&mut project, trim);
        assert_eq!(place(&project, &words(&project)).text, "00:00 A\n00:20 B\n00:40 C");
    }

    #[test]
    fn a_chapter_cut_to_the_very_start_is_left_out_and_the_next_still_found() {
        let mut project = take(60 * S);
        project.chapters = vec![
            chapter("A", 400_000),
            chapter("B", 10 * S + 400_000),
            chapter("C", 30 * S + 400_000),
            chapter("D", 45 * S + 400_000),
        ];
        edit(&mut project, json!({"type": "rippleDeleteRanges", "ranges": [{"startUs": 0, "endUs": 11 * S}]}));
        edit(&mut project, json!({"type": "rippleDeleteRanges", "ranges": [{"startUs": 19 * S, "endUs": 20 * S}]}));
        let list = place(&project, &words(&project));
        let placed: Vec<_> = list.chapters.iter().map(|c| (c.start_us, c.left_out.as_deref())).collect();
        assert_eq!(
            placed,
            [
                (Some(0), None),
                (Some(400_000), Some(TOO_CLOSE)),
                (Some(19 * S + 400_000), None),
                (Some(33 * S + 400_000), None)
            ]
        );
        assert_eq!(list.text, "00:00 A\n00:19 C\n00:33 D");
    }
}

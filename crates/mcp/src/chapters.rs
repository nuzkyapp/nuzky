//! YouTube chapters an agent proposes from the transcript. Each starts at a word, and Nuzky times it on the video
//! as it plays now (`nuzky_engine::chapters`).

use anyhow::{Result, bail, ensure};
use nuzky_engine::{
    Project,
    chapters::{ChapterList, MIN_CHAPTERS, MIN_VIDEO_US, place},
    model::{Chapter, MAX_CHAPTER_TITLE_CHARS, MAX_CHAPTERS},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::transcript::Derived;

/// A chapter an agent proposes.
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChapterProposal {
    /// The word of get_transcript the chapter starts with; the first chapter starts at 0:00 whatever its word.
    pub from: usize,
    /// Short, in the video's language, up to 100 characters on one line.
    pub title: String,
}

/// The project's chapters where the video plays them now.
pub fn list(project: &Project, derived: &Derived) -> ChapterList {
    place(project, &derived.words)
}

/// The chapters of `proposals`, each tied to its first word, refused unless YouTube would show every one of them.
pub fn plan(project: &Project, derived: &Derived, proposals: &[ChapterProposal]) -> Result<Vec<Chapter>> {
    ensure!(
        proposals.is_empty() || (MIN_CHAPTERS..=MAX_CHAPTERS).contains(&proposals.len()),
        "CHAPTERS_INVALID: YouTube shows chapters only when there are {MIN_CHAPTERS} or more; propose {MIN_CHAPTERS} to {MAX_CHAPTERS}, or [] to remove them"
    );
    let mut chapters = Vec::with_capacity(proposals.len());
    for (n, proposal) in proposals.iter().enumerate() {
        let title = proposal.title.trim();
        ensure!(
            !title.is_empty()
                && title.chars().count() <= MAX_CHAPTER_TITLE_CHARS
                && !title.chars().any(char::is_control),
            "CHAPTER_TITLE: chapter {}'s title is one line of 1 to {MAX_CHAPTER_TITLE_CHARS} characters",
            n + 1
        );
        let Some(word) = derived.words.get(proposal.from) else {
            bail!(
                "CHAPTER_OUTSIDE_TRANSCRIPT: the transcript has {} words, so it has no word {}",
                derived.words.len(),
                proposal.from
            )
        };
        if let Some(before) = n.checked_sub(1).map(|i| proposals[i].from) {
            ensure!(
                proposal.from > before,
                "CHAPTER_ORDER: chapters go in the order they play, so chapter {} starts after word {before}",
                n + 1
            );
        }
        chapters.push(Chapter {
            title: title.to_owned(),
            asset_id: word.asset_id.clone(),
            source_us: word.source_start_us,
        });
    }
    let placed = place(&Project { chapters: chapters.clone(), ..project.clone() }, &derived.words);
    if let Some(problem) = placed.problem.as_ref().filter(|_| project.duration_us() < MIN_VIDEO_US) {
        bail!("CHAPTERS_INVALID: {problem}");
    }
    let refused: Vec<_> = placed
        .chapters
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            c.left_out.as_ref().map(|why| format!("chapter {} {:?} at {}: {why}", i + 1, c.title, c.time))
        })
        .collect();
    ensure!(
        refused.is_empty(),
        "CHAPTERS_INVALID: YouTube shows chapters only when each lasts 10 s or more, so merge or move these: {}",
        refused.join("; ")
    );
    Ok(chapters)
}

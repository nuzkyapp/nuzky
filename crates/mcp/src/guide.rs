use std::sync::LazyLock;

use anyhow::{Result, bail};

const GUIDE: &str = include_str!("../../../skills/nuzky-edit/SKILL.md");

#[cfg(test)]
const CLIENT_LIMIT: usize = 4096;

pub(crate) struct Topic {
    pub id: &'static str,
    pub heading: &'static str,
    pub when: &'static str,
}

const fn topic(id: &'static str, heading: &'static str, when: &'static str) -> Topic {
    Topic { id, heading, when }
}

pub(crate) const TOPICS: &[Topic] = &[
    topic("cut", "Cut slips and pauses, then caption", "a rough cut by words, captions, checking cuts, export"),
    topic("translate", "Captions in another language", "captions in a second language"),
    topic("capcut", "Finish a draft from CapCut", "get_state has imported, or a CapCut look to copy"),
    topic("footage", "See the footage", "finding where something happens"),
    topic("clips", "Know each clip", "what a file holds: when recorded, frames, objects, hum"),
    topic("reframe", "Wide footage for a reel", "wide footage on a vertical canvas, crop, mask, covered faces"),
    topic("cover", "Pick a cover frame", "a cover or YouTube thumbnail"),
    topic("reels", "Reels from a long video", "several reels from one recording"),
    topic("chapters", "YouTube chapters", "chapters or a YouTube description"),
    topic("hooks", "Hook variants", "a file per recorded hook"),
    topic("zoom", "Make it dynamic, when asked or the style zooms", "zooms, punch-ins, slow camera moves"),
    topic("titles", "Titles and text styles", "a title or a named text or caption look"),
    topic("transitions", "Transitions, effects and keyframe curves", "transitions, effects, easing"),
    topic("broll", "B-roll and picture in picture", "B-roll, a web or generated picture, stickers"),
    topic("layout", "Split screen and one look over several clips", "split screen, one grade over many clips"),
    topic("brand", "A brand's logo and its colours", "a brand's logo, captions in its colours"),
    topic("music", "Music and sound effects", "music, sound effects, a YouTube link"),
    topic("beats", "Beat cuts", "cutting to the beat"),
    topic("headless", "Without the app", "no app open, AGENT_ACCESS_OFF, APP_NOT_RUNNING, APP_CLOSED"),
    topic("voiceover", "A script for a voiceover", "a script to read over the video"),
    topic("edits", "Runs and low-level edits", "apply_edits, transforms, track order, undo and history, fonts"),
    topic("plugins", "Plugins", "the user's plugins and effects"),
    topic("style", "The creator's style", "reading or changing how the creator edits"),
    topic("ask", "A decision that is the user's", "offering the user choices"),
    topic("copy", "Repeat, restyle and replace clips", "repeating clips, copying a look, other media in a clip"),
    topic("sound", "How a voice and music sound", "noise, echo, loudness, one ear, music under speech"),
    topic("look", "Log footage and what is behind a person", "log footage, LUT, a blurred or replaced background"),
    topic("speed", "Freeze, speed, slow motion and reverse", "a freeze frame, speed, slow motion, reverse"),
];

pub(crate) static DESCRIPTION: LazyLock<String> = LazyLock::new(|| {
    format!(
        "Read Nuzky's editing guide by topic: the steps, numbers and exact edits for a kind of work. Read a topic before you first work on it, several in one call (topics: [\"cut\", \"zoom\"]); each comes back whole, so do not read one twice in a conversation. Without topics it gives the rules that hold for every request and the list of topics. Read-only, needs no run. Topics:\n{}",
        index()
    )
});

fn index() -> String {
    TOPICS.iter().map(|topic| format!("- {}: {}", topic.id, topic.when)).collect::<Vec<_>>().join("\n")
}

fn text() -> &'static str {
    GUIDE.splitn(3, "---").nth(2).unwrap_or(GUIDE).trim_start()
}

fn opening() -> &'static str {
    let text = text();
    text.find("\n## ").map_or(text, |end| &text[..end]).trim_end()
}

fn section(topic: &Topic) -> Option<&'static str> {
    let text = text();
    let start = text.find(&format!("\n## {}\n", topic.heading))? + 1;
    let rest = &text[start..];
    Some(rest[3..].find("\n## ").map_or(rest, |end| &rest[..end + 3]).trim_end())
}

pub(crate) fn instructions(style: Option<&str>) -> String {
    let ids: Vec<&str> = TOPICS.iter().map(|topic| topic.id).collect();
    let guide = format!(
        "{}\n\nThe rest of this guide is in topics. Before you first work on one, read it with get_guide(topics), several in one call: {}.",
        opening(),
        ids.join(", ")
    );
    match style {
        Some(style) => format!(
            "This creator has their own editing style, EDIT.md, which follows these rules. Where it gives a rule or a number, follow it instead of the guide's defaults: what to cut, pause length, captions, framing and zoom, including zoom the user did not ask for. Under \"Your rules\" the creator wrote their own instructions; they win over everything else. When it ends cut short here, read it whole with get_style before you cut, caption or zoom.\n\n{guide}\n\n{}",
            style.trim_end()
        ),
        None => guide,
    }
}

pub(crate) fn read(topics: &[String]) -> Result<String> {
    if topics.is_empty() {
        return Ok(format!("{}\n\nTopics of the guide, for get_guide(topics):\n{}", opening(), index()));
    }
    if let Some(unknown) = topics.iter().find(|id| !TOPICS.iter().any(|topic| topic.id == id.as_str())) {
        let ids: Vec<&str> = TOPICS.iter().map(|topic| topic.id).collect();
        bail!("INVALID_ARGUMENTS: no topic {unknown:?}; the topics are {}", ids.join(", "));
    }
    let sections: Vec<&str> =
        TOPICS.iter().filter(|topic| topics.iter().any(|id| id == topic.id)).filter_map(section).collect();
    Ok(sections.join("\n\n"))
}

pub(crate) fn sections(ids: &[&str]) -> String {
    let ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
    read(&ids).unwrap_or_default()
}

pub(crate) fn whole() -> &'static str {
    GUIDE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_of_the_guide_is_a_topic_and_every_topic_a_section() {
        let headings: Vec<&str> = text().lines().filter_map(|line| line.strip_prefix("## ")).collect();
        let listed: Vec<&str> = TOPICS.iter().map(|topic| topic.heading).collect();
        assert_eq!(headings, listed);
        let mut ids: Vec<&str> = TOPICS.iter().map(|topic| topic.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), TOPICS.len(), "a topic id is used twice");
        let all: Vec<String> = TOPICS.iter().map(|topic| topic.id.to_owned()).collect();
        assert_eq!(format!("{}\n\n{}", opening(), read(&all).unwrap()), text().trim_end());
    }

    #[test]
    fn a_topic_is_its_own_section_only() {
        let cover = read(&["cover".to_owned()]).unwrap();
        assert!(cover.starts_with("## Pick a cover frame\n"), "{cover}");
        assert!(
            cover.contains("### A cover from a generated picture") && !cover.contains("## Reels from a long video")
        );
        let two = read(&["beats".to_owned(), "cut".to_owned()]).unwrap();
        assert!(two.starts_with("## Cut slips and pauses, then caption\n") && two.contains("\n\n## Beat cuts\n"));
        let list = read(&[]).unwrap();
        assert!(
            list.starts_with("# Edit with Nuzky\n") && list.contains("- cut: ") && !list.contains("\n## "),
            "{list}"
        );
        let error = read(&["cuts".to_owned()]).unwrap_err().to_string();
        assert!(
            error.starts_with("INVALID_ARGUMENTS: no topic \"cuts\"") && error.contains("cut, translate"),
            "{error}"
        );
    }

    #[test]
    fn every_tool_description_fits_what_claude_code_keeps() {
        for tool in crate::catalog().unwrap() {
            let length = tool.description.as_deref().unwrap_or_default().len();
            assert!(length <= CLIENT_LIMIT, "{} is described in {length} characters", tool.name);
        }
    }

    #[test]
    fn instructions_and_the_topic_list_fit_what_claude_code_keeps() {
        let plain = instructions(None);
        assert!(plain.starts_with("# Edit with Nuzky\n") && plain.ends_with("look, speed."), "{plain}");
        assert!(plain.len() <= CLIENT_LIMIT, "{} characters", plain.len());
        assert!(DESCRIPTION.len() <= CLIENT_LIMIT && DESCRIPTION.contains("- speed: "), "{}", DESCRIPTION.len());
        let styled = instructions(Some("# Editing style\n\nKeep pauses under 120 ms.\n"));
        let style = styled.find("# Editing style").unwrap();
        assert!(styled.starts_with("This creator has their own editing style") && style <= CLIENT_LIMIT, "{style}");
        assert!(styled.find("look, speed.").unwrap() < style && styled.ends_with("Keep pauses under 120 ms."));
    }
}

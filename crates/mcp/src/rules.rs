//! Access rules of every tool, in one table.
use serde_json::Value;

#[derive(Clone, Copy)]
pub(crate) enum Reads {
    Always,
    Never,
    /// Only with these arguments, such as job(get) or a dry run.
    When(fn(&Value) -> bool),
}

pub(crate) struct Rules {
    pub name: &'static str,
    /// Reads take no run lock and are open to read-only clients.
    pub reads: Reads,
    pub destructive: bool,
    pub idempotent: bool,
    /// The job joins the caller's open run, so stopping that run cancels it.
    pub run_job: bool,
}

const fn rules(name: &'static str, reads: Reads, destructive: bool, idempotent: bool, run_job: bool) -> Rules {
    Rules { name, reads, destructive, idempotent, run_job }
}

use Reads::{Always, Never, When};

pub(crate) const TOOLS: &[Rules] = &[
    rules("get_state", Always, false, true, false),
    rules("begin_run", Never, false, false, false),
    rules("resolve_recovery", Never, true, false, false),
    rules("apply_edits", Never, true, true, false),
    rules("end_run", Never, true, false, false),
    rules("undo_run", Never, true, false, false),
    // Keeps the user's last step as a version; the project is not touched.
    rules("list_history", Always, false, true, false),
    // Restoring the same version again changes nothing more.
    rules("undo_to", Never, true, true, false),
    rules("import_media", Never, false, false, false),
    // Searching reads catalogues only; adding downloads the one sound, then edits like import_media.
    rules("search_sounds", Always, false, true, false),
    rules("add_sound", Never, false, false, false),
    rules("add_youtube_video", Never, false, false, false),
    rules("import_web_image", Never, false, false, false),
    rules("add_sticker", Never, false, false, false),
    // Only renders, or decodes a finished export of this client; a missing person outline behind a clip's background
    // is made as a job into the cache.
    rules("inspect_frames", Always, false, true, true),
    // Only renders and mixes the timeline; preparing missing sound runs as a job.
    rules("activity", Always, false, true, true),
    // Only draws the sound of the timeline and its files; preparing missing sound runs as a job.
    rules("get_waveform", Always, false, true, true),
    // Retakes, emphasis and fillers of the whole timeline only read stored words and sound and
    // answer at once; thumbnail frames only render the timeline.
    rules(
        "analyze",
        When(|args| {
            matches!(args["kind"].as_str(), Some("retakes" | "emphasis" | "thumbnail_frames"))
                || args["kind"] == "fillers" && args["asset_id"].is_null()
        }),
        false,
        false,
        true,
    ),
    // Only renders a frame and writes its mask to the cache.
    rules("segment_subject", Always, false, true, true),
    // Replaces the keyframes of the clips it moves.
    rules("reframe", Never, true, false, true),
    rules("transcribe", Never, false, false, true),
    rules("get_transcript", Always, false, true, false),
    rules("edit_transcript", When(|args| args["dry_run"] == true), true, false, false),
    // Changes how words read, never what is heard; the recognised text stays in the project.
    rules("correct_words", Never, false, false, false),
    // Cancelling touches only the caller's own job, so a read-only client can stop what it started.
    rules(
        "job",
        When(|args| args["action"] == "get" || args["action"] == "wait" || args["action"] == "cancel"),
        false,
        false,
        false,
    ),
    rules("build_captions", Never, true, false, false),
    rules("apply_zooms", Never, true, false, false),
    // Leaves clips that already move alone, so the same call again changes nothing.
    rules("apply_motion", Never, false, true, false),
    // Proposing replaces the reels proposed before; making only adds new projects beside the source.
    rules("propose_reels", Never, true, false, false),
    rules("make_reels", Never, false, false, false),
    rules("export_video", Never, false, false, true),
    // Only draws the thumbnail; a missing mask of its frame is made as a job into the cache.
    rules("inspect_thumbnail", Always, false, true, true),
    rules("inspect_image", Always, false, true, false),
    rules("export_thumbnail", Never, false, false, true),
    // Only shows choices to the user; the project is not touched.
    rules("suggest_options", Always, false, true, false),
    // The creator's style, not the project: every change is a version they can restore.
    rules("get_style", Always, false, true, false),
    rules("change_style", Never, false, false, false),
];

/// Tools that reach the internet.
pub(crate) const ONLINE: &[&str] = &["search_sounds", "add_sound", "add_youtube_video", "import_web_image"];

pub(crate) fn find(name: &str) -> Option<&'static Rules> {
    TOOLS.iter().find(|rules| rules.name == name)
}

impl Rules {
    pub fn reads(&self, arguments: &Value) -> bool {
        match self.reads {
            Always => true,
            Never => false,
            When(reads) => reads(arguments),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeSet;

    #[test]
    fn every_tool_is_classified_once_and_rules_are_unchanged() {
        let catalog: BTreeSet<_> = crate::catalog().unwrap().into_iter().map(|tool| tool.name.to_string()).collect();
        let table: BTreeSet<_> = TOOLS.iter().map(|rules| rules.name.to_string()).collect();
        assert_eq!(catalog, table);
        assert_eq!(table.len(), TOOLS.len(), "a tool is listed twice");
        let names =
            |keep: fn(&Rules) -> bool| TOOLS.iter().filter(|r| keep(r)).map(|r| r.name).collect::<BTreeSet<_>>();
        let set = |names: &[&'static str]| names.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(
            names(|r| matches!(r.reads, Always)),
            set(&[
                "get_state",
                "get_transcript",
                "inspect_frames",
                "activity",
                "get_waveform",
                "segment_subject",
                "inspect_thumbnail",
                "inspect_image",
                "suggest_options",
                "list_history",
                "get_style",
                "search_sounds"
            ])
        );
        assert_eq!(
            names(|r| r.destructive),
            set(&[
                "apply_edits",
                "edit_transcript",
                "end_run",
                "undo_run",
                "undo_to",
                "build_captions",
                "apply_zooms",
                "propose_reels",
                "reframe",
                "resolve_recovery"
            ])
        );
        assert_eq!(
            names(|r| r.idempotent),
            set(&[
                "get_state",
                "get_transcript",
                "inspect_frames",
                "activity",
                "get_waveform",
                "segment_subject",
                "inspect_thumbnail",
                "inspect_image",
                "apply_edits",
                "apply_motion",
                "suggest_options",
                "list_history",
                "undo_to",
                "get_style",
                "search_sounds"
            ])
        );
        assert_eq!(
            names(|r| r.run_job),
            set(&[
                "analyze",
                "activity",
                "get_waveform",
                "inspect_frames",
                "segment_subject",
                "inspect_thumbnail",
                "transcribe",
                "reframe",
                "export_video",
                "export_thumbnail"
            ])
        );
        let job = find("job").unwrap();
        assert!(
            job.reads(&json!({"action":"get"}))
                && job.reads(&json!({"action":"cancel"}))
                && job.reads(&json!({"action":"wait"}))
        );
        let edit = find("edit_transcript").unwrap();
        assert!(edit.reads(&json!({"dry_run":true})) && !edit.reads(&json!({})));
        let analyze = find("analyze").unwrap();
        assert!(analyze.reads(&json!({"kind":"retakes"})) && !analyze.reads(&json!({"kind":"silences"})));
        assert!(analyze.reads(&json!({"kind":"emphasis"})) && analyze.reads(&json!({"kind":"thumbnail_frames"})));
        assert!(analyze.reads(&json!({"kind":"fillers"})) && !analyze.reads(&json!({"kind":"fillers","asset_id":"a"})));
        assert!(find("unknown").is_none());
    }
}

//! Plugins as the app runs them, on generated media: the guide's examples, Stop and the dry run.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use nuzky_engine::{
    Project,
    edit::{EditCmd, new_id},
    model::ClipContent,
};
use nuzky_mcp::plugins;
use nuzky_session::{Expect, Mode, ProjectSession, host::Host};
use serde_json::{Map, Value, json};

/// Nuzky's data folder for this test binary, so plugins and transcripts never touch the user's.
fn data() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("plugins-data");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: every test calls this first, before anything reads the environment, and only the first sets it.
        unsafe { std::env::set_var("NUZKY_DATA_DIR", &dir) };
        dir
    })
}

struct Fixture {
    host: Arc<Host>,
    dir: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A 4 s phone-sized video on the main track, split into clips of 0.5, 2.5, 0.3 and 0.7 s.
fn project() -> Fixture {
    data();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("plugins-{}", new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let video = dir.join("take.mp4");
    assert!(
        Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=s=108x192:r=30:d=4"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4", "-c:v", "libx264", "-preset", "ultrafast"])
            .args(["-c:a", "aac", "-shortest"])
            .arg(&video)
            .status()
            .unwrap()
            .success()
    );
    let path = dir.join("project.nuzky");
    std::fs::write(&path, serde_json::to_vec(&Project::new("Plugins")).unwrap()).unwrap();
    let host = Arc::new(Host::new(ProjectSession::open(&path, Mode::Write, None).unwrap(), dir.join("cache")).unwrap());
    let asset = nuzky_engine::media::probe(&video, "take".into()).unwrap();
    let edit = |cmds| host.session.edit(cmds, None, Expect::default()).unwrap();
    edit(vec![
        EditCmd::AddAssets { assets: vec![asset] },
        EditCmd::AddClip { asset_id: "take".into(), start_us: None, track_id: None },
    ]);
    for at_us in [3_300_000, 3_000_000, 500_000] {
        let clip_id = main(&host).iter().find(|c| c.start_us < at_us && c.end_us() > at_us).unwrap().id.clone();
        edit(vec![EditCmd::SplitClip { clip_id, at_us }]);
    }
    assert_eq!(durations(&host), [500_000, 2_500_000, 300_000, main(&host)[3].duration_us]);
    Fixture { host, dir }
}

fn main(host: &Host) -> Vec<nuzky_engine::model::Clip> {
    host.session.state().unwrap().project.tracks[0].clips.clone()
}

fn durations(host: &Host) -> Vec<i64> {
    main(host).iter().map(|c| c.duration_us).collect()
}

fn install(id: &str) {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/nuzky-plugin/examples").join(id);
    let read = |file: &str| std::fs::read_to_string(folder.join(file)).unwrap();
    let manifest: Value = serde_json::from_str(&read("plugin.json")).unwrap();
    let saved = plugins::save(&manifest, &read("plugin.js"), Some(&read("README.md"))).unwrap();
    assert_eq!(saved.status, "ok", "{:?}", saved.error);
}

fn inputs(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

/// The finished job, polled as the app's job events would report it.
fn finish(host: &Host, job: &Value, within: Duration) -> Value {
    let deadline = Instant::now() + within;
    loop {
        let status = host.jobs.get(job["job_id"].as_str().unwrap(), false).unwrap();
        if status["status"] != "running" {
            return status;
        }
        assert!(Instant::now() < deadline, "the plugin still runs: {status}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Both examples of the guide, as a user runs them from the Plugins panel, on real media: each is one run labelled
/// after the plugin and one undo step. A renamed edit or field in the engine breaks them here.
#[test]
fn the_guide_examples_edit_generated_media() {
    let f = project();
    install("remove-short-clips");
    install("follow-end-card");
    let before = f.host.session.state().unwrap().project;

    let job =
        plugins::run_in_app(&f.host, "remove-short-clips", &inputs(json!({"minSeconds": 0.6})), vec![], 0).unwrap();
    let done = finish(&f.host, &job, Duration::from_secs(10));
    assert_eq!(done["status"], "done", "{done}");
    assert_eq!(done["result"]["message"], "Removed 2 clips.");
    let kept = main(&f.host);
    assert_eq!(durations(&f.host), [2_500_000, before.tracks[0].clips[3].duration_us]);
    assert_eq!((kept[0].start_us, kept[1].start_us), (0, 2_500_000));
    assert!(f.host.session.state().unwrap().open_run.is_none());
    assert_eq!(f.host.session.list_history(1).unwrap().versions[0].label, "Plugin: Remove short clips");
    f.host.session.undo().unwrap();
    assert_eq!(f.host.session.state().unwrap().project, before);

    let end = before.duration_us();
    let job = plugins::run_in_app(
        &f.host,
        "follow-end-card",
        &inputs(json!({"text": "Follow me", "seconds": 2, "color": "#FFCC00"})),
        vec![],
        0,
    )
    .unwrap();
    assert_eq!(finish(&f.host, &job, Duration::from_secs(10))["status"], "done");
    let project = f.host.session.state().unwrap().project;
    let cards: Vec<_> = project.tracks.iter().skip(1).flat_map(|t| &t.clips).collect();
    assert_eq!(cards.len(), 1);
    assert_eq!((cards[0].start_us, cards[0].duration_us), (end - 2_000_000, 2_000_000));
    let ClipContent::Text { text, style, transform, .. } = &cards[0].content else { panic!("not a text") };
    assert_eq!((text.as_str(), style.color.as_str(), style.bold, transform.y), ("Follow me", "#FFCC00", true, 0.3));
    assert_eq!(project.tracks[0], before.tracks[0]);
    f.host.session.undo().unwrap();
    assert_eq!(f.host.session.state().unwrap().project, before);
}

/// Stop in the AI bar ends a plugin stuck in a loop within two seconds and nothing changes; a dry run answers the
/// project the edits would make and leaves the real one alone.
#[test]
fn stop_ends_a_stuck_plugin_and_a_dry_run_changes_nothing() {
    let f = project();
    install("remove-short-clips");
    let spin = json!({"nuzkyPlugin": 1, "id": "spin", "name": "Spin", "description": "Never ends", "version": "1"});
    plugins::save(&spin, "function run(ctx) {\n  while (true) {}\n}\n", None).unwrap();
    let before = f.host.session.state().unwrap().project;

    let dry = plugins::dry_run_in_app(&f.host, "remove-short-clips", &Map::new(), vec![], 0, Arc::default()).unwrap();
    assert_eq!(dry["valid"], true, "{dry}");
    assert_eq!(dry["edits"][0]["type"], "deleteClips");
    let after: Project = serde_json::from_value(dry["project"].clone()).unwrap();
    assert_eq!(after.tracks[0].clips.len(), 1);
    assert_eq!(f.host.session.state().unwrap().project, before);

    let job = plugins::run_in_app(&f.host, "spin", &Map::new(), vec![], 0).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(f.host.session.state().unwrap().open_run.unwrap().label, "Plugin: Spin");
    let stopping = Instant::now();
    f.host.stop_run().unwrap();
    let stopped = finish(&f.host, &job, Duration::from_secs(2));
    assert_eq!(stopped["status"], "cancelled", "{stopped}");
    assert!(stopping.elapsed() < Duration::from_secs(2), "stopped after {:?}", stopping.elapsed());
    let state = f.host.session.state().unwrap();
    assert!(state.open_run.is_none());
    assert_eq!(state.project, before);
}

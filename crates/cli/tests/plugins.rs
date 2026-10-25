//! Plugins as the app runs them, on generated media: the guide's examples, Stop and the dry run.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};
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
        plugins::use_sandbox_program(env!("CARGO_BIN_EXE_nuzky").into());
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

/// Stop in the AI bar ends a plugin stuck in a loop within two seconds and nothing changes, as closing the plugin ends
/// its preview; a dry run answers the project the edits would make and leaves the real one alone.
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

    let cancel = Arc::new(AtomicBool::new(false));
    let (host, stop) = (f.host.clone(), cancel.clone());
    let preview = std::thread::spawn(move || plugins::dry_run_in_app(&host, "spin", &Map::new(), vec![], 0, stop));
    std::thread::sleep(Duration::from_millis(300));
    let closing = Instant::now();
    cancel.store(true, Ordering::Relaxed);
    let error = preview.join().unwrap().unwrap_err();
    assert!(format!("{error:#}").starts_with("CANCELLED"), "{error:#}");
    assert!(closing.elapsed() < Duration::from_secs(1), "stopped after {:?}", closing.elapsed());
}

/// A plugin cannot grow the project without bound by copying a long text, which would take all memory already in
/// the preview, and saving a plugin whose folder holds a link never writes through the link.
#[test]
fn a_plugin_cannot_blow_up_the_project_or_write_through_a_link() {
    let f = project();
    let long = "x".repeat(1 << 20);
    let style = serde_json::from_value(json!({"fontSize": 60.0, "color": "#FFFFFF"})).unwrap();
    f.host.session.edit(vec![EditCmd::AddText { start_us: 0, text: long, style }], None, Expect::default()).unwrap();
    let copies =
        json!({"nuzkyPlugin": 1, "id": "copies", "name": "Copies", "description": "Copies the text", "version": "1"});
    let script = "function run(ctx) {\n  const id = ctx.project.tracks[1].clips[0].id;\n  \
                  return Array.from({length: 100}, () => ({type: 'duplicateClip', clipId: id}));\n}\n";
    plugins::save(&copies, script, None).unwrap();
    let before = f.host.session.state().unwrap().project;
    let dry = plugins::dry_run_in_app(&f.host, "copies", &Map::new(), vec![], 0, Arc::default()).unwrap();
    assert_eq!(dry["valid"], false, "{}", dry["error"]);
    assert!(dry["error"].as_str().unwrap().starts_with("PLUGIN_TOO_BIG: edit 63"), "{}", dry["error"]);
    assert_eq!(f.host.session.state().unwrap().project, before);

    #[cfg(unix)]
    {
        let video = f.dir.join("take.mp4");
        let original = std::fs::read(&video).unwrap();
        let folder = plugins::dir().unwrap().join("copies");
        std::fs::remove_file(folder.join("plugin.prev.js")).ok();
        std::os::unix::fs::symlink(&video, folder.join("plugin.prev.js")).unwrap();
        plugins::save(&copies, script, None).unwrap();
        assert_eq!(std::fs::read(&video).unwrap(), original, "saving wrote through the link into the video");
        assert!(!std::fs::symlink_metadata(folder.join("plugin.prev.js")).unwrap().is_symlink());
    }
}

/// One edit that copies a value into every caption cannot take all memory before the size check sees it, and a
/// README that is a link to another file never reaches the agent.
#[test]
fn a_plugin_cannot_spread_a_huge_text_or_read_through_a_link() {
    let f = project();
    let segments: Vec<_> = (0..200)
        .map(|i| json!({"startUs": i * 20_000, "endUs": i * 20_000 + 20_000, "text": format!("w{i}")}))
        .collect();
    let captions =
        json!({"type": "addCaptions", "segments": segments, "style": {"fontSize": 60.0, "color": "#FFFFFF"}});
    f.host.session.edit(vec![serde_json::from_value(captions).unwrap()], None, Expect::default()).unwrap();
    let restyle = json!({"nuzkyPlugin": 1, "id": "restyle", "name": "Restyle", "description": "Restyles captions", "version": "1"});
    let script = "function run(ctx) {\n  const track = ctx.project.tracks.find((t) => t.kind === 'text');\n  \
                  return [{type: 'restyleTrack', trackId: track.id, style: {fontFamily: 'x'.repeat(1 << 20)}}];\n}\n";
    plugins::save(&restyle, script, None).unwrap();
    let error = match plugins::dry_run_in_app(&f.host, "restyle", &Map::new(), vec![], 0, Arc::default()) {
        Ok(dry) => dry["error"].to_string(),
        Err(error) => format!("{error:#}"),
    };
    assert!(error.contains("text longer than"), "{error}");

    #[cfg(unix)]
    {
        let secret = f.dir.join("secret.env");
        std::fs::write(&secret, "TOKEN=hunter2").unwrap();
        let folder = plugins::dir().unwrap().join("restyle");
        std::os::unix::fs::symlink(&secret, folder.join("README.md")).unwrap();
        let listed = plugins::list().unwrap().into_iter().find(|p| p.id == "restyle").unwrap();
        assert!(listed.readme.is_none_or(|readme| !readme.contains("hunter2")), "the README link was read");
    }
}

/// Cancelling the job after the script returned, while its edits are still checked, applies none of them: a
/// cancelled job reports no result, so nothing it did may land.
#[test]
fn a_cancelled_plugin_job_applies_nothing() {
    let f = project();
    let segments: Vec<_> = (0..3000)
        .map(|i| json!({"startUs": i * 1_000, "endUs": i * 1_000 + 1_000, "text": format!("caption number {i}")}))
        .collect();
    let captions =
        json!({"type": "addCaptions", "segments": segments, "style": {"fontSize": 60.0, "color": "#FFFFFF"}});
    f.host.session.edit(vec![serde_json::from_value(captions).unwrap()], None, Expect::default()).unwrap();
    let renames =
        json!({"nuzkyPlugin": 1, "id": "renames", "name": "Renames", "description": "Renames a lot", "version": "1"});
    let script = "function run(ctx) {\n  const edits = Array.from({length: 5000}, (_, i) => ({type: 'renameProject', name: 'n' + i}));\n  \
                  nuzky.progress(1);\n  return edits;\n}\n";
    plugins::save(&renames, script, None).unwrap();
    let before = f.host.session.state().unwrap().project;
    let job = plugins::run_in_app(&f.host, "renames", &Map::new(), vec![], 0).unwrap();
    let id = job["job_id"].as_str().unwrap();
    while f.host.jobs.get(id, false).unwrap()["progress"] != 1.0 {
        std::thread::sleep(Duration::from_millis(1));
    }
    f.host.jobs.get(id, true).unwrap();
    assert_eq!(finish(&f.host, &job, Duration::from_secs(10))["status"], "cancelled");
    assert_eq!(f.host.session.state().unwrap().project, before, "the cancelled job's edits landed");
}

/// Pasting a clip with 50,000 keyframes onto 1,500 captions would copy about 4 GB before any check of the project's
/// size sees it; the plugin's own process hits its limit instead, and the app keeps its memory and the project.
#[test]
fn a_plugin_whose_edits_need_gigabytes_runs_out_of_its_own_memory() {
    let f = project();
    let segments: Vec<_> = (0..1500)
        .map(|i| json!({"startUs": i * 50_000, "endUs": i * 50_000 + 50_000, "text": format!("w{i}")}))
        .collect();
    let captions =
        json!({"type": "addCaptions", "segments": segments, "style": {"fontSize": 60.0, "color": "#FFFFFF"}});
    f.host.session.edit(vec![serde_json::from_value(captions).unwrap()], None, Expect::default()).unwrap();
    let bomb = json!({"nuzkyPlugin": 1, "id": "bomb", "name": "Bomb", "description": "Pastes a lot", "version": "1"});
    let script = "function run(ctx) {\n  const track = ctx.project.tracks.find((t) => t.kind === 'text');\n  \
                  const transform = {x: 0, y: 0, scale: 1, rotation: 0, opacity: 1};\n  \
                  const keyframes = Array.from({length: 50000}, (_, i) => ({tUs: i, transform}));\n  \
                  const source = {clip: {...track.clips[0], keyframes}, trackId: track.id, trackKind: 'text'};\n  \
                  return [{type: 'pasteAttributes', source, clipIds: track.clips.map((c) => c.id)}];\n}\n";
    plugins::save(&bomb, script, None).unwrap();
    let before = f.host.session.state().unwrap().project;

    let started = Instant::now();
    let error = plugins::dry_run_in_app(&f.host, "bomb", &Map::new(), vec![], 0, Arc::default()).unwrap_err();
    assert!(format!("{error:#}").starts_with("PLUGIN_OUT_OF_MEMORY"), "{error:#}");
    assert!(started.elapsed() < Duration::from_secs(10), "stopped after {:?}", started.elapsed());

    let job = plugins::run_in_app(&f.host, "bomb", &Map::new(), vec![], 0).unwrap();
    let failed = finish(&f.host, &job, Duration::from_secs(10));
    assert_eq!(failed["status"], "failed", "{failed}");
    assert!(failed["error"].as_str().unwrap().starts_with("PLUGIN_OUT_OF_MEMORY"), "{failed}");
    let state = f.host.session.state().unwrap();
    assert!(state.open_run.is_none());
    assert_eq!(state.project, before);
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let peak = status.lines().find_map(|line| line.strip_prefix("VmHWM:")).unwrap();
        let peak: u64 = peak.trim().trim_end_matches(" kB").parse().unwrap();
        assert!(peak < 512 << 10, "the test process took {} MB", peak >> 10);
    }
}

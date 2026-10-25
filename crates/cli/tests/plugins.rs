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

    let job = plugins::run_in_app(&f.host, "remove-short-clips", &inputs(json!({"minSeconds": 0.6})), None, vec![], 0)
        .unwrap();
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
        None,
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
    let spin = json!({"nuzkyPlugin": 2, "id": "spin", "name": "Spin", "description": "Never ends", "version": "1"});
    plugins::save(&spin, "function run(ctx) {\n  while (true) {}\n}\n", None).unwrap();
    let before = f.host.session.state().unwrap().project;

    let dry = plugins::dry_run_in_app(&f.host, "remove-short-clips", &Map::new(), vec![], 0, Arc::default()).unwrap();
    assert_eq!(dry["valid"], true, "{dry}");
    assert_eq!(dry["edits"][0]["type"], "deleteClips");
    let after: Project = serde_json::from_value(dry["project"].clone()).unwrap();
    assert_eq!(after.tracks[0].clips.len(), 1);
    assert_eq!(f.host.session.state().unwrap().project, before);

    let job = plugins::run_in_app(&f.host, "spin", &Map::new(), None, vec![], 0).unwrap();
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
        json!({"nuzkyPlugin": 2, "id": "copies", "name": "Copies", "description": "Copies the text", "version": "1"});
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
    let restyle = json!({"nuzkyPlugin": 2, "id": "restyle", "name": "Restyle", "description": "Restyles captions", "version": "1"});
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
        json!({"nuzkyPlugin": 2, "id": "renames", "name": "Renames", "description": "Renames a lot", "version": "1"});
    let script = "function run(ctx) {\n  const edits = Array.from({length: 5000}, (_, i) => ({type: 'renameProject', name: 'n' + i}));\n  \
                  nuzky.progress(1);\n  return edits;\n}\n";
    plugins::save(&renames, script, None).unwrap();
    let before = f.host.session.state().unwrap().project;
    let job = plugins::run_in_app(&f.host, "renames", &Map::new(), None, vec![], 0).unwrap();
    let id = job["job_id"].as_str().unwrap();
    while f.host.jobs.get(id, false).unwrap()["progress"] != 1.0 {
        std::thread::sleep(Duration::from_millis(1));
    }
    f.host.jobs.get(id, true).unwrap();
    assert_eq!(finish(&f.host, &job, Duration::from_secs(10))["status"], "cancelled");
    assert_eq!(f.host.session.state().unwrap().project, before, "the cancelled job's edits landed");
}

/// A plugin's script never runs in the app's webview, so it cannot reach the app's commands or the network: each of
/// these fails as PLUGIN_FAILED from the panel's button, and the rename after it never lands.
#[test]
fn a_plugin_reaches_neither_the_app_nor_the_network() {
    let f = project();
    let reaches = ["tauri", "window", "fetch", "xhr", "socket"];
    let options: Vec<_> = reaches.iter().map(|id| json!({"value": id, "label": id})).collect();
    let hostile = json!({"nuzkyPlugin": 2, "id": "hostile", "name": "Hostile", "description": "Reaches out",
        "version": "1", "inputs": [{"type": "choice", "id": "reach", "label": "Reach", "options": options, "default": "tauri"}]});
    let script = r#"function run(ctx) {
  const reach = {
    tauri: () => __TAURI_INTERNALS__.invoke("plugin:fs|write_file", {path: "/tmp/x"}),
    window: () => window.__TAURI_INTERNALS__.invoke("save_project"),
    fetch: () => fetch("http://127.0.0.1:9/"),
    xhr: () => new XMLHttpRequest().open("GET", "http://127.0.0.1:9/"),
    socket: () => new WebSocket("ws://127.0.0.1:9/"),
  }[ctx.inputs.reach];
  reach();
  return [{type: "renameProject", name: "Reached " + ctx.inputs.reach}];
}"#;
    plugins::save(&hostile, script, None).unwrap();
    let before = f.host.session.state().unwrap().project;
    for reach in reaches {
        let job = plugins::run_in_app(&f.host, "hostile", &inputs(json!({"reach": reach})), None, vec![], 0).unwrap();
        let done = finish(&f.host, &job, Duration::from_secs(10));
        let error = done["error"].as_str().unwrap_or_default();
        assert!(error.starts_with("PLUGIN_FAILED: plugin.js:") && error.contains("is not defined"), "{reach}: {done}");
        let state = f.host.session.state().unwrap();
        assert!(state.open_run.is_none());
        assert_eq!(state.project, before, "{reach} changed the project");
    }
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
    let bomb = json!({"nuzkyPlugin": 2, "id": "bomb", "name": "Bomb", "description": "Pastes a lot", "version": "1"});
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

    let job = plugins::run_in_app(&f.host, "bomb", &Map::new(), None, vec![], 0).unwrap();
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

/// A phone-sized video of `seconds` whose sound is the FFmpeg `aevalsrc` expression `sound` in `codec`, on the main
/// track from 0. PCM keeps the quiet exactly where the expression puts it.
fn with_sound(sound: &str, seconds: u32, codec: &str) -> Fixture {
    data();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("plugins-{}", new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let video = dir.join("take.mov");
    assert!(
        Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg(format!("color=s=108x192:r=30:d={seconds}"))
            .args(["-f", "lavfi", "-i"])
            .arg(format!("aevalsrc={sound}:s=48000:c=stereo:d={seconds}"))
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-shortest", "-c:a", codec])
            .arg(&video)
            .status()
            .unwrap()
            .success()
    );
    let path = dir.join("project.nuzky");
    std::fs::write(&path, serde_json::to_vec(&Project::new("Sound")).unwrap()).unwrap();
    let host = Arc::new(Host::new(ProjectSession::open(&path, Mode::Write, None).unwrap(), dir.join("cache")).unwrap());
    let asset = nuzky_engine::media::probe(&video, "take".into()).unwrap();
    host.session
        .edit(
            vec![
                EditCmd::AddAssets { assets: vec![asset] },
                EditCmd::AddClip { asset_id: "take".into(), start_us: None, track_id: None },
            ],
            None,
            Expect::default(),
        )
        .unwrap();
    Fixture { host, dir }
}

fn edit(host: &Host, edit: Value) {
    host.session.edit(vec![serde_json::from_value(edit).unwrap()], None, Expect::default()).unwrap();
}

/// Within one window of the silence finder.
fn near(actual: i64, expected: i64) -> bool {
    (actual - expected).abs() <= 10_000
}

fn source_in(clip: &nuzky_engine::model::Clip) -> i64 {
    match &clip.content {
        ClipContent::Media { source_in_us, .. } => *source_in_us,
        _ => panic!("not media"),
    }
}

/// The guide's Remove pauses on a take with a 1 s pause at 1.5 s and a 0.3 s one at 3.3 s of the file, whose first
/// 0.5 s is trimmed off: the silences arrive in timeline time, so the plugin cuts the long pause where the timeline
/// has it, keeping 0.1 s on each side, and leaves the short one. The preview finds nothing before the app has
/// prepared the sound; the run prepares it itself, as part of its job.
#[test]
fn remove_pauses_cuts_the_silences_where_the_timeline_has_them() {
    let f = with_sound("0.5*sin(2*PI*440*t)*(lt(t\\,1.5)+between(t\\,2.5\\,3.3)+gte(t\\,3.6))", 6, "pcm_s16le");
    let clip = main(&f.host)[0].id.clone();
    edit(
        &f.host,
        json!({"type": "trimClip", "clipId": clip, "startUs": 0, "durationUs": 5_500_000, "sourceInUs": 500_000}),
    );
    install("remove-pauses");
    let before = f.host.session.state().unwrap().project;

    let early = plugins::dry_run_in_app(&f.host, "remove-pauses", &Map::new(), vec![], 0, Arc::default()).unwrap_err();
    assert!(format!("{early:#}").starts_with("AUDIO_NOT_READY: "), "{early:#}");

    let job = plugins::run_in_app(&f.host, "remove-pauses", &Map::new(), None, vec![], 0).unwrap();
    let done = finish(&f.host, &job, Duration::from_secs(20));
    assert_eq!(done["status"], "done", "{done}");
    assert_eq!(done["result"]["message"], "Removed 1 pause, 0.8\u{a0}s in all.");
    // The pause is 1.0-2.0 s of the timeline, so 1.1-1.9 s goes.
    let clips = main(&f.host);
    assert_eq!(clips.len(), 2, "{clips:?}");
    assert!(near(clips[0].duration_us, 1_100_000) && source_in(&clips[0]) == 500_000, "{clips:?}");
    assert!(near(clips[1].start_us, 1_100_000) && near(source_in(&clips[1]), 2_400_000), "{clips:?}");
    assert!(near(clips[1].end_us(), 4_700_000), "{clips:?}");
    assert_eq!(f.host.session.list_history(1).unwrap().versions[0].label, "Plugin: Remove pauses");
    f.host.session.undo().unwrap();
    assert_eq!(f.host.session.state().unwrap().project, before);

    // The sound is prepared now, so the preview finds both pauses when asked for short ones.
    let short = inputs(json!({"minSeconds": 0.2, "keepSeconds": 0}));
    let both = plugins::dry_run_in_app(&f.host, "remove-pauses", &short, vec![], 0, Arc::default()).unwrap();
    let ranges = both["edits"][0]["ranges"].as_array().unwrap().clone();
    let at = |i: usize, key: &str| ranges[i][key].as_i64().unwrap();
    assert_eq!(ranges.len(), 2, "{ranges:?}");
    assert!(near(at(0, "startUs"), 1_000_000) && near(at(0, "endUs"), 2_000_000), "{ranges:?}");
    assert!(near(at(1, "startUs"), 2_800_000) && near(at(1, "endUs"), 3_100_000), "{ranges:?}");

    // A cut in the middle of the long pause gives one silence per clip; it is still one pause.
    edit(&f.host, json!({"type": "splitClip", "clipId": clip, "atUs": 1_500_000}));
    let split = plugins::dry_run_in_app(&f.host, "remove-pauses", &Map::new(), vec![], 0, Arc::default()).unwrap();
    assert_eq!(split["message"], "Removed 1 pause, 0.8\u{a0}s in all.", "{split}");
}

/// A clip that starts inside a loudness window hears only what it plays: the file is loud for its first 50 ms, the
/// clip plays it from 50 ms, so its first window is silent.
#[test]
fn loudness_of_a_trimmed_clip_leaves_out_what_it_does_not_play() {
    let f = with_sound("0.5*sin(2*PI*440*t)*lt(t\\,0.05)", 2, "pcm_s16le");
    let clip = main(&f.host)[0].id.clone();
    edit(
        &f.host,
        json!({"type": "trimClip", "clipId": clip, "startUs": 0, "durationUs": 1_000_000, "sourceInUs": 50_000}),
    );
    let reads = json!({"nuzkyPlugin": 2, "id": "loud", "name": "Loud", "description": "Logs the loudness",
        "version": "1", "needs": ["loudness"]});
    let script = "function run(ctx) { nuzky.log(JSON.stringify(ctx.loudness[0])); return []; }";
    plugins::save(&reads, script, None).unwrap();
    let job = plugins::run_in_app(&f.host, "loud", &Map::new(), None, vec![], 0).unwrap();
    let done = finish(&f.host, &job, Duration::from_secs(20));
    assert_eq!(done["status"], "done", "{done}");
    let loudness: Value = serde_json::from_str(done["result"]["logs"][0].as_str().unwrap()).unwrap();
    assert_eq!(loudness["times_us"][0], 0, "{loudness}");
    assert_eq!(loudness["dbfs"][0], -120.0, "{loudness}");
    assert_eq!(loudness["times_us"].as_array().unwrap().len(), 10, "{loudness}");
}

/// Beats and loudness on the timeline: a click every 0.5 s of the file, played from 2 s at double speed, is a beat
/// every 0.25 s of the timeline at 240 BPM; loudness windows start at the clip's start, half as long, and are quiet
/// where the file is.
#[test]
fn beats_and_loudness_arrive_in_timeline_time() {
    let f = with_sound("if(between(t\\,6\\,7)\\,0\\,0.8*exp(-60*mod(t\\,0.5))*sin(2*PI*1000*t))", 20, "pcm_s16le");
    let clip = main(&f.host)[0].id.clone();
    edit(
        &f.host,
        json!({"type": "trimClip", "clipId": clip, "startUs": 0, "durationUs": 18_000_000, "sourceInUs": 2_000_000}),
    );
    edit(&f.host, json!({"type": "updateClip", "clipId": clip, "speed": 2.0}));
    assert_eq!(main(&f.host)[0].duration_us, 9_000_000);
    let reads = json!({"nuzkyPlugin": 2, "id": "reads", "name": "Reads", "description": "Logs what it reads",
        "version": "1", "needs": ["beats", "loudness"]});
    let script = r#"function run(ctx) {
  const b = ctx.beats[0], l = ctx.loudness[0];
  nuzky.log(JSON.stringify({clips: [ctx.beats.length, ctx.loudness.length], same: b.clip_id === l.clip_id,
    track: b.track_id, bpm: b.bpm, confidence: b.confidence, beats: b.beats_us.slice(0, 6), count: b.beats_us.length,
    last: b.beats_us[b.beats_us.length - 1], down: b.downbeats_us.length, span: [b.start_us, b.end_us]}));
  nuzky.log(JSON.stringify({times: l.times_us.slice(0, 3), count: l.times_us.length, values: l.dbfs.length,
    quiet: l.dbfs[Math.floor(2200000 / 50000)], loud: Math.max(...l.dbfs.slice(0, 20))}));
  return [];
}"#;
    plugins::save(&reads, script, None).unwrap();
    let job = plugins::run_in_app(&f.host, "reads", &Map::new(), None, vec![], 0).unwrap();
    let done = finish(&f.host, &job, Duration::from_secs(20));
    assert_eq!(done["status"], "done", "{done}");
    let logs = done["result"]["logs"].as_array().unwrap();
    let beats: Value = serde_json::from_str(logs[0].as_str().unwrap()).unwrap();
    let loudness: Value = serde_json::from_str(logs[1].as_str().unwrap()).unwrap();
    assert_eq!((&beats["clips"], &beats["same"], &beats["track"]), (&json!([1, 1]), &json!(true), &json!("main")));
    assert_eq!(beats["span"], json!([0, 9_000_000]), "{beats}");
    let bpm = beats["bpm"].as_f64().unwrap();
    assert!((bpm - 240.0).abs() < 1.0 && beats["confidence"].as_f64().unwrap() > 0.3, "{beats}");
    // The click at 2 s of the file is found a hair before it, outside the clip, so the first beat is the next one.
    for (i, beat) in beats["beats"].as_array().unwrap().iter().enumerate() {
        assert!((beat.as_i64().unwrap() - (i as i64 + 1) * 250_000).abs() <= 15_000, "{beats}");
    }
    assert!((34..=36).contains(&beats["count"].as_i64().unwrap()), "{beats}");
    assert!(beats["last"].as_i64().unwrap() < 9_000_000, "{beats}");
    assert!((8..=10).contains(&beats["down"].as_i64().unwrap()), "{beats}");
    // 0.1 s of the file is 0.05 s of the timeline, and 6-7 s of the file is 2-2.5 s of the timeline.
    assert_eq!(loudness["times"], json!([0, 50_000, 100_000]), "{loudness}");
    assert_eq!((&loudness["count"], &loudness["values"]), (&json!(180), &json!(180)), "{loudness}");
    assert_eq!(loudness["quiet"], -120.0, "{loudness}");
    assert!(loudness["loud"].as_f64().unwrap() > -20.0, "{loudness}");
}

/// Stop in the AI bar ends a plugin within two seconds while Nuzky still prepares a long file's sound for it, and
/// nothing changes.
#[test]
fn stop_ends_a_plugin_while_it_prepares_the_sound() {
    let f = with_sound("0.3*sin(2*PI*220*t)", 1800, "aac");
    let waits = json!({"nuzkyPlugin": 2, "id": "waits", "name": "Waits", "description": "Needs the beat",
        "version": "1", "needs": ["beats"]});
    plugins::save(&waits, "function run(ctx) {\n  return [];\n}\n", None).unwrap();
    let before = f.host.session.state().unwrap().project;
    let job = plugins::run_in_app(&f.host, "waits", &Map::new(), None, vec![], 0).unwrap();
    let id = job["job_id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while f.host.jobs.get(id, false).unwrap()["progress"].as_f64().is_none_or(|p| p == 0.0) {
        assert!(Instant::now() < deadline, "no progress: {}", f.host.jobs.get(id, false).unwrap());
        std::thread::sleep(Duration::from_millis(5));
    }
    let listening = f.host.jobs.get(id, false).unwrap();
    assert_eq!(listening["phase"], "listening", "{listening}");
    let stopping = Instant::now();
    f.host.stop_run().unwrap();
    let stopped = finish(&f.host, &job, Duration::from_secs(2));
    assert_eq!(stopped["status"], "cancelled", "{stopped}");
    assert!(stopping.elapsed() < Duration::from_secs(2), "stopped after {:?}", stopping.elapsed());
    let state = f.host.session.state().unwrap();
    assert!(state.open_run.is_none());
    assert_eq!(state.project, before);
}

/// A plugin puts its own logo in the corner: Nuzky copies the picture into its own folder and adds it in the batch
/// that places it, as one undo step, a second run places the same copy, and the copy stays once the plugin is
/// deleted. Its folder may hold only plain files: a link there, even a hard one, could name one of the user's files.
#[test]
fn a_plugin_places_its_own_picture_which_outlives_it() {
    let f = project();
    let logo = f.dir.join("source-logo.png");
    let made = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=magenta:s=64x32", "-frames:v", "1"])
        .arg(&logo)
        .status()
        .unwrap();
    assert!(made.success());
    let mut manifest = json!({"nuzkyPlugin": 1, "id": "logo", "name": "Logo", "description": "Puts the logo in the corner",
        "version": "1", "files": ["logo.png"]});
    let script = "function run(ctx) {\n  const logo = ctx.files['logo.png'];\n  nuzky.log(logo.kind, logo.width, logo.height);\n  \
                  return [{type: 'addSticker', assetId: logo.assetId, startUs: 0, durationUs: 1000000,\n    \
                  transform: {x: 0.35, y: -0.35, scale: 0.2, rotation: 0, opacity: 1}}];\n}\n";
    let save = |manifest: &Value, files: &[(&str, &Path)]| {
        let files = files.iter().map(|(name, path)| (name.to_string(), path.to_path_buf())).collect();
        let effects = Default::default();
        plugins::save_with_files(manifest, Some(script), None, &files, &effects).map(drop).map_err(|e| format!("{e:#}"))
    };
    let old = save(&manifest, &[("logo.png", &logo)]).unwrap_err();
    assert!(old.starts_with("PLUGIN_BROKEN: plugin.json: files needs nuzkyPlugin 2"), "{old}");
    manifest["nuzkyPlugin"] = json!(2);
    let missing = save(&manifest, &[]).unwrap_err();
    assert!(missing.contains("give it in files"), "{missing}");
    let text = f.dir.join("text.png");
    std::fs::write(&text, "not a picture").unwrap();
    let unreadable = save(&manifest, &[("logo.png", &text)]).unwrap_err();
    assert!(unreadable.starts_with("PLUGIN_BROKEN: files: logo.png: not a picture Nuzky can read"), "{unreadable}");
    let mut twice = manifest.clone();
    twice["files"] = json!(["logo.png", "LOGO.PNG"]);
    let twice = save(&twice, &[("logo.png", &logo)]).unwrap_err();
    assert!(twice.starts_with("PLUGIN_BROKEN: plugin.json: files names LOGO.PNG twice"), "{twice}");
    save(&manifest, &[("logo.png", &logo)]).unwrap();
    let folder = plugins::dir().unwrap().join("logo");
    assert_eq!(std::fs::read(folder.join("logo.png")).unwrap(), std::fs::read(&logo).unwrap());

    #[cfg(unix)]
    let links: [fn(&Path, &Path) -> std::io::Result<()>; 2] =
        [|from, to| std::os::unix::fs::symlink(from, to), |from, to| std::fs::hard_link(from, to)];
    #[cfg(unix)]
    for link in links {
        std::fs::remove_file(folder.join("logo.png")).unwrap();
        link(&logo, &folder.join("logo.png")).unwrap();
        let error = format!("{:#}", plugins::run_in_app(&f.host, "logo", &Map::new(), None, vec![], 0).unwrap_err());
        assert!(error.starts_with("PLUGIN_BROKEN: logo.png: "), "{error}");
        save(&manifest, &[("logo.png", &logo)]).unwrap();
        assert!(!std::fs::symlink_metadata(folder.join("logo.png")).unwrap().is_symlink());
    }

    let before = f.host.session.state().unwrap().project;
    let dry = plugins::dry_run_in_app(&f.host, "logo", &Map::new(), vec![], 0, Arc::default()).unwrap();
    assert_eq!(
        (&dry["valid"], &dry["adds"], &dry["logs"]),
        (&json!(true), &json!(["logo.png"]), &json!(["image 64 32"]))
    );
    assert_eq!(f.host.session.state().unwrap().project, before);
    // Only a copy the edits place stays in Nuzky's folder; saving and previewing keep none.
    let kept_files = || std::fs::read_dir(data().join("plugin-files")).map_or(0, |dir| dir.count());
    assert_eq!(kept_files(), 0);

    for run in 1..=2 {
        let job = plugins::run_in_app(&f.host, "logo", &Map::new(), None, vec![], 0).unwrap();
        let done = finish(&f.host, &job, Duration::from_secs(10));
        assert_eq!(done["result"]["added"], if run == 1 { json!(["logo.png"]) } else { json!([]) }, "{done}");
    }
    let project = f.host.session.state().unwrap().project;
    let added: Vec<_> = project.assets.iter().filter(|a| a.name == "logo.png").collect();
    assert_eq!(added.len(), 1, "a second run added the logo again");
    let kept = PathBuf::from(&added[0].path);
    assert_eq!(kept.parent().unwrap(), dunce::canonicalize(data()).unwrap().join("plugin-files"));
    let placed = project.tracks.iter().flat_map(|t| &t.clips);
    let placed =
        placed.filter(|c| matches!(&c.content, ClipContent::Media { asset_id, .. } if *asset_id == added[0].id));
    assert_eq!(placed.count(), 2);
    assert_eq!(kept_files(), 1);

    std::fs::remove_dir_all(&folder).unwrap();
    assert_eq!(std::fs::read(&kept).unwrap(), std::fs::read(&logo).unwrap());
    f.host.session.undo().unwrap();
    f.host.session.undo().unwrap();
    assert_eq!(f.host.session.state().unwrap().project, before);
}

/// A plugin's sound may hold no picture: FFmpeg's look at a file decodes a cover of any size, so an MP3 whose cover is
/// 8192 × 4096 is refused before that, and so is MPEG-TS named .mp3, which can add a video after its header.
#[test]
fn a_plugins_sound_with_a_huge_cover_is_refused_unread() {
    let f = project();
    let manifest = json!({"nuzkyPlugin": 2, "id": "beep", "name": "Beep", "description": "Beeps",
        "version": "1", "files": ["beep.mp3"]});
    let save = |path: &Path| {
        let files = [("beep.mp3".to_string(), path.to_path_buf())].into_iter().collect();
        plugins::save_with_files(&manifest, "function run() { return []; }", None, &files).map(drop)
    };
    let ffmpeg = |args: &[&str], out: &Path| {
        let made = Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error"]).args(args).arg(out).status();
        assert!(made.unwrap().success());
    };
    let plain = f.dir.join("plain.mp3");
    ffmpeg(&["-f", "lavfi", "-i", "sine=d=1", "-c:a", "libmp3lame"], &plain);
    save(&plain).unwrap();
    let stream = f.dir.join("stream.mp3");
    ffmpeg(&["-f", "lavfi", "-i", "sine=d=1", "-c:a", "aac", "-f", "mpegts"], &stream);
    let covered = f.dir.join("covered.mp3");
    ffmpeg(
        &["-f", "lavfi", "-i", "sine=d=1", "-f", "lavfi", "-i", "color=s=8192x4096:d=1", "-map", "0", "-map", "1"]
            .into_iter()
            .chain(["-c:a", "libmp3lame", "-c:v", "png", "-frames:v", "1", "-disposition:v", "attached_pic"])
            .chain(["-id3v2_version", "3"])
            .collect::<Vec<_>>(),
        &covered,
    );
    for refused in [stream, covered] {
        let error = save(&refused).unwrap_err();
        assert!(format!("{error:#}").contains("or one with a picture in it"), "{refused:?}: {error:#}");
    }
}

/// Copying and checking a plugin's files happens in its job: Run answers at once, and Stop ends the check of a picture
/// file that holds a hundred huge frames within two seconds, with nothing added.
#[test]
fn stop_ends_the_check_of_a_plugins_files() {
    let f = project();
    let folder = plugins::dir().unwrap().join("frames");
    std::fs::create_dir_all(&folder).unwrap();
    let made = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=s=8192x3000:r=1:d=120"])
        .args(["-c:v", "png", "-f", "image2pipe"])
        .arg(folder.join("logo.png"))
        .status()
        .unwrap();
    assert!(made.success());
    let manifest = json!({"nuzkyPlugin": 2, "id": "frames", "name": "Frames", "description": "Places a logo",
        "version": "1", "files": ["logo.png"]});
    std::fs::write(folder.join("plugin.json"), manifest.to_string()).unwrap();
    std::fs::write(folder.join("plugin.js"), "function run() { return []; }").unwrap();
    let before = f.host.session.state().unwrap().project;
    let asked = Instant::now();
    let job = plugins::run_in_app(&f.host, "frames", &Map::new(), None, vec![], 0).unwrap();
    assert!(asked.elapsed() < Duration::from_secs(1), "Run waited {:?}", asked.elapsed());
    std::thread::sleep(Duration::from_millis(300));
    f.host.stop_run().unwrap();
    let done = finish(&f.host, &job, Duration::from_secs(2));
    assert_ne!(done["status"], "done", "{done}");
    assert_eq!(f.host.session.state().unwrap().project, before);
}

/// Edits that leave the project as it was make no undo step, and the run says it changed nothing, so the app's Undo for
/// a pass of plugins never takes back an edit made before it.
#[test]
fn a_plugin_whose_edits_change_nothing_says_so() {
    let f = project();
    let name = f.host.session.state().unwrap().project.name.clone();
    let same = json!({"nuzkyPlugin": 1, "id": "same", "name": "Same", "description": "Renames to the same name", "version": "1"});
    let script = format!("function run() {{ return [{{type: 'renameProject', name: {}}}]; }}", json!(name));
    plugins::save(&same, &script, None).unwrap();
    let job = plugins::run_in_app(&f.host, "same", &Map::new(), None, vec![], 0).unwrap();
    let done = finish(&f.host, &job, Duration::from_secs(10));
    assert_eq!((&done["result"]["applied"], &done["result"]["changed"]), (&json!(1), &json!(false)), "{done}");
}

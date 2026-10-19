//! A complete agent rough cut through the real CLI, without a desktop session.
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use nuzky_engine::{Project, edit::new_id, model::TrackKind};
use serde_json::{Value, json};

struct Client {
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<Value>,
    dir: PathBuf,
    seq: u64,
}
impl Client {
    fn start(root: &std::path::Path) -> Self {
        let dir = root.join(format!("tmp-test/headless-{}", new_id()));
        let data = dir.join("data/nuzky");
        let cache = dir.join("cache/nuzky");
        let home = dir.join("home");
        let runtime = dir.join("runtime");
        for path in [&data, &cache, &home, &runtime] {
            std::fs::create_dir_all(path).unwrap();
        }
        #[cfg(target_os = "macos")]
        {
            std::fs::create_dir_all(home.join("Library/Application Support")).unwrap();
            std::fs::create_dir_all(home.join("Library/Caches")).unwrap();
            std::os::unix::fs::symlink(&data, home.join("Library/Application Support/nuzky")).unwrap();
            std::os::unix::fs::symlink(&cache, home.join("Library/Caches/nuzky")).unwrap();
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_nuzky"));
        command
            .env("HOME", &home)
            .env("XDG_DATA_HOME", dir.join("data"))
            .env("XDG_CACHE_HOME", dir.join("cache"))
            .env("XDG_RUNTIME_DIR", runtime);
        let project = dir.join("project.nuzky");
        let created = command.arg("new").arg(&project).output().unwrap();
        assert!(created.status.success(), "{}", String::from_utf8_lossy(&created.stderr));
        std::fs::create_dir_all(data.join("models")).unwrap();
        for file in ["ggml-small.bin", "ggml-silero-v5.1.2.bin", "wav2vec2-base-960h.gguf"] {
            let from = root.join("tmp-test/xdg/data/nuzky/models").join(file);
            let to = data.join("models").join(file);
            std::fs::hard_link(&from, &to).or_else(|_| std::fs::copy(&from, &to).map(|_| ())).unwrap();
        }
        let mut child = Command::new(env!("CARGO_BIN_EXE_nuzky"))
            .args(["mcp", "--project"])
            .arg(project)
            .arg("--allow-write")
            .envs(command.get_envs().map(|(key, value)| (key, value.unwrap())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (tx, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap();
                let value = serde_json::from_str(&line).unwrap_or_else(|_| panic!("Non-JSON on MCP stdout: {line}"));
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        let mut client = Self { child, input, output, dir, seq: 0 };
        let init = client.rpc("initialize", json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"headless-test","version":"1"}}));
        assert_eq!(init["result"]["serverInfo"]["name"], "nuzky");
        writeln!(client.input.as_mut().unwrap(), "{}", json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .unwrap();
        client.input.as_mut().unwrap().flush().unwrap();
        client
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.seq += 1;
        writeln!(
            self.input.as_mut().unwrap(),
            "{}",
            json!({"jsonrpc":"2.0","id":self.seq,"method":method,"params":params})
        )
        .unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
        loop {
            let result = self.output.recv_timeout(Duration::from_secs(20)).expect("MCP response timed out");
            if result["id"] == self.seq {
                return result;
            }
        }
    }
    fn call(&mut self, tool: &str, args: Value) -> Value {
        let result = self.rpc("tools/call", json!({"name":tool,"arguments":args}));
        assert!(result.get("error").is_none(), "{result}");
        assert_ne!(result["result"]["isError"], true, "{result}");
        result["result"]["structuredContent"].clone()
    }
    fn wait_job(&mut self, job: Value) -> Value {
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            let status = self.call("job", json!({"job_id":job["job_id"],"action":"get"}));
            if status["status"] == "done" {
                return status;
            }
            assert_eq!(status["status"], "running", "{status}");
            assert!(Instant::now() < deadline, "Job timed out: {status}");
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    fn finish(&mut self) {
        self.input.take();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            assert!(Instant::now() < deadline, "MCP did not exit on EOF");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
#[ignore = "Requires ffprobe, tmp-test/talk.mp4 and small, Silero and English word timing models from scripts/fixtures.sh"]
fn headless_rough_cut_exports_reels_and_saves_captions() {
    let root = dunce::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap();
    let media = root.join("tmp-test/talk.mp4");
    let mut c = Client::start(&root);
    let empty = c.call("get_state", json!({}));
    assert_eq!(empty["duration_us"], 0);
    let run = c.call("begin_run", json!({"label":"import"}));
    let ids = c.call("import_media", json!({"run_id":run["run_id"],"paths":[media]}))["asset_ids"].clone();
    c.call(
        "apply_edits",
        json!({"run_id":run["run_id"],"request_id":"place","edits":[{"type":"addClip","assetId":ids[0]}]}),
    );
    c.call("end_run", json!({"run_id":run["run_id"],"action":"keep"}));
    let source = c.call("get_state", json!({}))["duration_us"].as_i64().unwrap();
    let job = c.call("transcribe", json!({"asset_ids":ids,"model":"small","language":"en"}));
    let done = c.wait_job(job);
    assert_eq!(done["result"]["assets"][0]["word_times"], "measured", "{done}");
    let transcript = c.call("get_transcript", json!({}));
    assert!(transcript["words"].as_array().unwrap().len() >= 2, "{transcript}");
    assert_eq!(transcript["untranscribed"], json!([]));
    let retakes = c.call("analyze", json!({"kind":"retakes"}));
    assert_eq!(retakes["transcript_key"], transcript["transcript_key"]);
    let mut args = json!({"transcript_key":transcript["transcript_key"],"delete":[[0,0]],"dry_run":true});
    let plan = c.call("edit_transcript", args.clone());
    assert!(plan["removed_us"].as_i64().unwrap() > 0, "{plan}");
    assert_eq!(c.call("get_state", json!({}))["duration_us"], source);
    assert_eq!(c.call("get_transcript", json!({}))["transcript_key"], transcript["transcript_key"]);
    let run = c.call("begin_run", json!({"label":"rough cut and captions"}));
    args["run_id"] = run["run_id"].clone();
    args["dry_run"] = json!(false);
    let cut = c.call("edit_transcript", args);
    assert_eq!(cut["duration_us"], plan["duration_us"]);
    let after = c.call("get_transcript", json!({}));
    assert!(after["words"].as_array().unwrap().len() < transcript["words"].as_array().unwrap().len());
    let captions = c.call("build_captions", json!({"run_id":run["run_id"],"style_preset":"reel"}));
    assert!(captions["caption_count"].as_u64().unwrap() > 0, "{captions}");
    c.call("end_run", json!({"run_id":run["run_id"],"action":"keep"}));
    let state = c.call("get_state", json!({}));
    let output = c.dir.join("rough-cut.mp4");
    let job = c.call("export_video", json!({"path":output,"preset":"reels"}));
    let exported = c.wait_job(job);
    assert_eq!(exported["result"]["duration_us"], state["duration_us"]);
    c.finish();

    let saved: Project = serde_json::from_slice(&std::fs::read(c.dir.join("project.nuzky")).unwrap()).unwrap();
    let duration = saved.duration_us();
    assert_eq!(json!(duration), state["duration_us"]);
    assert_eq!(json!(duration), cut["duration_us"]["after"]);
    assert!(duration > 0 && duration < source);
    assert_eq!(serde_json::to_value(&saved.tracks).unwrap(), state["tracks"]);
    let track = saved
        .tracks
        .iter()
        .find(|t| t.name == "Captions" && t.kind == TrackKind::Text)
        .expect("Saved project must have a Captions text track");
    assert!(!track.clips.is_empty());
    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-show_format", "-of", "json"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(probe.status.success(), "{}", String::from_utf8_lossy(&probe.stderr));
    let probe: Value = serde_json::from_slice(&probe.stdout).unwrap();
    let streams = probe["streams"].as_array().unwrap();
    let video = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
    let audio = streams.iter().find(|s| s["codec_type"] == "audio").unwrap();
    assert_eq!(video["codec_name"], "h264");
    assert_eq!((video["width"].clone(), video["height"].clone()), (json!(1080), json!(1920)));
    assert_eq!(video["avg_frame_rate"], "30/1");
    assert_eq!(audio["codec_name"], "aac");
    let exported_us = probe["format"]["duration"].as_str().unwrap().parse::<f64>().unwrap() * 1_000_000.0;
    assert!((exported_us - duration as f64).abs() <= 1_000_000.0 / 30.0 + 1.0, "{probe}");
    assert!(exported_us < source as f64);
    eprintln!(
        "Headless rough cut: source {source} us, saved {duration} us, export {exported_us} us, {} captions",
        track.clips.len()
    );
}

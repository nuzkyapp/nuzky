//! Open a real CapCut draft through the CLI and compare its cut with FFmpeg's source frames.
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

fn dir(name: &str) -> PathBuf {
    let root = dunce::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap();
    let dir = root.join("tmp-test").join("capcut-open").join(format!("{name}-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("runtime")).unwrap();
    std::fs::create_dir_all(dir.join("home")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("runtime"), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}

fn cli(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nuzky"));
    command
        .env("HOME", dir.join("home"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_RUNTIME_DIR", dir.join("runtime"));
    command
}

fn run(command: &mut Command) -> Output {
    let out = command.output().expect("starting nuzky or ffmpeg");
    assert!(out.status.success(), "{command:?}: {}", String::from_utf8_lossy(&out.stderr));
    out
}

fn ffmpeg() -> Command {
    let mut command = Command::new("ffmpeg");
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    command
}

fn hashes(folder: &Path) -> BTreeMap<PathBuf, u64> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(hashes(&path));
        } else {
            let mut hash = DefaultHasher::new();
            std::fs::read(&path).unwrap().hash(&mut hash);
            out.insert(path, hash.finish());
        }
    }
    out
}

fn segment(id: &str, at: i64, duration: i64, source: i64, speed: f64) -> Value {
    json!({"material_id": id, "target_timerange": {"start": at, "duration": duration},
        "source_timerange": {"start": source, "duration": (duration as f64 * speed) as i64},
        "speed": speed, "volume": 1.0,
        "clip": {"scale": {"x": 1.0, "y": 1.0}, "transform": {"x": 0.0, "y": 0.0}}})
}

fn pixels(path: &Path) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(std::fs::read(path).unwrap())).read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => panic!("unexpected frame colour type: {other:?}"),
    };
    let rgb = bytes[..info.buffer_size()].chunks_exact(channels).flat_map(|p| p[..3].iter().copied()).collect();
    (info.width, info.height, rgb)
}

fn pixel(frame: &(u32, u32, Vec<u8>), x: f64, y: f64) -> [u8; 3] {
    let at = (((frame.1 as f64 * y) as u32 * frame.0 + (frame.0 as f64 * x) as u32) * 3) as usize;
    frame.2[at..at + 3].try_into().unwrap()
}

fn close(actual: [u8; 3], expected: [u8; 3]) {
    assert!(actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 12), "{actual:?} != {expected:?}");
}

fn frame(dir: &Path, seconds: f64) -> (u32, u32, Vec<u8>) {
    let path = dir.join(format!("frame-{seconds}.png"));
    run(cli(dir).arg("frame").arg(dir.join("out.nuzky")).arg(seconds.to_string()).arg(&path).arg("270"));
    let frame = pixels(&path);
    assert_eq!((frame.0, frame.1), (270, 480));
    frame
}

#[test]
fn a_capcut_draft_opens_as_a_project_that_plays_as_cut() {
    let dir = dir("cut");
    let draft = dir.join("draft");
    let media = draft.join("media");
    std::fs::create_dir_all(&media).unwrap();
    let talk = media.join("talk.mp4");
    let photo = media.join("photo.png");
    let song = media.join("song.mp3");
    let palette =
        ["red", "lime", "blue", "yellow", "cyan", "orange", "purple", "pink", "teal", "navy", "gold", "green"];
    let mut picture = String::from("color=c=black:s=540x960:r=30:d=12");
    for (i, color) in palette.iter().enumerate() {
        picture
            .push_str(&format!(",drawbox=x=0:y=0:w=iw:h=ih:color={color}:t=fill:enable='gte(t,{i})*lt(t,{})'", i + 1));
    }
    picture.push_str(",drawbox=x=220:y=430:w=100:h=100:color=white:t=fill");
    run(ffmpeg()
        .args([
            "-f",
            "lavfi",
            "-i",
            &picture,
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=12",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            // Tagged, so Nuzky and FFmpeg turn it into the same RGB.
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(&talk));
    run(ffmpeg().args(["-f", "lavfi", "-i", "color=c=gray:s=540x960", "-frames:v", "1", "-threads", "1"]).arg(&photo));
    run(ffmpeg().args(["-f", "lavfi", "-i", "sine=frequency=880:duration=10", "-c:a", "libmp3lame"]).arg(&song));

    let mut first = segment("talk", 0, 2_000_000, 0, 1.0);
    first["volume"] = json!(0.5);
    first["common_keyframes"] = json!([{"property_type": "KFTypeScaleX", "keyframe_list": [
        {"time_offset": 0, "values": [1.0], "curveType": "Line"},
        {"time_offset": 2_000_000, "values": [1.5], "curveType": "Line"}]}]);
    first["extra_material_refs"] = json!(["brightness"]);
    let mut missing = segment("gone", 2_000_000, 1_000_000, 0, 1.0);
    missing["extra_material_refs"] = json!(["woosh"]);
    let mut fast = segment("talk", 3_000_000, 2_000_000, 5_000_000, 2.0);
    fast["extra_material_refs"] = json!(["mix"]);
    let mut overlay = segment("photo", 500_000, 2_000_000, 0, 1.0);
    overlay["clip"] = json!({"scale": {"x": 0.4, "y": 0.4}, "transform": {"x": -0.5, "y": 0.5}});
    let mut music = segment("song", 0, 7_000_000, 0, 1.0);
    music["volume"] = json!(0.3);
    music["extra_material_refs"] = json!(["fade"]);
    let texts: Vec<Value> = ["First words", "Next words", "Last words", "My title"]
        .iter()
        .enumerate()
        .map(|(i, text)| {
            json!({"id": format!("text{i}"), "type": if i < 3 { "subtitle" } else { "text" }, "has_shadow": i == 0,
            "content": json!({"text": text, "styles": [{"size": 12, "range": [0, text.len()],
                "fill": {"content": {"solid": {"color": [1.0, 1.0, 1.0]}}},
                "strokes": [{"content": {"solid": {"color": [0.0, 0.0, 0.0]}}, "width": 0.08}]}]}).to_string()})
        })
        .collect();
    let captions: Vec<Value> = (0..3)
        .map(|i| {
            let mut text = segment(&format!("text{i}"), i * 1_000_000, 1_000_000, 0, 1.0);
            text["clip"]["transform"]["y"] = json!(-0.8);
            text
        })
        .collect();
    let gone = dir.join("nonexistent/gone.mov");
    let mut title = segment("text3", 0, 3_000_000, 0, 1.0);
    title["clip"]["transform"]["y"] = json!(0.8);
    let content = json!({"fps": 30, "canvas_config": {"width": 540, "height": 960, "ratio": "original"},
        "platform": {"app_source": "cc", "app_version": "9.5.0", "os": "mac"},
        "materials": {
            "videos": [
                {"id": "talk", "type": "video", "path": talk, "material_name": "talk.mp4", "width": 540, "height": 960},
                {"id": "gone", "type": "video", "path": gone, "material_name": "gone.mov", "width": 540, "height": 960},
                {"id": "photo", "type": "photo", "path": photo, "material_name": "photo.png", "width": 540, "height": 960}],
            "audios": [{"id": "song", "type": "music", "path": song, "name": "song.mp3"}],
            "texts": texts, "effects": [{"id": "brightness", "type": "brightness", "value": 0.3}],
            "transitions": [{"id": "woosh", "name": "Woosh", "duration": 400_000}, {"id": "mix", "name": "Mix", "duration": 400_000}],
            "audio_fades": [{"id": "fade", "fade_in_duration": 1_000_000}]},
        "tracks": [
            {"type": "video", "attribute": 0, "segments": [first, missing, fast, segment("talk", 5_000_000, 2_000_000, 9_000_000, 1.0)]},
            {"type": "video", "attribute": 0, "segments": [overlay]},
            {"type": "video", "attribute": 2, "segments": [segment("talk", 0, 7_000_000, 0, 1.0)]},
            {"type": "text", "attribute": 0, "segments": captions},
            {"type": "text", "attribute": 0, "segments": [title]},
            {"type": "audio", "attribute": 1, "segments": [music]},
            {"type": "effect", "segments": [segment("brightness", 0, 7_000_000, 0, 1.0)]}]});
    std::fs::write(draft.join("draft_info.json"), serde_json::to_vec(&content).unwrap()).unwrap();
    std::fs::write(
        draft.join("draft_meta_info.json"),
        json!({"draft_name": "Test reel", "tm_duration": 7_000_000}).to_string(),
    )
    .unwrap();
    let before = hashes(&draft);
    let out = run(cli(&dir).arg("new").arg(dir.join("out.nuzky")).arg("--from").arg(&draft));
    let said = String::from_utf8_lossy(&out.stderr);
    let left: Vec<_> = said.lines().filter_map(|line| line.strip_prefix("Not brought over: ")).collect();
    assert!(!said.contains("Text shadows"), "{said}");
    for what in ["gone.mov", "Transition: Woosh", "Effects", "Color adjustments"] {
        assert!(left.iter().any(|line| line.contains(what)), "missing {what}: {said}");
    }
    let project: Value = serde_json::from_slice(&std::fs::read(dir.join("out.nuzky")).unwrap()).unwrap();
    assert_eq!(project["name"], "Test reel");
    assert_eq!(
        (project["canvas"]["width"].as_u64(), project["canvas"]["height"].as_u64(), project["canvas"]["fps"].as_u64()),
        (Some(540), Some(960), Some(30))
    );
    assert_eq!(project["imported"]["editor"], "CapCut");
    assert_eq!(project["imported"]["path"], dunce::canonicalize(&draft).unwrap().to_str().unwrap());
    assert_eq!(project["imported"]["leftOut"], json!(left));
    let tracks = project["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 6);
    let clips = tracks[0]["clips"].as_array().unwrap();
    assert_eq!(tracks[0]["kind"], "video");
    assert_eq!(clips.len(), 4);
    for (clip, (at, duration, source, speed)) in clips.iter().zip([
        (0, 2_000_000, 0, 1.0),
        (2_000_000, 1_000_000, 0, 1.0),
        (3_000_000, 2_000_000, 5_000_000, 2.0),
        (5_000_000, 2_000_000, 9_000_000, 1.0),
    ]) {
        assert_eq!(clip["startUs"], at);
        assert_eq!(clip["durationUs"], duration);
        assert_eq!(clip["content"]["sourceInUs"], source);
        assert_eq!(clip["content"]["speed"], speed);
    }
    assert_eq!(clips[0]["content"]["volume"], 0.5);
    let keys = clips[0]["keyframes"].as_array().unwrap();
    assert_eq!(keys.len(), 2);
    for (key, (time, scale)) in keys.iter().zip([(0, 1.0), (2_000_000, 1.5)]) {
        assert_eq!(key["tUs"], time);
        assert_eq!(key["transform"]["scale"], scale);
    }
    assert!(clips[2]["transitionIn"].is_null());
    assert_eq!(clips[3]["transitionIn"], json!({"kind": "dissolve", "durationUs": 400_000}));
    let assets = project["assets"].as_array().unwrap();
    let asset = |clip: &Value| assets.iter().find(|a| a["id"] == clip["content"]["assetId"]).unwrap();
    assert_eq!(asset(&clips[1])["path"], gone.to_str().unwrap());
    assert_eq!(tracks[1]["kind"], "video");
    assert_eq!(tracks[1]["clips"].as_array().unwrap().len(), 1);
    let image = &tracks[1]["clips"][0];
    assert_eq!(asset(image)["kind"], "image");
    assert_eq!(asset(image)["path"], photo.to_str().unwrap());
    assert_eq!((image["startUs"].as_i64(), image["durationUs"].as_i64()), (Some(500_000), Some(2_000_000)));
    let transform = &image["content"]["transform"];
    assert!((transform["scale"].as_f64().unwrap() - 0.4).abs() < 1e-6);
    assert_eq!(transform["x"], -0.25);
    assert_eq!(transform["y"], -0.25);
    assert_eq!(tracks[2]["hidden"], true);
    assert_eq!(tracks[2]["clips"][0]["durationUs"], 7_000_000);
    assert_eq!((tracks[3]["kind"].as_str(), tracks[3]["name"].as_str()), (Some("text"), Some("Captions")));
    assert_eq!(tracks[3]["clips"].as_array().unwrap().len(), 3);
    for (i, text) in tracks[3]["clips"].as_array().unwrap().iter().enumerate() {
        assert_eq!(text["content"]["text"], ["First words", "Next words", "Last words"][i]);
        assert_eq!(text["startUs"], i as i64 * 1_000_000);
        assert_eq!(text["durationUs"], 1_000_000);
        let style = &text["content"]["style"];
        assert!((style["fontSize"].as_f64().unwrap() - 30.6).abs() < 1e-4);
        assert_eq!(style["color"], "#ffffff");
        assert_eq!(style["strokeColor"], "#000000");
        assert!((style["strokeWidth"].as_f64().unwrap() - 2.448).abs() < 1e-4);
        // The first caption has CapCut's shadow, drawn as Nuzky draws one; the others have none.
        assert_eq!(style["shadow"].is_object(), i == 0, "{style}");
    }
    assert_eq!(tracks[4]["kind"], "text");
    assert_ne!(tracks[4]["name"], "Captions");
    assert_eq!(tracks[4]["clips"].as_array().unwrap().len(), 1);
    assert_eq!(tracks[4]["clips"][0]["content"]["text"], "My title");
    assert_eq!(tracks[4]["clips"][0]["startUs"], 0);
    assert_eq!(tracks[4]["clips"][0]["durationUs"], 3_000_000);
    assert_eq!(tracks[5]["kind"], "audio");
    assert_eq!(tracks[5]["muted"], true);
    assert_eq!(tracks[5]["keepInPlace"], true);
    assert_eq!(tracks[5]["clips"].as_array().unwrap().len(), 1);
    let audio = &tracks[5]["clips"][0]["content"];
    assert_eq!(asset(&tracks[5]["clips"][0])["path"], song.to_str().unwrap());
    assert!((audio["volume"].as_f64().unwrap() - 0.3).abs() < 1e-6);
    assert_eq!(audio["fadeInUs"], 1_000_000);

    for (seconds, source) in [(0.5, 0.5), (3.25, 5.5), (4.5, 8.0), (6.0, 10.0)] {
        let drawn = frame(&dir, seconds);
        let reference = dir.join(format!("reference-{source}.png"));
        run(ffmpeg()
            .arg("-ss")
            .arg(source.to_string())
            .arg("-i")
            .arg(&talk)
            .args(["-frames:v", "1", "-vf", "scale=270:480", "-threads", "1"])
            .arg(&reference));
        close(pixel(&drawn, 0.3, 0.75), pixel(&pixels(&reference), 0.3, 0.75));
        if seconds == 0.5 {
            close(pixel(&drawn, 0.25, 0.25), [128, 128, 128]);
        }
    }
    // The missing file keeps its place: a frame that needs it names it, never draws black in its place unasked.
    let missing =
        cli(&dir).arg("frame").arg(dir.join("out.nuzky")).arg("2.5").arg(dir.join("missing.png")).output().unwrap();
    assert!(!missing.status.success() && String::from_utf8_lossy(&missing.stderr).contains("gone.mov"));
    let white_width = |frame: (u32, u32, Vec<u8>)| {
        let start = (frame.1 / 2 * frame.0 * 3) as usize;
        frame.2[start..start + frame.0 as usize * 3]
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|p| p.iter().all(|v| *v > 235))
            .count()
    };
    let small = white_width(frame(&dir, 0.1));
    let large = white_width(frame(&dir, 1.9));
    assert!(
        small >= 48 && (1.4..1.55).contains(&(large as f64 / small as f64)),
        "white square widths: {small}, {large}"
    );
    assert_eq!(hashes(&draft), before, "opening and rendering changed the CapCut draft");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_encrypted_capcut_draft_does_not_open() {
    let dir = dir("encrypted");
    let draft = dir.join("draft");
    std::fs::create_dir_all(&draft).unwrap();
    std::fs::write(
        draft.join("draft_content.json"),
        "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo0123456789abcdefghijklmnopqrstuv",
    )
    .unwrap();
    let path = dir.join("out.nuzky");
    let out = cli(&dir).arg("new").arg(&path).arg("--from").arg(&draft).output().unwrap();
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{said}");
    assert!(said.contains("JianYing encrypts its drafts, so Nuzky cannot open this one."), "{said}");
    assert!(!path.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

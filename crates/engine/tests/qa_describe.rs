mod qa_support;
use nuzky_engine::media::describe;
use qa_support::*;

#[test]
fn describe_phone_headers_without_private_metadata() {
    assert!(available(), "ffmpeg is required for this test");
    let d = dir("describe-phone");
    let source = d.join("source.mov");
    ff(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=192x108:r=25:d=0.2",
            "-c:v",
            "libx265",
            "-pix_fmt",
            "yuv420p10le",
            "-preset",
            "ultrafast",
            "-x265-params",
            "pools=1:frame-threads=1:transfer=16:colorprim=9:colormatrix=9",
            "-color_trc",
            "smpte2084",
            "-colorspace",
            "bt2020nc",
            "-color_primaries",
            "bt2020",
        ],
        &source,
    );
    let phone = d.join("phone.mov");
    ff(
        &[
            "-display_rotation",
            "270",
            "-i",
            source.to_str().unwrap(),
            "-c",
            "copy",
            "-movflags",
            "use_metadata_tags",
            "-metadata",
            "com.apple.quicktime.model=  iPhone 15 Pro  ",
            "-metadata",
            "location=+50.1234+014.5678/",
            "-metadata",
            "title=Private family holiday",
            "-metadata",
            "com.apple.quicktime.software=  Camera\nApp\tVersion0123456789012345678901234567890123456789  ",
        ],
        &phone,
    );
    let reference = info(&phone);
    assert_eq!(reference["format"]["tags"]["location"], "+50.1234+014.5678/");
    assert_eq!(reference["format"]["tags"]["title"], "Private family holiday");
    assert_eq!(reference["streams"][0]["codec_name"], "hevc");
    assert_eq!(reference["streams"][0]["color_transfer"], "smpte2084");
    assert_eq!(reference["streams"][0]["side_data_list"][0]["rotation"], -90);
    let line = describe(&phone).unwrap();
    eprintln!("{line}");
    for part in [
        "mov",
        "hevc Main 10",
        "192×108",
        "yuv420p10le",
        "25.00 fps",
        "rotated 90°",
        "PQ HDR",
        "0.2 s",
        "iPhone 15 Pro",
    ] {
        assert!(line.split(" · ").any(|p| p == part), "missing {part}: {line}");
    }
    assert!(line.split(" · ").any(|p| p == "CameraAppVersion012345678901234567890123"));
    assert!(!line.contains("VFR"), "constant timing marked VFR: {line}");
    assert!(!line.contains("+50.1234+014.5678/"), "private location leaked: {line}");
    assert!(!line.contains("Private family holiday"), "private title leaked: {line}");
}

#[test]
fn describe_variable_timing_and_audio() {
    assert!(available(), "ffmpeg is required for this test");
    let d = dir("describe-vfr");
    let video = d.join("vfr.mp4");
    ff(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=160x90:r=30:d=1",
            "-f",
            "lavfi",
            "-i",
            "sine=sample_rate=48000:duration=1.5",
            "-vf",
            "setpts=if(lt(N\\,15)\\,N/(30*TB)\\,(15+2*(N-15))/(30*TB))",
            "-fps_mode",
            "vfr",
            "-c:v",
            "libx264",
            "-threads",
            "2",
            "-c:a",
            "aac",
            "-ac",
            "2",
        ],
        &video,
    );
    let reference = info(&video);
    let stream = &reference["streams"][0];
    assert_ne!(stream["avg_frame_rate"], stream["r_frame_rate"], "fixture must have variable timing");
    let line = describe(&video).unwrap();
    eprintln!("{line}");
    assert!(line.contains("h264"), "{line}");
    assert!(line.contains("160×90"), "{line}");
    assert!(line.contains("21.43 fps VFR"), "variable timing not reported: {line}");
    assert!(line.contains("aac 48 kHz 2 ch"), "{line}");
}

#[test]
fn describe_image_has_only_format_size_and_known_orientation() {
    assert!(available(), "ffmpeg is required for this test");
    let image = dir("describe-image").join("image.png");
    ff(&["-f", "lavfi", "-i", "color=blue:s=32x24", "-frames:v", "1"], &image);
    assert_eq!(describe(&image).unwrap(), "png_pipe · 32×24");
}

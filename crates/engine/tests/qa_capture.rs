mod qa_support;
use std::process::Command;

use nuzky_engine::capture::capture;
use qa_support::*;

/// An iPhone keeps the local time with its offset and the place in QuickTime tags; MP4's creation
/// time is UTC. Nuzky reads them as ffprobe does.
#[test]
fn phone_video_tells_when_and_where_it_was_recorded() {
    assert!(available(), "ffmpeg is required for this test");
    let d = dir("capture-phone");
    let iphone = d.join("iphone.mov");
    ff(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=160x90:r=25:d=0.2",
            "-c:v",
            "libx264",
            "-movflags",
            "use_metadata_tags",
            "-metadata",
            "com.apple.quicktime.creationdate=2024-05-12T14:03:22+0200",
            "-metadata",
            "com.apple.quicktime.location.ISO6709=+50.0755+014.4378+250.000/",
            "-metadata",
            "com.apple.quicktime.location.accuracy.horizontal=4.7",
            "-metadata",
            "com.apple.quicktime.make=Apple",
            "-metadata",
            "com.apple.quicktime.model=iPhone 15 Pro",
            "-metadata",
            "creation_time=2024-05-12T12:03:22.000000Z",
        ],
        &iphone,
    );
    let tags = &info(&iphone)["format"]["tags"];
    assert_eq!(tags["com.apple.quicktime.creationdate"], "2024-05-12T14:03:22+0200");
    assert_eq!(tags["com.apple.quicktime.location.ISO6709"], "+50.0755+014.4378+250.000/");
    let c = capture(&iphone).unwrap();
    assert_eq!(c.recorded_at.as_deref(), Some("2024-05-12T14:03:22+02:00"));
    assert_eq!(c.time_basis, Some("local"));
    let at = c.location.unwrap();
    assert_eq!((at.latitude, at.longitude, at.altitude_m, at.accuracy_m), (50.0755, 14.4378, Some(250.0), Some(4.7)));
    assert_eq!(c.device.as_deref(), Some("Apple iPhone 15 Pro"));

    // An Android phone: only the UTC creation time and the ©xyz place.
    let android = d.join("android.mp4");
    ff(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=160x90:r=25:d=0.2",
            "-c:v",
            "libx264",
            "-metadata",
            "creation_time=2025-01-02T08:15:00.000000Z",
            "-metadata",
            "location=-33.8688+151.2093/",
        ],
        &android,
    );
    let tags = &info(&android)["format"]["tags"];
    assert_eq!(tags["location"], "-33.8688+151.2093/");
    let c = capture(&android).unwrap();
    assert_eq!(c.recorded_at.as_deref(), Some("2025-01-02T08:15:00Z"));
    assert_eq!(c.time_basis, Some("utc"));
    let at = c.location.unwrap();
    assert_eq!((at.latitude, at.longitude, at.altitude_m), (-33.8688, 151.2093, None));

    // A file that says nothing gets nothing, not the date it was copied.
    let bare = d.join("bare.mp4");
    ff(&["-f", "lavfi", "-i", "testsrc2=s=160x90:r=25:d=0.2", "-c:v", "libx264", "-map_metadata", "-1"], &bare);
    assert_eq!(capture(&bare).unwrap(), Default::default());
}

/// EXIF of a photo, big-endian as iPhones write it: IFD0 with make, model and the Exif and GPS
/// pointers, the Exif IFD with the original time and its offset, the GPS IFD with the place.
fn exif_block() -> Vec<u8> {
    let mut t = b"MM\0*\0\0\0\x08".to_vec();
    let entry = |t: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32| {
        t.extend(tag.to_be_bytes());
        t.extend(kind.to_be_bytes());
        t.extend(count.to_be_bytes());
        t.extend(value.to_be_bytes());
    };
    // Values live after the three directories, from offset 200 on.
    let mut data: Vec<u8> = Vec::new();
    let mut put = |bytes: &[u8]| {
        let at = 200 + data.len() as u32;
        data.extend_from_slice(bytes);
        at
    };
    let make = put(b"Apple\0");
    let model = put(b"iPhone 15 Pro\0");
    let original = put(b"2024:05:12 14:03:22\0");
    let offset = put(b"+02:00\0");
    let rationals = |parts: &[(u32, u32)]| {
        parts.iter().flat_map(|(n, d)| [n.to_be_bytes(), d.to_be_bytes()]).flatten().collect::<Vec<u8>>()
    };
    let latitude = put(&rationals(&[(50, 1), (4, 1), (3027, 100)]));
    let longitude = put(&rationals(&[(14, 1), (26, 1), (15, 2)]));
    let altitude = put(&rationals(&[(501, 2)]));
    let (ifd0, exif, gps) = (8u32, 8 + 2 + 4 * 12 + 4, 8 + 2 + 4 * 12 + 4 + 2 + 2 * 12 + 4);
    assert_eq!(ifd0, 8);
    t.extend(4u16.to_be_bytes());
    entry(&mut t, 0x010F, 2, 6, make);
    entry(&mut t, 0x0110, 2, 14, model);
    entry(&mut t, 0x8769, 4, 1, exif);
    entry(&mut t, 0x8825, 4, 1, gps);
    t.extend(0u32.to_be_bytes());
    t.extend(2u16.to_be_bytes());
    entry(&mut t, 0x9003, 2, 20, original);
    entry(&mut t, 0x9011, 2, 7, offset);
    t.extend(0u32.to_be_bytes());
    t.extend(6u16.to_be_bytes());
    entry(&mut t, 1, 2, 2, u32::from_be_bytes(*b"N\0\0\0"));
    entry(&mut t, 2, 5, 3, latitude);
    entry(&mut t, 3, 2, 2, u32::from_be_bytes(*b"E\0\0\0"));
    entry(&mut t, 4, 5, 3, longitude);
    entry(&mut t, 5, 1, 1, 0);
    entry(&mut t, 6, 5, 1, altitude);
    t.extend(0u32.to_be_bytes());
    t.resize(200, 0);
    t.extend(data);
    t
}

#[test]
fn photo_exif_tells_when_and_where_as_ffprobe_reads_it() {
    assert!(available(), "ffmpeg is required for this test");
    let d = dir("capture-photo");
    let plain = d.join("plain.jpg");
    ff(&["-f", "lavfi", "-i", "color=c=orange:s=64x48", "-frames:v", "1"], &plain);
    let jpeg = std::fs::read(&plain).unwrap();
    assert_eq!(&jpeg[..2], [0xFF, 0xD8]);
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend(exif_block());
    let mut photo = jpeg[..2].to_vec();
    photo.extend([0xFF, 0xE1]);
    photo.extend(((app1.len() + 2) as u16).to_be_bytes());
    photo.extend(&app1);
    photo.extend(&jpeg[2..]);
    let path = d.join("photo.jpg");
    std::fs::write(&path, photo).unwrap();

    // ffprobe reads the same fields, so the block is the EXIF any reader sees.
    let probe = run(Command::new("ffprobe").args(["-v", "error", "-show_frames", "-of", "json"]).arg(&path)).stdout;
    let frame: serde_json::Value = serde_json::from_slice(&probe).unwrap();
    let tags = &frame["frames"][0]["tags"];
    // FFmpeg 8 and later prefix the directory ("ExifIFD/DateTimeOriginal"); FFmpeg 6.1 does not.
    let tag = |name: &str| {
        tags.as_object()
            .and_then(|t| t.iter().find(|(k, _)| k.rsplit('/').next() == Some(name)).map(|(_, v)| v.clone()))
    };
    if tags.is_object() {
        assert_eq!(tag("DateTimeOriginal"), Some("2024:05:12 14:03:22".into()), "{tags}");
        assert_eq!(tag("GPSLatitudeRef"), Some("N".into()), "{tags}");
        assert_eq!(tag("Model"), Some("iPhone 15 Pro".into()), "{tags}");
    } else {
        eprintln!("this ffprobe shows no EXIF tags; checking against the values written");
    }
    let c = capture(&path).unwrap();
    assert_eq!(c.recorded_at.as_deref(), Some("2024-05-12T14:03:22+02:00"));
    assert_eq!(c.time_basis, Some("local"));
    assert_eq!(c.device.as_deref(), Some("Apple iPhone 15 Pro"));
    let at = c.location.unwrap();
    let (lat, lon) = (50.0 + 4.0 / 60.0 + 30.27 / 3600.0, 14.0 + 26.0 / 60.0 + 7.5 / 3600.0);
    assert!((at.latitude - lat).abs() < 1e-9 && (at.longitude - lon).abs() < 1e-9, "{at:?}");
    assert_eq!(at.altitude_m, Some(250.5));
    assert_eq!(capture(&plain).unwrap(), Default::default());
}

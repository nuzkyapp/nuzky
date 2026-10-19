//! Small PNG thumbnails for the media panel and timeline clips.

use std::path::Path;

use anyhow::Result;
use base64::Engine as _;
use nuzky_engine::media::{decode_size, orient};
use nuzky_engine::model::{Asset, AssetKind};

const THUMB_HEIGHT: f32 = 120.0;
const MAX_SPRITE_PIXELS: u64 = 4_000_000;

pub fn thumbnail(cache_dir: &Path, asset: &Asset) -> Result<Option<String>> {
    picture(cache_dir, asset, THUMB_HEIGHT)
}

/// A PNG of the asset's picture `height` pixels high, from a tenth of the way in for a video.
pub fn picture(cache_dir: &Path, asset: &Asset, height: f32) -> Result<Option<String>> {
    let Some((rgba, w, h)) = frame(cache_dir, asset, height, u32::MAX)? else { return Ok(None) };
    let mut png_bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_bytes, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.write_header()?.write_image_data(&rgba)?;
    }
    Ok(Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png_bytes))))
}

/// The asset's picture as RGBA, `height` pixels high but no side over `max_side`, turned the way it is shown.
pub fn frame(cache_dir: &Path, asset: &Asset, height: f32, max_side: u32) -> Result<Option<(Vec<u8>, u32, u32)>> {
    if asset.kind == AssetKind::Audio || asset.width == 0 {
        return Ok(None);
    }
    let mut decoder = nuzky_engine::proxy::open_decoder(cache_dir, Path::new(&asset.path))?;
    // From the part the user chose, when the file has more around it.
    let (from, length) = asset.part.map_or((0, asset.duration_us), |p| (p.start_us, p.end_us - p.start_us));
    let target = if asset.kind == AssetKind::Image { 0 } else { from + (length / 10).min(1_000_000) };
    decoder.seek(target)?;
    let mut picked = None;
    while let Some((t, f)) = decoder.next_frame()? {
        let done = t >= target;
        picked = Some((t, f));
        if done {
            break;
        }
    }
    let Some((t, f)) = picked else { return Ok(None) };
    let display = (asset.width as f32 * height / asset.height as f32, height);
    let (w, h) = decode_size(decoder.source_size(), asset.rotation, display, max_side);
    let frame = decoder.convert(&f, t, w, h)?;
    Ok(Some(orient(&frame.data, frame.width, frame.height, asset)))
}

pub fn filmstrip(cache_dir: &Path, asset: &Asset) -> Result<Option<crate::Filmstrip>> {
    if asset.kind == AssetKind::Audio || asset.width == 0 || asset.height == 0 {
        return Ok(None);
    }
    let (frame_width, frame_height, count) = filmstrip_size(asset)?;
    let interval_us = (asset.duration_us / count as i64).max(500_000);
    strip(cache_dir, asset, 0, interval_us, (frame_width, frame_height, count)).map(Some)
}

/// `count` frames of a video `interval_us` apart from `from_us`, for a strip that shows only a stretch of it.
pub fn filmstrip_of(
    cache_dir: &Path,
    asset: &Asset,
    from_us: i64,
    interval_us: i64,
    count: u32,
) -> Result<crate::Filmstrip> {
    let (frame_width, frame_height, most) = filmstrip_size(asset)?;
    let count = count.clamp(1, most.max(1));
    strip(cache_dir, asset, from_us.max(0), interval_us.max(1), (frame_width, frame_height, count))
}

fn strip(
    cache_dir: &Path,
    asset: &Asset,
    from_us: i64,
    interval_us: i64,
    (frame_width, frame_height, count): (u32, u32, u32),
) -> Result<crate::Filmstrip> {
    let (dw, dh) = if asset.rotation % 180 == 90 { (frame_height, frame_width) } else { (frame_width, frame_height) };
    let width = frame_width * count;
    let mut sprite = vec![0; (width * frame_height * 4) as usize];
    let mut decoder = nuzky_engine::proxy::open_decoder(cache_dir, Path::new(&asset.path))?;
    let mut frames = [None, None];
    for i in 0..count {
        let target = (from_us + i as i64 * interval_us).min(asset.duration_us.saturating_sub(1).max(0));
        if i == 0 || interval_us >= 2_000_000 {
            decoder.seek(target)?;
            frames = [None, None];
        }
        frames = decoder.frame_covering(target, frames)?;
        let Some((t, frame)) = frames[0].as_ref().or(frames[1].as_ref()) else {
            anyhow::bail!("No filmstrip frame at {target} us in {}", asset.path)
        };
        let frame = decoder.convert(frame, *t, dw, dh)?;
        let (rgba, _, _) = orient(&frame.data, dw, dh, asset);
        let row = frame_width as usize * 4;
        for y in 0..frame_height as usize {
            let offset = (y * width as usize + i as usize * frame_width as usize) * 4;
            sprite[offset..offset + row].copy_from_slice(&rgba[y * row..(y + 1) * row]);
        }
    }
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, width, frame_height);
        enc.set_color(png::ColorType::Rgba);
        enc.write_header()?.write_image_data(&sprite)?;
    }
    let url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
    Ok(crate::Filmstrip { url, frame_width, frame_height, interval_us, count })
}

fn filmstrip_size(asset: &Asset) -> Result<(u32, u32, u32)> {
    let height = 90;
    let width = (asset.width as f64 * height as f64 / asset.height as f64).round().max(2.0) as u64;
    let pixels = width * height as u64;
    if pixels > MAX_SPRITE_PIXELS {
        anyhow::bail!("This aspect ratio is too wide for a filmstrip");
    }
    let count = if asset.kind == AssetKind::Image { 1 } else { (asset.duration_us / 500_000).clamp(1, 40) as u32 };
    Ok((width as u32, height, count.min((MAX_SPRITE_PIXELS / pixels) as u32)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filmstrip_limits_total_pixels() {
        let mut asset = Asset {
            id: "wide".into(),
            name: String::new(),
            path: String::new(),
            kind: AssetKind::Video,
            duration_us: 600_000_000,
            width: 10000,
            height: 100,
            fps: 25.0,
            has_audio: false,
            rotation: 0,
            mirror: false,
            credit: None,
            part: None,
        };
        let (w, h, count) = filmstrip_size(&asset).unwrap();
        assert_eq!(count, 4);
        assert!(w as u64 * h as u64 * count as u64 <= MAX_SPRITE_PIXELS);
        asset.width = u32::MAX;
        asset.height = 1;
        assert!(filmstrip_size(&asset).is_err());
    }

    #[test]
    #[ignore = "requires tmp-test/filmstrip-long.mp4"]
    fn filmstrip_long_gop_timing() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tmp-test/filmstrip-long.mp4");
        let mut asset = nuzky_engine::media::probe(&path, "long".into()).unwrap();
        for duration in [600_000_000, 60_000_000] {
            asset.duration_us = duration;
            let start = std::time::Instant::now();
            let strip = filmstrip(Path::new("/nonexistent"), &asset).unwrap().unwrap();
            eprintln!("filmstrip {duration} us: {:?}, {} frames", start.elapsed(), strip.count);
            assert_eq!(strip.count, 40);
        }
    }

    #[test]
    #[ignore = "requires the local tmp-test media fixtures"]
    fn filmstrip_media_fixtures() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tmp-test");
        let out = root.join("engine-evidence");
        std::fs::create_dir_all(&out).unwrap();
        for name in ["portrait.mp4", "wide.mp4", "phone_hevc_vfr.mov", "music.mp3", "engine-evidence/identity.png"] {
            let asset = nuzky_engine::media::probe(&root.join(name), name.into()).unwrap();
            let strip = filmstrip(Path::new("/nonexistent"), &asset).unwrap();
            if asset.kind == AssetKind::Audio {
                assert!(strip.is_none());
                continue;
            }
            let strip = strip.unwrap();
            assert_eq!(strip.frame_height, 90);
            assert!(strip.count <= 40 && strip.interval_us >= 500_000);
            if asset.kind == AssetKind::Image {
                assert_eq!(strip.count, 1);
            }
            let bytes = base64::engine::general_purpose::STANDARD.decode(strip.url.split_once(',').unwrap().1).unwrap();
            let reader = png::Decoder::new(std::io::Cursor::new(&bytes)).read_info().unwrap();
            assert_eq!(reader.info().width, strip.frame_width * strip.count);
            assert_eq!(reader.info().height, 90);
            std::fs::write(out.join(format!("filmstrip-{}.png", asset.name)), bytes).unwrap();
        }
    }
}

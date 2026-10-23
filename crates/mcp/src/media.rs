use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result, ensure};
use nuzky_engine::{
    Pending, Project, Renderer, Wait,
    media::VideoDecoder,
    model::{Background, ClipContent, ThumbnailFormat},
};
use serde_json::{Value, json};

const DEFAULT_WIDTH: u32 = 320;
pub const MAX_FRAMES: usize = 16;
const LABEL_HEIGHT: u32 = 24;
use crate::limits::MAX_SHEET_PIXELS;

pub fn check_media(project: &Project) -> Result<()> {
    for clip in project.tracks.iter().flat_map(|t| &t.clips) {
        if let ClipContent::Media { asset_id, background, .. } = &clip.content {
            let asset = project.asset(asset_id).context("UNKNOWN_ASSET: clip references absent asset")?;
            ensure!(Path::new(&asset.path).is_file(), "MEDIA_MISSING: {}", asset.path);
            if let Background::Image { asset_id } = background {
                let image =
                    project.asset(asset_id).context("UNKNOWN_ASSET: a background references an absent image")?;
                ensure!(Path::new(&image.path).is_file(), "MEDIA_MISSING: {}", image.path);
            }
        }
    }
    Ok(())
}

pub fn contact_sheet(
    project: &Project,
    times: &[i64],
    width: Option<u32>,
    safe_area: bool,
    cache_dir: &Path,
) -> Result<Vec<u8>> {
    ensure!(!times.is_empty() && times.len() <= MAX_FRAMES, "Provide 1..={MAX_FRAMES} frame times");
    ensure!(
        times.iter().all(|t| *t >= 0 && *t < project.duration_us()),
        "Frame times must be inside the timeline in integer microseconds"
    );
    check_media(project)?;
    let width = width.unwrap_or(DEFAULT_WIDTH);
    ensure!((96..=1280).contains(&width), "Frame width must be 96..=1280 pixels");
    let height = (width as u64 * project.canvas.height as u64 / project.canvas.width as u64) as u32;
    let columns = (times.len() as u32).min(4);
    let rows = (times.len() as u32).div_ceil(columns);
    let (sheet_w, sheet_h) = (columns * width, rows * (height + LABEL_HEIGHT));
    ensure!(
        sheet_w as u64 * sheet_h as u64 <= MAX_SHEET_PIXELS,
        "RESULT_TOO_LARGE: contact sheet exceeds transport budget; use a smaller width or fewer times"
    );
    let mut pixels = vec![0u8; sheet_w as usize * sheet_h as usize * 4];
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
    let mut renderer = Renderer::new().context("Starting frame renderer")?;
    renderer.use_mattes(cache_dir.to_path_buf(), Pending::Fail);
    for (index, &time) in times.iter().enumerate() {
        let mut rgba = renderer
            .render(project, time, width, height, Wait::Exact, false)
            .context("Rendering contact sheet frame")?;
        if safe_area {
            shade_unsafe(&mut rgba, width, height, &project.canvas);
        }
        let (x, y) = (index as u32 % columns * width, index as u32 / columns * (height + LABEL_HEIGHT));
        for row in 0..height as usize {
            let dest = ((y as usize + row) * sheet_w as usize + x as usize) * 4;
            let src = row * width as usize * 4;
            pixels[dest..dest + width as usize * 4].copy_from_slice(&rgba[src..src + width as usize * 4]);
        }
        label(
            &mut pixels,
            sheet_w,
            x + 6,
            y + height + 5,
            &format!("{}.{:06}", time / 1_000_000, time % 1_000_000),
            width - 12,
        );
    }
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, sheet_w, sheet_h);
    encoder.set_color(png::ColorType::Rgba);
    encoder
        .write_header()
        .context("Writing PNG header")?
        .write_image_data(&pixels)
        .context("Encoding contact sheet")?;
    Ok(png)
}

// A tiny fixed numeric font keeps timestamp labels independent of system fonts.
pub(crate) fn label(pixels: &mut [u8], stride: u32, x: u32, y: u32, text: &str, available: u32) {
    const DIGITS: [[u8; 5]; 11] = [
        [7, 5, 5, 5, 7],
        [2, 6, 2, 2, 7],
        [7, 1, 7, 4, 7],
        [7, 1, 7, 1, 7],
        [5, 5, 7, 1, 1],
        [7, 4, 7, 1, 7],
        [7, 4, 7, 5, 7],
        [7, 1, 1, 1, 1],
        [7, 5, 7, 5, 7],
        [7, 5, 7, 1, 7],
        [0, 0, 0, 0, 2],
    ];
    for (index, byte) in text.bytes().take((available / 8) as usize).enumerate() {
        let glyph = if byte == b'.' { 10 } else { (byte - b'0') as usize };
        for (row, bits) in DIGITS[glyph].iter().enumerate() {
            for col in 0..3 {
                if bits & (1 << (2 - col)) == 0 {
                    continue;
                }
                for dy in 0..2 {
                    for dx in 0..2 {
                        let offset =
                            (((y + row as u32 * 2 + dy) * stride + x + index as u32 * 8 + col * 2 + dx) * 4) as usize;
                        pixels[offset..offset + 4].fill(255);
                    }
                }
            }
        }
    }
}

fn shade_unsafe(pixels: &mut [u8], width: u32, height: u32, canvas: &nuzky_engine::model::Canvas) {
    let Some(area) = canvas.safe_area() else { return };
    for y in 0..height {
        for x in 0..width {
            let cx = (x as f32 + 0.5) * canvas.width as f32 / width as f32;
            let cy = (y as f32 + 0.5) * canvas.height as f32 / height as f32;
            if cx < area.left || cx >= area.right || cy < area.top || cy >= area.bottom {
                let offset = ((y * width + x) * 4) as usize;
                for (channel, overlay) in [240u16, 80, 60].into_iter().enumerate() {
                    pixels[offset + channel] = ((u16::from(pixels[offset + channel]) * 3 + overlay) / 4) as u8;
                }
            }
        }
    }
}

/// What the apps draw over a thumbnail, tinted: on a cover red where Reels puts its interface and blue
/// outside the 3:4 middle the profile grid shows; on a YouTube thumbnail red under the duration badge in
/// the bottom right corner.
pub fn shade_thumbnail_zones(pixels: &mut [u8], width: u32, height: u32, format: ThumbnailFormat) {
    let (tw, th) = format.size();
    let canvas =
        nuzky_engine::model::Canvas { width: tw, height: th, fps: 30, background: String::new(), background_blur: 0.0 };
    let area = canvas.safe_area();
    let grid = (th as f32 - tw as f32 * 4.0 / 3.0) / 2.0;
    for y in 0..height {
        for x in 0..width {
            let (cx, cy) = ((x as f32 + 0.5) * tw as f32 / width as f32, (y as f32 + 0.5) * th as f32 / height as f32);
            let covered = match format {
                ThumbnailFormat::Cover9x16 => {
                    area.is_some_and(|a| cx < a.left || cx >= a.right || cy < a.top || cy >= a.bottom)
                }
                ThumbnailFormat::Youtube16x9 => cx >= 0.8 * tw as f32 && cy >= 0.8 * th as f32,
            };
            let cut = format == ThumbnailFormat::Cover9x16 && (cy < grid || cy >= th as f32 - grid);
            let offset = ((y * width + x) * 4) as usize;
            for (tint, on) in [([240u16, 80, 60], covered), ([60, 120, 240], cut)] {
                if on {
                    for (channel, overlay) in tint.into_iter().enumerate() {
                        pixels[offset + channel] = ((u16::from(pixels[offset + channel]) * 3 + overlay) / 4) as u8;
                    }
                }
            }
        }
    }
}

/// The colours of the image file itself with its transparency, every few pixels of it: see `colors`. It is
/// converted at its own size, since scaling blends edges with the transparent black around them and moves the
/// colours of a palette PNG by one.
pub fn image_colors(path: &Path) -> Result<(f64, Vec<Value>)> {
    let mut decoder = VideoDecoder::open(path)?;
    let (t, frame) = decoder.next_frame()?.context("The image has no picture")?;
    let rgba = decoder.convert(&frame, t, frame.width(), frame.height())?;
    let (w, h) = (rgba.width as usize, rgba.height as usize);
    let step = (w.max(h) / 256).max(1);
    let (sw, sh) = (w.div_ceil(step), h.div_ceil(step));
    let sample: Vec<u8> = (0..sw * sh)
        .flat_map(|i| {
            let at = ((i / sw) * step * w + (i % sw) * step) * 4;
            rgba.data[at..at + 4].to_vec()
        })
        .collect();
    Ok(colors(&sample, sw as u32, sh as u32))
}

/// The share of pixels that show (alpha at least 128) and up to six groups of alike colours among them, largest
/// first. Each group is named by its most common exact colour, so a flat colour comes out exactly even with JPEG
/// noise or blended edges around it. The group most of a mostly opaque border falls in is the backdrop.
pub fn colors(rgba: &[u8], width: u32, height: u32) -> (f64, Vec<Value>) {
    const NEAR: u32 = 24 * 24;
    let round = |share: f64| (share * 100.0).round() / 100.0;
    let pixels = rgba.as_chunks::<4>().0;
    let mut counts = HashMap::<[u8; 3], u32>::new();
    for p in pixels.iter().filter(|p| p[3] >= 128) {
        *counts.entry([p[0], p[1], p[2]]).or_default() += 1;
    }
    let visible: u32 = counts.values().sum();
    let mut exact: Vec<_> = counts.into_iter().collect();
    exact.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let distance = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).map(|(&a, b)| u32::from(a.abs_diff(b)).pow(2)).sum::<u32>();
    let mut groups: Vec<([u8; 3], u32)> = Vec::new();
    let mut group_of = HashMap::new();
    for (rgb, count) in exact {
        let group = match groups.iter().position(|(seed, _)| distance(*seed, rgb) <= NEAR) {
            Some(group) => group,
            None if groups.len() < 32 => {
                groups.push((rgb, 0));
                groups.len() - 1
            }
            None => continue,
        };
        groups[group].1 += count;
        group_of.insert(rgb, group);
    }

    let (w, h) = (width as usize, height as usize);
    let border = (0..w * h).filter(|i| i % w == 0 || i % w == w - 1 || i / w == 0 || i / w == h - 1);
    let (mut edge, mut shown, mut on_edge) = (0, 0, vec![0; groups.len()]);
    for p in border.map(|i| pixels[i]) {
        edge += 1;
        if p[3] >= 128 {
            shown += 1;
            if let Some(&group) = group_of.get(&[p[0], p[1], p[2]]) {
                on_edge[group] += 1;
            }
        }
    }
    let backdrop = (shown * 4 >= edge * 3)
        .then(|| (0..groups.len()).max_by_key(|&g| on_edge[g]))
        .flatten()
        .filter(|&g| on_edge[g] * 4 >= shown * 3);

    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by(|&a, &b| groups[b].1.cmp(&groups[a].1).then(groups[a].0.cmp(&groups[b].0)));
    let list = order
        .into_iter()
        .filter(|&group| groups[group].1 as f64 >= 0.01 * visible as f64)
        .take(6)
        .map(|group| {
            let ([r, g, b], count) = groups[group];
            let mut color =
                json!({"hex": format!("#{r:02x}{g:02x}{b:02x}"), "share": round(count as f64 / visible as f64)});
            if backdrop == Some(group) {
                color["background"] = json!(true);
            }
            color
        })
        .collect();
    (round(visible as f64 / (w * h).max(1) as f64), list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_area_shades_only_unsafe_vertical_margins() {
        let mut canvas = Project::new("safe area").canvas;
        canvas.width = 1080;
        canvas.height = 1920;
        let mut pixels = vec![100; 108 * 192 * 4];
        shade_unsafe(&mut pixels, 108, 192, &canvas);
        assert_eq!(&pixels[(80 * 108 + 54) * 4..][..4], &[100; 4]);
        assert_eq!(&pixels[..4], &[135, 95, 90, 100]);
        canvas.width = 1920;
        canvas.height = 1080;
        let before = pixels.clone();
        shade_unsafe(&mut pixels, 108, 192, &canvas);
        assert_eq!(pixels, before);
    }

    fn image(w: usize, h: usize, pixel: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
        (0..w * h).flat_map(|i| pixel(i % w, i / w)).collect()
    }

    const RED: [u8; 4] = [0xe5, 0x09, 0x14, 255];

    /// A 20x20 red square in the middle of 40x40, half its pixels off by up to 6 as JPEG leaves them.
    fn square(x: usize, y: usize, outside: [u8; 4]) -> [u8; 4] {
        if !((10..30).contains(&x) && (10..30).contains(&y)) {
            return outside;
        }
        let [r, g, b, a] = RED;
        match (x + y * 3) % 6 {
            3 => [r + 6, g - 6, b + 6, a],
            4 => [r - 6, g + 6, b - 6, a],
            5 => [r + 6, g + 6, b + 6, a],
            _ => RED,
        }
    }

    #[test]
    fn colors_count_only_what_shows_and_name_a_noisy_colour_exactly() {
        let logo = image(40, 40, |x, y| square(x, y, if x < 5 { [255, 255, 255, 127] } else { [0; 4] }));
        let (visible, colors) = colors(&logo, 40, 40);
        assert_eq!(visible, 0.25);
        assert_eq!(colors, [json!({"hex": "#e50914", "share": 1.0})]);
    }

    #[test]
    fn colors_keep_close_brand_blues_apart_and_leave_out_specks() {
        let flag = image(40, 10, |x, y| match (x, y) {
            (39, 9) => [0, 0, 0, 255],
            (0..24, _) => [0x00, 0x57, 0xb8, 255],
            _ => [0x00, 0x78, 0xd4, 255],
        });
        let (visible, colors) = colors(&flag, 40, 10);
        assert_eq!(visible, 1.0);
        // Neither blue is most of the border, so neither is a backdrop.
        assert_eq!(colors, [json!({"hex": "#0057b8", "share": 0.6}), json!({"hex": "#0078d4", "share": 0.4})]);
    }

    #[test]
    fn colors_mark_the_backdrop_only_along_a_mostly_shown_border() {
        let white = [255; 4];
        let card = image(40, 40, |x, y| square(x, y, white));
        let (visible, colors) = colors(&card, 40, 40);
        assert_eq!(visible, 1.0);
        assert_eq!(
            colors,
            [json!({"hex": "#ffffff", "share": 0.75, "background": true}), json!({"hex": "#e50914", "share": 0.25})]
        );
        // With the left part of the border transparent, the white is not a backdrop any more.
        let torn = image(40, 40, |x, y| square(x, y, if x < 12 { [0; 4] } else { white }));
        let (_, torn) = super::colors(&torn, 40, 40);
        assert!(torn.iter().all(|c| c.get("background").is_none()), "{torn:?}");
        assert_eq!(torn[0]["hex"], "#ffffff");
    }
}

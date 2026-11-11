//! Text and stickers over a face: where a caption, title or sticker hides the speaker's eyes or mouth, and the
//! nearest place where it would not, inside what Reels and TikTok leave free on a vertical video. It only reports;
//! moving is an ordinary edit that the user or the agent chooses.
use std::path::Path;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result};
use nuzky_engine::{
    Project, Renderer, Wait,
    edit::{EditCmd, MAIN_TRACK},
    matte::without_backgrounds,
    model::{AssetKind, Clip, ClipContent, Keyframe, TrackKind, Transform},
    render::layer_bounds,
    text::TextRenderer,
};
use serde::Serialize;

use crate::frame::{Frame, Rect};
use crate::models;
use crate::nets::Yunet;
use crate::reframe::update;
use crate::thumbnails::in_parallel;
use crate::timeline;

/// Looks at each text or sticker, at least one, further apart when there would be more than `MAX_LOOKS` in all.
const LOOK_EVERY_US: i64 = 500_000;
const MAX_LOOKS: usize = 1_200;
/// Long side of a look; YuNet finds faces down to about 10 px there.
const LOOK_SIDE: u32 = 512;
/// An overlay covering this share of the canvas or more is B-roll or a second camera, not a sticker.
const STICKER_MAX_AREA: f32 = 0.5;
/// Steps a suggested place is searched in, in canvas pixels on 1080 × 1920.
const STEP: f32 = 20.0 / 1080.0;
/// Room kept between a moved layer and the face, as a share of the canvas height.
const GAP: f32 = 0.02;

/// A text or sticker that hides a face for part of its time.
#[derive(Debug, Serialize)]
pub struct Covered {
    pub clip_id: String,
    /// `caption`, `text` or `sticker`.
    pub kind: &'static str,
    /// The text, or the sticker's file name.
    pub label: String,
    /// Timeline range where it covers the face.
    pub start_us: i64,
    pub end_us: i64,
    /// When it was seen over the face first, clear of its entry animation: the frame to show the user.
    pub time_us: i64,
    /// Eyes to mouth of the face, and the layer, where it covers it first: x, y, width, height in canvas pixels.
    pub face: [i64; 4],
    pub layer: [i64; 4],
    /// The nearest place free of the face for its whole time; none when there is no such place.
    pub suggestion: Option<Suggestion>,
}

#[derive(Debug, Serialize)]
pub struct Suggestion {
    /// The layer there, where it covered the face first.
    pub layer: [i64; 4],
    /// Moves it there, keyframes included, as one step.
    pub edits: Vec<EditCmd>,
}

struct Layer<'a> {
    clip: &'a Clip,
    kind: &'static str,
    label: String,
}

/// Every text and sticker on the timeline that hides a face, in timeline order.
pub fn covered_faces(
    project: &Project,
    models_dir: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<Vec<Covered>> {
    models::require(models::REFRAME, models_dir)?;
    let mut text = TextRenderer::new();
    let layers = layers(project, &mut text);
    // The picture under them, where the face is looked for.
    let mut picture = without_backgrounds(project);
    picture.tracks.retain(|t| t.kind != TrackKind::Text);
    for track in &mut picture.tracks {
        track.clips.retain(|c| !layers.iter().any(|l| l.clip.id == c.id));
    }
    let total: i64 = layers.iter().map(|l| l.clip.duration_us).sum();
    let every = LOOK_EVERY_US.max(total / MAX_LOOKS as i64);
    let looks: Vec<Vec<i64>> = layers.iter().map(|l| times(l.clip, every)).collect();
    let mut all: Vec<i64> = looks.iter().flatten().copied().collect();
    all.sort_unstable();
    all.dedup();
    let size = timeline::size(&picture, LOOK_SIDE);
    let k = project.canvas.width as f32 / size.0 as f32;
    let faces = in_parallel(
        &all,
        cancel,
        progress,
        || Ok((Renderer::new().context("Starting frame renderer")?, Yunet::load(models_dir)?)),
        |(renderer, yunet), &t, cancel| {
            let rgba = renderer.render(&picture, t, size.0, size.1, Wait::Exact, false).context("Rendering a frame")?;
            Ok(yunet
                .detect(&Frame { width: size.0, height: size.1, rgba }, cancel)?
                .iter()
                .map(|d| eyes_to_mouth(&d.scaled(k)))
                .collect::<Vec<Rect>>())
        },
    )?;
    let area = free_area(project);
    let mut out = Vec::new();
    // Where the layers already given a place go, so two moved ones never land on each other.
    let mut shifts: Vec<(&str, [f32; 2])> = Vec::new();
    // Stickers first: larger, they have fewer places to go.
    let mut order: Vec<usize> = (0..layers.len()).collect();
    order.sort_by_key(|&i| layers[i].kind != "sticker");
    for (layer, times) in order.iter().map(|&i| (&layers[i], &looks[i])) {
        let samples: Vec<Sample> = times
            .iter()
            .filter_map(|t| {
                let bounds = layer_bounds(project, *t, &mut text);
                let rect = bounds.iter().find(|(id, ..)| *id == layer.clip.id).map(|(_, shown, _)| bbox(shown))?;
                let others = bounds
                    .iter()
                    .filter(|(id, ..)| *id != layer.clip.id && layers.iter().any(|l| l.clip.id == *id))
                    .map(|(id, shown, _)| {
                        let d = shifts.iter().find(|(moved, _)| moved == id).map_or([0.0, 0.0], |(_, d)| *d);
                        shifted(bbox(shown), d)
                    })
                    .collect();
                Some(Sample { t: *t, rect, faces: faces[all.binary_search(t).ok()?].clone(), others })
            })
            .collect();
        let hit: Vec<(&Sample, Rect)> =
            samples.iter().filter_map(|s| Some((s, *s.faces.iter().find(|f| overlap(s.rect, **f))?))).collect();
        let (Some((first, face)), Some((last, _))) = (hit.first(), hit.last()) else { continue };
        let clip = layer.clip;
        let half = every / 2;
        let suggestion = place(&samples, area, layer.kind != "sticker", project).map(|d| {
            shifts.push((&clip.id, d));
            Suggestion {
                layer: int(shifted(first.rect, d)),
                edits: moved(clip, [d[0] / project.canvas.width as f32, d[1] / project.canvas.height as f32]),
            }
        });
        out.push(Covered {
            clip_id: clip.id.clone(),
            kind: layer.kind,
            label: layer.label.clone(),
            start_us: (first.t - half).max(clip.start_us),
            end_us: (last.t + half).min(clip.start_us + clip.duration_us),
            time_us: first.t,
            face: int(*face),
            layer: int(first.rect),
            suggestion,
        });
    }
    out.sort_by_key(|c| c.start_us);
    Ok(out)
}

struct Sample {
    t: i64,
    rect: Rect,
    faces: Vec<Rect>,
    /// The other texts and stickers on screen then, which a moved layer must not cover either.
    others: Vec<Rect>,
}

/// The texts and stickers of the visible tracks. A sticker is a picture over the main track smaller than half the
/// canvas; a larger one is B-roll, which hides the face on purpose.
fn layers<'a>(project: &'a Project, text: &mut TextRenderer) -> Vec<Layer<'a>> {
    let canvas = (project.canvas.width * project.canvas.height) as f32;
    let mut layers = Vec::new();
    for track in project.tracks.iter().filter(|t| !t.hidden) {
        for clip in &track.clips {
            let layer = match (&clip.content, track.kind) {
                (ClipContent::Text { text, .. }, TrackKind::Text) => Layer {
                    clip,
                    kind: if track.is_captions() { "caption" } else { "text" },
                    label: text.chars().take(60).collect(),
                },
                (ClipContent::Media { asset_id, .. }, TrackKind::Video) if track.id != MAIN_TRACK => {
                    let Some(asset) = project.asset(asset_id).filter(|a| a.kind != AssetKind::Audio) else { continue };
                    let middle = clip.start_us + clip.duration_us / 2;
                    let small = layer_bounds(project, middle, text)
                        .iter()
                        .find(|(id, ..)| *id == clip.id)
                        .is_some_and(|(_, shown, _)| bbox(shown)[2] * bbox(shown)[3] < STICKER_MAX_AREA * canvas);
                    if !small {
                        continue;
                    }
                    Layer { clip, kind: "sticker", label: asset.name.clone() }
                }
                _ => continue,
            };
            layers.push(layer);
        }
    }
    layers
}

/// Times to look at a clip, away from its entry and exit animations, which move and scale it.
fn times(clip: &Clip, every: i64) -> Vec<i64> {
    let (start, end) = (clip.start_us, clip.start_us + clip.duration_us);
    let from = start + clip.anim_in.map_or(0, |a| a.duration_us);
    let to = end - clip.anim_out.map_or(0, |a| a.duration_us);
    let (from, to) = if from < to { (from, to) } else { (start, end) };
    let n = ((to - from) / every).max(1);
    (0..n).map(|i| from + (2 * i + 1) * (to - from) / (2 * n)).collect()
}

/// Eyebrows to lower lip, across both eyes: the part of a face a caption must not hide, measured from the eyes and
/// mouth so it fits a face of any size. A chin or forehead behind text is how captions on reels usually sit.
fn eyes_to_mouth(face: &crate::nets::Detection) -> Rect {
    let [right_eye, left_eye, _, right_mouth, left_mouth] = face.points;
    let eyes = (right_eye[1] + left_eye[1]) / 2.0;
    let mouth = (right_mouth[1] + left_mouth[1]) / 2.0;
    let d = (mouth - eyes).abs().max(1.0);
    let xs = face.points.map(|p| p[0]);
    let left = xs.into_iter().fold(f32::INFINITY, f32::min) - 0.3 * d;
    let right = xs.into_iter().fold(f32::NEG_INFINITY, f32::max) + 0.3 * d;
    let (top, bottom) = (eyes.min(mouth) - 0.4 * d, eyes.max(mouth) + 0.3 * d);
    [left, top, right - left, bottom - top]
}

/// Where a layer may go: what Reels and TikTok leave free on a vertical canvas, else the canvas.
fn free_area(project: &Project) -> Rect {
    let (w, h) = (project.canvas.width as f32, project.canvas.height as f32);
    project.canvas.safe_area().map_or([0.0, 0.0, w, h], |a| [a.left, a.top, a.right - a.left, a.bottom - a.top])
}

/// The nearest shift, in canvas pixels, that keeps the layer clear of every face and other layer at every look
/// and wholly inside `area`. Text moves only up or down when it can, as captions are centred.
fn place(samples: &[Sample], area: Rect, upright_first: bool, project: &Project) -> Option<[f32; 2]> {
    let (w, h) = (project.canvas.width as f32, project.canvas.height as f32);
    let gap = GAP * h;
    let free = |d: [f32; 2]| {
        samples.iter().all(|s| {
            let r = shifted(s.rect, d);
            (0..2).all(|i| r[i] >= area[i] && r[i] + r[i + 2] <= area[i] + area[i + 2])
                && s.faces.iter().all(|f| !overlap(r, [f[0] - gap, f[1] - gap, f[2] + 2.0 * gap, f[3] + 2.0 * gap]))
                && s.others.iter().all(|o| !overlap(r, *o))
        })
    };
    let step = STEP * w.min(h);
    let steps = |side: f32| {
        let n = (side / step) as i32;
        (-n..=n).map(move |i| i as f32 * step)
    };
    let nearest = |candidates: Vec<[f32; 2]>| {
        candidates.into_iter().filter(|d| free(*d)).min_by(|a, b| a[0].hypot(a[1]).total_cmp(&b[0].hypot(b[1])))
    };
    let upright = upright_first.then(|| nearest(steps(h).map(|dy| [0.0, dy]).collect())).flatten();
    upright.or_else(|| nearest(steps(w).flat_map(|dx| steps(h).map(move |dy| [dx, dy])).collect()))
}

/// The edits that shift a clip by `d`, in fractions of the canvas: its transform, or every keyframe, which replace it.
fn moved(clip: &Clip, [dx, dy]: [f32; 2]) -> Vec<EditCmd> {
    let shift = |t: &Transform| Transform { x: t.x + dx, y: t.y + dy, ..*t };
    if clip.keyframes.is_empty() {
        let (ClipContent::Media { transform, .. } | ClipContent::Text { transform, .. }) = &clip.content else {
            return Vec::new();
        };
        return vec![update(&clip.id, shift(transform))];
    }
    let keyframes = clip.keyframes.iter().map(|k| Keyframe { transform: shift(&k.transform), ..*k }).collect();
    vec![EditCmd::SetKeyframes { clip_id: clip.id.clone(), keyframes }]
}

fn bbox(quad: &[[f32; 2]; 4]) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for [x, y] in quad {
        (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y));
    }
    [x0, y0, x1 - x0, y1 - y0]
}

fn shifted(r: Rect, [dx, dy]: [f32; 2]) -> Rect {
    [r[0] + dx, r[1] + dy, r[2], r[3]]
}

fn overlap(a: Rect, b: Rect) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

fn int(r: Rect) -> [i64; 4] {
    r.map(|v| v.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(rect: Rect, faces: Vec<Rect>) -> Sample {
        Sample { t: 0, rect, faces, others: Vec::new() }
    }

    #[test]
    fn a_caption_over_the_mouth_moves_the_shortest_way_clear_of_it_and_stays_in_the_safe_area() {
        let project = Project::new("covered");
        let area = free_area(&project);
        // A 1080 × 1920 reel: eyes to mouth from 850 to 1250 px, a caption 120 px tall across the middle at 1190.
        let face = [380.0, 850.0, 320.0, 400.0];
        let caption = [180.0, 1190.0, 720.0, 120.0];
        let d = place(&[sample(caption, vec![face])], area, true, &project).unwrap();
        assert_eq!(d[0], 0.0, "a caption only moves up or down: {d:?}");
        let moved = shifted(caption, d);
        assert!(!overlap(moved, face) && moved[1] >= face[1] + face[3] + GAP * 1920.0, "{moved:?}");
        assert!(moved[1] + moved[3] <= area[1] + area[3], "below the buttons: {moved:?}");
        // With the space below the face taken, it goes above it, under the top of the safe area.
        let low = [380.0, 1000.0, 320.0, 400.0];
        let d = place(&[sample(caption, vec![low])], area, true, &project).unwrap();
        let moved = shifted(caption, d);
        assert!(moved[1] + moved[3] <= low[1] && moved[1] >= area[1], "{moved:?}");
        // A face filling the whole safe area leaves no place, and a text wider than it has none either.
        assert!(place(&[sample(caption, vec![area])], area, true, &project).is_none());
        assert!(place(&[sample([90.0, 1190.0, 900.0, 120.0], vec![face])], area, true, &project).is_none());
    }

    #[test]
    fn moving_shifts_the_transform_or_every_keyframe() {
        let key =
            |t_us, y| Keyframe { t_us, transform: Transform { y, ..Transform::default() }, ease: Default::default() };
        let mut clip = Clip::new(
            "c".into(),
            0,
            1_000_000,
            ClipContent::Text {
                text: "hi".into(),
                style: nuzky_engine::edit::caption_presets()[0].style.clone(),
                transform: Transform { y: 0.15, ..Transform::default() },
                words: Vec::new(),
            },
        );
        let edits = moved(&clip, [0.0, 0.1]);
        let [EditCmd::UpdateClip { transform: Some(t), .. }] = edits.as_slice() else { panic!() };
        assert!((t.y - 0.25).abs() < 1e-6);
        clip.keyframes = vec![key(0, 0.0), key(500_000, 0.2)];
        let edits = moved(&clip, [0.05, -0.1]);
        let [EditCmd::SetKeyframes { keyframes, .. }] = edits.as_slice() else { panic!() };
        assert!(
            keyframes.iter().zip([-0.1, 0.1]).all(|(k, y)| (k.transform.y - y).abs() < 1e-6 && k.transform.x == 0.05)
        );
    }
}

//! Layout: how wide pictures fill a taller canvas, as the Ratio menu offers it. Fill crops the picture to the frame
//! and follows the face (`nuzky_vision::reframe`); Fade shows it whole across the middle over a blurred copy of itself
//! (the canvas's background blur); Stack shows the clip twice, one over the other, as a split screen of the clip and a
//! silent copy of it. Every layout replaces the one before, so a clip's Stack copy goes when another layout is chosen.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::edit::MAIN_TRACK;
use crate::model::{Asset, AssetKind, Canvas, Clip, ClipContent, Crop, Project, TrackKind, Transform};

/// The blur Canvas: Blur starts with in the inspector, and Fade's when the canvas has none.
pub const DEFAULT_BACKGROUND_BLUR: f32 = 0.5;

/// A layout that needs no face: Fill is `reframe`.
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Layout {
    /// The whole picture across the middle, a blurred copy of it filling the canvas behind.
    Fade,
    /// The clip twice, over the top and the bottom half, each covering its half.
    Stack,
}

/// The clips a layout or reframe places, with their pictures: the video and image clips `clip_ids`, or by default
/// every one of the main track and any other that filled the canvas. A Stack copy stands for the clip it copies.
pub fn targets<'a>(project: &'a Project, clip_ids: Option<&[String]>) -> Result<Vec<(&'a Clip, &'a Asset)>> {
    let video: Vec<(usize, &Clip)> = project
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == TrackKind::Video)
        .flat_map(|(ti, t)| t.clips.iter().map(move |c| (ti, c)))
        .collect();
    let pictured = |clip: &'a Clip| match &clip.content {
        ClipContent::Media { asset_id, .. } => project.asset(asset_id).filter(|a| a.kind != AssetKind::Audio),
        ClipContent::Text { .. } | ClipContent::Adjustment { .. } => None,
    };
    let chosen: Vec<&Clip> = match clip_ids {
        Some(ids) => {
            for id in ids {
                ensure!(
                    video.iter().any(|(_, c)| &c.id == id && pictured(c).is_some()),
                    "INVALID_ARGUMENTS: {id} is not a video or image clip"
                );
            }
            video.iter().filter(|(_, c)| ids.contains(&c.id)).map(|&(ti, c)| original(&video, ti, c)).collect()
        }
        None => video
            .iter()
            .filter(|&&(ti, c)| {
                let track = &project.tracks[ti];
                track.id == MAIN_TRACK
                    || (!track.hidden && pictured(c).is_some_and(|a| fills(&project.canvas, a, &base(c))))
            })
            .map(|&(ti, c)| original(&video, ti, c))
            .collect(),
    };
    let mut found: Vec<(&Clip, &Asset)> = Vec::new();
    for clip in chosen {
        if let Some(asset) = pictured(clip)
            && !found.iter().any(|(c, _)| c.id == clip.id)
        {
            found.push((clip, asset));
        }
    }
    ensure!(!found.is_empty(), "INVALID_ARGUMENTS: there is no video or image clip to lay out");
    Ok(found)
}

/// The Stack copies of `clip_ids`, which another layout removes.
pub fn stack_copies(project: &Project, clip_ids: &[String]) -> Vec<String> {
    let video = || {
        project
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.kind == TrackKind::Video)
            .flat_map(|(ti, t)| t.clips.iter().map(move |c| (ti, c)))
    };
    video()
        .filter(|(_, c)| clip_ids.contains(&c.id))
        .flat_map(|(ti, clip)| video().filter(move |&(tj, other)| tj > ti && is_copy(other, clip)))
        .map(|(_, c)| c.id.clone())
        .collect()
}

/// The clip `clip` on track `ti` copies, or itself.
fn original<'a>(video: &[(usize, &'a Clip)], ti: usize, clip: &'a Clip) -> &'a Clip {
    video.iter().filter(|&&(tj, other)| tj < ti && is_copy(clip, other)).map(|&(_, c)| c).next().unwrap_or(clip)
}

/// Whether `copy` is a silent copy of `of` that plays the same part of the same file at the same time, as Stack
/// makes it.
fn is_copy(copy: &Clip, of: &Clip) -> bool {
    let (
        ClipContent::Media { asset_id: a, source_in_us: a_in, volume, speed: a_speed, reverse: a_rev, .. },
        ClipContent::Media { asset_id: b, source_in_us: b_in, speed: b_speed, reverse: b_rev, .. },
    ) = (&copy.content, &of.content)
    else {
        return false;
    };
    copy.id != of.id
        && *volume == 0.0
        && (a, a_in, a_speed, a_rev) == (b, b_in, b_speed, b_rev)
        && (copy.start_us, copy.duration_us) == (of.start_us, of.duration_us)
}

/// The clip's own framing: its first keyframe when it has keyframes, which replace its transform.
pub fn base(clip: &Clip) -> Transform {
    match (&clip.content, clip.keyframes.first()) {
        (_, Some(key)) => key.transform,
        (ClipContent::Media { transform, .. } | ClipContent::Text { transform, .. }, None) => *transform,
        (ClipContent::Adjustment { .. }, None) => Transform::default(),
    }
}

/// The picture's size on the canvas at scale 1, which fits it inside.
pub fn fitted(canvas: &Canvas, asset: &Asset) -> [f32; 2] {
    let (w, h) = (asset.width.max(1) as f32, asset.height.max(1) as f32);
    let fit = (canvas.width as f32 / w).min(canvas.height as f32 / h);
    [w * fit, h * fit]
}

/// The positions, as transform x or y, at which a picture `size` pixels long, visible from `from` to `to` of its
/// length, covers a canvas side `side` pixels long.
pub fn covering(side: f32, size: f32, from: f32, to: f32) -> (f32, f32) {
    (0.5 - (to - 0.5) * size / side, -0.5 - (from - 0.5) * size / side)
}

/// Whether the clip's visible picture covers the whole canvas, as a clip that fills the frame does and a picture
/// in picture does not.
pub fn fills(canvas: &Canvas, asset: &Asset, t: &Transform) -> bool {
    let visible = Crop::visible(t.crop);
    let size = fitted(canvas, asset).map(|v| v * t.scale);
    let side = [canvas.width as f32, canvas.height as f32];
    t.rotation.rem_euclid(360.0) == 0.0
        && [t.x, t.y].into_iter().enumerate().all(|(i, at)| {
            let (low, high) = covering(side[i], size[i], visible[i], visible[i + 2]);
            let slack = 1.0 / side[i];
            low - slack <= at && at <= high + slack
        })
}

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use nuzky_engine::{
    media::VideoDecoder,
    model::{Asset, AssetKind},
};

use crate::Range;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneParams {
    /// Minimum normalized RGB absolute difference, 0..=1. Default 0.18.
    pub threshold: f32,
    pub min_gap_us: i64,
}

impl Default for SceneParams {
    fn default() -> Self {
        Self { threshold: 0.18, min_gap_us: 300_000 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneCut {
    pub time_us: i64,
    pub score: f32,
}

/// Hard-cut proposals with real decoder PTS. Uses 64x36 RGB differences (colour
/// distinguishes equal-luma cuts), requiring an abrupt rise above recent motion.
/// Rotation is invariant: both compared frames retain the same source orientation.
/// Fades are intentionally excluded; flashes and abrupt camera motion can trigger.
pub fn scene_cuts(asset: &Asset, params: SceneParams) -> Result<Vec<SceneCut>> {
    scene_cuts_cancellable(asset, params, || false)
}

/// `scene_cuts` that stops at the next decoded frame once `cancelled` returns true.
pub fn scene_cuts_cancellable(
    asset: &Asset,
    params: SceneParams,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<SceneCut>> {
    let (mut decoder, mut detector) = Detector::open(asset, params)?;
    while let Some((time_us, frame)) = decoder.next_frame().context("Decoding scene frame")? {
        ensure!(!cancelled(), "CANCELLED: scene analysis cancelled");
        detector.look(time_us, decoder.convert(&frame, time_us, 64, 36).context("Downscaling scene frame")?.data);
    }
    Ok(detector.cuts)
}

/// The cuts of `scene_cuts` inside `windows`, half-open ranges of the video's own time, reading only around them:
/// from the keyframe a second before each window, or on from the last one when it is close behind.
pub fn scene_cuts_in(
    asset: &Asset,
    windows: &[Range],
    params: SceneParams,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<SceneCut>> {
    const LEAD_US: i64 = 1_000_000;
    const READ_ON_US: i64 = 3_000_000;
    let (mut decoder, mut detector) = Detector::open(asset, params)?;
    let mut windows = windows.to_vec();
    windows.sort_by_key(|w| w.start_us);
    for window in &windows {
        if detector.last_time.is_none_or(|t| window.start_us - t > READ_ON_US) {
            decoder.seek(window.start_us - LEAD_US).context("Seeking scene video")?;
            detector.previous = None;
            detector.motion = 0.0;
        }
        while detector.last_time.is_none_or(|t| t < window.end_us) {
            let Some((time_us, frame)) = decoder.next_frame().context("Decoding scene frame")? else { break };
            ensure!(!cancelled(), "CANCELLED: scene analysis cancelled");
            // A seek starts at a keyframe, which can lie before frames already looked at.
            if detector.last_time.is_none_or(|t| time_us > t) {
                detector
                    .look(time_us, decoder.convert(&frame, time_us, 64, 36).context("Downscaling scene frame")?.data);
            }
        }
    }
    detector.cuts.retain(|cut| windows.iter().any(|w| w.start_us <= cut.time_us && cut.time_us < w.end_us));
    Ok(detector.cuts)
}

/// Compares each frame of a video with the one before it.
struct Detector {
    params: SceneParams,
    previous: Option<Arc<Vec<u8>>>,
    motion: f32,
    last_time: Option<i64>,
    cuts: Vec<SceneCut>,
}

impl Detector {
    fn open(asset: &Asset, params: SceneParams) -> Result<(VideoDecoder, Self)> {
        ensure!(asset.kind == AssetKind::Video, "Scene analysis requires video");
        ensure!(
            params.threshold.is_finite() && params.threshold > 0.0 && params.threshold <= 1.0,
            "Scene threshold must be in (0, 1]"
        );
        ensure!(params.min_gap_us >= 0, "Scene gap must be nonnegative");
        let decoder = VideoDecoder::open(Path::new(&asset.path)).context("Opening scene video")?;
        Ok((decoder, Self { params, previous: None, motion: 0.0, last_time: None, cuts: Vec::new() }))
    }

    /// Takes the next frame, at `time_us`, downscaled to 64x36 RGBA.
    fn look(&mut self, time_us: i64, rgba: Arc<Vec<u8>>) {
        if let Some(ref old) = self.previous {
            let score = picture_difference(old.as_slice(), &rgba);
            let increasing = self.last_time.is_none_or(|last| time_us > last);
            let separated =
                self.cuts.last().is_none_or(|cut| time_us.saturating_sub(cut.time_us) >= self.params.min_gap_us);
            let abrupt = score >= self.params.threshold.max(self.motion * 3.0 + 0.02);
            if increasing && time_us >= 0 && separated && abrupt {
                self.cuts.push(SceneCut { time_us, score });
            } else {
                // A cut should not raise the motion baseline and hide the following cut, but motion that sets in at
                // once must, or it would end every gap with another cut.
                self.motion = 0.8 * self.motion + 0.2 * score;
            }
        }
        self.last_time = Some(time_us);
        self.previous = Some(rgba);
    }
}

pub use nuzky_engine::check::picture_difference;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn cancelled_scene_analysis_stops_at_the_next_frame() {
        let dir = std::env::temp_dir().join(format!("nuzky-scenes-{}", nuzky_engine::edit::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("long.mp4");
        let status = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc=size=64x36:rate=30:duration=10", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status()
            .expect("ffmpeg is required for scene tests");
        assert!(status.success());
        let asset = nuzky_engine::media::probe(&path, "long".into()).unwrap();
        let checks = AtomicUsize::new(0);
        let error =
            scene_cuts_cancellable(&asset, SceneParams::default(), || checks.fetch_add(1, Ordering::Relaxed) >= 3)
                .unwrap_err();
        assert!(error.to_string().starts_with("CANCELLED"), "{error:#}");
        assert_eq!(checks.load(Ordering::Relaxed), 4, "analysis must stop at the first frame after cancelling");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

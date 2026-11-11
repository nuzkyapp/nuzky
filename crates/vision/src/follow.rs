//! Follow: one video clip placed to keep in the frame what the user boxed in its picture. The box is followed from
//! frame to frame by how its patch of picture looks (normalised cross-correlation of brightness, at three sizes), ten
//! times a second, back from where it was boxed to the clip's start and on to its end. The picture is then placed and
//! moved as Reframe places it around a face, so it holds still while the thing moves a little and catches up on a
//! smooth curve. No model: it runs wherever the preview does.
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use nuzky_engine::{
    Project, Renderer, Wait,
    edit::EditCmd,
    effects::framing_at,
    model::{AssetKind, ClipContent, Ease, Keyframe, TrackKind, Transform},
};

use crate::frame::Frame;
use crate::nets::check_cancel;
use crate::reframe::{LOOK_EVERY_US, Placement, keys, smooth, solo, update};

/// Ten looks a second, so the thing moves little between two; a long clip is looked at more sparsely.
const STEP_US: i64 = 100_000;
const MAX_LOOKS: i64 = 1_800;
/// The longer side of the box in the frames looked at, in pixels, and so of the patch compared.
const PATCH: f32 = 32.0;
/// Frames are drawn in runs this long, so following back to the start decodes forward.
const RUN: usize = 30;

/// What following changes, as one edit: the clip's transform and keyframes.
pub struct Followed {
    pub commands: Vec<EditCmd>,
    pub keyframes: usize,
    /// How long of the clip the thing was found, and how long it was not, where the picture held.
    pub found_us: i64,
    pub lost_us: i64,
}

/// Places the video clip `clip_id` to keep the thing in `region` (left, top, width and height in fractions of the
/// upright picture, as it is at timeline time `at_us`) where Reframe puts a face. The clip fills the canvas upright,
/// keeping its zoom beyond filling it, its crop and its opacity as at `at_us`; its keyframes are replaced.
pub fn follow(
    project: &Project,
    clip_id: &str,
    at_us: i64,
    region: [f32; 4],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<Followed> {
    let clip = project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .flat_map(|t| &t.clips)
        .find(|c| c.id == clip_id)
        .with_context(|| format!("CLIP_NOT_FOUND: {clip_id}"))?;
    let asset = match &clip.content {
        ClipContent::Media { asset_id, .. } => project.asset(asset_id).filter(|a| a.kind == AssetKind::Video),
        ClipContent::Text { .. } | ClipContent::Adjustment { .. } => None,
    }
    .with_context(|| format!("INVALID_ARGUMENTS: {clip_id} is not a video clip; only a video's picture moves"))?;
    ensure!(clip.contains(at_us), "INVALID_RANGE: at_us must be inside the clip, {}..{}", clip.start_us, clip.end_us());
    // Only the part of the box on the picture counts.
    let [x, y] = [region[0].max(0.0), region[1].max(0.0)];
    let [w, h] = [(region[0] + region[2]).min(1.0) - x, (region[1] + region[3]).min(1.0) - y];
    ensure!(
        region.iter().all(|v| v.is_finite()) && w >= 0.005 && h >= 0.005,
        "INVALID_ARGUMENTS: box is [left, top, width, height] in fractions of the picture and must cover part of it"
    );
    ensure!(Path::new(&asset.path).is_file(), "MEDIA_MISSING: {}", asset.path);
    let base = framing_at(clip, at_us);
    let placement = Placement::new(&project.canvas, &project.canvas, asset, &base);
    ensure!(
        placement.has_room(),
        "NO_ROOM: the picture just fills the frame, so it has no room to follow anything. Zoom in on the clip first."
    );

    // Drawn so the box's longer side is about `PATCH` pixels, within what a look can afford.
    let long = asset.width.max(asset.height).max(1) as f32;
    let boxed = (w * asset.width as f32).max(h * asset.height as f32) / long;
    let side = (PATCH / boxed).clamp(96.0, 720.0).round() as u32;
    let (solo, size) = solo(project, clip, asset, side);
    let every = STEP_US.max(clip.duration_us / MAX_LOOKS);
    let from = (at_us - clip.start_us).clamp(0, clip.duration_us - 1);
    let ahead: Vec<i64> = (0..).map(|i| from + i * every).take_while(|&u| u < clip.duration_us).collect();
    let behind: Vec<i64> = (0..).map(|i| from - i * every).take_while(|&u| u >= 0).collect();
    let total = ahead.len() + behind.len();
    let done = AtomicUsize::new(0);
    let pixels = [x * size.0 as f32, y * size.1 as f32, w * size.0 as f32, h * size.1 as f32];
    let (ahead_found, behind_found) = std::thread::scope(|scope| {
        let (solo, done, ahead, behind) = (&solo, &done, &ahead[..], &behind[..]);
        let run = move |times: &[i64]| track(solo, size, times, pixels, cancel, done);
        let back = scope.spawn(move || run(behind));
        let on = scope.spawn(move || run(ahead));
        while !(back.is_finished() && on.is_finished()) {
            progress(done.load(Ordering::Relaxed) as f32 / total as f32);
            std::thread::sleep(Duration::from_millis(50));
        }
        let join = |h: std::thread::ScopedJoinHandle<'_, Result<Vec<Option<[f32; 2]>>>>| {
            h.join().map_err(|_| anyhow::anyhow!("JOB_FAILED: the follow worker panicked")).and_then(|r| r)
        };
        Ok::<_, anyhow::Error>((join(on)?, join(back)?))
    })?;
    check_cancel(cancel)?;

    // In clip order; the look at `from` is in both runs.
    let times: Vec<i64> = behind.iter().rev().chain(&ahead[1..]).copied().collect();
    let track: Vec<Option<[f32; 2]>> = behind_found.into_iter().rev().chain(ahead_found.into_iter().skip(1)).collect();
    let found = track.iter().filter(|f| f.is_some()).count() as i64;
    // Placed and moved at Reframe's pace, from the looks half a second apart.
    let pace = (LOOK_EVERY_US / every).max(1) as usize;
    let (times, track): (Vec<i64>, Vec<Option<[f32; 2]>>) =
        times.into_iter().zip(track).enumerate().filter(|(i, _)| i % pace == 0).map(|(_, look)| look).unzip();
    let at = smooth(&track).context("JOB_FAILED: the boxed thing was not found where it was boxed")?;
    let positions: Vec<[f32; 2]> = at.iter().map(|&p| placement.position(p, placement.aim)).collect();
    let transform = |[x, y]: [f32; 2]| Transform { x, y, scale: placement.scale, rotation: 0.0, ..base };
    let keyframes: Vec<Keyframe> = keys(&times, &positions)
        .into_iter()
        .map(|(t_us, p)| Keyframe { t_us, transform: transform(p), ease: Ease::Smooth })
        .collect();
    Ok(Followed {
        keyframes: keyframes.len(),
        commands: vec![
            update(clip_id, transform(positions[0])),
            EditCmd::SetKeyframes { clip_id: clip_id.into(), keyframes },
        ],
        found_us: (found * every).min(clip.duration_us),
        lost_us: ((total as i64 - 1 - found) * every).clamp(0, clip.duration_us),
    })
}

/// Follows `region` (pixels of the frame at `times[0]`) through the frames at `times`, in that order: its centre in
/// fractions of the picture at each, or None where it was not found.
fn track(
    solo: &Project,
    (width, height): (u32, u32),
    times: &[i64],
    region: [f32; 4],
    cancel: &AtomicBool,
    done: &AtomicUsize,
) -> Result<Vec<Option<[f32; 2]>>> {
    if times.is_empty() {
        return Ok(Vec::new());
    }
    let mut renderer = Renderer::new().context("Starting frame renderer")?;
    let mut tracker: Option<Tracker> = None;
    let mut out = Vec::with_capacity(times.len());
    for run in times.chunks(RUN) {
        // Drawn in the file's order, read in the run's.
        let mut order: Vec<usize> = (0..run.len()).collect();
        order.sort_by_key(|&i| run[i]);
        let mut frames: Vec<Option<Gray>> = (0..run.len()).map(|_| None).collect();
        for i in order {
            check_cancel(cancel)?;
            let rgba = renderer.render(solo, run[i], width, height, Wait::Exact, false).context("Drawing a frame")?;
            frames[i] = Some(Gray::new(&Frame { width, height, rgba }));
        }
        for frame in frames.into_iter().flatten() {
            let found = match &mut tracker {
                Some(tracker) => tracker.next(&frame),
                None => {
                    let first = Tracker::new(&frame, region);
                    let centre = first.centre;
                    tracker = Some(first);
                    Some(centre)
                }
            };
            out.push(found.map(|[x, y]| [x / width as f32, y / height as f32]));
            done.fetch_add(1, Ordering::Relaxed);
        }
    }
    Ok(out)
}

/// Brightness of a frame, row by row, 0 to 1.
struct Gray {
    width: usize,
    height: usize,
    px: Vec<f32>,
}

impl Gray {
    fn new(frame: &Frame) -> Gray {
        Gray { width: frame.width as usize, height: frame.height as usize, px: frame.luma() }
    }

    /// Bilinear at a point in pixels; past the edge the edge goes on.
    fn at(&self, x: f32, y: f32) -> f32 {
        let x = (x - 0.5).clamp(0.0, (self.width - 1) as f32);
        let y = (y - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let row = |y: usize| {
            let r = &self.px[y * self.width..];
            r[x0] + (r[x1] - r[x0]) * fx
        };
        row(y0) + (row(y1) - row(y0)) * fy
    }

    /// `cols`×`rows` samples spaced `cell` pixels apart, the first at `origin`.
    fn grid(&self, origin: [f32; 2], cell: [f32; 2], cols: usize, rows: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(cols * rows);
        for r in 0..rows {
            for c in 0..cols {
                out.push(self.at(origin[0] + c as f32 * cell[0], origin[1] + r as f32 * cell[1]));
            }
        }
        out
    }
}

/// A match at least this alike is the thing.
const FOUND: f32 = 0.5;
/// A match this alike also teaches the tracker how the thing looks now, so it follows a turn or a change of light.
const LEARN: f32 = 0.75;
const LEARN_RATE: f32 = 0.1;
/// Sizes tried against the last one, so a thing coming closer stays matched.
const SCALES: [f32; 3] = [0.95, 1.0, 1.05];

/// The thing being followed: how its patch looks and where it was last.
struct Tracker {
    /// Brightness of the patch, less its mean, at unit length, `cols`×`rows`.
    look: Vec<f32>,
    cols: usize,
    rows: usize,
    centre: [f32; 2],
    size: [f32; 2],
    first: [f32; 2],
    /// How far it moved in the last look, expected again.
    step: [f32; 2],
    /// How far around the expected place it is looked for, in its own sizes; wider while it is lost.
    reach: f32,
}

impl Tracker {
    fn new(frame: &Gray, [x, y, w, h]: [f32; 4]) -> Tracker {
        let k = PATCH / w.max(h);
        let (cols, rows) = (((w * k).round() as usize).max(4), ((h * k).round() as usize).max(4));
        let mut tracker = Tracker {
            look: Vec::new(),
            cols,
            rows,
            centre: [x + w / 2.0, y + h / 2.0],
            size: [w, h],
            first: [w, h],
            step: [0.0; 2],
            reach: 1.0,
        };
        tracker.look = normalised(tracker.patch(frame, tracker.centre, tracker.size)).0;
        tracker
    }

    /// The patch of `frame` of `size` around `centre`, sampled as the look is.
    fn patch(&self, frame: &Gray, centre: [f32; 2], size: [f32; 2]) -> Vec<f32> {
        let cell = [size[0] / self.cols as f32, size[1] / self.rows as f32];
        let origin = [0, 1].map(|i| centre[i] - size[i] / 2.0 + cell[i] / 2.0);
        frame.grid(origin, cell, self.cols, self.rows)
    }

    /// Where the thing is in `frame`, in pixels, or None when nothing there looks enough like it.
    fn next(&mut self, frame: &Gray) -> Option<[f32; 2]> {
        let guess = [0, 1].map(|i| self.centre[i] + self.step[i]);
        let mut best: Option<(f32, [f32; 2], f32)> = None;
        for scale in SCALES {
            let size = self.size.map(|v| v * scale);
            let cell = [size[0] / self.cols as f32, size[1] / self.rows as f32];
            let reach = (self.reach * size[0].max(size[1])).max(4.0);
            let (dx, dy) = ((reach / cell[0]).ceil() as usize, (reach / cell[1]).ceil() as usize);
            let (wc, wr) = (self.cols + 2 * dx, self.rows + 2 * dy);
            let origin = [0, 1].map(|i| guess[i] - size[i] / 2.0 + cell[i] / 2.0);
            let window = frame.grid([origin[0] - dx as f32 * cell[0], origin[1] - dy as f32 * cell[1]], cell, wc, wr);
            let (score, [ox, oy]) = self.best_offset(&window, wc, wr);
            // The same size wins a tie, so the box does not creep.
            let score = if scale == 1.0 { score } else { score - 0.01 };
            let at = [guess[0] + (ox as f32 - dx as f32) * cell[0], guess[1] + (oy as f32 - dy as f32) * cell[1]];
            if best.is_none_or(|(s, ..)| score > s) {
                best = Some((score, at, scale));
            }
        }
        let (score, at, scale) = best?;
        if score < FOUND {
            self.step = [0.0; 2];
            self.reach = (self.reach * 1.5).min(3.0);
            return None;
        }
        let at = [at[0].clamp(0.0, frame.width as f32), at[1].clamp(0.0, frame.height as f32)];
        self.step = [0, 1].map(|i| (self.step[i] + at[i] - self.centre[i]) / 2.0);
        self.centre = at;
        self.size = [0, 1].map(|i| (self.size[i] * scale).clamp(self.first[i] / 3.0, self.first[i] * 3.0));
        self.reach = 1.0;
        if score >= LEARN {
            let now = normalised(self.patch(frame, at, self.size)).0;
            let blend: Vec<f32> =
                self.look.iter().zip(&now).map(|(a, b)| a * (1.0 - LEARN_RATE) + b * LEARN_RATE).collect();
            self.look = normalised(blend).0;
        }
        Some(at)
    }

    /// The offset in `window` (`wc`×`wr` samples) where the patch matches it best, and how well: every second
    /// offset first, then the ones around the best of those.
    fn best_offset(&self, window: &[f32], wc: usize, wr: usize) -> (f32, [usize; 2]) {
        let (max_x, max_y) = (wc - self.cols, wr - self.rows);
        let mut best = (f32::MIN, [0, 0]);
        let consider = |ox: usize, oy: usize, best: &mut (f32, [usize; 2])| {
            let score = self.alike(window, wc, ox, oy);
            if score > best.0 {
                *best = (score, [ox, oy]);
            }
        };
        for oy in (0..=max_y).step_by(2) {
            for ox in (0..=max_x).step_by(2) {
                consider(ox, oy, &mut best);
            }
        }
        let [bx, by] = best.1;
        for oy in by.saturating_sub(1)..=(by + 1).min(max_y) {
            for ox in bx.saturating_sub(1)..=(bx + 1).min(max_x) {
                consider(ox, oy, &mut best);
            }
        }
        best
    }

    /// Normalised cross-correlation of the look with the window's patch at `ox`, `oy`: 1 for the same picture up to
    /// brightness and contrast, 0 for a flat patch.
    fn alike(&self, window: &[f32], wc: usize, ox: usize, oy: usize) -> f32 {
        let (mut dot, mut sum, mut squares) = (0.0f32, 0.0f32, 0.0f32);
        for r in 0..self.rows {
            let row = &window[(oy + r) * wc + ox..][..self.cols];
            let look = &self.look[r * self.cols..][..self.cols];
            for (a, b) in look.iter().zip(row) {
                dot += a * b;
                sum += b;
                squares += b * b;
            }
        }
        let n = (self.cols * self.rows) as f32;
        let spread = (squares - sum * sum / n).max(0.0).sqrt();
        if spread < 1e-3 { 0.0 } else { dot / spread }
    }
}

/// The values less their mean, at unit length, and that length.
fn normalised(mut values: Vec<f32>) -> (Vec<f32>, f32) {
    let mean = values.iter().sum::<f32>() / values.len().max(1) as f32;
    values.iter_mut().for_each(|v| *v -= mean);
    let length = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    if length > 1e-6 {
        values.iter_mut().for_each(|v| *v /= length);
    }
    (values, length)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A textured background with a brighter textured square of `side` pixels at `at`, its top-left corner.
    fn scene(width: usize, height: usize, at: [f32; 2], side: f32, hidden: bool) -> Gray {
        let texture = |x: f32, y: f32, k: f32| 0.5 + 0.25 * ((x * k).sin() * (y * 1.3 * k).cos());
        let mut px = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let (fx, fy) = (x as f32, y as f32);
                let inside = !hidden && (at[0]..at[0] + side).contains(&fx) && (at[1]..at[1] + side).contains(&fy);
                px.push(if inside {
                    0.3 + 0.6
                        * (((fx - at[0]) / 5.0).floor() as i32 + ((fy - at[1]) / 7.0).floor() as i32).rem_euclid(2)
                            as f32
                } else {
                    texture(fx, fy, 0.21)
                });
            }
        }
        Gray { width, height, px }
    }

    #[test]
    fn a_boxed_thing_is_followed_across_the_picture_and_found_again_after_it_was_hidden() {
        let side = 30.0;
        let path = |i: usize| [20.0 + i as f32 * 6.0, 40.0 + (i as f32 * 0.3).sin() * 15.0];
        let first = scene(320, 180, path(0), side, false);
        let mut tracker = Tracker::new(&first, [path(0)[0], path(0)[1], side, side]);
        for i in 1..40 {
            // Hidden behind something for three looks, while it goes on moving.
            let hidden = (20..23).contains(&i);
            let found = tracker.next(&scene(320, 180, path(i), side, hidden));
            let centre = [path(i)[0] + side / 2.0, path(i)[1] + side / 2.0];
            match found {
                Some(at) => {
                    assert!(!hidden, "found a hidden thing at look {i}: {at:?}");
                    assert!((at[0] - centre[0]).hypot(at[1] - centre[1]) < 3.0, "look {i}: {at:?}, not {centre:?}");
                }
                None => assert!(hidden, "lost at look {i}"),
            }
        }
    }

    #[test]
    fn a_flat_patch_matches_nothing() {
        let flat = Gray { width: 64, height: 64, px: vec![0.5; 64 * 64] };
        let mut tracker = Tracker::new(&scene(64, 64, [10.0, 10.0], 20.0, false), [10.0, 10.0, 20.0, 20.0]);
        assert_eq!(tracker.next(&flat), None);
    }

    /// The interview filmed wide, her face crossing from a third to two thirds of the way at 5 s (scripts/fixtures.sh),
    /// boxed around her head at 2 s on a vertical canvas. The detector finds the face whole and inside what Reels
    /// leaves free in every quarter second of the result as the renderer draws it.
    #[test]
    #[ignore = "Requires scripts/fixtures.sh media and models; run with scripts/isolated.sh"]
    fn a_boxed_head_crossing_a_wide_interview_stays_in_a_vertical_frame() {
        use crate::nets::Yunet;
        use nuzky_engine::model::Canvas;
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp-test/wide-head.mp4");
        let file = dunce::canonicalize(&file).expect("tmp-test/wide-head.mp4 is missing: run scripts/fixtures.sh");
        let data = std::env::var_os("XDG_DATA_HOME").expect("run with scripts/isolated.sh");
        let models = std::path::PathBuf::from(data).join("nuzky/models");
        let mut project = Project::new("follow");
        project.canvas = Canvas { width: 1080, height: 1920, ..project.canvas };
        let asset = nuzky_engine::media::probe(&file, "wide".into()).unwrap();
        project.apply(EditCmd::AddAssets { assets: vec![asset.clone()] }).unwrap();
        project.apply(EditCmd::AddClip { asset_id: "wide".into(), start_us: None, track_id: None }).unwrap();
        let clip = project.tracks[0].clips[0].id.clone();

        // The box a person draws around her head: the face as found, half as large again.
        let mut source = crate::source::Source::new(&asset, 960).unwrap();
        let (w, h) = source.size();
        let mut yunet = Yunet::load(&models).unwrap();
        let mut faces = yunet.detect(&source.frame(2_000_000).unwrap(), &AtomicBool::new(false)).unwrap();
        faces.sort_by(|a, b| (b.rect[2] * b.rect[3]).total_cmp(&(a.rect[2] * a.rect[3])));
        let [fx, fy, fw, fh] = faces[0].rect;
        let region = [(fx - fw / 4.0) / w as f32, (fy - fh / 4.0) / h as f32, 1.5 * fw / w as f32, 1.5 * fh / h as f32];
        let followed = follow(&project, &clip, 2_000_000, region, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(followed.lost_us, 0);
        for cmd in followed.commands {
            project.apply(cmd).unwrap();
        }

        let (w, h) = (540, 960);
        let area = Canvas { width: w, height: h, ..project.canvas.clone() }.safe_area().unwrap();
        let mut renderer = Renderer::new().unwrap();
        for t in (0..40).map(|i| 125_000 + i * 250_000) {
            let rgba = renderer.render(&project, t, w, h, Wait::Exact, false).unwrap();
            let mut faces = yunet.detect(&Frame { width: w, height: h, rgba }, &AtomicBool::new(false)).unwrap();
            faces.sort_by(|a, b| (b.rect[2] * b.rect[3]).total_cmp(&(a.rect[2] * a.rect[3])));
            let [x, y, fw, fh] = faces.first().unwrap_or_else(|| panic!("no face at {t} µs")).rect;
            let centre = x + fw / 2.0;
            assert!(x >= 0.0 && x + fw <= w as f32 && y >= 0.0 && y + fh <= h as f32, "face cut at {t} µs");
            assert!(area.left < centre && centre < area.right, "face outside the safe area at {t} µs: {centre}");
        }
    }
}

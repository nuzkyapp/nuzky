//! Where in its file a media clip plays at each moment of the timeline: its speed, a speed ramp and playing
//! backwards. A ramp is a shape: the speed moves evenly between its points and holds before the first and after the
//! last, scaled so its average is the clip's `speed`. Without a ramp, forwards, the clip plays
//! `source_in_us + offset * speed`, exactly as before ramps.

use anyhow::{Result, ensure};

use crate::edit::{MAX_SPEED, MIN_SPEED};
use crate::model::{Clip, ClipContent, SpeedPoint};

/// The most points a ramp has: the 16 the curve lets a person place, and room for the two held edges a trim past a
/// clip's ends adds (`Retime::part`).
pub const MAX_RAMP_POINTS: usize = 18;

/// A media clip's map from timeline offset (µs from its start, any value) to source time (µs in its file).
#[derive(Clone, Copy, Debug)]
pub struct Retime<'a> {
    source_in: f64,
    duration: f64,
    speed: f64,
    reverse: bool,
    ramp: &'a [SpeedPoint],
    /// The ramp's average over the clip, by which its shape is divided.
    mean: f64,
}

impl<'a> Retime<'a> {
    /// The map of a media clip with a speed; None for text and freeze frames.
    pub fn of(clip: &'a Clip) -> Option<Self> {
        let ClipContent::Media { source_in_us, speed, reverse, ramp, .. } = &clip.content else { return None };
        (*speed > 0.0).then(|| Self::new(*source_in_us, clip.duration_us, *speed, *reverse, ramp))
    }

    pub fn new(source_in_us: i64, duration_us: i64, speed: f32, reverse: bool, ramp: &'a [SpeedPoint]) -> Self {
        let mean = if ramp.is_empty() { 1.0 } else { area(ramp, 1.0) - area(ramp, 0.0) };
        Self {
            source_in: source_in_us as f64,
            duration: duration_us.max(1) as f64,
            speed: speed as f64,
            reverse,
            ramp,
            mean: if mean > 0.0 { mean } else { 1.0 },
        }
    }

    /// Whether this is more than a steady speed forwards.
    pub fn is_retimed(&self) -> bool {
        self.reverse || !self.ramp.is_empty()
    }

    pub fn reverse(&self) -> bool {
        self.reverse
    }

    /// How much source the clip plays from its start to `offset`, held at the edge speeds outside the clip.
    fn travel(&self, offset: f64) -> f64 {
        if self.ramp.is_empty() {
            return offset * self.speed;
        }
        let x = offset / self.duration;
        self.duration * self.speed * (area(self.ramp, x) - area(self.ramp, 0.0)) / self.mean
    }

    /// Where `travel` reaches `distance`.
    fn offset_for(&self, distance: f64) -> f64 {
        if self.ramp.is_empty() {
            return distance / self.speed;
        }
        let target = distance * self.mean / (self.duration * self.speed) + area(self.ramp, 0.0);
        area_inverse(self.ramp, target) * self.duration
    }

    /// The end of the clip's part of its file.
    fn source_end(&self) -> f64 {
        self.source_in + self.duration * self.speed
    }

    /// Source time at `offset`, unclamped and unrounded.
    pub fn source_at(&self, offset: f64) -> f64 {
        if self.reverse { self.source_end() - self.travel(offset) } else { self.source_in + self.travel(offset) }
    }

    /// The timeline offset at which source time `source` plays, unclamped: before 0 or after the clip's length when
    /// the clip does not play it.
    pub fn offset_of(&self, source: f64) -> f64 {
        self.offset_for(if self.reverse { self.source_end() - source } else { source - self.source_in })
    }

    /// The frame to show at `offset`: a clip holds its first frame before its start, and one played backwards
    /// shows the frame just before each moment, so its first frame is the last of its part of the file.
    pub fn frame_at(&self, offset: i64) -> i64 {
        let offset = offset.max(0) as f64;
        if self.reverse {
            ((self.source_end() - self.travel(offset)).round() as i64 - 1).max(0)
        } else if self.ramp.is_empty() {
            self.source_in as i64 + (offset * self.speed).round() as i64
        } else {
            (self.source_in + self.travel(offset)).round() as i64
        }
    }

    /// How fast the clip plays at `offset`, never negative: 2 is twice as fast, forwards or backwards.
    pub fn speed_at(&self, offset: f64) -> f64 {
        if self.ramp.is_empty() {
            return self.speed;
        }
        self.speed * shape(self.ramp, offset / self.duration) / self.mean
    }

    /// The clip cut down to timeline offsets `from..to` of this one, which may reach past its edges, where it holds
    /// its edge speeds: its source start, average speed and ramp, so it plays as this part did.
    pub fn part(&self, from: i64, to: i64) -> (i64, f32, Vec<SpeedPoint>) {
        let (a, b) = (from as f64, to.max(from + 1) as f64);
        let (start, end) = (self.travel(a), self.travel(b));
        let source_in = if self.reverse { self.source_end() - end } else { self.source_in + start };
        let source_in = source_in.round() as i64;
        if self.ramp.is_empty() {
            return (source_in, self.speed as f32, Vec::new());
        }
        let speed = ((end - start) / (b - a)) as f32;
        let (xa, xb) = (a / self.duration, b / self.duration);
        let point = |x: f64, at: f32| SpeedPoint {
            at,
            speed: ((self.speed * shape(self.ramp, x) / self.mean) as f32).clamp(MIN_SPEED, MAX_SPEED),
        };
        let mut ramp = vec![point(xa, 0.0)];
        for p in self.ramp.iter().filter(|p| xa < p.at as f64 && (p.at as f64) < xb) {
            let at = ((p.at as f64 - xa) / (xb - xa)) as f32;
            if at > ramp.last().unwrap().at && at < 1.0 {
                ramp.push(point(p.at as f64, at));
            }
        }
        ramp.push(point(xb, 1.0));
        // Reaching past the clip holds its edge speeds, so the old edge points fall on a straight line; a flat part
        // stays a ramp, so Keep pitch never starts to apply to it.
        let mut i = 1;
        while i + 1 < ramp.len() {
            if ramp[i - 1].speed == ramp[i].speed && ramp[i].speed == ramp[i + 1].speed {
                ramp.remove(i);
            } else {
                i += 1;
            }
        }
        (source_in, speed.clamp(MIN_SPEED, MAX_SPEED), ramp)
    }
}

/// The source time to show at timeline time `t_us`, for a media clip: its frame (`Retime::frame_at`).
pub fn source_time(clip: &Clip, t_us: i64) -> i64 {
    Retime::of(clip).map_or_else(
        || match clip.content {
            ClipContent::Media { source_in_us, .. } => source_in_us,
            ClipContent::Text { .. } | ClipContent::Adjustment { .. } => 0,
        },
        |map| map.frame_at(t_us - clip.start_us),
    )
}

/// Whether a media clip plays at more than a steady speed forwards.
pub fn is_retimed(clip: &Clip) -> bool {
    matches!(&clip.content, ClipContent::Media { reverse, ramp, .. } if *reverse || !ramp.is_empty())
}

/// Whether `next` starts where `clip` ends on the timeline and plays on in the same file from where it stops, in the
/// same direction and at the same speed, as the two halves of a split do; the file may jump by `tolerance_us`.
pub fn plays_on(clip: &Clip, next: &Clip, tolerance_us: f64) -> bool {
    let (ClipContent::Media { asset_id: a, .. }, ClipContent::Media { asset_id: b, .. }) =
        (&clip.content, &next.content)
    else {
        return false;
    };
    let (Some(x), Some(y)) = (Retime::of(clip), Retime::of(next)) else { return false };
    let end = clip.duration_us as f64;
    a == b
        && clip.end_us() == next.start_us
        && x.reverse() == y.reverse()
        && (x.source_at(end) - y.source_at(0.0)).abs() <= tolerance_us
        && (x.speed_at(end) - y.speed_at(0.0)).abs() <= 1e-3 * x.speed_at(end)
}

/// A ramp as an edit sets it: 2 to `MAX_RAMP_POINTS` points from 0 to 1 in order, at speeds from `MIN_SPEED` to
/// `MAX_SPEED`. Returns the ramp and its average, the clip's speed. A ramp drawn flat stays a ramp, so a point dragged
/// level with the others does not turn the clip back to one speed under the pointer.
pub fn checked_ramp(points: Vec<SpeedPoint>) -> Result<(Vec<SpeedPoint>, f32)> {
    ensure!(
        (2..=MAX_RAMP_POINTS).contains(&points.len()),
        "A speed ramp has 2 to {MAX_RAMP_POINTS} points, or none to play at one speed"
    );
    ensure!(
        points.first().is_some_and(|p| p.at == 0.0) && points.last().is_some_and(|p| p.at == 1.0),
        "A speed ramp's first point is at 0, its start, and its last at 1, its end"
    );
    ensure!(
        points.windows(2).all(|w| w[0].at < w[1].at) && points.iter().all(|p| p.at.is_finite()),
        "A speed ramp's points go in order along the clip, each after the one before"
    );
    ensure!(
        points.iter().all(|p| p.speed.is_finite() && (MIN_SPEED..=MAX_SPEED).contains(&p.speed)),
        "A speed ramp's speeds are from {MIN_SPEED}x to {MAX_SPEED}x"
    );
    let mean = (area(&points, 1.0) - area(&points, 0.0)) as f32;
    Ok((points, mean.clamp(MIN_SPEED, MAX_SPEED)))
}

/// Whether a ramp a project holds can be played: the shape `checked_ramp` gives.
pub fn valid_ramp(points: &[SpeedPoint]) -> bool {
    points.is_empty()
        || ((2..=MAX_RAMP_POINTS).contains(&points.len())
            && points[0].at == 0.0
            && points[points.len() - 1].at == 1.0
            && points.windows(2).all(|w| w[0].at < w[1].at)
            && points.iter().all(|p| p.speed.is_finite() && (MIN_SPEED..=MAX_SPEED).contains(&p.speed)))
}

/// Every point times `factor`, kept within the speed limits, and the new average.
pub fn scaled_ramp(points: &[SpeedPoint], factor: f32) -> (Vec<SpeedPoint>, f32) {
    let scaled: Vec<SpeedPoint> =
        points.iter().map(|p| SpeedPoint { at: p.at, speed: (p.speed * factor).clamp(MIN_SPEED, MAX_SPEED) }).collect();
    let mean = (area(&scaled, 1.0) - area(&scaled, 0.0)) as f32;
    (scaled, mean.clamp(MIN_SPEED, MAX_SPEED))
}

/// The ramp's speed at share `x` of the clip.
fn shape(points: &[SpeedPoint], x: f64) -> f64 {
    let (first, last) = (points[0], points[points.len() - 1]);
    if x <= first.at as f64 {
        return first.speed as f64;
    }
    if x >= last.at as f64 {
        return last.speed as f64;
    }
    let i = points.partition_point(|p| (p.at as f64) <= x) - 1;
    let (p, q) = (points[i], points[i + 1]);
    let t = (x - p.at as f64) / (q.at - p.at) as f64;
    p.speed as f64 + (q.speed - p.speed) as f64 * t
}

/// The ramp's speed summed from its first point to share `x` of the clip, negative before it.
fn area(points: &[SpeedPoint], x: f64) -> f64 {
    let first = points[0];
    if x <= first.at as f64 {
        return first.speed as f64 * (x - first.at as f64);
    }
    let mut sum = 0.0;
    for pair in points.windows(2) {
        let (p, q) = (pair[0], pair[1]);
        let (a, b) = (p.at as f64, q.at as f64);
        if x <= b {
            let w = x - a;
            let slope = (q.speed - p.speed) as f64 / (b - a);
            return sum + p.speed as f64 * w + slope * w * w / 2.0;
        }
        sum += (p.speed + q.speed) as f64 / 2.0 * (b - a);
    }
    let last = points[points.len() - 1];
    sum + last.speed as f64 * (x - last.at as f64)
}

/// The share `x` of the clip where `area` reaches `target`.
fn area_inverse(points: &[SpeedPoint], target: f64) -> f64 {
    let first = points[0];
    if target <= 0.0 {
        return first.at as f64 + target / first.speed as f64;
    }
    let mut sum = 0.0;
    for pair in points.windows(2) {
        let (p, q) = (pair[0], pair[1]);
        let (a, b) = (p.at as f64, q.at as f64);
        let piece = (p.speed + q.speed) as f64 / 2.0 * (b - a);
        if target <= sum + piece {
            let rest = target - sum;
            let v = p.speed as f64;
            let slope = (q.speed - p.speed) as f64 / (b - a);
            // v·w + slope·w²/2 = rest, in the form that stays exact when the slope is near 0.
            let w = 2.0 * rest / (v + (v * v + 2.0 * slope * rest).max(0.0).sqrt());
            return a + w;
        }
        sum += piece;
    }
    let last = points[points.len() - 1];
    last.at as f64 + (target - sum) / last.speed as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(points: &[(f32, f32)]) -> Vec<SpeedPoint> {
        points.iter().map(|&(at, speed)| SpeedPoint { at, speed }).collect()
    }

    #[test]
    fn steady_forwards_is_the_old_formula() {
        let map = Retime::new(500_000, 4_000_000, 2.0, false, &[]);
        assert_eq!(map.frame_at(1_000_000), 2_500_000);
        assert_eq!(map.frame_at(-5), 500_000);
        assert_eq!(map.part(1_000_000, 3_000_000), (2_500_000, 2.0, Vec::new()));
        assert!((map.offset_of(2_500_000.0) - 1_000_000.0).abs() < 1e-6);
    }

    #[test]
    fn backwards_starts_on_the_last_frame_and_splits_into_mirrored_parts() {
        // Source 1 s to 3 s at 2x over 1 s of timeline.
        let map = Retime::new(1_000_000, 1_000_000, 2.0, true, &[]);
        assert_eq!(map.frame_at(0), 2_999_999);
        assert_eq!(map.frame_at(250_000), 2_499_999);
        assert!((map.source_at(1_000_000.0) - 1_000_000.0).abs() < 1e-6);
        // The first half of the timeline plays the second half of the source.
        assert_eq!(map.part(0, 500_000).0, 2_000_000);
        assert_eq!(map.part(500_000, 1_000_000).0, 1_000_000);
        assert!((map.offset_of(2_000_000.0) - 500_000.0).abs() < 1e-6);
    }

    #[test]
    fn a_ramp_covers_the_same_source_and_moves_through_it_at_its_speeds() {
        // From 2x down to 0.5x: an average of 1.25x.
        let points = ramp(&[(0.0, 2.0), (1.0, 0.5)]);
        let (points, mean) = checked_ramp(points).unwrap();
        assert_eq!(mean, 1.25);
        let map = Retime::new(0, 4_000_000, mean, false, &points);
        assert!((map.source_at(4_000_000.0) - 5_000_000.0).abs() < 1e-3);
        assert!((map.speed_at(0.0) - 2.0).abs() < 1e-6 && (map.speed_at(4_000_000.0) - 0.5).abs() < 1e-6);
        // Faster at first: the first half of the time covers more than half of the source.
        assert!(map.source_at(2_000_000.0) > 2_500_000.0 + 1.0);
        for t in [0.0, 300_000.0, 2_000_000.0, 3_999_999.0, 5_000_000.0, -1_000.0] {
            assert!((map.offset_of(map.source_at(t)) - t).abs() < 1e-3, "{t}");
        }
        // Past its end the clip plays on at its last speed.
        assert!((map.source_at(5_000_000.0) - map.source_at(4_000_000.0) - 500_000.0).abs() < 1e-3);
    }

    #[test]
    fn parts_of_a_ramp_play_as_the_whole_did() {
        let (points, mean) = checked_ramp(ramp(&[(0.0, 1.0), (0.5, 0.25), (1.0, 3.0)])).unwrap();
        for reverse in [false, true] {
            let whole = Retime::new(2_000_000, 3_000_000, mean, reverse, &points);
            let cut = 1_100_000;
            let (src_a, speed_a, ramp_a) = whole.part(0, cut);
            let (src_b, speed_b, ramp_b) = whole.part(cut, 3_000_000);
            let first = Retime::new(src_a, cut, speed_a, reverse, &ramp_a);
            let second = Retime::new(src_b, 3_000_000 - cut, speed_b, reverse, &ramp_b);
            for t in (0..3_000_000).step_by(97_003) {
                let t = t as f64;
                let part = if t < cut as f64 { first.source_at(t) } else { second.source_at(t - cut as f64) };
                assert!((part - whole.source_at(t)).abs() < 2.0, "{reverse} {t}: {part} vs {}", whole.source_at(t));
            }
            assert!(valid_ramp(&ramp_a) && valid_ramp(&ramp_b));
            assert!((first.speed_at(cut as f64) - second.speed_at(0.0)).abs() < 1e-4);
        }
    }

    #[test]
    fn a_flat_part_stays_a_ramp_reaching_past_one_holds_its_edges_and_bad_ramps_are_refused() {
        let flat = ramp(&[(0.0, 2.0), (1.0, 2.0)]);
        assert_eq!(checked_ramp(flat.clone()).unwrap(), (flat, 2.0));
        let (points, mean) = checked_ramp(ramp(&[(0.0, 1.0), (0.5, 0.5), (1.0, 0.5)])).unwrap();
        let map = Retime::new(0, 1_000_000, mean, false, &points);
        let (_, speed, cut) = map.part(600_000, 1_000_000);
        assert!(cut.len() == 2 && (speed - 0.5).abs() < 1e-6, "{speed} {cut:?}");
        // Reaching 0.5 s past each end holds the edge speeds: the old end point on the flat stretch goes, the start
        // gains the corner where its hold meets the slope; reaching past again adds nothing more.
        let (src, sp, longer) = map.part(-500_000, 1_500_000);
        let speeds: Vec<f32> = longer.iter().map(|p| p.speed).collect();
        assert_eq!((longer.len(), speeds), (4, vec![1.0, 1.0, 0.5, 0.5]), "{longer:?}");
        let (_, _, again) = Retime::new(src, 2_000_000, sp, false, &longer).part(-200_000, 2_200_000);
        assert_eq!(again.len(), 4, "{again:?}");
        for bad in [
            ramp(&[(0.0, 1.0)]),
            ramp(&[(0.1, 1.0), (1.0, 2.0)]),
            ramp(&[(0.0, 1.0), (0.5, 2.0), (0.5, 1.0), (1.0, 2.0)]),
            ramp(&[(0.0, 1.0), (1.0, 20.0)]),
            ramp(&[(0.0, f32::NAN), (1.0, 1.0)]),
        ] {
            assert!(checked_ramp(bad.clone()).is_err() && !valid_ramp(&bad), "{bad:?}");
        }
    }
}

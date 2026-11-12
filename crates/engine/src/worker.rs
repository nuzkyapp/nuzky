//! Background decoding: one thread per clip keeps a short queue of converted frames
//! around the requested time, so the render thread never waits on a seek it can skip.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::media::{DecodedFrame, RgbaFrame, VideoDecoder};

const LOOKAHEAD_PLAYING: usize = 4;
/// Jumping further ahead than this seeks instead of decoding through.
const FORWARD_SEEK_US: i64 = 1_000_000;
const BLOCKING_TIMEOUT: Duration = Duration::from_secs(8);
/// A clip played backwards decodes its file forwards in pieces of about this many bytes of converted frames, at least
/// 4 frames, and keeps about two pieces, so playback neither seeks for every frame nor holds a whole file's frames.
const BACKWARD_BYTES: usize = 48 << 20;
const BACKWARD_FRAMES: std::ops::RangeInclusive<usize> = 4..=32;

#[derive(Default)]
struct State {
    want_us: i64,
    size: (u32, u32),
    playing: bool,
    /// The clip plays its file backwards, so frames before the wanted one are decoded ahead.
    backward: bool,
    /// Ascending by time. frames[i] is shown for [frames[i].t, frames[i+1].t).
    frames: VecDeque<RgbaFrame>,
    /// Time the queue was seeked to; frames[0] also covers [base_us, frames[0].t).
    base_us: i64,
    /// The decoder has no frames after the last queued one.
    eof: bool,
    /// The decoder's next frame does not follow the last queued one: it decoded a piece before the queue, or frames
    /// after the shown one were dropped while playing backwards. Frames after the queue then need a seek.
    detached: bool,
    /// HDR frames come in 16-bit floats, twice the bytes of 8-bit ones.
    deep: bool,
    /// Bumped when the queue is invalidated, so in-flight work is discarded.
    epoch: u64,
    error: Option<String>,
    shutdown: bool,
}

impl State {
    fn covering(&self, t: i64) -> Option<(usize, bool)> {
        let first = self.frames.front()?;
        if t < self.base_us {
            return None;
        }
        let idx = self.frames.iter().rposition(|f| f.t_us <= t).unwrap_or(0);
        if t < first.t_us && idx == 0 {
            // Between the seek point and the first real frame.
            return Some((0, true));
        }
        let exact = idx + 1 < self.frames.len() || self.eof;
        Some((idx, exact))
    }

    fn needs_seek(&self) -> bool {
        match (self.frames.front(), self.frames.back()) {
            (Some(_), Some(last)) => {
                self.want_us < self.base_us
                    || (!self.eof && self.want_us > last.t_us + FORWARD_SEEK_US)
                    || (self.detached && (self.want_us > last.t_us || (!self.reads_backward() && self.needs_ahead())))
            }
            _ => true,
        }
    }

    /// Converted frames in one piece of backward decoding.
    fn backward_frames(&self) -> usize {
        let texel = if self.deep { 8 } else { 4 };
        (BACKWARD_BYTES / (self.size.0 as usize * self.size.1 as usize * texel).max(1))
            .clamp(*BACKWARD_FRAMES.start(), *BACKWARD_FRAMES.end())
    }

    fn reads_backward(&self) -> bool {
        self.backward && self.playing
    }

    fn needs_more(&self) -> bool {
        if self.reads_backward() {
            let before = self.frames.iter().filter(|f| f.t_us <= self.want_us).count();
            return self.base_us > 0 && before <= self.backward_frames() / 2;
        }
        self.needs_ahead()
    }

    /// Frames after the wanted one, as playing forwards decodes them.
    fn needs_ahead(&self) -> bool {
        if self.eof {
            return false;
        }
        let ahead = self.frames.iter().filter(|f| f.t_us > self.want_us).count();
        ahead == 0 || (self.playing && ahead < LOOKAHEAD_PLAYING)
    }

    fn has_work(&self) -> bool {
        self.size.0 > 0 && self.error.is_none() && (self.needs_seek() || self.needs_more())
    }
}

struct Shared {
    state: Mutex<State>,
    cv: Condvar,
}

pub struct VideoWorker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    pub last_used: Instant,
    pub exact: bool,
    /// The preview proxy it decodes instead of the original file, if any.
    pub proxy: Option<PathBuf>,
}

impl VideoWorker {
    pub fn spawn(path: PathBuf) -> Self {
        Self::start(path, None, false, false)
    }

    /// For an HDR export, whose HDR frames it gives in 16-bit floats (`VideoDecoder::deep`).
    pub fn spawn_deep(path: PathBuf) -> Self {
        Self::start(path, None, false, true)
    }

    /// For the preview, which jumps around the file (`VideoDecoder::open_for_seeking`).
    pub fn spawn_for_seeking(path: PathBuf) -> Self {
        Self::start(path, None, true, false)
    }

    /// Decodes a preview proxy (`crate::proxy`), which gives the frames of its original at the same times.
    pub fn spawn_proxy(proxy: PathBuf) -> Self {
        Self::start(proxy.clone(), Some(proxy), false, false)
    }

    fn start(path: PathBuf, proxy: Option<PathBuf>, seeking: bool, deep: bool) -> Self {
        let state = State { deep, ..State::default() };
        let shared = Arc::new(Shared { state: Mutex::new(state), cv: Condvar::new() });
        let s = shared.clone();
        let is_proxy = proxy.is_some();
        let thread = std::thread::Builder::new()
            .name("video-decode".into())
            .spawn(move || run(s, path, is_proxy, seeking, deep))
            .expect("spawn decoder thread");
        Self { shared, thread: Some(thread), last_used: Instant::now(), exact: false, proxy }
    }

    pub fn error(&self) -> Option<String> {
        self.shared.state.lock().unwrap().error.clone()
    }

    /// Returns the frame to show at `t_us`. With `blocking`, waits for the exact frame;
    /// otherwise returns the nearest ready frame or `None` and lets decoding catch up. `backward` when the clip
    /// plays its file backwards.
    pub fn get(
        &mut self,
        t_us: i64,
        size: (u32, u32),
        playing: bool,
        blocking: bool,
        backward: bool,
    ) -> Option<RgbaFrame> {
        self.last_used = Instant::now();
        self.exact = false;
        let deadline = Instant::now() + BLOCKING_TIMEOUT;
        let mut st = self.shared.state.lock().unwrap();
        if st.size != size {
            st.size = size;
            st.frames.clear();
            st.epoch += 1;
        }
        st.want_us = t_us;
        st.playing = playing;
        st.backward = backward;
        loop {
            if let Some((idx, exact)) = st.covering(t_us) {
                // Drop frames that can no longer be shown: those already passed, and played backwards those after
                // the next one, which only tells the shown frame is exact.
                let shown = if backward {
                    if st.frames.len() > idx + 2 {
                        st.frames.truncate(idx + 2);
                        st.eof = false;
                        st.detached = true;
                    }
                    idx
                } else {
                    for _ in 0..idx {
                        st.frames.pop_front();
                    }
                    if idx > 0 {
                        st.base_us = st.frames[0].t_us;
                    }
                    0
                };
                let frame = st.frames[shown].clone();
                if exact || !blocking {
                    self.exact = exact;
                    self.shared.cv.notify_all();
                    return Some(frame);
                }
            }
            self.shared.cv.notify_all();
            let nearest = |st: &State| match backward {
                true => st.frames.back().cloned(),
                false => st.frames.front().cloned(),
            };
            if !blocking || st.error.is_some() {
                return nearest(&st);
            }
            let now = Instant::now();
            if now >= deadline {
                log::warn!("Timed out waiting for frame at {t_us} us");
                return nearest(&st);
            }
            st = self.shared.cv.wait_timeout(st, deadline - now).unwrap().0;
        }
    }
}

impl Drop for VideoWorker {
    fn drop(&mut self) {
        self.shared.state.lock().unwrap().shutdown = true;
        self.shared.cv.notify_all();
        if let Some(t) = self.thread.take() {
            t.join().ok();
        }
    }
}

fn run(shared: Arc<Shared>, path: PathBuf, proxy: bool, seeking: bool, deep: bool) {
    let opened = match (proxy, seeking) {
        (true, _) => VideoDecoder::open_proxy(&path),
        (false, true) => VideoDecoder::open_for_seeking(&path),
        (false, false) => VideoDecoder::open(&path),
    };
    let mut decoder = match opened {
        Ok(mut d) => {
            d.deep = deep;
            d
        }
        Err(e) => {
            let mut st = shared.state.lock().unwrap();
            st.error = Some(format!("{e:#}"));
            shared.cv.notify_all();
            return;
        }
    };

    loop {
        let (want, size, seek, epoch, backward, first, span) = {
            let mut st = shared.state.lock().unwrap();
            while !st.shutdown && !st.has_work() {
                st = shared.cv.wait(st).unwrap();
            }
            if st.shutdown {
                return;
            }
            let span = st.backward_frames() as i64 * decoder.frame_duration_us.max(1);
            (
                st.want_us,
                st.size,
                st.needs_seek(),
                st.epoch,
                st.reads_backward(),
                st.frames.front().map(|f| f.t_us),
                span,
            )
        };

        let result = match (seek, backward, first) {
            (true, true, _) => decode_piece(&mut decoder, (want - span).max(0), want, size),
            (true, false, _) => seek_to(&mut decoder, want, size),
            (false, true, Some(first)) => {
                decode_piece(&mut decoder, (first - span).max(0), first - 1, size).map(|step| match step {
                    Step::Piece { frames, base, .. } => {
                        Step::Before { frames: frames.into_iter().filter(|f| f.t_us < first).collect(), base, first }
                    }
                    other => other,
                })
            }
            (false, _, _) => decode_next(&mut decoder, size),
        };
        let mut st = shared.state.lock().unwrap();
        if st.epoch != epoch {
            continue;
        }
        match result {
            Ok(Step::Seeked { frames, eof }) => {
                st.frames = frames.into();
                st.base_us = want;
                st.eof = eof;
                st.detached = false;
            }
            Ok(Step::Piece { frames, base, eof }) => {
                st.frames = frames.into();
                st.base_us = base;
                st.eof = eof;
                st.detached = false;
            }
            // Only onto the piece it was decoded before; its start moves back even without a frame, so the next
            // piece starts earlier.
            Ok(Step::Before { frames, base, first }) => {
                if st.frames.front().is_some_and(|f| f.t_us == first) {
                    for frame in frames.into_iter().rev() {
                        st.frames.push_front(frame);
                    }
                    st.base_us = base.min(st.frames[0].t_us);
                    st.detached = true;
                }
            }
            Ok(Step::Next(Some(frame))) => st.frames.push_back(frame),
            Ok(Step::Next(None)) => st.eof = true,
            Err(e) => {
                log::error!("Decoding {} failed: {e:#}", path.display());
                st.error = Some(format!("{e:#}"));
            }
        }
        shared.cv.notify_all();
    }
}

enum Step {
    Seeked {
        frames: Vec<RgbaFrame>,
        eof: bool,
    },
    /// The frames from the one covering `base` on, for playing backwards.
    Piece {
        frames: Vec<RgbaFrame>,
        base: i64,
        eof: bool,
    },
    /// Frames before the queue's first frame `first`.
    Before {
        frames: Vec<RgbaFrame>,
        base: i64,
        first: i64,
    },
    Next(Option<RgbaFrame>),
}

/// Decodes from the frame covering `from` up to the frame after `until`, or to the end of the file, and converts
/// them. A file whose frame rate says less than it has keeps only the last frames, so memory stays bounded.
fn decode_piece(decoder: &mut VideoDecoder, from: i64, until: i64, size: (u32, u32)) -> anyhow::Result<Step> {
    decoder.seek(from)?;
    let [covering, next] = decoder.frame_covering(from, [None, None])?;
    let mut decoded: VecDeque<DecodedFrame> = [covering, next].into_iter().flatten().collect();
    while decoded.back().is_some_and(|(t, _)| *t <= until) {
        match decoder.next_frame()? {
            Some(frame) => decoded.push_back(frame),
            None => break,
        }
        if decoded.len() > *BACKWARD_FRAMES.end() * 2 {
            decoded.pop_front();
        }
    }
    let base = decoded.front().map_or(from, |(t, _)| (*t).min(from));
    let mut frames = Vec::with_capacity(decoded.len());
    for (t, frame) in &decoded {
        frames.push(decoder.convert(frame, *t, size.0, size.1)?);
    }
    Ok(Step::Piece { frames, base, eof: decoder.is_eof() })
}

/// Seeks to the frame covering `want`, keeping any decoded successor.
/// Only those frames get converted, so seeking through a long GOP stays cheap.
fn seek_to(decoder: &mut VideoDecoder, want: i64, size: (u32, u32)) -> anyhow::Result<Step> {
    decoder.seek(want)?;
    let [covering, next] = decoder.frame_covering(want, [None, None])?;
    let eof = decoder.is_eof();
    let mut frames = Vec::with_capacity(2);
    for (t, frame) in [covering, next].into_iter().flatten() {
        frames.push(decoder.convert(&frame, t, size.0, size.1)?);
    }
    Ok(Step::Seeked { frames, eof })
}

fn decode_next(decoder: &mut VideoDecoder, size: (u32, u32)) -> anyhow::Result<Step> {
    Ok(Step::Next(match decoder.next_frame()? {
        Some((t, f)) => Some(decoder.convert(&f, t, size.0, size.1)?),
        None => None,
    }))
}

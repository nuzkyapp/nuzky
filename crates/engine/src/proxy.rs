//! Preview proxies: a light H.264 copy of video that is slow to decode, such as HEVC from phones, made once
//! in the background. The preview and thumbnails decode it instead of the original once it is there; export
//! never reads it. A proxy keeps the original's frame times, colours and stored orientation, so the two
//! swap without moving a cut, and differs from it only by compression. It is a pure cache, named after the
//! file's path, size and time: a missing one only makes the preview slower.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::mpsc;

use anyhow::{Context as _, Result, bail};
use ff::software::scaling;
use ff::util::{color, format::Pixel, frame};
use ffmpeg_next as ff;

use crate::audio::lock_cache;
use crate::media::{
    VideoDecoder, file_key, is_full_range, is_image_format, matrix, open_input, set_sws_colorspace, sws_colorspace,
    video_stream,
};

/// Raised whenever proxies are made differently, so ones made before are made again.
pub const PROXY_VERSION: &str = "v2";
/// Proxies are at most 1080p: the preview is at most 1280 px on its longer side.
const SHORT_SIDE: u32 = 1080;
/// A keyframe every 15 frames, so a jump decodes at most 14 frames before the one it shows.
const GOP: u32 = 15;
/// Frame times in microseconds, the unit `VideoDecoder` reports them in.
const TIME_BASE: (i32, i32) = (1, 1_000_000);

pub fn proxy_path(cache_dir: &Path, source: &Path) -> PathBuf {
    cache_dir.join("proxy").join(format!("{}.{PROXY_VERSION}.mp4", file_key(&source.to_string_lossy())))
}

/// The proxy of `source`, when it has been made.
pub fn ready(cache_dir: &Path, source: &Path) -> Option<PathBuf> {
    Some(proxy_path(cache_dir, source)).filter(|path| path.exists())
}

/// The decoder the preview reads `source` with: its proxy once made, else the file itself.
pub fn open_decoder(cache_dir: &Path, source: &Path) -> Result<VideoDecoder> {
    match ready(cache_dir, source) {
        Some(proxy) => VideoDecoder::open_proxy(&proxy),
        None => VideoDecoder::open(source),
    }
}

/// Whether the video at `path` should preview from a proxy: HEVC, AV1 and VP9, which phones record and which
/// decode slowly, and anything larger than a proxy. Not stills, nor video with transparency, which a proxy
/// would show opaque.
pub fn wanted(path: &Path) -> Result<bool> {
    let input = open_input(path)?;
    let Some(stream) = video_stream(&input).filter(|_| !is_image_format(&input)) else { return Ok(false) };
    if ff::encoder::find_by_name("libx264").is_none() {
        return Ok(false);
    }
    let heavy = matches!(stream.parameters().id(), ff::codec::Id::HEVC | ff::codec::Id::AV1 | ff::codec::Id::VP9);
    let decoder = ff::codec::context::Context::from_parameters(stream.parameters())?.decoder().video()?;
    let alpha = decoder
        .format()
        .descriptor()
        .is_some_and(|d| unsafe { (*d.as_ptr()).flags } & ff::ffi::AV_PIX_FMT_FLAG_ALPHA as u64 != 0);
    Ok(!alpha && (heavy || decoder.width().min(decoder.height()) > SHORT_SIDE))
}

/// Makes the proxy of `source` unless it exists. Concurrent callers wait for one of them. An error from
/// `progress` stops it and leaves nothing behind; the next call starts again.
pub fn ensure_proxy(cache_dir: &Path, source: &Path, mut progress: impl FnMut(f32) -> Result<()>) -> Result<PathBuf> {
    let path = proxy_path(cache_dir, source);
    let _lock = lock_cache(&path, &mut progress)?;
    if !path.exists() {
        let tmp = path.with_extension(format!("mp4.{}.part", crate::edit::new_id()));
        let result = transcode(source, &tmp, &mut progress).and_then(|()| Ok(std::fs::rename(&tmp, &path)?));
        if result.is_err() {
            std::fs::remove_file(&tmp).ok();
        }
        result?;
        remove_others(&path);
    }
    Ok(path)
}

/// Removes the proxies of earlier versions of this file, or of this format, and unfinished ones: nothing
/// reads them any more. Names are `<size>-<time>-<path hash>.<version>.mp4`.
fn remove_others(path: &Path) {
    let source = |name: &str| name.split('.').next().and_then(|key| key.rsplit('-').next()).map(str::to_owned);
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().map(|n| n.to_string_lossy().into_owned())) else {
        return;
    };
    let lock = format!("{name}.lock");
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let other = entry.file_name().to_string_lossy().into_owned();
        if other != name && other != lock && source(&other) == source(&name) {
            std::fs::remove_file(entry.path()).ok();
        }
    }
}

/// Decodes on the GPU where FFmpeg can, a few parts of the video at once, and in software where it cannot or the
/// GPU fails on a frame. Both give the same frames.
fn transcode(source: &Path, out: &Path, progress: &mut impl FnMut(f32) -> Result<()>) -> Result<()> {
    let stopped = Cell::new(false);
    let mut report = |p| progress(p).inspect_err(|_| stopped.set(true));
    if let Ok((decoders, starts)) = on_gpu(source) {
        match encode(decoders, &starts, out, &mut report) {
            Err(e) if !stopped.get() => log::warn!("Decoding {} in software: {e:#}", source.display()),
            done => return done,
        }
    }
    encode(vec![VideoDecoder::open(source)?], &[i64::MIN], out, &mut report)
}

/// FFmpeg hands the GPU's decoder one frame of a video at a time. Two parts at once nearly double what it decodes
/// (4K HEVC on an M4 Pro: 150 frames a second, 280 with two); a third adds little and takes another encoder's
/// memory.
const PARTS_AT_ONCE: usize = 2;
/// Parts start at keyframes about this far apart, so the decoders finish at about the same time.
const PART_US: i64 = 2_000_000;

/// GPU decoders for the parts of the video, and where the parts start.
fn on_gpu(source: &Path) -> Result<(Vec<VideoDecoder>, Vec<i64>)> {
    let first = VideoDecoder::open_hardware(source)?;
    let starts = part_starts(source)?;
    let mut decoders = vec![first];
    for _ in 1..PARTS_AT_ONCE.min(starts.len()) {
        decoders.push(VideoDecoder::open_hardware(source)?);
    }
    Ok((decoders, starts))
}

/// The times the parts of a video start at: the first at its start, the others at a keyframe, so decoding a
/// part decodes no frame of the one before.
fn part_starts(source: &Path) -> Result<Vec<i64>> {
    let mut file = VideoDecoder::open(source)?;
    let duration = file.duration_us();
    let mut starts = vec![i64::MIN];
    for target in (1..).map(|i| i * PART_US).take_while(|t| *t < duration) {
        if let Some(t) = file.keyframe_at(target)
            && t > (*starts.last().unwrap()).max(0)
        {
            starts.push(t);
        }
    }
    Ok(starts)
}

/// Encoded frames of one part, in order.
struct Batch {
    part: usize,
    /// The encoder's, with the first frame of the part.
    parameters: Option<ff::codec::Parameters>,
    packets: Vec<ff::Packet>,
    last: bool,
}

/// What a decoder thread hands its encoder thread.
enum Decoded {
    Frame(usize, i64, frame::Video),
    /// All frames of the part came.
    End(usize),
}

/// Decodes, scales and encodes the parts of the video that start at `starts`, each decoder with its own encoder,
/// and writes their frames in order. A part encoded separately starts with a keyframe and needs the same stream
/// headers.
fn encode(
    decoders: Vec<VideoDecoder>,
    starts: &[i64],
    out: &Path,
    progress: &mut impl FnMut(f32) -> Result<()>,
) -> Result<()> {
    let codec = ff::encoder::find_by_name("libx264").context("Preview proxies need the libx264 encoder")?;
    let duration = decoders[0].duration_us().max(1);
    let frame_us = decoders[0].frame_duration_us;
    let mut octx = ff::format::output_as(out, "mp4").with_context(|| format!("Cannot create {}", out.display()))?;
    let (next, done, stop) = (AtomicUsize::new(0), AtomicI64::new(0), AtomicBool::new(false));
    let (batches, encoded) = mpsc::channel::<Result<Batch>>();
    std::thread::scope(|scope| {
        for mut decoder in decoders {
            // Decoding waits for the decoder, scaling and encoding for the CPU: on threads of their own they overlap.
            let (frames, decoded) = mpsc::sync_channel(2);
            let (next, done, stop) = (&next, &done, &stop);
            scope.spawn(move || {
                if let Err(e) = decode_parts(&mut decoder, starts, duration, next, done, stop, &frames) {
                    frames.send(Err(e)).ok();
                }
            });
            let batches = batches.clone();
            scope.spawn(move || {
                if let Err(e) = encode_parts(decoded, codec, frame_us, &batches) {
                    batches.send(Err(e)).ok();
                }
            });
        }
        drop(batches);
        let result = write_parts(&encoded, &mut octx, codec, starts.len(), frame_us, duration, &done, progress);
        if result.is_err() {
            stop.store(true, Ordering::Relaxed);
        }
        result
    })
}

/// Decodes the next part no other decoder took, until none is left: its frames from its start up to the next
/// part's, the first part's from the start of the file.
fn decode_parts(
    decoder: &mut VideoDecoder,
    starts: &[i64],
    duration: i64,
    next: &AtomicUsize,
    done: &AtomicI64,
    stop: &AtomicBool,
    frames: &mpsc::SyncSender<Result<Decoded>>,
) -> Result<()> {
    loop {
        let part = next.fetch_add(1, Ordering::Relaxed);
        let Some(&start) = starts.get(part) else { return Ok(()) };
        let end = starts.get(part + 1).copied().unwrap_or(i64::MAX);
        if start != i64::MIN {
            decoder.seek(start)?;
        }
        let (mut last, mut reached) = (i64::MIN, start.max(0));
        while let Some((t, f)) = decoder.next_frame()? {
            if stop.load(Ordering::Relaxed) {
                bail!("Stopped");
            }
            if t >= end {
                break;
            }
            // A frame at the time of the one before is never shown, and H.264 needs rising times.
            if t < start || t <= last {
                continue;
            }
            last = t;
            done.fetch_add(t.max(reached) - reached, Ordering::Relaxed);
            reached = t.max(reached);
            if frames.send(Ok(Decoded::Frame(part, t, f))).is_err() {
                // The encoder failed and said why.
                return Ok(());
            }
        }
        done.fetch_add(end.min(duration).max(reached) - reached, Ordering::Relaxed);
        if frames.send(Ok(Decoded::End(part))).is_err() {
            return Ok(());
        }
    }
}

/// Scales and encodes the decoded frames, each part with an encoder of its own.
fn encode_parts(
    decoded: mpsc::Receiver<Result<Decoded>>,
    codec: ff::Codec,
    frame_us: i64,
    batches: &mpsc::Sender<Result<Batch>>,
) -> Result<()> {
    let send = |batch| batches.send(Ok(batch)).map_err(|_| anyhow::anyhow!("Stopped"));
    let mut encoder = None;
    for message in decoded {
        match message? {
            Decoded::Frame(part, t, f) => {
                let mut parameters = None;
                let (enc, scaler, yuv) = match &mut encoder {
                    Some(encoder) => encoder,
                    None => {
                        let opened = open_encoder(codec, &f, frame_us)?;
                        parameters = Some(ff::codec::Parameters::from(&opened.0));
                        encoder.insert(opened)
                    }
                };
                scaler.run(&f, yuv)?;
                yuv.set_pts(Some(t));
                enc.send_frame(yuv)?;
                send(Batch { part, parameters, packets: receive_packets(enc), last: false })?;
            }
            Decoded::End(part) => {
                let mut packets = Vec::new();
                if let Some((mut enc, ..)) = encoder.take() {
                    enc.send_eof()?;
                    packets = receive_packets(&mut enc);
                }
                send(Batch { part, parameters: None, packets, last: true })?;
            }
        }
    }
    Ok(())
}

/// Writes the parts' frames in order as they come, and reports progress while it waits.
#[allow(clippy::too_many_arguments)]
fn write_parts(
    rx: &mpsc::Receiver<Result<Batch>>,
    octx: &mut ff::format::context::Output,
    codec: ff::Codec,
    parts: usize,
    frame_us: i64,
    duration: i64,
    done: &AtomicI64,
    progress: &mut impl FnMut(f32) -> Result<()>,
) -> Result<()> {
    let (mut part, mut waiting) = (0, std::collections::BTreeMap::<usize, Vec<Batch>>::new());
    let mut header: Option<Vec<u8>> = None;
    while part < parts {
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(batch) => {
                let batch = batch?;
                waiting.entry(batch.part).or_default().push(batch);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => bail!("A part of the preview proxy was not made"),
        }
        while let Some(batches) = waiting.remove(&part) {
            let mut finished = false;
            for batch in batches {
                if let Some(parameters) = &batch.parameters {
                    match &header {
                        None => header = Some(start_stream(octx, codec, parameters)?),
                        Some(first) if *first != extradata(parameters) => {
                            bail!("The parts of the preview proxy need different stream headers")
                        }
                        Some(_) => {}
                    }
                }
                let time_base = octx.stream(0).map(|s| s.time_base()).unwrap_or(TIME_BASE.into());
                for mut packet in batch.packets {
                    packet.set_stream(0);
                    // Every frame but the last lasts until the next one; the muxer takes the last one's from here.
                    packet.set_duration(frame_us);
                    packet.rescale_ts(TIME_BASE, time_base);
                    packet.write(octx)?;
                }
                finished |= batch.last;
            }
            if finished {
                part += 1;
            } else {
                // More of this part is coming.
                break;
            }
        }
        progress(done.load(Ordering::Relaxed) as f32 / duration as f32)?;
    }
    if header.is_none() {
        bail!("The video has no frames");
    }
    octx.write_trailer()?;
    Ok(())
}

fn extradata(parameters: &ff::codec::Parameters) -> Vec<u8> {
    unsafe {
        let p = parameters.as_ptr();
        if (*p).extradata.is_null() {
            return Vec::new();
        }
        std::slice::from_raw_parts((*p).extradata, (*p).extradata_size as usize).to_vec()
    }
}

fn start_stream(
    octx: &mut ff::format::context::Output,
    codec: ff::Codec,
    parameters: &ff::codec::Parameters,
) -> Result<Vec<u8>> {
    let mut stream = octx.add_stream(codec)?;
    stream.set_parameters(parameters.clone());
    stream.set_time_base(TIME_BASE);
    // The edit list that starts the picture after the sound is in the movie's units, by default milliseconds.
    let mut header = ff::Dictionary::new();
    header.set("movie_timescale", &TIME_BASE.1.to_string());
    octx.write_header_with(header)?;
    Ok(extradata(parameters))
}

fn receive_packets(enc: &mut ff::encoder::video::Encoder) -> Vec<ff::Packet> {
    let mut packets = Vec::new();
    let mut packet = ff::Packet::empty();
    while enc.receive_packet(&mut packet).is_ok() {
        packets.push(std::mem::replace(&mut packet, ff::Packet::empty()));
    }
    packets
}

/// The encoder, with the colours of the original's first frame, and the scaler to the proxy's size.
fn open_encoder(
    codec: ff::Codec,
    f: &frame::Video,
    frame_us: i64,
) -> Result<(ff::encoder::video::Encoder, scaling::Context, frame::Video)> {
    let scale = (SHORT_SIDE as f64 / f.width().min(f.height()) as f64).min(1.0);
    let even = |v: u32| ((v as f64 * scale / 2.0).round() as u32).max(1) * 2;
    let (w, h) = (even(f.width()), even(f.height()));
    // Named, so the smaller proxy is not shown with the matrix guessed for its own height.
    let (space, full) = (matrix(f), is_full_range(f));
    let mut enc = ff::codec::context::Context::new_with_codec(codec).encoder().video()?;
    enc.set_width(w);
    enc.set_height(h);
    enc.set_format(Pixel::YUV420P);
    enc.set_time_base(TIME_BASE);
    enc.set_frame_rate(Some((1_000_000, frame_us.clamp(1, i32::MAX as i64) as i32)));
    enc.set_gop(GOP);
    enc.set_max_b_frames(0);
    enc.set_colorspace(space);
    enc.set_color_range(if full { color::Range::JPEG } else { color::Range::MPEG });
    unsafe {
        let ctx = enc.as_mut_ptr();
        (*ctx).color_primaries = f.color_primaries().into();
        (*ctx).color_trc = f.color_transfer_characteristic().into();
    }
    // MP4 keeps the stream headers once, not before every keyframe.
    enc.set_flags(ff::codec::Flags::GLOBAL_HEADER);
    let mut options = ff::Dictionary::new();
    options.set("preset", "veryfast");
    options.set("tune", "fastdecode");
    options.set("crf", "20");
    // Two threads, as many as the encoder had when the CPU decoded: with two parts at once on the GPU, a proxy
    // takes about the cores the decoding took before, and leaves the others to the preview.
    options.set("threads", "2");
    let enc = enc.open_with(options).context("Opening the H.264 encoder for the preview proxy failed")?;
    let mut scaler =
        scaling::Context::get(f.format(), f.width(), f.height(), Pixel::YUV420P, w, h, scaling::Flags::BICUBIC)?;
    // Only the size and bit depth change: the same matrix and range on both sides.
    set_sws_colorspace(&mut scaler, sws_colorspace(space), full, sws_colorspace(space), full);
    Ok((enc, scaler, frame::Video::new(Pixel::YUV420P, w, h)))
}

//! Joins the picture and the sound YouTube serves as separate files into one MP4, optionally keeping only a part
//! of it, by copying their packets without re-encoding. MP4 holds every codec YouTube serves: H.264, VP9 and AV1
//! picture, AAC and Opus sound.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use anyhow::{Context as _, Result, bail, ensure};
use ff::util::mathematics::{Rescale, Rounding};
use ffmpeg_next as ff;

use crate::export::{check_cancel, write_beside};
use crate::media::{open_input, origin_us, video_stream};
use crate::model::Part;

/// Footage kept on each side of a chosen part, so its clip's edges can be dragged out a little.
const SPARE_US: i64 = 3_000_000;
/// Sound kept before the cut for the decoder to settle, as Opus needs (RFC 7845); MP4's edit list hides it.
const PREROLL_US: i64 = 80_000;
const MICROS: ff::Rational = ff::Rational(1, 1_000_000);

/// Writes the picture of `video` and the sound of `audio`, or of `video` itself when `audio` is None, to `out`.
/// With a `part`, keeps only it and up to [`SPARE_US`] around it, starting on a keyframe, and returns the part in
/// the new file's time. Refuses an existing `out`; a failure or `cancel` leaves nothing behind.
pub fn join(
    video: &Path,
    audio: Option<&Path>,
    out: &Path,
    part: Option<Part>,
    cancel: &AtomicBool,
) -> Result<Option<Part>> {
    let mut inputs = vec![open_input(video)?];
    if let Some(audio) = audio {
        inputs.push(open_input(audio)?);
    }
    let v = video_stream(&inputs[0]).with_context(|| format!("{} has no video", video.display()))?.index();
    // Output stream `o` copies stream `map[o].1` of input `map[o].0`; the picture is output stream 0.
    let mut map = vec![(0, v)];
    let sound = inputs.len() - 1;
    match inputs[sound].streams().best(ff::media::Type::Audio) {
        Some(stream) => map.push((sound, stream.index())),
        None if audio.is_some() => bail!("{} has no sound", audio.unwrap().display()),
        None => {}
    }
    let time_bases: Vec<_> = map.iter().map(|&(i, s)| inputs[i].stream(s).unwrap().time_base()).collect();

    // In each output stream's units: the cut keyframe's time, where the copy starts and where it ends.
    let mut bounds = None;
    if let Some(part) = part {
        ensure!(0 <= part.start_us && part.start_us < part.end_us, "The chosen part is empty");
        let origin = origin_us(&inputs[0]);
        let key = keyframe_before(&mut inputs[0], v, origin + (part.start_us - SPARE_US).max(0), cancel)?;
        let key_us = key.rescale(time_bases[0], MICROS);
        let end_us = part.end_us + SPARE_US;
        let end_us = (end_us < inputs[0].duration()).then_some(origin + end_us);
        for input in &mut inputs {
            seek(input, key_us - 1_000_000)?;
        }
        let at = |o: usize, tb| {
            let cut = key.rescale(time_bases[0], tb);
            let from = if o == 0 { cut } else { (key_us - PREROLL_US).rescale(MICROS, tb) };
            (cut, from, end_us.map_or(i64::MAX, |end| end.rescale(MICROS, tb)))
        };
        bounds = Some((time_bases.iter().enumerate().map(|(o, &tb)| at(o, tb)).collect::<Vec<_>>(), key_us - origin));
    }

    let mut kept = None;
    write_beside(out, false, cancel, |tmp| {
        let mut octx = ff::format::output_as(tmp, "mp4").with_context(|| format!("Cannot create {}", tmp.display()))?;
        for (&(i, s), &tb) in map.iter().zip(&time_bases) {
            let mut stream = octx.add_stream(ff::encoder::find(ff::codec::Id::None))?;
            stream.set_parameters(inputs[i].stream(s).unwrap().parameters());
            // The tag of the source container, such as WebM's, can be foreign to MP4; the muxer picks its own.
            unsafe { (*stream.parameters().as_mut_ptr()).codec_tag = 0 };
            stream.set_time_base(tb);
        }
        octx.write_header().context("MP4 cannot hold this video's codecs")?;
        let out_bases: Vec<_> = (0..map.len()).map(|o| octx.stream(o).unwrap().time_base()).collect();

        let mut packets: Vec<_> = inputs.iter_mut().map(|input| input.packets()).collect();
        let mut ahead: Vec<Option<(usize, ff::Packet)>> = packets.iter().map(|_| None).collect();
        let mut ended = vec![false; packets.len()];
        let mut done = vec![false; map.len()];
        let mut started = bounds.is_none();
        loop {
            check_cancel(cancel)?;
            for i in 0..packets.len() {
                while ahead[i].is_none() && !ended[i] {
                    let Some((stream, mut packet)) = packets[i].next() else {
                        ended[i] = true;
                        break;
                    };
                    let Some(o) = map.iter().position(|&m| m == (i, stream.index())) else { continue };
                    if let Some((at, _)) = &bounds {
                        let (cut, from, end) = at[o];
                        let Some(pts) = packet.pts() else { continue };
                        let dts = packet.dts().unwrap_or(pts);
                        if o == 0 && !started {
                            if !packet.is_key() || pts != cut {
                                continue;
                            }
                            started = true;
                        }
                        // Leading pictures of an open GOP, which need the one before, and sound before the preroll.
                        if pts < from {
                            continue;
                        }
                        // Every picture shown before the end decodes before it, as do the pictures it refers to.
                        if dts >= end {
                            done[o] = true;
                            ended[i] = map.iter().zip(&done).all(|(&(j, _), &done)| j != i || done);
                            continue;
                        }
                        packet.set_pts(Some(pts - cut));
                        packet.set_dts(Some(dts - cut));
                    }
                    ahead[i] = Some((o, packet));
                }
            }
            let next = |i: usize| {
                ahead[i]
                    .as_ref()
                    .map(|(o, p)| p.dts().or(p.pts()).map_or(i64::MIN, |t| t.rescale(time_bases[*o], MICROS)))
            };
            let Some(i) = (0..ahead.len()).filter(|&i| ahead[i].is_some()).min_by_key(|&i| next(i)) else { break };
            let (o, mut packet) = ahead[i].take().unwrap();
            packet.rescale_ts(time_bases[o], out_bases[o]);
            packet.set_position(-1);
            packet.set_stream(o);
            packet.write_interleaved(&mut octx)?;
        }
        ensure!(started, "{} has no keyframe at the chosen part", video.display());
        octx.write_trailer()?;
        drop(octx);

        if let (Some(part), Some((_, cut_us))) = (part, &bounds) {
            let written = open_input(tmp)?;
            // How much earlier the part is in the new file than in the source.
            let shift = cut_us + origin_us(&written);
            let end = (part.end_us - shift).min(written.duration());
            ensure!(end > 0, "The chosen part is past the end of {}", video.display());
            kept = Some(Part { start_us: (part.start_us - shift).clamp(0, end - 1), end_us: end });
        }
        Ok(())
    })?;
    Ok(kept)
}

/// The time of the last keyframe at or before `t_us`, in the stream's units, or of the first one when the picture
/// starts after it.
fn keyframe_before(
    input: &mut ff::format::context::Input,
    index: usize,
    t_us: i64,
    cancel: &AtomicBool,
) -> Result<i64> {
    let t = t_us.rescale_with(MICROS, input.stream(index).unwrap().time_base(), Rounding::Down);
    // A seek can land after the keyframe; then look again from the start.
    for from in [t_us, i64::MIN] {
        seek(input, from)?;
        let (mut found, mut first) = (None, None);
        for (stream, packet) in input.packets() {
            check_cancel(cancel)?;
            let Some(pts) = packet.pts().filter(|_| stream.index() == index) else { continue };
            if first.is_some() && packet.dts().unwrap_or(pts) > t {
                break;
            }
            if packet.is_key() {
                first.get_or_insert(pts);
                found = Some(pts).filter(|&pts| pts <= t).or(found);
            }
        }
        if found.is_some() || from == i64::MIN {
            return found.or(first).context("The video has no keyframe");
        }
    }
    unreachable!()
}

/// Moves to the keyframe at or before `t_us` in container time, or to the start.
fn seek(input: &mut ff::format::context::Input, t_us: i64) -> Result<()> {
    let start = origin_us(input).saturating_sub(1_000_000);
    let ptr = unsafe { input.as_mut_ptr() };
    let at = |ts, max| unsafe { ff::ffi::avformat_seek_file(ptr, -1, i64::MIN, ts, max, 0) } >= 0;
    if (t_us > start && at(t_us, t_us)) || at(start, i64::MAX) { Ok(()) } else { bail!("Cannot seek in the video") }
}

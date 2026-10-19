//! MP4 export: the preview renderer at canvas resolution, H.264 + AAC. A delivery preset fixes
//! the format a platform expects and levels the sound to its loudness.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, anyhow, bail};
use ff::software::scaling;
use ff::util::{color, format::Pixel, frame};
use ffmpeg_next as ff;

use crate::audio::{Chain, Mixer, chains, has_audio};
use crate::loudness::{Limiter, Meter, db_to_gain};
use crate::media::{Scaler, init};
use crate::model::{CHANNELS, Clip, ClipContent, Project, SAMPLE_RATE, TrackKind};
use crate::render::{Pending, Renderer, Wait};

#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    High,
    Recommended,
    Small,
}

impl Quality {
    pub fn crf(self) -> u8 {
        match self {
            Self::High => 17,
            Self::Recommended => 21,
            Self::Small => 26,
        }
    }
}

/// A platform's delivery format, shared by the export dialog, MCP and the CLI. Every preset writes
/// H.264 High and AAC 48 kHz stereo with the sound levelled to its loudness.
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// Instagram Reels and TikTok: 1080x1920 at 30 fps.
    Reels,
    /// YouTube Shorts: 1080x1920 at the project frame rate.
    Shorts,
    /// YouTube: 1920x1080 at the project frame rate.
    #[serde(rename = "youtube_1080p")]
    Youtube1080p,
    /// YouTube: 3840x2160 at the project frame rate.
    #[serde(rename = "youtube_4k")]
    Youtube4k,
    /// Instagram feed: 1080x1350 at 30 fps.
    InstagramFeed,
    /// A square feed post: 1080x1080 at 30 fps.
    Square,
}

impl Delivery {
    pub const ALL: [Self; 6] =
        [Self::Reels, Self::Shorts, Self::Youtube1080p, Self::Youtube4k, Self::InstagramFeed, Self::Square];

    pub fn label(self) -> &'static str {
        match self {
            Self::Reels => "Reels & TikTok",
            Self::Shorts => "YouTube Shorts",
            Self::Youtube1080p => "YouTube 1080p",
            Self::Youtube4k => "YouTube 4K",
            Self::InstagramFeed => "Instagram feed",
            Self::Square => "Square",
        }
    }

    /// Short side in pixels.
    pub fn resolution(self) -> u32 {
        match self {
            Self::Youtube4k => 2160,
            _ => 1080,
        }
    }

    /// `None` keeps the project frame rate, or the one asked for, up to `MAX_DELIVERY_FPS`:
    /// YouTube wants a video at the rate it was shot. Instagram plays everything at 30.
    pub fn fps(self) -> Option<u32> {
        match self {
            Self::Reels | Self::InstagramFeed | Self::Square => Some(30),
            Self::Shorts | Self::Youtube1080p | Self::Youtube4k => None,
        }
    }

    /// The canvas width to height ratio the preset is made for.
    pub fn aspect(self) -> (u32, u32) {
        match self {
            Self::Reels | Self::Shorts => (9, 16),
            Self::Youtube1080p | Self::Youtube4k => (16, 9),
            Self::InstagramFeed => (4, 5),
            Self::Square => (1, 1),
        }
    }

    /// Integrated loudness of the file in LUFS, where YouTube, Instagram and TikTok all normalise
    /// playback. Its true peak stays under -1 dBTP, which `LIMITER_CEILING_DB` keeps for every preset.
    pub fn loudness(self) -> f64 {
        -14.0
    }
}

/// The most frames per second a delivery preset writes; YouTube takes no more.
pub const MAX_DELIVERY_FPS: u32 = 60;

#[derive(Clone, Debug)]
pub struct ExportOptions {
    pub crf: u8,
    pub replace_existing: bool,
    /// The credits file of CC BY sounds may replace one beside the video. Asked apart from the video,
    /// since a creator may keep their own notes in a file of that name.
    pub replace_credits: bool,
    /// x264 speed preset.
    pub preset: String,
    /// Short side of the output in pixels (720, 1080, 1440, 2160); `None` keeps the canvas size.
    pub resolution: Option<u32>,
    /// Output frame rate; `None` uses the project frame rate.
    pub fps: Option<u32>,
    /// Delivery preset; `resolution` and `fps` must then be `None` or the preset's own values.
    pub delivery: Option<Delivery>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            crf: Quality::Recommended.crf(),
            replace_existing: false,
            replace_credits: false,
            preset: "veryfast".into(),
            resolution: None,
            fps: None,
            delivery: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportPhase {
    /// The audio-only passes of a delivery preset that measure and level the sound.
    Loudness,
    Rendering,
}

impl ExportPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Loudness => "Measuring loudness",
            Self::Rendering => "Rendering",
        }
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub phase: ExportPhase,
    /// Frames rendered so far; 0 while the loudness is measured.
    pub frame: u64,
    pub total_frames: u64,
    /// The whole export from 0 to 1, loudness passes included.
    pub fraction: f32,
}

/// Renders the whole timeline to `out`. Returns early with an error if `cancel` is set.
pub fn export(
    project: &Project,
    cache_dir: &Path,
    out: &Path,
    options: &ExportOptions,
    cancel: &AtomicBool,
    mut progress: impl FnMut(ExportProgress),
) -> Result<()> {
    init();
    let duration = project.duration_us();
    if duration <= 0 {
        bail!("The timeline is empty");
    }
    check_options(project, options)?;
    check_source_path(project, out)?;
    // CC BY sounds go out with their credits, beside the video and under the same consent.
    let credits = crate::credits::credits(project);
    let credits_out = crate::credits::credits_path(out);
    if credits.is_some() && !options.replace_credits && credits_out.symlink_metadata().is_ok() {
        bail!("OUTPUT_EXISTS: {} already exists; choose a new export path", credits_out.display());
    }
    prepare_sound(project, cache_dir, cancel)?;
    // A clip with a background needs its person outlines made first (`nuzky_vision::matte::prepare`).
    crate::matte::check(cache_dir, project)?;
    write_beside(out, options.replace_existing, cancel, |tmp| {
        encode(project, cache_dir, tmp, options, cancel, &mut progress, duration)
    })?;
    match credits {
        Some(text) => write_credits(&text, &credits_out, options.replace_credits).with_context(|| {
            format!("The video is saved, but its credits could not be written to {}", credits_out.display())
        }),
        None => Ok(()),
    }
}

/// Prepares the sound of every file heard in `project` as its clips play it: decoded into the cache, cleaned
/// and without reverb where a clip asks for it, and measured for Match loudness and Even out volume.
pub(crate) fn prepare_sound(project: &Project, cache_dir: &Path, cancel: &AtomicBool) -> Result<()> {
    let mut heard = std::collections::HashMap::<&str, (std::collections::HashSet<Chain>, bool)>::new();
    for clip in heard_clips(project) {
        if let ClipContent::Media { asset_id, .. } = &clip.content {
            let (chains, levels) = chains(clip);
            let entry = heard.entry(asset_id.as_str()).or_default();
            entry.0.extend(chains);
            entry.1 |= levels;
        }
    }
    for asset in project.assets.iter().filter(|a| has_audio(a)) {
        let Some((chains, levels)) = heard.get(asset.id.as_str()) else { continue };
        for chain in chains {
            let path = chain.ensure(cache_dir, asset, |_| check_cancel(cancel))?;
            if *levels {
                crate::level::ensure_blocks(&path, |_| check_cancel(cancel))?;
            }
        }
    }
    Ok(())
}

/// Writes `out` through a temporary `.nuzky-part-*` file beside it, so rename stays on the same filesystem and
/// nothing half written ever has its name.
pub(crate) fn write_beside(
    out: &Path,
    replace_existing: bool,
    cancel: &AtomicBool,
    write: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let tmp = loop {
        let path = out.with_file_name(format!(".nuzky-part-{}.mp4", uuid::Uuid::new_v4()));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => break path,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                use std::io::ErrorKind::{PermissionDenied, ReadOnlyFilesystem};
                let why = if matches!(e.kind(), PermissionDenied | ReadOnlyFilesystem) {
                    "NOT_WRITABLE: cannot write to the export folder"
                } else {
                    "Cannot reserve temporary export file"
                };
                return Err(e).context(why);
            }
        }
    };
    let result = write(&tmp).and_then(|()| publish(&tmp, out, replace_existing, cancel));
    if result.is_err() {
        // Report why the export failed, not a secondary cleanup problem.
        if let Err(e) = std::fs::remove_file(&tmp) {
            log::warn!("Cannot remove temporary export {}: {e}", tmp.display());
        }
    }
    result
}

/// Media clips whose sound is in the export: on a heard track, above zero volume, inside the timeline.
pub fn heard_clips(project: &Project) -> impl Iterator<Item = &Clip> {
    let duration = project.duration_us();
    project.tracks.iter().filter(|track| !track.muted && track.kind != TrackKind::Text).flat_map(|t| &t.clips).filter(
        move |clip| {
            matches!(&clip.content, ClipContent::Media { volume, .. } if *volume > 0.0)
                && clip.end_us() > 0
                && clip.start_us < duration
        },
    )
}

/// Through a temporary file beside it, like the video.
pub(crate) fn write_credits(text: &str, out: &Path, replace_existing: bool) -> Result<()> {
    let tmp = out.with_file_name(format!(".nuzky-part-{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, text).with_context(|| format!("Cannot write {}", tmp.display()))?;
    let result = publish(&tmp, out, replace_existing, &AtomicBool::new(false));
    if result.is_err() {
        std::fs::remove_file(&tmp).ok();
    }
    result
}

pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("CANCELLED: export cancelled");
    }
    Ok(())
}

/// Refuses a delivery preset the project cannot meet, before any work starts.
pub fn check_options(project: &Project, options: &ExportOptions) -> Result<()> {
    let Some(delivery) = options.delivery else { return Ok(()) };
    let (w, h) = (project.canvas.width, project.canvas.height);
    let (aw, ah) = delivery.aspect();
    if w as u64 * ah as u64 != h as u64 * aw as u64 {
        bail!(
            "{} needs a {aw}:{ah} video and this one is {}. Switch the format to {aw}:{ah}, then export again.",
            delivery.label(),
            ratio(w, h)
        );
    }
    let fixed = [("resolution", options.resolution, Some(delivery.resolution())), ("fps", options.fps, delivery.fps())];
    for (name, given, wanted) in fixed {
        if let (Some(given), Some(wanted)) = (given, wanted)
            && given != wanted
        {
            bail!("{} exports at {name} {wanted}; leave {name} out or set it to {wanted}", delivery.label());
        }
    }
    let fps = export_fps(project, options);
    if fps > MAX_DELIVERY_FPS {
        bail!(
            "{} takes at most {MAX_DELIVERY_FPS} fps and this export is {fps} fps. Set fps to {MAX_DELIVERY_FPS} or less.",
            delivery.label()
        );
    }
    Ok(())
}

fn export_fps(project: &Project, options: &ExportOptions) -> u32 {
    options.fps.or(options.delivery.and_then(Delivery::fps)).unwrap_or(project.canvas.fps)
}

fn ratio(w: u32, h: u32) -> String {
    let (mut a, mut b) = (w, h);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let d = a.max(1);
    format!("{}:{}", w / d, h / d)
}

pub fn check_source_path(project: &Project, path: &Path) -> Result<()> {
    if let Ok(target) = std::fs::canonicalize(path)
        && project.assets.iter().any(|a| std::fs::canonicalize(&a.path).ok().as_ref() == Some(&target))
    {
        bail!("{} is used in this project. Choose another file name.", path.display());
    }
    Ok(())
}

pub(crate) fn publish(tmp: &Path, out: &Path, replace_existing: bool, cancel: &AtomicBool) -> Result<()> {
    publish_with(tmp, out, replace_existing, cancel, |from, to| std::fs::hard_link(from, to))
}

/// `link` is `hard_link`, replaceable in tests by a filesystem without hard links.
fn publish_with(
    tmp: &Path,
    out: &Path,
    replace_existing: bool,
    cancel: &AtomicBool,
    link: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("CANCELLED: export cancelled");
    }
    let cannot_write = |error: std::io::Error| anyhow!(error).context(format!("Cannot write {}", out.display()));
    if replace_existing {
        return std::fs::rename(tmp, out).map_err(cannot_write);
    }
    let exists = || anyhow!("OUTPUT_EXISTS: choose a new export path");
    // Linking on the same filesystem atomically refuses an existing destination.
    match link(tmp, out) {
        Ok(()) => std::fs::remove_file(tmp).context("Removing published export temporary file")?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Err(exists()),
        // FAT32 and exFAT, e.g. USB sticks, have no hard links. Claiming the name first still
        // never replaces a file that was there; the rename then swaps in the export.
        Err(error) => {
            log::debug!("Cannot hard link {} ({error}); publishing by rename", out.display());
            match std::fs::OpenOptions::new().write(true).create_new(true).open(out) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Err(exists()),
                Err(error) => return Err(cannot_write(error)),
            }
            if let Err(error) = std::fs::rename(tmp, out) {
                std::fs::remove_file(out).ok();
                return Err(cannot_write(error));
            }
        }
    }
    Ok(())
}

fn output_size(project: &Project, options: &ExportOptions, max_dimension: u32) -> Result<(u32, u32)> {
    let (w, h) = (project.canvas.width, project.canvas.height);
    if w < 2 || h < 2 {
        bail!("Canvas dimensions must be at least 2 pixels");
    }
    let short = options.resolution.or(options.delivery.map(Delivery::resolution)).unwrap_or(w.min(h));
    if !(2..=7680).contains(&short) {
        bail!("Export resolution must be between 2 and 7680");
    }
    let scale = short as f64 / w.min(h) as f64;
    let even = |v: u32| (v as f64 * scale / 2.0).round().max(1.0) * 2.0;
    let (w, h) = (even(w), even(h));
    if w > max_dimension as f64 || h > max_dimension as f64 {
        bail!("This resolution is larger than your GPU supports (max {max_dimension} px)");
    }
    Ok((w as u32, h as u32))
}

/// Start of frame `index`, rounded to the nearest microsecond like cut points made at the
/// playhead, so a clip starting on a frame is the one shown on that frame. Computed from the
/// integer index, never by accumulating rounded durations.
pub fn frame_time_us(index: u64, fps: u32) -> i64 {
    ((2 * index as i128 * 1_000_000 + fps as i128) / (2 * fps as i128)) as i64
}

/// Number of frames that start before `duration_us`.
pub(crate) fn frame_count(duration_us: i64, fps: u32) -> u64 {
    if duration_us <= 0 {
        return 0;
    }
    // frame_time_us(i) < d exactly when i < (2d - 1) * fps / 2_000_000.
    (((2 * duration_us as i128 - 1) * fps as i128 + 1_999_999) / 2_000_000) as u64
}

fn encode(
    project: &Project,
    cache_dir: &Path,
    out: &Path,
    options: &ExportOptions,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(ExportProgress),
    duration: i64,
) -> Result<()> {
    let delivery = options.delivery;
    let fps = export_fps(project, options);
    if fps == 0 || fps > 240 {
        bail!("Export frame rate must be between 1 and 240");
    }
    let mut renderer = Renderer::new()?;
    renderer.use_mattes(cache_dir.to_path_buf(), Pending::Fail);
    let (w, h) = output_size(project, options, renderer.max_texture_dimension())?;
    let total_frames = frame_count(duration, fps);
    let total_samples = crate::audio::us_to_samples(duration);

    let vcodec = match delivery {
        // Another H.264 encoder would not promise the profile and settings the platforms expect.
        Some(delivery) => ff::encoder::find_by_name("libx264").ok_or_else(|| {
            anyhow!("{} needs the libx264 encoder, which this FFmpeg build does not have", delivery.label())
        })?,
        None => ff::encoder::find_by_name("libx264")
            .or_else(|| ff::encoder::find(ff::codec::Id::H264))
            .ok_or_else(|| anyhow!("No H.264 encoder available in this FFmpeg build"))?,
    };
    let render_start = if delivery.is_some() { LOUDNESS_SHARE } else { 0.0 };
    let mut sound = match delivery {
        Some(delivery) => {
            let report = |fraction: f32| {
                progress(ExportProgress {
                    phase: ExportPhase::Loudness,
                    frame: 0,
                    total_frames,
                    fraction: fraction * LOUDNESS_SHARE,
                })
            };
            let gain = plan_gain(project, cache_dir, total_samples, delivery.loudness(), cancel, report)?;
            Sound::Leveled(Box::new(Leveled::new(cache_dir, project, gain)))
        }
        None => Sound::Mix(Box::new(Mixer::new(cache_dir.to_path_buf()))),
    };
    // A delivery preset keeps the app's speed: veryfast writes High, which the profile pins.
    let x264 = match delivery {
        Some(_) => [("preset", "veryfast"), ("profile", "high")],
        None => [("preset", options.preset.as_str()), ("", "")],
    };
    let mut mp4 = Mp4Writer::create(out, vcodec, w, h, fps, options.crf, &x264)?;
    let render = |i| {
        let pixels = renderer.render(project, frame_time_us(i, fps), w, h, Wait::Exact, true)?;
        progress(ExportProgress {
            phase: ExportPhase::Rendering,
            frame: i + 1,
            total_frames,
            fraction: render_start + (1.0 - render_start) * (i + 1) as f32 / total_frames.max(1) as f32,
        });
        Ok(pixels)
    };
    let write = |i, pixels: Vec<u8>| {
        mp4.frame(i, &pixels)?;
        let audio_until = crate::audio::us_to_samples(frame_time_us(i + 1, fps)).min(total_samples);
        mp4.sound_until(audio_until, |pos, mix| sound.fill(project, pos, mix))
    };
    overlap(total_frames, cancel, render, write)?;
    mp4.finish()
}

/// Makes items `0..count` in order and hands each to `write` on another thread, so making the next one overlaps
/// writing the one before. The hand-off holds no item, so at most two exist at once. Both sides stop at `cancel`,
/// the writer before an item it has not started; the first error of either side ends both and is returned.
fn overlap<T: Send>(
    count: u64,
    cancel: &AtomicBool,
    mut make: impl FnMut(u64) -> Result<T>,
    mut write: impl FnMut(u64, T) -> Result<()> + Send,
) -> Result<()> {
    let (hand, take) = std::sync::mpsc::sync_channel(0);
    let writer_failed = &AtomicBool::new(false);
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || {
            let written = (0..).zip(take).try_for_each(|(i, item)| {
                check_cancel(cancel)?;
                write(i, item)
            });
            writer_failed.store(written.is_err(), Ordering::SeqCst);
            written
        });
        let mut made_first = false;
        let made = (0..count).try_for_each(|i| {
            check_cancel(cancel)?;
            let item = make(i).inspect_err(|_| made_first = !writer_failed.load(Ordering::SeqCst))?;
            // Refused only once the writer has stopped on an error, which is the one returned.
            hand.send(item).map_err(|_| anyhow!("The export writer stopped"))
        });
        drop(hand);
        let written: Result<()> = writer.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        if made_first { made.and(written) } else { written.and(made) }
    })
}

/// An MP4 of H.264 video and AAC sound at 48 kHz stereo, written a frame at a time.
pub(crate) struct Mp4Writer {
    octx: ff::format::context::Output,
    venc: ff::encoder::Video,
    aenc: ff::encoder::Audio,
    vindex: usize,
    aindex: usize,
    fps: u32,
    w: u32,
    h: u32,
    to_yuv: Scaler,
    rgba: frame::Video,
    yuv: frame::Video,
    aframe_size: usize,
    /// The next audio sample to write.
    audio_pos: i64,
    mix: Vec<f32>,
}

impl Mp4Writer {
    /// `x264` are encoder options; an empty name is skipped.
    pub(crate) fn create(
        out: &Path,
        vcodec: ff::Codec,
        w: u32,
        h: u32,
        fps: u32,
        crf: u8,
        x264: &[(&str, &str)],
    ) -> Result<Self> {
        let mut octx = ff::format::output(out).with_context(|| format!("Cannot create {}", out.display()))?;
        let global_header = octx.format().flags().contains(ff::format::Flags::GLOBAL_HEADER);
        let mut venc = ff::codec::context::Context::new_with_codec(vcodec).encoder().video()?;
        venc.set_width(w);
        venc.set_height(h);
        venc.set_format(Pixel::YUV420P);
        venc.set_time_base((1, fps as i32));
        venc.set_frame_rate(Some((fps as i32, 1)));
        venc.set_colorspace(color::Space::BT709);
        venc.set_color_range(color::Range::MPEG);
        unsafe {
            let ctx = venc.as_mut_ptr();
            (*ctx).color_primaries = ff::ffi::AVColorPrimaries::AVCOL_PRI_BT709;
            (*ctx).color_trc = ff::ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
        }
        if global_header {
            venc.set_flags(ff::codec::Flags::GLOBAL_HEADER);
        }
        let mut vopts = ff::Dictionary::new();
        for (name, value) in x264.iter().filter(|(name, _)| !name.is_empty()) {
            vopts.set(name, value);
        }
        vopts.set("crf", &crf.to_string());
        let venc = venc.open_with(vopts).context("Opening the H.264 encoder failed")?;
        let vindex = {
            let mut st = octx.add_stream(vcodec)?;
            st.set_parameters(&venc);
            st.set_time_base((1, fps as i32));
            st.index()
        };

        // The native AAC encoder; libfdk_aac is not GPL compatible.
        let acodec = ff::encoder::find_by_name("aac").ok_or_else(|| anyhow!("No AAC encoder available"))?;
        let mut aenc = ff::codec::context::Context::new_with_codec(acodec).encoder().audio()?;
        aenc.set_rate(SAMPLE_RATE as i32);
        aenc.set_channel_layout(ff::ChannelLayout::STEREO);
        aenc.set_format(ff::format::Sample::F32(ff::format::sample::Type::Planar));
        aenc.set_bit_rate(192_000);
        aenc.set_time_base((1, SAMPLE_RATE as i32));
        if global_header {
            aenc.set_flags(ff::codec::Flags::GLOBAL_HEADER);
        }
        let aenc = aenc.open_with(ff::Dictionary::new()).context("Opening the AAC encoder failed")?;
        let aframe_size = (aenc.frame_size() as usize).max(1024);
        let aindex = {
            let mut st = octx.add_stream(acodec)?;
            st.set_parameters(&aenc);
            st.set_time_base((1, SAMPLE_RATE as i32));
            st.index()
        };

        let mut header_opts = ff::Dictionary::new();
        header_opts.set("movflags", "+faststart");
        octx.write_header_with(header_opts)?;

        Ok(Self {
            octx,
            venc,
            aenc,
            vindex,
            aindex,
            fps,
            w,
            h,
            to_yuv: to_yuv(w, h)?,
            rgba: frame::Video::new(Pixel::RGBA, w, h),
            yuv: frame::Video::new(Pixel::YUV420P, w, h),
            aframe_size,
            audio_pos: 0,
            mix: vec![0f32; aframe_size * CHANNELS],
        })
    }

    /// Frame `index`: straight RGBA, `w` × `h`, rows without padding.
    pub(crate) fn frame(&mut self, index: u64, pixels: &[u8]) -> Result<()> {
        let stride = self.rgba.stride(0);
        let row = self.w as usize * 4;
        let dst = self.rgba.data_mut(0);
        for y in 0..self.h as usize {
            dst[y * stride..y * stride + row].copy_from_slice(&pixels[y * row..(y + 1) * row]);
        }
        self.to_yuv.run(&self.rgba, &mut self.yuv).context("Converting the frame to YUV failed")?;
        self.yuv.set_pts(Some(index as i64));
        self.venc.send_frame(&self.yuv)?;
        self.drain_video()
    }

    /// Writes sound up to sample `until` in whole AAC frames; `fill` mixes the frame starting at a sample, each
    /// call continuing where the last ended.
    pub(crate) fn sound_until(&mut self, until: i64, mut fill: impl FnMut(i64, &mut [f32])) -> Result<()> {
        while self.audio_pos < until {
            fill(self.audio_pos, &mut self.mix);
            encode_audio(&mut self.aenc, self.audio_pos, &self.mix, self.aframe_size)?;
            self.audio_pos += self.aframe_size as i64;
            self.drain_audio()?;
        }
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<()> {
        self.venc.send_eof()?;
        self.drain_video()?;
        self.aenc.send_eof()?;
        self.drain_audio()?;
        self.octx.write_trailer()?;
        Ok(())
    }

    fn drain_video(&mut self) -> Result<()> {
        let to = self.octx.stream(self.vindex).unwrap().time_base();
        let mut packet = ff::Packet::empty();
        while self.venc.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.vindex);
            packet.set_duration(1);
            packet.rescale_ts((1, self.fps as i32), to);
            packet.write_interleaved(&mut self.octx)?;
        }
        Ok(())
    }

    fn drain_audio(&mut self) -> Result<()> {
        let to = self.octx.stream(self.aindex).unwrap().time_base();
        let mut packet = ff::Packet::empty();
        while self.aenc.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.aindex);
            packet.rescale_ts((1, SAMPLE_RATE as i32), to);
            packet.write_interleaved(&mut self.octx)?;
        }
        Ok(())
    }
}

fn to_yuv(w: u32, h: u32) -> Result<Scaler> {
    let colorspace = [(ff::ffi::SWS_CS_DEFAULT, true), (ff::ffi::SWS_CS_ITU709, false)];
    Scaler::new((Pixel::RGBA, w, h), (Pixel::YUV420P, w, h), scaling::Flags::BICUBIC, colorspace)
}

/// Ceiling in dBTP the limiter holds the levelled mix under. The AAC encoder adds peaks of its
/// own: decoded files measured 0.2 to 0.4 dB over the ceiling for speech, 0.8 dB for music and
/// up to 1.6 dB for limited bursts of broadband noise such as claps (qa_export
/// `reels_true_peak_survives_aac_on_hard_material`), so -3 keeps all of them under -1 dBTP.
pub(crate) const LIMITER_CEILING_DB: f64 = -3.0;
/// The most a quiet timeline is raised, so near silence never turns into loud noise.
const MAX_GAIN_DB: f64 = 24.0;
/// Close enough to the target to stop measuring; AAC moves the level by about as much.
const TOLERANCE_LU: f64 = 0.1;
/// Levelled passes at most after the first measurement. Speech lands in one or two; sound
/// whose loudness sits in the peaks the limiter holds takes more.
const LEVELLING_PASSES: usize = 6;
/// Part of the progress bar the loudness passes take before rendering starts: they took 1 % of
/// a 47 s Reels export in a release build and 4 % in a debug one.
const LOUDNESS_SHARE: f32 = 0.05;
/// Audio between cancellation checks in the loudness passes, in frames (one second).
const MEASURE_CHUNK: usize = SAMPLE_RATE as usize;

/// The sound that goes into the file: the project mix as it plays, or for a delivery preset that
/// mix at the planned gain through the true-peak limiter.
enum Sound {
    Mix(Box<Mixer>),
    Leveled(Box<Leveled>),
}

impl Sound {
    /// `pos` must continue where the previous call ended.
    fn fill(&mut self, project: &Project, pos: i64, out: &mut [f32]) {
        match self {
            Self::Mix(mixer) => mixer.mix(project, pos, out),
            Self::Leveled(leveled) => {
                debug_assert_eq!(pos, leveled.read - Limiter::latency() as i64);
                leveled.fill(project, out)
            }
        }
    }
}

/// The timeline mix at a fixed gain through the true-peak limiter, frame for frame in step with
/// the timeline: the limiter's look-ahead is read before the first frame comes out.
struct Leveled {
    mixer: Mixer,
    gain: f32,
    limiter: Limiter,
    /// Timeline frame the mixer reads next.
    read: i64,
    input: Vec<f32>,
}

impl Leveled {
    fn new(cache_dir: &Path, project: &Project, gain_db: f64) -> Self {
        let mut leveled = Self {
            mixer: Mixer::new(cache_dir.to_path_buf()),
            gain: db_to_gain(gain_db),
            limiter: Limiter::new(db_to_gain(LIMITER_CEILING_DB)),
            read: 0,
            input: vec![0.0; Limiter::latency() * CHANNELS],
        };
        leveled.mixer.mix(project, 0, &mut leveled.input);
        leveled.read = Limiter::latency() as i64;
        for frame in leveled.input.as_chunks::<CHANNELS>().0 {
            let early = leveled.limiter.push(std::array::from_fn(|c| frame[c] * leveled.gain));
            debug_assert!(early.is_none());
        }
        leveled
    }

    /// The next `out.len() / CHANNELS` frames of the timeline.
    fn fill(&mut self, project: &Project, out: &mut [f32]) {
        self.input.resize(out.len(), 0.0);
        self.mixer.mix(project, self.read, &mut self.input);
        self.read += (out.len() / CHANNELS) as i64;
        for (out, frame) in out.as_chunks_mut::<CHANNELS>().0.iter_mut().zip(self.input.as_chunks::<CHANNELS>().0) {
            let gain = self.gain;
            let leveled = self.limiter.push(std::array::from_fn(|c| frame[c] * gain));
            out.copy_from_slice(&leveled.expect("the look-ahead is read first"));
        }
    }
}

/// Loudness of the first `total` timeline frames, read in one-second chunks from `fill`.
fn measure(
    total: i64,
    cancel: &AtomicBool,
    mut fill: impl FnMut(i64, &mut [f32]),
    mut report: impl FnMut(f32),
) -> Result<crate::loudness::Loudness> {
    let mut meter = Meter::new();
    let mut chunk = vec![0f32; MEASURE_CHUNK * CHANNELS];
    let mut pos = 0;
    while pos < total {
        check_cancel(cancel)?;
        let frames = MEASURE_CHUNK.min((total - pos) as usize);
        let chunk = &mut chunk[..frames * CHANNELS];
        fill(pos, chunk);
        meter.push(chunk);
        pos += frames as i64;
        report(pos as f32 / total as f32);
    }
    Ok(meter.finish())
}

/// Gain in dB that brings the timeline to `target` LUFS once the limiter has held its peaks,
/// from audio-only passes: the mix as it plays, then the levelled mix until it lands on the
/// target. Silence stays silence: with nothing above the -70 LUFS gate there is no gain.
fn plan_gain(
    project: &Project,
    cache_dir: &Path,
    total: i64,
    target: f64,
    cancel: &AtomicBool,
    mut report: impl FnMut(f32),
) -> Result<f64> {
    let passes = (1 + LEVELLING_PASSES) as f32;
    let mut mixer = Mixer::new(cache_dir.to_path_buf());
    let source = measure(total, cancel, |pos, out| mixer.mix(project, pos, out), |f| report(f / passes))?;
    let Some(lufs) = source.integrated else {
        log::info!("Loudness: nothing above -70 LUFS, exported as it is");
        return Ok(0.0);
    };
    let mut gain = (target - lufs).min(MAX_GAIN_DB);
    log::info!("Loudness: timeline {lufs:.2} LUFS, {:.2} dBTP; gain {gain:+.2} dB", source.true_peak_db());
    if source.true_peak_db() + gain <= LIMITER_CEILING_DB {
        // The limiter has nothing to hold, so the gain lands exactly.
        return Ok(gain);
    }
    // The limiter takes away part of every step up, more the harder it works. The loudness still
    // only grows with the gain, so the passes close in on the target from both sides.
    let mut best = (f64::INFINITY, gain);
    // Passes as (gain, loudness): the latest under and over the target, and the one before this.
    type Pass = Option<(f64, f64)>;
    let (mut below, mut above, mut previous): (Pass, Pass, Pass) = (None, None, None);
    for pass in 1..=LEVELLING_PASSES {
        let mut leveled = Leveled::new(cache_dir, project, gain);
        let at = pass as f32;
        let result = measure(total, cancel, |_, out| leveled.fill(project, out), |f| report((at + f) / passes))?;
        let Some(got) = result.integrated else { break };
        let error = target - got;
        log::info!("Loudness pass {pass}: gain {gain:+.2} dB gives {got:.2} LUFS, {:.2} dBTP", result.true_peak_db());
        if error.abs() < best.0 {
            best = (error.abs(), gain);
        }
        if error.abs() <= TOLERANCE_LU || (error > 0.0 && gain >= MAX_GAIN_DB) {
            break;
        }
        *(if error > 0.0 { &mut below } else { &mut above }) = Some((gain, got));
        let slope = previous
            .filter(|&(g, l)| (gain - g).abs() > 1e-6 && got != l)
            .map_or(1.0, |(g, l)| ((got - l) / (gain - g)).clamp(0.1, 1.0));
        previous = Some((gain, got));
        gain += error / slope;
        if let (Some(lo), Some(hi)) = (below, above)
            && !(lo.0 < gain && gain < hi.0)
        {
            gain = lo.0 + (target - lo.1) * (hi.0 - lo.0) / (hi.1 - lo.1);
        }
        gain = gain.min(MAX_GAIN_DB);
    }
    Ok(best.1)
}

fn encode_audio(enc: &mut ff::encoder::Audio, pos: i64, mix: &[f32], frame_size: usize) -> Result<()> {
    let mut f = frame::Audio::new(
        ff::format::Sample::F32(ff::format::sample::Type::Planar),
        frame_size,
        ff::ChannelLayout::STEREO,
    );
    f.set_rate(SAMPLE_RATE);
    for ch in 0..CHANNELS {
        let plane = f.plane_mut::<f32>(ch);
        for (n, s) in plane.iter_mut().enumerate().take(frame_size) {
            *s = mix[n * CHANNELS + ch];
        }
    }
    f.set_pts(Some(pos));
    enc.send_frame(&f)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_refuses_late_collision_unless_replace_was_confirmed() {
        let dir = std::env::temp_dir().join(format!("nuzky-publish-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let tmp = dir.join("rendered.mp4");
        let out = dir.join("out.mp4");
        let cancel = AtomicBool::new(false);
        std::fs::write(&tmp, b"rendered").unwrap();
        std::fs::write(&out, b"appeared during rendering").unwrap();
        assert!(!ExportOptions::default().replace_existing);
        assert!(publish(&tmp, &out, false, &cancel).unwrap_err().to_string().contains("OUTPUT_EXISTS"));
        assert_eq!(std::fs::read(&out).unwrap(), b"appeared during rendering");
        assert!(tmp.exists());
        publish(&tmp, &out, true, &cancel).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"rendered");
        assert!(!tmp.exists());
        std::fs::remove_file(&out).unwrap();
        std::fs::write(&tmp, b"new render").unwrap();
        publish(&tmp, &out, false, &cancel).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"new render");
        assert!(!tmp.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn publish_without_hard_links_claims_the_name_and_never_replaces() {
        // EPERM, what Linux returns for a hard link on FAT32 and exFAT.
        let no_links = |_: &Path, _: &Path| Err(std::io::Error::from_raw_os_error(1));
        let dir = std::env::temp_dir().join(format!("nuzky-publish-fat-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let tmp = dir.join("rendered.mp4");
        let out = dir.join("out.mp4");
        let cancel = AtomicBool::new(false);
        std::fs::write(&tmp, b"rendered").unwrap();
        std::fs::write(&out, b"appeared during rendering").unwrap();
        let error = publish_with(&tmp, &out, false, &cancel, no_links).unwrap_err();
        assert!(error.to_string().contains("OUTPUT_EXISTS"), "{error:#}");
        assert_eq!(std::fs::read(&out).unwrap(), b"appeared during rendering");
        assert!(tmp.exists());
        std::fs::remove_file(&out).unwrap();
        publish_with(&tmp, &out, false, &cancel, no_links).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"rendered");
        assert!(!tmp.exists());
        // A failed rename leaves neither the claimed name nor a partial file behind.
        let other = dir.join("other.mp4");
        assert!(publish_with(&tmp, &other, false, &cancel, no_links).is_err());
        assert!(!other.exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn publish_honors_cancel_after_rendering_in_both_modes() {
        let dir = std::env::temp_dir().join(format!("nuzky-cancel-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let tmp = dir.join("rendered.mp4");
        let out = dir.join("out.mp4");
        std::fs::write(&tmp, b"finished render").unwrap();
        let cancel = AtomicBool::new(false);
        cancel.store(true, Ordering::Relaxed);
        for replace in [false, true] {
            assert!(publish(&tmp, &out, replace, &cancel).unwrap_err().to_string().contains("cancelled"));
            assert!(!out.exists());
            std::fs::write(&out, b"original").unwrap();
            assert!(publish(&tmp, &out, replace, &cancel).is_err());
            assert_eq!(std::fs::read(&out).unwrap(), b"original");
            std::fs::remove_file(&out).unwrap();
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn yuv_on_several_threads_is_the_one_pass_conversion() {
        init();
        let (w, h) = (1920, 1080);
        let mut rgba = frame::Video::new(Pixel::RGBA, w, h);
        let stride = rgba.stride(0);
        // Noise, so a band edge that filtered or rounded chroma differently would show.
        let mut seed = 1u32;
        for row in rgba.data_mut(0).chunks_mut(stride) {
            for byte in &mut row[..w as usize * 4] {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                *byte = seed as u8;
            }
        }
        let mut one_pass =
            scaling::Context::get(Pixel::RGBA, w, h, Pixel::YUV420P, w, h, scaling::Flags::BICUBIC).unwrap();
        crate::media::set_sws_colorspace(&mut one_pass, ff::ffi::SWS_CS_DEFAULT, true, ff::ffi::SWS_CS_ITU709, false);
        let mut expected = frame::Video::new(Pixel::YUV420P, w, h);
        one_pass.run(&rgba, &mut expected).unwrap();

        let mut to_yuv = to_yuv(w, h).unwrap();
        // A machine with one core converts on one thread; the pixels must match either way.
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        assert!(to_yuv.threads() > 1 || cores == 1, "the conversion runs on {} thread", to_yuv.threads());
        let mut got = frame::Video::new(Pixel::YUV420P, w, h);
        to_yuv.run(&rgba, &mut got).unwrap();
        for plane in 0..3 {
            let width = got.plane_width(plane) as usize;
            let row = |f: &frame::Video, y: usize| f.data(plane)[y * f.stride(plane)..][..width].to_vec();
            let differ = (0..got.plane_height(plane) as usize).filter(|&y| row(&got, y) != row(&expected, y)).count();
            assert_eq!(differ, 0, "{differ} rows of plane {plane} differ");
        }
    }

    #[test]
    fn overlap_writes_in_order_and_stops_at_cancel_or_either_sides_error() {
        let cancel = AtomicBool::new(false);
        let mut written = Vec::new();
        overlap(
            5,
            &cancel,
            |i| Ok(i * 10),
            |i, item| {
                written.push((i, item));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(written, [(0, 0), (1, 10), (2, 20), (3, 30), (4, 40)]);

        // Cancelled while item 4 waits to be handed over: the writer must not write it.
        let mut written = Vec::new();
        let error = overlap(10, &cancel, Ok, |i, _| {
            written.push(i);
            if i == 3 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                cancel.store(true, Ordering::Relaxed);
            }
            Ok(())
        })
        .unwrap_err();
        assert!(error.to_string().starts_with("CANCELLED"), "{error:#}");
        assert_eq!(written, [0, 1, 2, 3]);

        cancel.store(false, Ordering::Relaxed);
        let error = overlap(10, &cancel, Ok, |i, _| if i == 2 { bail!("disk full") } else { Ok(()) }).unwrap_err();
        assert_eq!(error.to_string(), "disk full");
        let error = overlap(10, &cancel, |i| if i == 2 { bail!("decoding failed") } else { Ok(i) }, |_, _| Ok(()));
        assert_eq!(error.unwrap_err().to_string(), "decoding failed");
        // Making fails while the writer is busy with the frame before; the writer failing after it does not hide it.
        let error = overlap(
            10,
            &cancel,
            |i| if i == 2 { bail!("decoding failed") } else { Ok(i) },
            |i, _| {
                if i == 1 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    bail!("disk full");
                }
                Ok(())
            },
        );
        assert_eq!(error.unwrap_err().to_string(), "decoding failed");
    }

    #[test]
    fn quality_matches_desktop_presets() {
        assert_eq!(Quality::High.crf(), 17);
        assert_eq!(Quality::Recommended.crf(), 21);
        assert_eq!(Quality::Small.crf(), 26);
        assert_eq!(ExportOptions::default().crf, Quality::Recommended.crf());
    }

    #[test]
    fn frames_start_on_the_rounded_times_cuts_are_made_at() {
        // The playhead snaps to round(134 / 30 s) = 4_466_667 µs; the frame must not be 4_466_666.
        assert_eq!(frame_time_us(134, 30), 4_466_667);
        assert_eq!(frame_time_us(1, 30), 33_333);
        assert_eq!(frame_time_us(30, 30), 1_000_000);
        assert_eq!(frame_time_us(1, 24), 41_667);
    }

    #[test]
    fn frame_count_covers_frames_that_start_before_the_end() {
        assert_eq!(frame_count(0, 30), 0);
        assert_eq!(frame_count(1, 30), 1);
        assert_eq!(frame_count(91_900_000, 30), 2757);
        // A timeline ending on a rounded-up frame time has no frame at the end itself.
        assert_eq!(frame_count(4_466_667, 30), 134);
        assert_eq!(frame_count(4_466_668, 30), 135);
        for d in [1, 33_333, 33_334, 1_000_000, 4_466_666, 4_466_667, 91_900_000] {
            let n = frame_count(d, 30);
            assert!(frame_time_us(n - 1, 30) < d && frame_time_us(n, 30) >= d, "{d}");
        }
    }

    #[test]
    fn oversized_portrait_export_returns_error() {
        let project = Project::new("oversized");
        let options = ExportOptions { resolution: Some(7680), ..ExportOptions::default() };
        let result = output_size(&project, &options, 8192);
        assert!(result.is_err(), "7680 x 13654 exceeds the 8192 px device limit");
        assert_eq!(result.unwrap_err().to_string(), "This resolution is larger than your GPU supports (max 8192 px)");
    }

    #[test]
    fn output_resolution_keeps_aspect_and_even_dimensions() {
        let mut project = Project::new("size");
        let options = ExportOptions { resolution: Some(720), ..ExportOptions::default() };
        assert_eq!(output_size(&project, &options, 8192).unwrap(), (720, 1280));
        project.canvas.width = 1920;
        project.canvas.height = 1080;
        assert_eq!(output_size(&project, &options, 8192).unwrap(), (1280, 720));
        project.canvas.width = 1001;
        project.canvas.height = 777;
        let (w, h) = output_size(&project, &options, 8192).unwrap();
        assert_eq!((w % 2, h % 2, h), (0, 0, 720));
        assert!((w as f64 / h as f64 - 1001.0 / 777.0).abs() < 0.002);
    }
}

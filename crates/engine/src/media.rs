//! Probing, video decoding and audio extraction on top of FFmpeg.
//!
//! All media times are microseconds relative to the container start, so audio and
//! video of the same file share one origin.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use std::sync::{Arc, Once, OnceLock};

use anyhow::{Context as _, Result, anyhow, bail};
use ff::codec::packet::side_data::Type as SideDataType;
use ff::format::stream::Disposition;
use ff::software::{resampling, scaling};
use ff::util::{color, format::Pixel, frame};
use ffmpeg_next as ff;

use crate::audio::PeakWriter;
use crate::model::{Asset, AssetKind, CHANNELS, SAMPLE_RATE};

pub fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        ff::init().expect("FFmpeg failed to initialise");
        ff::util::log::set_level(ff::util::log::Level::Error);
    });
}

/// Demuxers that open other files or URLs named inside the media, or run scripts.
const INDIRECT_DEMUXERS: &[&str] =
    &["hls", "dash", "concat", "imf", "sdp", "rtsp", "rtp", "sap", "avisynth", "vapoursynth"];

/// Every demuxer except the indirect ones; FFmpeg checks it before reading the file header.
fn format_whitelist() -> &'static str {
    static LIST: OnceLock<String> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut names = Vec::new();
        let mut opaque = std::ptr::null_mut();
        loop {
            let format = unsafe { ff::ffi::av_demuxer_iterate(&mut opaque) };
            if format.is_null() {
                break;
            }
            let name = unsafe { std::ffi::CStr::from_ptr((*format).name) }.to_string_lossy();
            if !name.split(',').any(|n| INDIRECT_DEMUXERS.contains(&n)) {
                names.push(name.into_owned());
            }
        }
        names.join(",")
    })
}

/// Local media only: no network protocols, also for files a container refers to, and no playlists or scripts that
/// pull in other sources.
fn input_options() -> ff::Dictionary<'static> {
    let mut options = ff::Dictionary::new();
    options.set("protocol_whitelist", "file");
    options.set("format_whitelist", format_whitelist());
    options
}

pub(crate) fn open_input(path: &Path) -> Result<ff::format::context::Input> {
    init();
    ff::format::input_with_dictionary(path, input_options()).with_context(|| format!("Cannot open {}", path.display()))
}

pub(crate) fn origin_us(input: &ff::format::context::Input) -> i64 {
    let start = unsafe { (*input.as_ptr()).start_time };
    if start == ff::ffi::AV_NOPTS_VALUE { 0 } else { start }
}

pub(crate) fn is_image_format(input: &ff::format::context::Input) -> bool {
    let format = input.format();
    let name = format.name();
    name == "image2" || name.ends_with("_pipe") || tile_grid(input).is_some()
}

/// A picture stored as tiles, as iPhones keep HEIC photos. FFmpeg demuxes each tile as a stream of its own; only
/// its command-line tool joins them.
#[cfg_attr(not(ffmpeg_7_1), allow(dead_code))]
struct TileGrid {
    /// The stream of each tile and where its top left corner goes on the canvas.
    tiles: Vec<(usize, i32, i32)>,
    canvas: (u32, u32),
    /// The part of the canvas that is the picture: left, top, width, height.
    picture: [u32; 4],
    orientation: Orientation,
}

/// FFmpeg before 7.1 does not read tiles as a grid (see build.rs).
#[cfg(not(ffmpeg_7_1))]
fn tile_grid(_input: &ff::format::context::Input) -> Option<TileGrid> {
    None
}

#[cfg(ffmpeg_7_1)]
unsafe fn slice<'a, T>(data: *const T, len: usize) -> &'a [T] {
    if len == 0 { &[] } else { unsafe { std::slice::from_raw_parts(data, len) } }
}

#[cfg(ffmpeg_7_1)]
fn tile_grid(input: &ff::format::context::Input) -> Option<TileGrid> {
    use ff::ffi::*;
    let input = unsafe { &*input.as_ptr() };
    let groups: Vec<&AVStreamGroup> = unsafe { slice(input.stream_groups, input.nb_stream_groups as usize) }
        .iter()
        .map(|group| unsafe { &**group })
        .filter(|group| group.type_ == AVStreamGroupParamsType::AV_STREAM_GROUP_PARAMS_TILE_GRID)
        .collect();
    // An iPhone photo also keeps its HDR gain map as tiles, and the picture of a file can be a single image beside
    // grids of other items: the picture is the primary item.
    let primary = |disposition: i32| disposition & AV_DISPOSITION_DEFAULT != 0;
    let group = match groups.iter().find(|group| primary(group.disposition)) {
        Some(group) => group,
        None if unsafe { slice(input.streams, input.nb_streams as usize) }
            .iter()
            .any(|stream| primary(unsafe { (**stream).disposition })) =>
        {
            return None;
        }
        None => groups.first()?,
    };
    let grid = unsafe { &*group.params.tile_grid };
    let tiles = unsafe { slice(grid.offsets, grid.nb_tiles as usize) }
        .iter()
        .map(|tile| {
            (unsafe { (**group.streams.add(tile.idx as usize)).index } as usize, tile.horizontal, tile.vertical)
        })
        .collect();
    let orientation = unsafe { slice(grid.coded_side_data, grid.nb_coded_side_data as usize) }
        .iter()
        .filter(|sd| sd.type_ == AVPacketSideDataType::AV_PKT_DATA_DISPLAYMATRIX)
        .find_map(|sd| matrix_orientation(unsafe { slice(sd.data, sd.size) }))
        .unwrap_or((0, false));
    let size = |v: i32| u32::try_from(v).ok();
    let canvas = (size(grid.coded_width)?, size(grid.coded_height)?);
    let picture = [size(grid.horizontal_offset)?, size(grid.vertical_offset)?, size(grid.width)?, size(grid.height)?];
    let [left, top, width, height] = picture;
    let fits = width > 0 && height > 0 && left + width <= canvas.0 && top + height <= canvas.1;
    fits.then_some(TileGrid { tiles, canvas, picture, orientation })
}

fn join_tiles(
    input: &mut ff::format::context::Input,
    decoder: &mut ff::decoder::Video,
    grid: &TileGrid,
) -> Result<frame::Video> {
    // A grid may show one tile in several places.
    let mut packets = input.packets().filter_map(|(stream, mut packet)| {
        grid.tiles.iter().any(|tile| tile.0 == stream.index()).then(|| {
            packet.set_pts(Some(stream.index() as i64));
            packet
        })
    });
    let (mut canvas, mut tile, mut sent_all, mut failed) = (None, frame::Video::empty(), false, 0);
    while !sent_all {
        match packets.next() {
            Some(packet) => {
                if let Err(e) = decoder.send_packet(&packet) {
                    log::warn!("Skipping a corrupt tile: {e}");
                }
            }
            None => {
                decoder.send_eof().ok();
                sent_all = true;
            }
        }
        loop {
            match decoder.receive_frame(&mut tile) {
                Ok(()) => {}
                Err(ff::Error::Other { errno }) if errno == ff::error::EAGAIN => break,
                Err(ff::Error::Eof) => break,
                // As ffmpeg itself does: the next tiles may still decode. A tile fails at most once, so a decoder
                // that keeps failing, as one out of memory does, ends here.
                Err(e) if failed < grid.tiles.len() => {
                    failed += 1;
                    log::warn!("Skipping a corrupt tile: {e}");
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
            let canvas = match &mut canvas {
                Some(canvas) => canvas,
                None => canvas.insert(blank(&tile, grid.canvas)?),
            };
            for &(_, x, y) in grid.tiles.iter().filter(|(stream, _, _)| tile.pts() == Some(*stream as i64)) {
                paste(canvas, &tile, x, y);
            }
        }
    }
    let mut canvas = canvas.ok_or_else(|| anyhow!("No tile of the picture decodes"))?;
    let [left, top, width, height] = grid.picture;
    unsafe {
        let f = canvas.as_mut_ptr();
        ((*f).crop_left, (*f).crop_top) = (left as usize, top as usize);
        ((*f).crop_right, (*f).crop_bottom) =
            ((grid.canvas.0 - left - width) as usize, (grid.canvas.1 - top - height) as usize);
        if ff::ffi::av_frame_apply_cropping(f, ff::ffi::AV_FRAME_CROP_UNALIGNED as i32) < 0 {
            bail!("Cannot cut the picture out of its tiles");
        }
    }
    Ok(canvas)
}

/// Black, so a tile that does not decode never shows what the memory held.
fn blank(like: &frame::Video, (width, height): (u32, u32)) -> Result<frame::Video> {
    let mut canvas = frame::Video::empty();
    canvas.set_format(like.format());
    canvas.set_width(width);
    canvas.set_height(height);
    unsafe {
        let f = canvas.as_mut_ptr();
        if ff::ffi::av_frame_get_buffer(f, 0) < 0 {
            bail!("A picture of {width}×{height} does not fit in memory");
        }
        ff::ffi::av_frame_copy_props(f, like.as_ptr());
        let linesize = std::array::from_fn::<_, 4, _>(|i| (*f).linesize[i] as isize);
        ff::ffi::av_image_fill_black(
            (*f).data.as_ptr(),
            linesize.as_ptr(),
            like.format().into(),
            (*f).color_range,
            width as i32,
            height as i32,
        );
    }
    // HEIF without a colour matrix is BT.601, as FFmpeg and libheif read it; by its height the joined picture
    // would pass for HD video.
    if canvas.color_space() == color::Space::Unspecified {
        canvas.set_color_space(color::Space::SMPTE170M);
    }
    Ok(canvas)
}

fn paste(canvas: &mut frame::Video, tile: &frame::Video, x: i32, y: i32) {
    let format = canvas.format();
    let Some(descriptor) = format.descriptor() else { return };
    let (sub_x, sub_y) = (1 << descriptor.log2_chroma_w(), 1 << descriptor.log2_chroma_h());
    let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else { return };
    if tile.format() != format || x >= canvas.width() || y >= canvas.height() || x % sub_x != 0 || y % sub_y != 0 {
        log::warn!("Skipping a tile that does not fit its picture");
        return;
    }
    let (width, height) = (tile.width().min(canvas.width() - x), tile.height().min(canvas.height() - y));
    for plane in 0..canvas.planes() {
        let bytes =
            |px: u32| unsafe { ff::ffi::av_image_get_linesize(format.into(), px as i32, plane as i32) } as usize;
        let rows = |px: u32| if plane == 1 || plane == 2 { px.div_ceil(sub_y) } else { px } as usize;
        let (left, row, top) = (bytes(x), bytes(width), rows(y));
        let (from_stride, to_stride) = (tile.stride(plane), canvas.stride(plane));
        let (from, to) = (tile.data(plane), canvas.data_mut(plane));
        for r in 0..rows(height) {
            to[(top + r) * to_stride + left..][..row].copy_from_slice(&from[r * from_stride..][..row]);
        }
    }
}

/// The file at `path` as it is now: its size, modification time and a hash of the path, so a changed or
/// replaced file, or another file under the same name in a copied project, gets other caches.
pub(crate) fn file_key(path: &str) -> String {
    let revision = match std::fs::metadata(path) {
        Ok(metadata) => {
            let modified = metadata
                .modified()
                .ok()
                .map(|time| {
                    time.duration_since(std::time::UNIX_EPOCH).map_or_else(
                        |before| format!("pre{}", before.duration().as_nanos()),
                        |since| since.as_nanos().to_string(),
                    )
                })
                .unwrap_or_else(|| "unknown".into());
            format!("{}-{modified}", metadata.len())
        }
        Err(_) => "missing".into(),
    };
    // FNV-1a: stable across builds, so caches survive an update.
    let source = path.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    format!("{revision}-{source:016x}")
}

pub(crate) fn video_stream(input: &ff::format::context::Input) -> Option<ff::format::stream::Stream<'_>> {
    input
        .streams()
        .filter(|s| s.parameters().medium() == ff::media::Type::Video)
        .find(|s| !s.disposition().contains(Disposition::ATTACHED_PIC))
}

/// How to show the stored picture: rotated clockwise by 0/90/180/270 degrees, then mirrored left
/// to right when `.1` is set.
type Orientation = (u32, bool);

/// The orientation a display matrix asks for, snapped to quarter turns.
fn matrix_orientation(matrix: &[u8]) -> Option<Orientation> {
    if matrix.len() < 36 {
        return None;
    }
    let mut m: [i32; 9] = std::array::from_fn(|i| i32::from_ne_bytes(matrix[i * 4..i * 4 + 4].try_into().unwrap()));
    // FFmpeg maps (p, q) to (a p + c q, b p + d q), with a, b, c, d at 0, 1, 3, 4. A negative
    // determinant mirrors; mirroring the output back (negating a and c) leaves the rotation.
    let mirror = i64::from(m[0]) * i64::from(m[4]) - i64::from(m[1]) * i64::from(m[3]) < 0;
    if mirror {
        (m[0], m[3]) = (-m[0], -m[3]);
    }
    let ccw = unsafe { ff::ffi::av_display_rotation_get(m.as_ptr()) };
    if ccw.is_nan() {
        return Some((0, false));
    }
    let cw = (-ccw).rem_euclid(360.0);
    Some((((cw / 90.0).round() as u32 % 4) * 90, mirror))
}

fn display_orientation(stream: &ff::format::stream::Stream) -> Orientation {
    stream
        .side_data()
        .filter(|sd| sd.kind() == SideDataType::DisplayMatrix)
        .find_map(|sd| matrix_orientation(sd.data()))
        .unwrap_or((0, false))
}

/// Still images carry EXIF orientation only on the decoded frame.
fn frame_orientation(f: &frame::Video) -> Option<Orientation> {
    matrix_orientation(f.side_data(frame::side_data::Type::DisplayMatrix)?.data())
}

/// Decodes the first frame of a still image to read its EXIF orientation.
fn image_orientation(
    input: &mut ff::format::context::Input,
    index: usize,
    mut decoder: ff::decoder::Video,
) -> Orientation {
    let mut f = frame::Video::empty();
    for (stream, packet) in input.packets() {
        if stream.index() == index && decoder.send_packet(&packet).is_ok() && decoder.receive_frame(&mut f).is_ok() {
            return frame_orientation(&f).unwrap_or((0, false));
        }
    }
    decoder.send_eof().ok();
    if decoder.receive_frame(&mut f).is_ok() { frame_orientation(&f).unwrap_or((0, false)) } else { (0, false) }
}

/// Header and stream information safe to include in a bug report, without decoding frames.
pub fn describe(path: &Path) -> Result<String> {
    let input = open_input(path)?;
    let image = is_image_format(&input);
    let mut parts = vec![input.format().name().split(',').next().unwrap_or_default().to_owned()];
    if let Some(stream) = video_stream(&input) {
        let context = ff::codec::context::Context::from_parameters(stream.parameters())?;
        let codec = unsafe { &*context.as_ptr() };
        if !image {
            let mut name = stream.parameters().id().name().to_owned();
            let profile = unsafe { ff::ffi::avcodec_profile_name(codec.codec_id, codec.profile) };
            if !profile.is_null() {
                name.push(' ');
                name.push_str(&unsafe { std::ffi::CStr::from_ptr(profile) }.to_string_lossy());
            }
            parts.push(name);
        }
        let grid = tile_grid(&input);
        let (width, height) =
            grid.as_ref().map_or((codec.width, codec.height), |g| (g.picture[2] as i32, g.picture[3] as i32));
        if width > 0 && height > 0 {
            parts.push(format!("{width}×{height}"));
        }
        if !image {
            if let Some(pixel) = Pixel::from(codec.pix_fmt).descriptor() {
                parts.push(pixel.name().to_owned());
            }
            let avg = stream.avg_frame_rate();
            let rate = stream.rate();
            if avg.numerator() > 0 && avg.denominator() > 0 {
                let avg = f64::from(avg);
                let vfr = rate.numerator() > 0 && rate.denominator() > 0 && (f64::from(rate) / avg - 1.0).abs() > 0.005;
                parts.push(format!("{avg:.2} fps{}", if vfr { " VFR" } else { "" }));
            }
        }
        let (rotation, mirror) = grid.as_ref().map_or_else(|| display_orientation(&stream), |g| g.orientation);
        if rotation != 0 {
            parts.push(format!("rotated {rotation}°"));
        }
        if mirror {
            parts.push("mirrored".into());
        }
        if !image {
            match color::TransferCharacteristic::from(codec.color_trc) {
                color::TransferCharacteristic::SMPTE2084 => parts.push("PQ HDR".into()),
                color::TransferCharacteristic::ARIB_STD_B67 => parts.push("HLG HDR".into()),
                _ => {}
            }
        }
    }
    if !image {
        if let Some(stream) = input.streams().best(ff::media::Type::Audio) {
            let params = stream.parameters();
            let codec = unsafe { &*params.as_ptr() };
            let mut audio = vec![params.id().name().to_owned()];
            if codec.sample_rate > 0 {
                audio.push(format!("{} kHz", f64::from(codec.sample_rate) / 1000.0));
            }
            if codec.ch_layout.nb_channels > 0 {
                audio.push(format!("{} ch", codec.ch_layout.nb_channels));
            }
            parts.push(audio.join(" "));
        }
        if input.duration() > 0 {
            parts.push(format!("{:.1} s", input.duration() as f64 / 1e6));
        }
        const KEYS: &[&str] = &[
            "com.apple.quicktime.make",
            "com.apple.quicktime.model",
            "com.apple.quicktime.software",
            "make",
            "model",
            "com.android.manufacturer",
            "com.android.model",
            "com.android.version",
            "encoder",
        ];
        let streams: Vec<_> = input.streams().collect();
        for metadata in std::iter::once(input.metadata()).chain(streams.iter().map(|s| s.metadata())) {
            for key in KEYS {
                if let Some(value) = metadata.get(key) {
                    let value: String = value.chars().filter(|c| !c.is_control()).collect();
                    let value: String = value.trim().chars().take(40).collect();
                    if !value.is_empty() && !parts.contains(&value) {
                        parts.push(value);
                    }
                }
            }
        }
    }
    Ok(parts.join(" · "))
}

pub fn probe(path: &Path, id: String) -> Result<Asset> {
    let mut input = open_input(path)?;
    let is_image = is_image_format(&input);
    let video = video_stream(&input);
    let audio = input.streams().best(ff::media::Type::Audio);
    if video.is_none() && audio.is_none() {
        bail!("{} has no video or audio", path.display());
    }

    let mut asset = Asset {
        id,
        name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        path: dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().into_owned(),
        kind: if is_image {
            AssetKind::Image
        } else if video.is_some() {
            AssetKind::Video
        } else {
            AssetKind::Audio
        },
        duration_us: if is_image { 0 } else { input.duration().max(0) },
        width: 0,
        height: 0,
        fps: 0.0,
        has_audio: audio.is_some() && !is_image,
        rotation: 0,
        mirror: false,
        credit: None,
        part: None,
    };

    if let Some(grid) = tile_grid(&input) {
        let ([_, _, w, h], (rotation, mirror)) = (grid.picture, grid.orientation);
        (asset.width, asset.height) = if rotation % 180 == 90 { (h, w) } else { (w, h) };
        (asset.rotation, asset.mirror) = (rotation, mirror);
    } else if let Some(stream) = video {
        let decoder = ff::codec::context::Context::from_parameters(stream.parameters())?.decoder().video()?;
        let (mut rotation, mut mirror) = display_orientation(&stream);
        let (w, h) = (decoder.width(), decoder.height());
        let rate = stream.avg_frame_rate();
        let rate = if rate.denominator() == 0 || rate.numerator() == 0 { stream.rate() } else { rate };
        asset.fps = if rate.denominator() == 0 { 0.0 } else { f64::from(rate) };
        if is_image && (rotation, mirror) == (0, false) {
            let index = stream.index();
            (rotation, mirror) = image_orientation(&mut input, index, decoder);
        }
        (asset.width, asset.height) = if rotation % 180 == 90 { (h, w) } else { (w, h) };
        (asset.rotation, asset.mirror) = (rotation, mirror);
    }
    Ok(asset)
}

/// The size of a picture's first frame as FFmpeg decodes it, where FFmpeg refuses every frame of more than
/// `max_pixels` before making room for it. Run it on a picture from outside before `probe` or a decoder opens it:
/// their `avformat_find_stream_info` decodes frames of any size, and FFmpeg finds frames that a file's header does not
/// admit to. Errs when the file is not a picture, any frame fails, there is none, or `cancelled` says so.
pub fn picture_size(path: &Path, max_pixels: usize, cancelled: impl Fn() -> bool) -> Result<(u32, u32)> {
    init();
    let cannot = || format!("Cannot read {} as a picture", path.display());
    let url = std::ffi::CString::new(path.to_str().with_context(cannot)?).with_context(cannot)?;
    // Without `avformat_find_stream_info`, which ffmpeg-next's `input_with_dictionary` runs.
    let mut input = unsafe {
        let (mut context, mut options) = (std::ptr::null_mut(), input_options().disown());
        let opened = ff::ffi::avformat_open_input(&mut context, url.as_ptr(), std::ptr::null_mut(), &mut options);
        ff::Dictionary::own(options);
        if opened < 0 {
            return Err(ff::Error::from(opened)).with_context(cannot);
        }
        ff::format::context::Input::wrap(context)
    };
    // A picture's demuxer makes its one stream as it opens, so no stream comes later.
    if !is_image_format(&input) || input.streams().count() != 1 {
        bail!("{} is not a picture", path.display());
    }
    let mut context = ff::codec::context::Context::from_parameters(input.stream(0).with_context(cannot)?.parameters())?;
    unsafe { (*context.as_mut_ptr()).max_pixels = i64::try_from(max_pixels).unwrap_or(i64::MAX) };
    let mut decoder = context.decoder().video().with_context(cannot)?;
    let (mut frame, mut size) = (frame::Video::empty(), None);
    loop {
        if cancelled() {
            bail!("CANCELLED: measuring {} stopped", path.display());
        }
        let mut packet = ff::Packet::empty();
        match packet.read(&mut input) {
            Ok(()) if packet.stream() == 0 => decoder.send_packet(&packet),
            Ok(()) => continue,
            Err(ff::Error::Eof) => decoder.send_eof(),
            Err(error) => Err(error),
        }
        .with_context(cannot)?;
        loop {
            match decoder.receive_frame(&mut frame) {
                Ok(()) => _ = size.get_or_insert((frame.width(), frame.height())),
                Err(ff::Error::Other { errno }) if errno == ff::error::EAGAIN => break,
                Err(ff::Error::Eof) => return size.with_context(|| format!("{} has no picture", path.display())),
                Err(error) => return Err(error).with_context(cannot),
            }
        }
    }
}

/// One decoded frame converted to tightly packed RGBA, in source orientation.
#[derive(Clone)]
pub struct RgbaFrame {
    pub t_us: i64,
    pub width: u32,
    pub height: u32,
    pub data: Arc<Vec<u8>>,
    pub transfer: Transfer,
}

/// How a frame's RGB values encode light. HDR frames are BT.2020 and get tone mapped when drawn.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Transfer {
    #[default]
    Sdr,
    Pq,
    Hlg,
}

fn transfer(f: &frame::Video) -> Transfer {
    match f.color_transfer_characteristic() {
        color::TransferCharacteristic::SMPTE2084 => Transfer::Pq,
        color::TransferCharacteristic::ARIB_STD_B67 => Transfer::Hlg,
        _ => Transfer::Sdr,
    }
}

/// A swscale conversion that runs frames of Full HD and larger on several threads, each converting a band of rows the
/// way one pass over the whole frame would. swscale threads only a context given its thread count before `sws_init_context`, which
/// `scaling::Context` cannot do, and only in `sws_scale_frame`.
pub(crate) struct Scaler(*mut ff::ffi::SwsContext);

// Only the thread that owns the decoder or the writer touches the context.
unsafe impl Send for Scaler {}

impl Scaler {
    /// `colorspace` is the YUV matrix (`SWS_CS_*`) and full range of the source and of the destination; swscale
    /// would otherwise assume BT.601.
    pub(crate) fn new(
        (src, src_w, src_h): (Pixel, u32, u32),
        (dst, dst_w, dst_h): (Pixel, u32, u32),
        flags: scaling::Flags,
        colorspace: [(i32, bool); 2],
    ) -> Result<Self> {
        use ff::ffi::*;
        let scaler = Self(unsafe { sws_alloc_context() });
        if scaler.0.is_null() {
            bail!("Cannot allocate the colour converter");
        }
        let options = [
            (c"srcw", src_w as i64),
            (c"srch", src_h as i64),
            (c"src_format", AVPixelFormat::from(src) as i64),
            (c"dstw", dst_w as i64),
            (c"dsth", dst_h as i64),
            (c"dst_format", AVPixelFormat::from(dst) as i64),
            (c"sws_flags", flags.bits() as i64),
            (c"threads", scaler_threads(src, (src_w, src_h), (dst_w, dst_h))),
        ];
        for (name, value) in options {
            if unsafe { av_opt_set_int(scaler.0.cast(), name.as_ptr(), value, 0) } < 0 {
                bail!("The colour converter has no option {name:?}");
            }
        }
        if unsafe { sws_init_context(scaler.0, std::ptr::null_mut(), std::ptr::null_mut()) } < 0 {
            bail!("Cannot convert {src_w}x{src_h} {src:?} to {dst_w}x{dst_h} {dst:?}");
        }
        let [(src_cs, src_full), (dst_cs, dst_full)] = colorspace;
        unsafe { set_sws_colorspace_raw(scaler.0, src_cs, src_full, dst_cs, dst_full) };
        Ok(scaler)
    }

    pub(crate) fn run(&mut self, src: &frame::Video, dst: &mut frame::Video) -> Result<(), ff::Error> {
        match unsafe { ff::ffi::sws_scale_frame(self.0, dst.as_mut_ptr(), src.as_ptr()) } {
            error if error < 0 => Err(ff::Error::from(error)),
            _ => Ok(()),
        }
    }

    #[cfg(test)]
    pub(crate) fn threads(&self) -> i64 {
        let mut threads = 0;
        assert!(unsafe { ff::ffi::av_opt_get_int(self.0.cast(), c"threads".as_ptr(), 0, &mut threads) } >= 0);
        threads
    }
}

/// One thread per core, at most eight, for frames of Full HD and larger, as export converts them. On a 12-core Apple
/// M-series eight converted faster than twelve, whose bands on the efficiency cores finished last, and every video
/// layer of a project has a converter of its own. The preview's frames, at most 1280 px, stay on one thread: on eight
/// they took the cores the preview renders on, and a frame took twice as long to render while playing. A frame kept at
/// its size goes through swscale's special converters, already fast, and on several threads the NV12 one garbled rows
/// (FFmpeg 8.1); of those only the encoder's from RGBA, which the tests compare, gets threads.
fn scaler_threads(src: Pixel, src_size: (u32, u32), (dst_w, dst_h): (u32, u32)) -> i64 {
    if u64::from(dst_w) * u64::from(dst_h) < 1920 * 1080 || src_size == (dst_w, dst_h) && src != Pixel::RGBA {
        return 1;
    }
    std::thread::available_parallelism().map_or(1, |n| n.get().min(8)) as i64
}

impl Drop for Scaler {
    fn drop(&mut self) {
        unsafe { ff::ffi::sws_freeContext(self.0) }
    }
}

/// The YUV matrix a frame is shown with: its own, or by its height when the file does not name one.
pub(crate) fn matrix(f: &frame::Video) -> color::Space {
    use color::Space::*;
    match f.color_space() {
        space @ (BT709 | BT2020NCL | BT2020CL | SMPTE240M | FCC | BT470BG | SMPTE170M) => space,
        _ if f.height() >= 720 => BT709,
        _ => SMPTE170M,
    }
}

pub(crate) fn sws_colorspace(space: color::Space) -> i32 {
    use ff::ffi::*;
    match space {
        color::Space::BT709 => SWS_CS_ITU709,
        color::Space::BT2020NCL | color::Space::BT2020CL => SWS_CS_BT2020,
        color::Space::SMPTE240M => SWS_CS_SMPTE240M,
        color::Space::FCC => SWS_CS_FCC,
        _ => SWS_CS_ITU601,
    }
}

/// Sets the YUV matrix and range explicitly; swscale would otherwise assume BT.601.
pub(crate) fn set_sws_colorspace(ctx: &mut scaling::Context, src_cs: i32, src_full: bool, dst_cs: i32, dst_full: bool) {
    unsafe { set_sws_colorspace_raw(ctx.as_mut_ptr(), src_cs, src_full, dst_cs, dst_full) }
}

/// `set_sws_colorspace` for a context made without ffmpeg-next; `ptr` must be initialized and not yet freed.
unsafe fn set_sws_colorspace_raw(
    ptr: *mut ff::ffi::SwsContext,
    src_cs: i32,
    src_full: bool,
    dst_cs: i32,
    dst_full: bool,
) {
    use ff::ffi::*;
    unsafe {
        let (mut inv, mut table) = (std::ptr::null_mut(), std::ptr::null_mut());
        let (mut sr, mut dr, mut b, mut c, mut s) = (0, 0, 0, 0, 0);
        if sws_getColorspaceDetails(ptr, &mut inv, &mut sr, &mut table, &mut dr, &mut b, &mut c, &mut s) < 0 {
            return;
        }
        sws_setColorspaceDetails(
            ptr,
            sws_getCoefficients(src_cs),
            src_full as i32,
            sws_getCoefficients(dst_cs),
            dst_full as i32,
            b,
            c,
            s,
        );
    }
}

pub(crate) fn is_full_range(f: &frame::Video) -> bool {
    f.color_range() == color::Range::JPEG
        || matches!(f.format(), Pixel::YUVJ420P | Pixel::YUVJ422P | Pixel::YUVJ444P | Pixel::YUVJ440P)
}

fn is_rgb(format: Pixel) -> bool {
    use ff::ffi::*;
    let desc = unsafe { av_pix_fmt_desc_get(format.into()) };
    !desc.is_null() && unsafe { (*desc).flags } & (AV_PIX_FMT_FLAG_RGB | AV_PIX_FMT_FLAG_PAL) as u64 != 0
}

type ScalerKey = (Pixel, u32, u32, u32, u32);

/// Scaling 8-bit video, or converting 10-bit at any size, swscale's default RGB output drew limited range about 1.6 to
/// 2 levels of 255 darker than BT.709 and BT.2020 (FFmpeg 8.1); with chroma converted at full width it is exact.
const RGBA_FLAGS: scaling::Flags = scaling::Flags::BILINEAR.union(scaling::Flags::FULL_CHR_H_INT);

fn to_rgba(scaler: &mut Option<(ScalerKey, Scaler)>, f: &frame::Video, t_us: i64, w: u32, h: u32) -> Result<RgbaFrame> {
    let key = (f.format(), f.width(), f.height(), w, h);
    if scaler.as_ref().map(|s| s.0) != Some(key) {
        // swscale scales an RGB source in YUV and must convert it back with the matrix it went in with.
        let space = if is_rgb(f.format()) { ff::ffi::SWS_CS_DEFAULT } else { sws_colorspace(matrix(f)) };
        let colorspace = [(space, is_full_range(f)), (ff::ffi::SWS_CS_DEFAULT, true)];
        let src = (f.format(), f.width(), f.height());
        *scaler = Some((key, Scaler::new(src, (Pixel::RGBA, w, h), RGBA_FLAGS, colorspace)?));
    }
    let mut out = frame::Video::new(Pixel::RGBA, w, h);
    scaler.as_mut().unwrap().1.run(f, &mut out)?;
    let stride = out.stride(0);
    let row = w as usize * 4;
    let src = out.data(0);
    let mut data = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        data.extend_from_slice(&src[y * stride..y * stride + row]);
    }
    Ok(RgbaFrame { t_us, width: w, height: h, data: Arc::new(data), transfer: transfer(f) })
}

pub type DecodedFrame = (i64, frame::Video);

/// Sequential decoder with exact seeking. Picks the last frame whose real timestamp is
/// at or before the requested time, so variable frame rate footage stays exact.
pub struct VideoDecoder {
    input: ff::format::context::Input,
    stream_index: usize,
    decoder: ff::decoder::Video,
    time_base: ff::Rational,
    origin_us: i64,
    sent_eof: bool,
    eof: bool,
    is_image: bool,
    /// The only frame of a still image, replayed after seeking: image2 loses it when seeked.
    image: Option<DecodedFrame>,
    image_rewound: bool,
    grid: Option<TileGrid>,
    /// First frame after a seek, decoded to check where the seek landed.
    pending: Option<DecodedFrame>,
    scaler: Option<(ScalerKey, Scaler)>,
    pub rotation: u32,
    pub frame_duration_us: i64,
}

unsafe impl Send for VideoDecoder {}

/// Frame threading decodes one picture per thread, so the first picture after a seek, usually a large keyframe,
/// decodes on one thread. For `seeking`, HEVC whose rows decode in parallel (`hevc::parallel_rows`; Apple's encoder
/// writes it so) decodes each picture on several threads instead: the first frame shows sooner and seeks land
/// sooner. Reading straight through, as export and proxies do, stays with frame threading: slice threading was faster
/// there on Apple's HEVC but slower on x265's, and fell further behind when other work took the cores.
fn threading(parameters: &ff::codec::Parameters, seeking: bool) -> ff::codec::threading::Config {
    use ff::codec::threading::{Config, Type};
    // SAFETY: the stream owns its parameters and extradata while `parameters` lives.
    let extradata = unsafe {
        let par = &*parameters.as_ptr();
        match usize::try_from(par.extradata_size) {
            Ok(len) if len > 0 && !par.extradata.is_null() => std::slice::from_raw_parts(par.extradata, len),
            _ => &[],
        }
    };
    if seeking && parameters.id() == ff::codec::Id::HEVC && crate::hevc::parallel_rows(extradata) {
        // Rows wait for the row above them, so 4 threads stalled when other work took the cores; 8 stayed ahead.
        Config { kind: Type::Slice, count: 8 }
    } else {
        Config { kind: Type::Frame, count: 4 }
    }
}

impl VideoDecoder {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with(path, false)
    }

    /// For jumping around a file, as the preview and thumbnails do (`threading`).
    pub fn open_for_seeking(path: &Path) -> Result<Self> {
        Self::open_with(path, true)
    }

    fn open_with(path: &Path, seeking: bool) -> Result<Self> {
        let input = open_input(path)?;
        let is_image = is_image_format(&input);
        let grid = tile_grid(&input);
        let stream = match &grid {
            Some(grid) => grid.tiles.first().and_then(|tile| input.stream(tile.0)),
            None => video_stream(&input),
        }
        .ok_or_else(|| anyhow!("{} has no video", path.display()))?;
        let stream_index = stream.index();
        let rotation = grid.as_ref().map_or_else(|| display_orientation(&stream), |grid| grid.orientation).0;
        let time_base = stream.time_base();
        let rate = stream.avg_frame_rate();
        let fps = if rate.numerator() > 0 && rate.denominator() > 0 { f64::from(rate) } else { 30.0 };
        let mut ctx = ff::codec::context::Context::from_parameters(stream.parameters())?;
        ctx.set_threading(threading(&stream.parameters(), seeking));
        let decoder = ctx.decoder().video()?;
        let origin_us = origin_us(&input);
        Ok(Self {
            input,
            stream_index,
            decoder,
            time_base,
            origin_us,
            sent_eof: false,
            eof: false,
            is_image,
            image: None,
            image_rewound: false,
            grid,
            pending: None,
            scaler: None,
            rotation,
            frame_duration_us: (1_000_000.0 / fps.clamp(1.0, 240.0)) as i64,
        })
    }

    /// Opens a preview proxy (`crate::proxy`), whose frames are stored at the original's times.
    pub fn open_proxy(path: &Path) -> Result<Self> {
        let mut decoder = Self::open(path)?;
        decoder.origin_us = 0;
        Ok(decoder)
    }

    /// Decodes on the GPU's video decoder (VideoToolbox), which gives the software decoder's frames for a fraction
    /// of its CPU time. Errs where FFmpeg has no hardware decoder for the video, and `next_frame` errs on a frame the
    /// hardware does not decode, instead of skipping it or decoding it in software.
    pub(crate) fn open_hardware(path: &Path) -> Result<Self> {
        // Elsewhere there is no VideoToolbox: fail before opening the file, which the software decoder opens again.
        if !cfg!(target_os = "macos") {
            bail!("FFmpeg has no hardware video decoder here");
        }
        let mut decoder = Self::open(path)?;
        let stream = decoder.input.stream(decoder.stream_index).context("The video stream is gone")?;
        let mut ctx = ff::codec::context::Context::from_parameters(stream.parameters())?;
        if !on_videotoolbox(ctx.id()) {
            bail!("FFmpeg does not decode {:?} on the GPU", ctx.id());
        }
        // The codec context owns the reference and lets it go when it is freed.
        unsafe { (*ctx.as_mut_ptr()).hw_device_ctx = hardware_device()? };
        // FFmpeg hands the hardware one frame at a time, so more threads would only hold more frames.
        ctx.set_threading(ff::codec::threading::Config { kind: ff::codec::threading::Type::Frame, count: 1 });
        decoder.decoder = ctx.decoder().video()?;
        Ok(decoder)
    }

    fn is_hardware(&self) -> bool {
        unsafe { !(*self.decoder.as_ptr()).hw_device_ctx.is_null() }
    }

    pub fn is_image(&self) -> bool {
        self.is_image
    }

    pub fn duration_us(&self) -> i64 {
        self.input.duration().max(0)
    }

    pub fn source_size(&self) -> (u32, u32) {
        match &self.grid {
            Some(grid) => (grid.picture[2], grid.picture[3]),
            None => (self.decoder.width(), self.decoder.height()),
        }
    }

    pub fn is_eof(&self) -> bool {
        self.eof
    }

    /// Jumps to the keyframe at or before `t_us`. The next decoded frames start there.
    pub fn seek(&mut self, t_us: i64) -> Result<()> {
        if self.is_image {
            self.image_rewound = self.image.is_some();
            return Ok(());
        }
        // Demuxers without an index (MPEG-TS) search packet timestamps and can land after the
        // keyframe, so decoding starts at a later one, or finds none in the last GOP. Seek further
        // back until it does not.
        let mut back = 0;
        loop {
            let target = t_us.saturating_sub(back);
            self.seek_from(target);
            let first = self.next_frame()?;
            if first.as_ref().is_none_or(|(t, _)| *t > t_us) && target > 0 {
                back = (back * 2).max(SEEK_BACK_OFF_US);
                continue;
            }
            self.pending = first;
            return Ok(());
        }
    }

    fn seek_from(&mut self, t_us: i64) {
        use ff::util::mathematics::{Rescale, Rounding};
        let result = if t_us <= 0 {
            // Aim before the first packet: the first timestamp itself can also land past the keyframe.
            let ts = self.origin_us.saturating_sub(1_000_000);
            unsafe { ff::ffi::avformat_seek_file(self.input.as_mut_ptr(), -1, i64::MIN, ts, i64::MAX, 0) }
        } else {
            let ts = t_us.saturating_add(self.origin_us).rescale_with((1, 1_000_000), self.time_base, Rounding::Down);
            // A global-time seek rounds to the nearest stream tick and can skip the covering GOP.
            unsafe {
                ff::ffi::avformat_seek_file(self.input.as_mut_ptr(), self.stream_index as i32, i64::MIN, ts, ts, 0)
            }
        };
        // Seeking images and tiny files can fail harmlessly; decoding restarts from the start.
        if result < 0 {
            self.input.seek(0, ..).ok();
        }
        self.decoder.flush();
        self.pending = None;
        self.sent_eof = false;
        self.eof = false;
    }

    /// The time of the keyframe a seek to `t_us` lands on, read from the file without decoding: None when the
    /// file does not seek to keyframes. The next decoded frames start anywhere until the next `seek`.
    pub(crate) fn keyframe_at(&mut self, t_us: i64) -> Option<i64> {
        self.seek_from(t_us);
        let index = self.stream_index;
        let (_, packet) = self.input.packets().find(|(stream, _)| stream.index() == index)?;
        let pts = packet.pts().filter(|_| packet.is_key())?;
        Some(self.micros(pts))
    }

    fn micros(&self, pts: i64) -> i64 {
        (pts as f64 * f64::from(self.time_base) * 1e6).round() as i64 - self.origin_us
    }

    /// Decodes the next frame in display order. `None` at end of stream.
    pub fn next_frame(&mut self) -> Result<Option<(i64, frame::Video)>> {
        if std::mem::take(&mut self.image_rewound)
            && let Some((t, f)) = &self.image
        {
            return Ok(Some((*t, f.clone())));
        }
        if let Some(frame) = self.pending.take() {
            return Ok(Some(frame));
        }
        if self.eof {
            return Ok(None);
        }
        if let Some(grid) = &self.grid {
            self.eof = true;
            let f = join_tiles(&mut self.input, &mut self.decoder, grid)?;
            self.image = Some((0, f.clone()));
            return Ok(Some((0, f)));
        }
        let mut f = frame::Video::empty();
        loop {
            match self.decoder.receive_frame(&mut f) {
                Ok(()) => {
                    if self.is_hardware() {
                        f = from_hardware(&f, (self.decoder.width(), self.decoder.height()))?;
                    }
                    if self.is_image
                        && let Some((rotation, _)) = frame_orientation(&f)
                    {
                        self.rotation = rotation;
                    }
                    let t = self.micros(f.timestamp().or(f.pts()).unwrap_or(0));
                    if self.is_image {
                        self.image = Some((t, f.clone()));
                    }
                    return Ok(Some((t, f)));
                }
                Err(ff::Error::Eof) => {
                    self.eof = true;
                    return Ok(None);
                }
                Err(ff::Error::Other { errno }) if errno == ff::error::EAGAIN => {}
                Err(e) => return Err(e.into()),
            }
            if self.sent_eof {
                self.eof = true;
                return Ok(None);
            }
            loop {
                match self.input.packets().next() {
                    Some((stream, packet)) if stream.index() == self.stream_index => {
                        match self.decoder.send_packet(&packet) {
                            Err(e) if self.is_hardware() => bail!("The hardware decoder failed: {e}"),
                            Err(e) => log::warn!("Skipping corrupt packet: {e}"),
                            Ok(()) => {}
                        }
                        break;
                    }
                    Some(_) => continue,
                    None => {
                        self.decoder.send_eof().ok();
                        self.sent_eof = true;
                        break;
                    }
                }
            }
        }
    }

    /// Returns the frame covering `want` and the next frame, if decoded.
    /// Pass the previous pair when decoding forward; after seeking pass `[None, None]`.
    pub fn frame_covering(
        &mut self,
        want: i64,
        [mut covering, mut next]: [Option<DecodedFrame>; 2],
    ) -> Result<[Option<DecodedFrame>; 2]> {
        loop {
            let decoded = match next.take() {
                Some(frame) => Some(frame),
                None => self.next_frame()?,
            };
            match decoded {
                Some((t, f)) if t == want => return Ok([Some((t, f)), None]),
                Some((t, f)) if t < want || covering.is_none() && self.is_image() => covering = Some((t, f)),
                next => return Ok([covering, next]),
            }
        }
    }

    pub fn convert(&mut self, f: &frame::Video, t_us: i64, w: u32, h: u32) -> Result<RgbaFrame> {
        to_rgba(&mut self.scaler, f, t_us, w.max(2), h.max(2))
    }
}

/// First step back when a seek lands after the frame it was asked for; doubled on every retry.
const SEEK_BACK_OFF_US: i64 = 1_000_000;

/// VideoToolbox, the video decoder of Apple's GPUs, when FFmpeg was built with it.
fn hardware_device() -> Result<*mut ff::ffi::AVBufferRef> {
    #[cfg(target_os = "macos")]
    {
        let mut device = std::ptr::null_mut();
        let kind = ff::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX;
        if unsafe { ff::ffi::av_hwdevice_ctx_create(&mut device, kind, std::ptr::null(), std::ptr::null_mut(), 0) } >= 0
        {
            return Ok(device);
        }
    }
    bail!("FFmpeg has no hardware video decoder here")
}

/// Whether FFmpeg decodes `codec` with VideoToolbox. It does not decode Motion JPEG there, for one.
fn on_videotoolbox(codec: ff::codec::Id) -> bool {
    let Some(decoder) = ff::decoder::find(codec) else { return false };
    (0..).map_while(|i| unsafe { ff::ffi::avcodec_get_hw_config(decoder.as_ptr(), i).as_ref() }).any(|config| {
        config.device_type == ff::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX
            && config.methods & ff::ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32 != 0
    })
}

/// The picture of a frame the hardware decoded, read in place from the GPU's memory, which Apple's chips share
/// with the CPU. FFmpeg decodes in software what VideoToolbox refuses; that and a frame of another size than the
/// stream's are errors, so a caller can decode the whole file in software instead.
fn from_hardware(f: &frame::Video, size: (u32, u32)) -> Result<frame::Video> {
    if f.format() != Pixel::VIDEOTOOLBOX {
        bail!("The hardware decoder does not decode this video");
    }
    let mut mapped = frame::Video::empty();
    unsafe {
        let flags = ff::ffi::AV_HWFRAME_MAP_READ as i32;
        if ff::ffi::av_hwframe_map(mapped.as_mut_ptr(), f.as_ptr(), flags) < 0 {
            bail!("Cannot read the picture of the hardware decoder");
        }
    }
    if (mapped.width(), mapped.height()) != size {
        bail!("The hardware decoder gave a picture of {}×{}", mapped.width(), mapped.height());
    }
    Ok(mapped)
}

/// Size to decode at so a layer shown at `display` pixels (after rotation) stays sharp
/// without converting more pixels than needed, and no side exceeds `max_side`.
/// Returned in source orientation.
pub fn decode_size(src: (u32, u32), rotation: u32, display: (f32, f32), max_side: u32) -> (u32, u32) {
    let (dw, dh) = if rotation % 180 == 90 { (display.1, display.0) } else { display };
    let scale = (dw / src.0 as f32).max(dh / src.1 as f32).min(1.0).min(max_side as f32 / src.0.max(src.1) as f32);
    let even = |v: f32| (((v.round() as u32).max(2) + 1) & !1).min(max_side & !1);
    (even(src.0 as f32 * scale), even(src.1 as f32 * scale))
}

/// Decodes the whole audio stream to 48 kHz interleaved stereo f32 little-endian.
/// The file starts at the container origin, padded with silence if audio starts late.
/// An error from `progress` stops the extraction and removes the partial file.
pub fn extract_pcm(path: &Path, out: &Path, progress: impl FnMut(f32) -> Result<()>) -> Result<u64> {
    extract(path, out, None, progress)
}

/// Like `extract_pcm`, and writes the waveform peaks of the sound to `peaks` as it is decoded, so the
/// decoded part can be drawn while the rest of a long file is still decoding. They are complete once
/// `out` exists; a stopped or failed extraction removes them.
pub fn extract_pcm_with_peaks(
    path: &Path,
    out: &Path,
    peaks: &Path,
    progress: impl FnMut(f32) -> Result<()>,
) -> Result<u64> {
    extract(path, out, Some(peaks), progress)
}

fn extract(path: &Path, out: &Path, peaks: Option<&Path>, mut progress: impl FnMut(f32) -> Result<()>) -> Result<u64> {
    let mut input = open_input(path)?;
    let stream = input.streams().best(ff::media::Type::Audio).ok_or_else(|| anyhow!("No audio stream"))?;
    let stream_index = stream.index();
    let time_base = f64::from(stream.time_base());
    // Cover art (an MP3's attached picture) is a video stream of one frame, not a picture track.
    let video = input
        .streams()
        .best(ff::media::Type::Video)
        .filter(|s| !s.disposition().contains(ff::format::stream::Disposition::ATTACHED_PIC))
        .map(|s| (s.index(), f64::from(s.time_base())));
    let duration_us = input.duration().max(1) as f64;
    let origin = origin_us(&input);
    let mut decoder = ff::codec::context::Context::from_parameters(stream.parameters())?.decoder().audio()?;

    let tmp = out.with_file_name(format!(".nuzky-pcm-{}.part", crate::edit::new_id()));
    let file = File::options().write(true).create_new(true).open(&tmp)?;
    let result = (|| {
        let mut sink = PcmSink {
            writer: BufWriter::with_capacity(1 << 20, file),
            written: 0,
            resampler: None,
            mono: false,
            shift: None,
            skip: 0,
            audio: 0,
            gaps: 0,
            peaks: peaks.map(PeakWriter::create).transpose()?,
        };
        let mut decoded = frame::Audio::empty();
        let mut last_progress = 0.0;
        let start_us = |f: &frame::Audio| {
            f.timestamp().or(f.pts()).map(|pts| ((pts as f64 * time_base * 1e6) as i64).saturating_sub(origin))
        };

        // Video read so far shows how late real audio may start; demuxing interleaves by time.
        let mut video_end_us = 0;
        for (s, packet) in input.packets() {
            if s.index() != stream_index {
                if let (Some((index, base)), Some(pts)) = (video, packet.pts())
                    && s.index() == index
                {
                    let end_us = (pts.saturating_add(packet.duration()) as f64 * base * 1e6) as i64;
                    video_end_us = video_end_us.max(end_us.saturating_sub(origin));
                }
                continue;
            }
            if decoder.send_packet(&packet).is_err() {
                continue;
            }
            while decoder.receive_frame(&mut decoded).is_ok() {
                sink.write_frame(&decoded, start_us(&decoded), video.map(|_| video_end_us))?;
            }
            if let Some(pts) = packet.pts() {
                let p = (pts as f64 * time_base * 1e6 / duration_us) as f32;
                if p - last_progress > 0.02 {
                    last_progress = p;
                    progress(p.clamp(0.0, 1.0))?;
                }
            }
        }
        decoder.send_eof().ok();
        while decoder.receive_frame(&mut decoded).is_ok() {
            sink.write_frame(&decoded, start_us(&decoded), video.map(|_| video_end_us))?;
        }
        if let Some((mut ctx, _)) = sink.resampler.take() {
            loop {
                let mut tail = frame::Audio::new(PCM_FORMAT, 4096, ctx.output().channel_layout);
                if ctx.flush(&mut tail).is_err() || tail.samples() == 0 {
                    break;
                }
                sink.write_out(&tail)?;
            }
        }
        let PcmSink { mut writer, written, peaks, .. } = sink;
        writer.flush()?;
        drop(writer);
        peaks.map(PeakWriter::finish).transpose()?;
        std::fs::rename(&tmp, out)?;
        // The cache is complete and published; a late stop request has nothing left to stop.
        progress(1.0).ok();
        Ok(written)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
        if let Some(peaks) = peaks {
            let _ = std::fs::remove_file(peaks);
        }
    }
    result
}

/// Writes decoded audio as 48 kHz interleaved stereo f32, placed by its timestamps.
struct PcmSink {
    writer: BufWriter<File>,
    /// Sample frames written so far.
    written: u64,
    resampler: Option<(resampling::Context, (ff::format::Sample, u64, u32))>,
    mono: bool,
    /// Sample frames added to frame times so they land at their timestamps; set by the first timed frame
    /// and moved by timestamp jumps too large to be real.
    shift: Option<i64>,
    /// Converted sample frames still to drop because they overlap audio already written.
    skip: u64,
    /// Sample frames of decoded audio written, and of silence written for gaps after the first frame.
    audio: u64,
    gaps: u64,
    /// The waveform of what is written, when the caller wants it.
    peaks: Option<PeakWriter>,
}

fn us_to_frames(us: i64) -> i64 {
    (us as i128 * SAMPLE_RATE as i128 / 1_000_000) as i64
}

impl PcmSink {
    /// Writes resampled audio as stereo. Mono sources are resampled as mono and duplicated,
    /// so they keep their level instead of FFmpeg's -3 dB pan.
    fn write_out(&mut self, out: &frame::Audio) -> Result<()> {
        let n = out.samples();
        let dropped = self.skip.min(n as u64) as usize;
        self.skip -= dropped as u64;
        if n == dropped {
            return Ok(());
        }
        if self.mono {
            let src: &[f32] = &bytemuck_slice(out.data(0))[dropped..n];
            let mut buf = Vec::with_capacity(src.len() * CHANNELS * 4);
            for s in src {
                buf.extend_from_slice(&s.to_le_bytes());
                buf.extend_from_slice(&s.to_le_bytes());
            }
            self.writer.write_all(&buf)?;
            if let Some(peaks) = &mut self.peaks {
                peaks.push(src, 1)?;
            }
        } else {
            let bytes = &out.data(0)[dropped * CHANNELS * 4..n * CHANNELS * 4];
            self.writer.write_all(bytes)?;
            if let Some(peaks) = &mut self.peaks {
                peaks.push(bytemuck_slice(bytes), CHANNELS)?;
            }
        }
        self.written += (n - dropped) as u64;
        self.audio += (n - dropped) as u64;
        Ok(())
    }

    fn silence(&mut self, frames: i64) -> Result<()> {
        let frames = frames.max(0) as u64;
        std::io::copy(&mut std::io::repeat(0).take(frames * (CHANNELS * 4) as u64), &mut self.writer)?;
        if let Some(peaks) = &mut self.peaks {
            peaks.silence(frames)?;
        }
        self.written += frames;
        Ok(())
    }

    /// Lines up the next samples with `start_us`: the first ones with the container origin, later ones
    /// by filling gaps with silence and dropping overlaps, like FFmpeg's aresample async mode.
    /// `video_end_us` is how far the file's video has been read, or None without a picture track.
    fn place(&mut self, start_us: i64, video_end_us: Option<i64>) -> Result<()> {
        let max_lead_us = max_lead_us(video_end_us.unwrap_or(0));
        let buffered = self.resampler.as_ref().and_then(|(ctx, _)| ctx.delay()).map_or(0, |d| d.output);
        let position = self.written as i64 + buffered - self.skip as i64;
        let Some(shift) = self.shift else {
            // The container duration includes a broken offset, so it cannot bound the silence.
            if start_us > max_lead_us {
                bail!("Audio starts {} s after the video; the file's timestamps look broken", start_us / 1_000_000);
            }
            let target = us_to_frames(start_us).max(position);
            self.shift = Some(target - us_to_frames(start_us));
            return self.silence(target - position);
        };
        let target = us_to_frames(start_us) + shift;
        let gap = target - position;
        if gap.abs() <= us_to_frames(MIN_GAP_US) {
            return Ok(());
        }
        // Bounded, so broken timestamps cannot fill the disk. With video, audio stays near the video
        // read so far; audio alone may pause for as long as it has played, at least the lead-in limit,
        // so the real pauses of a long recording are kept.
        let real = match video_end_us {
            Some(_) => target <= us_to_frames(max_lead_us),
            None => (self.gaps as i64).saturating_add(gap) <= us_to_frames(MAX_AUDIO_LEAD_US).max(self.audio as i64),
        };
        if gap > 0 && real {
            self.gaps += gap as u64;
            return self.silence(gap);
        }
        if gap < 0 && -gap <= us_to_frames(MAX_OVERLAP_US) {
            self.skip += (-gap) as u64;
            return Ok(());
        }
        log::warn!("Ignoring an audio timestamp jump of {} ms", gap * 1000 / SAMPLE_RATE as i64);
        self.shift = Some(shift - gap);
        Ok(())
    }

    fn write_frame(&mut self, f: &frame::Audio, start_us: Option<i64>, video_end_us: Option<i64>) -> Result<()> {
        let mut layout = f.channel_layout();
        if layout.is_empty() {
            layout = ff::ChannelLayout::default(f.channels() as i32);
        }
        let key = (f.format(), layout.bits(), f.rate());
        if self.resampler.as_ref().map(|r| r.1) != Some(key) {
            self.mono = layout.channels() == 1;
            let dst = if self.mono { ff::ChannelLayout::MONO } else { ff::ChannelLayout::STEREO };
            let ctx = resampling::Context::get(f.format(), layout, f.rate(), PCM_FORMAT, dst, SAMPLE_RATE)?;
            self.resampler = Some((ctx, key));
        }
        if let Some(start_us) = start_us {
            self.place(start_us, video_end_us)?;
        }
        // A reference to the decoded samples, with the layout the resampler was made for in both of FFmpeg's
        // fields. ffmpeg-next's clone allocated from the old field, which FFmpeg 6 leaves empty for a mono
        // WAV, so the copy had no samples and libswresample crashed reading them.
        let mut frame = frame::Audio::empty();
        if unsafe { ff::ffi::av_frame_ref(frame.as_mut_ptr(), f.as_ptr()) } < 0 {
            bail!("Cannot read the decoded audio");
        }
        frame.set_channel_layout(layout);
        unsafe {
            let ch_layout = &mut (*frame.as_mut_ptr()).ch_layout;
            ff::ffi::av_channel_layout_uninit(ch_layout);
            ff::ffi::av_channel_layout_from_mask(ch_layout, layout.bits());
        }
        let (ctx, _) = self.resampler.as_mut().unwrap();
        // ffmpeg-next sizes the output like the input, which drops samples when upsampling
        // (22.05 or 44.1 kHz to 48 kHz). Allocate for the converted length instead.
        let capacity = f.samples() * SAMPLE_RATE as usize / f.rate().max(1) as usize + 256;
        let mut out = frame::Audio::new(PCM_FORMAT, capacity, ctx.output().channel_layout);
        ctx.run(&frame, &mut out)?;
        self.write_out(&out)
    }
}

/// Timestamp differences up to this are jitter, not gaps (FFmpeg's aresample min_hard_comp).
const MIN_GAP_US: i64 = 100_000;
/// Audio stepping back further than this restarted its timestamps rather than overlapping.
const MAX_OVERLAP_US: i64 = 1_000_000;

/// Audio may start this late even before the video read so far covers it; later starts past the
/// video are broken timestamps.
const MAX_AUDIO_LEAD_US: i64 = 600_000_000;
/// Muxers keep streams this close together in the file (FFmpeg's default max_interleave_delta).
const MAX_INTERLEAVE_US: i64 = 10_000_000;

fn max_lead_us(video_end_us: i64) -> i64 {
    video_end_us.saturating_add(MAX_INTERLEAVE_US).max(MAX_AUDIO_LEAD_US)
}

const PCM_FORMAT: ff::format::Sample = ff::format::Sample::F32(ff::format::sample::Type::Packed);

fn bytemuck_slice(bytes: &[u8]) -> &[f32] {
    bytemuck::cast_slice(&bytes[..bytes.len() / 4 * 4])
}

/// Turns tightly packed RGBA the way the asset is shown: its rotation, then its mirroring.
pub fn orient(src: &[u8], w: u32, h: u32, asset: &Asset) -> (Vec<u8>, u32, u32) {
    let (mut out, w, h) = rotate(src, w, h, asset.rotation);
    if asset.mirror {
        for row in out.chunks_exact_mut(w as usize * 4) {
            let pixels: Vec<[u8; 4]> = row.as_chunks::<4>().0.iter().rev().copied().collect();
            row.copy_from_slice(pixels.as_flattened());
        }
    }
    (out, w, h)
}

/// Rotates tightly packed RGBA clockwise by 0/90/180/270 degrees.
fn rotate(src: &[u8], w: u32, h: u32, rotation: u32) -> (Vec<u8>, u32, u32) {
    let (w, h) = (w as usize, h as usize);
    let px = |x: usize, y: usize| &src[(y * w + x) * 4..(y * w + x) * 4 + 4];
    match rotation {
        90 | 180 | 270 => {
            let (ow, oh) = if rotation == 180 { (w, h) } else { (h, w) };
            let mut out = Vec::with_capacity(src.len());
            for oy in 0..oh {
                for ox in 0..ow {
                    let (sx, sy) = match rotation {
                        90 => (oy, h - 1 - ox),
                        180 => (w - 1 - ox, h - 1 - oy),
                        _ => (w - 1 - oy, ox),
                    };
                    out.extend_from_slice(px(sx, sy));
                }
            }
            (out, ow as u32, oh as u32)
        }
        _ => (src.to_vec(), w as u32, h as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_preserves_corner_order() {
        let pixels: Vec<u8> = (0..6).flat_map(|i| [i, 0, 0, 255]).collect();
        for (angle, expected) in
            [(90, vec![3, 0, 4, 1, 5, 2]), (180, vec![5, 4, 3, 2, 1, 0]), (270, vec![2, 5, 1, 4, 0, 3])]
        {
            let (out, w, h) = rotate(&pixels, 3, 2, angle);
            assert_eq!(out.as_chunks::<4>().0.iter().map(|p| p[0]).collect::<Vec<_>>(), expected);
            assert_eq!((w, h), if angle == 180 { (3, 2) } else { (2, 3) });
        }
    }

    /// A mirrored quarter turn is a transpose: stored rows become columns, in order.
    #[test]
    fn mirroring_follows_the_rotation() {
        let pixels: Vec<u8> = (0..6).flat_map(|i| [i, 0, 0, 255]).collect();
        let mut asset = Asset {
            id: "x".into(),
            name: "x".into(),
            path: "/x.jpg".into(),
            kind: AssetKind::Image,
            duration_us: 0,
            width: 2,
            height: 3,
            fps: 0.0,
            has_audio: false,
            rotation: 90,
            mirror: true,
            credit: None,
            part: None,
        };
        let (out, w, h) = orient(&pixels, 3, 2, &asset);
        assert_eq!(
            (out.as_chunks::<4>().0.iter().map(|p| p[0]).collect::<Vec<_>>(), w, h),
            (vec![0, 3, 1, 4, 2, 5], 2, 3)
        );
        asset.mirror = false;
        assert_eq!(
            orient(&pixels, 3, 2, &asset).0.as_chunks::<4>().0.iter().map(|p| p[0]).collect::<Vec<_>>(),
            vec![3, 0, 4, 1, 5, 2]
        );
    }

    #[test]
    fn seek_before_keyframe_respects_container_start_offset() {
        let path = std::env::temp_dir().join(format!("nuzky-seek-{}.mp4", uuid::Uuid::new_v4()));
        let encoded = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=96x64:r=25:d=1",
                "-c:v",
                "libx264",
                "-threads",
                "2",
                "-g",
                "10",
                "-bf",
                "3",
                "-output_ts_offset",
                "5",
            ])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        if !encoded {
            eprintln!("ffmpeg CLI with libx264 not available, skipping");
            return;
        }
        let mut decoder = VideoDecoder::open(&path).unwrap();
        assert_eq!(decoder.origin_us, 5_000_000);
        decoder.seek(399_999).unwrap();
        let [covering, next] = decoder.frame_covering(399_999, [None, None]).unwrap();
        assert_eq!(covering.unwrap().0, 360_000);
        assert_eq!(next.unwrap().0, 400_000);
        drop(decoder);
        std::fs::remove_file(path).unwrap();
    }

    /// A frame filled with noise within each sample's bit depth, so a band edge that filtered or rounded
    /// differently would show.
    fn noise(format: Pixel, (w, h): (u32, u32), space: color::Space, range: color::Range) -> frame::Video {
        let mut f = frame::Video::new(format, w, h);
        f.set_color_space(space);
        f.set_color_range(range);
        let component = unsafe { (*ff::ffi::av_pix_fmt_desc_get(format.into())).comp[0] };
        let mut seed = 1u32;
        let mut fill = |bytes: &mut [u8]| {
            let mut next = || {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed
            };
            if component.step >= 2 && component.depth < 16 {
                let mask = ((1u16 << component.depth) - 1) << component.shift;
                for sample in bytes.as_chunks_mut::<2>().0 {
                    *sample = (next() as u16 & mask).to_le_bytes();
                }
            } else {
                bytes.iter_mut().for_each(|byte| *byte = next() as u8);
            }
        };
        for plane in 0..f.planes() {
            fill(f.data_mut(plane));
        }
        if format == Pixel::PAL8 {
            fill(unsafe { std::slice::from_raw_parts_mut((*f.as_mut_ptr()).data[1], 256 * 4) });
        }
        f
    }

    #[test]
    fn rgba_on_several_threads_is_the_one_pass_conversion() {
        use color::{Range, Space};
        use ff::ffi::{SWS_CS_BT2020, SWS_CS_DEFAULT, SWS_CS_ITU601, SWS_CS_ITU709};
        init();
        let full_hd = (1920, 1080);
        // The source, its colour description, the size converted to, and the matrix and range it must be read with.
        let cases = [
            (Pixel::YUV420P, (3840, 2160), Space::Unspecified, Range::MPEG, full_hd, (SWS_CS_ITU709, false)),
            (Pixel::YUV420P, full_hd, Space::BT709, Range::MPEG, full_hd, (SWS_CS_ITU709, false)),
            (Pixel::YUV420P, (1080, 1920), Space::BT470BG, Range::MPEG, (608, 1080), (SWS_CS_ITU601, false)),
            (Pixel::YUV420P, (640, 360), Space::Unspecified, Range::Unspecified, (640, 360), (SWS_CS_ITU601, false)),
            (Pixel::YUV420P, full_hd, Space::BT709, Range::MPEG, (64, 36), (SWS_CS_ITU709, false)),
            (Pixel::YUV420P, (1888, 1062), Space::SMPTE170M, Range::JPEG, (1888, 1062), (SWS_CS_ITU601, true)),
            (Pixel::YUVJ420P, (1001, 667), Space::Unspecified, Range::Unspecified, (1001, 667), (SWS_CS_ITU601, true)),
            (Pixel::YUV420P10LE, (3840, 2160), Space::BT2020NCL, Range::MPEG, full_hd, (SWS_CS_BT2020, false)),
            (Pixel::YUV420P10LE, full_hd, Space::BT2020NCL, Range::MPEG, full_hd, (SWS_CS_BT2020, false)),
            (Pixel::YUV422P, full_hd, Space::BT709, Range::JPEG, full_hd, (SWS_CS_ITU709, true)),
            (Pixel::YUV422P10LE, (3840, 2160), Space::BT709, Range::MPEG, (1024, 576), (SWS_CS_ITU709, false)),
            (Pixel::YUV444P, (1280, 720), Space::SMPTE170M, Range::JPEG, (1280, 720), (SWS_CS_ITU601, true)),
            (Pixel::YUV444P10LE, full_hd, Space::BT2020NCL, Range::JPEG, (960, 540), (SWS_CS_BT2020, true)),
            (Pixel::NV12, full_hd, Space::BT709, Range::MPEG, (1024, 576), (SWS_CS_ITU709, false)),
            (Pixel::NV12, (3840, 2160), Space::BT709, Range::MPEG, full_hd, (SWS_CS_ITU709, false)),
            (Pixel::P010LE, (3840, 2160), Space::BT2020NCL, Range::MPEG, (3840, 2160), (SWS_CS_BT2020, false)),
            (Pixel::YUVA420P, full_hd, Space::Unspecified, Range::MPEG, full_hd, (SWS_CS_ITU709, false)),
            (Pixel::GRAY8, (800, 600), Space::Unspecified, Range::JPEG, (800, 600), (SWS_CS_ITU601, true)),
            (Pixel::RGB24, (1000, 750), Space::Unspecified, Range::Unspecified, (1000, 750), (SWS_CS_DEFAULT, false)),
            (Pixel::RGBA, (2000, 1500), Space::Unspecified, Range::JPEG, (1000, 750), (SWS_CS_DEFAULT, true)),
            (Pixel::BGRA, full_hd, Space::Unspecified, Range::Unspecified, full_hd, (SWS_CS_DEFAULT, false)),
            (Pixel::RGB48BE, (640, 480), Space::Unspecified, Range::Unspecified, (640, 480), (SWS_CS_DEFAULT, false)),
            (Pixel::PAL8, (500, 300), Space::Unspecified, Range::Unspecified, (250, 150), (SWS_CS_DEFAULT, false)),
        ];
        // Each source also converted at Full HD, the size that runs on several threads.
        let cases = cases.into_iter().flat_map(|(format, size, space, range, to, cs)| {
            [to, full_hd].map(|to| (format, size, space, range, to, cs))
        });
        for (format, size, space, range, (w, h), (cs, full)) in cases {
            let case = format!("{format:?} {}x{} {space:?} {range:?} to {w}x{h}", size.0, size.1);
            let f = noise(format, size, space, range);
            let mut one_pass = scaling::Context::get(format, size.0, size.1, Pixel::RGBA, w, h, RGBA_FLAGS).unwrap();
            set_sws_colorspace(&mut one_pass, cs, full, SWS_CS_DEFAULT, true);
            let mut expected = frame::Video::new(Pixel::RGBA, w, h);
            one_pass.run(&f, &mut expected).unwrap();

            let mut scaler = None;
            let got = to_rgba(&mut scaler, &f, 0, w, h).unwrap();
            let threads = scaler.unwrap().1.threads();
            let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
            let threaded = w * h >= 1920 * 1080 && (size != (w, h) || format == Pixel::RGBA) && cores > 1;
            assert_eq!(threads > 1, threaded, "{case} converts on {threads} threads");
            let row = w as usize * 4;
            let differ = (0..h as usize)
                .filter(|&y| got.data[y * row..][..row] != expected.data(0)[y * expected.stride(0)..][..row])
                .count();
            assert_eq!(differ, 0, "{differ} rows of {case} differ");
        }
    }

    #[test]
    #[ignore = "requires the local tmp-test media fixtures"]
    fn covering_frames_match_sequential_decode() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp-test");
        for name in ["wide.mp4", "phone_hevc_vfr.mov"] {
            let mut reference = VideoDecoder::open(&root.join(name)).unwrap();
            let mut times = Vec::new();
            while let Some((t, _)) = reference.next_frame().unwrap() {
                times.push(t);
            }
            let mut decoder = VideoDecoder::open(&root.join(name)).unwrap();
            let mut pair = [None, None];
            for target in [0, 1, 33_333, 500_001, 900_005, 1_000_000, 2_333_333, 10_000_000] {
                pair = decoder.frame_covering(target, pair).unwrap();
                let expected = times.iter().copied().rfind(|t| *t <= target);
                assert_eq!(pair[0].as_ref().map(|f| f.0), expected, "{name} at {target}");
                decoder.seek(target).unwrap();
                pair = decoder.frame_covering(target, [None, None]).unwrap();
                assert_eq!(pair[0].as_ref().map(|f| f.0), expected, "seek {name} at {target}");
            }
        }
    }

    #[test]
    fn seeking_decodes_each_picture_of_hevc_with_wavefronts_on_all_threads() {
        // x265 writes wavefronts (WPP) unless told not to.
        for (x265, slices) in [("log-level=error", true), ("wpp=0:log-level=error", false)] {
            let path = std::env::temp_dir().join(format!("nuzky-wpp-{}.mp4", uuid::Uuid::new_v4()));
            let encoded = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=s=320x240:r=25:d=0.2", "-c:v", "libx265"])
                .args(["-x265-params", x265])
                .arg(&path)
                .status()
                .is_ok_and(|s| s.success());
            if !encoded {
                eprintln!("ffmpeg CLI with libx265 not available, skipping");
                return;
            }
            let active = |decoder: VideoDecoder| unsafe { (*decoder.decoder.as_ptr()).active_thread_type };
            assert_eq!(
                active(VideoDecoder::open_for_seeking(&path).unwrap()) == ff::ffi::FF_THREAD_SLICE,
                slices,
                "{x265}"
            );
            assert_eq!(active(VideoDecoder::open(&path).unwrap()), ff::ffi::FF_THREAD_FRAME, "{x265}");
            std::fs::remove_file(path).unwrap();
        }
    }

    /// FFmpeg 8 image2 returns no packets once seeked, so a .jpg never reached the preview.
    #[test]
    fn still_jpeg_decodes_after_every_seek() {
        let path = std::env::temp_dir().join(format!("nuzky-still-{}.jpg", uuid::Uuid::new_v4()));
        let encoded = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=s=96x64", "-frames:v", "1"])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        if !encoded {
            eprintln!("ffmpeg CLI not available, skipping");
            return;
        }
        let mut decoder = VideoDecoder::open(&path).unwrap();
        for _ in 0..3 {
            decoder.seek(0).unwrap();
            let [covering, next] = decoder.frame_covering(0, [None, None]).unwrap();
            assert_eq!(covering.map(|(t, f)| (t, f.width(), f.height())), Some((0, 96, 64)));
            assert!(next.is_none());
        }
        drop(decoder);
        std::fs::remove_file(path).unwrap();
    }

    /// A JPEG whose first frame header says 1x1 still decodes to the 6000x5000 frame after it, plain or JPEG-LS.
    #[test]
    fn a_picture_measures_as_ffmpeg_decodes_it_and_never_past_the_limit() {
        let dir = std::env::temp_dir().join(format!("nuzky-picture-size-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let make = |name: &str, codec: &str, size: &str| {
            let path = dir.join(name);
            let out = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-f", "lavfi", "-i", &format!("color=c=red:s={size}"), "-frames:v", "1"])
                .args(["-c:v", codec])
                .arg(&path)
                .output()
                .unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            path
        };
        let tiny_first = |name: &str, real: &Path| {
            let sof = [0xff, 0xc0, 0, 17, 8, 0, 1, 0, 1, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1];
            let path = dir.join(name);
            std::fs::write(&path, [&[0xff, 0xd8][..], &sof, &std::fs::read(real).unwrap()[2..]].concat()).unwrap();
            path
        };
        let (limit, room) = (25_000_000, 32_000_000);
        let small = make("small.png", "png", "64x48");
        assert_eq!(picture_size(&small, limit, || false).unwrap(), (64, 48));
        let big = make("big.png", "png", "6000x5000");
        assert!(picture_size(&big, limit, || false).is_err());
        assert_eq!(picture_size(&big, room, || false).unwrap(), (6000, 5000));
        for (name, codec) in [("lying.jpg", "mjpeg"), ("lying-ls.jpg", "jpegls")] {
            let lying = tiny_first(name, &make(&format!("real-{name}"), codec, "6000x5000"));
            assert!(picture_size(&lying, limit, || false).is_err(), "{name}");
            // FFmpeg 6.1 does not decode a JPEG-LS frame after a baseline header at all; 8.1 does.
            if codec == "mjpeg" {
                assert_eq!(picture_size(&lying, room, || false).unwrap(), (6000, 5000), "{name}");
            }
        }
        assert!(picture_size(&small, limit, || true).unwrap_err().to_string().starts_with("CANCELLED"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn decode_size_keeps_aspect_and_never_upscales() {
        assert_eq!(decode_size((1920, 1080), 0, (960.0, 540.0), 8192), (960, 540));
        assert_eq!(decode_size((1920, 1080), 0, (4000.0, 3000.0), 8192), (1920, 1080));
        // Rotated portrait source shown 540 wide, 960 tall.
        assert_eq!(decode_size((1920, 1080), 90, (540.0, 960.0), 8192), (960, 540));
        // A zoomed panorama stays inside the GPU texture limit and keeps its aspect.
        assert_eq!(decode_size((20000, 3000), 0, (12800.0, 1920.0), 8192), (8192, 1230));
        assert_eq!(decode_size((3000, 20000), 90, (12800.0, 1920.0), 8191), (1230, 8190));
    }

    /// Recorders that drop audio leave timestamp gaps; joined files can step back and overlap.
    #[test]
    fn pcm_places_samples_by_timestamp() {
        let dir = std::env::temp_dir().join(format!("nuzky-pcm-gap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ffmpeg = |args: &[&str], out: &Path| {
            std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=red:s=64x64:r=25", "-f", "lavfi", "-i"])
                .args(args)
                .args(["-c:v", "libx264", "-threads", "2", "-c:a", "aac"])
                .arg(out)
                .status()
                .is_ok_and(|s| s.success())
        };
        let (gap, a, b) = (dir.join("gap.mkv"), dir.join("a.ts"), dir.join("b.ts"));
        // A 1 s tone, 1 s without audio packets, another 1 s tone.
        if !ffmpeg(&["sine=f=440:r=48000:d=2,asetpts='PTS+if(gte(T,1),1/TB,0)'", "-t", "3"], &gap)
            || !ffmpeg(&["sine=f=440:r=48000:d=1", "-t", "1"], &a)
            || !ffmpeg(&["sine=f=880:r=48000:d=1", "-t", "1", "-output_ts_offset", "0.7"], &b)
        {
            eprintln!("ffmpeg CLI with libx264 not available, skipping");
            return;
        }
        let pcm = |path: &Path| {
            let (out, peaks) = (path.with_extension("f32"), path.with_extension("peaks"));
            extract_pcm_with_peaks(path, &out, &peaks, |_| Ok(())).unwrap();
            // Peaks written during the extraction are those of the finished samples, filled gaps and dropped
            // overlaps included.
            let finished = crate::audio::peaks(&crate::audio::Pcm::open(&out).unwrap(), 50);
            assert_eq!(std::fs::read(&peaks).unwrap(), finished, "{}", path.display());
            let samples: Vec<f32> = bytemuck::cast_slice(&std::fs::read(&out).unwrap()).to_vec();
            // Peak level between two times in seconds.
            move |from: f64, to: f64| {
                let end = ((to * 96_000.0) as usize).min(samples.len());
                samples[((from * 96_000.0) as usize).min(end)..end].iter().fold(0f32, |m, s| m.max(s.abs()))
            }
        };
        let peak = pcm(&gap);
        assert!(peak(0.1, 0.9) > 0.05 && peak(2.1, 2.9) > 0.05, "tones");
        assert!(peak(1.1, 1.9) < 0.001, "the gap is silent");
        assert!(peak(2.95, f64::MAX) > 0.05 && peak(3.05, f64::MAX) == 0.0, "the second tone ends at 3 s");
        // The second file's audio starts about 0.4 s before the first one's ends. Appending it would
        // give 2 s; placing it by time drops the overlapping start and leaves no hole.
        let joined = dir.join("overlap.ts");
        std::fs::write(&joined, [std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()].concat()).unwrap();
        let peak = pcm(&joined);
        let length = (0..).map(|i| i as f64 * 0.05).find(|&t| peak(t, f64::MAX) == 0.0).unwrap();
        assert!((1.55..1.75).contains(&length), "length {length} s");
        assert!((0..30).all(|i| peak(i as f64 * 0.05, i as f64 * 0.05 + 0.05) > 0.05), "no hole");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Audio without video keeps a real pause late in a long recording.
    #[test]
    fn pcm_keeps_late_pauses_of_long_audio() {
        let dir = std::env::temp_dir().join(format!("nuzky-pcm-late-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ffmpeg = |filter: &str, out: &Path| {
            std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-f", "lavfi", "-i", filter, "-c:a", "aac", "-b:a", "48k"])
                .arg(out)
                .status()
                .is_ok_and(|s| s.success())
        };
        let late = dir.join("late.mka");
        // 700 s of tone with 1 s without audio packets at 660 s.
        if !ffmpeg("sine=f=440:r=48000:d=700,asetpts='PTS+if(gte(T,660),1/TB,0)'", &late) {
            eprintln!("ffmpeg CLI not available, skipping");
            return;
        }
        let seconds = |path: &Path| {
            extract_pcm(path, &path.with_extension("f32"), |_| Ok(())).unwrap() as f64 / SAMPLE_RATE as f64
        };
        assert!((seconds(&late) - 701.0).abs() < 0.2, "the pause at 660 s is kept");
        let samples: Vec<f32> = bytemuck::cast_slice(&std::fs::read(late.with_extension("f32")).unwrap()).to_vec();
        let peak = |from: f64, to: f64| {
            samples[(from * 96_000.0) as usize..(to * 96_000.0) as usize].iter().fold(0f32, |m, s| m.max(s.abs()))
        };
        assert!(
            peak(659.0, 659.9) > 0.05 && peak(660.1, 660.9) < 0.001 && peak(661.1, 662.0) > 0.05,
            "tone, pause, tone"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A jump of 200 million s overflowed the old bound, which then wrote silence until the disk
    /// was full. FFmpeg smooths such jumps when encoding, so the sink gets them directly.
    #[test]
    fn an_absurd_timestamp_jump_writes_no_silence() {
        let path = std::env::temp_dir().join(format!("nuzky-pcm-jump-{}", uuid::Uuid::new_v4()));
        let writer = BufWriter::new(File::create(&path).unwrap());
        let mut sink = PcmSink {
            writer,
            written: 0,
            resampler: None,
            mono: false,
            shift: None,
            skip: 0,
            audio: 0,
            gaps: 0,
            peaks: None,
        };
        sink.place(0, None).unwrap();
        // One second of audio played.
        (sink.written, sink.audio) = (48_000, 48_000);
        sink.place(200_000_000 * 1_000_000, None).unwrap();
        assert_eq!(sink.written, 48_000, "no silence for the jump");
        // The audio after it continues where the audio before it ended.
        sink.place(200_000_000 * 1_000_000, None).unwrap();
        assert_eq!(sink.written, 48_000);
        drop(sink);
        std::fs::remove_file(path).ok();
    }

    /// Upsampled sources used to lose their tail; mono sources came out 3 dB quieter.
    #[test]
    fn pcm_keeps_full_length_and_level_when_upsampling() {
        let dir = std::env::temp_dir().join(format!("nuzky-pcm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (rate, layout) in [(22_050, "mono"), (44_100, "stereo")] {
            let src = dir.join(format!("tone_{rate}.wav"));
            let ok = std::process::Command::new("ffmpeg")
                .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
                .arg(format!("sine=frequency=440:sample_rate={rate}:duration=3"))
                .args(["-af", &format!("aformat=channel_layouts={layout}"), "-ar", &rate.to_string()])
                .arg(&src)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                eprintln!("ffmpeg CLI not available, skipping");
                return;
            }
            let out = dir.join(format!("tone_{rate}.f32"));
            let frames = extract_pcm(&src, &out, |_| Ok(())).unwrap();
            assert!((frames as i64 - 3 * SAMPLE_RATE as i64).abs() < 2_000, "{rate} Hz gave {frames} frames");
            let bytes = std::fs::read(&out).unwrap();
            let samples: &[f32] = bytemuck::cast_slice(&bytes);
            let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
            // FFmpeg's sine source peaks at 1/8 of full scale; its own stereo upmix is -3 dB.
            let expected = if layout == "mono" { 0.125 } else { 0.125 * std::f32::consts::FRAC_1_SQRT_2 };
            assert!((peak - expected).abs() < 0.01, "{rate} Hz {layout} peak {peak}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}

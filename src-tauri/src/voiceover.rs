//! Record voiceover: the microphone into a WAV file in Nuzky's folder while the timeline plays from the playhead,
//! added there as one undo step. The microphone is open only while the user has the voiceover panel open, and
//! agents never record.
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use nuzky_engine::edit::{EditCmd, new_id};
use nuzky_engine::model::{Asset, AssetKind};
use nuzky_session::Expect;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{AppState, CmdResult, Snapshot};

/// The microphone as the meter shows it, 20 times a second.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MicLevel {
    /// The loudest sample of the last 50 ms in dBFS, -60 at the least.
    pub peak_db: f32,
    pub warning: Option<MicWarning>,
    /// How long the take being recorded is, in microseconds; null while nothing records.
    pub recorded_us: Option<i64>,
}

/// What the creator should change before or while recording.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MicWarning {
    /// The voice reached -1 dBFS in the last 2 s, so it may distort.
    Loud,
    /// Something is said, but its loudest moments stay under -24 dBFS.
    Quiet,
    /// Nothing at all comes from the microphone, as when the system does not let Nuzky use it.
    Silent,
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceoverState {
    /// The microphone listened to, or null while it is closed.
    pub microphone: Option<String>,
    /// Where on the timeline the take being recorded starts.
    pub recording_from_us: Option<i64>,
    /// A take the project refused, as while the AI edits, kept until it is added or deleted.
    pub unsaved: bool,
}

const BLOCKS_PER_SECOND: usize = 20;
/// How far back the quiet check looks.
const WINDOW_BLOCKS: usize = 3 * BLOCKS_PER_SECOND;
const LOUD: f32 = 0.891; // -1 dBFS
const LOUD_HOLD_BLOCKS: u32 = 2 * BLOCKS_PER_SECOND as u32;
const QUIET_DB: f32 = -24.0;
/// Speech rises and falls between syllables and pauses; steady room noise does not.
const SPEECH_RANGE_DB: f32 = 12.0;
const SILENT_BLOCKS: u32 = 2 * BLOCKS_PER_SECOND as u32;
const FLOOR_DB: f32 = -60.0;

fn db(v: f32) -> f32 {
    20.0 * v.max(1e-7).log10()
}

/// Turns 50 ms blocks of the microphone's sound into the meter's level and warning.
#[derive(Default)]
pub(crate) struct Meter {
    /// Peak and RMS in dB of the last blocks, oldest first.
    window: VecDeque<(f32, f32)>,
    loud_for: u32,
    silent_for: u32,
}

impl Meter {
    pub(crate) fn push(&mut self, block: &[f32]) -> (f32, Option<MicWarning>) {
        let peak = block.iter().fold(0f32, |m, x| m.max(x.abs()));
        let rms = (block.iter().map(|x| x * x).sum::<f32>() / block.len().max(1) as f32).sqrt();
        self.loud_for = if peak >= LOUD { LOUD_HOLD_BLOCKS } else { self.loud_for.saturating_sub(1) };
        self.silent_for = if peak < 1e-6 { self.silent_for + 1 } else { 0 };
        self.window.push_back((db(peak), db(rms)));
        if self.window.len() > WINDOW_BLOCKS {
            self.window.pop_front();
        }
        let loudest = self.window.iter().map(|b| b.0).fold(f32::MIN, f32::max);
        let (high, low) = self.window.iter().fold((f32::MIN, f32::MAX), |(h, l), b| (h.max(b.1), l.min(b.1)));
        let warning = if self.loud_for > 0 {
            Some(MicWarning::Loud)
        } else if self.silent_for >= SILENT_BLOCKS {
            Some(MicWarning::Silent)
        } else if self.window.len() >= BLOCKS_PER_SECOND && loudest < QUIET_DB && high - low >= SPEECH_RANGE_DB {
            Some(MicWarning::Quiet)
        } else {
            None
        };
        (db(peak).max(FLOOR_DB), warning)
    }
}

/// A take being written: 16-bit mono PCM at the microphone's rate.
struct Take {
    file: BufWriter<File>,
    path: PathBuf,
    rate: u32,
    frames: u64,
    start_us: i64,
    /// The project it was recorded in, the only one it is added to.
    project: PathBuf,
    failed: Option<String>,
}

fn wav_header(rate: u32, frames: u64) -> [u8; 44] {
    let data = (frames * 2).min(u32::MAX as u64 - 36) as u32;
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data).to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    h[24..28].copy_from_slice(&rate.to_le_bytes());
    h[28..32].copy_from_slice(&(rate * 2).to_le_bytes());
    h[32..34].copy_from_slice(&2u16.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data.to_le_bytes());
    h
}

impl Take {
    fn create(start_us: i64, rate: u32, project: PathBuf) -> CmdResult<Self> {
        let dir = crate::store::data_dir().join("recordings");
        std::fs::create_dir_all(&dir).map_err(crate::err)?;
        let path = dir.join(format!("voiceover-{}.wav", new_id()));
        let mut file = BufWriter::new(File::create_new(&path).map_err(crate::err)?);
        file.write_all(&wav_header(rate, 0)).map_err(crate::err)?;
        Ok(Self { file, path, rate, frames: 0, start_us, project, failed: None })
    }

    fn write(&mut self, samples: &[f32]) {
        if self.failed.is_some() {
            return;
        }
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for s in samples {
            bytes.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
        }
        match self.file.write_all(&bytes) {
            Ok(()) => self.frames += samples.len() as u64,
            Err(e) => {
                self.failed =
                    Some(format!("Nuzky could not write the voiceover ({e}). Free some disk space and record again."))
            }
        }
    }

    fn recorded_us(&self) -> i64 {
        (self.frames * 1_000_000 / self.rate as u64) as i64
    }

    /// Writes the sizes into the header, so the file is whole.
    fn finish(mut self) -> Result<(PathBuf, i64), (PathBuf, String)> {
        if let Some(error) = self.failed.take() {
            return Err((self.path, error));
        }
        let header = wav_header(self.rate, self.frames);
        let done = self.file.flush().and_then(|_| {
            let file = self.file.get_mut();
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&header)?;
            file.sync_all()
        });
        let length = self.recorded_us();
        match done {
            Ok(()) => Ok((self.path, length)),
            Err(e) => Err((self.path, format!("Nuzky could not finish the voiceover ({e})."))),
        }
    }
}

/// The open microphone: a thread that captures it and one that meters it and writes the take.
struct Mic {
    name: String,
    rate: u32,
    stop: Arc<AtomicBool>,
    take: Arc<Mutex<Option<Take>>>,
    threads: Vec<JoinHandle<()>>,
}

impl Mic {
    fn close(mut self) -> Option<Take> {
        self.stop.store(true, Ordering::Release);
        for thread in self.threads.drain(..) {
            thread.join().ok();
        }
        self.take.lock().unwrap().take()
    }
}

/// A finished take the project refused, kept until it is added or deleted.
struct Unsaved {
    asset: Asset,
    start_us: i64,
    project: PathBuf,
}

#[derive(Default)]
pub struct Voiceover {
    mic: Mutex<Option<Mic>>,
    unsaved: Mutex<Option<Unsaved>>,
}

impl Voiceover {
    pub fn recording(&self) -> bool {
        let mic = self.mic.lock().unwrap();
        mic.as_ref().is_some_and(|m| m.take.lock().unwrap().is_some()) || self.unsaved.lock().unwrap().is_some()
    }

    /// Closes the microphone, as quitting does, and deletes a take that is not in the project.
    pub fn quit(&self) {
        if let Some(take) = self.mic.lock().unwrap().take().and_then(Mic::close) {
            let path = take.path.clone();
            drop(take);
            std::fs::remove_file(path).ok();
        }
        if let Some(unsaved) = self.unsaved.lock().unwrap().take() {
            std::fs::remove_file(unsaved.asset.path).ok();
        }
    }
}

/// Samples, mixed to mono, from the capture thread to the meter.
type Sink = rtrb::Producer<f32>;

/// Opens the system's default microphone in a thread of its own, which keeps the stream until `stop`.
fn capture(app: AppHandle, mut sink: Sink, stop: Arc<AtomicBool>) -> CmdResult<(String, u32, JoinHandle<()>)> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("NUZKY_FAKE_MICROPHONE") {
        let thread = std::thread::spawn(move || fake_capture(PathBuf::from(path), sink, stop));
        return Ok(("Test microphone".into(), FAKE_RATE, thread));
    }
    #[cfg(target_os = "macos")]
    {
        // Until the user answers, the system holds every call to the microphone, so Nuzky asks first and waits.
        let asked = mac::undecided();
        if asked {
            app.emit("voiceover-asking", ()).ok();
            mac::ask();
            if mac::undecided() {
                return Err(UNANSWERED.into());
            }
        }
        if mac::denied() {
            // Settings open for a refusal the user made before, not for the one they just clicked.
            if !asked {
                std::process::Command::new("open")
                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
                    .spawn()
                    .ok();
            }
            return Err(PERMISSION.into());
        }
    }
    let (opened, result) = sync_channel(1);
    let stopped = stop.clone();
    let thread = std::thread::Builder::new()
        .name("mic-capture".into())
        .spawn(move || {
            use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
            let open = || -> CmdResult<(String, u32, cpal::Stream)> {
                let device = cpal::default_host().default_input_device().ok_or(MISSING)?;
                let config = device.default_input_config().map_err(|e| open_error(&e))?;
                let (rate, channels) = (config.sample_rate(), config.channels() as usize);
                let lost = move |e: cpal::Error| {
                    log::warn!("Microphone stream error: {e}");
                    if matches!(e.kind(), cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated) {
                        app.emit("voiceover-microphone-lost", ()).ok();
                    }
                };
                let mut mono = Vec::new();
                let mut push = move |data: &[f32]| {
                    mono.clear();
                    mono.extend(data.chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32));
                    let n = mono.len().min(sink.slots());
                    if let Ok(chunk) = sink.write_chunk_uninit(n) {
                        chunk.fill_from_iter(mono.iter().copied());
                    }
                };
                let stream_config = config.config();
                let stream = match config.sample_format() {
                    cpal::SampleFormat::F32 => {
                        device.build_input_stream::<f32, _, _>(stream_config, move |d, _| push(d), lost, None)
                    }
                    cpal::SampleFormat::I16 => device.build_input_stream::<i16, _, _>(
                        stream_config,
                        move |d, _| push(&d.iter().map(|&s| s as f32 / 32768.0).collect::<Vec<_>>()),
                        lost,
                        None,
                    ),
                    cpal::SampleFormat::I32 => device.build_input_stream::<i32, _, _>(
                        stream_config,
                        move |d, _| push(&d.iter().map(|&s| s as f32 / 2_147_483_648.0).collect::<Vec<_>>()),
                        lost,
                        None,
                    ),
                    cpal::SampleFormat::U16 => device.build_input_stream::<u16, _, _>(
                        stream_config,
                        move |d, _| push(&d.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).collect::<Vec<_>>()),
                        lost,
                        None,
                    ),
                    other => {
                        return Err(format!("The microphone gives sound as {other}, which Nuzky cannot record yet."));
                    }
                }
                .map_err(|e| open_error(&e))?;
                stream.play().map_err(|e| open_error(&e))?;
                Ok((device.to_string(), rate, stream))
            };
            match open() {
                Ok((name, rate, stream)) => {
                    opened.send(Ok((name, rate))).ok();
                    while !stopped.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    drop(stream);
                }
                Err(error) => {
                    opened.send(Err(error)).ok();
                }
            }
        })
        .map_err(crate::err)?;
    match result.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok((name, rate))) => Ok((name, rate, thread)),
        Ok(Err(error)) => {
            thread.join().ok();
            Err(error)
        }
        Err(_) => {
            stop.store(true, Ordering::Release);
            Err("The microphone did not answer. Check it in the system's sound settings and try again.".into())
        }
    }
}

const MISSING: &str = "MIC_MISSING: Nuzky finds no microphone. Connect one, or choose one in the system's sound settings, then try again.";
#[cfg(target_os = "macos")]
const UNANSWERED: &str =
    "MIC_PERMISSION: macOS is still asking whether Nuzky may use the microphone. Answer it, then try again.";
#[cfg(target_os = "macos")]
const PERMISSION: &str =
    "MIC_PERMISSION: Allow Nuzky in System Settings › Privacy & Security › Microphone, then open the voiceover again.";

fn open_error(e: &cpal::Error) -> String {
    match e.kind() {
        cpal::ErrorKind::PermissionDenied if cfg!(windows) => {
            "MIC_PERMISSION: Let desktop apps use the microphone in Windows Settings › Privacy & security › Microphone, then try again.".into()
        }
        cpal::ErrorKind::PermissionDenied => {
            "MIC_PERMISSION: The system does not let Nuzky use the microphone. Allow it in the system's privacy settings, then try again.".into()
        }
        cpal::ErrorKind::DeviceBusy => "Another app is using the microphone. Close it, then try again.".into(),
        cpal::ErrorKind::DeviceNotAvailable => MISSING.into(),
        _ => format!("Nuzky could not open the microphone ({e}). Check it in the system's sound settings and try again."),
    }
}

#[cfg(debug_assertions)]
const FAKE_RATE: u32 = 48_000;

/// The UI flows' microphone: raw 32-bit float mono samples at 48 kHz from a file, played in real time and over again.
/// A new file put in its place is heard from its start, so a flow can change what the microphone hears.
#[cfg(debug_assertions)]
fn fake_capture(path: PathBuf, mut sink: Sink, stop: Arc<AtomicBool>) {
    let read = |path: &PathBuf| -> (Vec<f32>, Option<std::time::SystemTime>) {
        let bytes = std::fs::read(path).unwrap_or_default();
        let samples = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        (samples, std::fs::metadata(path).and_then(|m| m.modified()).ok())
    };
    let (mut samples, mut modified) = read(&path);
    let mut at = 0;
    let chunk = FAKE_RATE as usize / 100;
    let started = std::time::Instant::now();
    let mut sent = 0u64;
    while !stop.load(Ordering::Acquire) {
        let now = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if now != modified {
            (samples, modified) = read(&path);
            at = 0;
        }
        let due = started.elapsed().as_micros() as u64 * FAKE_RATE as u64 / 1_000_000;
        while sent + chunk as u64 <= due {
            let block: Vec<f32> =
                (0..chunk).map(|i| if samples.is_empty() { 0.0 } else { samples[(at + i) % samples.len()] }).collect();
            at = if samples.is_empty() { 0 } else { (at + chunk) % samples.len() };
            if let Ok(c) = sink.write_chunk_uninit(chunk.min(sink.slots())) {
                c.fill_from_iter(block);
            }
            sent += chunk as u64;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Meters what the microphone hears and writes it into the take while one records.
fn meter(
    app: AppHandle,
    mut source: rtrb::Consumer<f32>,
    rate: u32,
    take: Arc<Mutex<Option<Take>>>,
    stop: Arc<AtomicBool>,
) {
    let block = (rate as usize / BLOCKS_PER_SECOND).max(1);
    let mut meter = Meter::default();
    let mut pending: Vec<f32> = Vec::with_capacity(block);
    let mut buf = vec![0f32; 4096];
    while !stop.load(Ordering::Acquire) {
        let n = source.slots().min(buf.len());
        if n == 0 {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        if let Ok(chunk) = source.read_chunk(n) {
            let (a, b) = chunk.as_slices();
            buf[..a.len()].copy_from_slice(a);
            buf[a.len()..a.len() + b.len()].copy_from_slice(b);
            chunk.commit_all();
        }
        let got = &buf[..n];
        let recorded_us = {
            let mut take = take.lock().unwrap();
            take.as_mut().map(|t| {
                t.write(got);
                t.recorded_us()
            })
        };
        for &s in got {
            pending.push(s);
            if pending.len() == block {
                let (peak_db, warning) = meter.push(&pending);
                pending.clear();
                app.emit("voiceover-level", MicLevel { peak_db, warning, recorded_us }).ok();
            }
        }
    }
}

/// Opens the microphone for the meter; recording starts with `voiceover_record_start`. Off the main thread, as the
/// system may take a moment to start the microphone.
#[tauri::command]
pub async fn voiceover_open(app: AppHandle) -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(move || open(app)).await.map_err(crate::err)?
}

fn open(app: AppHandle) -> CmdResult<String> {
    let state = app.state::<AppState>();
    if let Some(mic) = state.voiceover.mic.lock().unwrap().as_ref() {
        return Ok(mic.name.clone());
    }
    // Opening may wait minutes for the user's answer to macOS, so nothing the main thread takes, such as closing the
    // window asking whether a voiceover records, is locked meanwhile.
    let (sink, source) = rtrb::RingBuffer::<f32>::new(192_000 * 2);
    let stop = Arc::new(AtomicBool::new(false));
    let (name, rate, capturing) = capture(app.clone(), sink, stop.clone())?;
    let take = Arc::new(Mutex::new(None));
    let (a, t, s) = (app.clone(), take.clone(), stop.clone());
    let metering = std::thread::Builder::new()
        .name("mic-meter".into())
        .spawn(move || meter(a, source, rate, t, s))
        .map_err(crate::err)?;
    let mic = Mic { name: name.clone(), rate, stop, take, threads: vec![capturing, metering] };
    let mut current = state.voiceover.mic.lock().unwrap();
    if let Some(open) = current.as_ref() {
        let name = open.name.clone();
        drop(current);
        mic.close();
        return Ok(name);
    }
    log::info!(target: "action", "voiceover microphone open");
    *current = Some(mic);
    Ok(name)
}

/// Closes the microphone. A take still recording is deleted; stop it with `voiceover_record_stop` to keep it.
#[tauri::command]
pub fn voiceover_close(app: AppHandle) {
    let state = app.state::<AppState>();
    let mic = state.voiceover.mic.lock().unwrap().take();
    if let Some(take) = mic.and_then(Mic::close) {
        let path = take.path.clone();
        drop(take);
        std::fs::remove_file(path).ok();
    }
}

#[tauri::command]
pub fn voiceover_state(app: AppHandle) -> VoiceoverState {
    let state = app.state::<AppState>();
    let mic = state.voiceover.mic.lock().unwrap();
    VoiceoverState {
        microphone: mic.as_ref().map(|m| m.name.clone()),
        recording_from_us: mic.as_ref().and_then(|m| m.take.lock().unwrap().as_ref().map(|t| t.start_us)),
        unsaved: state.voiceover.unsaved.lock().unwrap().is_some(),
    }
}

/// Starts writing what the microphone hears, to go on the timeline at `start_us`.
#[tauri::command]
pub fn voiceover_record_start(app: AppHandle, start_us: i64) -> CmdResult<()> {
    let state = app.state::<AppState>();
    if state.voiceover.unsaved.lock().unwrap().is_some() {
        return Err("VOICEOVER_BUSY: Add or discard the last voiceover first.".into());
    }
    let mic = state.voiceover.mic.lock().unwrap();
    let mic = mic.as_ref().ok_or("VOICEOVER_BUSY: Open the voiceover again; its microphone was closed.")?;
    let mut take = mic.take.lock().unwrap();
    if take.is_some() {
        return Err("VOICEOVER_BUSY: A voiceover is already being recorded.".into());
    }
    let project = state.session.lock().unwrap().path.clone();
    *take = Some(Take::create(start_us.max(0), mic.rate, project)?);
    log::info!(target: "action", "voiceover record");
    Ok(())
}

/// "Voiceover", then "Voiceover 2" and on, after those the project has.
fn take_name(assets: &[Asset]) -> String {
    let taken = |n: usize| {
        let name = if n == 1 { "Voiceover".to_owned() } else { format!("Voiceover {n}") };
        assets.iter().any(|a| a.name == name).then_some(()).ok_or(name)
    };
    (1..).find_map(|n| taken(n).err()).unwrap_or_default()
}

/// Stops the take. `keep` puts it on the timeline where it started, with its file in Media, as one undo step;
/// otherwise it is deleted. A take the project refuses, as while the AI edits, is kept for another stop.
#[tauri::command]
pub async fn voiceover_record_stop(
    app: AppHandle,
    keep: bool,
    expected_epoch: Option<String>,
) -> CmdResult<Option<Snapshot>> {
    let take = {
        let state = app.state::<AppState>();
        let mic = state.voiceover.mic.lock().unwrap();
        mic.as_ref().and_then(|m| m.take.lock().unwrap().take())
    };
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let unsaved = match take {
            Some(take) => {
                let (start_us, project) = (take.start_us, take.project.clone());
                let (path, length) = take.finish().map_err(|(path, error)| {
                    std::fs::remove_file(path).ok();
                    error
                })?;
                if !keep || length < 200_000 {
                    std::fs::remove_file(&path).ok();
                    return if keep { Err("Nothing was recorded. Record a little longer.".into()) } else { Ok(None) };
                }
                let mut asset = nuzky_engine::media::probe(&path, new_id()).map_err(|e| {
                    std::fs::remove_file(&path).ok();
                    format!("Nuzky could not read the voiceover it recorded ({e:#})")
                })?;
                if asset.kind != AssetKind::Audio {
                    std::fs::remove_file(&path).ok();
                    return Err("Nuzky could not read the voiceover it recorded".into());
                }
                asset.name = take_name(&state.project()?.assets);
                Unsaved { asset, start_us, project }
            }
            None => match state.voiceover.unsaved.lock().unwrap().take() {
                Some(unsaved) => unsaved,
                None => return Ok(None),
            },
        };
        if !keep {
            std::fs::remove_file(&unsaved.asset.path).ok();
            return Ok(None);
        }
        let elsewhere = state.session.lock().unwrap().path != unsaved.project;
        if elsewhere {
            let name = unsaved.project.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            *state.voiceover.unsaved.lock().unwrap() = Some(unsaved);
            return Err(format!(
                "VOICEOVER_BUSY: The last voiceover was recorded in another project, {name}. Open it to add the voiceover, or discard it."
            ));
        }
        let edits = vec![
            EditCmd::AddAssets { assets: vec![unsaved.asset.clone()] },
            EditCmd::AddClip { asset_id: unsaved.asset.id.clone(), start_us: Some(unsaved.start_us), track_id: None },
        ];
        match state.apply_batch(edits, None, Expect::default(), expected_epoch.as_deref()) {
            Ok(snap) => {
                crate::jobs::prepare_media(&state, &snap.project);
                Ok(Some(snap))
            }
            Err(error) => {
                *state.voiceover.unsaved.lock().unwrap() = Some(unsaved);
                Err(error)
            }
        }
    })
    .await
    .map_err(crate::err)?
}

#[cfg(target_os = "macos")]
mod mac {
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, Bool};
    use objc2_foundation::NSString;

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {
        static AVMediaTypeAudio: &'static NSString;
    }

    /// AVAuthorizationStatus: 0 not determined, 1 restricted, 2 denied, 3 authorized.
    fn status() -> isize {
        let Some(class) = AnyClass::get(c"AVCaptureDevice") else { return 3 };
        unsafe { msg_send![class, authorizationStatusForMediaType: AVMediaTypeAudio] }
    }

    /// The user has not been asked yet.
    pub fn undecided() -> bool {
        status() == 0
    }

    /// The user or a policy refused Nuzky the microphone.
    pub fn denied() -> bool {
        matches!(status(), 1 | 2)
    }

    /// Shows macOS's question and waits for the answer, at most two minutes.
    pub fn ask() {
        let Some(class) = AnyClass::get(c"AVCaptureDevice") else { return };
        let (answered, answer) = std::sync::mpsc::channel();
        let done = RcBlock::new(move |_: Bool| {
            answered.send(()).ok();
        });
        unsafe {
            let _: () = msg_send![class, requestAccessForMediaType: AVMediaTypeAudio, completionHandler: &*done];
        }
        answer.recv_timeout(Duration::from_secs(120)).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 50 ms blocks of a voice whose syllables reach `peak`, with quiet between them, as speech has.
    fn speech(peak: f32, seconds: usize) -> Vec<Vec<f32>> {
        (0..seconds * BLOCKS_PER_SECOND)
            .map(|i| {
                let level = if i % 4 == 3 { peak * 0.05 } else { peak };
                (0..2400).map(|n| level * (n as f32 * 0.07).sin()).collect()
            })
            .collect()
    }

    fn last(meter: &mut Meter, blocks: &[Vec<f32>]) -> (f32, Option<MicWarning>) {
        blocks.iter().map(|b| meter.push(b)).last().unwrap()
    }

    #[test]
    fn a_voice_at_a_good_level_gets_no_warning() {
        let mut meter = Meter::default();
        let levels: Vec<_> = speech(0.3, 4).iter().map(|b| meter.push(b)).collect();
        let loudest = levels.iter().map(|l| l.0).fold(f32::MIN, f32::max);
        assert!((loudest - db(0.3)).abs() < 0.5, "{loudest}");
        assert!(levels.iter().all(|l| l.1.is_none()), "{levels:?}");
    }

    #[test]
    fn a_quiet_voice_is_too_quiet_but_room_noise_and_a_pause_are_not() {
        let mut meter = Meter::default();
        // Under a second, nothing is said about it yet.
        assert_eq!(last(&mut meter, &speech(0.02, 1)[..BLOCKS_PER_SECOND - 1]).1, None);
        assert_eq!(last(&mut meter, &speech(0.02, 3)).1, Some(MicWarning::Quiet));
        // Steady noise at -50 dBFS, as a quiet room between sentences: not a voice.
        let mut meter = Meter::default();
        let noise: Vec<Vec<f32>> =
            (0..60).map(|_| (0..2400).map(|n| 0.003 * (n as f32 * 1.3).sin()).collect()).collect();
        assert_eq!(last(&mut meter, &noise).1, None);
        // A good voice, then a pause: still fine while the voice is in the last 3 s.
        let mut meter = Meter::default();
        last(&mut meter, &speech(0.3, 3));
        assert_eq!(last(&mut meter, &noise[..40]).1, None);
    }

    #[test]
    fn a_voice_reaching_minus_one_dbfs_is_too_loud_for_two_seconds() {
        let mut meter = Meter::default();
        last(&mut meter, &speech(0.3, 1));
        assert_eq!(meter.push(&vec![0.95; 2400]).1, Some(MicWarning::Loud));
        let good = speech(0.3, 3);
        assert_eq!(last(&mut meter, &good[..2 * BLOCKS_PER_SECOND - 1]).1, Some(MicWarning::Loud));
        assert_eq!(meter.push(&good[0]).1, None);
    }

    #[test]
    fn a_microphone_giving_nothing_at_all_is_silent_after_two_seconds() {
        let mut meter = Meter::default();
        let zeros = vec![vec![0.0; 2400]; 2 * BLOCKS_PER_SECOND];
        assert_eq!(last(&mut meter, &zeros[1..]).1, None);
        let (peak, warning) = meter.push(&zeros[0]);
        assert_eq!((peak, warning), (FLOOR_DB, Some(MicWarning::Silent)));
        assert_eq!(meter.push(&speech(0.3, 1)[0]).1, None);
    }

    #[test]
    fn takes_are_named_after_the_voiceovers_the_project_has() {
        let asset = |name: &str| Asset {
            name: name.into(),
            ..serde_json::from_value(serde_json::json!({
                "id": "a", "kind": "audio", "path": "/v.wav", "name": "", "durationUs": 1, "width": 0, "height": 0,
                "fps": 0.0, "hasAudio": true
            }))
            .unwrap()
        };
        assert_eq!(take_name(&[]), "Voiceover");
        assert_eq!(take_name(&[asset("Voiceover"), asset("talk.mp4")]), "Voiceover 2");
        assert_eq!(take_name(&[asset("Voiceover 2")]), "Voiceover");
    }

    #[test]
    fn a_finished_take_is_a_wav_that_ffmpeg_reads_at_its_length() {
        let dir = std::env::temp_dir().join(format!("nuzky-voiceover-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("take.wav");
        let mut take = Take {
            file: BufWriter::new(File::create(&path).unwrap()),
            path: path.clone(),
            rate: 44_100,
            frames: 0,
            start_us: 0,
            project: PathBuf::new(),
            failed: None,
        };
        take.file.write_all(&wav_header(44_100, 0)).unwrap();
        let tone: Vec<f32> = (0..44_100 * 3 / 2).map(|n| 0.5 * (n as f32 * 0.06).sin()).collect();
        take.write(&tone);
        let (written, length) = take.finish().map_err(|(_, e)| e).unwrap();
        assert_eq!(length, 1_500_000);
        let asset = nuzky_engine::media::probe(&written, "a".into()).unwrap();
        assert_eq!(asset.kind, AssetKind::Audio);
        assert!((asset.duration_us - 1_500_000).abs() < 1_000, "{}", asset.duration_us);
        std::fs::remove_dir_all(dir).ok();
    }
}

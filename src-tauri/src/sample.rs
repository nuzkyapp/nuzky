//! The sample video of the tour for new users: opening it as a new project, and the tour's AI edit of it. That
//! edit is Nuzky's own, with no agent, network or model: a real run of the project's session, live on the
//! timeline, that the user can stop and undo in one step like any agent's.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use nuzky_analysis::CaptionGrouping;
use nuzky_engine::edit::new_id;
use nuzky_engine::model::{Asset, ClipContent};
use nuzky_mcp::{sample, transcript};
use nuzky_session::{EndAction, Expect, host::Host, transcripts::TranscriptStore};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{AppState, CmdResult, ImportResult, OpenSession, err};

const VIDEO: &[u8] = include_bytes!("../../assets/sample/spotlight-effect.mp4");
/// In a numbered folder of Nuzky's data folder, the first one free or holding this video, so a file that
/// changed there, and projects using it, are left as they are.
const FILE: &str = "The spotlight effect.mp4";
const PROJECT: &str = "Sample video";
/// She speaks without retakes, so the cut finds none and only shortens pauses.
const LABEL: &str = "Cut long pauses";
/// An agent's pause length (edit_transcript's `shorten_pauses_us`); the Transcript tab's 0.5 s would cut 1.3 s
/// of this minute, too little to see.
const PAUSE_US: i64 = 300_000;
/// Between the cut and the captions, so the timeline is seen changing twice.
const BREATH: Duration = Duration::from_millis(800);

/// What the tour's edit did, counted as the run's card counts it.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SampleRun {
    before_us: i64,
    after_us: i64,
    /// Words cut out, those of retakes included.
    words: usize,
    /// Earlier attempts at a sentence that were cut out.
    retakes: usize,
    /// Cuts without a word: pauses shortened or removed.
    pauses: usize,
    /// Captions on the timeline once it is done.
    captions: usize,
}

/// The sample's file in `data`, written when it is not there yet, with its transcript stored unless the file
/// has one, which the user may have made again.
pub(crate) fn install(data: &Path, transcripts: &TranscriptStore) -> Result<Asset> {
    let path = (1..)
        .map(|n| data.join(format!("samples/{n}/{FILE}")))
        .find(|path| std::fs::read(path).map_or(true, |bytes| bytes == VIDEO))
        .context("no folder for the sample")?;
    if !path.exists() {
        nuzky_mcp::sounds::publish(&path, |file| Ok(std::io::Write::write_all(file, VIDEO)?))?;
    }
    let mut asset = nuzky_engine::media::probe(&path, new_id())?;
    asset.credit = Some(sample::manifest().credit.clone());
    if transcripts.get(&asset)?.is_none() {
        transcripts.put(&asset, &sample::transcript())?;
    }
    Ok(asset)
}

/// A new project of the sample video, opened like one made from media; each call makes a new one, uncut.
#[tauri::command]
pub async fn open_sample(app: AppHandle) -> CmdResult<ImportResult> {
    log::info!(target: "action", "open the sample video");
    let asset = tauri::async_runtime::spawn_blocking(|| {
        install(&nuzky_engine::data_dir().context("Nuzky has no folder for its data")?, &TranscriptStore::open()?)
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    crate::open_new_project(&app, PROJECT, vec![asset], Vec::new())
}

/// The tour's AI edit of the open sample project. Stop in the top bar ends it with what it did so far, and
/// it then fails with CANCELLED.
#[tauri::command]
pub async fn sample_run(app: AppHandle, expected_epoch: Option<String>) -> CmdResult<SampleRun> {
    tauri::async_runtime::spawn_blocking(move || {
        let (host, run_id, samples) =
            begin(&app.state::<AppState>().session, expected_epoch.as_deref()).map_err(err)?;
        run(&host, &run_id, &samples, BREATH).map_err(err)
    })
    .await
    .map_err(err)?
}

/// Opens the run while the app's project is still the one the caller saw, so opening another project ends the
/// run and with it the edit.
fn begin(session: &Mutex<OpenSession>, expected_epoch: Option<&str>) -> Result<(Arc<Host>, String, Vec<Asset>)> {
    let current = crate::lock_session(session, expected_epoch).map_err(anyhow::Error::msg)?;
    let host = current.host.clone();
    let state = host.session.state()?;
    ensure!(state.open_run.is_none(), "RUN_ACTIVE: stop the open run before editing");
    let samples = main_samples(&state.project)?;
    for asset in &samples {
        if host.transcripts.get(asset)?.is_none() {
            host.transcripts.put(asset, &sample::transcript())?;
        }
    }
    let run_id = host.session.begin_run(LABEL.into())?.run_id;
    Ok((host, run_id, samples))
}

/// What "Remove retakes and pauses" in the Transcript tab cuts with pauses of `PAUSE_US`, then after `breath`
/// captions of what is left, in the run `begin` opened. The session lock is held only while each edit is
/// applied, so the timeline shows both.
fn run(host: &Host, run_id: &str, samples: &[Asset], breath: Duration) -> Result<SampleRun> {
    let before = host.session.state()?.project;
    let done = edit(host, run_id, samples, breath).and_then(|()| host.session.end_run(run_id, EndAction::Keep));
    if let Err(error) = done {
        // Stop, or another project opening, ended the run, keeping what it had done.
        if host.session.check_run(run_id).is_err() {
            bail!("CANCELLED: the edit was stopped");
        }
        host.session.end_run(run_id, EndAction::Discard)?;
        return Err(error);
    }
    let after = host.session.state()?.project;
    let captions = after.tracks.iter().filter(|t| t.is_captions()).map(|t| t.clips.len()).sum();
    let (recap, _) = nuzky_mcp::recap::recap(before, after, &host.transcripts)?;
    let totals = recap.totals();
    Ok(SampleRun {
        before_us: totals.before_us,
        after_us: totals.after_us,
        words: totals.words.unwrap_or(0),
        retakes: totals.retakes.unwrap_or(0),
        pauses: totals.pauses.unwrap_or(0),
        captions,
    })
}

/// The assets of the main track's clips, when every one of them is the sample.
fn main_samples(project: &nuzky_engine::Project) -> Result<Vec<Asset>> {
    let mut assets: Vec<Asset> = Vec::new();
    for clip in project.tracks.iter().filter(|t| t.id == nuzky_engine::edit::MAIN_TRACK).flat_map(|t| &t.clips) {
        if let ClipContent::Media { asset_id, .. } = &clip.content
            && !assets.iter().any(|a| a.id == *asset_id)
        {
            assets.extend(project.asset(asset_id).cloned());
        }
    }
    ensure!(
        !assets.is_empty() && assets.iter().all(sample::is_sample),
        "NOT_SAMPLE: this edit is only for the sample video. Open it again from the tour."
    );
    Ok(assets)
}

fn edit(host: &Host, run_id: &str, samples: &[Asset], breath: Duration) -> Result<()> {
    let going = || host.session.check_run(run_id);
    // Hesitations and key words are heard in the sound, so the edit is the same however soon it starts.
    for asset in samples {
        nuzky_engine::audio::ensure_pcm(&host.cache_dir, asset, |_| going())?;
    }
    let state = host.session.state()?;
    let derived = transcript::derive(&state.project, &host.transcripts)?;
    let hesitations = transcript::hesitations(&state.project, &derived, &host.cache_dir);
    let ranges = crate::transcripts::retake_ranges(&state.project, &derived, PAUSE_US, &hesitations)?;
    if !ranges.is_empty() {
        let cut = transcript::plan_cut(&state.project, &derived, ranges)?;
        let expect = Expect { revision: Some(state.stamp.revision), speech_layout_key: Some(state.speech_layout_key) };
        host.session.apply_edits(run_id, &new_id(), vec![cut.edit], expect)?;
    }
    let until = Instant::now() + breath;
    while Instant::now() < until {
        going()?;
        std::thread::sleep(Duration::from_millis(50));
    }
    let state = host.session.state()?;
    let derived = transcript::derive(&state.project, &host.transcripts)?;
    let energy =
        nuzky_analysis::prepared_word_energy(&state.project, &derived.words, &derived.sources, &host.cache_dir)?;
    let keys = nuzky_analysis::key_words(&derived.words, &energy);
    let look = nuzky_engine::edit::caption_presets()[0].clone();
    let (captions, _) =
        transcript::caption_edit(&derived.words, &keys, &state.project, look, CaptionGrouping::default())?;
    let expect = Expect { revision: Some(state.stamp.revision), speech_layout_key: None };
    host.session.apply_edits(run_id, &new_id(), vec![captions], expect)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuzky_engine::edit::{EditCmd, TimeRange};
    use nuzky_session::{Mode, ProjectSession};
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;

    /// A data folder of its own, with no models in it, and the sample installed there.
    fn installed(name: &str) -> (PathBuf, TranscriptStore, Asset) {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tmp-test").join(format!("sample-{name}-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dunce::canonicalize(dir).unwrap();
        let store = TranscriptStore::at(dir.join("transcripts")).unwrap();
        let asset = install(&dir, &store).unwrap();
        (dir, store, asset)
    }

    /// `host`'s project open in the app, and the test's own hold on it.
    fn app(host: Host) -> (Mutex<OpenSession>, Arc<Host>) {
        let path = host.session.path();
        let host = Arc::new(host);
        let current = OpenSession {
            host: host.clone(),
            #[cfg(unix)]
            listener: None,
            bridge_error: None,
            path,
            stopped: Arc::default(),
            learn_at: Arc::default(),
        };
        (Mutex::new(current), host)
    }

    /// The sample as `open_sample` opens it.
    fn opened(name: &str) -> (PathBuf, Mutex<OpenSession>, Arc<Host>) {
        let (dir, transcripts, asset) = installed(name);
        let path = dir.join("projects/sample.nuzky");
        crate::store::create(&path, &crate::project_from_media(PROJECT, vec![asset]).unwrap()).unwrap();
        let session = ProjectSession::open(&path, Mode::Write, None).unwrap();
        let (app, host) = app(Host { session, jobs: Default::default(), transcripts, cache_dir: dir.join("cache") });
        (dir, app, host)
    }

    /// The tour's edit as `sample_run` makes it.
    fn tour(app: &Mutex<OpenSession>, breath: Duration) -> Result<SampleRun> {
        let (host, run_id, samples) = begin(app, None)?;
        run(&host, &run_id, &samples, breath)
    }

    /// Closes the project as the app does, then removes its folder once nothing holds the project.
    fn close(dir: PathBuf, app: Mutex<OpenSession>, host: Arc<Host>) {
        drop(app);
        while Arc::strong_count(&host) > 1 {
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(host);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_bundled_transcript_belongs_to_the_bundled_video() {
        let manifest = sample::manifest();
        let sha256: String = Sha256::digest(VIDEO).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!((VIDEO.len() as u64, sha256.as_str()), (manifest.bytes, manifest.sha256.as_str()));
        // Storing the transcript checks its fingerprint and duration against the file.
        let (dir, store, asset) = installed("install");
        assert_eq!(store.get(&asset).unwrap(), Some(sample::transcript()));
        assert!(sample::is_sample(&asset));
        assert_eq!(asset.credit.as_ref().unwrap().author, "Dianne Glavaš");
        // Words the user recognised again stay, as projects of the sample read them.
        let mut again = sample::transcript();
        again.model = "small".into();
        again.words[0].text = "Again".into();
        store.put(&asset, &again).unwrap();
        install(&dir, &store).unwrap();
        assert_eq!(store.get(&asset).unwrap(), Some(again));
        let file = Path::new(&asset.path);
        let written = std::fs::metadata(file).unwrap().modified().unwrap();
        assert_eq!(install(&dir, &store).unwrap().path, asset.path);
        assert_eq!(std::fs::metadata(file).unwrap().modified().unwrap(), written);
        // A file changed since is someone's, and projects may use it: the sample goes beside it.
        std::fs::write(file, b"not the sample").unwrap();
        assert!(!sample::is_sample(&asset));
        let beside = install(&dir, &store).unwrap();
        assert!(std::fs::read(file).unwrap() == b"not the sample");
        assert_ne!(beside.path, asset.path);
        assert!(std::fs::read(&beside.path).unwrap() == VIDEO);
        assert_eq!(install(&dir, &store).unwrap().path, beside.path);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_tour_edit_cuts_and_captions_the_sample_as_one_run_that_one_undo_takes_back() {
        let (dir, app, host) = opened("run");
        let original = host.session.state().unwrap().project;
        assert_eq!((original.name.as_str(), original.canvas.width, original.canvas.height), (PROJECT, 1920, 1080));

        let done = tour(&app, Duration::ZERO).unwrap();
        let state = host.session.state().unwrap();
        let edited = state.project;
        assert!(state.open_run.is_none());
        assert_eq!((done.before_us, done.after_us), (original.duration_us(), edited.duration_us()));
        // Over 3 s of pauses go, and nothing else: the run's label promises only pauses.
        assert!(done.after_us < done.before_us - 3_000_000 && done.pauses >= 10, "{done:?}");
        assert_eq!((done.words, done.retakes), (0, 0), "{done:?}");
        // The run's card says the same.
        let card = nuzky_session::summary::summarize(&original, &edited);
        assert!(card.iter().any(|c| c.text == format!("Added {} captions", done.captions)), "{card:?}");
        assert!(done.captions >= 20, "{done:?}");
        // The export's credits name her as the footage, cut.
        let credits = nuzky_engine::credits::credits(&edited).unwrap();
        assert!(credits.starts_with("Video:\n\"Stop Performing") && credits.contains("by Dianne Glavaš"), "{credits}");
        assert!(credits.ends_with("licensed under CC BY 3.0 (https://creativecommons.org/licenses/by/3.0/). Cut.\n"));

        // Nothing is learned from the sample: neither its export nor the creator's edits after the AI's cut.
        let learns = |edited| {
            let project = host.session.state().unwrap().project;
            nuzky_mcp::style::moment(&host.session, project, false, edited).unwrap().is_some()
        };
        assert!(!learns(false));
        host.session.undo().unwrap();
        assert_eq!(host.session.state().unwrap().project, original);
        host.session.redo().unwrap();
        let cut = EditCmd::RippleDeleteRanges {
            ranges: vec![TimeRange { start_us: 0, end_us: 500_000 }],
            keep_track_ids: None,
        };
        host.session.edit(vec![cut], None, Expect::default()).unwrap();
        assert!(!learns(true) && !learns(false));
        close(dir, app, host);
    }

    #[test]
    fn stop_ends_the_tour_edit_with_what_it_did() {
        let (dir, app, host) = opened("stop");
        let original = host.session.state().unwrap().project;
        std::thread::scope(|scope| {
            let running = scope.spawn(|| tour(&app, Duration::from_secs(10)));
            // Stop in the top bar, once the cut shows and the captions wait.
            while host.session.state().unwrap().project.duration_us() == original.duration_us() {
                std::thread::sleep(Duration::from_millis(20));
            }
            host.end_open_run(EndAction::Keep).unwrap();
            let error = running.join().unwrap().unwrap_err().to_string();
            assert!(error.starts_with("CANCELLED"), "{error}");
        });
        let state = host.session.state().unwrap();
        assert!(state.open_run.is_none());
        assert!(!state.project.tracks.iter().any(|t| t.is_captions()));
        host.session.undo().unwrap();
        assert_eq!(host.session.state().unwrap().project, original);
        close(dir, app, host);
    }

    #[test]
    fn opening_another_project_stops_the_tour_edit_before_it_starts() {
        let (dir, app, host) = opened("switch");
        let original = host.session.state().unwrap().project;
        let epoch = host.session.stamp().session_epoch;
        assert!(begin(&app, Some("another")).err().unwrap().to_string().starts_with("EPOCH_CHANGED"));
        let (worker, run_id, samples) = begin(&app, Some(&epoch)).unwrap();
        // Another project opens before the worker edits.
        app.lock().unwrap().prepare_switch().unwrap();
        let error = run(&worker, &run_id, &samples, Duration::ZERO).unwrap_err().to_string();
        assert!(error.starts_with("CANCELLED"), "{error}");
        drop(worker);
        assert!(host.session.state().unwrap().open_run.is_none());
        assert_eq!(crate::store::load(&host.session.path()).unwrap(), original);
        close(dir, app, host);
    }

    #[test]
    fn the_tour_edit_is_only_for_the_sample_and_never_inside_another_run() {
        let (dir, host) = crate::transcripts::tests::fixture();
        let (app, host) = app(host);
        let refused = tour(&app, Duration::ZERO).unwrap_err().to_string();
        assert!(refused.starts_with("NOT_SAMPLE"), "{refused}");
        let agent = host.session.begin_run("Agent".into()).unwrap().run_id;
        let refused = tour(&app, Duration::ZERO).unwrap_err().to_string();
        assert!(refused.starts_with("RUN_ACTIVE"), "{refused}");
        host.session.end_run(&agent, EndAction::Keep).unwrap();
        close(dir, app, host);
    }
}

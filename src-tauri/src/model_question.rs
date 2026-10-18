//! When an agent's job needs models that are not installed, the app asks the user whether to download them and
//! downloads them as the panels do: the speech model for a transcript, the face and person models for covers,
//! reframing and clip backgrounds. Jobs that need the same kind meanwhile wait for the same answer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use nuzky_analysis::models_dir;
use nuzky_engine::edit::new_id;
use nuzky_mcp::transcript;
use nuzky_session::jobs::{Progress, check_cancel};
use nuzky_vision::models::{self, Model};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;
use crate::jobs::{Reporter, download_failed, download_speech, ensure_mattes, missing_speech_mb, register, unregister};

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Speech,
    Vision,
}

/// What the user is asked, sent as `speech-model-question` or `vision-model-question`; null closes it.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelQuestion {
    pub id: String,
    pub kind: ModelKind,
    /// What the download adds up to.
    pub size_mb: u32,
}

/// One question to the user and the download that follows it.
struct Round {
    question: ModelQuestion,
    /// The face and person models it downloads.
    vision: Vec<Model>,
    /// Jobs waiting for the answer; when the last one stops before the user answered, the question closes.
    waiting: usize,
    downloading: bool,
    /// How it ended, for every job that waited on it.
    outcome: Arc<OnceLock<Result<(), String>>>,
}

static ROUNDS: Mutex<[Option<Round>; 2]> = Mutex::new([None, None]);

impl ModelKind {
    fn event(self) -> &'static str {
        match self {
            Self::Speech => "speech-model-question",
            Self::Vision => "vision-model-question",
        }
    }

    fn what(self) -> &'static str {
        match self {
            Self::Speech => "the speech model",
            Self::Vision => "the face and person models",
        }
    }

    fn without_it(self) -> &'static str {
        match self {
            Self::Speech => {
                "Do not call transcribe again in this conversation. Tell the user what you can do without a \
                 transcript, such as cutting the long silences analyze kind silences finds, and that the Transcript \
                 tab downloads the model whenever they want."
            }
            Self::Vision => {
                "Do not call tools that need them again in this conversation: analyze kind thumbnail_frames, \
                 segment_subject, reframe and clip backgrounds. Tell the user what you can do without them, such as \
                 choosing a cover frame yourself with inspect_frames, and that Get set up downloads them whenever \
                 they want."
            }
        }
    }

    fn close(self, app: &AppHandle) {
        app.emit(self.event(), None::<ModelQuestion>).ok();
    }
}

/// Blocks an agent's transcription job until the speech model is installed, asking the user first.
pub fn ask_speech(app: &AppHandle, cancel: &AtomicBool, progress: &Progress) -> anyhow::Result<()> {
    let model = transcript::best_model();
    // Installed meanwhile, by another agent's answer or a panel.
    if transcript::models(model).is_ok() {
        return Ok(());
    }
    let installed = || transcript::models(model).is_ok();
    ask(app, ModelKind::Speech, Vec::new(), missing_speech_mb(model), &installed, cancel, progress)
}

/// Blocks an agent's job until the face and person models it needs are installed, asking the user first.
pub fn ask_vision(app: &AppHandle, needed: &[Model], cancel: &AtomicBool, progress: &Progress) -> anyhow::Result<()> {
    // A question open for another tool's models may download fewer than this one needs; then it asks again.
    loop {
        let missing = models::missing(needed, &models_dir());
        if missing.is_empty() {
            return Ok(());
        }
        let size_mb = missing.iter().map(|m| m.size).sum::<u64>().div_ceil(1_000_000) as u32;
        let installed = || models::missing(needed, &models_dir()).is_empty();
        ask(app, ModelKind::Vision, missing, size_mb, &installed, cancel, progress)?;
    }
}

fn ask(
    app: &AppHandle,
    kind: ModelKind,
    vision: Vec<Model>,
    size_mb: u32,
    installed: &dyn Fn() -> bool,
    cancel: &AtomicBool,
    progress: &Progress,
) -> anyhow::Result<()> {
    let slot = kind as usize;
    let outcome = {
        let mut rounds = ROUNDS.lock().unwrap();
        if let Some(round) = rounds[slot].as_mut() {
            round.waiting += 1;
            round.outcome.clone()
        } else {
            let question = ModelQuestion { id: new_id(), kind, size_mb };
            app.emit(kind.event(), &question).ok();
            let outcome = Arc::default();
            rounds[slot] =
                Some(Round { question, vision, waiting: 1, downloading: false, outcome: Arc::clone(&outcome) });
            outcome
        }
    };
    loop {
        if let Some(result) = outcome.get() {
            return result.clone().map_err(anyhow::Error::msg);
        }
        let downloading = {
            let mut rounds = ROUNDS.lock().unwrap();
            let ours = rounds[slot].as_mut().filter(|round| Arc::ptr_eq(&round.outcome, &outcome));
            if cancel.load(Ordering::Relaxed) {
                // A download the user started goes on; a question nobody needs the answer to goes away.
                let unneeded = ours.is_some_and(|ours| {
                    ours.waiting -= 1;
                    ours.waiting == 0 && !ours.downloading
                });
                if unneeded && let Some(ours) = rounds[slot].take() {
                    let _ = ours.outcome.set(Err("CANCELLED: job cancelled".into()));
                    kind.close(app);
                }
                return check_cancel(cancel);
            }
            let downloading = ours.is_some_and(|ours| ours.downloading);
            // Downloaded meanwhile by a panel, Get set up or the cover editor: nothing is left to ask.
            if !downloading
                && installed()
                && let Some(ours) = rounds[slot].take_if(|r| Arc::ptr_eq(&r.outcome, &outcome))
            {
                let _ = ours.outcome.set(Ok(()));
                kind.close(app);
                continue;
            }
            downloading
        };
        // The download's own job shows its percentage; this one's would make remaining_s the download's alone.
        let phase = match kind {
            ModelKind::Speech => "downloading_speech_model",
            ModelKind::Vision => "downloading_vision_models",
        };
        progress.set(if downloading { phase } else { "waiting_for_user" }, None);
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The user's answer to question `id`: Download starts the download as a job with Stop; Not now tells the agents.
#[tauri::command]
pub fn answer_model_question(app: AppHandle, id: String, download: bool) -> Result<(), String> {
    let mut rounds = ROUNDS.lock().unwrap();
    // Answered already, or no agent waits for it any more.
    let Some(slot) = rounds.iter().position(|r| r.as_ref().is_some_and(|r| r.question.id == id && !r.downloading))
    else {
        return Ok(());
    };
    let current = rounds[slot].as_mut().unwrap();
    let kind = current.question.kind;
    let (what, without_it) = (kind.what(), kind.without_it());
    if !download {
        kind.close(&app);
        let _ = current
            .outcome
            .set(Err(format!("MODEL_DECLINED: the user chose not to download {what} now. {without_it}")));
        rounds[slot] = None;
        return Ok(());
    }
    let (job_id, busy) = match kind {
        ModelKind::Speech => (format!("speech-model:{id}"), "The speech model is already downloading"),
        ModelKind::Vision => (format!("vision-models:{id}"), "The face and person models are already downloading"),
    };
    // Another download of the same models runs, e.g. the cover editor's: the question stays until it ends.
    let Some(cancel) = register(&app, &job_id) else {
        app.emit(kind.event(), &current.question).ok();
        return Err(format!("{busy}. The AI goes on once it is done."));
    };
    kind.close(&app);
    current.downloading = true;
    let vision = current.vision.clone();
    drop(rounds);
    let (worker_app, worker_job) = (app.clone(), job_id.clone());
    let spawned = std::thread::Builder::new().name("model-question".into()).spawn(move || {
        let result = match kind {
            ModelKind::Speech => {
                let mut rep = Reporter::new(&worker_app, &worker_job, "speech-model", "Speech model".into());
                let result =
                    download_speech(transcript::best_model(), &cancel, |done, phase| rep.progress(done, Some(phase)));
                let failed = result.as_ref().err().map(|error| format!("{error:#}"));
                rep.finish(result.map(|_| None), cancel.load(Ordering::Relaxed));
                failed
            }
            ModelKind::Vision => {
                let label = "Face and person models";
                let mut rep = Reporter::new(&worker_app, &worker_job, "vision-models", label.into());
                let result = crate::cover::download_models(
                    &vision,
                    false,
                    "Downloading face and person models",
                    &cancel,
                    &mut rep,
                )
                .map_err(|error| download_failed(error, "face and person models"));
                let failed = result.as_ref().err().map(|error| format!("{error:#}"));
                rep.finish(result.map(|()| None), cancel.load(Ordering::Relaxed));
                // Clip backgrounds that waited for the person model find the person now.
                let state = worker_app.state::<AppState>();
                if failed.is_none()
                    && let Ok(open) = state.project()
                {
                    ensure_mattes(&state, &open);
                }
                failed
            }
        };
        unregister(&worker_app, &worker_job);
        let outcome = match result {
            None => Ok(()),
            Some(_) if cancel.load(Ordering::Relaxed) => {
                Err(format!("MODEL_DECLINED: the user stopped downloading {what}. {without_it}"))
            }
            Some(error) => Err(format!("MODEL_MISSING: {what} could not be downloaded ({error}). {without_it}")),
        };
        if let Some(round) = ROUNDS.lock().unwrap()[slot].take_if(|round| round.question.id == id) {
            let _ = round.outcome.set(outcome);
        }
    });
    if let Err(error) = spawned {
        unregister(&app, &job_id);
        let message = format!("Starting the download: {error}");
        if let Some(round) = ROUNDS.lock().unwrap()[slot].take() {
            let _ = round.outcome.set(Err(format!("MODEL_MISSING: {message}. {without_it}")));
        }
        return Err(message);
    }
    Ok(())
}

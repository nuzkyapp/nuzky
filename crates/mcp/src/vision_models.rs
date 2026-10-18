//! The face and person models agents' tools need, and how the open app gets them: it asks the user while the tool's
//! job waits. Without the app, as in a headless bridge where nobody can be asked, the tools answer MODEL_MISSING.
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Result, bail};
use nuzky_engine::model::Project;
use nuzky_session::jobs::Progress;
use nuzky_vision::models::{self, Model};

type AskForModels = dyn Fn(&[Model], &AtomicBool, &Progress) -> Result<()> + Send + Sync;
static ASK_FOR_MODELS: OnceLock<Box<AskForModels>> = OnceLock::new();

/// Why this agent cannot have the models: the user said not now, stopped the download or it failed.
pub(crate) type Refused = Arc<Mutex<Option<String>>>;

/// How the app gets the models a tool needs: it asks the user and downloads them when they agree, blocking until
/// they are installed, and fails with what the agent should do when they do not.
pub fn ask_for_models_with(ask: impl Fn(&[Model], &AtomicBool, &Progress) -> Result<()> + Send + Sync + 'static) {
    let _ = ASK_FOR_MODELS.set(Box::new(ask));
}

/// Before a tool starts its job: with the app, missing models are asked for inside the job, unless the user
/// already refused them to this agent.
pub(crate) fn check(needed: &[Model], refused: &Refused) -> Result<()> {
    let dir = nuzky_analysis::models_dir();
    if ASK_FOR_MODELS.get().is_none() {
        return models::require(needed, &dir);
    }
    nuzky_vision::runtime::require()?;
    if !models::missing(needed, &dir).is_empty()
        && let Some(reason) = refused.lock().unwrap().clone()
    {
        bail!(reason);
    }
    Ok(())
}

/// Inside the job: waits while the app asks the user for the missing models and downloads them.
pub(crate) fn ensure(needed: &[Model], refused: &Refused, cancel: &AtomicBool, progress: &Progress) -> Result<()> {
    let Some(ask) = ASK_FOR_MODELS.get() else { return Ok(()) };
    if models::missing(needed, &nuzky_analysis::models_dir()).is_empty() {
        return Ok(());
    }
    ask(needed, cancel, progress).inspect_err(|error| {
        if !error.to_string().starts_with("CANCELLED") {
            *refused.lock().unwrap() = Some(format!("{error:#}"));
        }
    })
}

/// Finds the person in the frames the project's clip backgrounds show, getting the person model first.
pub(crate) fn prepare_mattes(
    project: &Project,
    cache: &Path,
    refused: &Refused,
    cancel: &AtomicBool,
    progress: &Progress,
) -> Result<()> {
    if nuzky_engine::matte::missing(cache, project).is_empty() {
        return Ok(());
    }
    ensure(models::BACKGROUND, refused, cancel, progress)?;
    nuzky_vision::matte::prepare(project, cache, &nuzky_analysis::models_dir(), cancel, &mut |p| {
        progress.set("finding_the_person", Some(p))
    })
}

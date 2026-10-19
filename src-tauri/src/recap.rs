use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Result, ensure};
use nuzky_engine::{
    edit::new_id,
    recap::{Recap, RecapOptions, RecapTotals},
};
use nuzky_session::host::Host;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{
    AppState,
    jobs::{Reporter, check_destination, crashed, register, unregister},
};

/// Snips of real scissors the recap plays as cuts snap shut, in turn, so no snip follows itself.
const SNIPS: [&str; 3] = ["nuzky:scissors-snip", "nuzky:scissors-snip-2", "nuzky:scissors-snip-3"];

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecapInfo {
    pub totals: RecapTotals,
    /// The lengths and what went, written as the run summary writes them.
    pub before: String,
    pub after: String,
    #[cfg_attr(test, ts(optional = nullable))]
    pub removed: Option<String>,
    /// Passages cut; a recap has nothing to show without one.
    pub cuts: usize,
    /// How long the recap is with its end card.
    pub duration_us: i64,
    /// Files a cut touches that have no transcript.
    pub untranscribed: Vec<String>,
}

/// The recap and the files its cuts touch that have no transcript.
fn build(host: &Host, run_id: Option<&str>) -> Result<(Recap, Vec<String>)> {
    let (before, after) = match run_id {
        Some(id) => host.session.run_pair(id)?,
        None => {
            let project = host.session.state()?.project;
            let before = nuzky_mcp::recap::raw_import(&project, &host.session.kept_main_assets()?)?;
            (before, project)
        }
    };
    ensure!(after.duration_us() > 0, "Add something to the timeline before exporting a recap.");
    nuzky_mcp::recap::recap(before, after, &host.transcripts)
}

fn host(app: &AppHandle, expected_epoch: Option<&str>) -> Result<Arc<Host>, String> {
    let state = app.state::<AppState>();
    Ok(crate::lock_session(&state.session, expected_epoch)?.host.clone())
}

#[tauri::command]
pub async fn recap_info(
    app: AppHandle,
    run_id: Option<String>,
    expected_epoch: Option<String>,
) -> Result<RecapInfo, String> {
    let host = host(&app, expected_epoch.as_deref())?;
    tauri::async_runtime::spawn_blocking(move || -> Result<RecapInfo> {
        let (recap, missing) = build(&host, run_id.as_deref())?;
        let untranscribed = missing
            .iter()
            .filter_map(|id| {
                recap.before.asset(id).map(|a| {
                    std::path::Path::new(&a.path)
                        .file_name()
                        .map_or_else(|| a.name.clone(), |name| name.to_string_lossy().into_owned())
                })
            })
            .collect();
        let totals = recap.totals();
        Ok(RecapInfo {
            totals,
            before: nuzky_engine::recap::duration_text(totals.before_us),
            after: nuzky_engine::recap::duration_text(totals.after_us),
            removed: nuzky_mcp::recap::line(&totals),
            cuts: recap.cuts.len(),
            duration_us: nuzky_engine::recap::duration_us(&recap, true),
            untranscribed,
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(crate::err)
}

#[tauri::command]
pub async fn start_recap(
    app: AppHandle,
    out: PathBuf,
    run_id: Option<String>,
    end_card: bool,
    replace_existing: bool,
    expected_epoch: Option<String>,
) -> Result<String, String> {
    check_destination(&out, replace_existing)?;
    let host = host(&app, expected_epoch.as_deref())?;
    let source = host.clone();
    let (mut recap, _) = tauri::async_runtime::spawn_blocking(move || build(&source, run_id.as_deref()))
        .await
        .map_err(|e| e.to_string())?
        .map_err(crate::err)?;
    // A recap of an older run must not replace a file imported since, either.
    let now = host.session.state().map_err(crate::err)?.project;
    for project in [&recap.before, &recap.after, &now] {
        nuzky_engine::export::check_source_path(project, &out).map_err(crate::err)?;
    }
    let id = format!("export:{}", new_id());
    let state = app.state::<AppState>();
    let current = crate::lock_session(&state.session, expected_epoch.as_deref())?;
    if !Arc::ptr_eq(&host, &current.host) {
        return Err("EPOCH_CHANGED: another project is open".into());
    }
    let cancel = register(&app, &id).ok_or("An export is already running")?;
    drop(current);
    let cache = app.state::<AppState>().cache_dir.clone();
    let (worker, job_id) = (app.clone(), id.clone());
    let spawned = std::thread::Builder::new().name("recap".into()).spawn(move || {
        let name = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut rep = Reporter::new(&worker, &job_id, "export", format!("Exporting {name}"));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            nuzky_session::jobs::check_cancel(&cancel)?;
            let sound = |id: &str| nuzky_mcp::sounds::asset(id, &cache, || cancel.load(Ordering::Relaxed), |_| {});
            recap.snips = SNIPS.iter().map(|id| sound(id)).collect::<Result<_, _>>()?;
            recap.whoosh = Some(sound("nuzky:swish-short")?);
            recap.land = Some(sound("nuzky:soft-hit")?);
            nuzky_vision::matte::prepare(&recap.after, &cache, &nuzky_analysis::models_dir(), &cancel, &mut |p| {
                rep.progress(p, Some("Finding the person"));
            })?;
            nuzky_engine::recap::render(
                &recap,
                &cache,
                &out,
                &RecapOptions { end_card, replace_existing },
                &cancel,
                |p| {
                    rep.progress(p, Some("Rendering recap"));
                },
            )
        }))
        .unwrap_or_else(|p| Err(crashed(&p)));
        rep.finish(result.map(|_| Some(out.to_string_lossy().into_owned())), cancel.load(Ordering::Relaxed));
        unregister(&worker, &job_id);
    });
    if let Err(error) = spawned {
        unregister(&app, &id);
        return Err(error.to_string());
    }
    Ok(id)
}

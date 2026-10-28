//! Making the reels the user picked from a long video, the same way agents do with make_reels.

use anyhow::Result;
use nuzky_engine::{
    edit::{EditCmd, new_id},
    model::ReelCandidate,
};
use nuzky_mcp::reels::{self, Finish, Framing, MadeReel};
use nuzky_mcp::transcript;
use nuzky_session::{Expect, SessionState, host::Host};
use std::sync::{Arc, atomic::Ordering};
use tauri::{AppHandle, Emitter, Manager};

use crate::jobs::{Reporter, check_cancelled, crashed, register, unregister};
use crate::{AppState, CmdResult};

/// Writes the reels, calling `step` before each, then hands the edit that marks them made to `apply`; when it is
/// refused, the reels go again.
fn make(
    host: &Host,
    state: &SessionState,
    ids: &[String],
    (framing, finish): (Framing, Finish),
    step: &mut dyn FnMut(usize) -> Result<()>,
    cancelled: &dyn Fn() -> bool,
    apply: impl FnOnce(Vec<EditCmd>, Expect) -> Result<()>,
) -> Result<Vec<MadeReel>> {
    anyhow::ensure!(state.open_run.is_none(), "RUN_ACTIVE: stop the open run before editing");
    let derived = transcript::derive(&state.project, &host.transcripts)?;
    let stored = (&host.transcripts, host.cache_dir.as_path());
    let path = host.session.path();
    let (made, edits) = reels::make(&path, &state.project, &derived, ids, framing, finish, stored, step, cancelled)?;
    let expect = Expect { revision: Some(state.stamp.revision), speech_layout_key: None };
    if let Err(error) = apply(edits, expect) {
        reels::forget(&made, &host.session.state()?.project);
        return Err(error);
    }
    Ok(made)
}

#[tauri::command]
pub async fn reel_bounds(app: AppHandle, id: String, from: usize, to: usize) -> CmdResult<ReelCandidate> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<AppState>().session.lock().unwrap().host.clone();
        let state = host.session.state().map_err(crate::err)?;
        let derived = transcript::derive(&state.project, &host.transcripts).map_err(crate::err)?;
        reels::bounds(&state.project, &derived, &id, from, to).map_err(crate::err)
    })
    .await
    .map_err(crate::err)?
}

#[tauri::command]
pub fn start_make_reels(
    app: AppHandle,
    ids: Vec<String>,
    canvas: Option<Framing>,
    finish: Option<Finish>,
    expected_epoch: Option<String>,
) -> CmdResult<String> {
    let state = app.state::<AppState>();
    let (host, view) = {
        let current = crate::lock_session(&state.session, expected_epoch.as_deref())?;
        (Arc::downgrade(&current.host), current.host.session.state().map_err(crate::err)?)
    };
    let id = format!("reels:{}", new_id());
    let cancel = register(&app, &id).ok_or("Reels are already being made")?;
    let (worker, job) = (app.clone(), id.clone());
    let spawned = std::thread::Builder::new().name("reels".into()).spawn(move || {
        let mut rep = Reporter::new(&worker, &job, "reels", "Making reels".into());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            const SWITCHED: &str = "Another project was opened, so the reels were not applied.";
            let source = host.upgrade().ok_or_else(|| anyhow::anyhow!(SWITCHED))?;
            let made = make(
                &source,
                &view,
                &ids,
                (canvas.unwrap_or_default(), finish.unwrap_or_default()),
                &mut |i| {
                    check_cancelled(&cancel)?;
                    rep.progress(i as f32 / ids.len() as f32, Some(&format!("Making reel {} of {}", i + 1, ids.len())));
                    Ok(())
                },
                &|| cancel.load(Ordering::Relaxed),
                |edits, expect| {
                    check_cancelled(&cancel)?;
                    let state = worker.state::<AppState>();
                    let current = state.session.lock().unwrap();
                    anyhow::ensure!(host.ptr_eq(&Arc::downgrade(&current.host)), SWITCHED);
                    current.host.session.edit(edits, None, expect)?;
                    if let Ok(snap) = current.snapshot(Vec::new()) {
                        worker.emit("project-changed", snap).ok();
                    }
                    Ok(())
                },
            )?;
            // The reels are made either way; the user hears when the home screen will not list them.
            let listed = worker.state::<AppState>().library.remember_reels(&source.session.path(), &made);
            let unlisted = listed.err().map(|error| format!("{error:#}"));
            Ok(Some(serde_json::json!({ "reels": made, "unlisted": unlisted }).to_string()))
        }))
        .unwrap_or_else(|p| Err(crashed(&p)));
        rep.finish(result, cancel.load(Ordering::Relaxed));
        unregister(&worker, &job);
    });
    if let Err(error) = spawned {
        unregister(&app, &id);
        return Err(format!("Starting reels: {error}"));
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuzky_engine::model::ReelStatus;
    use nuzky_mcp::reels::ReelProposal;
    use nuzky_session::EndAction;

    /// The app makes a reel as agents do, as one undo step, and never while an agent's run holds the project.
    #[test]
    fn the_app_makes_a_reel_as_one_undo_and_never_during_a_run() {
        let (dir, host) = crate::transcripts::tests::fixture();
        let state = host.session.state().unwrap();
        let derived = transcript::derive(&state.project, &host.transcripts).unwrap();
        let proposal = ReelProposal { from: 2, to: 4, title: "Middle".into(), why: String::new(), score: 0.5 };
        let max = reels::DEFAULT_MAX_DURATION_US;
        let candidates = reels::plan(&state.project, &derived, &[proposal], max, transcript::DEFAULT_PAUSE_US).unwrap();
        let proposed = EditCmd::SetReelCandidates { candidates: candidates.clone() };
        host.session.edit(vec![proposed], None, Expect::default()).unwrap();
        let ids = vec![candidates[0].id.clone()];
        let reel = dunce::canonicalize(&dir).unwrap().join("project-reel-1.nuzky");

        let make = || {
            let state = host.session.state().unwrap();
            super::make(
                &host,
                &state,
                &ids,
                (Framing::Crop, Finish::default()),
                &mut |_| Ok(()),
                &|| false,
                |edits, expect| host.session.edit(edits, None, expect).map(|_| ()),
            )
        };
        let run = host.session.begin_run("Agent".into()).unwrap();
        let refused = make().unwrap_err().to_string();
        assert!(refused.starts_with("RUN_ACTIVE"), "{refused}");
        assert!(!reel.exists(), "a reel the project does not name was left behind");
        host.session.end_run(&run.run_id, EndAction::Keep).unwrap();

        let made = make().unwrap();
        assert_eq!(made[0].path, reel);
        let status = |host: &Host| host.session.state().unwrap().project.reel_candidates[0].status;
        assert_eq!(status(&host), ReelStatus::Made);
        host.session.undo().unwrap();
        assert_eq!(status(&host), ReelStatus::Proposed);
        assert!(reel.is_file());
        drop(host);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

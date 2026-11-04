//! Suggested B-roll in the Transcript panel: a screen recording over the whole frame while the speaker names the app
//! it shows, found as agents find it (`find_broll`) and added as one undo step.

use anyhow::{Result, ensure};
use nuzky_engine::edit::EditCmd;
use nuzky_mcp::{broll, transcript};
use nuzky_session::{Expect, host::Host};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::{AppState, CmdResult, Snapshot};

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedBroll {
    /// Inclusive word numbers of the transcript view.
    from: usize,
    to: usize,
    start_us: i64,
    end_us: i64,
    text: String,
    asset_id: String,
    asset_name: String,
    /// The words heard that name the recording.
    said: Vec<String>,
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrollSuggestions {
    /// The transcript key the word numbers belong to; adding is refused once it changed.
    key: String,
    items: Vec<SuggestedBroll>,
    /// Silent videos in the project, which B-roll can come from.
    recordings: usize,
}

/// A suggestion the user accepted.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrollChoice {
    from: usize,
    to: usize,
    asset_id: String,
}

fn suggest(host: &Host) -> Result<BrollSuggestions> {
    let project = host.session.state()?.project;
    let derived = transcript::derive(&project, &host.transcripts)?;
    let items = broll::suggest(&project, &derived.words, None)?
        .into_iter()
        .map(|b| SuggestedBroll {
            from: b.from,
            to: b.to,
            start_us: b.start_us,
            end_us: b.end_us,
            text: b.text,
            asset_id: b.asset_id,
            asset_name: b.asset_name,
            said: b.said,
        })
        .collect();
    let recordings = broll::recordings(&project).len();
    Ok(BrollSuggestions { key: transcript::word_key(&project, &derived.words), items, recordings })
}

/// Each recording over the whole frame from the first word of its range to the last, as one user edit.
fn add(host: &Host, key: &str, choices: &[BrollChoice]) -> Result<Vec<String>> {
    let state = host.session.state()?;
    let derived = transcript::derive(&state.project, &host.transcripts)?;
    transcript::check_word_key(&state.project, &derived, key)?;
    let recordings = broll::recordings(&state.project);
    let mut edits = Vec::new();
    for choice in choices {
        ensure!(
            choice.from <= choice.to && choice.to < derived.words.len(),
            "SPEECH_CHANGED: the words changed in the meantime"
        );
        ensure!(recordings.iter().any(|a| a.id == choice.asset_id), "The recording is no longer in the project");
        let (start_us, end_us) = (derived.words[choice.from].start_us, derived.words[choice.to].end_us);
        edits.push(EditCmd::AddOverlay {
            asset_id: choice.asset_id.clone(),
            start_us,
            duration_us: Some(end_us - start_us),
        });
    }
    let edited =
        host.session.edit(edits, None, Expect { revision: None, speech_layout_key: Some(state.speech_layout_key) })?;
    Ok(edited.outcome.select)
}

fn explain(error: anyhow::Error) -> String {
    let text = format!("{error:#}");
    match text.split_once(':').map(|(code, _)| code) {
        Some("SPEECH_CHANGED") => "The transcript changed in the meantime. Suggest B-roll again.".into(),
        _ => text,
    }
}

#[tauri::command]
pub async fn suggest_broll(app: AppHandle) -> CmdResult<BrollSuggestions> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<AppState>().session.lock().unwrap().host.clone();
        suggest(&host).map_err(explain)
    })
    .await
    .map_err(crate::err)?
}

#[tauri::command]
pub async fn add_broll(
    app: AppHandle,
    key: String,
    choices: Vec<BrollChoice>,
    expected_epoch: Option<String>,
) -> CmdResult<Snapshot> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let current = crate::lock_session(&state.session, expected_epoch.as_deref())?;
        let select = add(&current.host, &key, &choices).map_err(explain)?;
        current.snapshot(select)
    })
    .await
    .map_err(crate::err)?
}

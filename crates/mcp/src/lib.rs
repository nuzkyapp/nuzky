//! Stdio MCP catalog with local and app-connected backends.
mod activity;
pub mod bridge;
pub mod caption_styles;
mod fonts;
pub mod import;
#[cfg(unix)]
pub mod ipc;
mod limits;
mod media;
pub mod model_download;
mod params;
pub mod recap;
pub mod reels;
mod rules;
pub mod sample;
pub mod sounds;
pub mod stickers;
pub mod style;
mod tools;
pub mod transcript;
pub mod videos;
pub mod vision_models;
mod waveform;
pub mod youtube;
pub mod zooms;

use std::sync::Arc;

use anyhow::{Context, Result};
use rmcp::{ErrorData, RoleServer, ServerHandler, model::*, service::RequestContext};
use schemars::{JsonSchema, schema_for};
use serde_json::Value;

use bridge::Target;
pub use tools::{Access, Backend, Client};

static STICKER_DESCRIPTION: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "Put any single emoji (including skin tones, such as 🔥 or 👍🏽), or a Nuzky sticker ({}) on a free overlay video track at at_us, as one edit of your run. Requires run_id. Pass the emoji itself when possible; CLDR English names and :shortcodes: also work. Works offline. Defaults to 3 seconds, centred, with its longer side 30% of the canvas's shorter side; optional duration_us and transform override this. Returns revision, session_epoch, asset_id, clip_id, track_id, start_us, duration_us, transform and sticker {{key, emoji, name}}. Reuse request_id with the same arguments to retry without another clip; changed arguments fail REQUEST_CONFLICT. STICKER_UNKNOWN offers close matches. No export credit needed.",
        stickers::catalog()
            .groups
            .iter()
            .flat_map(|g| &g.items)
            .filter(|i| i.emoji.is_none())
            .map(|i| i.key.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
});

const GUIDE: &str = include_str!("../../../skills/nuzky-edit/SKILL.md");
const STYLE_FIRST: &str = "This creator has their own editing style: their EDIT.md, also at nuzky://style, with rules learned from their finished cuts and their own instructions. It follows here, and Nuzky's guide comes after it. Where the style gives a rule or a number, follow it instead of the guide's defaults: what to cut, pause length, captions, framing and zoom, including zoom the user did not ask for. Anything it does not cover keeps the guide's defaults. Under \"Your rules\" the creator wrote their own instructions; they win over everything else.\n\n";
/// The whole way from raw takes to a reel, as the guide describes it, so one prompt starts it.
const ROUGH_CUT: &str = "Make a rough cut of this whole project for Instagram Reels and TikTok, then export it, following the guide below and the creator's style, when there is one.\n\n1. get_state. Transcribe whatever is untranscribed and wait with job action wait until done.\n2. get_transcript, read every sentence, then analyze(kind: \"retakes\"). Keep the last complete attempt of each restarted sentence and drop the filler words; check every group against the text and decide each review pair yourself. analyze(kind: \"fillers\") also lists hesitations recognition did not write; cut them with delete_hesitations. Add other slips and false starts you find. A closing call to action goes or stays as the creator's style says, else as the guide says.\n3. Plan with edit_transcript dry_run, then in one run: edit_transcript with the deletions, get_waveform at every cut, fixing any that takes part of a word or keeps a breath, build_captions, the zoom the creator's style asks for, if it zooms, inspect_frames with safe_area around cuts and captions, end_run keep.\n4. export_video with preset \"reels\" to a new .mp4 in the user's Videos folder (an absolute path), named after the project, wait with job action wait until done and report the path, the duration and anything you were unsure about.";

/// Both covers from the best frame, with a hook from the transcript, in one run, so one prompt makes them.
const THUMBNAIL: &str = "Make a 9:16 cover for Reels and TikTok (cover_9x16, 1080x1920) and a YouTube thumbnail (youtube_16x9, 1280x720) for this video, then export both, following the guide below.\n\n1. get_state. analyze(kind: \"thumbnail_frames\") and wait with job action wait; its parts score the framing for both formats. inspect_frames(times_us) the top 4-6 candidates on one contact sheet and choose a frame for each format, the same or two: eyes open, facing the camera, an expression that fits the hook, and for 16:9 room beside the person.\n2. The hook: get_transcript (transcribe first if needed) and write 2 to 5 words in the video's language that make someone want to watch: its main promise or surprise, from what is actually said, never invented. Uppercase reads best on a thumbnail.\n3. segment_subject(time_us) of each chosen frame and wait with job action wait: subject_box [x,y,w,h] in canvas pixels says where the person and their head are.\n4. In one run (begin_run), apply_edits with both setThumbnail edits in one batch. Text styles come from the presets below; when the creator's style below names fonts or colours, use those instead. Sizes are thumbnail pixels for the cover; on youtube_16x9 use about two thirds. Text transform x and y are fractions of the thumbnail from its centre, like clips.\n   - cover_9x16: frame {x:0,y:0,scale:1,rotation:0,opacity:1} on a 9:16 video. The hook in Big with behind: true, its middle a little above the top of the head (subject_box y), so the head covers the lower part of the letters; or Yellow box or Outlined in front, inside the Reels safe area.\n   - youtube_16x9 from a vertical video: background {picture: true, blur: 1, dim: 0.2, color: \"#000000\"} or a solid colour (picture: false), the frame at scale 1.2-1.6 with the person on the left or right third: frame x = (1/3 or 2/3) - 0.5 - (person centre x / canvas width - 0.5) * frame width / 1280, where the frame is canvas width * min(1280 / canvas width, 720 / canvas height) * scale thumbnail pixels wide; y moves the face up when the person is small. outline {color: \"#ffffff\", width: 10}. The hook big over the other two thirds, Outlined or Yellow box, wrapped onto two lines with style.maxWidth. On a 16:9 video use the frame as it is.\n   - A cover keeps its overlays, images such as the user's logo: setThumbnail replaces the whole cover, so send them again unless the wishes say otherwise. A logo or picture the user wants on the covers goes in overlays [{assetId, transform}] after import_media of its file, never into the video or a generated picture: as large as a square a fifth of the thumbnail's shorter side, a wide logo at most two fifths of it long, in the top right corner, on cover_9x16 inside the Reels safe area (a square logo: scale 0.2, x 0.21, y -0.3), on youtube_16x9 32 px from the corner (scale 0.2, x 0.42, y -0.36).\n5. inspect_thumbnail(format, safe_zones: true) for both. Keep faces, the hook and any logo out of the tints (Reels interface, the 3:4 profile grid, YouTube's duration badge), the hook readable at a glance, nothing important cut by an edge, text behind the person with hidden about 0.05-0.3. MASK_NOT_READY names a job: wait with job action wait, then inspect again. Fix it with another setThumbnail in the same run until both look right.\n6. end_run keep. export_thumbnail cover_9x16 to a new .png and youtube_16x9 to a new .jpg in the user's Pictures folder (absolute paths), named after the project; wait with job action wait for both until done and report the paths, the hook and anything you were unsure about.\n\nThumbnail text presets (assets/presets/thumbnails.json):\n";
const THUMBNAIL_PRESETS: &str = include_str!("../../../assets/presets/thumbnails.json");

/// The creator's EDIT.md, read anew each time so edits to it apply to the next request.
fn style() -> Option<String> {
    std::fs::read_to_string(nuzky_analysis::style::style_path()).ok().filter(|text| !text.trim().is_empty())
}

/// The creator's style, then the guide, so the style is the first thing an agent reads and a client
/// that shortens long instructions keeps it. Without EDIT.md this is the guide alone.
pub fn guide() -> String {
    match style() {
        // A blank line, so the guide's front matter cannot turn the style's last line into a heading.
        Some(style) => format!("{STYLE_FIRST}{}\n\n{GUIDE}", style.trim_end()),
        None => GUIDE.to_owned(),
    }
}

/// The `thumbnail` prompt as an agent gets it, from MCP or from the app's AI panel.
pub fn thumbnail_prompt(wishes: &str) -> String {
    format!("{THUMBNAIL}{THUMBNAIL_PRESETS}\nThe user's wishes, which win over the steps: {wishes}\n\n{}", guide())
}

#[derive(Clone)]
struct Server {
    target: Arc<Target>,
    tools: Arc<Vec<Tool>>,
}

fn tool<T: JsonSchema>(name: &'static str, description: &'static str) -> Result<Tool> {
    let schema = schema_for!(T);
    let object = schema.as_object().context("Tool schema must be an object")?.clone();
    let rules = rules::find(name).with_context(|| format!("Tool {name} has no access rules"))?;
    let annotations = ToolAnnotations::new()
        .read_only(matches!(rules.reads, rules::Reads::Always))
        .destructive(rules.destructive)
        .idempotent(rules.idempotent)
        .open_world(rules::ONLINE.contains(&name));
    Ok(Tool::new(name, description, object).with_annotations(annotations))
}

/// Every tool as MCP's `tools/list` lists it, for the AI panel's ChatGPT agent, which calls them
/// without an MCP client.
pub fn tool_list() -> Result<Value> {
    Ok(serde_json::to_value(catalog()?)?)
}

fn catalog() -> Result<Vec<Tool>> {
    Ok(vec![
        tool::<params::State>(
            "get_state",
            "Read compact project assets, tracks and clips, thumbnails, reel_candidates (see propose_reels), imported (the CapCut draft the project was opened from and what did not come over; see the guide's Finish a draft from CapCut), selection (empty headless), playhead (0 headless), timeline-layout speech_layout_key (for apply_edits), revision, session_epoch, open_run and recovery_checkpoint. Times are integer microseconds; ranges are [start,end). Optional range and clip_ids filter clips only; read every clip you need in one call, not one call per clip. duration_us, caption_stats (count, max_chars, max_words) and missing_fonts (font families of text clips and covers that this computer does not have, so that text is drawn in Inter) always describe the full timeline. Use caption_stats after manual text corrections. Media source out = sourceInUs + durationUs * speed. The main track is magnetic: edits pack clips back-to-back from zero. Read nuzky://guide before editing.",
        )?,
        tool::<params::Begin>(
            "begin_run",
            "Begin one reversible editing run and persist its checkpoint. Requires --allow-write. Only this run may mutate the project. End with keep or discard. Two minutes without tool calls from this client auto-keeps the run; plan or transcribe before opening a run. Every response includes revision and session_epoch. Revision changes only when the project changes; beginning a run leaves it unchanged.",
        )?,
        tool::<params::Recovery>(
            "resolve_recovery",
            "Resolve get_state.recovery_checkpoint after a crash. Ask the user before choosing: keep preserves the current project file; restore validates and saves the checkpoint project as an undoable edit. Both remove the checkpoint. Requires --allow-write and no open run. Revision changes only if the project changes.",
        )?,
        tool::<params::Apply>(
            "apply_edits",
            "Atomically apply EditCmd JSON (camelCase fields and type tags). All times are integer microseconds. Supply a unique request_id within this run; retry identical content with the same id while the run is open for the stored result. Ended runs reject retries with INVALID_RUN; runs stopped by the user return RUN_STOPPED. expected_revision rejects stale edits; expected_speech_layout_key rejects moved speech with SPEECH_CHANGED while allowing unrelated caption restyles. Main track is magnetic and repacks after edits. A clip that addClip or duplicateClip puts on the main track before other main-track clips opens its time at that cut on every other track except the keepInPlace ones, so captions and overlays after it stay over their picture; one appended at its end, or moved onto the track with moveClip, changes nothing else. trimClip of a main-track clip opens or cuts the same time at the moved edge on every other track except the keepInPlace ones, so captions and overlays stay over the same picture. deleteClips of a main-track clip cuts its time from those tracks the same way, so captions over it go and later ones move with the picture. rippleDeleteRanges removes the union of half-open timeline ranges from every track except keepTrackIds, then closes gaps; omit keepTrackIds to leave the tracks marked keepInPlace (music) alone; ranges refer to the timeline BEFORE this command. A generated caption loses the words heard inside the ranges: with words left on both sides it becomes two captions, with none left it goes. Source analysis times must first be mapped through sourceInUs, startUs and speed. addCaptions creates a new track; replaceCaptions replaces only the named captions track (a text track named Captions). restyleTrack {trackId, style?, transform?} changes every clip of a text track in place, such as all captions, as one step: style takes any TextStyle fields (null clears fontFamily, background, maxWidth, highlight or keywords), transform any of x, y, scale, rotation, opacity; ids, text, words, timing, animations and fields left out stay. Use it to resize, recolour or move captions instead of rebuilding them or updateClip per caption. Clips with keyframes keep their transform and are listed in skipped. Other text never lands on a captions track automatically. Rejected commands or validation leave the project and history unchanged. Successful batches are saved before returning. A save failure reports an error but keeps the live edit; retry the identical request to retry saving without applying it twice. Result contains created/changed/removed ids and, in clips, the created and changed clips in full with their actual times. When those would exceed 8000 characters (say a restyle of every caption), clips is left out and clips_omitted gives their count; read the ones you need with get_state(clip_ids). Warnings appear when the edits give text a font this computer does not have, one per font: that text is drawn in Inter. setThumbnail {thumbnail} replaces the whole cover or thumbnail of its format, one per format, so send again what it should keep; its overlays [{assetId, transform}] are images of the project drawn over it, such as the user's logo (import_media the file first; a PNG keeps its transparency), in front of the person and behind the texts in front, the transform as for its frame; removeThumbnail {format} removes it; rippleDeleteRanges moves a thumbnail's frame time with the picture. A transform's x and y move the centre from the canvas centre by fractions of its width and height, positive y down; scale 1 fits media inside the canvas and draws text at its style's canvas-pixel sizes; rotation is degrees clockwise.",
        )?,
        tool::<params::End>(
            "end_run",
            "End the owning run: keep saves all its edits as ONE undo entry; discard drops the run entry and saves its previous state without a redo entry. The run id is then revoked. A clean disconnect keeps changes too. Keep leaves revision unchanged; discard increments it only if the project changes.",
        )?,
        tool::<params::Undo>(
            "undo_run",
            "Undo this whole run only if it is the LAST entry in the shared user/run history and no run is open. Saves the restored project. The UI can redo the whole run. History belongs to this session epoch. Revision increments only if the project changes.",
        )?,
        tool::<params::ListHistory>(
            "list_history",
            "List the kept versions of the project, newest first: index (stays with the version), hash (first 12 hex digits of the SHA-256 of the project's compact JSON), label, at_ms (Unix time in ms) and run_id for a version an agent's run made (none for the user's own steps). A version is the project after each kept run, each of the user's steps, each undo, redo and restore, and as it was opened. The user's step still in progress (a drag) leads the list under the index it will keep. current_hash is the project now, so the version with that hash is the current one, and older counts versions kept but not listed. Versions outlive restarts, up to the newest 200. Read-only; works while a run is open, but an open run becomes a version only when it ends.",
        )?,
        tool::<params::UndoTo>(
            "undo_to",
            "Restore a kept version from list_history by index or hash prefix (4+ hex digits) as ONE new undo step, so Undo in the app or undo_to the version before takes it back; versions after it stay listed, nothing is lost. Requires --allow-write and no open run (RUN_ACTIVE). The project then has exactly that version's hash. Saves before returning. changed is false and revision stays when the project already is that version. UNKNOWN_VERSION when no kept version matches or a prefix matches several.",
        )?,
        tool::<params::Import>(
            "import_media",
            "Probe existing local file paths and add assets through the owning run. Relative paths resolve beside the project. Optional request_id: reuse it with the same paths after a save failure to finish the original import without adding the assets twice. Returns asset_ids; follow with addClip edits to place them in the video, or put an image such as a logo on a cover with setThumbnail overlays, which leaves the video as it is. No uploads, downloads or automatic insertion.",
        )?,
        tool::<params::SearchSounds>(
            "search_sounds",
            "Find music (kind music) or sound effects (kind effect) the user may put in a video they monetize: online results are CC0 (no credit needed) and CC BY (the video's description must credit it). Both kinds start with the matching sounds built into Nuzky, which work offline (CC0, provider \"Built in\", a category such as \"Whoosh\" or \"Music\"). Music then lists songs the user already added from YouTube links, in any project, without the link and even offline (license null); use one only when the user asks for that song, as its rights are theirs to check. With a query, online results follow from Openverse, or for effects from Freesound when the user turned it on with their key. Only the query leaves the computer. Each sound has id, title, author, duration_us, license (cc0|by), license_version, license_url, url (its page at the source, where its licence can be checked) and provider. more is true while another page (page 2-12) has results. Offline or rate-limited, online_error says so and the built-in sounds and matching downloaded music still come back; with nothing to show the call fails with OFFLINE, RATE_LIMITED or SOURCE_FAILED. An empty query lists the built-in sounds of the kind, for music also the downloaded songs. Search in English with short words (\"whoosh\", \"upbeat\", \"cinematic\"). A query that is a link to one YouTube or YouTube Music video (youtube.com, music.youtube.com, youtu.be; not a playlist) instead returns that video as one music sound through the yt-dlp installed on this computer (a song added before comes from what Nuzky saved, without yt-dlp): id youtube:<video id>, license null, online \"YouTube\" and rights_confirmed. Only the user's own song choice should come from a link; its rights are the user's to check. YTDLP_MISSING: the user pastes the link in the Music tab of the sound library and clicks Download yt-dlp under it, and YTDLP_OUTDATED (the installed yt-dlp could not get the video) asks the same; INVALID_LINK and PLAYLIST mean the link is not one video; a private or removed video fails with SOUND_UNAVAILABLE.",
        )?,
        tool::<params::AddSound>(
            "add_sound",
            "Add a sound from search_sounds at at_us on an audio track, as ONE edit of your run (the user can undo it with the run). Requires run_id. Downloads the file first when it is not on this computer yet (seconds for music), into Nuzky's sound library; a sound the project already has gets another clip of the same asset. The asset remembers the source, author and licence, so export writes the credits beside the video. Returns asset_id, clip_id, track_id, start_us, duration_us, needs_credit and credit; when needs_credit is true, tell the user the video's description must credit it (the app's Copy credits and the .credits.txt beside the export have the text). Optional request_id: reuse it with the same id after a save failure to finish without adding the sound twice. Fails with SOUND_UNKNOWN for an id that did not come from search_sounds, OFFLINE when the file cannot be downloaded. A youtube: sound downloads only its sound through yt-dlp; it carries no credit. Before the first one, RIGHTS_UNCONFIRMED: tell the user they are responsible for the rights to the music and for following YouTube's terms, ask them (suggest_options), and only after they agree call add_sound again with rights_confirmed true; Nuzky then remembers it.",
        )?,
        tool::<params::AddYoutubeVideo>(
            "add_youtube_video",
            "Add the video from a YouTube link the user gave to the project's media, as ONE edit of your run. Requires run_id. Optional start_us and end_us select a part in the video's own time: pass both or neither for the whole video. If the user described a moment without times, ask for its start and end first. Downloads the whole video once through yt-dlp (can take minutes for long videos), then keeps the chosen part plus up to 3 s hidden around it that dragging clip edges can reveal. Stop ends the download with the run. Optional at_us also places a clip at that timeline time, initially showing only the part. Returns asset_id, asset_duration_us (the saved file's duration), part (startUs/endUs inside that file, or null for the whole video), clip_id/track_id/start_us (null without a clip), and duration_us (the clip's duration when placed, otherwise the asset's). Optional request_id: reuse it with the same link, part and placement after a save failure to finish without adding it twice. Before the first YouTube video or song, RIGHTS_UNCONFIRMED: tell the user they are responsible for the rights to the video and for following YouTube's terms, ask with suggest_options, and only after they agree call again with rights_confirmed true; Nuzky remembers it. INVALID_LINK or PLAYLIST: supply one video link; INVALID_ARGUMENTS: fix the part or placement; YTDLP_MISSING or YTDLP_OUTDATED: ask the user to install or update yt-dlp through Nuzky's sound library. SOUND_UNAVAILABLE means private, removed, too large or unplayable video; OFFLINE, RATE_LIMITED or SOURCE_REFUSED mean try again later; YTDLP_FAILED or SOURCE_FAILED mean yt-dlp could not complete the request; CANCELLED means the run stopped; REQUEST_CONFLICT means request_id was reused with different arguments.",
        )?,
        tool::<params::AddSticker>("add_sticker", &STICKER_DESCRIPTION)?,
        tool::<params::Inspect>(
            "inspect_frames",
            "Render 1–16 timeline times_us as ONE PNG contact sheet with timestamp labels and revision. Or, instead of times_us, sample: \"changes\" picks the frames itself: in range_us=[start,end) (whole timeline by default) it looks at a candidate every 0.25 s (at most 120, further apart in a longer range; candidate_every_us), compares their perceptual hashes and keeps a frame only if it differs from each of the last four kept by more than min_change of 64 bits (default 16; gestures within one shot mostly stay below it, a cut to other footage is 30 or more), at most max_frames (1-16, default 16) per page. It returns times_us of the kept frames, skipped (candidates that looked like a kept frame) and next: pass it as cursor, with sample and without range_us or min_change, for the next page without repeats; null when the range is done. A static talking head gives one frame per shot. width is per-frame pixels (96–1280; default 320). safe_area=true overlays translucent unsafe margins on vertical canvases to check captions and faces. Clips with a background need the person found in their frames first: MATTE_NOT_READY names the job doing it; wait with job action wait, then inspect again. When the face and person models are missing, the open Nuzky app asks the user while the job waits (phase waiting_for_user, then downloading_vision_models); keep waiting. MODEL_DECLINED means the user said not now or stopped it: do not call tools needing those models again, carry on without them and mention that Get set up downloads them. Without the app MODEL_MISSING says what to tell the user. Missing media or times outside the timeline are errors.",
        )?,
        tool::<params::Activity>(
            "activity",
            "Read, as text and cheaply, where the picture and sound change, before looking at frames. range_us=[start,end) (whole timeline by default) is split into points steps (8-200, default 60) of step_us; point i covers [start + i*step_us, start + (i+1)*step_us). change: the biggest jump between consecutive looks at the picture within the step (a look every look_every_us at a tiny render of the timeline as exported); motion: how much the picture moves within 0.1 s, averaged over the step; both are the mean colour difference in percent, the measure of analyze scenes, where 18 or more is a hard cut. loudness_db: RMS dBFS of the timeline mix over the step, -120 is silence. peaks lists the strongest local maxima of each curve, {t_us, value} in time order; a change peak's t_us is the look where the new picture showed. A longer range is looked at more sparsely rather than for longer, so a fast pan can score like a cut and a very short insert can fall between looks: confirm with inspect_frames before cutting. AUDIO_NOT_READY names a job that prepares the sound of the files; wait with job action wait, then call again. Missing media is an error.",
        )?,
        tool::<params::Waveform>(
            "get_waveform",
            "Hear your cuts: draw range_us=[start,end) of the timeline (at most 60 s; a second or two around a cut reads best) as ONE PNG. The waveform is the speech as heard (the sound of the heard video clips, music left out), peaks from -60 dBFS so breaths and room noise show; under it the transcript's words at their times. At every cut an orange line with its time, and shaded beyond it up to 0.5 s of the source the cut threw away on each side: the rest of the clip that stops there, then the lead-in of the clip that starts there, the whole skipped part when it is under 1 s, a dashed line where more lies between or another file starts. Sound that runs on from a line into the shade was cut off, such as the end of a word; sound kept right beside a line with no word over it is usually a breath or noise the cut kept, though word times can miss a syllable. Words in the shade are cut out, red words are split by a cut. The text part lists cuts {t_us, split_words} and untranscribed asset_ids, whose words are missing. width: pixels, 400-2400, default 1200. AUDIO_NOT_READY names a job that prepares the sound of the files; wait with job action wait, then call again. Missing media is an error.",
        )?,
        tool::<params::Analyze>(
            "analyze",
            "Local analysis. retakes needs no asset_id and answers at once: it reads the stored words of the whole timeline, numbered exactly as get_transcript (same transcript_key), and fails with TRANSCRIPT_MISSING while a heard clip is untranscribed. It returns groups of attempts at one restarted sentence, also across clips: sentences {from,to,start_us,end_us,text,complete}, keep (index of the last complete attempt, the recommended one) and delete (the other attempts); fillers {from,to,text}: hesitation sounds recognition wrote (\"ehm\") and words such as \"jakoby\" or \"like\" set apart by a comma or a pause, at the start of a sentence or, set apart on both sides, inside one; only in Czech and English; covering only the filler words; unfinished {from,to,text}: words that trail off (recognised with an ellipsis) before a new sentence starts; restarts inside one sentence without a full stop are groups too; review: consecutive near-repeats that differ in a negation, a number or a content word, and a sentence that says again the last clause of the sentence before it (after a comma, semicolon or colon; the earlier attempt starts after it), with a reason, never deleted for you; suggested_delete: all group deletes, fillers and unfinished attempts, sorted and merged, INCLUSIVE word ranges ready for edit_transcript delete. Same words give the same result. Read the groups against the text before deleting. emphasis also needs no asset_id and answers at once, with the same word numbering and transcript_key: zooms {from,to,start_us,end_us,text,score,scale} are sentences for a punch-in, scored by how much louder they are than the median sentence (measured in the files' sound over the words), an exclamation and the opening hook; only sentences of 1.2-8 s with three words or more, at least 5 s apart and about one per 12 s of timeline; scale 1.15-1.3 by score. Same words and sound give the same result. AUDIO_NOT_READY names a job that prepares the sound of the files; wait with job action wait, then analyze again. Pass the chosen zooms to apply_zooms. fillers without asset_id answers at once the same way: fillers as in retakes, transcript_key and hesitations {after_word,start_us,end_us}: voice of at least 0.15 s between two words of one clip that recognition did not write, such as an \"eee\", parted from both words by quiet; cut the ones you keep out with edit_transcript delete_hesitations. The other kinds need asset_id, start a job (wait with job action wait) and use source microseconds. silences: threshold_db, min_silence_us (400000), pad_us (120000); loudness: window_us (100000, wider for media over ten hours; at most 360000 values), RMS dBFS per window plus integrated_lufs (ITU-R BS.1770, null for silence) and true_peak_dbtp of the whole file; scenes: threshold (0.18), min_gap_us (300000); fillers of one asset: ranges of dictionary fillers and stutters in its stored words (Czech and English) and hesitations, transcribe first. Review filler suggestions in context. thumbnail_frames needs no asset_id and runs as a job (phases looking, scoring): it picks frames of the whole timeline worth a cover or thumbnail, looking at the picture as exported (colours, zooms, overlays) without text tracks. A look every 0.25 s (sharpness, exposure, a face) shortlists about 40 moments away from cuts, transitions and clip animations; a closer look scores those. Result candidates (6-8, best first, at least 2 s apart, fewer in a short timeline) {time_us, score, parts, faces, subject_box, crops}: parts are 0..1, higher is better: sharpness, exposure, and with a face eyes_open, mouth (not caught mid-word), facing (eyes and head toward the camera), smile, framing_9x16 (face size, inside the Reels/TikTok safe area and the 3:4 profile grid) and framing_16x9 (size, headroom, clear of YouTube's duration badge); params.format \"9:16\" or \"16:9\" decides which framing counts (the canvas shape by default). faces [{box,confidence}] and subject_box (the person) are [x,y,w,h] canvas pixels, crops the 9:16 and 16:9 cuts of the canvas around the face. Look at the top candidates with inspect_frames before choosing. When the face and person models are missing, the open Nuzky app asks the user while the job waits (phase waiting_for_user, then downloading_vision_models); keep waiting. MODEL_DECLINED means the user said not now or stopped it: do not call tools needing those models again, carry on without them and mention that Get set up downloads them. Without the app MODEL_MISSING says what to tell the user. VISION_UNAVAILABLE means covers cannot run on this computer; tell the user, editing works as before.",
        )?,
        tool::<params::SegmentSubject>(
            "segment_subject",
            "Mask the subject of the timeline frame at time_us, or with format instead the picture of that cover (its image, or its frame), as a job (phases rendering, waiting_for_other_mask, loading_model, segmenting; masks are made one at a time): the person when there is one, otherwise the most prominent object, for placing text behind them on a cover. The frame is rendered as exported without text tracks. Result: mask_path (8-bit grayscale PNG in Nuzky's cache at canvas resolution, or the image's own for a cover made from an image; white = subject), width, height, subject_box [x,y,w,h] in those pixels (null when nothing stands out), subject_share (share of the canvas the subject covers, 0..1), person (a face is in the frame) and cached. The same frame, edit and model reuse the cached mask. Takes a few seconds on the CPU. When the face and person models are missing, the open Nuzky app asks the user while the job waits (phase waiting_for_user, then downloading_vision_models); keep waiting. MODEL_DECLINED means the user said not now or stopped it: do not call tools needing those models again, carry on without them and mention that Get set up downloads them. Without the app MODEL_MISSING says what to tell the user. VISION_UNAVAILABLE as for thumbnail_frames.",
        )?,
        tool::<params::Reframe>(
            "reframe",
            "Reframe a wide talking head for Reels, TikTok or Shorts inside run_id as a job; wait with job action wait. format is \"9:16\" (1080x1920), \"4:5\" (1080x1350) or \"1:1\" (1080x1080). Optional clip_ids selects video/image clips; by default moves main-track clips and overlays that filled the canvas. Follows the speaker's face its crop leaves visible, centres clips without a face, sets the picture upright, keeps a punch-in zoom and replaces the moved clips' keyframes. Applies the canvas and clips as one edit, undone with the run. Result: format, width, height, followed and centred clip ids, new revision. If the project changed meanwhile, call reframe again. When the face and person models are missing, the open Nuzky app asks the user while the job waits (phase waiting_for_user, then downloading_vision_models); keep waiting. MODEL_DECLINED means the user said not now or stopped it: do not call tools needing those models again, carry on without them and mention that Get set up downloads them. Without the app MODEL_MISSING says what to tell the user. Stopping the run cancels the job.",
        )?,
        tool::<params::Transcribe>(
            "transcribe",
            "Recognise source files locally and store reusable word timestamps. Omit asset_ids to recognise every heard asset lacking a stored transcript (includes detached sound). Supply asset_ids to re-recognise them. language defaults to auto. model defaults to the most accurate installed Whisper model (large-v3-turbo-q5_0, small, base); an absolute local model path is accepted. When none is installed and model is left out, the open Nuzky app asks the user whether to download one: the job's phase is waiting_for_user, then downloading_speech_model, and it goes on by itself; keep waiting. MODEL_DECLINED: the user said no or stopped the download; do not call transcribe again, tell the user what you can do without a transcript. Without the app MODEL_MISSING says what to tell the user. The word timing model of the recognised language (Czech, English) is downloaded once, then word times are measured in the sound; without it they are Whisper's estimates aligned to pauses. Returns job_id; wait with job action wait and call it again while running (phases transcribing, downloading_word_timing_model, measuring_word_times). Result assets lists asset_id, words count, language and word_times: measured or estimated. Then use get_transcript.",
        )?,
        tool::<params::GetTranscript>(
            "get_transcript",
            "Read derived timeline speech: transcript_key, revision, words {i,start_us,end_us,text,p}, sentences {from,to,start_us,end_us,text}, pauses {after_word,gap_us}, untranscribed asset ids. i/from/to are global zero-based INCLUSIVE word indices valid for this transcript_key; p is recognition probability. Sentences split at phrase punctuation or gaps >=600000 us; pauses include gaps >=300000 us. Optional range_us=[start,end) filters results without renumbering. Read long videos cheaply with detail: \"sentences\" and limit (1-500), passing next_cursor as cursor until it is null; pass the same range_us on each page. Request full word-level detail only for the part you cut. Use word indices with edit_transcript; never map source times by hand.",
        )?,
        tool::<params::EditTranscript>(
            "edit_transcript",
            "Cut by INCLUSIVE zero-based word ranges [[from,to],...]. Requires the current get_transcript transcript_key (includes recognition contents) and run_id unless dry_run. Optional request_id: reuse it with identical arguments after a save failure to finish the original save without cutting again; omitted ids are generated. Supply delete OR keep; omit both to shorten pauses only. delete_hesitations [after_word,...] also cuts those hesitations from analyze fillers, like deleted words. shorten_pauses_us left out is the creator's style's (EDIT.md Settings, edit_transcript shorten_pauses_us), else 300000. Removed passages take surrounding silence, kept passages retain at most 80 ms before and 120 ms after a boundary word, counted from where the word is heard: recognition moves word boundaries into the silence between words, so cuts do not clip syllables, and every cut crossfades over 20 ms through quiet sound; internal long pauses lose their middle. No boundary lands inside a word. One ripple edit respects tracks kept in place. dry_run=true simulates without changing project/history and needs no open run, so plan before begin_run. Returns duration_us {before,after}, removed_us, ranges (engine startUs/endUs), preview_text (first 400 characters), predicted/new transcript_key, revision, dry_run, from_style {shorten_pauses_us} when the style gave it and style_warnings when EDIT.md could not be read or a Settings row could not be used, which then keeps the default. Apply using the ORIGINAL transcript_key after checking the dry run. SPEECH_CHANGED rejects stale speech; overlapping speech may be rejected.",
        )?,
        tool::<params::CorrectWords>(
            "correct_words",
            "Correct misrecognised words, e.g. \"oka\" written for a spoken \"okna\": corrections [{i, text, expected}] with i from get_transcript, the text the word should read, punctuation included, and expected, the word's text as get_transcript shows it now (after earlier corrections, compared trimmed). Always pass expected: if any word reads otherwise, for example because i is off by one, the whole call is refused with WORD_MISMATCH and nothing changes. Requires run_id and the current get_transcript transcript_key (SPEECH_CHANGED when stale). The correction belongs to the project and to that recognised word, so get_transcript, analyze retakes, build_captions and the app's transcript all read it, also after rebuilding captions; it changes no timing and cuts nothing. Caption clips on the Captions track that show the word get it replaced in the same step, keeping style and timing. A word build_captions split over several captions changes in each: every part keeps its number of words, the last takes any extra, and a caption left without a word is deleted. That happens only while every part still reads exactly as build_captions wrote it; when a part was deleted, moved off the word's time or edited by hand, none changes and they come back in captions_kept with a note, since build_captions makes them again from the corrected word. A word heard in several clips is corrected everywhere. Text equal to the recognised word removes the correction. If the file is recognised again differently, the correction no longer applies. Optional request_id: reuse it with identical arguments after a save failure to finish the original save. Returns words {i, before, after} for every place a word is heard, captions_changed (clip ids), captions_kept when there are any, and the new transcript_key.",
        )?,
        tool::<params::Job>(
            "job",
            "Read job progress/result or request cancellation. Includes owner, kind, snapshot revision and session_epoch. Use action wait (timeout_s defaults to 25, at most 30) and call it again while running. elapsed_s and remaining_s are whole seconds; remaining_s is null until enough progress has been measured. Tell the user once what runs and roughly how long it takes from remaining_s. After that, write only when something changes. Never ask whether to keep waiting. Cancellation is cooperative after the current engine operation; completed assets remain stored when a later asset is cancelled.",
        )?,
        tool::<params::Captions>(
            "build_captions",
            "Build captions from stored words mapped through every heard clip, including detached audio and speed changes. No caption spans a clip cut. Requires run_id. max_words and max_chars left out are the creator's style's (EDIT.md Settings, build_captions max_words and max_chars), else 2 and 15; a single longer word stays intact. max_words counts the words shown: a corrected word with spaces counts each, and one with more words than max_words is split over consecutive captions that share its time by length, unless a part would last under a frame. Adds or replaces ONE Captions track, preserving other text. Multiple Captions tracks require explicit replaceCaptions. Without style or style_preset the captions take the creator's learned look (build_captions style, its sizes for a 1080 px short side scaled to the canvas), else the Reel style: size 95, white, regular, black stroke 7.5, no background, Inter. The result's from_style names each value taken from the style; style_warnings says when EDIT.md could not be read or a Settings row could not be used, which then keeps the default. style_preset picks a built-in style package or a saved style from My styles by name: its look, font when it has one, entry/exit animations for every caption and key word colour. karaoke and green_box highlight the spoken word. Key words are numbers and words said louder than the median word, scored from sound as analyze emphasis does. They are marked on caption words when captions are made, so they survive regeneration and word corrections. Loudness counts for files whose sound is prepared, as it is after transcribe; it never waits for sound. Karaoke captions store each word's time and highlight only while a word is spoken; updateClip text with as many words keeps that timing, adding or removing words turns the caption's highlight off. Give style or style_preset, not both. warnings names a font this computer does not have; the captions are then drawn in Inter. Vertical canvases automatically wrap to the IG/TikTok safe width. transform places the new captions, only the fields given, e.g. {\"y\": 0.3} (x and y offset the centre from the canvas centre as a fraction of its width and height; default y 0.15); without it, rebuilt captions stay where the captions they replace were. To restyle or move captions that exist, use apply_edits restyleTrack, which keeps captions edited by hand; rebuilding replaces them. Inspect frames with safe_area=true.",
        )?,
        tool::<params::ApplyZooms>(
            "apply_zooms",
            "Punch in on INCLUSIVE word ranges as one edit of the run: zooms [{from,to,scale}] such as analyze(kind: \"emphasis\").zooms, with the current transcript_key (SPEECH_CHANGED when stale). Each zoom becomes a main-track timeline range from midway into the silence before its first word to midway into the silence after its last, at most 150 ms out, or on to the cut when only silence lies between; main-track clips are split there and the pieces inside get their scale multiplied by scale, keeping position and rotation. An edge that would leave a piece under 0.3 s, or cut a transition, animation or fade short, moves to the clip's edge. Sound plays on unchanged. Clips with keyframes are left alone and listed in skipped. Zooms must not overlap; scale 0.25-4. Requires run_id. Optional request_id: reuse it with identical arguments after a save failure to finish the original save without zooming twice. Returns ranges (startUs, endUs, scale), skipped clip ids, the new transcript_key and revision.",
        )?,
        tool::<params::ApplyMotion>(
            "apply_motion",
            "A slow camera move as one edit of the run, without working out keyframes: kind pushIn zooms in, pullOut starts zoomed in and ends on the clip's own framing, kenBurns zooms in while drifting right; strength is how far, 0.06 subtle, 0.1 or 0.15 strong (0.02-0.5). clip_id moves one video or image clip on any track, over the part of it inside range_us or its whole length; without clip_id, range_us moves every main-track video and image clip under it as one continuous motion, e.g. one sentence from get_transcript start_us of its first word to end_us of its last. Each clip gets two keyframes on a smooth curve (ease \"smooth\": it starts and ends gently) from its own transform, editable afterwards with setKeyframes. The zoom centres on the middle of the IG/TikTok safe area on vertical canvases, else on the canvas centre, so a face framed there stays put. Clips that already have keyframes are left alone and listed in skipped; remove their keyframes first to replace them. Requires run_id. Optional request_id: reuse it with identical arguments after a save failure. Returns revision, changed clip ids and skipped.",
        )?,
        tool::<params::ProposeReels>(
            "propose_reels",
            "Reels from a long video: propose moments of the transcript that each work as a reel on their own, as one edit of your run. Read the video first with get_transcript(detail: \"sentences\") page by page. candidates [{from, to, title, why, score}] are INCLUSIVE word ranges of the current transcript_key (SPEECH_CHANGED when stale): from starts a sentence and to ends one (REEL_BOUNDARY; a comma ends no sentence), inside the transcript (REEL_OUTSIDE_TRANSCRIPT), sharing no words with each other (REEL_OVERLAP; reels already made do not count, since cuts may have moved their words), and at most max_duration_us long once made (default 60000000; REEL_TOO_LONG), measured as make_reels cuts: the words with at most 80 ms before and 120 ms after, longer pauses shortened to shorten_pauses_us. Leave shorten_pauses_us out unless the user or the creator's rules ask for another pause length in reels: it is then the creator's style's (EDIT.md Settings, edit_transcript shorten_pauses_us), else 300000, and from_style names it when the style gave it. Each candidate keeps its pause length as shortenPausesUs, so trimming and making it later cut the same way whatever the style says by then. title up to 100 characters in the video's language; why up to 300: what makes it work alone, such as a strong opening line, a payoff, one clear idea; score 0-1. Replaces the reels proposed before; made ones stay. Returns candidates with id, from, to, start_us, end_us, hook (the sentence the reel opens with), duration_us, shortenPausesUs and status proposed; get_state lists them as reel_candidates and the user picks them in the app. Requires run_id. Optional request_id: reuse it with identical arguments after a save failure.",
        )?,
        tool::<params::MakeReels>(
            "make_reels",
            "Make each reel candidate in ids (get_state.reel_candidates) a project of its own beside this one, as one edit of your run: the same media files, never copied, only the candidate's words, cut as edit_transcript keep cuts them with the pause length each was proposed with (its shortenPausesUs; 300 ms when it has none), without the clips before and after them, on a 1080x1920 canvas. canvas crop (default) fills the frame with the middle of the picture; blur fits the whole picture with a blurred copy of it behind. This project changes only in those candidates: status made and project_path. Returns reels [{id, path, duration_us}], and warnings when the reels' text uses a font this computer does not have, one per font: that text is drawn in Inter. Make what the user picked (status picked) unless they asked for others. Each reel is its own project: captions, covers and export happen in it, through an MCP server started on its path, since this server edits only this project. UNKNOWN_REEL; REEL_MADE when it is made already; SPEECH_CHANGED when the video changed since it was proposed, so the reel would not be what was proposed: propose again. Requires run_id. Optional request_id: reuse it with identical arguments after a save failure; the projects written then are kept.",
        )?,
        tool::<params::Export>(
            "export_video",
            "Start a LOCAL H.264/AAC export of an immutable snapshot; job reports its revision. Presets (size, fps, required canvas): reels for Instagram Reels/TikTok (1080x1920, 30, 9:16); shorts (1080x1920, project fps, 9:16); youtube_1080p (1920x1080, project fps, 16:9); youtube_4k (3840x2160, project fps, 16:9); instagram_feed (1080x1350, 30, 4:5); square (1080x1080, 30, 1:1). YouTube and Shorts keep project fps or the given fps, max 60; the others fix 30. All presets write H.264 High, AAC 48 kHz stereo, sound levelled to -14 LUFS with true peak <= -1 dBTP in the file (the project and preview keep their levels). With a preset, resolution/fps may be omitted; given resolution must match its short side and given fps must meet its rules. Without preset resolution (the SHORT side in pixels, 1080 gives 1080x1920 on a portrait canvas) and fps=1..240 are required. quality high/recommended/small, default recommended. path must be new; relative paths resolve beside the project. Requires --allow-write. Job phases: finding_the_person (clips with a background, the first time), measuring_loudness (preset only), exporting. When the face and person models are missing, the open Nuzky app asks the user while the job waits (phase waiting_for_user, then downloading_vision_models); keep waiting. MODEL_DECLINED means the user said not now or stopped it: do not call tools needing those models again, carry on without them and mention that Get set up downloads them. Without the app MODEL_MISSING says what to tell the user. Wait with job action wait until done before reporting success.",
        )?,
        tool::<params::InspectThumbnail>(
            "inspect_thumbnail",
            "Draw the project's thumbnail of format (cover_9x16 is 1080x1920, youtube_16x9 1280x720) as ONE PNG, width pixels wide (96 up to its own width, half by default), exactly as export_thumbnail writes it, with texts [{text, behind, hidden}]: hidden is the share of a text behind the person that they cover, 0..1, always 0 in front; about 0.05-0.3 lets the head overlap the letters while they stay readable. safe_zones=true tints what the apps cover: on a cover red for the Reels interface and blue outside the 3:4 middle the profile grid shows; on a YouTube thumbnail red for the duration badge in the bottom right corner. Only reads. Text behind the person, an outline, or a background that is blurred, dimmed or a colour needs the mask of the person in the frame, or in the image a cover made from one shows; when it is not in the cache yet MASK_NOT_READY names the job making it: wait with job action wait, then inspect again. THUMBNAIL_MISSING: apply_edits setThumbnail first. INVALID_RANGE: cuts left the frame past the end; set timeUs again. MEDIA_MISSING names a file the cover is drawn from, also an overlay's: ask the user to put it back, or drop that overlay. When the face and person models are missing, the open Nuzky app asks the user while the job waits (phase waiting_for_user, then downloading_vision_models); keep waiting. MODEL_DECLINED means the user said not now or stopped it: do not call tools needing those models again, carry on without them and mention that Get set up downloads them. Without the app MODEL_MISSING says what to tell the user. VISION_UNAVAILABLE as for segment_subject.",
        )?,
        tool::<params::InspectImage>(
            "inspect_image",
            "Show an image of the project (get_state assets of kind image) as ONE PNG, size pixels on its long side (768 by default), as Nuzky draws it, turned as the file says; transparency shows black. Use it to look at a logo or a picture the user imported, also when it is not on the timeline, for example before handing it to an image generator as a reference. Only reads. UNKNOWN_ASSET: the id is not an image of this project. MEDIA_MISSING: the file is gone; ask the user to put it back.",
        )?,
        tool::<params::ExportThumbnail>(
            "export_thumbnail",
            "Write the thumbnail of format at its full size to a NEW .png, .jpg or .jpeg file as a job (phases rendering, and while the mask of the person is made waiting_for_other_mask, loading_model, segmenting); wait with job action wait until done. JPEG keeps the best quality under 2 MB, YouTube's limit, so use .jpg for youtube_16x9. Relative paths resolve beside the project; OUTPUT_EXISTS when the file is there. Result: path, width, height, bytes and texts with hidden as inspect_thumbnail. When the face and person models are missing, the open Nuzky app asks the user while the job waits (phase waiting_for_user, then downloading_vision_models); keep waiting. MODEL_DECLINED means the user said not now or stopped it: do not call tools needing those models again, carry on without them and mention that Get set up downloads them. Without the app MODEL_MISSING says what to tell the user. Requires --allow-write.",
        )?,
        tool::<params::GetStyle>(
            "get_style",
            "Read the creator's editing style as the app's Your style page shows it: text is EDIT.md exactly as every agent gets it (also nuzky://style), own the creator's own rules in their words, rules the learned rules in it with a one-line summary (byYou when they changed one by hand, so learning leaves it alone; confidence only while learning makes the same choices; state: backed when it does, changeSuggested when learning would change it and the change waits in suggestions, kept when learning finds other choices but the creator kept the rule by rejecting the change, reverting it or changing it by hand, notSeen when learning no longer finds it), suggestions what learning found that is not in it yet (update when it would change a rule: changes from and to, confidence by videos and moments, the first moments, the text it would add), notLearned the rules learning leaves alone and why, sources the videos learned from, versions the newest changes, and version, which setText needs. The style is about all of the creator's videos, not the open project. Read-only.",
        )?,
        tool::<params::ChangeStyle>(
            "change_style",
            "Change the creator's editing style, which every agent follows in all their videos, once the creator agreed to that exact change. action.type: setOwn {text} keeps an instruction in the creator's own words as their rule, the way to keep anything they tell you about how to edit; with index it changes their rule there, and empty text removes it. accept {titles} or reject {title} a suggestion from get_style; remove {title} or revert {title} a learned rule; learnAgain {title} lets learning suggest it again; setText {text, baseVersion} replaces the whole EDIT.md, so keep everything you do not mean to change (STYLE_CHANGED when the style changed since get_style; read it again); restore {index} brings back a version. The app shows each change at once as a version starting with AI: that the creator can undo; going back to the default style is theirs to do in the app. Needs --allow-write; no run, and no project changes. Returns the style as get_style does.",
        )?,
        tool::<params::SuggestOptions>(
            "suggest_options",
            "Only when you run as Nuzky's AI panel (your instructions say so): offer the user 2-6 choices when a decision is theirs to make, e.g. which take to keep, cut tight or loose, which caption style. They show as buttons under your message, and the label picked arrives as the user's next message. Labels are short and distinct, at most 80 characters, in the user's language; detail is one optional line on what each choice does. Call it as the last thing in your turn, end the turn right after, and do not repeat the options in text. Anywhere else, such as a terminal, nobody sees the buttons: ask in plain text instead. Never offer choices for something you can decide from the guide or the user's style.",
        )?,
    ])
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_resources().enable_prompts().build())
            .with_server_info(Implementation::new("nuzky", env!("CARGO_PKG_VERSION")))
            .with_instructions(guide())
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.as_ref().clone()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|t| t.name == name).cloned()
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        Ok(self
            .target
            .clone()
            .call(request.name.to_string(), Value::Object(request.arguments.unwrap_or_default()))
            .await
            .into())
    }

    async fn list_resources(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let mut resources = vec![
            Resource::new("nuzky://guide", "Editing guide").with_mime_type("text/markdown"),
            Resource::new("nuzky://schema", "Project JSON schema").with_mime_type("application/schema+json"),
        ];
        if style().is_some() {
            resources
                .push(Resource::new("nuzky://style", "This creator's editing style").with_mime_type("text/markdown"));
        }
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let text = match request.uri.as_str() {
            "nuzky://guide" => guide(),
            "nuzky://schema" => schema_for!(nuzky_engine::Project).to_value().to_string(),
            "nuzky://style" => style().ok_or_else(|| {
                ErrorData::invalid_params("No EDIT.md yet: Nuzky writes one as it learns from the creator's videos (Your style in the app, or nuzky style learn)", None)
            })?,
            _ => return Err(ErrorData::invalid_params("Unknown Nuzky resource", None)),
        };
        Ok(ReadResourceResult::new(vec![ResourceContents::text(text, request.uri)]).into())
    }

    async fn list_prompts(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::with_all_items(vec![
            Prompt::new(
                "edit_selected",
                Some("Edit the selected clips toward a goal"),
                Some(vec![PromptArgument::new("goal").with_required(true)]),
            ),
            Prompt::new(
                "thumbnail",
                Some(
                    "Make a Reels and TikTok cover and a YouTube thumbnail from the best frame, with a hook, and export them",
                ),
                Some(vec![
                    PromptArgument::new("wishes")
                        .with_description("Anything to do differently, e.g. the hook text or a colour")
                        .with_required(false),
                ]),
            ),
            Prompt::new(
                "rough_cut",
                Some("Rough-cut the whole project into a Reels and TikTok video with captions and export it"),
                Some(vec![
                    PromptArgument::new("wishes")
                        .with_description("Anything to do differently, e.g. keep the closing call to action")
                        .with_required(false),
                ]),
            ),
        ]))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        let argument = |name: &str| request.arguments.as_ref().and_then(|a| a.get(name)).and_then(Value::as_str);
        let wishes = argument("wishes").filter(|w| !w.trim().is_empty()).unwrap_or("none");
        if request.name == "rough_cut" {
            return Ok(GetPromptResult::new(vec![PromptMessage::new_text(
                Role::User,
                format!("{ROUGH_CUT}\n\nThe user's wishes, which win over the steps: {wishes}\n\n{}", guide()),
            )])
            .into());
        }
        if request.name == "thumbnail" {
            return Ok(GetPromptResult::new(vec![PromptMessage::new_text(Role::User, thumbnail_prompt(wishes))]).into());
        }
        if request.name != "edit_selected" {
            return Err(ErrorData::invalid_params("Unknown prompt", None));
        }
        let goal = request
            .arguments
            .as_ref()
            .and_then(|a| a.get("goal"))
            .and_then(Value::as_str)
            .ok_or_else(|| ErrorData::invalid_params("goal is required", None))?;
        Ok(GetPromptResult::new(vec![PromptMessage::new_text(Role::User, format!("{goal}\n\nRead get_state and use its selection. Headless selection is empty: use the whole timeline.\n\n{}", guide()))]).into())
    }
}

---
name: nuzky-plugin
description: Write, try and fix a Nuzky plugin, a small script that adds an edit Nuzky lacks, through Nuzky MCP.
---

# Write a Nuzky plugin

A plugin is a small JavaScript file that reads the project and returns edits. The user asks for an edit Nuzky does not have ("delete every clip shorter than a second", "put my end card on every video"), you write it once, and it becomes a button in the app's Plugins panel that they can run on any project, with inputs they set. Plugins live in Nuzky's own data folder, so they survive updates, and a project a plugin edited opens anywhere without it.

## What a plugin can and cannot do

- It reads: the project as `get_state` gives it, the clips the user selected, the playhead, the values of its inputs and, when it asks for them, the words of the transcript and the silences, loudness and beats Nuzky finds in the sound.
- It returns edits, the same EditCmd JSON as `apply_edits`. Nuzky applies them as ONE batch: all of them or none. The user undoes them in one step.
- It cannot read or write files, reach the network, start processes, use timers or import modules. There is no `fetch`, `require`, `import`, `setTimeout` or `process`. `Math`, `Date`, `JSON`, strings, arrays and regular expressions work. `run` must return its result, not a Promise.
- It cannot add media of its own choosing: `addAssets` is refused, so are reel candidates pointing at a new project file. It works with the assets the project already has (`addClip`, `replaceClip` with an existing `assetId`) and with its own pictures and sounds, such as a logo or a beep (Pictures and sounds below).
- The user can turn it on to run by itself after every import they make in the app, or before every export, with the inputs they last set. Only the user turns that on, in the app; you and the plugin cannot. Such a run gets no selection and the playhead at 0, and may meet the same project again, after another import or the next export. Write a plugin meant for that to change nothing when its work is already done, such as a logo already in the corner, and say so in `message`.
- It runs at most 60 seconds, with 256 MB of memory for the script. It runs in a process of its own, where Nuzky also applies its edits to a copy of the project first; that process may take at most 1 GB. plugin.js is at most 1 MB and a run returns at most 5000 edits.

## Files

`save_plugin` writes them into `plugins/<id>/` of Nuzky's data folder: `plugin.json` (the manifest), `plugin.js` and an optional `README.md` the app shows. Never write them yourself.

```json
{
  "nuzkyPlugin": 2,
  "id": "remove-short-clips",
  "name": "Remove short clips",
  "description": "Deletes clips shorter than a length you choose, such as stray frames left between cuts.",
  "version": "1.0.0",
  "action": "Remove clips",
  "needs": [],
  "inputs": [
    { "type": "number", "id": "minSeconds", "label": "Shorter than", "min": 0.1, "max": 10, "step": 0.1, "default": 1, "unit": "s" }
  ]
}
```

- `nuzkyPlugin` is the plugin API, 2; plugins made for 1 still run, but one that ships files or needs silences, loudness or beats needs 2. `list_plugins` gives the `api` this Nuzky runs, and an older Nuzky refuses a plugin for a newer API with PLUGIN_API_NEWER.
- `id`: 1 to 40 lowercase letters, digits and dashes; also the folder's name. Saving with the same id replaces the plugin and keeps the old files as plugin.prev.json and plugin.prev.js.
- `name` (at most 40 characters), `description` (300), `version` (20) and `action`, the verb on its button (24, "Run" when left out), or `actions` for several buttons (A richer panel below). Write them in the user's language, plainly, saying what will happen.
- `needs`: what the plugin reads beyond the project, any of `"words"` (the transcript), `"silences"`, `"loudness"` and `"beats"` (found in the sound; these three need `nuzkyPlugin` 2). Ask only for what `run` uses: finding them in a long video takes a while the first time.
- `inputs`, at most 20, each with `id` (letters, digits and `_`), `label` (at most 40 characters), `type` and `default`:
  - `number`: `min`, `max`, `step`, `default`, optional `unit` such as `"s"` or `"%"`.
  - `toggle`: `default` true or false.
  - `choice`: `options` `[{"value": "main", "label": "Main track"}]`, `default` one of the values.
  - `text`: `default`, at most 500 characters.
  - `color`: `default` such as `"#FFCC00"`.
  - `multi` (API 2): checkboxes, `options` as for `choice` and `default` a list of the values checked, such as `[]`. Its value is that list.
- `files`: the plugin's own pictures and sounds by file name, such as `["logo.png"]` (Pictures and sounds below).

Ask the user only for what changes between runs. A value the plugin can work out from the project is not an input.

### A richer panel

With `"nuzkyPlugin": 2` a plugin with many inputs can group them, show some only when they matter, offer rows it finds in the project and have more than one button. The panel is data: Nuzky draws it with its own controls and shows every text as plain text, so markup, links and styles in a label show as the characters they are. There is no HTML and no script in the panel, and an unknown field such as `html` makes plugin.json invalid.

```json
{
  "nuzkyPlugin": 2,
  "id": "title-cards",
  "name": "Title cards",
  "description": "Puts a title card before the clips you choose.",
  "version": "1.0.0",
  "actions": [{ "id": "add", "label": "Add cards" }, { "id": "remove", "label": "Remove cards" }],
  "inputs": [
    { "type": "choice", "id": "clip", "label": "Before", "options": "dynamic" },
    { "type": "toggle", "id": "custom", "label": "Own text", "default": false },
    { "type": "text", "id": "text", "label": "Text", "default": "Part 1", "when": { "input": "custom", "equals": true } },
    { "type": "multi", "id": "look", "label": "Look", "options": [{ "value": "bold", "label": "Bold" }, { "value": "shadow", "label": "Shadow" }], "default": ["bold"] }
  ],
  "sections": [{ "label": "Text", "inputs": ["custom", "text"] }, { "label": "Style", "inputs": ["look"] }]
}
```

- `actions`: 1 to 4 buttons `{id, label}`, the first the prominent one. They replace `action`. `ctx.action` is the `id` of the one the user pressed, so `run` can do something else for each, such as add or remove. The preview shows what the first one would do. A plugin without `actions` has one, `run`, with `action` as its label. `id` is 1 to 40 letters, digits, `-` and `_`; `label` at most 24 characters.
- `sections`: at most 10 groups `{label, inputs}` with a heading of at most 40 characters, in this order, each input in at most one. Inputs in no section come first, without a heading.
- `when: {input, equals}` on an input shows it only while an earlier `toggle` or `choice` with listed options has that value, or one of a list of values (`"equals": ["fast", "faster"]`). A hidden input still has its value in `ctx.inputs`, so check the same condition in `run`.
- `"options": "dynamic"` on a `choice` or `multi`: the rows come from the project, such as its clips or texts. Leave out `default`; the choice starts with none, `""`, and the checkboxes with `[]`. Return the rows from `run` in `items`, every time it runs: `{edits, message, items: {clip: [{value: "clip-2", label: "Interview"}]}}`, at most 500 rows an input, `value` and `label` plain text of at most 100 characters without line breaks, values distinct. The preview's run gives the panel its rows. The real run gets them again from `run`, and a value that is no longer one of them, such as a clip deleted since, fails with INVALID_INPUT and changes nothing.

## plugin.js

```js
function run(ctx) {
  nuzky.progress(0.5);
  nuzky.log("clips:", ctx.project.tracks[0].clips.length);
  return { edits: [{ type: "renameProject", name: ctx.inputs.title }], message: "Renamed the project." };
}
```

`run(ctx)` returns an array of edits, or `{edits, message, items}`, where `message` is one sentence for the user, at most 500 characters, such as "Removed 6 clips.", and `items` the rows of dynamic inputs (A richer panel above). Return no edits and a message saying why when there is nothing to do.

`ctx`:

- `api`: 2.
- `project`: what `get_state` answers: `canvas`, `assets`, `tracks` (bottom to top; `tracks[0]` is the main track, id `"main"`), each with `clips` `{id, startUs, durationUs, content, animIn, animOut, keyframes, transitionIn}`, `duration_us`, `thumbnails`, `selection`, `playhead_us`. `content.type` is `"media"` (`assetId`, `sourceInUs`, `speed`, `volume`, `transform`…) or `"text"` (`text`, `style`, `transform`).
- `inputs`: every input's value by id, checked against the manifest; those the user did not set have their default.
- `action`: the `id` of the button pressed, `"run"` for a plugin without `actions`.
- `selection`: ids of the clips the user selected, maybe none.
- `playheadUs`: where the playhead is.
- `files`: each of the manifest's files by name, `{assetId, kind, width, height, durationUs}`, such as `ctx.files["logo.png"].assetId`. `kind` is `"image"` or `"audio"`; a picture has `durationUs` 0, a sound `width` and `height` 0.
- `words`: only with `"needs": ["words"]`, the timeline's words as `get_transcript` gives them: `{i, start_us, end_us, text, p}`. Without a transcript the run fails with TRANSCRIPT_MISSING, so the user transcribes first.
- `silences`, `loudness`, `beats`: only when `needs` names them. Nuzky finds them in the sound of every media clip whose file has sound, on every track, muted or not, and places them on the timeline through the clip: times are timeline microseconds within the clip, a sped-up clip's are shorter, and the part of the file a clip does not play is left out. Each entry names its `clip_id`, `track_id` and `asset_id`, so keep the clips you want, such as `track_id === "main"` for the main track's own sound. The levels are the file's, before the clip's volume and fades.
  - `silences`: `[{clip_id, track_id, asset_id, start_us, end_us}]` in timeline order, every quiet stretch of at least 0.1 s of the file, unpadded. Quiet is at most 6 dB above the level the quietest tenth of the file stays under, and never above -35 dBFS, so it follows the room's own noise. Choose your own minimum length, and keep a margin on each side so speech does not start abruptly.
  - `loudness`: one `{clip_id, track_id, asset_id, start_us, end_us, times_us, dbfs}` per clip, in track and clip order. `dbfs[i]` is the RMS level in dBFS of the window starting at `times_us[i]` and lasting until `times_us[i + 1]`, the last one until `end_us`; -120 is digital silence. A window is 0.1 s of the file, longer for files of many hours, counted from where the clip starts playing it, so the first and last measure what the clip plays to within a tenth of a window.
  - `beats`: one `{clip_id, track_id, asset_id, start_us, end_us, bpm, confidence, beats_us, downbeats_us, steady_from_us, drops, sections}` per clip, as `analyze` finds the beat. `bpm` is the tempo heard on the timeline. Under a `confidence` of 0.3 there is no beat worth cutting to, as in speech. `beats_us` is every beat in the clip, `downbeats_us` those that start a 4/4 bar; beats before `steady_from_us` continue the beat's grid back to the start without being heard. `drops` `[{at_us, rise_db}]` are bars where the music gets at least 6 dB louder; `sections` `[{at_us, level_db}]` start where the level changes by 6 dB, the first at the clip's start when it began before.

Host functions: `nuzky.progress(fraction)` reports 0..1 for a long run; `nuzky.log(...values)` keeps a line (objects as JSON), at most the last 200, which come back to you with the result and with an error.

## Times and edits

Every time is an integer number of microseconds: one second is `1000000`. Round what you compute with `Math.round`. Timeline times count from the start of the video, source times (`sourceInUs`) from the start of the file.

The edits apply in order, each to the project the edits before it left:

- The main track is magnetic: its clips sit back to back from 0. Deleting, trimming or splitting a main-track clip moves every later clip, and what lies over it on other tracks moves with it. Plan from the project in `ctx` and work from the end of the timeline backwards, or use one edit that takes all the times at once (`deleteClips` with every id, `rippleDeleteRanges` with every range).
- An edit cannot name a clip another edit of the same batch makes: a split's second half, an added text and a pasted clip get ids of their own. To add a clip with its own length, paste it whole (`pasteClips`).

The edits plugins use most, as JSON:

Delete clips; on the main track the gaps close.

```json
{"type": "deleteClips", "clipIds": ["clip-1", "clip-2"]}
```

Cut timeline ranges out of every track and close the gaps, so the picture, sound and captions stay together.

```json
{"type": "rippleDeleteRanges", "ranges": [{"startUs": 2000000, "endUs": 2500000}, {"startUs": 8000000, "endUs": 9000000}]}
```

Split a clip at a timeline time, and trim one: `startUs` and `durationUs` on the timeline, `sourceInUs` where in its file it starts.

```json
{"type": "splitClip", "clipId": "clip-1", "atUs": 1500000}
```

```json
{"type": "trimClip", "clipId": "clip-1", "startUs": 0, "durationUs": 1200000, "sourceInUs": 300000}
```

Move a clip to a time, optionally on another track.

```json
{"type": "moveClip", "clipId": "clip-3", "startUs": 4000000}
```

Change a clip: any of `transform`, `volume`, `text`, `style`, `speed`, `adjust`, `fadeInUs`, `fadeOutUs`, `cleanVoice`, `normalize`…; what you leave out stays.

```json
{"type": "updateClip", "clipId": "clip-1", "volume": 0.5, "fadeOutUs": 500000}
```

```json
{"type": "updateClip", "clipId": "title-1", "text": "New title", "transform": {"x": 0, "y": -0.3, "scale": 1, "rotation": 0, "opacity": 1}}
```

Add a text of 3 seconds at a time, or paste a whole text clip with its own length onto a free text track at `atUs`. A pasted clip needs `id`, `startUs` and `durationUs`, but gets a new id and lands at `atUs`.

```json
{"type": "addText", "startUs": 0, "text": "Hook", "style": {"fontSize": 120, "color": "#FFCC00", "bold": true}}
```

```json
{"type": "pasteClips", "atUs": 12000000, "clips": [{"trackId": "card", "trackKind": "text", "clip": {"id": "card", "startUs": 0, "durationUs": 3000000, "content": {"type": "text", "text": "Follow for more", "style": {"fontSize": 96, "color": "#FFFFFF", "bold": true}, "transform": {"x": 0, "y": 0.3, "scale": 1, "rotation": 0, "opacity": 1}}}}]}
```

Captions with exact times, one text track named Captions; `build_captions` makes them from the words better, so use this only for text the transcript does not have.

```json
{"type": "addCaptions", "segments": [{"startUs": 0, "endUs": 1200000, "text": "Wait for it"}], "style": {"fontSize": 80, "color": "#FFFFFF", "bold": true}}
```

Put a project asset on the timeline: on the main track by default, or at `startUs` on `trackId`.

```json
{"type": "addClip", "assetId": "asset-1", "startUs": 0, "trackId": "track-2"}
```

Animate: an entrance or exit, a transition into a main-track clip from the one before it, keyframes relative to the clip's start.

```json
{"type": "setAnimation", "clipId": "title-1", "slot": "in", "animation": {"kind": "pop", "durationUs": 300000}}
```

```json
{"type": "setTransition", "clipId": "clip-2", "transition": {"kind": "dissolve", "durationUs": 500000}}
```

```json
{"type": "setKeyframes", "clipId": "clip-1", "keyframes": [{"tUs": 0, "transform": {"x": 0, "y": 0, "scale": 1, "rotation": 0, "opacity": 1}}, {"tUs": 2000000, "transform": {"x": 0, "y": 0, "scale": 1.2, "rotation": 0, "opacity": 1}, "ease": "smooth"}]}
```

Mute or hide a track, and rename the project.

```json
{"type": "updateTrack", "trackId": "track-2", "muted": true}
```

```json
{"type": "renameProject", "name": "Final cut"}
```

`apply_edits`'s schema (`nuzky://schema` for the project) lists every edit and field.

## Pictures and sounds

A plugin can ship its own pictures and sounds, such as a logo for the corner or a beep over a swear word. List them in the manifest's `files` with `"nuzkyPlugin": 2`, and give them to `save_plugin` in `files`, each name to the absolute path of a file on this computer, such as a picture `import_web_image` downloaded; Nuzky copies them into the plugin's folder. Leave out a file the folder already has.

- A name is 1 to 60 letters, digits, dots, dashes and `_`, without a folder, not a name Windows keeps for a device (`CON`, `NUL`, `COM1`…), not another name's in other capitals, and ends in `.png`, `.jpg`, `.jpeg` or `.webp` for a picture, or `.mp3`, `.wav`, `.m4a`, `.aac` or `.flac` for a sound. At most 20 files of at most 20 MB each; a picture at most 8192 px a side and 25 megapixels.
- Each must be a plain file, never a link, and what its name says: Nuzky reads it as it reads any media. A sound holds no picture, such as an MP3's cover.
- The script never reads the files. Place one by its `assetId` from `ctx.files` with `addSticker`, `addClip`, `addPictureInPicture` or `replaceClip`. Nuzky copies it into its own folder and adds it to the project's media in the same batch, only when an edit names it, so the run is still one undo step. The project never refers to the plugin's folder, so it keeps working after the plugin is deleted. A dry run answers the names it would add in `adds`; a run answers them in `added`. Run again, the plugin places the copy the project already has.

A logo in the top right corner for the whole video:

```json
{"type": "addSticker", "assetId": "<ctx.files[\"logo.png\"].assetId>", "startUs": 0, "durationUs": 12000000, "transform": {"x": 0.35, "y": -0.4, "scale": 0.2, "rotation": 0, "opacity": 1}}
```

## Try it

1. Read `get_state` so you know the project the plugin will meet, and write the plugin for any project, not only this one.
2. `save_plugin(manifest, script, readme, files)`. PLUGIN_BROKEN names the field of plugin.json, or the line and column of plugin.js (`plugin.js:3:1: SyntaxError: …`). Fix it and save again.
3. `run_plugin(id, inputs, dry_run: true)` changes nothing. Check `edits`, `adds`, `message`, `logs` and `items`; `valid: false` with `error` says which edit would fail. Give `action` to try another of its buttons. Try the inputs the user will use, and an edge such as no selection.
4. `begin_run`, then `run_plugin(id, inputs, run_id)`. Its edits join your run. A long run answers with a job: wait with `job` action `wait`. A plugin that needs silences, loudness or beats first prepares the sound of each file Nuzky has not prepared yet, as part of that job.
5. `inspect_frames` where it changed something, and `get_state`. If it is wrong, `end_run` with discard, or keep and `undo_run`, fix the plugin and try again.
6. Tell the user the plugin is in their Plugins panel, what it does and what its inputs mean. When they want it every time, tell them to turn on Run after import or Run before export there.

## Errors

- PLUGIN_NOT_FOUND: no plugin with that id; `list_plugins`.
- PLUGIN_BROKEN: plugin.json or plugin.js is invalid, or one of its files is missing, a link or not the picture or sound its name says; the message says where.
- PLUGIN_API_NEWER: it needs a newer Nuzky. PLUGIN_API_OLD: it was made for an older one.
- INVALID_INPUT: `<input> must be …`, outside its bounds or options, or not an input of the plugin; `"<label>": the one you chose is gone. Choose another.`, a dynamic row gone since the preview, named by the input's label; or an action the plugin does not have.
- TRANSCRIPT_MISSING: it needs the words; `transcribe` first.
- MEDIA_MISSING: it needs the sound of a file that is missing; the user relinks it.
- PLUGIN_FAILED: the script threw, with `plugin.js:<line>:<column>`, or returned something that is not edits; the last lines of its log follow.
- PLUGIN_TIMEOUT: it ran longer than 60 seconds. CANCELLED or RUN_STOPPED: the user stopped it.
- PLUGIN_OUT_OF_MEMORY: the script took more than its 256 MB, or its edits applied to the copy of the project took more than 1 GB. Make it keep and change less at once, such as fewer clips or fewer keyframes per clip. A script that runs out of memory while QuickJS makes its error fails with `the script threw null` instead.
- PLUGIN_TOO_MANY_EDITS: more than 5000 edits.
- PLUGIN_FORBIDDEN_EDIT: `edit <index> (<type>)` would add a file; place the plugin's own files by their `assetId` in `ctx.files`.
- PLUGIN_BAD_EDIT: `edit <index> (<type>): …` is not a valid edit or does not apply (index counts from 0), or the edits together leave the project invalid.

After any error nothing was applied.

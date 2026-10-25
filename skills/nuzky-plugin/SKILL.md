---
name: nuzky-plugin
description: Write, try and fix a Nuzky plugin, a small script that adds an edit Nuzky lacks, through Nuzky MCP.
---

# Write a Nuzky plugin

A plugin is a small JavaScript file that reads the project and returns edits. The user asks for an edit Nuzky does not have ("delete every clip shorter than a second", "put my end card on every video"), you write it once, and it becomes a button in the app's Plugins panel that they can run on any project, with inputs they set. Plugins live in Nuzky's own data folder, so they survive updates, and a project a plugin edited opens anywhere without it.

## What a plugin can and cannot do

- It reads: the project as `get_state` gives it, the clips the user selected, the playhead, the values of its inputs and, when it asks for them, the words of the transcript.
- It returns edits, the same EditCmd JSON as `apply_edits`. Nuzky applies them as ONE batch: all of them or none. The user undoes them in one step.
- It cannot read or write files, reach the network, start processes, use timers or import modules. There is no `fetch`, `require`, `import`, `setTimeout` or `process`. `Math`, `Date`, `JSON`, strings, arrays and regular expressions work. `run` must return its result, not a Promise.
- It cannot add media: `addAssets` is refused, so are reel candidates pointing at a new project file. It works with the assets the project already has (`addClip`, `replaceClip` with an existing `assetId`).
- It runs at most 60 seconds, with 256 MB of memory for the script. It runs in a process of its own, where Nuzky also applies its edits to a copy of the project first; that process may take at most 1 GB. plugin.js is at most 1 MB and a run returns at most 5000 edits.

## Files

`save_plugin` writes them into `plugins/<id>/` of Nuzky's data folder: `plugin.json` (the manifest), `plugin.js` and an optional `README.md` the app shows. Never write them yourself.

```json
{
  "nuzkyPlugin": 1,
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

- `nuzkyPlugin` is the plugin API, 1. `list_plugins` gives the `api` this Nuzky runs.
- `id`: 1 to 40 lowercase letters, digits and dashes; also the folder's name. Saving with the same id replaces the plugin and keeps the old files as plugin.prev.json and plugin.prev.js.
- `name` (at most 40 characters), `description` (300), `version` (20) and `action`, the verb on its button (24, "Run" when left out). Write them in the user's language, plainly, saying what will happen.
- `needs`: `["words"]` when the plugin reads the transcript.
- `inputs`, at most 20, each with `id` (letters, digits and `_`), `label` (at most 40 characters), `type` and `default`:
  - `number`: `min`, `max`, `step`, `default`, optional `unit` such as `"s"` or `"%"`.
  - `toggle`: `default` true or false.
  - `choice`: `options` `[{"value": "main", "label": "Main track"}]`, `default` one of the values.
  - `text`: `default`, at most 500 characters.
  - `color`: `default` such as `"#FFCC00"`.

Ask the user only for what changes between runs. A value the plugin can work out from the project is not an input.

## plugin.js

```js
function run(ctx) {
  nuzky.progress(0.5);
  nuzky.log("clips:", ctx.project.tracks[0].clips.length);
  return { edits: [{ type: "renameProject", name: ctx.inputs.title }], message: "Renamed the project." };
}
```

`run(ctx)` returns an array of edits, or `{edits, message}`, where `message` is one sentence for the user, at most 500 characters, such as "Removed 6 clips." Return no edits and a message saying why when there is nothing to do.

`ctx`:

- `api`: 1.
- `project`: what `get_state` answers: `canvas`, `assets`, `tracks` (bottom to top; `tracks[0]` is the main track, id `"main"`), each with `clips` `{id, startUs, durationUs, content, animIn, animOut, keyframes, transitionIn}`, `duration_us`, `thumbnails`, `selection`, `playhead_us`. `content.type` is `"media"` (`assetId`, `sourceInUs`, `speed`, `volume`, `transform`…) or `"text"` (`text`, `style`, `transform`).
- `inputs`: every input's value by id, checked against the manifest; those the user did not set have their default.
- `selection`: ids of the clips the user selected, maybe none.
- `playheadUs`: where the playhead is.
- `words`: only with `"needs": ["words"]`, the timeline's words as `get_transcript` gives them: `{i, start_us, end_us, text, p}`. Without a transcript the run fails with TRANSCRIPT_MISSING, so the user transcribes first.

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

## Try it

1. Read `get_state` so you know the project the plugin will meet, and write the plugin for any project, not only this one.
2. `save_plugin(manifest, script, readme)`. PLUGIN_BROKEN names the field of plugin.json, or the line and column of plugin.js (`plugin.js:3:1: SyntaxError: …`). Fix it and save again.
3. `run_plugin(id, inputs, dry_run: true)` changes nothing. Check `edits`, `message` and `logs`; `valid: false` with `error` says which edit would fail. Try the inputs the user will use, and an edge such as no selection.
4. `begin_run`, then `run_plugin(id, inputs, run_id)`. Its edits join your run. A long run answers with a job: wait with `job` action `wait`.
5. `inspect_frames` where it changed something, and `get_state`. If it is wrong, `end_run` with discard, or keep and `undo_run`, fix the plugin and try again.
6. Tell the user the plugin is in their Plugins panel, what it does and what its inputs mean.

## Errors

- PLUGIN_NOT_FOUND: no plugin with that id; `list_plugins`.
- PLUGIN_BROKEN: plugin.json or plugin.js is invalid; the message says where.
- PLUGIN_API_NEWER: it needs a newer Nuzky. PLUGIN_API_OLD: it was made for an older one.
- INVALID_INPUT: `<input> must be …`, outside its bounds or options, or not an input of the plugin.
- TRANSCRIPT_MISSING: it needs the words; `transcribe` first.
- PLUGIN_FAILED: the script threw, with `plugin.js:<line>:<column>`, or returned something that is not edits; the last lines of its log follow.
- PLUGIN_TIMEOUT: it ran longer than 60 seconds. CANCELLED or RUN_STOPPED: the user stopped it.
- PLUGIN_OUT_OF_MEMORY: the script took more than its 256 MB, or its edits applied to the copy of the project took more than 1 GB. Make it keep and change less at once, such as fewer clips or fewer keyframes per clip. A script that runs out of memory while QuickJS builds its error fails with PLUGIN_FAILED, `the script threw null`.
- PLUGIN_TOO_MANY_EDITS: more than 5000 edits.
- PLUGIN_FORBIDDEN_EDIT: `edit <index> (<type>)` would add a file.
- PLUGIN_BAD_EDIT: `edit <index> (<type>): …` is not a valid edit or does not apply (index counts from 0), or the edits together leave the project invalid.

After any error nothing was applied.

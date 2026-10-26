import json, time

from e2e.harness import FIXTURES, Bridge, flow, preview_brightness, wait, webdriver

TOOLBAR = "section[aria-label=Timeline] .bar"
BUTTONS = f"""return [...document.querySelectorAll('{TOOLBAR} button')].map((b) => [b.getAttribute('aria-label'),
  b.getAttribute('aria-disabled') === 'true' || b.disabled]);"""
PRESS = f"""const b = [...document.querySelectorAll('{TOOLBAR} button')].find((b) => b.getAttribute('aria-label').startsWith(arguments[0]));
if (!b || b.getAttribute('aria-disabled') === 'true' || b.disabled) return false;
b.click();
return true;"""
HEADER_UNDO = "return [...document.querySelectorAll('header button')].some((b) => /^(Undo|Redo)/.test(b.getAttribute('aria-label') ?? ''))"
KEYS = """return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips)
  .find((c) => c.id === arguments[0]).keyframes.map((k) => k.transform.rotation)"""
CLIP = """const c = window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips).find((c) => c.id === arguments[0]);
return {rotation: c.content.transform.rotation, volume: c.content.volume};"""
CLIP_OPACITY = """return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips)
  .find((c) => c.id === arguments[0]).content.transform.opacity"""


@flow('toolbar', "The timeline's toolbar leads with Undo and Redo, which leave the top bar; Rotate 90° turns the picture "
      'in the preview and on disk, Undo and Redo there take it back and forth, Detach audio moves the sound to its own '
      'track, and while the AI edits every button says why it is off')
def toolbar(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    clip = r.track()[0]['id']
    r.seek(1_000_000)
    r.focus_clip(clip)
    time.sleep(1.5)
    labels = [label for label, _ in r.s.run(BUTTONS)]
    r.check('the toolbar reads Undo, Nothing to redo, Split, Delete left, Delete right, Delete, Duplicate, Detach audio, Rotate, Snapping',
            [l.split(' (')[0].split(':')[0] for l in labels] == ['Undo', 'Nothing to redo', 'Split at playhead', 'Delete left of playhead',
             'Delete right of playhead', 'Delete selection', 'Duplicate', 'Detach audio', 'Rotate 90°', 'Snapping on'], labels)
    r.check('with nothing to redo, Redo says so', labels[1] == 'Nothing to redo', labels[1])
    r.check('the top bar has no Undo or Redo of its own', not r.s.run(HEADER_UNDO))
    r.shot('before')
    # talk.mp4 is portrait and fills the 9:16 canvas; turned a quarter it lies across the middle, black above it.
    upright = preview_brightness(r, r.work / 'before.png', 0.1)

    r.check('Rotate 90° turns the clip a quarter', r.s.run(PRESS, 'Rotate') and wait(lambda: r.s.run(CLIP, clip)['rotation'] == 90, 5),
            r.s.run(CLIP, clip))
    time.sleep(1.5)
    r.shot('rotated')
    turned = preview_brightness(r, r.work / 'rotated.png', 0.1)
    r.check('the preview shows the picture turned, the top of the frame now black', upright > 40 and turned < 10,
            [round(upright, 1), round(turned, 1)])
    saved = lambda: next(c['content']['transform']['rotation'] for t in json.loads(r.saved_project().read_text())['tracks']
                         for c in t['clips'] if c['id'] == clip)
    r.check('the project file on disk has the turned clip', wait(lambda: saved() == 90, 10), saved())
    r.s.run(PRESS, 'Rotate')
    r.s.run(PRESS, 'Rotate')
    r.check('two more quarters go the short way round, to -90°', wait(lambda: r.s.run(CLIP, clip)['rotation'] == -90, 5),
            r.s.run(CLIP, clip))

    r.check("the toolbar's Undo takes the last quarter back", r.s.run(PRESS, 'Undo') and wait(lambda: r.s.run(CLIP, clip)['rotation'] == 180, 5),
            r.s.run(CLIP, clip))
    r.check("the toolbar's Redo does it again", r.s.run(PRESS, 'Redo') and wait(lambda: r.s.run(CLIP, clip)['rotation'] == -90, 5),
            r.s.run(CLIP, clip))
    for _ in range(3):
        r.s.run(PRESS, 'Undo')
    r.check('three more Undos stand it upright', wait(lambda: r.s.run(CLIP, clip)['rotation'] == 0, 5), r.s.run(CLIP, clip))

    # Keyframes turn by the clip's amount, so 0° to 150° becomes 90° to 240°; looking at 240° in the inspector keeps it.
    r.s.call("""window.__nuzky.store.getState().edit((p) => { const c = p.tracks[0].clips.find((c) => c.id === arguments[0]);
        const at = (tUs, rotation) => ({tUs, ease: 'linear', transform: {...c.content.transform, rotation}});
        return {type: 'setKeyframes', clipId: c.id, keyframes: [at(0, 0), at(2_000_000, 150)]}; })""", clip)
    r.focus_clip(clip)
    r.s.run(PRESS, 'Rotate')
    r.check('Rotate 90° turns keyframes 0° and 150° to 90° and 240°', wait(lambda: r.s.run(KEYS, clip) == [90, 240], 5), r.s.run(KEYS, clip))
    r.seek(2_000_000)
    time.sleep(0.5)
    # The app's window is never active in a run, so the field gets its focus events by hand.
    field = "document.querySelector('input[aria-label=\"Rotation value\"]').dispatchEvent(new FocusEvent(arguments[0], {bubbles: true}))"
    r.s.run(field, 'focusin')
    time.sleep(0.3)
    r.s.run(field, 'focusout')
    time.sleep(1)
    r.check('focusing and leaving the 240° rotation in the inspector changes nothing', r.s.run(KEYS, clip) == [90, 240], r.s.run(KEYS, clip))
    r.key('z', ctrlKey=True)
    r.key('z', ctrlKey=True)
    wait(lambda: r.s.run(KEYS, clip) == [], 5)

    r.focus_clip(clip)
    r.check('Detach audio moves the sound to an audio track and silences the video',
            r.s.run(PRESS, 'Detach audio') and wait(lambda: any(t['kind'] == 'audio' and t['clips'] for t in r.state()['tracks']), 5)
            and r.s.run(CLIP, clip)['volume'] == 0, r.state()['tracks'])
    r.focus_clip(clip)
    time.sleep(0.5)
    detach = next(label for label, off in r.s.run(BUTTONS) if label.startswith('Detach audio') and off)
    r.check('with the sound gone, Detach audio is off and says why', detach == 'Detach audio: sound is already detached or muted', detach)
    r.shot('toolbar')
    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    wait(lambda: r.s.run('return [innerWidth, innerHeight]') == [1024, 640], 5)
    time.sleep(1)
    r.shot('toolbar-small-window')

    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Look at the toolbar'})
        wait(lambda: r.state()['aiRun'], 10)
        time.sleep(0.5)
        buttons = r.s.run(BUTTONS)
        editing = [label for label, off in buttons if not label.startswith('Snapping')]
        r.check('while the AI edits, every editing button is off and says so',
                all(off for label, off in buttons if not label.startswith('Snapping'))
                and all('AI' in label for label in editing), buttons)
        bridge.call('end_run', {'run_id': run['run_id'], 'action': 'keep'})
    finally:
        bridge.close()


HOVER = """const b = document.querySelector(`[data-peek-button=${arguments[0]}]`);
b.dispatchEvent(new PointerEvent(arguments[1] ? 'pointerover' : 'pointerout', {bubbles: true, pointerId: 1,
  relatedTarget: arguments[1] ? null : document.body}));"""
SELECTION = 'return window.__nuzky.store.getState().selection'
TIP_TEXT = "return document.querySelector('[data-testid=history-peek]')?.innerText ?? null"
DIMMED = "return [...document.querySelectorAll('[data-clip-id].opacity-35')].map((el) => el.dataset.clipId)"
WORDS = """return Object.fromEntries([...document.querySelectorAll('[data-peek]')].map((el) => [el.dataset.clipId,
  [...el.querySelectorAll('span')].find((s) => s.classList.contains('right-1'))?.textContent ?? null]))"""
MARKS = "return Object.fromEntries([...document.querySelectorAll('[data-peek]')].map((el) => [el.dataset.clipId, el.dataset.peek]))"


@flow('undo_preview', 'Resting the pointer on Undo or Redo shows on the timeline what the step changes, a clip that comes '
      'back dashed in its place and the clips it pushes along outlined, and names it in a tip, also a change the timeline '
      'cannot show and clips out of view; nothing changes until the click, after which the next step shows')
def undo_preview(r):
    r.import_media(FIXTURES / 'talk.mp4', FIXTURES / 'wide.mp4')
    for name in ('talk.mp4', 'wide.mp4'):
        r.add_clip(name)
    r.seek(2_000_000)
    r.focus_clip(r.track()[0]['id'])
    r.key('s')
    split = wait(lambda: (c := r.track()) and len(c) == 3 and c)
    piece, wide = split[1], split[2]
    r.focus_clip(piece['id'])
    r.key('Delete')
    deleted = wait(lambda: (c := r.track()) and len(c) == 2 and c)
    saved = lambda: [c['id'] for c in json.loads(r.saved_project().read_text())['tracks'][0]['clips']]
    wait(lambda: len(saved()) == 2, 10)
    r.seek(0)
    # The pointer goes to the toolbar; focus leaves the clips, as a click elsewhere would.
    r.s.run('document.activeElement?.blur()')
    time.sleep(0.5)

    r.s.run(HOVER, 'undo', True)
    time.sleep(0.12)
    r.check('passing over Undo for a moment shows nothing', r.s.run(TIP_TEXT) is None and r.s.run(MARKS) == {}, r.s.run(MARKS))
    tip = wait(lambda: r.s.run(TIP_TEXT), 5)
    r.check('resting on Undo names the step and what it brings back',
            tip and tip.startswith('Undo · Delete') and 'Brings back talk.mp4' in tip, tip)
    marks = r.s.run(MARKS)
    r.check('the timeline draws the deleted piece back in its place, marks the clip it pushes along and dims the rest',
            marks == {piece['id']: 'back', wide['id']: 'moved'} and r.s.run(DIMMED) == [deleted[0]['id']], [marks, r.s.run(DIMMED)])
    r.check('each marked clip says how it changes in words', r.s.run(WORDS) == {piece['id']: 'Back', wide['id']: 'Moved'}, r.s.run(WORDS))
    r.shot('undo-hover')
    r.check('nothing changed yet, in the editor or on disk', r.track() == deleted and saved() == [c['id'] for c in deleted])
    r.s.run(HOVER, 'undo', False)
    r.check('leaving Undo puts the timeline back', wait(lambda: r.s.run(TIP_TEXT) is None and r.s.run(MARKS) == {}, 2), r.s.run(MARKS))

    # Zoomed in at the start, the clip the undo pushes along lies past the right edge.
    r.s.run('window.__nuzky.store.getState().setZoom(600)')
    time.sleep(0.5)
    r.s.run(HOVER, 'undo', True)
    tip = wait(lambda: r.s.run(TIP_TEXT), 5)
    r.check('a change out of view is named with the side it is on', tip and '1 changed clip off screen, to the right' in tip, tip)
    r.s.run(HOVER, 'undo', False)
    r.s.run('window.__nuzky.store.getState().setZoom(60)')

    # A change the lanes cannot show: the tip says the values.
    first = deleted[0]['id']
    r.s.call("""window.__nuzky.store.getState().edit((p) => { const c = p.tracks[0].clips.find((c) => c.id === arguments[0]);
        return {type: 'updateClip', clipId: c.id, transform: {...c.content.transform, opacity: 0.4}}; })""", first)
    wait(lambda: r.s.run(CLIP_OPACITY, first) == 0.4, 5)
    r.s.run(HOVER, 'undo', True)
    tip = wait(lambda: r.s.run(TIP_TEXT), 5)
    r.check('Undo of an opacity change says the opacity it brings back', tip and 'talk.mp4: Opacity 40 % → 100 %' in tip, tip)
    r.check('and rings the clip it changes', r.s.run(MARKS) == {first: 'changed'}, r.s.run(MARKS))
    r.shot('undo-hover-opacity')
    r.s.run("document.querySelector('[data-peek-button=undo]').click()")
    r.check('clicking Undo takes the opacity back', wait(lambda: r.s.run(CLIP_OPACITY, first) == 1, 5), r.s.run(CLIP_OPACITY, first))
    time.sleep(1)
    r.check('after the click the timeline shows the result, not the next step', r.s.run(TIP_TEXT) is None and r.s.run(MARKS) == {},
            [r.s.run(TIP_TEXT), r.s.run(MARKS)])
    r.s.run(HOVER, 'undo', False)
    r.s.run(HOVER, 'undo', True)
    tip = wait(lambda: r.s.run(TIP_TEXT), 5)
    r.check('coming back to Undo shows the step before', tip and tip.startswith('Undo · Delete'), tip)
    r.s.run(HOVER, 'undo', False)
    r.s.run(HOVER, 'redo', True)
    tip = wait(lambda: r.s.run(TIP_TEXT), 5)
    r.check('Redo says what it would do again', tip and tip.startswith('Redo · Change clip') and 'Opacity 100 % → 40 %' in tip, tip)
    r.s.run(HOVER, 'redo', False)

    # Redo of the delete only takes a clip away: the timeline stays and the piece is hatched where it is.
    r.s.run("document.querySelector('[data-peek-button=undo]').click()")
    wait(lambda: len(r.track()) == 3, 5)
    r.s.run(HOVER, 'undo', False)
    r.s.run(HOVER, 'redo', True)
    tip = wait(lambda: r.s.run(TIP_TEXT), 5)
    r.check('Redo of a delete names the piece it removes', tip and tip.startswith('Redo · Delete') and 'Removes talk.mp4' in tip, tip)
    r.check('and marks it Removed where it is now', r.s.run(MARKS).get(piece['id']) == 'gone' and r.s.run(WORDS).get(piece['id']) == 'Removed',
            [r.s.run(MARKS), r.s.run(WORDS)])
    r.shot('redo-hover-removes')
    # The pointer rests on Redo while focus is on a clip, which Esc would otherwise deselect.
    r.focus_clip(piece['id'])
    r.key('Escape')
    r.check('Esc hides the preview and leaves the selection alone',
            wait(lambda: r.s.run(TIP_TEXT) is None and r.s.run(MARKS) == {}, 2) and r.s.run(SELECTION) == [piece['id']],
            [r.s.run(TIP_TEXT), r.s.run(SELECTION)])
    r.s.run(HOVER, 'redo', False)

    # Undo of the split takes the second piece away and gives the first its old length: the lanes draw that length.
    r.s.run(HOVER, 'undo', True)
    tip = wait(lambda: r.s.run(TIP_TEXT), 5)
    r.check('Undo of a split names the piece that goes and the length the first part gets back',
            tip and tip.startswith('Undo · Split') and 'Removes talk.mp4' in tip and 'talk.mp4 length 2.0 s → 14.3 s' in tip, tip)
    first_width = "return document.querySelector(`[data-clip-id='${arguments[0]}']`).getBoundingClientRect().width"
    whole_s = (split[0]['durationUs'] + piece['durationUs']) / 1e6
    r.check('and draws the first part at its old length, marked Trimmed', r.s.run(MARKS).get(first) == 'trimmed'
            and abs(r.s.run(first_width, first) - whole_s * r.s.run('return window.__nuzky.store.getState().zoom')) < 4,
            [r.s.run(MARKS), r.s.run(first_width, first)])
    r.s.run(HOVER, 'undo', False)

    # One slider moved on every selected clip is one line naming the slider; the timeline marks the clips.
    ids = [c['id'] for c in r.track()]
    r.s.call("""window.__nuzky.store.getState().edit((p) => p.tracks[0].clips.map((c) =>
        ({type: 'updateClip', clipId: c.id, adjust: {...c.content.adjust, vignette: 0.3}})))""")
    vignettes = "return window.__nuzky.store.getState().snap.project.tracks[0].clips.map((c) => c.content.adjust.vignette)"
    wait(lambda: r.s.run(vignettes) == [0.3] * len(ids), 5)
    r.s.run(HOVER, 'undo', True)
    tip = wait(lambda: r.s.run(TIP_TEXT), 5)
    r.check('Undo of a slider moved on many clips says it once, with the values', tip and [l for l in tip.splitlines() if ':' in l] == ['3 clips: Vignette 30 → 0'], tip)
    r.check('and marks every clip it changes', r.s.run(MARKS) == {i: 'changed' for i in ids}, r.s.run(MARKS))
    r.shot('undo-hover-many')
    r.s.run(HOVER, 'undo', False)

    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Look at undo'})
        wait(lambda: r.state()['aiRun'], 10)
        r.s.run(HOVER, 'undo', True)
        time.sleep(1)
        r.check('while the AI edits, resting on Undo shows nothing', r.s.run(TIP_TEXT) is None and r.s.run(MARKS) == {}, r.s.run(MARKS))
        r.s.run(HOVER, 'undo', False)
        bridge.call('end_run', {'run_id': run['run_id'], 'action': 'keep'})
    finally:
        bridge.close()

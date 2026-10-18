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
    r.check('the toolbar reads Undo, Redo, Split, Delete left, Delete right, Delete, Duplicate, Detach audio, Rotate, Snapping',
            [l.split(' (')[0].split(':')[0] for l in labels] == ['Undo', 'Redo', 'Split at playhead', 'Delete left of playhead',
             'Delete right of playhead', 'Delete selection', 'Duplicate', 'Detach audio', 'Rotate 90°', 'Snapping on'], labels)
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

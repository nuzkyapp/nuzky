import json, statistics, subprocess, time

from e2e.ducking import RATE, TONE, WINDOW, decode, tone_levels
from e2e.harness import FIXTURES, MACOS, Bridge, export, flow, preview_brightness, wait, webdriver

# Presses what is under the middle of a fade dot of a clip and drags it `arguments[2]` px along the timeline, without
# letting go.
FADE_DRAG = """const knob = document.querySelector(`[data-clip-id='${arguments[0]}'] [data-fade='${arguments[1]}']`);
if (!knob) return false;
const b = knob.getBoundingClientRect();
const at = (type, dx) => new PointerEvent(type, {bubbles: true, button: 0, pointerId: 1,
  clientX: b.left + b.width / 2 + dx, clientY: b.top + b.height / 2});
document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2).dispatchEvent(at('pointerdown', 0));
window.dispatchEvent(at('pointermove', arguments[2]));
return true;"""
RELEASE = "window.dispatchEvent(new PointerEvent('pointerup', {bubbles: true, button: 0, pointerId: 1}))"
# Presses a clip just inside its right edge and drags that edge `arguments[1]` px along the timeline, without letting go.
TRIM_RIGHT = """const el = document.querySelector(`[data-clip-id='${arguments[0]}']`);
const b = el.getBoundingClientRect();
const at = (type, dx) => new PointerEvent(type, {bubbles: true, button: 0, pointerId: 1,
  clientX: b.right - 3 + dx, clientY: b.top + b.height / 2});
el.dispatchEvent(at('pointerdown', 0));
window.dispatchEvent(at('pointermove', arguments[1]));"""
# Presses a clip just inside its left edge and drags that edge `arguments[1]` px along the timeline, without letting go.
TRIM_LEFT = """const el = document.querySelector(`[data-clip-id='${arguments[0]}']`);
const b = el.getBoundingClientRect();
const at = (type, dx) => new PointerEvent(type, {bubbles: true, button: 0, pointerId: 1,
  clientX: b.left + 3 + dx, clientY: b.top + b.height / 2});
el.dispatchEvent(at('pointerdown', 0));
window.dispatchEvent(at('pointermove', arguments[1]));"""
# The left edge of the clip `arguments[0]` and of the ghost a drag draws, in window pixels.
LEFTS = """const ghost = document.querySelector('section[aria-label=Timeline] .opacity-85');
return {next: document.querySelector(`[data-clip-id='${arguments[0]}']`).getBoundingClientRect().left,
  ghost: ghost ? ghost.getBoundingClientRect().left : null};"""
# The filmstrip frames drawn in the clip `arguments[0]`, or in the ghost a drag draws when it is null.
TILES = """const el = arguments[0] ? document.querySelector(`[data-clip-id='${arguments[0]}']`) : document.querySelector('section[aria-label=Timeline] .opacity-85');
return el ? [...el.querySelectorAll('div[style*="background-position"]')].map((d) => d.style.backgroundPosition) : [];"""
TIP = "return [...document.querySelectorAll('section[aria-label=Timeline] div')].some((d) => d.textContent === arguments[0])"
FADES = """const c = window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips).find((c) => c.id === arguments[0]);
return [c.content.fadeInUs, c.content.fadeOutUs];"""
UNDER = """const b = document.querySelector(`[data-clip-id='${arguments[0]}'] [data-fade='${arguments[1]}']`).getBoundingClientRect();
return document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2)?.closest('[data-fade]')?.dataset.fade ?? null;"""
KNOBS = "return document.querySelectorAll(`[data-clip-id='${arguments[0]}'] [data-fade]`).length"
SELECTION = 'return window.__nuzky.store.getState().selection'
# Sends `arguments[2]` Ctrl + wheel events of `arguments[1]` px over the clip `arguments[0]` and returns the zoom.
WHEEL = """const b = document.querySelector(`[data-clip-id='${arguments[0]}']`).getBoundingClientRect();
const el = document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2);
for (let i = 0; i < arguments[2]; i++) el.dispatchEvent(new WheelEvent('wheel', {bubbles: true, cancelable: true,
  ctrlKey: true, deltaY: arguments[1], clientX: b.left + b.width / 2, clientY: b.top + b.height / 2}));
return window.__nuzky.store.getState().zoom;"""
# Clicks the middle of a clip with the modifier keys in `arguments[1]`.
CLICK_CLIP = """const b = document.querySelector(`[data-clip-id='${arguments[0]}']`).getBoundingClientRect();
const at = (type) => new PointerEvent(type, Object.assign({bubbles: true, button: 0, pointerId: 1,
  clientX: b.left + b.width / 2, clientY: b.top + b.height / 2}, arguments[1]));
document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2).dispatchEvent(at('pointerdown'));
window.dispatchEvent(at('pointerup'));"""


@flow('edit', 'Import two clips, zoom the timeline with a touchpad and a wheel, split one with S, delete a piece with Delete, undo, find the result saved, drag a clip edge '
      'and see a title move with the clip after it, see imported music '
      'in the Audio tab, and fade a sound in by dragging the dot on its clip, heard in the export; Esc cancels such a drag')
def edit(r):
    r.import_media(FIXTURES / 'talk.mp4', FIXTURES / 'wide.mp4')
    for name in ('talk.mp4', 'wide.mp4'):
        r.add_clip(name)
    clips = r.track()
    r.check('clips sit back to back on the main track', clips[1]['startUs'] == clips[0]['startUs'] + clips[0]['durationUs'], clips)
    r.check('both clips get thumbnails', wait(lambda: len(r.state()['thumbs']) == 2), r.state()['thumbs'])
    r.s.run('window.__nuzky.store.getState().setZoom(60)')
    pinched = r.s.run(WHEEL, clips[0]['id'], -3, 20)
    r.check('a touchpad pinch zooms the timeline in proportion to the gesture', 75 < pinched < 90, pinched)
    notch = r.s.run(WHEEL, clips[0]['id'], -100, 1)
    r.check('one mouse wheel notch zooms by 15 %', abs(notch - pinched * 1.15) < 0.01, [pinched, notch])
    r.s.run('window.__nuzky.store.getState().setZoom(60)')
    r.seek(2_000_000)
    r.focus_clip(clips[0]['id'])
    r.key('s')
    split = wait(lambda: (c := r.track()) and len(c) == 3 and c)
    r.check('S splits the clip at the playhead', split and split[0]['durationUs'] == 2_000_000, split)
    piece = split[1]
    r.focus_clip(piece['id'])
    r.key('Delete')
    deleted = wait(lambda: (c := r.track()) and len(c) == 2 and c)
    end = lambda clips: clips[-1]['startUs'] + clips[-1]['durationUs']
    r.check('Delete removes the piece and closes the gap', deleted and end(deleted) == end(split) - piece['durationUs'], deleted)
    r.key('z', ctrlKey=True)
    undone = wait(lambda: (c := r.track()) and len(c) == 3 and c)
    r.check('Ctrl+Z brings the piece back', undone == split, undone)
    time.sleep(1.5)
    r.shot('timeline')
    # talk.mp4 is portrait and fills the 9:16 canvas; wide.mp4, which followed the deleted piece, leaves it black at the top.
    top = preview_brightness(r, r.work / 'timeline.png', 0.1)
    r.check('the preview shows the restored piece under the playhead', top > 40, round(top, 1))
    saved = lambda: len(json.loads(r.saved_project().read_text())['tracks'][0]['clips'])
    r.check('the project file on disk has the same three clips', wait(lambda: saved() == 3, 10), saved())
    # Cmd on a Mac, where Ctrl-click is a right click.
    add = {'metaKey': True} if MACOS else {'ctrlKey': True}
    first, second = undone[0]['id'], undone[2]['id']
    r.s.run(CLICK_CLIP, first, {})
    r.s.run(CLICK_CLIP, second, add)
    r.check('Cmd/Ctrl-click adds a second clip to the selection', wait(lambda: r.s.run(SELECTION) == [first, second], 5), r.s.run(SELECTION))
    r.shot('two-selected')
    r.s.run(CLICK_CLIP, second, add)
    r.check('Cmd/Ctrl-click on a selected clip takes it out', wait(lambda: r.s.run(SELECTION) == [first], 5), r.s.run(SELECTION))

    # Dragging a main-track clip's right edge out shows more of its video and moves what follows, a title over it too.
    wide = undone[2]
    r.s.call("window.__nuzky.store.getState().edit({type: 'addText', startUs: arguments[0], text: 'Wide',"
             " style: {fontSize: 80, color: '#ffffff'}})", wide['startUs'])
    title = lambda: next((c for t in r.state()['tracks'] if t['kind'] == 'text' for c in t['clips']), None)
    r.check('a title starts with the wide clip', wait(lambda: (c := title()) and c['startUs'] == wide['startUs'], 5), title())
    r.seek(0)
    r.s.run('window.__nuzky.store.getState().setZoom(100)')
    time.sleep(0.5)
    r.s.run(TRIM_RIGHT, undone[0]['id'], 100)
    time.sleep(0.3)
    r.s.run(RELEASE)
    grown = wait(lambda: (c := r.track()) and c[0]['durationUs'] > undone[0]['durationUs'] and c, 5)
    moved = grown and grown[2]['startUs'] - wide['startUs']
    r.check('dragging the first clip\'s right edge out moves the wide clip later', grown and moved > 0, grown)
    r.check('the title moves with the wide clip', grown and title()['startUs'] == grown[2]['startUs'],
            {'title': title(), 'wide': grown and grown[2]})
    r.s.run('window.__nuzky.store.getState().setZoom(40)')
    time.sleep(1)
    r.shot('trim-moves-title')
    for _ in range(2):
        r.key('z', ctrlKey=True)
    r.check('undo puts the clip and the title back', wait(lambda: r.track() == undone and title() is None, 5), r.track())

    # Dragging a main-track clip's left edge in, as in CapCut, moves only that edge while dragging; the clips after it
    # stay still and close up on release. The right side of the clip and everything after it once slid left instead.
    r.s.run('window.__nuzky.store.getState().setZoom(100)')
    time.sleep(0.5)
    before = r.s.run(LEFTS, undone[1]['id'])
    first_frames = r.s.run(TILES, undone[0]['id'])
    r.s.run(TRIM_LEFT, undone[0]['id'], 50)
    time.sleep(0.3)
    during = r.s.run(LEFTS, undone[1]['id'])
    r.check('dragging the first clip\'s left edge 50 px in leaves the next clip where it was',
            during['next'] == before['next'], {'before': before, 'during': during})
    r.check('the trimmed clip\'s left edge follows the pointer', during['ghost'] is not None and abs(during['ghost'] - before['next'] + 150) < 2,
            {'before': before, 'during': during})
    dragged = r.s.run(TILES, None)
    r.shot('trim-left-drag')
    r.s.run(RELEASE)
    cut = wait(lambda: (c := r.track()) and c[0]['durationUs'] < undone[0]['durationUs'] and c, 5)
    r.check('on release the clip keeps its start and the next clip closes up behind it',
            cut and cut[0]['startUs'] == undone[0]['startUs'] and cut[1]['startUs'] == cut[0]['startUs'] + cut[0]['durationUs']
            and cut[0]['durationUs'] == undone[0]['durationUs'] - 500_000, cut)
    # The footage goes from the left: while dragging, the clip already shows the frames it keeps. It once showed its
    # first frames and lost them from the right, and only jumped to the kept ones on release.
    kept = wait(lambda: (t := r.s.run(TILES, undone[0]['id'])) and t != first_frames and t, 5)
    r.check('while the left edge is dragged, the clip shows the frames it keeps on release',
            bool(dragged) and dragged == kept, {'dragged': dragged, 'kept': kept})
    r.key('z', ctrlKey=True)
    r.check('undo puts the left edge back', wait(lambda: r.track() == undone, 5), r.track())

    # Deleting a main-track clip takes the captions over it along and moves the later ones with their picture, also
    # when the captions were selected with it. Deleting the video alone once left its captions, and deleting both
    # left a gap in the captions.
    piece, wide = undone[1], undone[2]
    segment = lambda start, text: {'startUs': start + 200_000, 'endUs': start + 700_000, 'text': text, 'words': []}
    r.s.call("window.__nuzky.store.getState().edit({type: 'addCaptions', segments: arguments[0],"
             " style: {fontSize: 60, color: '#ffffff'}})", [segment(piece['startUs'], 'Piece'), segment(wide['startUs'], 'Wide')])
    captions = lambda: [(c['text'], c['startUs']) for t in r.state()['tracks'] if t['kind'] == 'text' for c in t['clips']]
    r.check('a caption over the piece and one over the wide clip',
            wait(lambda: captions() == [('Piece', piece['startUs'] + 200_000), ('Wide', wide['startUs'] + 200_000)], 5), captions())
    after = [('Wide', piece['startUs'] + 200_000)]
    r.focus_clip(piece['id'])
    r.key('Delete')
    r.check('deleting only the piece removes its caption and moves the next one with the wide clip',
            wait(lambda: captions() == after, 5), captions())
    r.key('z', ctrlKey=True)
    wait(lambda: len(captions()) == 2, 5)
    caption = next(c['id'] for t in r.state()['tracks'] if t['kind'] == 'text' for c in t['clips'] if c['text'] == 'Piece')
    r.s.run(CLICK_CLIP, piece['id'], {})
    r.s.run(CLICK_CLIP, caption, add)
    wait(lambda: r.s.run(SELECTION) == [piece['id'], caption], 5)
    r.key('Delete')
    r.check('deleting the piece with its caption leaves no gap before the next caption', wait(lambda: captions() == after, 5),
            captions())
    time.sleep(1)
    r.shot('delete-with-captions')
    # Cutting them and pasting them back at the same place puts every caption back over its picture.
    r.key('z', ctrlKey=True)
    wait(lambda: len(captions()) == 2, 5)
    r.s.run(CLICK_CLIP, piece['id'], {})
    r.s.run(CLICK_CLIP, caption, add)
    wait(lambda: r.s.run(SELECTION) == [piece['id'], caption], 5)
    r.key('x', ctrlKey=True)
    wait(lambda: captions() == after, 5)
    r.seek(piece['startUs'])
    r.key('v', ctrlKey=True)
    layout = lambda: [(c['startUs'], c['durationUs']) for c in r.track()]
    r.check('Ctrl+X and Ctrl+V back at the same place leave the video and both captions where they were',
            wait(lambda: layout() == [(c['startUs'], c['durationUs']) for c in undone] and captions() == [
                ('Piece', piece['startUs'] + 200_000), ('Wide', wide['startUs'] + 200_000)], 5), {'video': layout(), 'captions': captions()})
    for count in (1, 2):
        r.key('z', ctrlKey=True)
        wait(lambda: len(captions()) == count, 5)
    r.key('z', ctrlKey=True)
    r.check('undo takes the captions away again', wait(lambda: r.track() == undone and captions() == [], 5), captions())
    # Opening the Audio tab once blanked the whole window: its store selector built a new list on every read.
    r.import_media(FIXTURES / 'music.mp3')
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === 'Audio').click()")
    listed = wait(lambda: r.s.run("return document.body.innerText.includes('music.mp3')"), 5)
    r.check('the Audio tab opens and lists the imported music', listed)

    # Fades as in CapCut: a 4 s tone on an audio track from 0 s, its fade-in dot dragged 100 px at 100 px a second.
    tone = r.work / 'tone.wav'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', f'sine=frequency={TONE}:sample_rate={RATE}:duration=4',
                    '-c:a', 'pcm_s16le', str(tone)], check=True)
    r.import_media(tone)
    asset = next(a['id'] for a in r.state()['assets'] if a['name'] == 'tone.wav')
    r.s.call("window.__nuzky.store.getState().edit({type: 'addClip', assetId: arguments[0], startUs: 0, trackId: null})", asset)
    clip = wait(lambda: next((c['id'] for t in r.state()['tracks'] for c in t['clips'] if c['assetId'] == asset), None), 10)
    if not clip:
        raise RuntimeError('the tone was not placed')
    r.seek(0)
    r.s.run('window.__nuzky.store.getState().setZoom(100)')
    r.focus_clip(clip)
    time.sleep(0.5)
    r.check('the selected sound clip shows a fade dot at each end', r.s.run(KNOBS, clip) == 2, r.s.run(KNOBS, clip))
    r.s.run(FADE_DRAG, clip, 'in', 100)
    r.check('dragging the fade-in dot 100 px shows "Fade in 1.0 s"', wait(lambda: r.s.run(TIP, 'Fade in 1.0 s'), 5))
    r.shot('fade-drag')
    r.s.run(RELEASE)
    r.check('letting go fades the sound in over 1 s', wait(lambda: r.s.run(FADES, clip) == [1_000_000, 0], 5), r.s.run(FADES, clip))
    project = lambda: json.loads(r.saved_project().read_text())
    saved_fades = lambda: next(([c['content']['fadeInUs'], c['content']['fadeOutUs']] for t in project()['tracks']
                                for c in t['clips'] if c['id'] == clip), None)
    r.check('the project file on disk has the 1 s fade-in', wait(lambda: saved_fades() == [1_000_000, 0], 10), saved_fades())

    # Esc in the middle of a drag puts everything back; the next Esc clears the selection.
    r.s.run(FADE_DRAG, clip, 'out', -80)
    r.check('dragging the fade-out dot 80 px shows "Fade out 0.8 s"', wait(lambda: r.s.run(TIP, 'Fade out 0.8 s'), 5))
    r.key('Escape')
    r.s.run(RELEASE)
    time.sleep(1.5)
    r.check('Esc in the middle of the drag changes nothing, on screen or on disk',
            r.s.run(FADES, clip) == [1_000_000, 0] and saved_fades() == [1_000_000, 0], [r.s.run(FADES, clip), saved_fades()])
    r.check('the value hides and the clip stays selected', not r.s.run(TIP, 'Fade out 0.8 s') and r.s.run(SELECTION) == [clip],
            r.s.run(SELECTION))
    r.key('Escape')
    r.check('the next Esc clears the selection', wait(lambda: r.s.run(SELECTION) == [], 5), r.s.run(SELECTION))
    r.focus_clip(clip)
    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    wait(lambda: r.s.run('return [innerWidth, innerHeight]') == [1024, 640], 5)
    time.sleep(1)
    r.shot('fade-small-window')

    # What a listener gets: the tone's level in 50 ms windows against its level once the fade is over (1.5-3.5 s).
    # The fade is linear, so a window starting at t s is about 20·log10(t + 0.025) dB down.
    levels = tone_levels(decode(export(r, 'fade.mp4'), f'highpass=f={TONE - 500},highpass=f={TONE - 500}'))
    full = statistics.median(levels[30:70])
    at = lambda seconds: levels[round(seconds * RATE / WINDOW)] - full
    rise = ', '.join(f'{s:.2f} s {at(s):+.1f} dB' for s in (0, 0.25, 0.5, 0.75, 1.05))
    r.check('the export starts the tone at least 20 dB down', at(0) < -20, rise)
    r.check('half way through the fade it is 6 dB down', abs(at(0.5) + 5.6) < 1.5, rise)
    r.check('after the fade it plays at full level', abs(at(1.05)) < 1, rise)

    # While an AI agent edits, the dots are gone, as the trim handles are.
    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Look at the fades'})
        r.check('while the AI edits, the clip has no fade dots',
                wait(lambda: r.state()['aiRun'], 10) and wait(lambda: r.s.run(KNOBS, clip) == 0, 5), r.s.run(KNOBS, clip))
        bridge.call('end_run', {'run_id': run['run_id'], 'action': 'keep'})
    finally:
        bridge.close()
    r.check('they come back when the run ends', wait(lambda: r.s.run(KNOBS, clip) == 2, 10), r.s.run(KNOBS, clip))

    # Fades meeting in the middle of the clip: each dot is still the one under the pointer at its place.
    r.s.call("window.__nuzky.store.getState().edit({type: 'updateClip', clipId: arguments[0], fadeInUs: 2000000, fadeOutUs: 2000000})", clip)
    wait(lambda: r.s.run(FADES, clip) == [2_000_000, 2_000_000], 5)
    under = [r.s.run(UNDER, clip, end) for end in ('in', 'out')]
    r.check('with both fades at half the clip, either dot can still be grabbed', under == ['in', 'out'], under)

    r.check('no error toast', not r.errors(), r.errors())

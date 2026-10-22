import time as clock

from e2e.harness import FIXTURES, flow, wait

SELECTION = 'return window.__nuzky.store.getState().selection'
TIME = 'return window.__nuzky.store.getState().timeUs'
FPS = 'return window.__nuzky.store.getState().snap.project.canvas.fps'
RECT = """const b = document.querySelector(`[data-clip-id='${arguments[0]}']`).getBoundingClientRect();
return [b.left, b.top, b.right, b.bottom];"""
# What a press at a point of the page lands on: empty lane space or something else.
EMPTY = 'return document.elementFromPoint(arguments[0], arguments[1])?.hasAttribute("data-lane") ?? false'
# Presses at (arguments[0], arguments[1]) with the modifier keys in arguments[2] and moves through the points of
# arguments[3], without letting go.
PRESS = """const [x, y, keys, path] = arguments;
const at = (type, x, y) => new PointerEvent(type, Object.assign({bubbles: true, button: 0, pointerId: 1, clientX: x, clientY: y}, keys));
document.elementFromPoint(x, y).dispatchEvent(at('pointerdown', x, y));
for (const [px, py] of path) window.dispatchEvent(at('pointermove', px, py));"""
RELEASE = """const [x, y, keys] = arguments;
window.dispatchEvent(new PointerEvent('pointerup', Object.assign({bubbles: true, button: 0, pointerId: 1, clientX: x, clientY: y}, keys)));"""
BOX = "return !!document.querySelector('[data-box]')"
# The timeline scroller's client box and scroll position, and a way to scroll it.
VIEW = """const el = document.querySelector('[data-ruler]').closest('.overflow-auto'); const r = el.getBoundingClientRect();
return [r.left, r.top, r.left + el.clientWidth, r.top + el.clientHeight, el.scrollLeft];"""
SCROLL_TO = "document.querySelector('[data-ruler]').closest('.overflow-auto').scrollLeft = arguments[0]"
CUT = 'return window.__nuzky.store.getState().cut'
HOVER_UNDO = """document.querySelector('[data-peek-button=undo]').dispatchEvent(new PointerEvent(arguments[0] ? 'pointerover' : 'pointerout',
  {bubbles: true, pointerId: 1, relatedTarget: arguments[0] ? null : document.body}));"""
PEEK = "return document.querySelector('[data-testid=history-peek]')?.innerText ?? null"
CAPTION = """window.__nuzky.store.getState().edit({type: 'addText', startUs: arguments[0], text: 'Caption',
  style: {fontSize: 64, color: '#ffffff', bold: false, strokeWidth: 0, strokeColor: '#000000'}})"""


@flow('timeline_select', 'A click on empty timeline space, below the tracks too, moves the playhead there and clears the '
      'selection; a drag from empty space selects the clips its box touches, Shift adds to the selection, Esc puts it back')
def timeline_select(r):
    r.import_media(FIXTURES / 'talk.mp4', FIXTURES / 'wide.mp4')
    for name in ('talk.mp4', 'wide.mp4'):
        r.add_clip(name)
    r.s.call(CAPTION, 500_000)
    caption = wait(lambda: next((t['clips'][0]['id'] for t in r.state()['tracks'] if t['kind'] == 'text' and t['clips']), None))
    first, second = (c['id'] for c in r.track())
    r.s.run('window.__nuzky.store.getState().setZoom(60)')
    clock.sleep(0.5)
    fps = r.s.run(FPS)
    rect = lambda clip: r.s.run(RECT, clip)
    a, b = rect(first), rect(second)
    below = a[3] + 40
    r.check('the space below the last track is empty lane space', r.s.run(EMPTY, a[0] + 30, below), below)

    # A click below the tracks, 1.2 s into the first clip.
    r.s.run('window.__nuzky.store.getState().select([arguments[0]])', caption)
    x = a[0] + 72
    r.s.run(PRESS, x, below, {}, [])
    r.s.run(RELEASE, x, below, {})
    expected = (x - a[0]) / 60 * 1_000_000
    landed = wait(lambda: abs(r.s.run(TIME) - expected) < 1_000_000 / fps + 20_000, 5)
    r.check('a click below the tracks moves the playhead to that point', landed, (r.s.run(TIME), expected))
    r.check('and lands on a frame', r.s.run(TIME) in [round(i * 1_000_000 / fps) for i in range(5 * fps)], r.s.run(TIME))
    r.check('and clears the selection', r.s.run(SELECTION) == [], r.s.run(SELECTION))

    # A click on a clip selects it and leaves the playhead alone.
    before = r.s.run(TIME)
    r.s.run(PRESS, a[0] + 20, (a[1] + a[3]) / 2, {}, [])
    r.s.run(RELEASE, a[0] + 20, (a[1] + a[3]) / 2, {})
    r.check('a click on a clip selects it', wait(lambda: r.s.run(SELECTION) == [first], 5), r.s.run(SELECTION))
    r.check('and does not move the playhead', r.s.run(TIME) == before, r.s.run(TIME))

    # A box from below the tracks up into both main-track clips, short of the caption track.
    start = (a[0] + 10, below)
    path = [(start[0] + 20, below - 10), (b[0] + 30, a[1] + 10)]
    r.s.run(PRESS, *start, {}, path)
    r.check('dragging from empty space draws a box', wait(lambda: r.s.run(BOX), 5))
    r.check('the box selects both video clips it touches, not the caption above it',
            wait(lambda: sorted(r.s.run(SELECTION)) == sorted([first, second]), 5), r.s.run(SELECTION))
    r.shot('box')
    r.s.run(RELEASE, *path[-1], {})
    clock.sleep(0.3)
    r.check('letting go hides the box and keeps the selection',
            not r.s.run(BOX) and sorted(r.s.run(SELECTION)) == sorted([first, second]), r.s.run(SELECTION))
    r.check('a box drag does not move the playhead', r.s.run(TIME) == before, r.s.run(TIME))

    # Shift adds what the box touches; Esc in the middle puts the selection back.
    c = rect(caption)
    lane = (c[2] + 40, (c[1] + c[3]) / 2)
    r.check('the caption track has empty space after the caption', r.s.run(EMPTY, *lane), lane)
    r.s.run(PRESS, *lane, {'shiftKey': True}, [(lane[0] - 20, lane[1]), (c[2] - 5, lane[1] + 2)])
    r.check('Shift and a box over the caption add it to the selection',
            wait(lambda: sorted(r.s.run(SELECTION)) == sorted([first, second, caption]), 5), r.s.run(SELECTION))
    r.key('Escape')
    r.s.run(RELEASE, c[2] - 5, lane[1] + 2, {})
    clock.sleep(0.3)
    r.check('Esc in the middle of a box puts the selection back',
            not r.s.run(BOX) and sorted(r.s.run(SELECTION)) == sorted([first, second]), r.s.run(SELECTION))

    # A selected transition comes back too: it is selected apart from the clips.
    r.s.run('window.__nuzky.store.getState().selectCut(arguments[0])', second)
    r.s.run(PRESS, *start, {}, path)
    r.key('Escape')
    r.s.run(RELEASE, *path[-1], {})
    clock.sleep(0.3)
    r.check('Esc in the middle of a box keeps a selected transition selected', r.s.run(CUT) == second, r.s.run(CUT))
    # A box that replaces the selection takes a selected transition out of it too, even when it touches no clip.
    r.s.run(PRESS, start[0], below + 10, {}, [(start[0] + 40, below + 30)])
    r.s.run(RELEASE, start[0] + 40, below + 30, {})
    clock.sleep(0.3)
    r.check('a box over empty space clears a selected transition', r.s.run(CUT) is None and r.s.run(SELECTION) == [],
            [r.s.run(CUT), r.s.run(SELECTION)])

    # Undo's preview draws an earlier project, whose clips a box would select; it waits while a box is drawn.
    r.s.run(PRESS, *start, {}, path)
    r.s.run(HOVER_UNDO, True)
    clock.sleep(1)
    r.check('resting on Undo during a box shows no preview', r.s.run(PEEK) is None, r.s.run(PEEK))
    r.s.run(HOVER_UNDO, False)
    r.s.run(RELEASE, *path[-1], {})

    # Zoomed in, the second clip is out of view; holding the box at the right edge scrolls the timeline on to it.
    r.s.run('window.__nuzky.store.getState().select([]); window.__nuzky.store.getState().setZoom(200)')
    clock.sleep(0.5)
    r.s.run(SCROLL_TO, 0)
    view = r.s.run(VIEW)
    a = rect(first)
    r.check('zoomed in, the second clip starts past the right edge', rect(second)[0] > view[2], (rect(second), view))
    edge = (view[2] - 4, a[1] + 10)
    r.s.run(PRESS, a[0] + 100, a[3] + 30, {}, [(a[0] + 120, a[3] + 30), edge])
    reached = wait(lambda: second in r.s.run(SELECTION), 15)
    r.check('holding the box at the right edge scrolls the timeline until it reaches the second clip', reached,
            (r.s.run(SELECTION), r.s.run(VIEW)))
    r.shot('box-scrolled')
    r.s.run(RELEASE, *edge, {})

    # While playing the playhead keeps itself in view, but not during a box, which would then jerk back and forth.
    r.s.run(SCROLL_TO, 0)
    r.seek(0)
    r.s.run('window.__nuzky.store.getState().togglePlay()')
    r.check('playback runs', wait(lambda: r.s.run('return window.__nuzky.store.getState().playing'), 5))
    r.s.run(PRESS, a[0] + 100, a[3] + 30, {}, [(a[0] + 300, a[1] + 10)])
    wait(lambda: r.s.run(BOX), 5)
    r.seek(16_000_000)
    clock.sleep(0.6)
    r.check('during a box the playing playhead does not scroll the timeline to itself', r.s.run(VIEW)[4] == 0, r.s.run(VIEW))
    r.s.run(RELEASE, a[0] + 300, a[1] + 10, {})
    r.s.run('window.__nuzky.store.getState().togglePlay()')
    r.check('nothing in the app failed', not r.errors(), r.errors())


@flow('timeline_move', 'Dragging one of several selected captions moves them all by the same time as one undo step and keeps '
      'them selected; when one would land on another caption they all go onto one new track right above, and a selected '
      'main-track clip stays in its place')
def timeline_move(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    segment = lambda start, text: {'startUs': start, 'endUs': start + 600_000, 'text': text, 'words': []}
    r.s.call("window.__nuzky.store.getState().edit({type: 'addCaptions', segments: arguments[0],"
             " style: {fontSize: 60, color: '#ffffff'}})",
             [segment(1_000_000, 'One'), segment(1_600_000, 'Two'), segment(2_200_000, 'Three'), segment(4_000_000, 'Four')])
    captions = lambda: {c['text']: (t['id'], c['startUs']) for t in r.state()['tracks'] if t['kind'] == 'text' for c in t['clips']}
    wait(lambda: len(captions()) == 4, 5)
    ids = {c['text']: c['id'] for t in r.state()['tracks'] if t['kind'] == 'text' for c in t['clips']}
    track = captions()['One'][0]
    video = r.track()[0]
    r.s.run('window.__nuzky.store.getState().setZoom(100)')
    r.s.run("document.querySelector('[aria-label=\"Snapping on\"]').click()")
    r.check('snapping turns off', wait(lambda: r.s.run("return !!document.querySelector('[aria-label=\"Snapping off\"]')"), 5))
    clock.sleep(0.5)
    middle = lambda clip: (lambda b: ((b[0] + b[2]) / 2, (b[1] + b[3]) / 2))(r.s.run(RECT, clip))
    click = lambda clip, keys: (r.s.run(PRESS, *middle(clip), keys, []), r.s.run(RELEASE, *middle(clip), keys))

    click(ids['One'], {})
    click(ids['Two'], {'shiftKey': True})
    click(ids['Three'], {'shiftKey': True})
    picked = sorted(ids[t] for t in ('One', 'Two', 'Three'))
    r.check('Shift-clicks select three captions', wait(lambda: sorted(r.s.run(SELECTION)) == picked, 5), r.s.run(SELECTION))

    # Grabbing the middle one and moving it 50 px, half a second at this zoom, takes the other two along.
    x, y = middle(ids['Two'])
    r.s.run(PRESS, x, y, {}, [(x + 10, y), (x + 50, y)])
    clock.sleep(0.3)
    r.shot('group-move')
    r.s.run(RELEASE, x + 50, y, {})
    moved = {'One': (track, 1_500_000), 'Two': (track, 2_100_000), 'Three': (track, 2_700_000), 'Four': (track, 4_000_000)}
    r.check('every selected caption moves half a second on its track, the one not selected stays',
            wait(lambda: captions() == moved, 5), captions())
    r.check('the three captions stay selected', sorted(r.s.run(SELECTION)) == picked, r.s.run(SELECTION))

    # With the video selected too, a drag a second on would put Three over Four: the three go onto one new track
    # right above the captions, keeping their offsets, and the magnetic main track stays as it was.
    click(video['id'], {'shiftKey': True})
    wait(lambda: video['id'] in r.s.run(SELECTION), 5)
    x, y = middle(ids['Three'])
    r.s.run(PRESS, x, y, {}, [(x + 10, y), (x + 100, y)])
    r.s.run(RELEASE, x + 100, y, {})
    wait(lambda: captions()['One'][1] == 2_500_000, 5)
    tracks = [t['id'] for t in r.state()['tracks']]
    now = captions()
    r.check('a group that would overlap a caption goes together onto a new track right above, a second later',
            [now[t][1] for t in ('One', 'Two', 'Three')] == [2_500_000, 3_100_000, 3_700_000]
            and len({now[t][0] for t in ('One', 'Two', 'Three')}) == 1
            and tracks.index(now['One'][0]) == tracks.index(track) + 1 and now['Four'] == (track, 4_000_000), (now, tracks))
    r.check('the selected main-track clip stays where it was', r.track()[0]['startUs'] == video['startUs'] and len(r.track()) == 1,
            r.track())
    r.check('and everything stays selected', sorted(r.s.run(SELECTION)) == sorted(picked + [video['id']]), r.s.run(SELECTION))

    r.key('z', ctrlKey=True)
    r.check('one undo puts the three back on the captions track', wait(lambda: captions() == moved, 5), captions())

    # With only the main-track clip selected beside it, a caption dragged up past the tracks still moves in time only.
    click(ids['Four'], {})
    click(video['id'], {'shiftKey': True})
    pair = sorted([ids['Four'], video['id']])
    wait(lambda: sorted(r.s.run(SELECTION)) == pair, 5)
    x, y = middle(ids['Four'])
    r.s.run(PRESS, x, y, {}, [(x + 10, y), (x + 30, y - 80)])
    r.s.run(RELEASE, x + 30, y - 80, {})
    r.check('a caption selected with the main-track clip moves 0.3 s on its own track',
            wait(lambda: captions()['Four'] == (track, 4_300_000), 5), captions())
    r.check('and both stay selected', sorted(r.s.run(SELECTION)) == pair, r.s.run(SELECTION))
    r.check('nothing in the app failed', not r.errors(), r.errors())

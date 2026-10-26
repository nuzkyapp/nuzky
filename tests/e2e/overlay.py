"""Putting a picture over the video. Overlay on a media tile puts it over the whole picture at the playhead on a track
above the main one, without touching the talking head's clip; the tile's menu, opened from the keyboard, offers the same
and picture in picture."""
import time

from PIL import Image, ImageStat

from e2e.harness import FIXTURES, Bridge, changed_share, flow, preview_crop, preview_rect, preview_redraw, wait, webdriver

AT_US = 1_000_000
OVERLAY = "document.querySelector('[aria-label=\"Put red.png over the video\"]')"


def project(r):
    return r.s.run('return window.__nuzky.store.getState().snap.project', retries=3)


def placed(r, asset_id):
    """(track index, clip) of every clip of the asset off the main track."""
    return [(i, c) for i, t in enumerate(project(r)['tracks']) if i > 0 for c in t['clips'] if c['content'].get('assetId') == asset_id]


def region_change(a, b, area):
    return changed_share(a.crop(tuple(round(v) for v in area)), b.crop(tuple(round(v) for v in area)))


def middle(image):
    """Average colour of the middle of a preview crop."""
    w, h = image.size
    return ImageStat.Stat(image.crop((w // 2 - 10, h // 2 - 10, w // 2 + 10, h // 2 + 10))).mean


@flow('overlay', 'Overlay on a picture\'s tile puts it over the whole video at the playhead on a track above the main one, '
      'one undo takes it back, and the tile\'s menu from the keyboard offers it and picture in picture')
def overlay(r):
    Image.new('RGB', (1280, 720), (230, 30, 30)).save(r.work / 'red.png')
    r.import_media(FIXTURES / 'talk.mp4', r.work / 'red.png')
    r.add_clip('talk.mp4')
    red = next(a['id'] for a in r.state()['assets'] if a['name'] == 'red.png')
    main = r.track()
    r.seek(AT_US)
    r.s.run('window.__nuzky.store.getState().select([])')
    rect = preview_rect(r)
    drawn = lambda: r.s.run("return document.querySelector('section[aria-label=Preview] canvas').dataset.us")
    if not r.check('the preview draws the frame at the playhead', wait(lambda: drawn() == str(AT_US), 10), drawn()):
        return
    time.sleep(1)
    r.shot('talking-head')
    before = preview_crop(r.work / 'talking-head.png', rect)

    if not r.check('a picture\'s tile offers Overlay', r.s.run(f'return {OVERLAY}?.textContent') == 'Overlay'):
        return
    for width, height, name in ((1440, 900, 'tile'), (1024, 640, 'tile-1024')):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        r.s.run(f'{OVERLAY}.focus()')
        time.sleep(0.5)
        r.shot(name)
        whole = r.s.run(f'const s = {OVERLAY}.firstElementChild; return s.scrollWidth <= s.clientWidth')
        r.check(f'the whole word Overlay fits on the tile in a {width} × {height} window', whole)
    webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
    wait(lambda: r.s.run('return window.innerWidth') == 1440, 5)
    rect = preview_rect(r)

    r.s.run(f'{OVERLAY}.click()')
    found = wait(lambda: placed(r, red) or None, 10)
    if not r.check('the picture lands on a track right above the main track', found and found[0][0] == 1, found):
        return
    clip = found[0][1]
    r.check('it starts at the playhead and lasts 3 s', (clip['startUs'], clip['durationUs']) == (AT_US, 3_000_000),
            (clip['startUs'], clip['durationUs']))
    t = clip['content']['transform']
    r.check('it covers the picture as on the main track, without a window\'s corners',
            (t['x'], t['y'], t['scale']) == (0, 0, 1) and not clip['content'].get('shape'), clip['content'])
    r.check('the new clip is selected', r.s.run('return window.__nuzky.store.getState().selection') == [clip['id']])
    r.check('the talking head\'s clip stays as it was', r.track() == main)

    r.s.run('window.__nuzky.store.getState().select([])')
    after, redrawn = preview_redraw(r, 'overlay', before)
    red_middle = middle(after)
    r.check('the preview shows the picture over the middle of the video',
            redrawn and red_middle[0] > 170 and red_middle[1] < 90, [round(v) for v in red_middle])
    # A 16:9 picture fitted to a 9:16 frame leaves the talking head above and below it.
    top = region_change(before, after, (0, 0, after.width, after.height * 0.25))
    r.check('the talking head still shows above the picture', top < 0.01, round(top, 4))

    r.key('z', ctrlKey=True)
    r.check('one undo removes the picture and its track', wait(lambda: not placed(r, red) and len(project(r)['tracks']) == 1, 5))

    # The menu from the keyboard: Shift+F10 on the tile's focused button, then ↓ ↓ Enter.
    r.s.run("document.querySelector('[aria-label=\"Add red.png at playhead\"]').focus()")
    r.key('F10', shiftKey=True)
    items = lambda: r.s.run("const m = document.querySelector('[role=menu][aria-label=\"red.png\"]');"
                            "return m && [...m.querySelectorAll('[role=menuitem]')].map((i) => i.textContent)")
    shown = wait(items, 5)
    r.check('Shift+F10 opens the tile\'s menu', shown == ['Add at playhead', 'Overlay', 'Picture in picture', 'Remove'], shown)
    time.sleep(0.3)
    r.shot('menu')
    for key in ('ArrowDown', 'ArrowDown', 'Enter'):
        r.key(key)
    found = wait(lambda: placed(r, red) or None, 10)
    shape = found and found[0][1]['content'].get('shape') or {}
    r.check('Picture in picture from the menu puts it in a rounded window', found and abs(shape.get('radius', 0) - 0.15) < 1e-6, shape)

    # A picture larger than the frame both ways fills it; the 16:9 one above was only fitted.
    r.key('z', ctrlKey=True)
    wait(lambda: not placed(r, red), 5)
    Image.new('RGB', (2000, 2400), (30, 60, 230)).save(r.work / 'blue.png')
    r.import_media(r.work / 'blue.png')
    blue = next(a['id'] for a in r.state()['assets'] if a['name'] == 'blue.png')
    r.s.run('window.__nuzky.store.getState().select([])')
    r.s.run("document.querySelector('[aria-label=\"Put blue.png over the video\"]').click()")
    found = wait(lambda: placed(r, blue) or None, 10)
    scale = found and found[0][1]['content']['transform']['scale']
    r.check('a picture larger than the frame fills it', scale and abs(scale - 1.481) < 1e-4, scale)
    r.s.run('window.__nuzky.store.getState().select([])')
    after, redrawn = preview_redraw(r, 'overlay-fill', before)
    corner = ImageStat.Stat(after.crop((4, 4, 24, 24))).mean
    r.check('the preview shows it over the whole frame, corners included', redrawn and corner[2] > 170 and corner[0] < 90, [round(v) for v in corner])
    r.check('no error toast', not r.errors(), r.errors())


ZOOM = 60
# Presses the middle of a media tile and drags it to `arguments[1]`, `arguments[2]`, without letting go.
DRAG_TILE = """const tile = document.querySelector(`[aria-label='Add ${arguments[0]} at playhead']`).closest('.cursor-grab');
const b = tile.getBoundingClientRect();
const at = (type, x, y) => new PointerEvent(type, {bubbles: true, button: 0, pointerId: 1, clientX: x, clientY: y});
const [x, y] = [b.left + b.width / 2, b.top + b.height / 3];
document.elementFromPoint(x, y).dispatchEvent(at('pointerdown', x, y));
window.dispatchEvent(at('pointermove', x + 20, y));
window.dispatchEvent(at('pointermove', arguments[1], arguments[2]));"""
RELEASE = """window.dispatchEvent(new PointerEvent('pointerup', {bubbles: true, button: 0, pointerId: 1,
  clientX: arguments[0], clientY: arguments[1]}))"""
PRESS = """const [x, y, path] = arguments;
const at = (type, x, y) => new PointerEvent(type, {bubbles: true, button: 0, pointerId: 1, clientX: x, clientY: y});
document.elementFromPoint(x, y).dispatchEvent(at('pointerdown', x, y));
for (const [px, py] of path) window.dispatchEvent(at('pointermove', px, py));"""
RECT = """const el = document.querySelector(`[data-clip-id='${arguments[0]}']`); const b = el.getBoundingClientRect();
return [b.left, b.top, b.right, b.bottom, el.closest('[role=listbox]').getBoundingClientRect().bottom];"""
LANE = "const l = document.querySelector('[data-testid=overlay-lane]'); return l && [l.getBoundingClientRect().top, l.getBoundingClientRect().bottom, l.textContent]"


@flow('overlay-lane', 'Over a talking head on the main track, an empty lane says where a picture over the video goes; a '
      'picture dragged there lights it up and lands over the video at the drop line on a track above the main one')
def overlay_lane(r):
    Image.new('RGB', (1280, 720), (230, 30, 30)).save(r.work / 'red.png')
    r.import_media(FIXTURES / 'talk.mp4', r.work / 'red.png')
    r.add_clip('talk.mp4')
    red = next(a['id'] for a in r.state()['assets'] if a['name'] == 'red.png')
    main = r.track()
    r.s.run('const st = window.__nuzky.store.getState(); st.select([]); st.seek(0); st.setZoom(arguments[0])', ZOOM)
    lane = wait(lambda: r.s.run(LANE), 5)
    if not r.check('an empty lane above the main track says what goes there',
                   lane and lane[2] == 'Drag a video or picture here to put it over the video', lane):
        return
    main_top = r.s.run("return document.querySelector(`[data-clip-id='${arguments[0]}']`).getBoundingClientRect().top", main[0]['id'])
    r.check('the lane sits right above the main track', lane[1] <= main_top, [lane, main_top])
    time.sleep(0.5)
    r.shot('lane')

    left = r.s.run("return document.querySelector(`[data-clip-id='${arguments[0]}']`).getBoundingClientRect().left", main[0]['id'])
    x, y = left + 2 * ZOOM, (lane[0] + lane[1]) / 2
    r.s.run(DRAG_TILE, 'red.png', x, y)
    hot = wait(lambda: (l := r.s.run(LANE)) and l[2] == 'Drop to put it over the video', 5)
    r.check('dragged over it, the lane says the picture will go over the video', hot, r.s.run(LANE))
    line = r.s.run("return document.querySelector('[data-testid=drop-line]')?.getBoundingClientRect().left ?? null")
    r.check('the drop line shows where it will start', line is not None and abs(line - x) < 2, [line, x])
    time.sleep(0.3)
    r.shot('lane-drag')
    r.s.run(RELEASE, x, y)

    found = wait(lambda: placed(r, red) or None, 10)
    if not r.check('the picture lands on a track right above the main track', found and found[0][0] == 1, found):
        return
    clip = found[0][1]
    r.check('it starts at the drop line and lasts 3 s', abs(clip['startUs'] - 2_000_000) < 2e6 / ZOOM and clip['durationUs'] == 3_000_000,
            (clip['startUs'], clip['durationUs']))
    t = clip['content']['transform']
    r.check('it covers the picture as Overlay does', (t['x'], t['y'], t['scale']) == (0, 0, 1) and not clip['content'].get('shape'), t)
    r.check('the talking head\'s clip stays as it was', r.track() == main)
    r.check('the new track takes the lane\'s place', wait(lambda: r.s.run(LANE) is None, 5))
    r.s.run('window.__nuzky.store.getState().select([])')
    time.sleep(0.5)
    r.shot('lane-dropped')

    r.key('z', ctrlKey=True)
    r.check('one undo removes the picture and brings the lane back', wait(lambda: not placed(r, red) and r.s.run(LANE) is not None, 5))

    # Files from the desktop over the middle of the talking head: one video offers to replace it, several never do,
    # as several dropped there go in one after another.
    talk_left, talk_top, talk_right, talk_bottom, _ = r.s.run(RECT, main[0]['id'])
    mid = ((talk_left + talk_right) / 2, (talk_top + talk_bottom) / 2)
    chip = lambda: r.s.run("return document.querySelector(`[data-clip-id='${arguments[0]}']`).textContent.includes('Replace')", main[0]['id'])
    for several, expect in ((False, True), (True, False)):
        r.s.run("window.__nuzky.store.setState({fileDrag: {x: arguments[0], y: arguments[1], kind: 'video', several: arguments[2]}})", *mid, several)
        shown = wait(lambda: chip() == expect or None, 3)
        r.check(f'{"several videos" if several else "one video"} from the desktop over a clip {"never offer" if several else "offers"} to replace it', shown, chip())
    r.s.run('window.__nuzky.store.setState({fileDrag: null})')

    # Under the AI lock the lane says why and takes nothing.
    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Edit'})['run_id']
        r.check('the app shows the AI lock', wait(lambda: r.state()['aiRun'], 5))
        lane = wait(lambda: (l := r.s.run(LANE)) and l[2].startswith('AI is editing') and l, 5)
        r.check('under the AI lock the lane says why it takes nothing', lane, r.s.run(LANE))
        if lane:
            y = (lane[0] + lane[1]) / 2
            r.s.run(DRAG_TILE, 'red.png', x, y)
            time.sleep(0.5)
            r.check('dragged over under the lock, it does not light up', r.s.run(LANE)[2].startswith('AI is editing'), r.s.run(LANE))
            r.shot('lane-locked')
            r.s.run(RELEASE, x, y)
            time.sleep(1)
            r.check('and dropping there adds nothing', not placed(r, red))
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
    finally:
        bridge.close()
    wait(lambda: not r.state()['aiRun'], 5)

    # A picture put on the main track by mistake goes up onto the lane, under a title above the main track.
    r.s.call("window.__nuzky.store.getState().edit({type: 'addText', startUs: 0, text: 'Hook', style: {fontSize: 80, color: '#ffffff'}})")
    r.add_clip('red.png')
    title = next(i for i, t in enumerate(project(r)['tracks']) if t['kind'] == 'text')
    shot = next(c for c in project(r)['tracks'][0]['clips'] if c['content'].get('assetId') == red)
    lane = wait(lambda: r.s.run(LANE), 5)
    if not r.check('with a title above the main track the lane still shows, between them', lane, r.s.run(LANE)):
        return
    clip_left, clip_top, _, clip_bottom, _ = r.s.run(RECT, shot['id'])
    sx, sy = clip_left + 20, (clip_top + clip_bottom) / 2
    ly = (lane[0] + lane[1]) / 2
    r.s.run(PRESS, sx, sy, [[sx + 2, sy - 10], [sx, ly]])
    lit = wait(lambda: (l := r.s.run(LANE)) and l[2] == 'Drop to put it over the video', 5)
    r.check('a picture dragged up from the main track lights the lane up', lit, r.s.run(LANE))
    time.sleep(0.3)
    r.shot('lane-clip-drag')
    r.s.run(RELEASE, sx, ly)
    moved = wait(lambda: placed(r, red) or None, 10)
    tracks = project(r)['tracks']
    r.check('it goes onto a new track right above the main track', moved and tracks[moved[0][0]]['kind'] == 'video' and moved[0][0] == 1, moved)
    r.check('it keeps its time and the main track closes the gap', moved and moved[0][1]['startUs'] == shot['startUs'] and r.track() == main,
            [moved and moved[0][1]['startUs'], shot['startUs']])
    r.check('the title stays on its own track', any(t['kind'] == 'text' and t['clips'] for t in tracks), title)
    r.check('no error toast', not r.errors(), r.errors())


ZONE = "const z = document.querySelector(`[data-zone=${arguments[0]}]`); if (!z) return null; const b = z.getBoundingClientRect(); return [b.left + b.width / 2, b.top + b.height / 2, !!z.dataset.hot]"


@flow('overlay-preview', 'A picture dragged over the preview shows where it can go: dropped on the frame it goes over the '
      'video at the playhead, dropped on the corner box it becomes a picture in picture there')
def overlay_preview(r):
    Image.new('RGB', (1280, 720), (230, 30, 30)).save(r.work / 'red.png')
    r.import_media(FIXTURES / 'talk.mp4', r.work / 'red.png')
    r.add_clip('talk.mp4')
    red = next(a['id'] for a in r.state()['assets'] if a['name'] == 'red.png')
    main = r.track()
    r.seek(AT_US)
    r.s.run('window.__nuzky.store.getState().select([])')
    rect = preview_rect(r)
    middle_x, middle_y = rect['left'] + rect['width'] / 2, rect['top'] + rect['height'] * 0.6

    r.s.run(DRAG_TILE, 'red.png', middle_x, middle_y)
    frame = wait(lambda: r.s.run(ZONE, 'overlay'), 5)
    corner = r.s.run(ZONE, 'pictureInPicture')
    r.check('while a picture is dragged, the preview offers the frame and the corner box', frame and corner, [frame, corner])
    r.check('the frame under the pointer lights up', frame and frame[2] and not corner[2], [frame, corner])
    time.sleep(0.3)
    r.shot('preview-drag')
    r.s.run(RELEASE, middle_x, middle_y)
    found = wait(lambda: placed(r, red) or None, 10)
    if not r.check('dropped on the frame, the picture lands on a track above the main track', found and found[0][0] == 1, found):
        return
    clip = found[0][1]
    t = clip['content']['transform']
    r.check('it goes over the whole video at the playhead',
            clip['startUs'] == AT_US and (t['x'], t['y'], t['scale']) == (0, 0, 1) and not clip['content'].get('shape'), clip)
    r.check('the talking head\'s clip stays as it was', r.track() == main)
    r.check('no zones show once it is dropped', not r.s.run(ZONE, 'overlay'))
    r.key('z', ctrlKey=True)
    wait(lambda: not placed(r, red), 5)

    for width, height, name in ((1440, 900, 'preview-drag-pip'), (1024, 640, 'preview-drag-pip-1024')):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        rect = preview_rect(r)
        r.s.run(DRAG_TILE, 'red.png', rect['left'] + 4, rect['top'] + rect['height'] / 2)
        box = wait(lambda: r.s.run(ZONE, 'pictureInPicture'), 5)
        r.s.run("window.dispatchEvent(new PointerEvent('pointermove', {bubbles: true, button: 0, pointerId: 1, clientX: arguments[0], clientY: arguments[1]}))",
                box[0], box[1])
        hot = wait(lambda: (z := r.s.run(ZONE, 'pictureInPicture')) and z[2], 5)
        r.check(f'the corner box under the pointer lights up in a {width} × {height} window', hot)
        label = r.s.run("const z = document.querySelector('[data-zone=pictureInPicture]'), s = z.firstElementChild.getBoundingClientRect(),"
                        "b = z.getBoundingClientRect(), f = document.querySelector('[data-testid=preview-drop]').getBoundingClientRect();"
                        "return s.left >= f.left && s.right <= f.right && s.bottom <= f.bottom && s.top >= b.bottom")
        r.check(f'its label shows whole in the frame, under the box, in a {width} × {height} window', label)
        time.sleep(0.3)
        r.shot(name)
        r.s.run(RELEASE, box[0], box[1])
        found = wait(lambda: placed(r, red) or None, 10)
        shape = found and found[0][1]['content'].get('shape') or {}
        r.check(f'dropped there in a {width} × {height} window, it becomes a picture in picture at the playhead',
                found and found[0][1]['startUs'] == AT_US and abs(shape.get('radius', 0) - 0.15) < 1e-6, found)
        r.key('z', ctrlKey=True)
        wait(lambda: not placed(r, red), 5)

    # The corner box's label is part of it: a drop on the words makes a picture in picture too.
    LABEL = "const b = document.querySelector('[data-zone=pictureInPicture]').firstElementChild.getBoundingClientRect(); return [b.left + b.width / 2, b.top + b.height / 2]"
    rect = preview_rect(r)
    r.s.run(DRAG_TILE, 'red.png', rect['left'] + 4, rect['top'] + rect['height'] / 2)
    lx, ly = wait(lambda: r.s.run(ZONE, 'pictureInPicture') and r.s.run(LABEL), 5)
    r.s.run("window.dispatchEvent(new PointerEvent('pointermove', {bubbles: true, button: 0, pointerId: 1, clientX: arguments[0], clientY: arguments[1]}))", lx, ly)
    r.check('over the corner box\'s label, the box lights up', wait(lambda: (z := r.s.run(ZONE, 'pictureInPicture')) and z[2], 5))
    r.s.run(RELEASE, lx, ly)
    found = wait(lambda: placed(r, red) or None, 10)
    shape = found and found[0][1]['content'].get('shape') or {}
    r.check('dropped on the label, it becomes a picture in picture', found and abs(shape.get('radius', 0) - 0.15) < 1e-6, found)
    r.key('z', ctrlKey=True)
    wait(lambda: not placed(r, red), 5)

    # Under the AI lock the preview offers nothing and takes nothing.
    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Edit'})['run_id']
        r.check('the app shows the AI lock', wait(lambda: r.state()['aiRun'], 5))
        rect = preview_rect(r)
        x, y = rect['left'] + rect['width'] / 2, rect['top'] + rect['height'] * 0.6
        r.s.run(DRAG_TILE, 'red.png', x, y)
        time.sleep(0.5)
        r.check('under the AI lock a dragged picture shows no places on the preview', not r.s.run(ZONE, 'overlay'))
        r.s.run(RELEASE, x, y)
        time.sleep(1)
        r.check('and dropping it there adds nothing', not placed(r, red))
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
    finally:
        bridge.close()
    r.check('no error toast', not r.errors(), r.errors())

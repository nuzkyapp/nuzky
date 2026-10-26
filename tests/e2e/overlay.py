"""Putting a picture over the video. Overlay on a media tile puts it over the whole picture at the playhead on a track
above the main one, without touching the talking head's clip; the tile's menu, opened from the keyboard, offers the same
and picture in picture."""
import time

from PIL import Image, ImageStat

from e2e.harness import FIXTURES, changed_share, flow, preview_crop, preview_rect, preview_redraw, wait, webdriver

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
    r.check('Shift+F10 opens the tile\'s menu', shown == ['Add at playhead', 'Over the video', 'Picture in picture', 'Remove'], shown)
    time.sleep(0.3)
    r.shot('menu')
    for key in ('ArrowDown', 'ArrowDown', 'Enter'):
        r.key(key)
    found = wait(lambda: placed(r, red) or None, 10)
    shape = found and found[0][1]['content'].get('shape') or {}
    r.check('Picture in picture from the menu puts it in a rounded window', found and abs(shape.get('radius', 0) - 0.15) < 1e-6, shape)
    r.check('no error toast', not r.errors(), r.errors())

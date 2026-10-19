"""Layers of pictures over the main track. Two pictures in picture in the same corner: the one on the higher track
covers the other. Dropping the top one on the line between the lower one's track and the main track puts it on a new
track there, under the other one, and the preview shows the other picture on top. Alt+↑ and Alt+↓ move the focused
clip a layer up or down, never under the main track."""
import json, time as clock

from PIL import Image

from e2e.harness import FIXTURES, flow, preview_crop, preview_rect, wait

AT_US = 1_000_000
PRESS = """const [x, y, path] = arguments;
const at = (type, x, y) => new PointerEvent(type, {bubbles: true, button: 0, pointerId: 1, clientX: x, clientY: y});
document.elementFromPoint(x, y).dispatchEvent(at('pointerdown', x, y));
for (const [px, py] of path) window.dispatchEvent(at('pointermove', px, py));"""
RELEASE = """window.dispatchEvent(new PointerEvent('pointerup', {bubbles: true, button: 0, pointerId: 1,
  clientX: arguments[0], clientY: arguments[1]}))"""
# The clip's box and the bottom of its track's lane, in window pixels.
RECT = """const el = document.querySelector(`[data-clip-id='${arguments[0]}']`); const b = el.getBoundingClientRect();
return [b.left, b.top, b.right, b.bottom, el.closest('[role=listbox]').getBoundingClientRect().bottom];"""
GAP = "return !!document.querySelector('[data-testid=track-gap]')"
FOCUSED = "return document.activeElement?.getAttribute('data-clip-id') ?? null"
PIP = """(() => { const st = window.__nuzky.store.getState();
  const asset = st.snap.project.assets.find((a) => a.name === arguments[0]);
  return st.edit({type: 'addPictureInPicture', assetId: asset.id, startUs: 0, durationUs: 3000000}); })()"""


def layers(tracks):
    """The ids of the clips over the main track, one per video track, from the bottom up."""
    return [t['clips'][0]['id'] for t in tracks[1:] if t['kind'] == 'video' and t['clips']]


@flow('layers', 'Drop the top of two overlapping pictures on the line under the other one\'s track and it goes under it, '
      'in the preview and the saved project; Alt+↑/↓ move a focused clip a layer up or down, not under the main track')
def layers_flow(r):
    colours = {'red.png': (230, 30, 30), 'blue.png': (30, 60, 230)}
    for name, rgb in colours.items():
        Image.new('RGB', (1280, 720), rgb).save(r.work / name)
    r.import_media(FIXTURES / 'talk.mp4', *(r.work / name for name in colours))
    r.add_clip('talk.mp4')
    for name in colours:
        before = len(layers(r.state()['tracks']))
        r.s.call(PIP, name)
        r.check(f'{name} goes over the talking head as a picture in picture', wait(lambda: len(layers(r.state()['tracks'])) == before + 1, 10))
    assets = {a['id']: a['name'] for a in r.state()['assets']}
    clip_asset = {c['id']: assets.get(c['assetId']) for t in r.state()['tracks'] for c in t['clips']}
    low, high = layers(r.state()['tracks'])
    r.check('the two pictures are on two tracks, one above the other', {clip_asset[low], clip_asset[high]} == set(colours),
            [clip_asset[low], clip_asset[high]])

    canvas = r.s.run('return window.__nuzky.store.getState().snap.project.canvas')

    def on_top(name):
        """The colour in the middle of the pictures' corner of the preview, shot as `name`."""
        found = r.s.call(f'window.__nuzky.api.layerBounds({AT_US}).then((b) => JSON.stringify(b))')
        corners = next(b['corners'] for b in json.loads(found['value']) if b['clipId'] == high)
        cx = sum(p[0] for p in corners) / 4 / canvas['width']
        cy = sum(p[1] for p in corners) / 4 / canvas['height']
        clock.sleep(1)
        r.shot(name)
        crop = preview_crop(r.work / f'{name}.png', preview_rect(r))
        red, _, blue = crop.getpixel((int(cx * crop.width), int(cy * crop.height)))
        return 'red.png' if red > blue else 'blue.png'

    r.s.run('window.__nuzky.store.getState().select([])')
    r.seek(AT_US)
    r.check('the picture on the higher track covers the other in the preview', wait(lambda: on_top('before') == clip_asset[high], 10),
            on_top('before'))

    # Dragged down onto the line between the lower picture's track and the main track.
    r.s.run('window.__nuzky.store.getState().setZoom(60)')
    clock.sleep(0.5)
    h, l = r.s.run(RECT, high), r.s.run(RECT, low)
    x, y = (h[0] + h[2]) / 2, (h[1] + h[3]) / 2
    gap = l[4] + 1
    r.s.run(PRESS, x, y, [(x, y + 10), (x, gap)])
    r.check('over the line under the lower track a bar shows where the new track goes', wait(lambda: r.s.run(GAP), 5))
    r.shot('drop-between')
    r.s.run(RELEASE, x, gap)
    moved = wait(lambda: layers(r.state()['tracks']) == [high, low], 10)
    r.check('dropped there, the top picture goes on a new track under the other one', moved, layers(r.state()['tracks']))
    starts = {c['id']: c['startUs'] for t in r.state()['tracks'] for c in t['clips']}
    r.check('and keeps its time', starts[high] == 0, starts[high])
    r.check('the main track is still the lowest', r.state()['tracks'][0]['id'] == 'main')
    r.s.run('window.__nuzky.store.getState().select([])')
    r.check('the preview now shows the other picture on top', wait(lambda: on_top('after') == clip_asset[low], 10), on_top('after'))
    saved = lambda: layers(json.loads(r.saved_project().read_text())['tracks'])
    r.check('the saved project has the new order', wait(lambda: saved() == [high, low], 10), saved())

    # The keyboard: Alt+↑ a layer up, Alt+↓ a layer down, and no further down than just over the main track.
    r.focus_clip(high)
    r.key('ArrowUp', altKey=True)
    r.check('Alt+↑ moves the focused picture a layer up, over the other one',
            wait(lambda: layers(r.state()['tracks']) == [low, high], 5), layers(r.state()['tracks']))
    r.check('the moved clip keeps the focus', wait(lambda: r.s.run(FOCUSED) == high, 5), r.s.run(FOCUSED))
    r.key('ArrowDown', altKey=True)
    r.check('Alt+↓ moves it a layer down again', wait(lambda: layers(r.state()['tracks']) == [high, low], 5), layers(r.state()['tracks']))
    revision = r.s.run('return window.__nuzky.store.getState().snap.revision')
    r.key('ArrowDown', altKey=True)
    clock.sleep(0.8)
    r.check('Alt+↓ right over the main track changes nothing',
            layers(r.state()['tracks']) == [high, low] and r.s.run('return window.__nuzky.store.getState().snap.revision') == revision,
            layers(r.state()['tracks']))
    r.check('nothing in the app failed', not r.errors(), r.errors())

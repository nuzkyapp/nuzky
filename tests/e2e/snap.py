"""Snapping in the preview. A picture in picture dragged near the left edge of the frame lands on it, dragged further
away it stays where it is put, and near the middle it centres; its corner scales it to exactly the width of the frame;
its crop bar stops on the middle line; a caption dragged near the middle lands there. A guide shows on the line while
the box sits on it."""
import json, time as clock

from PIL import Image

from e2e.harness import FIXTURES, flow, wait

AT_US = 1_000_000
PIP = """(() => { const st = window.__nuzky.store.getState();
  const asset = st.snap.project.assets.find((a) => a.name === arguments[0]);
  return st.edit({type: 'addPictureInPicture', assetId: asset.id, startUs: 0, durationUs: 3000000}); })()"""
CAPTION = """window.__nuzky.store.getState().edit({type: 'addText', startUs: 0, text: 'Caption',
  style: {fontSize: 64, color: '#ffffff', bold: false, strokeWidth: 0, strokeColor: '#000000'}})"""
PLACE = """window.__nuzky.store.getState().edit({type: 'updateClip', clipId: arguments[0],
  transform: {x: arguments[1], y: arguments[2], scale: arguments[3], rotation: 0, opacity: 1, crop: arguments[4] ?? null}})"""
# The frame in window pixels: the selection box draws over exactly the frame.
FRAME = """const b = document.querySelector('[data-testid=layer-overlay]').getBoundingClientRect();
return {left: b.left, top: b.top, width: b.width};"""
POINTER = """const [type, x, y] = arguments;
const e = new PointerEvent(type, {bubbles: true, button: 0, pointerId: 1, clientX: x, clientY: y});
if (type === 'pointerdown') document.elementFromPoint(x, y).dispatchEvent(e); else window.dispatchEvent(e);"""
HANDLE = """const b = document.querySelectorAll(arguments[0])[arguments[1]].getBoundingClientRect();
return [b.left + b.width / 2, b.top + b.height / 2];"""
CROP = """return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips)
  .find((c) => c.id === arguments[0]).content.transform.crop;"""
SCALE = """return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips)
  .find((c) => c.id === arguments[0]).content.transform.scale;"""
GUIDES = """return [...document.querySelectorAll('[data-testid=layer-overlay] [data-guide]')]
  .map((g) => [g.dataset.guide, g.dataset.guide === 'x' ? g.offsetLeft : g.offsetTop]);"""


@flow('snap', 'Moving, scaling and cropping a layer in the preview, and moving a caption, snap to the edges and middle of '
      'the frame with a guide on the line; away from a line the layer goes where it is put')
def snap(r):
    Image.new('RGB', (1280, 720), (230, 30, 30)).save(r.work / 'red.png')
    r.import_media(FIXTURES / 'talk.mp4', r.work / 'red.png')
    r.add_clip('talk.mp4')
    r.s.call(PIP, 'red.png')
    clip = wait(lambda: next((c for t in r.state()['tracks'][1:] for c in t['clips'] if t['kind'] == 'video'), None), 10)
    if not r.check('a red picture goes over the talking head', clip):
        return
    pip = clip['id']
    canvas = r.s.run('return window.__nuzky.store.getState().snap.project.canvas')
    w, h = canvas['width'], canvas['height']
    r.s.call(PLACE, pip, -0.25, -0.2, 0.3)
    r.seek(AT_US)

    def bounds(clip_id):
        found = r.s.call(f'window.__nuzky.api.layerBounds({AT_US}).then((b) => JSON.stringify(b))')
        return next((b for b in json.loads(found['value']) if b['clipId'] == clip_id), None) if found['ok'] else None

    def box(clip_id):
        """Left, top, right and bottom of the layer's box in canvas pixels."""
        b = bounds(clip_id)
        if not b:
            return None
        xs, ys = [p[0] for p in b['corners']], [p[1] for p in b['corners']]
        return min(xs), min(ys), max(xs), max(ys)

    placed = wait(lambda: (b := box(pip)) and abs(b[0] - 0.1 * w) < 1 and b, 5)
    if not r.check('the picture sits away from every line of the frame', placed, box(pip)):
        return
    r.s.run('window.__nuzky.store.getState().select(arguments[0])', [pip])
    wait(lambda: r.s.run("return !!document.querySelector('[data-handle=rotate]')"), 5)
    frame = r.s.run(FRAME)
    k = frame['width'] / w
    screen = lambda x, y: (frame['left'] + x * k, frame['top'] + y * k)

    def drag(start, to, shot):
        """Presses at `start`, moves to `to` (window pixels), shoots the preview with the guides showing and releases.
        Returns the guides as frame lines in canvas pixels."""
        r.s.run(POINTER, 'pointerdown', *start)
        clock.sleep(0.4)  # A press on the picture asks the engine where its layers are before the drag starts.
        r.s.run(POINTER, 'pointermove', *to)
        shown = wait(lambda: r.s.run(GUIDES), 2) or []
        clock.sleep(0.6)  # The engine draws the layer where it moved.
        r.shot(shot)
        r.s.run(POINTER, 'pointerup', *to)
        return [(axis, at / k) for axis, at in shown]

    def on(guides, axis, line):
        return any(a == axis and abs(at - line) <= 2 / k for a, at in guides)

    def settles(clip_id, ok):
        return wait(lambda: (b := box(clip_id)) and ok(b) and b, 5)

    # Moving: the left edge dragged 3 px short of the frame's left edge lands on it.
    left, top, right, bottom = placed
    middle = ((left + right) / 2, (top + bottom) / 2)
    guides = drag(screen(*middle), screen(middle[0] - left + 3 / k, middle[1]), 'snap-edge')
    moved = settles(pip, lambda b: abs(b[0]) < 0.5)
    r.check('dragged 3 px from the left edge, the picture lands on it', moved, box(pip))
    r.check('a guide shows on the left edge while it is there', on(guides, 'x', 0), guides)

    # 20 px from the edge it stays where it is put.
    left, top, right, bottom = box(pip)
    middle = ((left + right) / 2, (top + bottom) / 2)
    guides = drag(screen(*middle), screen(middle[0] + 20 / k, middle[1]), 'snap-free')
    free = settles(pip, lambda b: abs(b[0] - 20 / k) < 1)
    r.check('dragged 20 px from the edge, the picture stays 20 px from it', free, box(pip))
    r.check('and no guide shows', guides == [], guides)

    # Its centre dragged 3 px past the middle of the frame lands on it.
    left, top, right, bottom = box(pip)
    middle = ((left + right) / 2, (top + bottom) / 2)
    guides = drag(screen(*middle), screen(w / 2 + 3 / k, middle[1]), 'snap-centre')
    centred = settles(pip, lambda b: abs((b[0] + b[2]) / 2 - w / 2) < 0.5)
    r.check('dragged 3 px past the middle, the picture centres', centred, box(pip))
    r.check('a guide shows on the middle line', on(guides, 'x', w / 2), guides)

    # A press that shakes 3 px is not a drag towards the line: the picture moves the 3 px and stays off the middle.
    guides = drag(screen(w / 2, middle[1]), screen(w / 2 + 3 / k, middle[1]), 'snap-nudge')
    nudged = settles(pip, lambda b: abs((b[0] + b[2]) / 2 - w / 2 - 3 / k) < 0.5)
    r.check('moved 3 px off the middle, the picture stays 3 px off it', nudged, box(pip))
    r.check('and no guide shows', guides == [], guides)
    r.key('z', ctrlKey=True)
    r.check('Undo centres it again', settles(pip, lambda b: abs((b[0] + b[2]) / 2 - w / 2) < 0.5), box(pip))

    # The bottom right corner dragged until the box is 3 px short of the frame's width scales it to exactly the width.
    left, top, right, bottom = box(pip)
    pivot = ((left + right) / 2, (top + bottom) / 2)
    corner = r.s.run(HANDLE, '[data-handle=scale]', 2)
    start = ((corner[0] - frame['left']) / k, (corner[1] - frame['top']) / k)
    ratio = (w - 3 / k - pivot[0]) / (right - pivot[0])
    guides = drag(corner, screen(pivot[0] + (start[0] - pivot[0]) * ratio, pivot[1] + (start[1] - pivot[1]) * ratio), 'snap-scale')
    scaled = settles(pip, lambda b: abs(b[0]) < 0.5 and abs(b[2] - w) < 0.5)
    r.check('scaled to 3 px short of the frame\'s width, the picture fills its width exactly', scaled, box(pip))
    r.check('guides show on both edges', on(guides, 'x', 0) and on(guides, 'x', w), guides)

    # The right crop bar dragged 3 px past the middle line stops on it.
    bar = r.s.run(HANDLE, '[data-handle=crop-right]', 0)
    guides = drag(bar, (frame['left'] + (w / 2 + 3 / k) * k, bar[1]), 'snap-crop')
    crop = lambda: r.s.run(CROP, pip) or {}
    cropped = settles(pip, lambda b: abs(b[2] - w / 2) < 0.5)
    r.check('cropped 3 px past the middle, the right edge stops on the middle line', cropped and abs(crop().get('right', 0) - 0.5) < 1e-4,
            {'box': box(pip), 'crop': crop()})
    r.check('a guide shows on the middle line while cropping', on(guides, 'x', w / 2), guides)

    # A picture cropped down to its right part, its left edge just short of the middle line: scaling it down keeps
    # that edge near the middle instead of pulling the whole picture to it.
    r.s.call(PLACE, pip, 0, -0.2, 0.3, {'left': 0.45, 'right': 0, 'top': 0, 'bottom': 0})
    left, top, right, bottom = settles(pip, lambda b: abs(b[2] - 0.65 * w) < 1) or box(pip)
    r.check('the cropped picture\'s left edge is just short of the middle line', 0 < w / 2 - left < 6 / k, (left, w / 2))
    pivot = (w / 2, (top + bottom) / 2)
    corner = r.s.run(HANDLE, '[data-handle=scale]', 2)
    start = ((corner[0] - frame['left']) / k, (corner[1] - frame['top']) / k)
    drag(corner, screen(pivot[0] + (start[0] - pivot[0]) * 0.8, pivot[1] + (start[1] - pivot[1]) * 0.8), 'snap-scale-cropped')
    scale = lambda: r.s.run(SCALE, pip)
    shrunk = wait(lambda: abs(scale() - 0.24) < 0.003, 5)
    r.check('scaled down 20 %, it is 20 % smaller', shrunk, scale())

    # A caption dragged with its middle 3 px under the middle of the frame lands there.
    r.s.call(CAPTION)
    caption = wait(lambda: next((c['id'] for t in r.state()['tracks'] if t['kind'] == 'text' for c in t['clips']), None), 10)
    # Captions sit in the lower part of the frame.
    caption and r.s.call(PLACE, caption, 0, 0.3, 1)
    shown = caption and wait(lambda: (b := box(caption)) and (b[1] + b[3]) / 2 > h * 0.75 and b, 5)
    if not r.check('a caption is added in the lower part of the frame', shown, caption and box(caption)):
        return
    left, top, right, bottom = shown
    middle = ((left + right) / 2, (top + bottom) / 2)
    guides = drag(screen(*middle), screen(middle[0], h / 2 + 3 / k), 'snap-caption')
    landed = settles(caption, lambda b: abs((b[1] + b[3]) / 2 - h / 2) < 0.5)
    r.check('dragged 3 px under the middle, the caption sits on the middle line', landed, box(caption))
    r.check('a guide shows across the middle', on(guides, 'y', h / 2), guides)
    r.check('no error toast', not r.errors(), r.errors())

"""A split screen and an adjustment layer, as a person makes them. Two clips selected show Split screen in the inspector:
Side by side puts the higher track's red picture in the left half and the talking head in the right, and the button in
the section's header swaps them, each a step Undo takes back. Adjustment layer in the Filters tab adds a layer from the
playhead and selects it; Saturation -100 typed into its Adjust tab turns the whole picture below it grey in the preview
and in the export, while the text above it keeps its colour."""
import json, subprocess, time

from PIL import Image

from e2e.harness import FIXTURES, export, flow, preview_crop, preview_rect, press, wait, webdriver

AT_US = 1_000_000
TAB = ("const t = [...document.querySelectorAll(arguments[0] + ' [role=tab]')].find((t) => t.textContent.trim() === arguments[1]);"
       'if (!t) return false; t.click(); return true')
CLICK = "const b = document.querySelector(arguments[0]); if (!b || b.disabled || b.getAttribute('aria-disabled') === 'true') return false; b.click(); return true"
OVERLAY = """(() => { const st = window.__nuzky.store.getState();
  const asset = st.snap.project.assets.find((a) => a.name === arguments[0]);
  return st.edit({type: 'addOverlay', assetId: asset.id, startUs: 0, durationUs: 3000000}); })()"""
TITLE = """(() => window.__nuzky.store.getState().edit({type: 'addText', startUs: 0, text: 'GREEN',
  style: {fontSize: 150, color: '#20e020', bold: true, strokeWidth: 0, strokeColor: '#000000', background: null}}))()"""


def clips(r):
    return r.s.run('return Object.fromEntries(window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips).map((c) => [c.id, c]))')


def tracks(r):
    return r.s.run('return window.__nuzky.store.getState().snap.project.tracks.map((t) => ({kind: t.kind, ids: t.clips.map((c) => c.id)}))')


def x_of(r, clip_id):
    return round(clips(r)[clip_id]['content']['transform']['x'], 3)


def colour(crop, x, y):
    """Mean colour of a small box around (x, y), in shares of the crop."""
    box = crop.crop((int((x - 0.05) * crop.width), int((y - 0.05) * crop.height), int((x + 0.05) * crop.width), int((y + 0.05) * crop.height)))
    pixels = list(box.getdata())
    return [round(sum(p[c] for p in pixels) / len(pixels)) for c in range(3)]


def chroma(rgb):
    return max(rgb) - min(rgb)


def preview_at(r, name, done, timeout=8):
    """The preview once `done(crop)` holds, so it shows the last edit."""
    end = time.time() + timeout
    while True:
        time.sleep(0.5)
        r.shot(name)
        crop = preview_crop(r.work / f'{name}.png', preview_rect(r))
        if done(crop) or time.time() > end:
            return crop


def smallest(r, name):
    """Shoots `name` at the smallest window too, then goes back to 1440 x 900."""
    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        time.sleep(1)
        r.shot(f'{name}{suffix}')


def type_value(r, label, value):
    field = f"document.querySelector('aside[aria-label=Inspector] input[aria-label=\"{label} value\"]')"
    wait(lambda: r.s.run(f'const f = {field}; if (!f) return false; f.focus(); f.select(); return true'), 5)
    time.sleep(0.3)
    for key in str(value):
        press(key)
    press('Return')


@flow('split_screen', 'Split screen from two selected clips puts each in its half and swaps them, an adjustment layer from '
      'the Filters tab greys everything below it in the preview and the export but not the text above it')
def split_screen(r):
    Image.new('RGB', (1280, 720), (230, 30, 30)).save(r.work / 'red.png')
    r.import_media(FIXTURES / 'talk.mp4', r.work / 'red.png')
    r.add_clip('talk.mp4')
    r.s.call(OVERLAY, 'red.png')
    wait(lambda: len(r.state()['tracks']) >= 2, 10)
    main, red = r.track()[0]['id'], r.state()['tracks'][1]['clips'][0]['id']
    r.seek(AT_US)
    r.s.run('window.__nuzky.store.getState().select(arguments[0])', [main, red])

    tile = '[data-split-screen] button[title="Side by side"]'
    r.check('two selected clips offer Split screen in the inspector', wait(lambda: r.s.run(f"return !!document.querySelector('{tile}')"), 5))
    smallest(r, 'split-choices')
    r.check('Side by side applies', r.s.run(CLICK, tile))
    r.check('the red picture on the higher track goes left, the talking head right',
            wait(lambda: (x_of(r, red), x_of(r, main)) == (-0.25, 0.25), 5), (x_of(r, red), x_of(r, main)))
    crop = preview_at(r, 'split', lambda c: chroma(colour(c, 0.25, 0.5)) > 120)
    left, right = colour(crop, 0.25, 0.5), colour(crop, 0.75, 0.5)
    r.check('the preview shows red in the left half only', chroma(left) > 120 and left[0] > 180 and not (right[0] > 180 and chroma(right) > 120),
            [left, right])
    r.check('Side by side shows chosen', r.s.run(f"return document.querySelector('{tile}').getAttribute('aria-pressed')") == 'true')

    swap = 'button[aria-label="Move each clip to the next part"]'
    r.check('the swap button moves each clip on', r.s.run(CLICK, swap))
    r.check('the clips swap halves', wait(lambda: (x_of(r, red), x_of(r, main)) == (0.25, -0.25), 5), (x_of(r, red), x_of(r, main)))
    r.shot('split-swapped')
    r.s.call('window.__nuzky.store.getState().undo()')
    r.check('Undo takes the swap back in one step', wait(lambda: (x_of(r, red), x_of(r, main)) == (-0.25, 0.25), 5))

    # Both clips moved into the right half are in no layout: none shows chosen and the swap button stays off.
    r.s.call("""(() => { const st = window.__nuzky.store.getState();
      const clips = st.snap.project.tracks.flatMap((t) => t.clips).filter((c) => arguments[0].includes(c.id));
      return st.edit(clips.map((c) => ({type: 'updateClip', clipId: c.id, transform: {...c.content.transform, x: 0.25, y: 0}}))); })()""", [main, red])
    wait(lambda: (x_of(r, red), x_of(r, main)) == (0.25, 0.25), 5)
    time.sleep(0.5)
    r.check('two clips in the same half are in no layout', r.s.run(f"return document.querySelector('{tile}').getAttribute('aria-pressed')") == 'false')
    off = "const b = document.querySelector('button[aria-label^=\"Move each clip to the next part\"]'); return b.disabled || b.getAttribute('aria-disabled') === 'true'"
    r.check('and the swap button is off', r.s.run(off))
    r.s.call('window.__nuzky.store.getState().undo()')
    wait(lambda: (x_of(r, red), x_of(r, main)) == (-0.25, 0.25), 5)

    r.s.call(TITLE)
    title = wait(lambda: next((c['id'] for c in clips(r).values() if c['content']['type'] == 'text'), None), 5)
    r.s.run('window.__nuzky.store.getState().select([])')
    r.seek(0)
    r.check('the Filters tab opens', wait(lambda: r.s.run(TAB, '[role=tablist][aria-label=Library]', 'Filters'), 5))
    r.check('Adjustment layer is in the Filters tab', wait(lambda: r.s.run(CLICK, '[data-add-adjustment]'), 5))
    layer = wait(lambda: next((c['id'] for c in clips(r).values() if c['content']['type'] == 'adjustment'), None), 5)
    r.check('a new adjustment layer is selected', layer and r.s.run('return window.__nuzky.store.getState().selection') == [layer])
    listed = tracks(r)
    on = next(i for i, t in enumerate(listed) if layer in t['ids'])
    above = [t['kind'] for t in listed[on + 1:]]
    r.check('it goes above the video tracks and below the text', listed[on]['kind'] == 'video' and above == ['text'], [t['kind'] for t in listed])
    r.check('the inspector shows the adjustment layer',
            wait(lambda: r.s.run("return document.querySelector('aside[aria-label=Inspector]').textContent.includes('Adjustment layer')"), 5))
    type_value(r, 'Saturation', -100)
    r.check('Saturation -100 is set on the layer', wait(lambda: clips(r)[layer]['content']['adjust']['saturation'] == -1, 5),
            clips(r)[layer]['content']['adjust'])
    r.seek(AT_US)
    crop = preview_at(r, 'adjustment', lambda c: chroma(colour(c, 0.25, 0.3)) < 20)
    grey = colour(crop, 0.25, 0.3)
    r.check('the red picture below the layer turns grey in the preview', chroma(grey) < 20, grey)
    pixels = list(crop.getdata())
    green = sum(1 for p in pixels if p[1] > 150 and p[0] < 110 and p[2] < 110)
    r.check('the text above the layer keeps its green', green > 200, green)
    smallest(r, 'adjustment-inspector')
    r.s.run('window.__nuzky.store.getState().select([])')
    time.sleep(0.5)
    r.shot('adjustment-timeline')

    out = export(r, 'graded.mp4')
    frame = r.work / 'graded-1s.png'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-ss', '1', '-i', str(out), '-frames:v', '1', str(frame)], check=True)
    image = Image.open(frame).convert('RGB')
    exported = colour(image, 0.25, 0.3)
    r.check('the export greys the red picture too', chroma(exported) < 20, exported)
    r.check('the export keeps the text green', sum(1 for p in image.getdata() if p[1] > 150 and p[0] < 110 and p[2] < 110) > 200)

    saved = json.loads(r.saved_project().read_text())
    kinds = [c['content']['type'] for t in saved['tracks'] for c in t['clips']]
    r.check('the saved project holds the layer and the split screen', 'adjustment' in kinds and title is not None, kinds)

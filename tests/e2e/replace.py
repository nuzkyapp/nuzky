"""Replace media. A red clip made smaller and moved right is duplicated, and blue media dragged from the media panel onto
the middle of the copy puts blue in the copy: the copy keeps its id, place, length, size and position, the original
stays red, the preview shows blue where the copy is, the saved project has it and one undo brings red back. Media too
short for the clip changes nothing and says why, and media dropped at a clip's end goes in beside it."""
import json, subprocess, time as clock

from e2e.harness import flow, preview_crop, preview_rect, wait

PROJECT = 'return window.__nuzky.store.getState().snap.project'
TILE = """return (() => { const t = [...document.querySelectorAll('[title]')].find((e) => e.title.startsWith(arguments[0] + '\\n'));
  if (!t) return null; const b = t.getBoundingClientRect(); return [b.left + b.width / 2, b.top + b.height / 2]; })()"""
PRESS = """const [x, y, path] = arguments;
const at = (type, x, y) => new PointerEvent(type, {bubbles: true, button: 0, pointerId: 1, clientX: x, clientY: y});
document.elementFromPoint(x, y).dispatchEvent(at('pointerdown', x, y));
for (const [px, py] of path) window.dispatchEvent(at('pointermove', px, py));"""
RELEASE = """window.dispatchEvent(new PointerEvent('pointerup', {bubbles: true, button: 0, pointerId: 1,
  clientX: arguments[0], clientY: arguments[1]}))"""
RECT = "const b = document.querySelector(`[data-clip-id='${arguments[0]}']`).getBoundingClientRect(); return [b.left, b.top, b.right, b.bottom];"
CHIP = "return document.querySelector(`[data-clip-id='${arguments[0]}']`)?.textContent.includes('Replace') ?? false"
LINE = "return !!document.querySelector('[data-testid=drop-line]')"
TRANSFORM = {'x': 0.25, 'y': 0.0, 'scale': 0.5, 'rotation': 0.0, 'opacity': 1.0}


def main(r):
    return next(t['clips'] for t in r.s.run(PROJECT, retries=3)['tracks'] if t['id'] == 'main')


def drag(r, name, to, check=None):
    """Drags a media tile to `to`, runs `check` while it is over it, then drops it."""
    x, y = r.s.run(TILE, name)
    r.s.run(PRESS, x, y, [(x + 10, y + 10), to])
    result = check() if check else None
    r.s.run(RELEASE, *to)
    return result


@flow('replace', 'Blue media dropped on the middle of a styled red copy puts blue in the copy and keeps its place, length '
      'and transform, in the preview and the saved file, undone in one step; too short media is refused with the reason; '
      'a drop at the end of a clip inserts beside it')
def replace_flow(r):
    for name, colour, seconds in [('red', 'red', 4), ('blue', 'blue', 6), ('green', 'green', 1)]:
        subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', f'color=c={colour}:s=640x360:r=30:d={seconds}',
                        '-f', 'lavfi', '-i', f'sine=frequency=440:duration={seconds}', '-shortest', '-c:v', 'libx264',
                        '-pix_fmt', 'yuv420p', '-c:a', 'aac', str(r.work / f'{name}.mp4')], check=True)
    r.import_media(*(r.work / f'{n}.mp4' for n in ('red', 'blue', 'green')))
    r.add_clip('red.mp4')
    original = main(r)[0]['id']
    r.s.call("window.__nuzky.store.getState().edit({type: 'updateClip', clipId: arguments[0], transform: arguments[1]})", original, TRANSFORM)
    r.s.call("window.__nuzky.store.getState().edit({type: 'duplicateClip', clipId: arguments[0]})", original)
    if not r.check('the styled red clip and its copy are on the main track', wait(lambda: len(main(r)) == 2, 10), main(r)):
        return
    copy = main(r)[1]
    assets = {a['name']: a['id'] for a in r.s.run(PROJECT)['assets']}
    r.s.run('window.__nuzky.store.getState().setZoom(60); window.__nuzky.store.getState().select([])')
    r.s.run("document.querySelector('[data-tab=\"media\"]')?.click()")
    clock.sleep(0.5)

    left, top, right, bottom = r.s.run(RECT, copy['id'])
    middle = ((left + right) / 2, (top + bottom) / 2)

    def over_middle():
        ringed = wait(lambda: r.s.run(CHIP, copy['id']), 5)
        r.shot('replace-target')
        return ringed, r.s.run(LINE)

    ringed, line = drag(r, 'blue.mp4', middle, over_middle)
    r.check('over the middle of the copy, the copy shows Replace and no drop line', ringed and not line, [ringed, line])
    replaced = wait(lambda: main(r)[1]['content']['assetId'] == assets['blue.mp4'], 10)
    clips = main(r)
    r.check('dropped there, the copy plays the blue media', replaced, clips[1]['content']['assetId'])
    after = clips[1]
    r.check('the copy keeps its id, place, length and transform',
            [after['id'], after['startUs'], after['durationUs'], after['content']['transform']]
            == [copy['id'], copy['startUs'], copy['durationUs'], copy['content']['transform']], after)
    r.check('and starts at the start of the blue media', after['content']['sourceInUs'] == 0, after['content']['sourceInUs'])
    r.check('the original stays red and nothing was added',
            len(clips) == 2 and clips[0]['content']['assetId'] == assets['red.mp4'], clips)
    r.check('the copy is selected', r.s.run('return window.__nuzky.store.getState().selection') == [copy['id']])

    def colour_at(name):
        """The colour of the preview where the transform puts the clip: right of the middle."""
        clock.sleep(1)
        r.shot(name)
        crop = preview_crop(r.work / f'{name}.png', preview_rect(r))
        red, _, blue = crop.getpixel((int(0.75 * crop.width), int(0.5 * crop.height)))[:3]
        left_red, _, left_blue = crop.getpixel((int(0.1 * crop.width), int(0.5 * crop.height)))[:3]
        return ('blue' if blue > red + 60 else 'red' if red > blue + 60 else 'none'), max(left_red, left_blue) < 60

    r.s.run('window.__nuzky.store.getState().select([])')
    r.seek(copy['startUs'] + copy['durationUs'] // 2)
    shown = wait(lambda: colour_at('replaced') == ('blue', True), 10)
    r.check('the preview shows blue, small and on the right, where the copy plays', shown, colour_at('replaced'))
    r.seek(1_000_000)
    r.check('the original still shows red there', wait(lambda: colour_at('original')[0] == 'red', 10), colour_at('original'))
    saved = lambda: [c['content']['assetId'] for c in json.loads(r.saved_project().read_text())['tracks'][0]['clips']]
    r.check('the saved project has blue in the copy', wait(lambda: saved() == [assets['red.mp4'], assets['blue.mp4']], 10), saved())

    r.key('z', ctrlKey=True)
    r.check('one undo brings the red media back with everything else as it was', wait(lambda: main(r)[1] == copy, 10), main(r)[1])
    r.check('nothing in the app failed', not r.errors(), r.errors())

    before = r.s.run(PROJECT)
    drag(r, 'green.mp4', middle)
    refused = wait(lambda: any(t.startswith('This media is 1.0 s and the clip needs 4.0 s') for t in r.errors()), 5)
    r.check('a 1 s file dropped on a 4 s clip is refused with both lengths', refused, r.errors())
    r.check('and changes nothing', r.s.run(PROJECT) == before)
    r.shot('too-short')

    end = (left + 6, middle[1])
    ringed, line = drag(r, 'blue.mp4', end, lambda: (r.s.run(CHIP, copy['id']), wait(lambda: r.s.run(LINE), 5)))
    r.check('at the end of a clip the drop line shows and nothing is ringed', line and not ringed, [ringed, line])
    r.check('dropped there, the media goes in beside the clip', wait(lambda: len(main(r)) == 3, 10), main(r))
    r.check('and the copy keeps the red media', main(r)[2]['content']['assetId'] == assets['red.mp4'] and main(r)[2]['id'] == copy['id'],
            main(r))

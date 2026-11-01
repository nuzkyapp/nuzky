"""Playing a media item in the preview without putting it on the timeline. A click on its tile shows it in the preview
at its own shape with its own Play, time and slider; the project and the timeline's playhead stay as they were, and
Esc or moving the playhead brings the timeline back."""
import subprocess, time

from PIL import ImageStat

from e2e.harness import CLI, flow, preview_crop, preview_rect, wait, webdriver

AT_US = 1_000_000
TILE = "document.querySelector('[aria-label=\"Add clip.mp4 at playhead\"]').closest('.group').firstElementChild"
SOURCE = "return window.__nuzky.source.getState()"
CHIP = "return document.querySelector('section[aria-label=Preview] [aria-label^=\"Back to the timeline\"]')?.parentElement.textContent ?? null"
SHOWN = "return document.querySelector('section[aria-label=Preview] canvas[aria-label=\"Preview of clip.mp4\"]')?.dataset.us ?? null"


def setup(r):
    # The timeline: red and upright. The media item: blue for 2 s, then green, wide, with sound.
    talk, clip = r.work / 'talk.mp4', r.work / 'clip.mp4'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'color=c=red:s=540x960:r=30:d=4', '-c:v', 'libx264',
                    '-pix_fmt', 'yuv420p', str(talk)], check=True)
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'color=c=blue:s=1280x720:r=30:d=4', '-f', 'lavfi',
                    '-i', 'sine=f=440:d=4', '-vf', "drawbox=c=green:t=fill:enable='gte(t,2)'", '-shortest',
                    '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-c:a', 'aac', str(clip)], check=True)
    project = r.work / 'data/nuzky/projects/media-preview.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(talk)], env=r.env, check=True, capture_output=True)


def colour(r, name):
    """Average colour of the middle of the preview, as on screen."""
    r.shot(name)
    crop = preview_crop(r.work / f'{name}.png', preview_rect(r))
    w, h = crop.size
    return [round(v) for v in ImageStat.Stat(crop.crop((w // 2 - 20, h // 2 - 20, w // 2 + 20, h // 2 + 20))).mean]


def is_(rgb, channel):
    return rgb[channel] > 90 and all(v < 50 for i, v in enumerate(rgb) if i != channel)


@flow('media_preview', 'A click on a media tile plays it in the preview at its own shape without adding it to the '
      'timeline; Space, the arrows and the slider move through it, and Esc or moving the playhead shows the timeline again',
      before=setup)
def media_preview(r):
    r.import_media(r.work / 'clip.mp4')
    before = r.state()['tracks']
    r.seek(AT_US)
    r.s.run('window.__nuzky.store.getState().select([])')
    drawn = lambda: r.s.run("return document.querySelector('section[aria-label=Preview] canvas').dataset.us")
    if not r.check('the preview shows the timeline at the playhead', wait(lambda: drawn() == str(AT_US), 10), drawn()):
        return
    time.sleep(0.5)
    r.check('the timeline is red', is_(colour(r, 'timeline'), 0), colour(r, 'timeline'))

    r.s.run(f'{TILE}.click()')
    if not r.check('a click on the tile shows the media item in the preview', wait(lambda: r.s.run(SHOWN) == '0', 15), r.s.run(SOURCE)):
        return
    time.sleep(0.5)
    r.check('a chip names it, with the way back', 'clip.mp4' in (r.s.run(CHIP) or ''), r.s.run(CHIP))
    rect = preview_rect(r)
    r.check('the preview takes the item\'s own wide shape, not the upright timeline\'s',
            abs(rect['width'] / rect['height'] - 16 / 9) < 0.02, rect)
    r.check('it shows the item\'s first frame', is_(colour(r, 'item-start'), 2), colour(r, 'item-start'))
    r.check('the tile shows which item is in the preview',
            'outline-accent' in r.s.run(f'return {TILE}.className'), r.s.run(f'return {TILE}.className'))
    r.check('nothing was added to the project', r.state()['tracks'] == before, r.state()['tracks'])

    r.key(' ')
    played = wait(lambda: r.s.run(SOURCE)['timeUs'] > 500_000, 8)
    r.key(' ')
    r.check('Space plays the item, the slider and time following it, and pauses it',
            played and wait(lambda: r.s.run(SOURCE)['playing'] is False, 5),
            r.s.run(SOURCE))
    r.check('the timeline\'s playhead stays where it was', r.s.run('return window.__nuzky.store.getState().timeUs') == AT_US)

    r.key('End')
    r.check('End goes to the item\'s end', wait(lambda: r.s.run(SHOWN) not in (None, '0') and r.s.run(SOURCE)['timeUs'] == 4_000_000, 8),
            r.s.run(SOURCE))
    time.sleep(0.5)
    r.check('the item\'s later picture shows', is_(colour(r, 'item-end'), 1), colour(r, 'item-end'))
    r.s.run("const s = document.querySelector('section[aria-label=Preview] input[type=range]');"
            "Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(s, 500000);"
            "s.dispatchEvent(new Event('input', {bubbles: true}))")
    r.check('the slider seeks in the item', wait(lambda: r.s.run(SHOWN) == '500000', 8), r.s.run(SHOWN))
    time.sleep(0.5)
    r.check('and the picture follows it', is_(colour(r, 'item-slider'), 2), colour(r, 'item-slider'))

    for width, height, name in ((1440, 900, 'item-1440'), (1024, 640, 'item-1024')):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        time.sleep(0.6)
        r.shot(name)
        fits = r.s.run("const bar = document.querySelector('section[aria-label=Preview] .bar').getBoundingClientRect();"
                       "return [...document.querySelectorAll('section[aria-label=Preview] .bar > *')]"
                       ".every((e) => { const b = e.getBoundingClientRect(); return b.width > 0 && b.left >= bar.left && b.right <= bar.right; })")
        r.check(f'its controls fit the bar in a {width} × {height} window', fits)

    r.key('Escape')
    r.check('Esc shows the timeline again', wait(lambda: r.s.run(CHIP) is None, 5) and is_(colour(r, 'back'), 0), colour(r, 'back'))
    r.check('the timeline is where it was', r.s.run('return window.__nuzky.store.getState().timeUs') == AT_US)

    r.s.run(f'{TILE}.click()')
    wait(lambda: r.s.run(SHOWN) == '0', 15)
    r.focus_clip(r.track()[0]['id'])
    r.check('choosing a clip on the timeline shows the timeline again, so its look is what the preview shows',
            wait(lambda: r.s.run(CHIP) is None, 5))
    r.s.run('window.__nuzky.store.getState().select([])')

    r.s.run(f'{TILE}.click()')
    wait(lambda: r.s.run(SHOWN) == '0', 15)
    r.seek(2_000_000)
    r.check('moving the playhead shows the timeline again', wait(lambda: r.s.run(CHIP) is None, 5)
            and wait(lambda: drawn() == '2000000', 8), drawn())
    r.check('the project is as it was', r.state()['tracks'] == before, r.state()['tracks'])
    r.check('no error toast', not r.errors(), r.errors())

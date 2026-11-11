"""Nuzky's own looks in the editor: the newer transitions from the Transitions tab, an effect of Nuzky's own from the
Effects menu without any plugin, and a keyframe's curve from Transform. Each one changes the preview and lands in the
saved project, and the export finishes with them."""
import json
import time

from e2e.harness import FIXTURES, export, flow, preview_crop, preview_rect, wait, webdriver

INSPECTOR = 'aside[aria-label=Inspector]'
CLIPS = 'return window.__nuzky.store.getState().snap.project.tracks[0].clips'
LIBRARY = 'aside[data-tour=library]'
TILE = f"""const b = [...document.querySelectorAll('{LIBRARY} button[aria-pressed]')].find((b) => b.textContent.trim() === arguments[0]);
if (!b) return false; b.click(); return true;"""
TILES = f"return [...document.querySelectorAll('{LIBRARY} button[aria-pressed]')].map((b) => b.textContent.trim())"
TAB = (f"const t = [...document.querySelectorAll('{INSPECTOR} [role=tab]')]"
       ".find((t) => t.textContent.trim() === arguments[0]); if (!t) return false; t.click(); return true")
ADD = f"document.querySelector('{INSPECTOR} button[aria-label^=\"Add effect\"]')"
MENU_ITEM = "[...document.querySelectorAll('[role=menuitem]')].find((b) => b.textContent.trim() === arguments[0])"
CURVE = f"document.querySelector('{INSPECTOR} [role=group][aria-label=\"Curve to next keyframe\"]')"
CURVE_STATE = f"""const g = {CURVE}; if (!g) return null;
return {{pressed: g.querySelector('button[aria-pressed=true]')?.textContent.trim() ?? null,
  disabled: [...g.querySelectorAll('button')].every((b) => b.disabled || b.getAttribute('aria-disabled') === 'true')}};"""
CURVE_CLICK = f"""const b = [...{CURVE}.querySelectorAll('button')].find((b) => b.textContent.trim() === arguments[0]);
if (!b) return false; b.click(); return true;"""


def clips(r):
    return r.s.run(CLIPS, retries=3)


def picture(r, name):
    """The preview as it is now, once it has drawn the last change: two screenshots alike half a second apart."""
    from PIL import ImageChops, ImageStat
    rect = preview_rect(r)
    last = None
    for attempt in range(12):
        r.shot(name)
        crop = preview_crop(r.work / f'{name}.png', rect).convert('L')
        if last is not None and ImageStat.Stat(ImageChops.difference(crop, last)).mean[0] < 0.5:
            return crop
        last = crop
        time.sleep(0.5)
    return last


def redrawn(r, name, before, enough=4, timeout=8):
    """The preview once it differs from `before`, after the last change; as it is at `timeout` otherwise."""
    rect = preview_rect(r)
    end = time.time() + timeout
    while True:
        r.shot(name)
        crop = preview_crop(r.work / f'{name}.png', rect).convert('L')
        if difference(crop, before) > enough or time.time() > end:
            return crop
        time.sleep(0.5)


def difference(a, b):
    from PIL import ImageChops, ImageStat
    return ImageStat.Stat(ImageChops.difference(a, b)).mean[0]


def size(r, width, height):
    webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
    wait(lambda: r.s.run('return window.innerWidth') == width, 5)


def both_sizes(r, name, selector):
    """`selector` in view at the smallest window, 1024 x 640, then at 1440 x 900, where the flow goes on."""
    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        size(r, width, height)
        r.s.run(f"document.querySelector(arguments[0])?.scrollIntoView({{block: 'center'}})", selector)
        time.sleep(1)
        r.shot(name + suffix)


def tab(r, clip_id, name):
    r.focus_clip(clip_id)
    return wait(lambda: r.s.run(TAB, name), 5)


@flow('looks', 'The Transitions tab offers the newer transitions and Spin draws differently from Dissolve; the '
      'Effects menu lists Nuzky\'s own effects without any plugin and Pixelate changes the preview; with two keyframes '
      'Curve to next keyframe sets Ease out, which moves the picture elsewhere than Linear and is disabled past the last '
      'keyframe; the saved project keeps all three and the export finishes')
def looks(r):
    r.import_media(FIXTURES / 'talk.mp4', FIXTURES / 'wide.mp4')
    for name in ('talk.mp4', 'wide.mp4'):
        r.add_clip(name)
    first, second = clips(r)

    # Transitions.
    r.s.run("window.__nuzky.store.setState({panelTab: 'transitions'})")
    r.s.run('window.__nuzky.store.getState().selectCut(arguments[0])', second['id'])
    wanted = ['Slide right', 'Slide down', 'Wipe right', 'Wipe up', 'Wipe down', 'Zoom out', 'Spin']
    r.check('the Transitions tab offers the newer transitions', wait(lambda: all(t in (r.s.run(TILES) or []) for t in wanted), 5),
            r.s.run(TILES))
    both_sizes(r, 'transitions-tab', f'{LIBRARY} button[aria-pressed]:last-of-type')
    r.check('clicking Spin puts it on the cut', r.s.run(TILE, 'Spin')
            and wait(lambda: clips(r)[1].get('transitionIn') == {'kind': 'spin', 'durationUs': 500_000}, 5), clips(r)[1].get('transitionIn'))
    r.s.run(TILE, 'Dissolve')
    wait(lambda: clips(r)[1]['transitionIn']['kind'] == 'dissolve', 5)
    r.seek(second['startUs'] + 50_000)
    time.sleep(1)
    dissolve = picture(r, 'dissolve-midway')
    r.s.run(TILE, 'Spin')
    wait(lambda: clips(r)[1]['transitionIn']['kind'] == 'spin', 5)
    spin = redrawn(r, 'spin-midway', dissolve)
    r.check('midway Spin draws another picture than Dissolve', difference(spin, dissolve) > 4, difference(spin, dissolve))

    # Nuzky's own effect on the first clip, with no plugin installed.
    r.seek(first['durationUs'] // 3)
    r.check('the inspector opens the Adjust tab', tab(r, first['id'], 'Adjust'))
    before = picture(r, 'before-effect')
    r.check('the Effects + is there without any plugin', wait(lambda: r.s.run(f'return !!{ADD} && !{ADD}.getAttribute("aria-disabled")'), 10))
    r.s.run(f'{ADD}.click()')
    r.check('its menu lists Pixelate', wait(lambda: r.s.run(f'return !!{MENU_ITEM}', 'Pixelate'), 5))
    time.sleep(0.3)
    r.shot('effects-menu')
    r.s.run(f'{MENU_ITEM}.click()', 'Pixelate')
    added = wait(lambda: clips(r)[0].get('effects'), 5)
    r.check("the clip gets Nuzky's Pixelate with its default size", added == [
        {'plugin': 'nuzky', 'effect': 'pixelate', 'name': 'Pixelate', 'version': '1', 'params': {'size': 16}}], added)
    # On the test pattern's wide bars Pixelate shows on their edges, the diagonal line and the checks: 2 levels.
    pixelated = redrawn(r, 'pixelate', before, enough=1)
    r.check('the preview draws Pixelate', difference(pixelated, before) > 1, difference(pixelated, before))
    both_sizes(r, 'pixelate-section', f"{INSPECTOR} [data-effect='nuzky/pixelate']")
    r.check('no toast says an effect is missing', not [t for t in r.s.run(
        "return window.__nuzky.store.getState().toasts.map((t) => t.text)") if 'Pixelate' in t])

    # A keyframe's curve on the second clip: scale 0.5 at its start, 1.2 and to the right 0.8 s later.
    start, end = 0, 800_000
    small = {'x': -0.2, 'y': 0, 'scale': 0.5, 'rotation': 0, 'opacity': 1}
    large = {'x': 0.2, 'y': 0, 'scale': 1.2, 'rotation': 0, 'opacity': 1}
    r.s.call("window.__nuzky.store.getState().edit({type: 'setKeyframes', clipId: arguments[0], keyframes: arguments[1]})",
             second['id'], [{'tUs': start, 'transform': small, 'ease': 'linear'}, {'tUs': end, 'transform': large, 'ease': 'linear'}])
    wait(lambda: len(clips(r)[1]['keyframes']) == 2, 5)
    r.check('the inspector opens the Video tab', tab(r, second['id'], 'Video'))
    r.seek(second["startUs"] + 1_200_000)
    r.check('past the last keyframe the curve is disabled', wait(lambda: (c := r.s.run(CURVE_STATE)) and c['disabled'], 5), r.s.run(CURVE_STATE))
    r.seek(second['startUs'] + 400_000)
    r.check('between the two keyframes Curve to next keyframe shows Linear',
            wait(lambda: r.s.run(CURVE_STATE) == {'pressed': 'Linear', 'disabled': False}, 5), r.s.run(CURVE_STATE))
    linear = picture(r, 'curve-linear')
    r.check('clicking Ease out sets the curve of the first keyframe', r.s.run(CURVE_CLICK, 'Ease out')
            and wait(lambda: [k['ease'] for k in clips(r)[1]['keyframes']] == ['easeOut', 'linear'], 5),
            [k['ease'] for k in clips(r)[1]['keyframes']])
    eased = redrawn(r, 'curve-ease-out', linear)
    r.check('Ease out moves the picture elsewhere than Linear at the same time', difference(eased, linear) > 4, difference(eased, linear))
    r.check('and Ease out shows as chosen', r.s.run(CURVE_STATE) == {'pressed': 'Ease out', 'disabled': False}, r.s.run(CURVE_STATE))
    both_sizes(r, 'curve-section', f'{INSPECTOR} [role=group][aria-label="Curve to next keyframe"]')

    saved = lambda: json.loads(r.saved_project().read_text())['tracks'][0]['clips']
    kept = wait(lambda: (c := saved()) and c[1].get('transitionIn', {}).get('kind') == 'spin'
                and c[0].get('effects', [{}])[0].get('effect') == 'pixelate' and c[1]['keyframes'][0]['ease'] == 'easeOut' and c, 10)
    r.check('the saved project keeps the transition, the effect and the curve', kept, saved())
    out = export(r, 'looks.mp4')
    r.check('the export is there', out.exists() and out.stat().st_size > 0)
    r.check('no error toast', not r.errors(), r.errors())

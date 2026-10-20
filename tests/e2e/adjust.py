import json, subprocess, time

from e2e.harness import CLI, ROOT, flow, preview_crop, preview_rect, press, wait, webdriver

# What adjustments must not spoil, in a 2 x 2 grid: black hair, a white wall, mid grey and lit skin.
PATCHES = {'black': (0, 0, 0), 'white': (255, 255, 255), 'grey': (128, 128, 128), 'skin': (242, 184, 148)}
WIDTH, HEIGHT = 360, 640
PHOTO = ROOT / 'site/public/media/ugc.jpg'
PUNCH = {'contrast': 0.5, 'saturation': 0.1, 'vignette': 0.25}
ADJUST = "return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips).find((c) => c.id === arguments[0]).content.adjust"
TAB = ("const t = [...document.querySelectorAll(arguments[0] + ' [role=tab]')].find((t) => t.textContent.trim() === arguments[1]);"
       'if (!t) return false; t.click(); return true')


def centre(crop, index):
    """Mean colour of the middle of grid cell `index` (row by row) in a crop of the picture."""
    column, row = index % 2, index // 2
    w, h = crop.width / 2, crop.height / 2
    box = crop.crop((int((column + 0.3) * w), int((row + 0.3) * h), int((column + 0.7) * w), int((row + 0.7) * h)))
    pixels = list(box.getdata())
    return [round(sum(p[c] for p in pixels) / len(pixels)) for c in range(3)]


def patches(crop):
    return {name: centre(crop, i) for i, name in enumerate(PATCHES)}


def shown(r, name, rect, done):
    """The patches in the preview once `done(patches)` holds, so the picture follows the last edit."""
    end = time.time() + 6
    while True:
        r.shot(name)
        found = patches(preview_crop(r.work / f'{name}.png', rect))
        if done(found) or time.time() > end:
            return found
        time.sleep(0.5)


def warm_hue(rgb):
    """Hue in degrees of a colour whose red is highest and blue lowest, such as skin."""
    r, g, b = rgb
    return 60 * (g - b) / max(r - b, 1)


def type_value(r, label, value):
    """Types into an Adjust row's number field and confirms with Enter, as a person does."""
    r.s.run(f"document.querySelector('aside[aria-label=Inspector] input[aria-label=\"{label} value\"]').focus()")
    r.s.run(f"document.querySelector('aside[aria-label=Inspector] input[aria-label=\"{label} value\"]').select()")
    for key in str(value):
        press(key)
    typed = lambda: r.s.run(f"return document.querySelector('aside[aria-label=Inspector] input[aria-label=\"{label} value\"]').value")
    r.check(f'{label} holds the typed text before Enter', wait(lambda: typed() == str(value), 3), typed())
    press('Return')


def saved_adjust(r):
    clips = [c for t in json.loads(r.saved_project().read_text())['tracks'] for c in t['clips']]
    return clips[0]['content']['adjust'] if clips else None


@flow('adjust', 'Brightness and Contrast typed into the inspector keep black black, white white and skin its hue in the '
      'preview, as the renderer draws them, and a filter from the Filters tab applies its values')
def adjust(r):
    from PIL import Image
    image = Image.new('RGB', (WIDTH, HEIGHT))
    for i, colour in enumerate(PATCHES.values()):
        column, row = i % 2, i // 2
        image.paste(colour, (column * WIDTH // 2, row * HEIGHT // 2, (column + 1) * WIDTH // 2, (row + 1) * HEIGHT // 2))
    photo = r.work / 'patches.png'
    image.save(photo)
    r.import_media(photo)
    r.add_clip(photo.name)
    clip = r.track()[0]
    middle = clip['startUs'] + clip['durationUs'] // 2
    r.seek(middle)
    r.focus_clip(clip['id'])
    wait(lambda: r.s.run(TAB, 'aside[aria-label=Inspector]', 'Adjust'), 5)
    time.sleep(1.5)
    rect = preview_rect(r)
    before = shown(r, 'original', rect, lambda p: True)
    value = lambda key: r.s.run(ADJUST, clip['id'])[key]

    type_value(r, 'Brightness', 50)
    r.check('typing 50 into Brightness sets it on the clip', wait(lambda: value('brightness') == 0.5, 5), value('brightness'))
    bright = shown(r, 'brightness', rect, lambda p: p['grey'][1] - before['grey'][1] >= 10)
    r.check('Brightness +50 lifts mid grey', bright['grey'][1] - before['grey'][1] >= 25, [before['grey'], bright['grey']])
    # Brightness added 0.2 to every channel: black showed as 51 grey, a grey veil over the whole picture.
    r.check('Brightness +50 keeps black black in the preview', max(bright['black']) <= 4, bright['black'])
    r.check('Brightness +50 keeps white white', min(bright['white']) >= 250, bright['white'])

    type_value(r, 'Brightness', 0)
    wait(lambda: value('brightness') == 0, 5)
    shown(r, 'reset', rect, lambda p: abs(p['grey'][1] - before['grey'][1]) <= 3)
    type_value(r, 'Contrast', 50)
    r.check('typing 50 into Contrast sets it on the clip', wait(lambda: value('contrast') == 0.5, 5), value('contrast'))
    contrast = shown(r, 'contrast', rect, lambda p: max(abs(a - b) for a, b in zip(p['skin'], before['skin'])) >= 5)
    # Contrast stretched each channel apart and clipped red alone: lit skin turned 8 degrees towards yellow.
    moved = abs(warm_hue(contrast['skin']) - warm_hue(before['skin']))
    r.check('Contrast +50 changes lit skin but keeps its hue', contrast['skin'] != before['skin'] and moved <= 2,
            f"{before['skin']} became {contrast['skin']}, {moved:.1f} degrees")
    r.check('Contrast +50 keeps black and white as they are', max(contrast['black']) <= 4 and min(contrast['white']) >= 250,
            [contrast['black'], contrast['white']])
    r.check('Contrast +50 turns around mid grey, which stays', abs(contrast['grey'][1] - before['grey'][1]) <= 3,
            [before['grey'], contrast['grey']])

    saved = wait(lambda: (a := saved_adjust(r)) and a['contrast'] == 0.5 and a['brightness'] == 0 and a, 10)
    r.check('the project saves the adjustment', saved, saved_adjust(r))
    rendered_png = r.work / 'rendered.png'
    subprocess.run([str(CLI), 'frame', str(r.saved_project()), str(middle / 1e6), str(rendered_png), str(WIDTH)],
                   env=r.env, check=True, capture_output=True)
    rendered = patches(Image.open(rendered_png).convert('RGB'))
    off = max(abs(a - b) for name in PATCHES for a, b in zip(contrast[name], rendered[name]))
    r.check('the preview shows the same colours as the renderer the export uses', off <= 8, f'{off} levels apart')
    r.shot('adjust-inspector')

    r.import_media(PHOTO)
    r.add_clip(PHOTO.name)
    photo_clip = r.track()[1]
    r.seek(photo_clip['startUs'] + photo_clip['durationUs'] // 2)
    r.focus_clip(photo_clip['id'])
    wait(lambda: r.s.run(TAB, '[role=tablist][aria-label=Library]', 'Filters'), 5)
    punch = "document.querySelector('button[title=\"Apply Punch\"]')"
    wait(lambda: r.s.run(f'return !!{punch}'), 5)
    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        time.sleep(1)
        r.shot(f'filters{suffix}')
    r.s.run(f'{punch}.click()')
    punched = lambda: all(abs(v - PUNCH.get(k, 0)) < 1e-6 for k, v in r.s.run(ADJUST, photo_clip['id']).items())
    r.check('picking Punch puts its values on the clip', wait(punched, 5), r.s.run(ADJUST, photo_clip['id']))
    r.check('and marks its tile', wait(lambda: r.s.run(f"return {punch}.getAttribute('aria-pressed') === 'true'"), 5))
    time.sleep(1)
    r.shot('filters-punch')
    r.check('nothing shows an error', not r.errors(), r.errors())

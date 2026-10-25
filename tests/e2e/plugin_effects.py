import json
import shutil
import time

from e2e.harness import ROOT, changed_share, export, flow, preview_crop, preview_rect, wait, webdriver

EXAMPLE = ROOT / 'skills/nuzky-plugin/examples/vhs'
EFFECTS = ("return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips)"
           ".find((c) => c.id === arguments[0]).effects ?? []")
TAB = ("const t = [...document.querySelectorAll('aside[aria-label=Inspector] [role=tab]')]"
       ".find((t) => t.textContent.trim() === arguments[0]); if (!t) return false; t.click(); return true")
ROW = "document.querySelector('aside[aria-label=Inspector] [data-effect=\"vhs/vhs\"]')"


def prepare(r):
    shutil.copytree(EXAMPLE, r.work / 'data/nuzky/plugins/vhs')


def phone_photo(r):
    """A grey photo stored on its side with EXIF orientation 6, as phones save portrait shots."""
    from PIL import Image
    image = Image.new('RGB', (400, 200), (150, 150, 150))
    exif = Image.Exif()
    exif[0x0112] = 6
    path = r.work / 'portrait.jpg'
    image.save(path, exif=exif.tobytes(), quality=95)
    return path


def picture(crop):
    """The middle of the clip in a preview crop, clear of its edges."""
    w, h = crop.size
    return crop.crop((int(w * 0.3), int(h * 0.3), int(w * 0.7), int(h * 0.7))).convert('L')


def stripes(crop):
    """How much the mean brightness varies from row to row and from column to column of the clip's middle."""
    box = picture(crop)
    w, h = box.size
    pixels = list(box.getdata())
    rows = [sum(pixels[y * w:(y + 1) * w]) / w for y in range(h)]
    columns = [sum(pixels[x::w]) / h for x in range(w)]
    spread = lambda values: (sum((v - sum(values) / len(values)) ** 2 for v in values) / len(values)) ** 0.5
    return spread(rows), spread(columns)


def look(r, name):
    """The Effects section in view, at the smallest window, 1024 x 640, then at 1440 x 900, where the flow goes on."""
    show = ("[...document.querySelectorAll('aside[aria-label=Inspector] h3')].find((h) => h.textContent === 'Effects')"
            ".closest('section').scrollIntoView({block: 'center'})")
    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        r.s.run(show)
        time.sleep(1)
        r.shot(name + suffix)


def difference(a, b):
    """Mean brightness difference, 0 to 255, of the clip's middle in two preview crops."""
    from PIL import ImageChops, ImageStat
    return ImageStat.Stat(ImageChops.difference(picture(a), picture(b))).mean[0]


def toasts(r, text):
    return [t for t in r.state()['toasts'] if text in t['text']]


@flow('plugin_effects', 'A plugin effect added in the inspector changes the preview with its bands across the rotated '
      'photo as shown; a broken shader leaves the clip drawn without it and says so; the project opens and exports '
      'without the plugin and says what it was drawn without', before=prepare)
def plugin_effects(r):
    photo = phone_photo(r)
    r.import_media(photo)
    r.add_clip(photo.name)
    clip = r.track()[0]
    r.focus_clip(clip['id'])
    r.seek(clip['durationUs'] // 2)
    r.check('the inspector opens the Adjust tab', wait(lambda: r.s.run(TAB, 'Adjust'), 5))
    r.check('the Adjust tab has an Effects section', wait(lambda: r.s.run(
        "return [...document.querySelectorAll('aside[aria-label=Inspector] h3')].some((h) => h.textContent === 'Effects')"), 5))
    time.sleep(1.5)
    rect = preview_rect(r)
    r.shot('before')
    before = preview_crop(r.work / 'before.png', rect)

    add = "document.querySelector('aside[aria-label=Inspector] button[aria-label=\"Add effect\"]')"
    r.check('Add effect is enabled once the plugin loads', wait(lambda: r.s.run(f'return !!{add} && !{add}.getAttribute("aria-disabled")'), 10))
    r.s.run(f'{add}.click()')
    item = "[...document.querySelectorAll('[role=menuitem]')].find((b) => b.textContent.includes('VHS'))"
    r.check('the menu lists the VHS effect', wait(lambda: r.s.run(f'return !!{item}'), 5))
    r.s.run(f'{item}.click()')
    added = wait(lambda: r.s.run(EFFECTS, clip['id']), 5)
    r.check('the clip gets the effect with its defaults', added and added[0]['plugin'] == 'vhs' and added[0]['effect'] == 'vhs'
            and added[0]['params'] == {'amount': 0.6, 'lines': True, 'tint': '#FFC890'}, added)
    saved = lambda: [c for t in json.loads(r.saved_project().read_text())['tracks'] for c in t['clips']][0].get('effects')
    r.check('the saved project names the effect, without its shader', wait(lambda: (saved() or [{}])[0].get('name') == 'VHS', 5)
            and 'fn effect' not in r.saved_project().read_text(), saved())
    r.check('the effect shows its settings', wait(lambda: r.s.run(f"return !!{ROW}?.querySelector('input[type=range][aria-label=\"Amount\"]')"), 5))

    end = time.time() + 8
    while True:
        r.shot('effect')
        shown = preview_crop(r.work / 'effect.png', rect)
        rows, columns = stripes(shown)
        if (difference(shown, before) > 8 and rows > 3 * columns) or time.time() > end:
            break
        time.sleep(0.5)
    r.check('the preview draws the effect', difference(shown, before) > 8, difference(shown, before))
    r.check('its bands run across the photo as shown, not across the stored sideways picture', rows > 3 * columns,
            {'rows': rows, 'columns': columns})
    r.check('nothing says the effect is missing', not toasts(r, 'Drawn without'), r.state()['toasts'])
    look(r, 'effect-section')

    effect_file = r.work / 'data/nuzky/plugins/vhs/effects/vhs.wgsl'
    effect_file.write_text('fn effect(p: Pixel) -> vec4<f32> {\n    var c = p.color;\n    loop { break; }\n    return c;\n}\n')
    warning = f"return {ROW}?.textContent ?? ''"
    r.check('the inspector says the broken effect is left out, with its file and line', wait(lambda:
            'VHS has an error, so the clip is drawn without it: effects/vhs.wgsl:3:5' in r.s.run(warning), 10), r.s.run(warning))
    r.check('a warning says the preview is drawn without it', wait(lambda: toasts(r, 'Drawn without VHS'), 10), r.state()['toasts'])
    end = time.time() + 8
    while True:
        r.shot('broken')
        broken = preview_crop(r.work / 'broken.png', rect)
        if difference(broken, before) < 2 or time.time() > end:
            break
        time.sleep(0.5)
    r.check('the preview still draws the clip, without the effect', difference(broken, before) < 2
            and changed_share(broken, before) < 0.02, difference(broken, before))

    shutil.rmtree(r.work / 'data/nuzky/plugins/vhs')
    # A copy, as a project sent from another computer: opening the project already open would not open it again.
    path = r.work / 'from-another-computer.nuzky'
    shutil.copy(r.saved_project(), path)
    epoch = r.state()['epoch']
    r.s.run('window.__nuzky.store.setState({toasts: []})')
    r.s.run('window.__nuzky.openProject({path: arguments[0], name: "effects"})', str(path))
    r.check('the project opens again without the plugin', wait(lambda: r.state()['epoch'] != epoch and r.track(), 15))
    r.check('opening it says what it is drawn without', wait(lambda: toasts(r, 'Drawn without VHS: it is missing'), 10),
            r.state()['toasts'])
    clip = r.track()[0]
    r.check('the clip keeps the effect it cannot draw', r.s.run(EFFECTS, clip['id'])[0]['name'] == 'VHS')
    r.focus_clip(clip['id'])
    wait(lambda: r.s.run(TAB, 'Adjust'), 5)
    r.check('the inspector says the effect is not installed', wait(lambda:
            'VHS is not installed on this computer, so the clip is drawn without it.' in r.s.run(warning), 10), r.s.run(warning))
    look(r, "missing")

    out = export(r, 'without.mp4')
    job = next(j for j in r.state()['jobs'] if j['kind'] == 'export')
    r.check('the export finishes and says it was drawn without VHS', out.exists() and (job['message'] or '').startswith(
        'Exported without VHS'), job)
    r.check('a warning toast says so too', wait(lambda: toasts(r, 'Exported without VHS'), 5), r.state()['toasts'])
    r.shot('exported')
    r.check('no error toast', not r.errors(), r.errors())

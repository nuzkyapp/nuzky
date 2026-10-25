import json
import shutil
import time

from e2e.harness import ROOT, changed_share, export, flow, preview_crop, preview_rect, wait, webdriver

EXAMPLES = ROOT / 'skills/nuzky-plugin/examples'
EFFECTS = ("return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips)"
           ".find((c) => c.id === arguments[0]).effects ?? []")
TAB = ("const t = [...document.querySelectorAll('aside[aria-label=Inspector] [role=tab]')]"
       ".find((t) => t.textContent.trim() === arguments[0]); if (!t) return false; t.click(); return true")
ROW = "document.querySelector('aside[aria-label=Inspector] [data-effect=\"vhs/vhs\"]')"
SECTION = ("[...document.querySelectorAll('aside[aria-label=Inspector] h3')].find((h) => h.textContent === 'Effects')"
           "?.closest('section')")
ADD = "document.querySelector('aside[aria-label=Inspector] button[aria-label^=\"Add effect\"]')"
CLICK = """const b = [...document.querySelectorAll(arguments[0] + ' button')].find((b) => b.textContent.trim() === arguments[1]);
if (!b) return false; b.click(); return true;"""
MENU_ITEM = "[...document.querySelectorAll('[role=menuitem]')].find((b) => b.textContent.includes(arguments[0]))"
INSPECTOR = "return document.querySelector('aside[aria-label=Inspector]')?.innerText ?? ''"
DRAFT = "return window.__nuzky.agent.getState().draft"
TOASTS = "return window.__nuzky.store.getState().toasts.map((t) => ({kind: t.kind, text: t.text, action: t.action?.label ?? null}))"
WARM = 'fn effect(p: Pixel) -> vec4<f32> {\n    let k = p.params[0];\n    return vec4<f32>(p.color.r, p.color.g * (1.0 - 0.1 * k), p.color.b * (1.0 - 0.3 * k), p.color.a);\n}\n'


def plugins(r):
    return r.work / 'data/nuzky/plugins'


def install_warm(r):
    """A plugin with a script, the one that removes short clips, and an effect beside it."""
    folder = plugins(r) / 'warm'
    shutil.copytree(EXAMPLES / 'remove-short-clips', folder)
    manifest = json.loads((folder / 'plugin.json').read_text())
    manifest.update(nuzkyPlugin=2, id='warm', name='Warm and tidy', description='Removes short clips and adds a warm look.', effects=[
        {'id': 'warm', 'name': 'Warm', 'params': [
            {'type': 'number', 'id': 'strength', 'label': 'Strength', 'min': 0, 'max': 1, 'step': 0.05, 'default': 0.5}]}])
    (folder / 'plugin.json').write_text(json.dumps(manifest))
    (folder / 'effects').mkdir()
    (folder / 'effects/warm.wgsl').write_text(WARM)


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


def size(r, width, height):
    webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
    wait(lambda: r.s.run('return window.innerWidth') == width, 5)


def look(r, name):
    """The Effects section in view, at the smallest window, 1024 x 640, then at 1440 x 900, where the flow goes on."""
    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        size(r, width, height)
        r.s.run(f"{SECTION}.scrollIntoView({{block: 'center'}})")
        time.sleep(1)
        r.shot(name + suffix)


def difference(a, b):
    """Mean brightness difference, 0 to 255, of the clip's middle in two preview crops."""
    from PIL import ImageChops, ImageStat
    return ImageStat.Stat(ImageChops.difference(picture(a), picture(b))).mean[0]


def toasts(r, text):
    return [t for t in r.s.run(TOASTS) if text in t['text']]


def adjust(r, clip_id):
    r.focus_clip(clip_id)
    return wait(lambda: r.s.run(TAB, 'Adjust'), 5)


def open_plugin(r, plugin_id):
    r.s.run("window.__nuzky.store.setState({panelTab: 'plugins'})")
    wait(lambda: r.s.run(f"return !!document.querySelector('[data-plugin=\"{plugin_id}\"]')"), 5)
    r.s.run(f"document.querySelector('[data-plugin=\"{plugin_id}\"]').click()")
    return wait(lambda: r.s.run(f"return !!document.querySelector('aside[aria-label=Inspector] [data-plugin-effect]')"), 5)


def add_effect(r, name):
    r.s.run(f'{ADD}.click()')
    wait(lambda: r.s.run(f'return !!{MENU_ITEM}', name), 5)
    r.s.run(f'{MENU_ITEM}.click()', name)


@flow('plugin_effects', 'Without an effect plugin the Effects section says where effects come from; an effect added '
      'from the + menu changes the preview with its bands across the rotated photo as shown, and a second one stacks '
      'under a line; the Plugins tab lists each plugin\'s effects; a broken shader leaves the clip drawn without it, '
      'says so plainly once and hands the detail to the AI; the project opens and exports without the plugin, warning '
      'once, in the export dialog or a toast')
def plugin_effects(r):
    # Fix with AI and Make an effect put their text in the AI panel's field without running an agent.
    r.s.run("window.__nuzky.agent.setState({agent: 'antigravity'})")
    photo = phone_photo(r)
    r.import_media(photo)
    r.add_clip(photo.name)
    clip = r.track()[0]
    r.seek(clip['durationUs'] // 2)
    r.check('the inspector opens the Adjust tab', adjust(r, clip['id']))
    empty = lambda: r.s.run(f"const s = {SECTION}; return s && {{text: s.innerText, add: !!{ADD}, "
                            "make: [...s.querySelectorAll('button')].some((b) => b.textContent.trim() === 'Make an effect')}")
    r.check('without an effect plugin the Effects section says where effects come from, offers Make an effect and has no +',
            wait(lambda: (e := empty()) and 'Effects come from plugins.' in e['text'] and e['make'] and not e['add'], 10), empty())
    look(r, 'empty')
    r.s.run(CLICK, 'aside[aria-label=Inspector]', 'Make an effect')
    r.check('Make an effect opens the AI panel with the request started',
            wait(lambda: r.s.run(DRAFT) == 'Make a Nuzky plugin with an effect that ' and r.s.run('return window.__nuzky.dock.getState().open'), 5),
            r.s.run(DRAFT))
    r.s.run("window.__nuzky.dock.setState({open: false}); window.__nuzky.agent.setState({draft: ''})")

    shutil.copytree(EXAMPLES / 'vhs', plugins(r) / 'vhs')
    install_warm(r)
    r.check('Add effect is enabled once the plugins load', wait(lambda: r.s.run(f'return !!{ADD} && !{ADD}.getAttribute("aria-disabled")'), 10))
    time.sleep(1.5)
    rect = preview_rect(r)
    r.shot('before')
    before = preview_crop(r.work / 'before.png', rect)

    r.s.run(f'{ADD}.click()')
    r.check('the menu lists the VHS and Warm effects', wait(lambda: r.s.run(f'return !!{MENU_ITEM}', 'VHS') and r.s.run(f'return !!{MENU_ITEM}', 'Warm'), 5))
    time.sleep(0.3)
    r.shot('add-menu')
    r.s.run(f'{MENU_ITEM}.click()', 'VHS')
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
    r.check('nothing warns about the effect', not toasts(r, 'VHS'), r.s.run(TOASTS))
    look(r, 'effect-section')

    add_effect(r, 'Warm')
    r.check('a second effect stacks after the first', wait(lambda: [e['effect'] for e in r.s.run(EFFECTS, clip['id'])] == ['vhs', 'warm'], 5),
            r.s.run(EFFECTS, clip['id']))
    groups = lambda: r.s.run("return [...document.querySelectorAll('aside[aria-label=Inspector] [data-effect]')].map((g) => ({"
                             "line: getComputedStyle(g).borderTopWidth, name: getComputedStyle(g.querySelector('span')).fontSize}))")
    r.check('a line parts the two effects, and their names are smaller than the section title',
            wait(lambda: groups() == [{'line': '0px', 'name': '12px'}, {'line': '1px', 'name': '12px'}], 5), groups())
    look(r, 'two-effects')
    r.s.run("document.querySelector('aside[aria-label=Inspector] button[aria-label=\"Remove Warm\"]').click()")
    r.check('× removes the second effect', wait(lambda: [e['effect'] for e in r.s.run(EFFECTS, clip['id'])] == ['vhs'], 5))

    # A switch left on from before the plugin lost its script, as an older version of it could have had.
    auto = "return window.__nuzky.plugins.getState().auto"
    r.s.run("window.__nuzky.plugins.setState((s) => ({auto: {import: [...s.auto.import, 'vhs'], export: [...s.auto.export, 'vhs']}}))")
    manifest = plugins(r) / 'vhs/plugin.json'
    manifest.write_text(manifest.read_text())
    r.check('the plugins list turns off the automatic runs of an effects-only plugin, which has nothing to run',
            wait(lambda: (a := r.s.run(auto)) and 'vhs' not in a['import'] and 'vhs' not in a['export'], 5), r.s.run(auto))
    r.check('an effects-only plugin lists its effect and says where to add it', open_plugin(r, 'vhs') and wait(lambda:
            'This plugin only adds effects. Select a video or image, then add them in Adjust → Effects.' in r.s.run(INSPECTOR)
            and 'VHS' in r.s.run(INSPECTOR), 5), r.s.run(INSPECTOR))
    r.check('it offers no switches to run by itself and no button', not any(
        word in r.s.run(INSPECTOR) for word in ('Run after import', 'Run before export', 'Run\n')), r.s.run(INSPECTOR))
    time.sleep(0.5)
    r.shot('plugin-effects-only')
    r.check('a plugin with a script lists its effects after its settings, above its switches and its button', open_plugin(r, 'warm') and wait(lambda:
            (t := r.s.run(INSPECTOR)) and 'Remove clips' in t and 'Warm' in t and 'Run after import' in t
            and t.index('Effects') < t.index('Run after import') < t.index('Remove clips')
            and 'Select a video or image, then add them in Adjust → Effects.' in t, 5),
            r.s.run(INSPECTOR))
    time.sleep(0.5)
    r.shot('plugin-script-and-effects')
    r.s.run("window.__nuzky.store.setState({panelTab: 'media'})")
    adjust(r, clip['id'])

    effect_file = plugins(r) / 'vhs/effects/vhs.wgsl'
    effect_file.write_text('fn effect(p: Pixel) -> vec4<f32> {\n    var c = p.color;\n    loop { break; }\n    return c;\n}\n')
    warning = f"return {ROW}?.textContent ?? ''"
    r.check('the row says plainly that the broken effect is left out, without the file and line', wait(lambda:
            r.s.run(warning) == 'VHSVHS has an error, so the clip is drawn without it. Fix with AI', 10), r.s.run(warning))
    said = 'VHS has an error, so the video plays without it.'
    r.check('one warning says the video plays without it, with Fix with AI', wait(lambda: toasts(r, said), 10)
            and toasts(r, 'VHS') == [{'kind': 'warning', 'text': said, 'action': 'Fix with AI'}], r.s.run(TOASTS))
    end = time.time() + 8
    while True:
        r.shot('broken')
        broken = preview_crop(r.work / 'broken.png', rect)
        if difference(broken, before) < 2 or time.time() > end:
            break
        time.sleep(0.5)
    r.check('the preview still draws the clip, without the effect', difference(broken, before) < 2
            and changed_share(broken, before) < 0.02, difference(broken, before))
    look(r, 'broken-section')
    r.s.run(CLICK, 'aside[aria-label=Inspector] [data-effect="vhs/vhs"]', 'Fix with AI')
    r.check('Fix with AI hands the file, line and why to the AI panel', wait(lambda: (d := r.s.run(DRAFT)) and d.startswith(
            'Fix the effect “VHS” of the Nuzky plugin “VHS” (vhs). Nuzky says: effects/vhs.wgsl:3:5:'), 5), r.s.run(DRAFT))
    r.s.run("window.__nuzky.dock.setState({open: false}); window.__nuzky.agent.setState({draft: ''})")
    time.sleep(2)
    r.check('the warning stays one toast', len(toasts(r, 'VHS')) == 1, r.s.run(TOASTS))

    plugin_row = "return document.querySelector('[data-plugin=\"vhs\"]')?.closest('li')?.innerText ?? ''"
    r.check('the plugin with the broken effect opens on its effect with why, in warn, and Fix with AI', open_plugin(r, 'vhs') and wait(lambda:
            'Can’t draw: effects/vhs.wgsl:3:5' in r.s.run(INSPECTOR) and r.s.run(
                "const p = [...document.querySelectorAll('aside[aria-label=Inspector] [data-plugin-effect] p')].find((p) => p.textContent.startsWith('Can’t draw'));"
                "return !!p && p.classList.contains('text-warn') && !!p.parentElement.querySelector('button')"), 5), r.s.run(INSPECTOR))
    r.check('its row in the Plugins tab is marked, with Fix with AI', 'Can’t draw VHS' in r.s.run(plugin_row) and 'Fix with AI' in r.s.run(plugin_row),
            r.s.run(plugin_row))
    time.sleep(0.5)
    r.shot('plugin-broken')
    r.s.run("window.__nuzky.store.setState({panelTab: 'media'})")

    shutil.rmtree(plugins(r) / 'vhs')
    # A copy, as a project sent from another computer: opening the project already open would not open it again.
    path = r.work / 'from-another-computer.nuzky'
    shutil.copy(r.saved_project(), path)
    epoch = r.state()['epoch']
    r.s.run('window.__nuzky.store.setState({toasts: []})')
    r.s.run('window.__nuzky.openProject({path: arguments[0], name: "effects"})', str(path))
    r.check('the project opens again without the plugin', wait(lambda: r.state()['epoch'] != epoch and r.track(), 15))
    said = "VHS isn't installed on this computer, so the video plays and exports without it."
    r.check('opening it says once that the video plays and exports without it', wait(lambda: toasts(r, said), 10)
            and toasts(r, 'VHS') == [{'kind': 'warning', 'text': said, 'action': None}], r.s.run(TOASTS))
    clip = r.track()[0]
    r.check('the clip keeps the effect it cannot draw', r.s.run(EFFECTS, clip['id'])[0]['name'] == 'VHS')
    adjust(r, clip['id'])
    r.check('the inspector says the effect is not installed', wait(lambda:
            r.s.run(warning) == "VHSVHS isn't installed on this computer, so the clip is drawn without it.", 10), r.s.run(warning))
    look(r, 'missing')

    exported = "Exported without VHS: it isn't installed on this computer."
    options = {'resolution': 720, 'fps': 30, 'quality': 'small'}
    start = 'window.__nuzky.api.startExport(arguments[0], arguments[1], arguments[2], arguments[3])'
    started = r.s.call(start, str(r.work / 'in-dialog.mp4'), options, r.state()['epoch'], True)
    r.s.run('window.__nuzky.store.setState({exportJobId: arguments[0], exportOpen: true})', started.get('value'))
    alert = "return [...document.querySelectorAll('[role=dialog] [role=alert]')].map((a) => a.textContent)"
    r.check('the export dialog says what the export was drawn without and why', wait(lambda: r.s.run(alert) == [exported], 120), r.s.run(alert))
    time.sleep(0.5)
    r.shot('export-dialog')
    r.check('with the dialog open no second toast repeats it', not toasts(r, 'Exported without') and len(toasts(r, 'VHS')) == 1, r.s.run(TOASTS))
    r.s.run(CLICK, '[role=dialog]', 'Done')
    wait(lambda: not r.s.run("return !!document.querySelector('[role=dialog]')"), 5)

    export(r, 'again.mp4')
    time.sleep(1)
    r.check('with the dialog closed and the warning still open, the export adds no toast', not toasts(r, 'Exported without'), r.s.run(TOASTS))
    r.s.run('window.__nuzky.store.setState({toasts: []})')
    out = export(r, 'without.mp4')
    job = r.state()['jobs'][-1]
    r.check('the export finishes and says it was drawn without VHS because it is not installed', out.exists() and job['message'] == exported, job)
    r.check('once that warning is gone, a toast says it', wait(lambda: toasts(r, 'Exported without') == [{'kind': 'warning', 'text': exported, 'action': None}], 5),
            r.s.run(TOASTS))
    r.shot('exported')
    r.check('no error toast', not r.errors(), r.errors())

"""CapCut's text settings in the inspector: case, letter spacing and a shadow, set by keyboard, change what the
preview, the engine's frame and the export draw, while the stored text stays as typed. The text ends in the creator's
CapCut caption look (size 12 at 152 %, #fff059, a black shadow at 41 %, Blur 23 %, Distance 5, -45°); two quick
changes to the shadow both stick, an AI run locks the controls with a reason, and one undo takes a change back. With
three texts selected, as with every caption, one shadow goes on all of them at once."""
import json, subprocess, time

from e2e.adjust import select_all
from e2e.harness import AI_EDITING, CLI, Bridge, changed_share, export, flow, preview_crop, preview_rect, press, wait, webdriver

SECTION = 'aside[aria-label=Inspector]'
STYLE = """const p = window.__nuzky.store.getState().snap.project;
const c = p.tracks.flatMap((t) => t.clips).find((c) => c.content.type === 'text');
return c ? {id: c.id, text: c.content.text, style: c.content.style} : null;"""
# CapCut's size 12 on a 1080-wide frame, as opening a draft converts it, in the creator's font when it is installed.
ADD_TEXT = """window.__nuzky.store.getState().edit({type: 'addText', startUs: 0, text: 'ahoj světe',
  style: {fontSize: 61.2, fontFamily: arguments[0], color: '#fff059', bold: false, strokeWidth: 0, strokeColor: '#000000',
          background: null}})"""
SCALE = """window.__nuzky.store.getState().edit((p) => {
  const c = p.tracks.flatMap((t) => t.clips).find((c) => c.id === arguments[0]);
  return {type: 'updateClip', clipId: c.id, transform: {...c.content.transform, scale: 1.52}};
})"""
CASE = f"document.querySelector('{SECTION} [role=group][aria-label=Case]')"
MORE = f"[...document.querySelectorAll('{SECTION} button[aria-expanded]')].find((b) => b.textContent.startsWith('More style'))"
# CapCut's own shadow, as turning it on starts it, and the creator's.
SHADOW_ON = {'color': '#000000', 'opacity': 0.9, 'blur': 15, 'distance': 5, 'angle': -45}
LOOK = {'color': '#000000', 'opacity': 0.41, 'blur': 23, 'distance': 5, 'angle': -45}
# Two slider moves in one go, before the first change comes back from the engine.
QUICK = f"""const set = (name, v) => {{
  const input = document.querySelector(`{SECTION} input[type=range][aria-label="${{name}}"]`);
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, String(v));
  input.dispatchEvent(new Event('input', {{bubbles: true}}));
}};
set('Shadow opacity', 41);
set('Shadow blur', 23);"""


def same(shadow, wanted):
    """The project keeps numbers in single precision: 0.6 comes back as 0.6000000238."""
    return bool(shadow) and shadow['color'] == wanted['color'] and all(abs(shadow[k] - wanted[k]) < 1e-6 for k in wanted if k != 'color')


def grey(r):
    r.grey = r.work / 'grey.png'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'color=0x808080:s=1080x1920', '-frames:v', '1',
                    str(r.grey)], check=True)


def ink(image):
    """Where the text's yellow and its shadow lie: the centre of each as fractions of the picture, the shadow's share
    and how wide the yellow runs. On the grey frame (128) the shadow is what is clearly darker than grey."""
    from PIL import Image
    image = (Image.open(image) if not hasattr(image, 'convert') else image).convert('RGB')
    w, h = image.size
    yellow, dark = [], []
    for i, (red, green, blue) in enumerate(image.getdata()):
        if red > 200 and green > 190 and blue < 140:
            yellow.append(i)
        elif red + green + blue < 3 * 115:
            dark.append(i)
    centre = lambda found: (sum(i % w for i in found) / len(found) / w, sum(i // w for i in found) / len(found) / h) if found else None
    span = (max(i % w for i in yellow) - min(i % w for i in yellow)) / w if yellow else 0
    return centre(yellow), centre(dark), len(dark) / (w * h), span


def shadow_shift(image):
    pixels = list(image.getdata())
    yellow = [(i % image.width, i // image.width) for i, (r, g, b) in enumerate(pixels) if r > 200 and g > 190 and b < 140]
    dark = {(i % image.width, i // image.width): 128 - max(px) for i, px in enumerate(pixels) if max(px) < 128}
    score, dx, dy = max((sum(dark.get((x + dx, y + dy), 0) for x, y in yellow), dx, dy)
                       for dx in range(-4, 5) for dy in range(-4, 5))
    return (dx, dy) if score else (0, 0)


def frame(screenshot, rect):
    """The grey frame the preview draws inside its canvas, without the dark stage around it."""
    crop = preview_crop(screenshot, rect).convert('RGB')
    w = crop.width
    grey = [i for i, px in enumerate(crop.getdata()) if all(abs(c - 128) < 8 for c in px)]
    xs, ys = [i % w for i in grey], [i // w for i in grey]
    return crop.crop((min(xs), min(ys), max(xs) + 1, max(ys) + 1))


def engine_frame(r, name):
    path = r.work / f'{name}.png'
    subprocess.run([str(CLI), 'frame', str(r.saved_project()), '1.0', str(path), '720'], env=r.env, check=True, capture_output=True)
    r.shots.append(path.name)
    return path


def saved_style(r):
    clips = [c for t in json.loads(r.saved_project().read_text())['tracks'] for c in t['clips'] if c['content']['type'] == 'text']
    return clips[0]['content'] if clips else None


def type_value(r, label, value):
    """Types into a property row's number field and confirms with Enter, as a person does."""
    wait(lambda: select_all(r, f"document.querySelector('{SECTION} input[aria-label=\"{label} value\"]')"), 5)
    for key in str(value):
        press(key)
    typed = lambda: r.s.run(f"return document.querySelector('{SECTION} input[aria-label=\"{label} value\"]').value")
    r.check(f'{label} holds the typed text before Enter', wait(lambda: typed() == str(value), 3), typed())
    press('Return')


def look(r, name):
    """The Style section in view, from Case and down to the shadow, at 1440 x 900 and at the smallest window,
    1024 x 640."""
    views = {'case': f'{CASE}.scrollIntoView({{block: "center"}})', 'shadow': SHADOW_VIEW}
    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        for part, show in views.items():
            r.s.run(show)
            time.sleep(1)
            r.shot(f'{name}-{part}{suffix}')


SHADOW_VIEW = f"{MORE}.closest('section').querySelector('input[aria-label=\"Shadow angle value\"]').scrollIntoView({{block: 'end'}})"
TEXTS = """return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips)
  .filter((c) => c.content.type === 'text').map((c) => ({id: c.id, shadow: c.content.style.shadow ?? null}));"""
PLACE = """window.__nuzky.store.getState().edit((p) => {
  const c = p.tracks.flatMap((t) => t.clips).find((c) => c.id === arguments[0]);
  return {type: 'updateClip', clipId: c.id, transform: {...c.content.transform, y: arguments[1]}};
})"""
SHADOW_BOX = f"[...document.querySelectorAll('{SECTION} label')].find((l) => l.textContent.trim() === 'Shadow')?.querySelector('input')"
OPACITY = f"document.querySelector('{SECTION} input[aria-label=\"Shadow opacity value\"]')"
BOX = """window.__nuzky.store.getState().edit((p) => arguments[0].map(([id, background]) => {
  const c = p.tracks.flatMap((t) => t.clips).find((c) => c.id === id);
  return {type: 'updateClip', clipId: id, style: {...c.content.style, background}};
}))"""
BOXES = """return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips)
  .filter((c) => c.content.type === 'text').map((c) => [c.id, c.content.style.background]);"""
PICK = f"""const input = document.querySelector('{SECTION} input[type=color][aria-label="Box color"]');
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, arguments[0]);
input.dispatchEvent(new Event('input', {{bubbles: true}}));"""
BOX_LABEL = f"""return [...document.querySelectorAll('{SECTION} label')].find((l) => l.querySelector('input[aria-label="Box color"]'))
  ?.querySelector('span.tabular')?.textContent"""


def shadow(r):
    return r.s.run(STYLE)['style'].get('shadow') or {}


@flow('text-style', 'Case, letter spacing and a shadow from the text inspector, by keyboard, in the preview, the frame and '
      'the export, in the creator\'s CapCut look, with the typed text kept, quick changes kept, locked during an AI run '
      'and undone in one step, and on three selected texts at once', before=grey)
def text_style(r):
    r.import_media(r.grey)
    r.add_clip('grey.png')
    fonts = json.loads(r.s.call('window.__nuzky.api.listFonts().then((f) => JSON.stringify(f))')['value'])
    font = 'Safiro' if 'Safiro' in fonts['bundled'] + fonts['system'] else None
    r.check('a text is added in the creator\'s look, in Safiro when it is installed and else the default font',
            r.s.call(ADD_TEXT, font)['ok'], {'font': font or 'the default font, Safiro is not installed'})
    text = wait(lambda: r.s.run(STYLE), 5)
    r.check('at 152 %', r.s.call(SCALE, text['id'])['ok'])
    r.focus_clip(text['id'])
    r.check('Case is in view and the rest of the style waits folded under More style',
            wait(lambda: r.s.run(f"return !!{CASE} && {MORE}?.getAttribute('aria-expanded') === 'false' && "
                                 f"!document.querySelector('{SECTION} input[aria-label=\"Letter spacing value\"]')"), 10))
    r.s.run(f'{MORE}.focus()')
    press('Return')
    if not r.check('Enter opens More style, with Letter spacing, Line spacing and Shadow',
                   wait(lambda: r.s.run(f"return !!{CASE} && ['Letter spacing value', 'Line spacing value'].every((l) => "
                                        f"document.querySelector(`{SECTION} input[aria-label=\"${{l}}\"]`)) && "
                                        f"[...document.querySelectorAll('{SECTION} label')].some((l) => l.textContent.trim() === 'Shadow')"), 10)):
        r.shot('inspector-missing')
        return
    r.seek(1_000_000)
    time.sleep(1.5)
    r.s.run('window.__nuzky.store.getState().select([])')
    time.sleep(1)
    rect = preview_rect(r)
    r.shot('preview-plain')
    plain = ink(frame(r.work / 'preview-plain.png', rect))
    r.check('without a shadow nothing under the text is darker than the frame', plain[0] and plain[2] < 0.0002, plain)
    r.focus_clip(text['id'])

    # Case: Tab from As typed to All capitals, then Enter.
    r.s.run(f'{CASE}.querySelector("button").focus()')
    r.check('As typed is a neutral sign named for screen readers, not letters that read as a case',
            r.s.run(f'const b = {CASE}.querySelector("button"); return b.querySelector("svg") && b.textContent.trim() === "As typed"'))
    press('Tab')
    r.check('Tab from As typed reaches AB', wait(lambda: r.s.run('return document.activeElement.textContent') == 'AB', 3, 0.05))
    press('Return')
    r.check('Enter draws the text in capitals', wait(lambda: r.s.run(STYLE)['style'].get('letterCase') == 'upper', 5), r.s.run(STYLE)['style'])
    r.check('and keeps the text as typed', r.s.run(STYLE)['text'] == 'ahoj světe', r.s.run(STYLE)['text'])

    type_value(r, 'Letter spacing', 10)
    r.check('Letter spacing 10 % is a tenth of the font size', wait(lambda: abs(r.s.run(STYLE)['style'].get('letterSpacing', 0) - 0.1) < 1e-6, 5),
            r.s.run(STYLE)['style'])

    # Shadow: Space on its checkbox turns it on as CapCut starts one.
    r.s.run(f"[...document.querySelectorAll('{SECTION} label')].find((l) => l.textContent.trim() === 'Shadow').querySelector('input').focus()")
    press('space')
    r.check('Space turns the shadow on as CapCut starts one: black at 90 %, Blur 15 %, Distance 5, -45°',
            wait(lambda: same(shadow(r), SHADOW_ON), 5), shadow(r))
    r.check('its rows read Opacity, Blur, Distance and Angle, and screen readers hear Shadow before each',
            r.s.run(f"""const s = {MORE}.closest('section');
                return ['Opacity', 'Blur', 'Distance', 'Angle'].every((l, i) => [...s.querySelectorAll('span')].some((e) => e.textContent === l)
                  && s.querySelector(`input[type=range][aria-label="Shadow ${{l.toLowerCase()}}"]`))"""))
    r.s.run(QUICK)
    wait(lambda: shadow(r).get('blur') == 23, 5)
    time.sleep(1)
    r.check('opacity and blur moved at once both stick: 41 % and Blur 23 %', same(shadow(r), LOOK), shadow(r))

    # Angles wrap: 225 is the same direction as -135.
    type_value(r, 'Shadow angle', 225)
    r.check('a typed angle of 225° becomes -135°', wait(lambda: shadow(r).get('angle') == -135, 5), shadow(r))
    type_value(r, 'Shadow angle', 315)
    r.check('and 315° is -45° again', wait(lambda: shadow(r).get('angle') == -45, 5), shadow(r))

    # One undo takes one change back.
    type_value(r, 'Shadow distance', 24)
    r.check('a typed distance moves it', wait(lambda: shadow(r).get('distance') == 24, 5), shadow(r))
    r.s.run('document.activeElement?.blur()')
    r.key('z', ctrlKey=True)
    r.check('Ctrl+Z takes the distance back to 5 and leaves the rest of the look',
            wait(lambda: same(shadow(r), LOOK), 5), shadow(r))

    r.focus_clip(text['id'])
    look(r, 'inspector-text-style')
    time.sleep(1.5)  # Saved about a second after the edit; `nuzky frame` reads the saved file.
    saved = saved_style(r)
    r.check('the project file has the case, spacing and the creator\'s shadow, and the text as typed',
            saved and saved['text'] == 'ahoj světe' and saved['style'].get('letterCase') == 'upper'
            and saved['style'].get('letterSpacing') == 0.1 and same(saved['style'].get('shadow'), LOOK), saved)

    # What is drawn: the shadow below and right of the letters, in the preview where the engine draws it.
    engine = ink(engine_frame(r, 'engine-shadow'))
    r.check('the engine draws the shadow down and to the right of the text',
            engine[0] and engine[1] and engine[1][0] > engine[0][0] and engine[1][1] > engine[0][1] and engine[2] > 0.0005, engine)
    r.s.run('window.__nuzky.store.getState().select([])')
    time.sleep(1.5)
    r.shot('preview-shadow')
    shown = frame(r.work / 'preview-shadow.png', rect)
    preview = ink(shown)
    # The preview draws the frame much smaller, so the soft shadow's edge fades sooner: compare at its size.
    from PIL import Image
    drawn = Image.open(r.work / 'engine-shadow.png').convert('RGB').resize(shown.size, Image.LANCZOS)
    engine = ink(drawn)
    shifts = {'preview': shadow_shift(shown), 'engine': shadow_shift(drawn)}
    near = lambda a, b: a and b and all(abs(x - y) < 0.02 for x, y in zip(a, b))
    r.check('the preview draws text and shadow where the engine does',
            near(preview[0], engine[0]) and all(dx > 0 and dy > 0 for dx, dy in shifts.values()),
            {'preview': preview, 'engine': engine, 'shifts': shifts})
    r.check('capitals and spacing make the text wider, still centred where it was',
            preview[3] > plain[3] * 1.15 and preview[0] and abs(preview[0][0] - plain[0][0]) < 0.01, {'now': preview, 'typed': plain})
    target = export(r, 'text-style.mp4')
    exported = r.work / 'export-shadow.png'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-i', str(target), '-vf', 'select=eq(n\\,30)', '-frames:v', '1', '-update', '1',
                    str(exported)], check=True)
    r.shots.append(exported.name)
    changed = changed_share(r.work / 'engine-shadow.png', exported)
    r.check('the exported frame matches the frame the renderer draws for the preview', changed < 0.005, f'{changed:.3%} of pixels differ')

    # An AI run locks the controls and says why.
    r.focus_clip(text['id'])
    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Restyle captions'})
        r.check('the UI knows an agent is editing', wait(lambda: r.state()['aiRun'], 10))
        locked = r.s.run(f"""const b = {CASE}.querySelector('button');
            const s = {MORE}.closest('section');
            const box = [...s.querySelectorAll('label')].find((l) => l.textContent.trim() === 'Shadow');
            return {{caseLocked: b.getAttribute('aria-disabled') === 'true' || b.disabled, why: b.title,
                     shadowLocked: box.querySelector('input').disabled,
                     slidersLocked: ['Letter spacing', 'Line spacing', 'Shadow opacity', 'Shadow blur', 'Shadow distance', 'Shadow angle']
                       .every((l) => s.querySelector(`input[type=range][aria-label="${{l}}"]`).disabled)}};""")
        r.check('Case, spacing and every shadow control are locked and say why',
                locked['caseLocked'] and locked['shadowLocked'] and locked['slidersLocked'] and locked['why'] == AI_EDITING, locked)
        r.s.run(f'{CASE}.scrollIntoView({{block: "start"}})')
        time.sleep(1)
        r.shot('inspector-locked')
        bridge.call('end_run', {'run_id': run['run_id'], 'action': 'keep'})
        r.check('the lock ends with the run', wait(lambda: not r.state()['aiRun'], 10))
    finally:
        bridge.close()
    several(r, text['id'], font)
    r.check('no error toast', not r.errors(), r.errors())


def several(r, first, font):
    """Two more texts above and below the first, without a shadow, and all three selected, as with every caption."""
    for _ in range(2):
        r.s.call(ADD_TEXT, font)
    others = [t['id'] for t in wait(lambda: len(r.s.run(TEXTS)) == 3 and r.s.run(TEXTS), 5) if t['id'] != first]
    for clip_id, y in zip(others, (-0.3, 0.3)):
        r.s.call(PLACE, clip_id, y)
    r.s.run('window.__nuzky.store.getState().select(arguments[0])', [first, *others])
    box = wait(lambda: r.s.run(f'const b = {SHADOW_BOX}; return b && {{mixed: b.indeterminate, rows: !!{OPACITY}}}'), 10)
    r.check('three selected texts have Shadow, with a dash as only one has it, and no shadow rows yet',
            box and box['mixed'] and not box['rows'], box)
    shadows = lambda: {t['id']: t['shadow'] for t in r.s.run(TEXTS)}
    r.s.run(f'{SHADOW_BOX}.focus()')
    press('space')
    r.check("Space gives the two without one CapCut's shadow, and the first keeps its own",
            wait(lambda: same(shadows()[first], LOOK) and all(same(shadows()[i], SHADOW_ON) for i in others), 5), shadows())
    r.check('Opacity shows a dash, as the shadows differ', r.s.run(f'return {OPACITY}.value') == '—', r.s.run(f'return {OPACITY}.value'))
    type_value(r, 'Shadow opacity', 60)
    r.check('a typed opacity of 60 % goes on all three, each keeping its own blur',
            wait(lambda: all(abs(s['opacity'] - 0.6) < 1e-6 for s in shadows().values()), 5)
            and shadows()[first]['blur'] == 23 and all(shadows()[i]['blur'] == 15 for i in others), shadows())
    look(r, 'inspector-several-texts')
    r.s.run('document.activeElement?.blur()')
    r.key('z', ctrlKey=True)
    r.check('one Ctrl+Z takes the opacity back on all three',
            wait(lambda: same(shadows()[first], LOOK) and all(same(shadows()[i], SHADOW_ON) for i in others), 5), shadows())
    time.sleep(1.5)  # Saved about a second after the edit; `nuzky frame` reads the saved file.
    from PIL import Image
    drawn = Image.open(engine_frame(r, 'engine-three-shadows')).convert('RGB')
    w, h = drawn.size
    bands = [ink(drawn.crop((0, h * i // 3, w, h * (i + 1) // 3))) for i in range(3)]
    r.check('the engine draws a shadow down and to the right of each of the three texts',
            all(b[0] and b[1] and b[1][0] > b[0][0] and b[1][1] > b[0][1] and b[2] > 0.0005 for b in bands), bands)

    r.s.call(BOX, [[first, '#000000b3'], *[[i, '#ff000080'] for i in others]])
    r.check('Box color shows a dash while the box colours differ', wait(lambda: r.s.run(BOX_LABEL) == '—', 5), r.s.run(BOX_LABEL))
    r.s.run(PICK, '#2050ff')
    boxes = lambda: dict(r.s.run(BOXES))
    r.check('a picked box colour goes on all three, each keeping its own opacity',
            wait(lambda: boxes()[first] == '#2050ffb3' and all(boxes()[i] == '#2050ff80' for i in others), 5), boxes())
    r.check('and the dash gives way to the picked colour', wait(lambda: r.s.run(BOX_LABEL) == '#2050FF', 5), r.s.run(BOX_LABEL))

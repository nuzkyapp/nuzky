"""CapCut's text settings in the inspector: case, letter spacing and a shadow, set by keyboard, change what the
preview, the engine's frame and the export draw, while the stored text stays as typed; an AI run locks them with a
reason, and one undo takes a change back."""
import json, subprocess, time

from e2e.harness import AI_EDITING, CLI, Bridge, changed_share, export, flow, preview_crop, preview_rect, press, wait, webdriver

SECTION = 'aside[aria-label=Inspector]'
STYLE = """const p = window.__nuzky.store.getState().snap.project;
const c = p.tracks.flatMap((t) => t.clips).find((c) => c.content.type === 'text');
return c ? {id: c.id, text: c.content.text, style: c.content.style} : null;"""
ADD_TEXT = """window.__nuzky.store.getState().edit({type: 'addText', startUs: 0, text: 'ahoj světe',
  style: {fontSize: 120, color: '#fff059', bold: false, strokeWidth: 0, strokeColor: '#000000', background: null}})"""
CASE = f"document.querySelector('{SECTION} [role=group][aria-label=Case]')"
SHADOW_ON = {'color': '#000000', 'opacity': 0.6, 'blur': 7, 'distance': 5, 'angle': -45}


def same(shadow, wanted):
    """The project keeps numbers in single precision: 0.6 comes back as 0.6000000238."""
    return shadow is not None and shadow['color'] == wanted['color'] and all(abs(shadow[k] - wanted[k]) < 1e-6 for k in wanted if k != 'color')


def grey(r):
    r.grey = r.work / 'grey.png'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'color=0x808080:s=1080x1920', '-frames:v', '1',
                    str(r.grey)], check=True)


def ink(image):
    """Where the text's yellow and its shadow lie: the centre of each as fractions of the picture, the shadow's share
    and how wide the yellow runs. On the grey frame the shadow is what is clearly darker than grey."""
    from PIL import Image
    image = (Image.open(image) if not hasattr(image, 'convert') else image).convert('RGB')
    w, h = image.size
    yellow, dark = [], []
    for i, (red, green, blue) in enumerate(image.getdata()):
        if red > 200 and green > 190 and blue < 140:
            yellow.append(i)
        elif red + green + blue < 3 * 100:
            dark.append(i)
    centre = lambda found: (sum(i % w for i in found) / len(found) / w, sum(i // w for i in found) / len(found) / h) if found else None
    span = (max(i % w for i in yellow) - min(i % w for i in yellow)) / w if yellow else 0
    return centre(yellow), centre(dark), len(dark) / (w * h), span


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
    r.s.run(f"document.querySelector('{SECTION} input[aria-label=\"{label} value\"]').select()")
    for key in str(value):
        press(key)
    press('Return')


def look(r, name):
    """The Style section in view, from Case and down to the shadow, at 1440 x 900 and at the smallest window,
    1024 x 640."""
    views = {'case': f'{CASE}.scrollIntoView({{block: "center"}})',
             'shadow': f"{CASE}.closest('section').querySelector('input[aria-label=\"Shadow angle value\"]').scrollIntoView({{block: 'end'}})"}
    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        for part, show in views.items():
            r.s.run(show)
            time.sleep(1)
            r.shot(f'{name}-{part}{suffix}')


@flow('text-style', 'Case, letter spacing and a shadow from the text inspector, by keyboard, in the preview, the frame and '
      'the export, with the typed text kept, locked during an AI run and undone in one step', before=grey)
def text_style(r):
    r.import_media(r.grey)
    r.add_clip('grey.png')
    r.check('a text is added', r.s.call(ADD_TEXT)['ok'])
    text = wait(lambda: r.s.run(STYLE), 5)
    r.focus_clip(text['id'])
    if not r.check('the inspector has Case, Letter spacing, Line spacing and Shadow',
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
    press('Tab')
    r.check('Tab from Aa reaches AB', wait(lambda: r.s.run('return document.activeElement.textContent') == 'AB', 3, 0.05))
    press('Return')
    r.check('Enter draws the text in capitals', wait(lambda: r.s.run(STYLE)['style'].get('letterCase') == 'upper', 5), r.s.run(STYLE)['style'])
    r.check('and keeps the text as typed', r.s.run(STYLE)['text'] == 'ahoj světe', r.s.run(STYLE)['text'])

    type_value(r, 'Letter spacing', 10)
    r.check('Letter spacing 10 % is a tenth of the font size', wait(lambda: abs(r.s.run(STYLE)['style'].get('letterSpacing', 0) - 0.1) < 1e-6, 5),
            r.s.run(STYLE)['style'])

    # Shadow: Space on its checkbox turns it on as it starts, then a longer distance typed in.
    r.s.run(f"[...document.querySelectorAll('{SECTION} label')].find((l) => l.textContent.trim() === 'Shadow').querySelector('input').focus()")
    press('space')
    r.check('Space turns the shadow on, black, down and to the right',
            wait(lambda: same(r.s.run(STYLE)['style'].get('shadow'), SHADOW_ON), 5), r.s.run(STYLE)['style'].get('shadow'))
    type_value(r, 'Shadow distance', 24)
    r.check('a typed distance moves it', wait(lambda: (r.s.run(STYLE)['style'].get('shadow') or {}).get('distance') == 24, 5),
            r.s.run(STYLE)['style'].get('shadow'))
    look(r, 'inspector-text-style')
    time.sleep(1.5)  # Saved about a second after the edit; `nuzky frame` reads the saved file.
    saved = saved_style(r)
    r.check('the project file has the case, spacing and shadow, and the text as typed',
            saved and saved['text'] == 'ahoj světe' and saved['style'].get('letterCase') == 'upper'
            and saved['style'].get('letterSpacing') == 0.1 and saved['style'].get('shadow', {}).get('distance') == 24, saved)

    # What is drawn: the shadow below and right of the letters, in the preview where the engine draws it.
    engine = ink(engine_frame(r, 'engine-shadow'))
    r.check('the engine draws the shadow down and to the right of the text',
            engine[0] and engine[1] and engine[1][0] > engine[0][0] and engine[1][1] > engine[0][1] and engine[2] > 0.001, engine)
    r.s.run('window.__nuzky.store.getState().select([])')
    time.sleep(1.5)
    r.shot('preview-shadow')
    preview = ink(frame(r.work / 'preview-shadow.png', rect))
    near = lambda a, b: a and b and all(abs(x - y) < 0.02 for x, y in zip(a, b))
    r.check('the preview draws text and shadow where the engine does', near(preview[0], engine[0]) and near(preview[1], engine[1]),
            {'preview': preview, 'engine': engine})
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
            const box = [...document.querySelectorAll('{SECTION} label')].find((l) => l.textContent.trim() === 'Shadow');
            return {{caseLocked: b.getAttribute('aria-disabled') === 'true' || b.disabled, why: b.title,
                     shadowLocked: box.querySelector('input').disabled}};""")
        r.check('Case and Shadow are locked and say why', locked['caseLocked'] and locked['shadowLocked'] and locked['why'] == AI_EDITING, locked)
        r.shot('inspector-locked')
        bridge.call('end_run', {'run_id': run['run_id'], 'action': 'keep'})
        r.check('the lock ends with the run', wait(lambda: not r.state()['aiRun'], 10))
    finally:
        bridge.close()

    r.s.run('document.activeElement?.blur()')
    r.key('z', ctrlKey=True)
    r.check('Ctrl+Z takes the distance back to where the shadow started',
            wait(lambda: (r.s.run(STYLE)['style'].get('shadow') or {}).get('distance') == 5, 5), r.s.run(STYLE)['style'].get('shadow'))
    r.check('no error toast', not r.errors(), r.errors())

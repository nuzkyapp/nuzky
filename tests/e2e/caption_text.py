"""A caption's text style goes to every caption unless Apply to says This caption, as CapCut's Apply to all captions:
All caps from one caption's inspector capitalises every caption and survives picking another style, This caption keeps
a colour to one caption, More style stays folded until opened and then aligns every caption and gives them a box with
its opacity and fully round corners, and Position moves every caption at once and undoes in one step. The Captions tab
shows the same controls for every caption. Apply to stays usable while an AI run locks the style. What the user sees is
checked in the project, the saved file and the engine's frame."""
import json, time

from e2e.harness import AI_EDITING, Bridge, flow, press, wait, webdriver
from e2e.text_style import engine_frame, grey, type_value

SECTION = 'aside[aria-label=Inspector]'
LIBRARY = 'aside[data-tour=library]'
REEL = {'fontSize': 95, 'color': '#ffffff', 'bold': False, 'strokeWidth': 7.5, 'strokeColor': '#000000', 'background': None}
TEXTS = ['no a pak', 'tohle je fakt', 'ne?']
ADD = """window.__nuzky.store.getState().edit({type: 'addCaptions', style: arguments[0],
  segments: arguments[1].map((text, i) => ({startUs: i * 1000000, endUs: (i + 1) * 1000000, text}))})"""
CAPTIONS = """const t = window.__nuzky.store.getState().snap.project.tracks.find((t) => t.kind === 'text' && t.name === 'Captions');
return t ? t.clips.map((c) => ({id: c.id, text: c.content.text, style: c.content.style, y: c.content.transform.y})) : [];"""
FOLD = "[...document.querySelectorAll('{0} button[aria-expanded]')].find((b) => b.textContent.startsWith('{1}'))"


def captions(r):
    return r.s.run(CAPTIONS)


def group(scope, label):
    return f"document.querySelector('{scope} [role=group][aria-label=\"{label}\"]')"


def segment(scope, label, text):
    """A segment of a segmented control by its text, or by its tooltip for an icon."""
    return f"[...{group(scope, label)}.querySelectorAll('button')].find((b) => b.textContent.trim() === {text!r} || b.title === {text!r})"


def pressed(r, scope, label):
    return r.s.run(f"return [...{group(scope, label)}.querySelectorAll('button')].find((b) => b.getAttribute('aria-pressed') === 'true')?.textContent.trim() ?? null")


def checkbox(scope, label):
    return f"[...document.querySelectorAll('{scope} label')].find((l) => l.textContent.trim() === {label!r})?.querySelector('input')"


def open_fold(r, scope, title):
    """Opens a folded section from the keyboard, as a person does."""
    fold = FOLD.format(scope, title)
    r.s.run(f'{reveal(fold)} {fold}.focus()')
    press('Return')
    return wait(lambda: r.s.run(f"return {fold}.getAttribute('aria-expanded')") == 'true', 5)


def every(r, test):
    return wait(lambda: (found := captions(r)) and all(test(c) for c in found), 5)


def reveal(selector):
    """Scrolls the panel that holds the element so it is in view, without moving the window as scrollIntoView can."""
    return (f"const el = {selector}; const pane = el.parentElement.closest('.overflow-y-auto');"
            "pane.scrollTop += el.getBoundingClientRect().top - pane.getBoundingClientRect().top - 12;")


def views(r, name, show):
    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        r.s.run(show)
        time.sleep(1)
        r.shot(f'{name}{suffix}')


def box_corner(path):
    """How much of the box's corner is grey frame: the patch a tenth of the box's height in from the corner of its
    bounds lies outside a box of one line rounded fully, and inside one with the usual small radius. None without a box.
    The box at 40 % over the grey frame and the black outline are both darker than the frame."""
    from PIL import Image
    image = Image.open(path).convert('RGB')
    w = image.width
    dark = [(i % w, i // w) for i, px in enumerate(image.getdata()) if max(px) < 100]
    if len(dark) < 500:
        return None
    left, top, bottom = min(x for x, _ in dark), min(y for _, y in dark), max(y for _, y in dark)
    inset = round((bottom - top) * 0.1)
    patch = [image.getpixel((left + inset + dx, top + inset + dy)) for dx in range(-1, 2) for dy in range(-1, 2)]
    return sum(1 for px in patch if all(abs(c - 128) < 20 for c in px)) / len(patch)


@flow('caption-text', 'A caption\'s text style goes to every caption unless Apply to says This caption: All caps, a style '
      'that keeps the caps, one caption\'s own colour, More style\'s alignment, box opacity and round corners, and Position '
      'for every caption, in the project and the engine\'s frame, with the same controls in the Captions tab', before=grey)
def caption_text(r):
    r.import_media(r.grey)
    r.add_clip('grey.png')
    r.check('three captions are added', r.s.call(ADD, REEL, TEXTS)['ok'] and wait(lambda: len(captions(r)) == 3, 5), captions(r))
    second = captions(r)[1]['id']
    r.focus_clip(second)
    if not r.check('a caption\'s inspector offers Apply to, set to All captions',
                   wait(lambda: r.s.run(f"return !!{group(SECTION, 'Apply style to')}") and pressed(r, SECTION, 'Apply style to') == 'All captions', 10)):
        r.shot('apply-to-missing')
        return
    folded = r.s.run(f"""return {{case: !!{group(SECTION, 'Case')},
        more: {FOLD.format(SECTION, 'More style')}?.getAttribute('aria-expanded'),
        position: {FOLD.format(SECTION, 'Position of all captions')}?.getAttribute('aria-expanded'),
        spacing: !!document.querySelector('{SECTION} input[aria-label="Letter spacing value"]'),
        shadow: !!{checkbox(SECTION, 'Shadow')}}}""")
    r.check('Case is in view, while spacing, the shadow and Position wait folded under More style and Position',
            folded == {'case': True, 'more': 'false', 'position': 'false', 'spacing': False, 'shadow': False}, folded)
    views(r, 'inspector-caption', reveal(f"document.querySelector('{SECTION} textarea[aria-label=Text]')"))

    # All caps from one caption's inspector, by keyboard: Tab from As typed to AB, then Enter.
    r.s.run(f"{group(SECTION, 'Case')}.querySelector('button').focus()")
    press('Tab')
    r.check('Tab from As typed reaches AB, named All caps', wait(lambda: r.s.run('return [document.activeElement.textContent, document.activeElement.title]') == ['AB', 'All caps'], 3, 0.05))
    press('Return')
    r.check('Enter draws every caption in capitals', every(r, lambda c: c['style'].get('letterCase') == 'upper'), captions(r))
    r.check('and keeps each caption\'s text as typed', [c['text'] for c in captions(r)] == TEXTS, captions(r))

    r.s.run(f"document.querySelector('{SECTION} button[title=\"Apply Outline to all captions\"]').click()")
    r.check('the Outline style goes on every caption and keeps the capitals',
            every(r, lambda c: c['style']['strokeWidth'] == 7 and c['style']['bold'] and c['style'].get('letterCase') == 'upper'), captions(r))
    r.check('its tile is checked', wait(lambda: r.s.run(f"return document.querySelector('{SECTION} button[title=\"Apply Outline to all captions\"]').getAttribute('aria-pressed')") == 'true', 5))

    # This caption: a colour for the selected caption only.
    r.s.run(f'{segment(SECTION, "Apply style to", "This caption")}.click()')
    r.check('Apply to switches to This caption', wait(lambda: pressed(r, SECTION, 'Apply style to') == 'This caption', 5))
    r.s.run(f"""const input = document.querySelector('{SECTION} input[type=color][aria-label="Color"]');
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, '#ffe14d');
        input.dispatchEvent(new Event('input', {{bubbles: true}}));""")
    colours = lambda: [c['style']['color'] for c in captions(r)]
    r.check('a colour picked for This caption goes on the selected caption only',
            wait(lambda: colours() == ['#ffffff', '#ffe14d', '#ffffff'], 5), colours())
    r.seek(1_500_000)
    views(r, 'inspector-this-caption', reveal(f"document.querySelector('{SECTION} textarea[aria-label=Text]')"))
    r.s.run(f'{segment(SECTION, "Apply style to", "All captions")}.click()')
    r.check('and Apply to goes back to All captions', wait(lambda: pressed(r, SECTION, 'Apply style to') == 'All captions', 5))
    r.check('where the colours differ, Color shows a dash',
            wait(lambda: r.s.run(f"return [...document.querySelectorAll('{SECTION} label')].find((l) => l.querySelector('input[aria-label=\"Color\"]'))?.querySelector('span.tabular')?.textContent") == '—', 5))

    # More style: alignment, a box, its opacity and corners on every caption.
    if not r.check('Enter opens More style', open_fold(r, SECTION, 'More style')):
        return
    r.s.run(f'{segment(SECTION, "Alignment", "Align right")}.click()')
    r.check('Align right goes on every caption', every(r, lambda c: c['style'].get('align') == 'right'), [c['style'].get('align') for c in captions(r)])
    r.s.run(f'{checkbox(SECTION, "Background box")}.click()')
    r.check('Background box gives every caption a black box at 70 %', every(r, lambda c: c['style']['background'] == '#000000b3'), [c['style']['background'] for c in captions(r)])
    type_value(r, 'Box opacity', 40)
    r.check('Opacity 40 % makes every box 40 % opaque, still black', every(r, lambda c: c['style']['background'] == '#00000066'), [c['style']['background'] for c in captions(r)])
    type_value(r, 'Box corners', 100)
    r.check('Corners 100 % rounds every box fully', every(r, lambda c: abs(c['style'].get('boxRadius', 0.25) - 1) < 1e-6), [c['style'].get('boxRadius') for c in captions(r)])
    views(r, 'inspector-more-style', reveal(FOLD.format(SECTION, 'More style')))

    # Position: every caption up at once, and one undo takes it back.
    if not r.check('Enter opens Position', open_fold(r, SECTION, 'Position of all captions')):
        return
    type_value(r, 'Position Y', -20)
    r.check('Position Y -20 % moves every caption up there', every(r, lambda c: abs(c['y'] + 0.2) < 1e-6), [c['y'] for c in captions(r)])
    shown = lambda: r.s.run(f"""return Object.fromEntries(['Scale', 'Position X', 'Position Y', 'Rotation', 'Opacity'].map((l) =>
        [l, document.querySelector(`{SECTION} input[aria-label="${{l}} value"]`)?.value]))""")
    r.check('each Position row shows its own value', wait(lambda: shown() == {'Scale': '100', 'Position X': '0.0', 'Position Y': '-20.0', 'Rotation': '0', 'Opacity': '100'}, 5), shown())
    views(r, 'inspector-position', reveal(FOLD.format(SECTION, 'Position of all captions')))
    r.s.run('document.activeElement?.blur()')
    r.key('z', ctrlKey=True)
    r.check('one Ctrl+Z puts every caption back where captions start and keeps their look',
            every(r, lambda c: abs(c['y'] - 0.15) < 1e-6 and abs(c['style'].get('boxRadius', 0.25) - 1) < 1e-6), captions(r))

    time.sleep(1.5)  # Saved about a second after the edit; `nuzky frame` reads the saved file.
    saved = [c['content'] for t in json.loads(r.saved_project().read_text())['tracks'] if t.get('name') == 'Captions' for c in t['clips']]
    r.check('the project file has the capitals, alignment, box and its corners on every caption, and the text as typed',
            len(saved) == 3 and all(c['style'].get('letterCase') == 'upper' and c['style'].get('align') == 'right' and c['style']['background'] == '#00000066'
                                    and c['style'].get('boxRadius') == 1 for c in saved) and [c['text'] for c in saved] == TEXTS, saved)
    corner = box_corner(engine_frame(r, 'engine-round-box'))
    r.check('the engine draws the caption on a box whose corner is round, leaving the frame grey there', corner is not None and corner > 0.8, corner)

    # The Captions tab: the same controls, for every caption.
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === 'Captions').click()")
    if not r.check('the Captions tab shows Case with AB, as every caption is in capitals',
                   wait(lambda: r.s.run(f"return !!{group(LIBRARY, 'Case')}") and pressed(r, LIBRARY, 'Case') == 'AB', 10)):
        r.shot('captions-tab-missing')
        return
    r.s.run(f"""const input = document.querySelector('{LIBRARY} input[aria-label="Size value"]');
        input.focus(); input.select();""")
    for key in '120':
        press(key)
    press('Return')
    r.check('Size 120 in the Captions tab goes on every caption', every(r, lambda c: c['style']['fontSize'] == 120), [c['style']['fontSize'] for c in captions(r)])
    # My styles keeps the look, and its tile shows the box as round and the label lined up as the captions are.
    r.s.run(f"document.querySelector('{LIBRARY} button[aria-label=\"Save the current look as a style\"]').click()")
    name = f"document.querySelector('{LIBRARY} input[aria-label=\"Style name\"]')"
    if r.check('saving asks for a name in place', wait(lambda: r.s.run(f'return !!{name}'), 5)):
        r.s.run(f"const f = {name}; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(f, 'Round box');"
                "f.dispatchEvent(new Event('input', {bubbles: true}));"
                "f.dispatchEvent(new KeyboardEvent('keydown', {key: 'Enter', bubbles: true}));")
        swatch = lambda: r.s.run(f"""const t = [...document.querySelectorAll('{LIBRARY} button[aria-pressed]')].find((b) => b.textContent.trim() === 'Round box');
            const s = t && getComputedStyle(t.querySelector('span'));
            return s ? {{radius: s.borderTopLeftRadius, align: s.textAlign}} : null""")
        r.check('the saved style\'s tile draws a fully round box and lines its name up right',
                wait(lambda: swatch() == {'radius': '11px', 'align': 'right'}, 5), swatch())
    views(r, 'captions-tab', reveal(f"[...document.querySelectorAll('{LIBRARY} span')].find((s) => s.textContent === 'Font')"))

    # An agent can add a second captions track; a caption there styles the captions of its own track only.
    tracks = lambda: r.s.run("""return window.__nuzky.store.getState().snap.project.tracks.filter((t) => t.kind === 'text' && t.name === 'Captions')
        .map((t) => t.clips.map((c) => ({id: c.id, text: c.content.text, case: c.content.style.letterCase ?? 'none'})))""")
    added = r.s.call(ADD, REEL, ['druhá', 'stopa'])['ok'] and wait(lambda: len(tracks()) == 2, 5)
    if r.check('an agent\'s second captions track is there', added, tracks()):
        other = next(t for t in tracks() if [c['text'] for c in t] == ['druhá', 'stopa'])
        r.focus_clip(other[0]['id'])
        wait(lambda: r.s.run(f"return !!{group(SECTION, 'Case')}"), 5)
        r.s.run(f'{segment(SECTION, "Case", "ab")}.click()')
        cases = lambda: {c['text']: c['case'] for t in tracks() for c in t}
        r.check('small letters go on every caption of that track, and the first track keeps its capitals',
                wait(lambda: cases() == {'no a pak': 'upper', 'tohle je fakt': 'upper', 'ne?': 'upper', 'druhá': 'lower', 'stopa': 'lower'}, 5), cases())
        r.focus_clip(second)

    # An AI run locks the style, while Apply to only chooses what the controls edit.
    r.focus_clip(second)
    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Restyle captions'})
        r.check('the UI knows an agent is editing', wait(lambda: r.state()['aiRun'], 10))
        locked = r.s.run(f"""const scope = [...{group(SECTION, 'Apply style to')}.querySelectorAll('button')];
            const c = {group(SECTION, 'Case')}.querySelector('button');
            return {{applyTo: scope.every((b) => !b.disabled && b.getAttribute('aria-disabled') !== 'true'),
                     case: c.getAttribute('aria-disabled') === 'true', why: c.title}}""")
        r.check('Case is locked and says why, Apply to stays usable', locked == {'applyTo': True, 'case': True, 'why': AI_EDITING}, locked)
        bridge.call('end_run', {'run_id': run['run_id'], 'action': 'keep'})
        r.check('the lock ends with the run', wait(lambda: not r.state()['aiRun'], 10))
    finally:
        bridge.close()
    r.check('no error toast', not r.errors(), r.errors())

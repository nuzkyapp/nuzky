"""The library of text styles. The Text tab lists them in groups; pointing at a style, or reaching it with the keyboard,
draws it in the preview before it is used, without changing the project, and leaving it draws the project again. A
click adds the text with the style's animation as one undo step. A caption's inspector previews a caption style on
every caption the same way, and a title's inspector previews and applies another style from Show all. The export
draws the box where the preview showed it."""
import subprocess, time

from e2e.harness import export, flow, preview_crop, preview_rect, press, wait, webdriver
from e2e.text_style import grey

SECTION = 'aside[aria-label=Inspector]'
LIBRARY = 'aside[data-tour=library]'
TEXTS = """return window.__nuzky.store.getState().snap.project.tracks.filter((t) => t.name !== 'Captions')
  .flatMap((t) => t.clips).filter((c) => c.content.type === 'text')
  .map((c) => ({id: c.id, style: c.content.style, animIn: c.animIn ?? null, animOut: c.animOut ?? null}));"""
CAPTIONS = """const t = window.__nuzky.store.getState().snap.project.tracks.find((t) => t.name === 'Captions');
return t ? t.clips.map((c) => ({id: c.id, style: c.content.style})) : [];"""
REEL = {'fontSize': 95, 'color': '#ffffff', 'bold': False, 'strokeWidth': 7.5, 'strokeColor': '#000000', 'background': None}
ADD_CAPTIONS = """window.__nuzky.store.getState().edit({type: 'addCaptions', style: arguments[0],
  segments: ['no a pak', 'tohle je fakt', 'ne?'].map((text, i) => ({startUs: i * 1000000, endUs: (i + 1) * 1000000, text}))})"""
REVISION = 'return window.__nuzky.store.getState().snap.revision'
OVER = ".dispatchEvent(new PointerEvent('pointerover', {bubbles: true, pointerType: 'mouse'}))"
OUT = ".dispatchEvent(new PointerEvent('pointerout', {bubbles: true, pointerType: 'mouse', relatedTarget: document.body}))"


def library_tile(name):
    return f"document.querySelector('{LIBRARY} [data-text-preset=\"{name}\"]')"


def inspector_tile(name):
    """A style tile in the inspector by the name drawn in it; the app's tooltip takes a hovered button's title."""
    return f"[...document.querySelectorAll('{SECTION} button[aria-pressed]')].find((b) => b.textContent.trim() === '{name}')"


def red(px):
    """The Red style's #e11d48 box, over the grey picture."""
    return px[0] > 170 and px[1] < 90 and px[2] < 120


def blue(px):
    """The Blue style's #2563eb box."""
    return px[2] > 170 and px[0] < 100 and px[1] < 150


def ink(image, colour):
    """The share of the picture in `colour`, and the middle of it and its width as fractions of the picture."""
    image = image.convert('RGB')
    w, h = image.size
    found = [i for i, px in enumerate(image.getdata()) if colour(px)]
    if not found:
        return 0, None, 0
    xs, ys = [i % w for i in found], [i // w for i in found]
    return len(found) / (w * h), ((min(xs) + max(xs)) / 2 / w, (min(ys) + max(ys)) / 2 / h), (max(xs) - min(xs)) / w


def preview(r, name):
    r.shot(name)
    return preview_crop(r.work / f'{name}.png', preview_rect(r))


def shows(r, name, colour, present, timeout=5):
    """Screenshots the preview until it shows `colour` (or no longer does, for `present` False); returns what it saw."""
    end = time.time() + timeout
    while True:
        seen = ink(preview(r, name), colour)
        if (seen[0] > 0.002) == present and (present or seen[0] < 0.0002):
            return True, seen
        if time.time() > end:
            return False, seen
        time.sleep(0.4)


def window(r, width, height):
    webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
    wait(lambda: r.s.run('return window.innerWidth') == width, 5)
    time.sleep(1)


@flow('text-styles', 'The Text tab\'s styles in groups, each drawn in the preview while pointed at or focused before it is '
      'used and gone again after, added with its animation as one undo step, previewed on every caption from a caption '
      'style, applied to a title from its inspector, and exported where the preview showed it', before=grey)
def text_styles(r):
    r.import_media(r.grey)
    r.add_clip('grey.png')
    r.seek(0)
    r.s.run("window.__nuzky.store.setState({panelTab: 'text'})")
    groups = "return [...document.querySelectorAll(`" + LIBRARY + " section`)].map((s) => [s.getAttribute('aria-label'), s.querySelectorAll('[data-text-preset]').length])"
    listed = wait(lambda: (g := r.s.run(groups)) and len(g) >= 7 and g, 10)
    r.check('the Text tab lists 30 styles in the groups Basic, Outline, Shadow, Glow, Box, Bold and Animated',
            listed and [n for n, _ in listed] == ['Basic', 'Outline', 'Shadow', 'Glow', 'Box', 'Bold', 'Animated']
            and sum(c for _, c in listed) == 30, listed)
    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        window(r, width, height)
        r.shot(f'library{suffix}')
    time.sleep(1)
    before = r.s.run(REVISION)
    r.check('the grey picture has nothing red or blue before a style is pointed at', ink(preview(r, 'plain'), red)[0] == 0)

    # Pointing at a style draws its text in the preview, as the export would, and changes nothing.
    r.s.run(library_tile('Red') + OVER)
    ok, seen = shows(r, 'library-hover-red-box', red, True)
    r.check('pointing at Red draws its red box in the preview', ok, seen)
    r.check('and adds no text to the project', r.s.run(TEXTS) == [] and r.s.run(REVISION) == before, r.s.run(TEXTS))
    r.s.run(library_tile('Red') + OUT)
    ok, seen = shows(r, 'library-left', red, False)
    r.check('leaving it draws the project again', ok, seen)
    r.s.run(library_tile('Red') + OVER)
    shows(r, 'library-hover-again', red, True)
    r.s.run('window.__nuzky.store.getState().togglePlay()')
    ok, seen = shows(r, 'library-play', red, False)
    r.check('playback that starts while a style is previewed plays the project as it is', ok, seen)
    r.s.run('window.__nuzky.store.getState().playing && window.__nuzky.store.getState().togglePlay()')
    r.s.run(library_tile('Red') + OUT)
    r.seek(0)

    # The keyboard's way: focus previews, Tab moves the preview on to the next style.
    r.s.run(f"{library_tile('Blue')}.focus()")
    ok, seen = shows(r, 'library-focus-blue-box', blue, True)
    r.check('focusing Blue with the keyboard previews it', ok, seen)
    press('Tab')
    r.check('Tab reaches Pill', wait(lambda: r.s.run('return document.activeElement.dataset.textPreset') == 'Pill', 3))
    ok, seen = shows(r, 'library-tab-pill', blue, False)
    r.check('and the preview follows it: Blue is gone', ok, seen)
    r.s.run('document.activeElement.blur()')

    # A click adds the style with its animations, as one undo step.
    r.s.run(f"{library_tile('Rise')}.focus()")
    press('Return')
    rise = wait(lambda: (t := r.s.run(TEXTS)) and t[0], 5)
    r.check('Enter on Rise adds its text in Poppins with a soft shadow, sliding up in and fading out',
            rise and rise['style'].get('fontFamily') == 'Poppins' and rise['style'].get('shadow', {}).get('blur') == 40
            and (rise['animIn'] or {}).get('kind') == 'slideUp' and (rise['animOut'] or {}).get('kind') == 'fade', rise)
    r.check('and moves the playhead past its 0.3 s entry, so the new text shows',
            wait(lambda: r.s.run('return window.__nuzky.store.getState().timeUs') == 300_000, 3), r.s.run('return window.__nuzky.store.getState().timeUs'))
    r.s.run('document.activeElement.blur()')
    # Two clicks before the first add comes back from the engine.
    r.s.run(f"{library_tile('Rise')}.click(); {library_tile('Rise')}.click()")
    r.check('two quick clicks add two more, each animated', wait(lambda: (t := r.s.run(TEXTS)) and len(t) == 3 and all(c['animIn'] for c in t), 5), r.s.run(TEXTS))
    for left in (2, 1):
        r.s.call('window.__nuzky.store.getState().undo()')
        r.check(f'one undo takes one whole text away, {left} left, each still animated',
                wait(lambda: (t := r.s.run(TEXTS)) and len(t) == left and all(c['animIn'] for c in t), 5), r.s.run(TEXTS))
    r.s.call('window.__nuzky.store.getState().undo()')
    r.check('one undo takes the text and its animations away', wait(lambda: r.s.run(TEXTS) == [], 5), r.s.run(TEXTS))
    r.s.call('window.__nuzky.store.getState().redo()')
    rise = wait(lambda: (t := r.s.run(TEXTS)) and t[0], 5)
    r.check('and redo brings both back', rise and (rise['animIn'] or {}).get('kind') == 'slideUp', rise)

    # A caption style previews on every caption, without changing them.
    r.check('three captions in the Reel style', r.s.call(ADD_CAPTIONS, REEL)['ok'] and wait(lambda: len(r.s.run(CAPTIONS)) == 3, 5))
    caption = r.s.run(CAPTIONS)[1]
    r.seek(1_500_000)
    r.focus_clip(caption['id'])
    style_tile = inspector_tile('Red box')
    r.check('a caption\'s inspector offers the new caption styles', wait(lambda: r.s.run(f'return !!{style_tile}'), 10))
    r.s.run(f'{style_tile}.scrollIntoView({{block: "center"}})')
    time.sleep(0.5)
    r.s.run(style_tile + OVER)
    ok, seen = shows(r, 'caption-hover-red-box', red, True)
    r.check('pointing at the Red box caption style draws the caption in its box', ok, seen)
    r.check('while every caption keeps the Reel style', all(c['style'].get('background') is None for c in r.s.run(CAPTIONS)))
    r.s.run(style_tile + OUT)
    ok, seen = shows(r, 'caption-left', red, False)
    r.check('leaving it draws the captions as they are', ok, seen)
    r.s.call('window.__nuzky.store.getState().undo()')
    r.check('the captions go with undo', wait(lambda: r.s.run(CAPTIONS) == [], 5))

    # A title's inspector: Show all, preview, then apply.
    r.focus_clip(rise['id'])
    show_all = f"[...document.querySelectorAll('{SECTION} button[aria-expanded]')].find((b) => b.textContent.startsWith('Show all'))"
    r.check('the title\'s inspector shows the Basic styles and Show all 30 styles',
            wait(lambda: r.s.run(f"return {show_all}?.textContent") == 'Show all 30 styles', 10), r.s.run(f"return {show_all}?.textContent"))
    r.s.run(f'{show_all}.click()')
    red_tile = inspector_tile('Red')
    r.check('Show all lists every group', wait(lambda: r.s.run(f'return !!{red_tile} && document.querySelectorAll(`{SECTION} [role=group][aria-label=Glow]`).length === 1'), 5))
    r.s.run(f'{red_tile}.scrollIntoView({{block: "center"}})')
    time.sleep(0.5)
    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        window(r, width, height)
        r.s.run(f'{red_tile}.scrollIntoView({{block: "center"}})')
        time.sleep(0.5)
        r.shot(f'inspector{suffix}')
    r.s.run(red_tile + OVER)
    ok, hovered = shows(r, 'title-hover-red-box', red, True)
    r.check('pointing at Red in the title\'s inspector previews it', ok, hovered)
    r.check('and the title keeps Rise', r.s.run(TEXTS)[0]['style'].get('background') is None)
    r.s.run(f'{red_tile}.click()')
    applied = wait(lambda: (t := r.s.run(TEXTS)[0]) and t['style'].get('background') == '#e11d48ee' and t, 5)
    r.check('a click gives the title Red: Montserrat 800 in capitals on the red box, keeping its own animation',
            applied and applied['style'].get('fontFamily') == 'Montserrat' and applied['style'].get('fontWeight') == 800
            and applied['style'].get('letterCase') == 'upper' and (applied['animIn'] or {}).get('kind') == 'slideUp', applied)
    r.s.run(red_tile + OUT)
    r.s.run('window.__nuzky.store.getState().select([])')
    time.sleep(1.5)
    ok, shown = shows(r, 'title-red-box', red, True)
    r.check('the preview keeps the red box after the pointer leaves', ok, shown)
    r.check('the preview drew while pointing what the click applied', abs(shown[0] - hovered[0]) < 0.002 and
            abs(shown[1][0] - hovered[1][0]) < 0.01 and abs(shown[1][1] - hovered[1][1]) < 0.01, {'hovered': hovered, 'applied': shown})

    video = export(r, 'red-box.mp4')
    frame = r.work / 'export-1.5s.png'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-ss', '1.5', '-i', str(video), '-frames:v', '1', str(frame)], check=True)
    r.shots.append(frame.name)
    from PIL import Image
    exported = ink(Image.open(frame), red)
    r.check('the export draws the red box where the preview did, as wide', exported[1] is not None and
            abs(exported[1][0] - shown[1][0]) < 0.02 and abs(exported[1][1] - shown[1][1]) < 0.02 and abs(exported[2] - shown[2]) < 0.03,
            {'preview': shown, 'export': exported})

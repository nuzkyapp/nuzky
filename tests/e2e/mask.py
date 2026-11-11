"""Masks. The Mask section shows a vertical talking head only inside a circle in the middle; Invert shows only
around it; keyframes slide the circle to the right over time, typed into Position X at a later playhead; the preview
draws the mask's edge, a click where the mask hides the picture selects nothing, and the export shows the same
frame as the renderer."""
import json, subprocess, time

from PIL import Image, ImageStat

from e2e.harness import CLI, FIXTURES, changed_share, export, flow, preview_crop, preview_rect, press, wait, webdriver

AT_US = 1_000_000
LATER_US = 2_500_000
SECTION = "aside[aria-label=Inspector]"
MASK = f"{SECTION} [role=group][aria-label='Mask']"
# Parts of the preview, in fractions of its width and height.
MIDDLE = (0.46, 0.47, 0.54, 0.53)
CORNER = (0.0, 0.0, 0.12, 0.07)
RIGHT = (0.77, 0.47, 0.83, 0.53)


def clip(r):
    return r.s.run('return window.__nuzky.store.getState().snap.project.tracks[0].clips[0]', retries=3)


def mask(r):
    return clip(r)['content']['transform'].get('mask')


def select(r, ids):
    r.s.run('window.__nuzky.store.getState().select(arguments[0])', ids)


def box_px(image, box):
    w, h = image.size
    return (int(box[0] * w), int(box[1] * h), int(box[2] * w), int(box[3] * h))


def brightness(image, box):
    return ImageStat.Stat(image.crop(box_px(image, box)).convert('L')).mean[0]


def preview(r, name, ready):
    """The preview once `ready` says the picture shows the last edit, or as it is after 10 s."""
    rect = preview_rect(r)

    def shot():
        r.shot(name)
        crop = preview_crop(r.work / f'{name}.png', rect)
        return crop if ready(crop) else None
    return wait(shot, 10, 0.5) or preview_crop(r.work / f'{name}.png', rect)


def engine_frame(r, us, name):
    path = r.work / f'{name}.png'
    subprocess.run([str(CLI), 'frame', str(r.saved_project()), f'{us / 1e6:.6f}', str(path), '720'], env=r.env, check=True,
                   capture_output=True)
    return path


def click_preview(r, fx, fy):
    """Presses and releases the pointer at a point of the preview."""
    return r.s.run("""const o = document.querySelector('[data-testid=layer-overlay]'); if (!o) return false;
        const b = o.getBoundingClientRect(); const x = b.left + b.width * arguments[0], y = b.top + b.height * arguments[1];
        const at = (type) => new PointerEvent(type, {bubbles: true, button: 0, clientX: x, clientY: y, pointerId: 1});
        o.dispatchEvent(at('pointerdown')); window.dispatchEvent(at('pointerup')); return true;""", fx, fy)


@flow('mask', 'A circle mask shows a talking head only in the middle, Invert only around it, keyframes slide it right, '
      'the preview draws its edge, a click where it hides the picture selects nothing, and the export matches')
def masks(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    talk = clip(r)
    r.seek(AT_US)
    select(r, [])
    drawn = lambda: r.s.run("return document.querySelector('section[aria-label=Preview] canvas').dataset.us")
    if not r.check('the preview draws the frame at the playhead', wait(lambda: drawn() == str(AT_US), 10), drawn()):
        return
    plain = preview(r, 'plain', lambda c: brightness(c, CORNER) > 40)
    r.check('without a mask the whole picture shows, corners included', brightness(plain, CORNER) > 40, brightness(plain, CORNER))
    saved = wait(lambda: r.saved_project().exists() and json.loads(r.saved_project().read_text()), 5) or {}
    r.check('a project without a mask saves no mask', saved and 'mask' not in json.dumps(saved), saved.get('tracks'))

    # Circle in the Mask section: round, half the picture's shorter side across, in the middle.
    select(r, [talk['id']])
    if not r.check('the inspector has a Mask section', wait(lambda: r.s.run(f'return !!document.querySelector("{MASK}")'), 5)):
        r.shot('inspector-missing')
        return
    r.s.run(f'document.querySelector("{MASK} [data-seg=circle]").click()')
    circle = wait(lambda: (m := mask(r)) and m['kind'] == 'circle' and m, 5)
    r.check('Circle puts a round circle half the shorter side across in the middle',
            circle and abs(circle['width'] - 0.5) < 1e-6 and abs(circle['height'] - 0.28125) < 1e-6 and circle['x'] == 0 and circle['y'] == 0, circle)
    r.check("the preview draws the mask's edge over the selected clip",
            wait(lambda: r.s.run('return !!document.querySelector(\'[data-mask-edge="circle"]\')'), 5))
    time.sleep(0.5)
    r.shot('mask-circle-selected')
    select(r, [])
    shown = preview(r, 'mask-circle', lambda c: brightness(c, CORNER) < 12)
    r.check('the preview shows the talking head only inside the circle',
            brightness(shown, CORNER) < 12 and changed_share(shown.crop(box_px(shown, MIDDLE)), plain.crop(box_px(plain, MIDDLE))) < 0.02,
            {'corner': brightness(shown, CORNER), 'middle': brightness(shown, MIDDLE)})

    # A click where the mask hides the picture selects nothing; one inside the circle selects the clip.
    select(r, [talk['id']])
    click_preview(r, 0.05, 0.05)
    r.check('a click where the mask hides the picture selects nothing',
            wait(lambda: r.s.run('return window.__nuzky.store.getState().selection') == [], 5))
    click_preview(r, 0.5, 0.5)
    r.check('a click inside the circle selects the clip',
            wait(lambda: r.s.run('return window.__nuzky.store.getState().selection') == [talk['id']], 5))

    # Invert shows the picture only around the circle; a second click shows it inside again.
    invert = f"[...document.querySelectorAll('{SECTION} label')].find((l) => l.textContent.trim() === 'Invert')?.querySelector('input')"
    wait(lambda: r.s.run(f'return !!({invert})'), 5)
    r.s.run(f'{invert}.click()')
    r.check('Invert turns the mask inside out', wait(lambda: (mask(r) or {}).get('invert') is True, 5), mask(r))
    select(r, [])
    inverted = preview(r, 'mask-inverted', lambda c: brightness(c, MIDDLE) < 12)
    r.check('the preview shows the talking head only around the circle',
            brightness(inverted, MIDDLE) < 12 and brightness(inverted, CORNER) > 40,
            {'corner': brightness(inverted, CORNER), 'middle': brightness(inverted, MIDDLE)})
    select(r, [talk['id']])
    wait(lambda: r.s.run(f'return !!({invert})'), 5)
    r.s.run(f'{invert}.click()')
    r.check('Invert again shows the inside', wait(lambda: mask(r) and not mask(r).get('invert'), 5), mask(r))

    # Keyframes: one at the playhead with the circle in the middle, then Position X typed 30 at a later playhead.
    add = f"{SECTION} button[aria-label='Add keyframe at playhead']"
    wait(lambda: r.s.run(f'return !!document.querySelector("{add}")'), 5)
    r.s.run(f'document.querySelector("{add}").click()')
    r.check('the diamond adds a keyframe with the mask', wait(lambda: len(clip(r)['keyframes']) == 1, 5), clip(r)['keyframes'])
    r.seek(LATER_US)
    wait(lambda: drawn() == str(LATER_US), 10)
    field = f"document.querySelector('{SECTION} input[aria-label=\"Mask position X value\"]')"
    wait(lambda: r.s.run(f'const f = {field}; if (!f) return false; f.focus(); f.select(); return true'), 5)
    time.sleep(0.3)
    for key in '30':
        press(key)
    press('Return')
    keys = wait(lambda: len((k := clip(r)['keyframes'])) == 2 and k, 5) or clip(r)['keyframes']
    moved = [round(k['transform'].get('mask', {}).get('x', -1), 3) for k in keys]
    r.check('typing 30 into Position X writes a second keyframe with the circle moved right', moved == [0, 0.3], moved)
    r.shot('mask-keyframes-inspector')

    select(r, [])
    later = preview(r, 'mask-later', lambda c: brightness(c, RIGHT) > 40 and brightness(c, MIDDLE) < 12)
    r.check('later the circle shows the right of the picture and hides the middle',
            brightness(later, RIGHT) > 40 and brightness(later, MIDDLE) < 12,
            {'middle': brightness(later, MIDDLE), 'right': brightness(later, RIGHT)})
    r.seek(AT_US)
    earlier = preview(r, 'mask-earlier', lambda c: brightness(c, MIDDLE) > 40 and brightness(c, RIGHT) < 12)
    r.check('at the first keyframe the circle is still in the middle',
            brightness(earlier, MIDDLE) > 40 and brightness(earlier, RIGHT) < 12,
            {'middle': brightness(earlier, MIDDLE), 'right': brightness(earlier, RIGHT)})

    # The export shows the frame the renderer draws for the preview.
    time.sleep(1.5)
    engine = engine_frame(r, LATER_US, 'engine-later')
    out = export(r, 'mask.mp4')
    exported = r.work / 'export-later.png'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-i', str(out), '-vf', f'select=eq(n\\,{LATER_US * 30 // 1_000_000})',
                    '-frames:v', '1', '-update', '1', str(exported)], check=True)
    r.shots.append(exported.name)
    changed = changed_share(engine, exported)
    r.check('the exported frame matches the frame the renderer draws for the preview', changed < 0.002, f'{changed:.3%} of pixels differ')
    out_image = Image.open(exported).convert('RGB')
    r.check('the export shows the moved circle', brightness(out_image, RIGHT) > 40 and brightness(out_image, MIDDLE) < 12,
            {'middle': brightness(out_image, MIDDLE), 'right': brightness(out_image, RIGHT)})

    # None removes the mask from the whole clip, keyframes included, and one undo brings it back.
    select(r, [talk['id']])
    wait(lambda: r.s.run(f'return !!document.querySelector("{MASK}")'), 5)
    r.s.run(f'document.querySelector("{MASK} [data-seg=none]").click()')
    gone = lambda: not mask(r) and all('mask' not in k['transform'] for k in clip(r)['keyframes'])
    r.check('None removes the mask, every keyframe included', wait(gone, 5), clip(r)['keyframes'])
    r.key('z', ctrlKey=True)
    r.check('one undo brings the mask back', wait(lambda: len([k for k in clip(r)['keyframes'] if 'mask' in k['transform']]) == 2, 5))
    r.check('no error toast', not r.errors(), r.errors())

    # Proof of the look: the section in the largest and the smallest window.
    r.seek(AT_US)
    show = f'document.querySelector("{MASK}").closest("section").scrollIntoView({{block: "center"}})'
    for width, height, name in ((1440, 900, 'look'), (1024, 640, 'look-1024')):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        r.s.run(show)
        time.sleep(1)
        r.shot(name)

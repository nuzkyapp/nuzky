import hashlib, json, subprocess, time

from e2e.harness import CLI, ROOT, export, flow, preview_crop, preview_rect, press, wait, webdriver

WIDTH, HEIGHT = 360, 640
PHOTO = ROOT / 'site/public/media/ugc.jpg'
CLIP = "return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips).find((c) => c.id === arguments[0]).content"
TAB = ("const t = [...document.querySelectorAll(arguments[0] + ' [role=tab]')].find((t) => t.textContent.trim() === arguments[1]);"
       'if (!t) return false; t.click(); return true')
SECTION = "document.querySelector('aside[data-tour=library] section[aria-label=\"' + arguments[0] + '\"]')"
# The labels of the tiles a section of the Filters tab shows.
TILES = f"const s = {SECTION}; return s ? [...s.querySelectorAll('button[aria-pressed]')].map((b) => b.textContent.trim()) : null"
CLICK = f"""const b = [...{SECTION}.querySelectorAll('button[aria-pressed]')].find((b) => b.textContent.trim() === arguments[1]);
if (!b) return false; b.click(); return true"""
PRESSED = f"""const b = [...{SECTION}.querySelectorAll('button[aria-pressed]')].find((b) => b.textContent.trim() === arguments[1]);
return b ? b.getAttribute('aria-pressed') === 'true' : null"""
SHOW_ALL = f"const b = [...{SECTION}.querySelectorAll('button')].find((b) => /^Show (all|fewer)/.test(b.textContent.trim())); b.click(); return b.textContent.trim()"
ALERT = f"const a = {SECTION}.querySelector('[role=alert]'); return a ? a.textContent : null"
INSPECTOR = "const s = [...document.querySelectorAll('aside[aria-label=Inspector] section')].find((s) => s.querySelector('h3')?.textContent === 'Colour'); return s ? s.innerText : null"
TOGGLE = """const b = [...document.querySelectorAll('section[aria-label=Preview] button')].find((b) => b.querySelector('.lucide-square-split-horizontal'));
if (!b) return null; if (arguments[0]) b.click(); return b.getAttribute('aria-pressed') === 'true'"""
# The tile's picture, inside its border.
TILE_RECT = f"""const b = [...{SECTION}.querySelectorAll('button[aria-pressed]')].find((b) => b.textContent.trim() === arguments[1]).querySelector('span').getBoundingClientRect();
return {{left: b.left + 3, top: b.top + 3, width: b.width - 6, height: b.height - 6, viewport: window.innerWidth}};"""


def apple_log_video(work):
    """The portrait as an iPhone records it in Apple Log: its light (the photo through the inverse BT.709 camera curve)
    in BT.2020, Apple Log encoded, ProRes 422 HQ 10-bit with BT.2020 tags, limited range and no transfer named."""
    import numpy as np
    from PIL import Image
    photo = np.asarray(Image.open(PHOTO).convert('RGB').resize((WIDTH, HEIGHT)), dtype=np.float64) / 255
    light = np.where(photo < 0.081, photo / 4.5, ((photo + 0.099) / 1.099) ** (1 / 0.45))
    to_2020 = np.array([[0.627404, 0.329283, 0.043313], [0.069097, 0.919540, 0.011362], [0.016391, 0.088013, 0.895595]])
    light = np.clip(light @ to_2020.T, 0, None)
    r0, rt, sigma, beta, gamma, delta = -0.05641088, 0.01, 47.28711236, 0.00964052, 0.08550479, 0.69336945
    log = np.where(light >= rt, gamma * np.log2(light + beta) + delta, sigma * (light - r0) ** 2)
    # 16-bit RGB as raw samples, which FFmpeg reads as rgb48le.
    raw = work / 'apple-log.rgb48'
    raw.write_bytes((np.clip(log, 0, 1) * 65535).round().astype('<u2').tobytes())
    out = work / 'IMG_4821.MOV'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'rawvideo', '-pix_fmt', 'rgb48le', '-s', f'{WIDTH}x{HEIGHT}', '-framerate', '30',
                    '-stream_loop', '-1', '-i', str(raw), '-t', '3', '-vf', 'scale=out_color_matrix=bt2020:out_range=tv:flags=accurate_rnd+full_chroma_int',
                    '-c:v', 'prores_ks', '-profile:v', '3', '-pix_fmt', 'yuv422p10le', '-colorspace', 'bt2020nc',
                    '-color_primaries', 'bt2020', '-color_range', 'tv', str(out)], check=True)
    return out


def cube(path, size, colour):
    lines = [f'TITLE "{path.stem}"', f'LUT_3D_SIZE {size}']
    for b in range(size):
        for g in range(size):
            for r in range(size):
                lines.append(' '.join(f'{v:.6f}' for v in colour(r / (size - 1), g / (size - 1), b / (size - 1))))
    path.write_text('\n'.join(lines) + '\n')
    return path


def stats(crop):
    """Mean saturation (max minus min channel) and the spread of brightness, which a flat log picture lacks."""
    pixels = list(crop.convert('RGB').getdata())
    luma = [0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2] for p in pixels]
    mean = sum(luma) / len(luma)
    spread = (sum((y - mean) ** 2 for y in luma) / len(luma)) ** 0.5
    return {'saturation': round(sum(max(p) - min(p) for p in pixels) / len(pixels), 1), 'contrast': round(spread, 1)}


def difference(a, b):
    a, b = a.convert('RGB'), b.convert('RGB').resize(a.size)
    total = sum(abs(x - y) for p, q in zip(a.getdata(), b.getdata()) for x, y in zip(p, q))
    return total / (a.width * a.height * 3)


@flow('log-lut', 'An iPhone clip recorded in Apple Log turns from flat grey into normal colour with one click on its tile '
      'in Filters, which splits the clip as recorded and converted; a .cube LUT imported there goes over it at the '
      'strength the inspector sets, a broken .cube says why and changes nothing, Compare and the export show the same, '
      'and each change is one undo step')
def log_lut(r):
    from PIL import Image
    video = apple_log_video(r.work)
    r.import_media(video)
    r.add_clip(video.name)
    clip = r.track()[0]
    middle = clip['startUs'] + clip['durationUs'] // 2
    r.seek(middle)
    r.focus_clip(clip['id'])
    wait(lambda: r.s.run(TAB, '[role=tablist][aria-label=Library]', 'Filters'), 5)
    wait(lambda: r.s.run(TAB, 'aside[aria-label=Inspector]', 'Adjust'), 5)
    content = lambda: r.s.run(CLIP, clip['id'])
    time.sleep(2)
    rect = preview_rect(r)

    def preview(name, done=lambda crop: True, timeout=8):
        """The preview's picture once `done(crop)` holds, so it follows the last change."""
        end = time.time() + timeout
        while True:
            r.shot(name)
            crop = preview_crop(r.work / f'{name}.png', rect)
            if done(crop) or time.time() > end:
                return crop
            time.sleep(0.4)

    flat = stats(preview('log-flat'))
    shown = wait(lambda: r.s.run(TILES, 'Log footage'), 5)
    r.check('Log footage offers the phones and common cameras, with Show all for the rest',
            shown == ['Apple Log', 'Apple Log 2', 'Xiaomi Log', 'S-Log3 Cine', 'Canon Log 3', 'V-Log'], shown)

    # The tile's bottom right half is the clip converted, the top left half as recorded.
    def tile_halves(name, section, label):
        r.shot(name)
        crop = preview_crop(r.work / f'{name}.png', r.s.run(TILE_RECT, section, label))
        w, h = crop.size
        px = crop.load()
        upper = [px[x, y] for y in range(h) for x in range(w) if x / w + y / h < 0.75]
        lower = [px[x, y] for y in range(h) for x in range(w) if x / w + y / h > 1.25]
        sat = lambda pixels: sum(max(p) - min(p) for p in pixels) / len(pixels)
        return sat(upper), sat(lower)

    halves = wait(lambda: (h := tile_halves('log-tiles', 'Log footage', 'Apple Log')) and h[1] > h[0] + 8 and h, 10) or tile_halves('log-tiles', 'Log footage', 'Apple Log')
    r.check('the Apple Log tile shows the clip flat top left and converted bottom right', halves[1] > halves[0] + 8, halves)

    r.s.run(CLICK, 'Log footage', 'Apple Log')
    r.check('one click on Apple Log converts the clip', wait(lambda: content().get('log') == 'appleLog', 5), content().get('log'))
    r.check('and marks its tile', wait(lambda: r.s.run(PRESSED, 'Log footage', 'Apple Log'), 5))
    converted_crop = preview('log-converted', lambda c: stats(c)['saturation'] > flat['saturation'] * 1.6)
    converted = stats(converted_crop)
    r.check('the preview turns from flat grey into normal colour and contrast',
            converted['saturation'] > flat['saturation'] * 1.6 and converted['contrast'] > flat['contrast'] * 1.3, [flat, converted])
    original = stats(Image.open(PHOTO).convert('RGB').resize((WIDTH, HEIGHT)))
    r.check('close to the photo the log was made from', abs(converted['saturation'] - original['saturation']) < original['saturation'] * 0.35
            and abs(converted['contrast'] - original['contrast']) < original['contrast'] * 0.25, [converted, original])
    colour = wait(lambda: r.s.run(INSPECTOR), 5)
    r.check('the inspector says the clip is converted from Apple Log, recorded on which iPhones',
            colour and 'Apple Log' in colour and 'iPhone 15 Pro' in colour, colour)
    r.shot('log-inspector')

    r.check('Show all lists the 17 profiles by maker', r.s.run(SHOW_ALL, 'Log footage') == 'Show all 17'
            and wait(lambda: len(r.s.run(TILES, 'Log footage') or []) == 17, 5), r.s.run(TILES, 'Log footage'))
    time.sleep(1.5)
    r.shot('log-all')
    r.s.run(SHOW_ALL, 'Log footage')

    broken = cube(r.work / 'Moody Night.cube', 33, lambda *rgb: rgb)
    broken.write_text('\n'.join(broken.read_text().splitlines()[:1202]) + '\n')
    r.s.call('window.__nuzky.importLutPaths(arguments[0])', [str(broken)])
    alert = wait(lambda: r.s.run(ALERT, 'LUTs'), 10)
    r.check('a broken .cube says why in Filters', alert and 'Moody Night.cube' in alert and 'cut short or damaged' in alert, alert)
    r.check('and changes neither the library nor the clip', r.s.run('return window.__nuzky.luts.getState().list.length') == 0
            and content().get('lut') is None, content().get('lut'))
    r.s.run(f"{SECTION.replace('arguments[0]', repr('LUTs'))}.scrollIntoView({{block: 'center'}})")
    time.sleep(0.5)
    r.shot('lut-error')

    warm = cube(r.work / 'Warm Film.cube', 17, lambda red, green, blue: (red ** 0.8, green * 0.95, blue * blue))
    r.s.call('window.__nuzky.importLutPaths(arguments[0])', [str(warm)])
    lut = wait(lambda: (l := content().get('lut')) and l.get('type') == 'file' and l, 10)
    r.check('an imported .cube goes on the selected clip at full strength', lut and lut['name'] == 'Warm Film' and lut['strength'] == 1, lut)
    r.check('the error goes once a LUT reads', not r.s.run(ALERT, 'LUTs'), r.s.run(ALERT, 'LUTs'))
    library = r.work / 'data/nuzky/luts'
    copies = sorted(p.name for p in library.glob('*.cube'))
    r.check("Nuzky keeps its own copy, named by the file's hash", copies == [f"{lut['id']}.cube"], [library, copies])
    graded = stats(preview('lut-full', lambda c: abs(stats(c)['saturation'] - converted['saturation']) > 2))
    r.check('the preview shows the LUT over the converted picture', abs(graded['saturation'] - converted['saturation']) > 2, [converted, graded])

    field = "document.querySelector('aside[aria-label=Inspector] input[aria-label=\"Warm Film strength value\"]')"
    wait(lambda: r.s.run(f'const f = {field}; if (!f) return false; f.focus(); f.select(); return true'), 5)
    time.sleep(0.3)
    for key in '50':
        press(key)
    press('Return')
    r.check('typing 50 into Strength mixes the LUT at half', wait(lambda: content()['lut']['strength'] == 0.5, 5), content()['lut'])
    half_crop = preview('lut-half', lambda c: abs(stats(c)['saturation'] - graded['saturation']) > 1)
    r.shot('lut-inspector')

    saved = wait(lambda: next((c['content'] for t in json.loads(r.saved_project().read_text())['tracks'] for c in t['clips']), {}).get('lut', {}).get('strength') == 0.5, 10)
    r.check('the project saves the profile and the LUT', saved)
    rendered_png = r.work / 'rendered.png'
    subprocess.run([str(CLI), 'frame', str(r.saved_project()), str(middle / 1e6), str(rendered_png), str(WIDTH)],
                   env=r.env, check=True, capture_output=True)
    rendered = Image.open(rendered_png)
    off = difference(rendered, half_crop)
    r.check('the preview shows what the renderer the export uses draws', off <= 6, f'{off:.1f} levels apart')

    exported = r.work / 'exported.png'
    video_out = export(r, 'log-lut.mp4')
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-ss', str(middle / 1e6), '-i', str(video_out), '-frames:v', '1', str(exported)], check=True)
    off = difference(Image.open(exported), rendered)
    r.check('the export shows the profile and the LUT as the frame does', off <= 4, f'{off:.1f} levels apart')

    r.s.run(TOGGLE, True)
    r.check('Compare turns on for a converted clip', wait(lambda: r.s.run(TOGGLE, False), 5))
    sides = lambda crop: [stats(crop.crop((0, 0, int(crop.width * 0.45), crop.height))), stats(crop.crop((int(crop.width * 0.55), 0, crop.width, crop.height)))]
    left, right = sides(preview('log-compare', lambda c: (h := sides(c))[1]['saturation'] > h[0]['saturation'] * 1.5))
    r.check('left of the line the clip shows as recorded, right of it converted', right['saturation'] > left['saturation'] * 1.5, [left, right])
    r.s.run(TOGGLE, True)
    r.check('Compare turns off', wait(lambda: r.s.run(TOGGLE, False) is False, 5))
    whole = sides(preview('log-compare-off', lambda c: sides(c)[0]['saturation'] > left['saturation'] * 1.5))
    r.check('and the whole picture is converted again', whole[0]['saturation'] > left['saturation'] * 1.5, [left, whole[0]])

    elsewhere = {'type': 'file', 'id': 'ffffffffffffffff', 'name': 'Teal Orange', 'strength': 1}
    r.s.call("window.__nuzky.store.getState().edit({type: 'updateClip', clipId: arguments[0], lut: arguments[1]})", clip['id'], elsewhere)
    missing = wait(lambda: (t := r.s.run(INSPECTOR)) and "isn't on this computer" in t and t, 5)
    r.check('a LUT the project names but this computer lacks says so at the clip', missing, r.s.run(INSPECTOR))
    without = preview('lut-missing', lambda c: difference(c, converted_crop) < 3)
    r.check('and the clip plays without it, as its profile makes it', difference(without, converted_crop) < 3, f'{difference(without, converted_crop):.1f} levels from Apple Log alone')

    steps = [('the LUT from elsewhere', lambda c: c['lut']['id'] == lut['id']), ('the strength', lambda c: c['lut']['strength'] == 1),
             ('the LUT', lambda c: c.get('lut') is None), ('Apple Log', lambda c: c.get('log') is None)]
    for what, back in steps:
        r.key('z', ctrlKey=True)
        r.check(f'one Undo takes back {what}', wait(lambda: back(content()), 5), content())

    # A project that names a LUT this computer lacks shows it as soon as the user imports it, also while paused and
    # with nothing selected, which changes no clip.
    teal = cube(r.work / 'Teal Orange.cube', 9, lambda red, green, blue: (red * 0.9, green, min(1, blue * 1.1 + 0.05)))
    teal_id = hashlib.sha256(teal.read_bytes()).hexdigest()[:16]
    named = {'type': 'file', 'id': teal_id, 'name': 'Teal Orange', 'strength': 1}
    r.s.call("window.__nuzky.store.getState().edit({type: 'updateClip', clipId: arguments[0], log: 'appleLog', lut: arguments[1]})", clip['id'], named)
    missing = preview('teal-missing', lambda c: difference(c, converted_crop) < 3)
    r.check('a LUT from elsewhere draws nothing until it is imported', difference(missing, converted_crop) < 3, f'{difference(missing, converted_crop):.1f}')
    r.s.run('window.__nuzky.store.getState().select([])')
    r.s.call('window.__nuzky.importLutPaths(arguments[0])', [str(teal)])
    found = preview('teal-imported', lambda c: difference(c, converted_crop) > 3)
    r.check('importing it shows it on the paused preview at once', difference(found, converted_crop) > 3, f'{difference(found, converted_crop):.1f}')
    r.check('and changes no clip', r.s.run(CLIP, clip['id'])['lut'] == named, r.s.run(CLIP, clip['id'])['lut'])
    r.focus_clip(clip['id'])

    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        time.sleep(1)
        r.shot(f'filters-window{suffix}')
    r.check('nothing shows an error', not r.errors(), r.errors())

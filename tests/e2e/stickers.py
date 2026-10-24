"""Emoji and stickers in the Text panel. Searching "fire" finds 🔥, a click puts it at the playhead on a track of its
own over the video, centred, as one undo step, and the export shows it where the preview does; the same emoji
again reuses its picture, a skin tone picks the toned emoji, Nuzky's own arrow comes from the Stickers tab, the AI
lock stops adding but not searching, and an agent adds an emoji with add_sticker."""
import json, subprocess, time

import numpy as np
from PIL import Image

from e2e.harness import CLI, FIXTURES, Bridge, changed_share, flow, preview_crop, preview_rect, wait
from e2e.layout import resize

AT_US = 1_000_000
PROJECT = 'return window.__nuzky.store.getState().snap.project'
CLICK = "const el = document.querySelector(arguments[0]); if (!el) return false; el.click(); return true;"
TYPE = """const input = document.querySelector(`input[aria-label="${arguments[0]}"]`); input.focus();
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, arguments[1]);
input.dispatchEvent(new Event('input', {bubbles: true}));"""
FIRE = '1f525'


def project(r):
    return r.s.run(PROJECT, retries=3)


def stickers(r):
    """The project's assets drawn from the sticker library, by name."""
    return {a['name']: a for a in project(r)['assets'] if '/nuzky/stickers/' in a['path'].replace('\\', '/')}


def clips_of(r, asset_id):
    return [(i, c) for i, t in enumerate(project(r)['tracks']) for c in t['clips'] if c['content'].get('assetId') == asset_id]


def select(r, ids):
    r.s.run('window.__nuzky.store.getState().select(arguments[0])', ids)


def bounds(r, clip_id, at_us):
    found = r.s.call(f'window.__nuzky.api.layerBounds({at_us}).then((b) => JSON.stringify(b))')
    return next((b for b in json.loads(found['value']) if b['clipId'] == clip_id), None) if found['ok'] else None


def orange(image, box):
    """Share of the box in the flame's orange; the colour bars under it have none."""
    a = np.asarray(image.convert('RGB').crop(tuple(round(v) for v in box)), dtype=int)
    return float(((a[..., 0] > 200) & (a[..., 1] > 60) & (a[..., 1] < 190) & (a[..., 2] < 90)).mean())


def engine_frame(r, name, at_us):
    path = r.work / f'{name}.png'
    subprocess.run([str(CLI), 'frame', str(r.saved_project()), f'{at_us / 1e6:.6f}', str(path), '720'], env=r.env, check=True,
                   capture_output=True)
    return path


def emoji_cell(key):
    return f'[data-emoji="{key}"]'


@flow('stickers', 'Searching the emoji for "fire" and clicking 🔥 puts it centred over the video at the playhead as one '
      'undo step; the export draws it where the preview does; skin tones, Nuzky\'s own stickers, the AI lock and '
      'add_sticker for agents')
def stickers_flow(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    canvas = project(r)['canvas']
    main = [c['id'] for c in project(r)['tracks'][0]['clips']]
    r.seek(AT_US)
    select(r, [])
    time.sleep(1.5)  # `nuzky frame` reads the saved file, written about a second after an edit.
    plain = engine_frame(r, 'engine-plain', AT_US)

    r.s.run(CLICK, '[role=tablist][aria-label=Library] [data-tab="text"]')
    r.s.run(CLICK, '#text-tab-emoji')
    groups = wait(lambda: r.s.run("return [...document.querySelectorAll('[data-group]')].map((g) => g.dataset.group)"), 10) or []
    r.check('the Emoji tab lists the emoji in their nine groups', len(groups) == 9 and groups[0] == 'Smileys and emotions', groups)
    drawn = wait(lambda: r.s.run("return [...document.querySelectorAll('[data-emoji] img')].filter((i) => i.src.startsWith('data:image/png')).length") >= 40, 10)
    r.check('the first group\'s pictures are drawn', drawn)
    r.s.run(TYPE, 'Search emoji', 'fire')
    found = wait(lambda: (k := r.s.run("return [...document.querySelectorAll('[data-emoji]')].map((b) => b.dataset.emoji)")) and k[0] == FIRE and k, 5)
    r.check('searching "fire" puts 🔥 first and finds the fire engine too', found and '1f692' in found, found)
    r.check('the picture of 🔥 is drawn', wait(lambda: r.s.run(f"return document.querySelector('{emoji_cell(FIRE)} img')?.src.startsWith('data:image/png') ?? false"), 5))
    r.shot('emoji-search')

    started = time.time()
    r.s.run(CLICK, emoji_cell(FIRE))
    fire = wait(lambda: stickers(r).get('Fire'), 10)
    took = time.time() - started
    if not r.check('a click adds 🔥 as an image from the sticker library', fire and fire['kind'] == 'image', stickers(r)):
        return
    r.check('it is drawn at 1024 px', (fire['width'], fire['height']) == (1024, 1024), (fire['width'], fire['height']))
    placed = clips_of(r, fire['id'])
    index, clip = placed[0] if placed else (None, None)
    r.check('it lands on a track over the main track, at the playhead, for 3 s',
            index == 1 and (clip['startUs'], clip['durationUs']) == (AT_US, 3_000_000), placed)
    r.check('centred, its longer side 30 % of the canvas\'s shorter side, with no shape',
            all(abs(clip['content']['transform'][f] - v) < 1e-6 for f, v in (('x', 0), ('y', 0), ('scale', 0.3), ('rotation', 0), ('opacity', 1)))
            and not clip['content']['transform'].get('crop') and not clip['content'].get('shape'),
            clip['content'])
    r.check('the main track is untouched', [c['id'] for c in project(r)['tracks'][0]['clips']] == main)
    r.check('the new clip is selected', r.s.run('return window.__nuzky.store.getState().selection') == [clip['id']])
    r.check('adding it takes under a second', took < 1, f'{took:.2f} s')

    # Typing in the search field kept the focus there, where Ctrl+Z undoes the typing; the editor's undo is outside it.
    r.s.run('document.activeElement.blur()')
    r.key('z', ctrlKey=True)
    r.check('one undo removes the emoji, its track and its asset', wait(lambda: not stickers(r) and len(project(r)['tracks']) == 1, 5),
            (list(stickers(r)), len(project(r)['tracks'])))
    r.key('z', ctrlKey=True, shiftKey=True)
    r.check('redo brings it back', wait(lambda: clips_of(r, fire['id']), 5))

    # The preview and the export draw it where the engine says it is.
    select(r, [])
    time.sleep(1.5)
    r.shot('preview-fire')
    shown = preview_crop(r.work / 'preview-fire.png', preview_rect(r))
    box = bounds(r, clip['id'], AT_US)
    k = shown.width / canvas['width']
    xs, ys = [p[0] * k for p in box['corners']], [p[1] * k for p in box['corners']]
    r.check('the preview shows the flame in the middle of the frame', orange(shown, (min(xs), min(ys), max(xs), max(ys))) > 0.15,
            round(orange(shown, (min(xs), min(ys), max(xs), max(ys))), 3))

    r.seek(2_500_000)
    r.s.run(CLICK, emoji_cell(FIRE))
    r.check('🔥 again adds a second clip of the same picture', wait(lambda: len(clips_of(r, fire['id'])) == 2, 10) and len(stickers(r)) == 1,
            stickers(r))

    # A skin tone.
    r.s.run(TYPE, 'Search emoji', 'thumbs up')
    r.s.run(CLICK, 'button[aria-label^="Skin tone"]')
    picked = r.s.run("const b = [...document.querySelectorAll('[data-menu] [role^=menuitem]')].find((b) => b.textContent.trim() === 'Medium');"
                     "if (!b) return false; b.click(); return true;")
    r.check('the skin tone menu offers Medium', picked)
    r.check('the thumbs up shows in the medium tone', wait(lambda: r.s.run("return !!document.querySelector('[data-emoji=\"1f44d_1f3fd\"]')"), 5))
    r.s.run(CLICK, '[data-emoji="1f44d_1f3fd"]')
    r.check('it adds the thumbs up in that tone', wait(lambda: 'Thumbs up: medium skin tone' in stickers(r), 10), list(stickers(r)))
    r.check('the tone is remembered', r.s.run("return localStorage.getItem('nuzky.skinTone')") == '3')
    r.s.run(TYPE, 'Search emoji', 'woman shrugging')
    r.check('gendered emoji are there too, in the chosen tone',
            wait(lambda: r.s.run("return document.querySelector('[data-emoji]')?.dataset.emoji") == '1f937_1f3fd_200d_2640', 5),
            r.s.run("return [...document.querySelectorAll('[data-emoji]')].map((b) => b.dataset.emoji)"))

    # Undo pressed while a new picture is still being drawn waits for it: it takes the emoji back, not the edit before.
    r.s.run(CLICK, '#text-tab-text')
    r.s.run(CLICK, 'button[title$="text at the playhead"]')
    texts = lambda: [c for t in project(r)['tracks'] for c in t['clips'] if c['content'].get('type') == 'text']
    r.check('a text preset adds a text clip', wait(lambda: len(texts()) == 1, 5))
    r.s.run(CLICK, '#text-tab-emoji')
    r.s.run(TYPE, 'Search emoji', 'party popper')
    wait(lambda: r.s.run("return !!document.querySelector('[data-emoji=\"1f389\"]')"), 5)
    r.s.run("""document.querySelector('[data-emoji="1f389"]').click(); document.activeElement.blur();
        document.body.dispatchEvent(new KeyboardEvent('keydown', {key: 'z', ctrlKey: true, bubbles: true, cancelable: true}));""")
    time.sleep(2)
    r.check('Undo right after a click takes the new emoji back and keeps the text added before it',
            'Party popper' not in stickers(r) and len(texts()) == 1, (list(stickers(r)), len(texts())))
    r.key('z', ctrlKey=True, shiftKey=True)
    r.check('Redo brings the emoji back', wait(lambda: 'Party popper' in stickers(r), 5), list(stickers(r)))

    # Nuzky's own sticker.
    r.s.run(CLICK, '#text-tab-stickers')
    r.check('the Stickers tab draws Nuzky\'s stickers',
            wait(lambda: r.s.run("return [...document.querySelectorAll('[data-sticker] img')].length") == 12, 10),
            r.s.run("return [...document.querySelectorAll('[data-sticker]')].map((b) => b.dataset.sticker)"))
    r.shot('stickers-tab')
    r.s.run(CLICK, '[data-sticker="arrow"]')
    arrow = wait(lambda: stickers(r).get('Arrow'), 10)
    r.check('a click adds the arrow over the video', arrow and all(i > 0 for i, _ in clips_of(r, arrow['id'])), arrow)

    # Under the AI lock nothing is added, but searching still works.
    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Add a sticker'})['run_id']
        r.check('the app shows the AI lock', wait(lambda: r.state()['aiRun'], 5))
        r.s.run(CLICK, '#text-tab-emoji')
        r.s.run(TYPE, 'Search emoji', 'star')
        r.check('searching still works', wait(lambda: r.s.run("return !!document.querySelector('[data-emoji=\"2b50\"]')"), 5))
        r.check('the emoji are disabled with the reason', r.s.run(
            "const b = document.querySelector('[data-emoji=\"2b50\"]'); return b.getAttribute('aria-disabled') === 'true' && !!b.title"))
        count = len(project(r)['assets'])
        r.s.run(CLICK, '[data-emoji="2b50"]')
        time.sleep(1)
        r.check('a click adds nothing', len(project(r)['assets']) == count)
        agent = bridge.call('add_sticker', {'run_id': run, 'sticker': '🔥', 'at_us': 4_000_000})
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
        r.check('add_sticker puts 🔥 at 4 s with the picture the project has', agent['asset_id'] == fire['id'] and agent['start_us'] == 4_000_000, agent)
        r.check('the app shows the agent\'s emoji at once', wait(lambda: len(clips_of(r, fire['id'])) == 3, 5))
    finally:
        bridge.close()

    # The export draws the flame where the preview does.
    time.sleep(1.5)
    engine = engine_frame(r, 'engine-fire', AT_US)
    target = r.work / 'stickers.mp4'
    started = r.s.call('window.__nuzky.api.startExport(arguments[0], arguments[1], arguments[2], arguments[3])', str(target),
                       {'resolution': 720, 'fps': 30, 'quality': 'recommended'}, r.state()['epoch'], False)
    job = started['ok'] and wait(lambda: next((j for j in r.state()['jobs'] if j['kind'] == 'export' and j['status'] != 'running'), None), 180)
    if not r.check('the export finishes', job and job['status'] == 'done', job or started):
        return
    exported = r.work / 'export-fire.png'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-i', str(target), '-vf', f'select=eq(n\\,{AT_US * 30 // 1_000_000})',
                    '-frames:v', '1', '-update', '1', str(exported)], check=True)
    r.shots.append(exported.name)
    changed = changed_share(engine, exported)
    r.check('the exported frame matches the frame the renderer draws for the preview', changed < 0.002, f'{changed:.3%} of pixels differ')
    out, without = Image.open(exported), Image.open(plain)
    k = out.width / canvas['width']
    xs, ys = [p[0] * k for p in box['corners']], [p[1] * k for p in box['corners']]
    inside = (min(xs), min(ys), max(xs), max(ys))
    r.check('the export shows the flame in the middle and no flame there without it',
            orange(out, inside) > 0.15 and orange(without, inside) < 0.01, (round(orange(out, inside), 3), round(orange(without, inside), 3)))
    below = (0, max(ys) + 10, out.width, out.height)
    r.check('the picture outside the emoji is the video\'s', changed_share(out.crop(tuple(round(v) for v in below)),
                                                                        without.crop(tuple(round(v) for v in below))) < 0.005)
    r.check('no error toast', not r.errors(), r.errors())

    # Proof of the look in the largest and the smallest window.
    r.s.run(TYPE, 'Search emoji', '')
    for width, height, name in ((1440, 900, 'emoji'), (1024, 640, 'emoji-1024')):
        resize(r, width, height)
        r.s.run(CLICK, '#text-tab-emoji')
        time.sleep(1)
        r.shot(name)
    r.s.run(CLICK, '#text-tab-stickers')
    time.sleep(0.5)
    r.shot('stickers-1024')

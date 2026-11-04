"""Find text over faces on a close selfie: the fixture is the NASA interview cropped tight to 9:16, her mouth at about
1220 px of 1920, where Reels-style captions sit. Four captions in the Reel style and a 🔥 sticker added at the playhead
and made smaller both cover her face. Find text over faces in the Captions tab, reached by keyboard, finds all five, names the first and
selects it at the playhead, Next goes through them, and Move all moves them clear of her face inside the Reels safe
area as one step, which the same check then confirms and Ctrl+Z undoes. Before and after are the engine's frames of
the saved project. The face model is linked from tmp-test, so nothing is downloaded."""
import json, subprocess, time

from e2e.harness import CLI, FIXTURES, flow, link_models, press, wait
from e2e.layout import resize

PROJECT = 'return window.__nuzky.store.getState().snap.project'
REEL = {'fontSize': 95, 'color': '#ffffff', 'bold': False, 'strokeWidth': 7.5, 'strokeColor': '#000000', 'background': None}
TEXTS = ['Hello there', 'this is', 'a close', 'selfie']
ADD = """window.__nuzky.store.getState().edit({type: 'addCaptions', style: arguments[0],
  segments: arguments[1].map((text, i) => ({startUs: i * 1500000, endUs: (i + 1) * 1500000, text, words: []}))})"""
CLICK = "const el = document.querySelector(arguments[0]); if (!el) return false; el.click(); return true;"
TYPE = """const input = document.querySelector(`input[aria-label="${arguments[0]}"]`); input.focus();
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, arguments[1]);
input.dispatchEvent(new Event('input', {bubbles: true}));"""
CHIP = 'return document.querySelector("section[aria-label=Preview] [role=status]")?.innerText ?? null'
CHIP_BUTTON = """return [...document.querySelectorAll('section[aria-label=Preview] [role=status] button')]
    .find((b) => b.textContent.trim() === arguments[0])?.click() ?? false"""
STICKER_AT = 4_600_000
# What Reels and TikTok leave free down a 1920 px tall canvas.
SAFE = (250, 1420)


def faces_setup(r):
    link_models(r, names=('yunet-2026may.onnx',))


def layers(r):
    """Captions and the sticker: id, kind, start and the transform's y."""
    project = r.s.run(PROJECT, retries=3)
    return [{'id': c['id'], 'track': t['name'], 'start': c['startUs'], 'y': c['content']['transform']['y']}
            for t in project['tracks'] if t['id'] != 'main' for c in t['clips']]


def engine_frame(r, name, at_us):
    path = r.work / f'{name}.png'
    subprocess.run([str(CLI), 'frame', str(r.saved_project()), f'{at_us / 1e6:.6f}', str(path), '540'], env=r.env, check=True,
                   capture_output=True)
    if f'{name}.png' not in r.shots:
        r.shots.append(f'{name}.png')
    return path


def faces_jobs(r):
    return r.s.run("return Object.values(window.__nuzky.store.getState().jobs).filter((j) => j.kind === 'faces')", retries=3)


def check_faces(r):
    """Starts the check from the Captions tab by keyboard and waits for its job."""
    before = {j['id'] for j in faces_jobs(r)}
    r.s.run('document.querySelector("[data-check-faces]").focus()')
    press('Return')
    return wait(lambda: (j := next((j for j in faces_jobs(r) if j['id'] not in before), None)) and j['status'] != 'running' and j, 120)


@flow('covered_faces', 'Find text over faces on a close selfie finds the Reel captions and a sticker over her face, goes '
      'through them, moves them clear of it inside the safe area as one step that the check confirms and Ctrl+Z undoes',
      before=faces_setup)
def covered_faces(r):
    r.import_media(FIXTURES / 'close-head.mp4')
    r.add_clip('close-head.mp4')
    added = r.s.call(ADD, REEL, TEXTS)
    r.check('four captions in the Reel style', added['ok'] and wait(lambda: len(layers(r)) == 4, 5), layers(r))
    r.seek(STICKER_AT)
    r.s.run("window.__nuzky.store.getState().select([])")
    r.s.run(CLICK, '[role=tablist][aria-label=Library] [data-tab="text"]')
    r.s.run(CLICK, '#text-tab-emoji')
    wait(lambda: r.s.run("return !!document.querySelector('input[aria-label=\"Search emoji\"]')"), 10)
    r.s.run(TYPE, 'Search emoji', 'fire')
    wait(lambda: r.s.run("return document.querySelector('[data-emoji=\"1f525\"]') !== null"), 5)
    r.s.run(CLICK, '[data-emoji="1f525"]')
    r.check('a 🔥 sticker centred over her face at 4.6 s', wait(lambda: len(layers(r)) == 5, 10), layers(r))
    # Made smaller, as at its first size it is too large to sit anywhere beside so close a face.
    sticker = next(l['id'] for l in layers(r) if l['track'] != 'Captions')
    r.s.call("window.__nuzky.store.getState().edit({type: 'updateClip', clipId: arguments[0], transform: {...window.__nuzky.store"
             ".getState().snap.project.tracks.flatMap((t) => t.clips).find((c) => c.id === arguments[0]).content.transform, scale: 0.2}})",
             sticker)
    scale = "return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips).find((c) => c.id === arguments[0]).content.transform.scale"
    r.check('the sticker is a fifth of the width', wait(lambda: abs(r.s.run(scale, sticker) - 0.2) < 1e-6, 5))
    start = layers(r)
    time.sleep(1.5)  # `nuzky frame` reads the saved file, written about a second after an edit.
    before = engine_frame(r, 'faces-before', 2_250_000)

    r.s.run(CLICK, '[role=tablist][aria-label=Library] [data-tab="captions"]')
    wait(lambda: r.s.run('return !!document.querySelector("[data-check-faces]")'), 5)
    job = check_faces(r)
    r.check('Find text over faces runs as a job and finishes', job and job['status'] == 'done', job)
    found = json.loads(job['output'])['covered'] if job and job.get('output') else []
    kinds = sorted(c['kind'] for c in found)
    r.check('it finds the four captions and the sticker over her face', kinds == ['caption'] * 4 + ['sticker'],
            [(c['kind'], c['label'], c['face'], c['layer']) for c in found])
    r.check('each has a place clear of it inside the safe area', all(
        c['suggestion'] and SAFE[0] <= c['suggestion']['layer'][1] and c['suggestion']['layer'][1] + c['suggestion']['layer'][3] <= SAFE[1]
        for c in found), [c['suggestion'] and c['suggestion']['layer'] for c in found])
    chip = wait(lambda: (t := r.s.run(CHIP)) and 'over a face' in t and t, 10)
    r.check('the preview names the first one, when it covers the face, and counts them', chip and
            '00:00.00 · Caption over a face · “Hello there”' in chip and '1 of 5' in chip, chip)
    first = found[0]['clip_id'] if found else None
    state = r.s.run('const s = window.__nuzky.store.getState(); return {selection: s.selection, t: s.timeUs}')
    r.check('it selects that caption and puts the playhead on it', state['selection'] == [first] and state['t'] == found[0]['start_us'], state)
    time.sleep(1)
    r.shot('faces-found')
    r.s.run(CHIP_BUTTON, 'Next')
    r.check('Next goes to the second', wait(lambda: '2 of 5' in (r.s.run(CHIP) or ''), 5), r.s.run(CHIP))
    if r.check('the window goes to its smallest size', resize(r, 1024, 640)):
        time.sleep(1)
        whole = r.s.run('const c = document.querySelector("section[aria-label=Preview] [role=status] span"); return c.textContent.startsWith("00:01.50 · Caption over a face")')
        r.check('in the smallest window the chip still shows when and what', whole)
        r.shot('faces-1024')
        resize(r, 1440, 900)

    r.s.run(CHIP_BUTTON, 'Move all')
    moved = wait(lambda: (t := r.s.run(CHIP)) and 'Moved 4 captions and 1 sticker off the face' in t and t, 10)
    r.check('Move all moves all five and offers Undo', moved and 'Undo' in moved, r.s.run(CHIP))
    after_move = layers(r)
    r.check('every one moved', all(abs(a['y'] - b['y']) > 0.01 for a, b in zip(start, after_move)), after_move)
    time.sleep(1.5)
    after = engine_frame(r, 'faces-after', 2_250_000)
    r.seek(2_250_000)
    time.sleep(1)
    r.shot('faces-moved')
    again = check_faces(r)
    r.check('the same check finds nothing over her face now', again and json.loads(again['output'])['covered'] == [], again)
    r.check('and says so', wait(lambda: any(t['text'] == 'No caption, text or sticker covers a face.' for t in r.state()['toasts']), 5))
    r.s.run('document.activeElement.blur()')
    r.key('z', ctrlKey=True)
    r.check('one undo puts all five back', wait(lambda: [l['y'] for l in layers(r)] == [l['y'] for l in start], 5), layers(r))
    on_disk = lambda: sorted(c['content']['transform']['y'] for t in json.loads(r.saved_project().read_text())['tracks']
                             if t['id'] != 'main' for c in t['clips'])
    r.check('the saved project has them back too', wait(lambda: all(abs(a - b) < 1e-5 for a, b in zip(on_disk(), sorted(l['y'] for l in start))), 10),
            (on_disk(), start))
    r.check('before and after frames were drawn', before.exists() and after.exists())

"""The cover editor: a person opens it from the Cover tile, lets Pick for me choose the frame, chooses another one,
writes a hook, puts it behind the person, undoes and redoes that, lets the AI make both covers with one click and
exports the cover as PNG and the YouTube thumbnail as JPEG. Checks the saved project, the pixels of the cover the
editor draws and the exported files. The face and person models are linked from tmp-test, so nothing downloads."""
import json, os, shutil, subprocess, time
from pathlib import Path

from e2e.harness import CLI, FIXTURES, MODELS, WINDOWS, Session, changed_share, close_window, flow, leave_editor, wait, webdriver

FAKE = Path(__file__).resolve().parent / 'fake_agent'
VISION = ('yunet-2026may.onnx', 'face-landmarks-v2.onnx', 'face-blendshapes-v2.onnx', 'selfie-segmenter.onnx',
          'birefnet-lite.onnx')
CLICK = """const scope = arguments[1] ? document.querySelector(arguments[1]) : document;
const all = [...scope.querySelectorAll('button, [role=option]')];
const b = all.find((b) => b.textContent.trim() === arguments[0] || b.getAttribute('aria-label') === arguments[0] || b.title === arguments[0])
  ?? all.find((b) => b.textContent.trim().startsWith(arguments[0]));
if (!b) return null; b.click(); return b.getAttribute('aria-disabled') ?? 'false';"""
COVER = """const p = window.__nuzky.store.getState().snap.project;
return (p.thumbnails ?? []).find((t) => t.format === arguments[0]) ?? null;"""
# What the cover canvas in the editor shows: its time, whether it waits for the person's mask, and the share of
# its pixels that are the hook's pure green.
SHOWN = """const c = document.querySelector('[data-testid=cover-canvas]'); const stage = document.querySelector('[data-testid=cover-stage]');
if (!c || !c.width || c.width === 300) return null;
const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let n = 0;
for (let i = 0; i < d.length; i += 4) if (d[i + 1] > 180 && d[i] < 110 && d[i + 2] < 110) n++;
const cover = window.__nuzky.cover.getState();
return {green: n / (d.length / 4), width: c.width, height: c.height, time: Number(stage.dataset.time), waiting: stage.dataset.mask === 'missing',
        hidden: cover.hidden, revision: Number(stage.dataset.revision), now: window.__nuzky.store.getState().snap.revision};"""
# What is drawn over the cover besides the selection box, however small.
OVER_COVER = """const stage = document.querySelector('[data-testid=cover-stage]'); const c = stage.querySelector('[data-testid=cover-canvas]');
return [...stage.querySelectorAll('*')].filter((e) => !e.contains(c) && !e.closest('[data-testid=layer-overlay]') && e.getClientRects().length > 0)
  .map((e) => e.outerHTML.slice(0, 120));"""
# Sets a field as typing does: WebKitWebDriver cannot send keys to it.
TYPE = """const t = document.querySelector(arguments[0]); t.focus();
const proto = t instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
Object.getOwnPropertyDescriptor(proto, 'value').set.call(t, arguments[1]);
t.dispatchEvent(new Event('input', {bubbles: true})); t.dispatchEvent(new Event('change', {bubbles: true}));"""
DRAG = """const s = document.querySelector('[data-testid=cover-stage]').getBoundingClientRect();
const o = document.querySelector('[data-testid=cover-stage] [data-testid=layer-overlay]');
const x = s.left + s.width * (0.5 + arguments[0]), y = s.top + s.height * (0.5 + arguments[1]);
const at = (type, x, y) => new PointerEvent(type, {bubbles: true, button: 0, clientX: x, clientY: y, pointerId: 1});
o.dispatchEvent(at('pointerdown', x, y));
setTimeout(() => { window.dispatchEvent(at('pointermove', x, y + arguments[2] / 2));
  window.dispatchEvent(at('pointermove', x, y + arguments[2])); setTimeout(() => window.dispatchEvent(at('pointerup', x, y + arguments[2])), 100); }, 300);"""


def setup(r):
    project = r.work / 'data/nuzky/projects/face.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(FIXTURES / 'face-thumb.mp4')], env=r.env, check=True,
                   capture_output=True)
    models = r.work / 'data/nuzky/models'
    models.mkdir(parents=True, exist_ok=True)
    for name in VISION:
        try:
            os.link(MODELS / name, models / name)
        except OSError:
            shutil.copy(MODELS / name, models / name)
    r.env.update(NUZKY_AGENT_CLAUDE=str(FAKE), NUZKY_AGENT_CODEX=str(r.work / 'no-codex'), NUZKY_FAKE_AGENT_LOG=str(r.work / 'fake'))


def click(r, label, scope=None):
    return r.s.run(CLICK, label, scope)


def cover(r, fmt='cover_9x16'):
    return r.s.run(COVER, fmt, retries=3)


def shown(r):
    return r.s.run(SHOWN, retries=3)


def job(r, prefix, timeout):
    """The newest job whose id starts with `prefix`, once it is no longer running."""
    script = """const jobs = Object.values(window.__nuzky.store.getState().jobs).filter((j) => j.id.startsWith(arguments[0]));
    const j = jobs[jobs.length - 1]; return j && j.status !== 'running' ? j : null;"""
    return wait(lambda: r.s.run(script, prefix), timeout, 0.5)


@flow('thumbnail', 'The cover editor: Pick for me, a hook behind the person, undo, Make a thumbnail and PNG and JPEG export',
      before=setup)
def thumbnail(r):
    webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
    time.sleep(1)
    # A copy of the clip to undo in the editor, with the playhead in it: the shorter video clamps the playhead.
    r.focus_clip(r.track()[0]['id'])
    r.key('d', ctrlKey=True)
    copied = wait(lambda: len(r.track()) == 2, 5)
    r.seek(18_000_000)
    # Opened while playing: the playback stops, and where it stops is no frame the user chose.
    r.s.run("window.__nuzky.store.getState().togglePlay()")
    wait(lambda: r.s.run("return window.__nuzky.store.getState().playing"), 5)
    r.check('the Cover tile opens the cover editor', click(r, 'Make a cover') == 'false'
            and wait(lambda: r.s.run("return !!document.querySelector('[data-testid=cover-stage]')"), 5))
    time.sleep(1)
    r.check('nothing is made before a change, also when the editor opens during playback',
            cover(r) is None and not r.s.run("return window.__nuzky.store.getState().playing"))
    r.key('z', ctrlKey=True)
    undone = wait(lambda: len(r.track()) == 1, 5)
    time.sleep(1)
    r.check('Undo that shortens the video under the playhead saves no draft and keeps Redo',
            copied and undone and cover(r) is None and r.s.run("return window.__nuzky.store.getState().snap.canRedo"))
    r.check('the cover editor names its view and offers Back to video and Export cover in the top bar', r.s.run("""
        const h = document.querySelector('header'); const buttons = [...h.querySelectorAll('button')].map((b) => b.textContent.trim());
        return h.textContent.includes('Cover') && buttons.includes('Back to video') && buttons.includes('Export cover');"""))
    r.check('the cover editor, which has no timeline toolbar, keeps Undo and Redo in the top bar', r.s.run(
        """const labels = [...document.querySelectorAll('header button')].map((b) => b.getAttribute('aria-label') ?? '');
        return labels.some((l) => l.startsWith('Undo')) && labels.some((l) => l.startsWith('Redo'));"""))
    r.check('before the first cover Make it with AI leads Frames and Pick for me is secondary', r.s.run("""
        const buttons = [...document.querySelectorAll('[data-testid=cover-frames] button')];
        return buttons.find((b) => b.textContent.trim() === 'Make it with AI')?.classList.contains('btn-prominent')
          && !buttons.find((b) => b.textContent.trim() === 'Pick for me')?.classList.contains('btn-prominent');"""))
    time.sleep(2)
    r.shot('cover-draft')
    r.seek(2_000_000)
    chosen = wait(lambda: cover(r), 5)
    r.check('choosing a frame on the timeline saves the draft at that frame', chosen and abs(chosen['timeUs'] - 2_000_000) < 50_000, chosen)
    r.key('z', ctrlKey=True)
    r.check('Undo takes the made draft back', wait(lambda: cover(r) is None, 5))
    click(r, 'Back to video', 'header')
    r.check('Back to video leaves the cover editor', wait(lambda: r.s.run(
        "return !document.querySelector('[data-testid=cover-stage]') && !!document.querySelector('section[aria-label=Preview] canvas')"), 5))

    # Make the app see missing models, but restore its isolated files before any job can download them.
    models, aside = r.work / 'data/nuzky/models', r.work / 'models-aside'
    models.rename(aside)
    try:
        click(r, 'Make a cover')
        missing = wait(lambda: r.s.run("return window.__nuzky.cover.getState().models?.downloaded === false"), 5)
    finally:
        aside.rename(models)
    restored = r.s.call('window.__nuzky.api.visionModels().then((v) => JSON.stringify(v))')
    if not missing or not restored['ok'] or not json.loads(restored['value'])['downloaded']:
        r.shot('cover-models-missing')
        raise RuntimeError('Cannot safely test the models job without all fixture models restored')
    # A double click while the download is being asked for still lets the AI make the cover once the models are there.
    r.s.run("""const b = [...document.querySelectorAll('[data-testid=cover-frames] button')].find((b) => b.textContent.trim() === 'Make it with AI');
        b.click(); b.click();""")
    r.shot('make-a-thumbnail-models')
    checked = job(r, 'vision-models:', 60)
    hooked = wait(lambda: all((cover(r, f) or {}).get('texts', [{}])[0].get('text') == 'MY HOOK'
                  for f in ('cover_9x16', 'youtube_16x9')) and r.s.run("return window.__nuzky.agent.getState().status === 'idle'"), 60)
    r.check('Make it with AI, clicked twice, checks the restored models then runs the agent on both covers',
            checked and checked['status'] == 'done' and hooked and (r.work / 'fake/runs.jsonl').exists(), checked)
    undo = click(r, 'Undo', 'aside[aria-label=AI]')
    r.check('the models-first AI run undoes back to the empty draft', undo == 'false' and wait(lambda: cover(r) is None
            and cover(r, 'youtube_16x9') is None, 10))
    r.s.run("window.__nuzky.dock.setState({open: false})")

    click(r, 'Pick for me')
    picked = job(r, 'cover-pick:', 240)
    made = wait(lambda: cover(r), 10)
    candidates = r.s.run("return document.querySelectorAll('[data-candidate]').length")
    # The fixture: [0,3) s blurred, [3,6) a blink, [6,12) sharp open eyes.
    r.check('Pick for me picks a frame with sharp open eyes and shows 6 to 8 frames with their reasons',
            picked and picked['status'] == 'done' and made and made['timeUs'] >= 6_000_000 and 1 <= candidates <= 8,
            {'job': picked, 'cover': made, 'candidates': candidates})
    reasons = r.s.run("return [...document.querySelectorAll('[data-candidate]')].map((b) => b.innerText)")
    r.check('each frame says its score and why', reasons and all(any(c.isdigit() for c in t) and len(t) > 8 for t in reasons), reasons)
    time.sleep(2)
    r.shot('cover-picked')

    # Another of the frames it found: the playhead goes there and the cover takes it.
    second = r.s.run("return Number(document.querySelectorAll('[data-candidate]')[1].dataset.candidate)")
    r.s.run("document.querySelectorAll('[data-candidate]')[1].click()")
    r.check('choosing another frame sets the cover to it and moves the playhead there',
            wait(lambda: (cover(r) or {}).get('timeUs') == second, 5)
            and wait(lambda: abs(r.s.run('return window.__nuzky.store.getState().timeUs') - second) < 1000, 5),
            {'cover': cover(r), 'second': second})

    # Add text starts Outlined, in front in the lower middle, selected and ready to type over.
    click(r, 'Add text')
    added = wait(lambda: r.s.run("""const t = document.querySelector('[data-cover-text]');
        return t && document.activeElement === t && t.value === 'YOUR HOOK' && t.selectionStart === 0
          && t.selectionEnd === t.value.length && window.__nuzky.cover.getState().selected === 0;"""), 5)
    text = (cover(r) or {}).get('texts', [{}])[0]
    r.check('Add text adds an Outlined hook in front, low on the cover, selects it and focuses its selected words', added
            and text.get('text') == 'YOUR HOOK' and not text['behind'] and text['style']['fontFamily'] == 'Anton'
            and text['style']['fontSize'] == 170 and text['style']['strokeWidth'] == 14 and text['transform']['y'] > 0, text)
    r.s.run(TYPE, '[data-cover-text]', 'I QUIT SUGAR')
    r.s.run(TYPE, 'input[type=color][aria-label=Color]', '#00ff00')
    text = wait(lambda: (t := (cover(r) or {}).get('texts', [{}])[0]) and t.get('text') == 'I QUIT SUGAR'
                and t['style']['color'] == '#00ff00' and t, 5)
    r.check('the text is typed and coloured in the inspector and saved in front of the person',
            text and not text['behind'], text)
    # Up over the head, so behind the person the head covers part of it.
    click(r, 'Position', '[data-testid=cover-text-inspector]')
    r.s.run(TYPE + "t.blur();", 'input[aria-label="Position Y value"]', '-22')
    wait(lambda: abs(cover(r)['texts'][0]['transform']['y'] + 0.22) < 0.01, 5)
    front = wait(lambda: (v := shown(r)) and v['green'] > 0.01 and v['revision'] == v['now'] and v, 10)
    r.check('the cover on screen shows the hook', front, front)

    # Behind person: the person is cut out of the frame a moment later, and the head covers part of the hook.
    click(r, 'Behind person', '[data-testid=cover-text-inspector]')
    r.check('Behind person is saved', wait(lambda: (cover(r) or {}).get('texts', [{}])[0].get('behind'), 5), cover(r))
    behind = wait(lambda: front and (v := shown(r)) and not v['waiting'] and v['revision'] == v['now'] and v['green'] < front['green'] * 0.95 and v, 120)
    r.check('behind the person the head covers part of the hook on screen, and the share is known',
            behind and behind['green'] > 0.002 and 0.02 < (behind['hidden'] or [0])[0] < 0.95, {'front': front, 'behind': behind})
    r.shot('cover-behind')

    r.key('z', ctrlKey=True)
    undone = wait(lambda: not (cover(r) or {}).get('texts', [{}])[0].get('behind', True)
                  and (v := shown(r)) and v['revision'] == v['now'] and v, 10)
    r.check('Undo puts the hook in front again, on screen too', undone and front and abs(undone['green'] - front['green']) < 0.003,
            {'undone': undone, 'front': front})
    r.key('z', ctrlKey=True, shiftKey=True)
    redone = wait(lambda: (cover(r) or {}).get('texts', [{}])[0].get('behind')
                  and (v := shown(r)) and v['revision'] == v['now'] and not v['waiting'] and v, 10)
    r.check('Redo puts it behind the person again', redone and behind and abs(redone['green'] - behind['green']) < 0.003,
            {'redone': redone, 'behind': behind})

    # Dragging the hook in the cover moves it, as text moves in the preview.
    y = cover(r)['texts'][0]['transform']['y']
    r.s.run("window.__nuzky.cover.setState({selected: 0})")
    r.s.run(DRAG, 0, y, -40)
    moved = wait(lambda: (t := cover(r)['texts'][0]['transform'])['y'] < y - 0.02 and t, 5)
    r.check('dragging the hook moves it up', moved, {'before': y, 'after': cover(r)['texts'][0]['transform']})

    # The other format, made from the Export dialog with the playhead elsewhere, shows the hook without saving: the
    # editor moving the playhead to the cover's frame chooses nothing. Export and selecting its row each make that draft.
    click(r, 'Back to video', 'header')
    r.seek(1_000_000)
    r.key('e', ctrlKey=True)
    click(r, 'Cover', '[aria-label="What to export"]')
    click(r, '16:9 YouTube', '[aria-label="Cover format"]')
    click(r, 'Make the thumbnail', '[role=dialog]')
    draft = wait(lambda: (v := shown(r)) and abs(v['width'] / v['height'] - 16 / 9) < 0.01
                 and not v['waiting'] and v['revision'] == v['now'] and v['green'] > 0.002 and v, 120)
    r.check('16:9 shows the other cover hook as an unsaved draft', draft and cover(r, 'youtube_16x9') is None
            and r.s.run("return [...document.querySelectorAll('[aria-label=\"Texts on the cover\"] button')].some((b) => b.textContent.includes('I QUIT SUGAR'))"), draft)
    r.shot('youtube-draft')
    click(r, 'Export cover', 'header')
    r.check('Export cover saves the draft and opens the dialog on the YouTube thumbnail', wait(lambda:
        cover(r, 'youtube_16x9') and r.s.run("""return [...document.querySelectorAll('[role=dialog] h2')].some((h) => h.textContent === 'Export cover')
          && document.querySelector('[role=dialog] [aria-label="Cover format"] button[aria-pressed=true]')?.textContent === '16:9 YouTube';"""), 5))
    r.key('Escape')
    r.key('z', ctrlKey=True)
    unsaved = wait(lambda: cover(r, 'youtube_16x9') is None, 5)
    # The hook of the draft sits beside the person, a sixth right of the middle and a tenth above it.
    r.s.run(DRAG, 1 / 6, -0.1, 40)
    dragged = wait(lambda: (c := cover(r, 'youtube_16x9')) and c['texts'][0]['transform']['y'] > -0.08 and c, 5)
    r.check('dragging a text of the unsaved draft saves the YouTube thumbnail and moves the text', unsaved and dragged, dragged)
    r.key('z', ctrlKey=True)
    wait(lambda: (c := cover(r, 'youtube_16x9')) and c['texts'][0]['transform']['y'] < -0.09, 5)
    r.key('z', ctrlKey=True)
    unsaved = wait(lambda: cover(r, 'youtube_16x9') is None, 5)
    click(r, 'I QUIT SUGAR', '[aria-label="Texts on the cover"]')
    wide = wait(lambda: cover(r, 'youtube_16x9'), 5)
    r.check('selecting the draft text saves the YouTube thumbnail with the person on a third and the hook beside them',
            unsaved and wide and wide['timeUs'] == cover(r)['timeUs'] and wide['frame']['x'] < -0.1 and wide['frame']['scale'] > 1.2
            and wide['texts'] and wide['texts'][0]['transform']['x'] > 0.1 and not wide['texts'][0]['behind']
            and wide['texts'][0]['style']['fontSize'] < cover(r)['texts'][0]['style']['fontSize']
            and r.s.run("return window.__nuzky.cover.getState().selected === 0 && !!document.querySelector('[data-cover-text]')"), wide)
    time.sleep(1.5)
    r.shot('youtube-start')
    # The hook over the person and behind them, from the inspector: the list, Behind person, then Position X.
    click(r, 'Behind person', '[data-testid=cover-text-inspector]')
    click(r, 'Position', '[data-testid=cover-text-inspector]')
    r.s.run(TYPE + "t.blur();", 'input[aria-label="Position X value"]', '-16.7')
    placed = wait(lambda: (t := cover(r, 'youtube_16x9')['texts'][0])['behind'] and abs(t['transform']['x'] + 0.167) < 0.01 and t, 5)
    r.check('the hook goes behind the person on the YouTube thumbnail too', placed, cover(r, 'youtube_16x9'))
    wide_shown = wait(lambda: (v := shown(r)) and not v['waiting'] and v['revision'] == v['now'] and (v['hidden'] or [0])[0] > 0 and v, 120)
    r.check('the YouTube thumbnail on screen is 16:9 with the hook partly behind the person',
            wide_shown and wide_shown['green'] > 0.002 and 0 < (wide_shown['hidden'] or [0])[0] < 0.95, wide_shown)
    r.check('nothing but the selection box is drawn over the YouTube thumbnail', not (over := r.s.run(OVER_COVER)), over)
    r.shot('youtube-behind')

    # The smallest window keeps the cover, its bar and the frames usable.
    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    wait(lambda: r.s.run('return window.innerWidth') == 1024, 5)
    click(r, '9:16 Cover')
    time.sleep(2)
    r.shot('cover-1024')
    webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
    wait(lambda: r.s.run('return window.innerWidth') == 1440, 5)

    # One click sends the thumbnail prompt without wishes and makes both covers live in one undoable run.
    before = {f: cover(r, f) for f in ('cover_9x16', 'youtube_16x9')}
    runs = len((r.work / 'fake/runs.jsonl').read_text().splitlines())
    click(r, 'Make it with AI', '[data-testid=cover-frames]')
    r.shot('make-a-thumbnail-started')
    hooked = wait(lambda: all((cover(r, f) or {}).get('texts', [{}])[0].get('text') == 'MY HOOK' for f in before), 60)
    r.check('the agent keeps the frame the user chose', cover(r)['timeUs'] == before['cover_9x16']['timeUs'],
            {'before': before['cover_9x16']['timeUs'], 'after': cover(r)['timeUs']})
    sent = (r.work / 'fake/runs.jsonl').read_text().splitlines()[runs:]
    prompt = json.loads(sent[0])['prompt'] if sent else ''
    r.check('with a cover present one click sends the thumbnail prompt without wishes and makes both covers',
            hooked and len(sent) == 1 and prompt.startswith('Make a 9:16 cover') and 'which win over the steps: none' in prompt,
            {'prompt': prompt[:120], 'covers': {f: cover(r, f) for f in before}})
    card = wait(lambda: r.s.run("""const items = window.__nuzky.agent.getState().items; const run = items.findLast((i) => i.kind === 'run');
        return run && !run.undone && window.__nuzky.agent.getState().status === 'idle' ? run.changes.map((c) => c.text) : null;"""), 30)
    r.check('its run card says it made both covers', card and any('9:16 cover' in t for t in card) and any('YouTube' in t for t in card), card)
    time.sleep(1)
    r.shot('make-a-thumbnail')
    click(r, 'Undo', 'aside[aria-label=AI]')
    r.check('one Undo takes the whole run back', wait(lambda: {f: cover(r, f) for f in before} == before, 10),
            {'now': {f: cover(r, f) for f in before}, 'before': before})
    r.s.run("window.__nuzky.dock.setState({open: false})")

    # Export: PNG of the cover, JPEG of the YouTube thumbnail, and an existing file only after confirmation.
    r.key('e', ctrlKey=True)
    r.check('Ctrl+E in the cover editor opens the export on the cover',
            wait(lambda: r.s.run("return [...document.querySelectorAll('[role=dialog] h2')].some((h) => h.textContent === 'Export cover')"), 5))
    time.sleep(0.5)  # the dialog fades in over 200 ms
    r.shot('export-cover')
    r.key('Escape')
    epoch = r.state()['epoch']
    png, jpg = r.work / 'cover.png', r.work / 'thumbnail.jpg'

    def export(fmt, path, replace):
        started = r.s.call('window.__nuzky.api.startCoverExport(arguments[0], arguments[1], arguments[2], arguments[3])',
                           fmt, str(path), replace, epoch)
        if not started['ok']:
            return started
        return wait(lambda: (j := r.s.run('return window.__nuzky.store.getState().jobs[arguments[0]] ?? null', started['value']))
                    and j['status'] != 'running' and j, 120)

    done = export('cover_9x16', png, False)
    from PIL import Image
    image = Image.open(png).convert('RGB') if png.exists() else None
    r.check('the cover exports as a 1080 x 1920 PNG', done and done.get('status') == 'done' and image and image.size == (1080, 1920)
            and png.read_bytes()[:4] == b'\x89PNG', done)
    if image:
        import numpy
        pixels = numpy.asarray(image.resize((256, 455)), dtype=int)
        green = float(((pixels[..., 1] > 180) & (pixels[..., 0] < 110) & (pixels[..., 2] < 110)).mean())
        r.check('the exported cover has the hook behind the person, as the editor shows it',
                redone and front and abs(green - redone['green']) < 0.01 and green < front['green'] * 0.95,
                {'export': green, 'editor': redone, 'front': front})
        click(r, '9:16 Cover')
        r.s.run("window.__nuzky.cover.setState({selected: null})")
        wait(lambda: (v := shown(r)) and v['revision'] == v['now'] and not v['waiting'], 10)
        time.sleep(0.5)
        r.check('nothing is drawn over the cover', not (over := r.s.run(OVER_COVER)), over)
        rect = r.s.run("""const b = document.querySelector('[data-testid=cover-canvas]').getBoundingClientRect();
            return [b.left, b.top, b.right, b.bottom, window.innerWidth];""")
        r.shot('cover-as-exported')
        screen = Image.open(r.work / 'cover-as-exported.png').convert('RGB')
        on_screen = screen.crop(tuple(round(v * screen.width / rect[4]) for v in rect[:4]))
        # At half size the editor's and Pillow's resampling of the same detailed frame stay apart from an overlay.
        half = (on_screen.width // 2, on_screen.height // 2)
        differs = changed_share(on_screen.resize(half, Image.BOX), image.resize(half, Image.BOX))
        r.check('the cover on screen is the exported cover', differs < 0.03, differs)
    stamp = png.read_bytes()
    refused = export('cover_9x16', png, False)
    r.check('an existing file is not replaced without asking', not refused['ok'] and 'DESTINATION_EXISTS' in refused['error']
            and png.read_bytes() == stamp, refused)
    replaced = export('cover_9x16', png, True)
    r.check('once confirmed, it is replaced', replaced and replaced.get('status') == 'done', replaced)
    done = export('youtube_16x9', jpg, False)
    data = jpg.read_bytes() if jpg.exists() else b''
    r.check('the YouTube thumbnail exports as a 1280 x 720 JPEG under 2 MB', done and done.get('status') == 'done'
            and data[:2] == b'\xff\xd8' and Image.open(jpg).size == (1280, 720) and len(data) < 2 * 1024 * 1024,
            {'job': done, 'bytes': len(data)})

    r.check('the saved project holds both covers with the hook behind the person', wait(lambda: all(
        any(t['text'] == 'I QUIT SUGAR' and t['behind'] for t in c['texts'])
        for c in json.loads(r.saved_project().read_text()).get('thumbnails', [])) and len(json.loads(r.saved_project().read_text()).get('thumbnails', [])) == 2, 5),
        json.loads(r.saved_project().read_text()).get('thumbnails'))
    r.key('Escape')
    r.key('Escape')
    r.check('Esc leaves the cover editor and the video shows again',
            wait(lambda: not r.s.run("return !!document.querySelector('[data-testid=cover-stage]')"), 5))
    r.check('no error toast', not r.errors(), r.errors())


# Where the logo's pure magenta is on the cover canvas, as shares of its width and height.
LOGO = """const c = document.querySelector('[data-testid=cover-canvas]'); const stage = document.querySelector('[data-testid=cover-stage]');
if (!c || !c.width || c.width === 300) return null;
const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let n = 0, sx = 0, sy = 0;
for (let i = 0; i < d.length; i += 4) if (d[i] > 230 && d[i + 1] < 40 && d[i + 2] > 230) { const p = i / 4; n++; sx += p % c.width + 0.5; sy += Math.floor(p / c.width) + 0.5; }
return {share: n / (d.length / 4), x: n ? sx / n / c.width : null, y: n ? sy / n / c.height : null, aspect: c.width / c.height,
        revision: Number(stage.dataset.revision), now: window.__nuzky.store.getState().snap.revision,
        alert: stage.querySelector('[role=alert]')?.innerText ?? null};"""


def logo_setup(r):
    """A plain grey 9:16 video, so only the logo is magenta, and a logo: a magenta square in the middle half of a
    transparent 400 px PNG."""
    from PIL import Image
    video = r.work / 'grey.mp4'
    subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'color=c=0x808080:s=540x960:d=4:r=30', '-pix_fmt', 'yuv420p',
                    str(video)], check=True)
    project = r.work / 'data/nuzky/projects/logo.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(video)], env=r.env, check=True, capture_output=True)
    logo = Image.new('RGBA', (400, 400), (0, 0, 0, 0))
    logo.paste((255, 0, 255, 255), (100, 100, 300, 300))
    logo.save(r.work / 'logo.png')


@flow('cover_images', 'A logo on the cover: Add image, drag, size, Delete with Undo, the YouTube draft, export and a missing file',
      before=logo_setup)
def cover_images(r):
    from e2e.setup_ai import ANSWER_FILE_DIALOG
    from PIL import Image
    webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
    time.sleep(1)
    click(r, 'Make a cover')
    wait(lambda: r.s.run("return !!document.querySelector('[data-testid=cover-stage]')"), 5)
    logo = r.work / 'logo.png'
    r.s.run(ANSWER_FILE_DIALOG, str(logo))
    click(r, 'Add image')
    added = wait(lambda: (c := cover(r)) and c.get('overlays') and c, 10)
    assets = r.s.run("return window.__nuzky.store.getState().snap.project.assets")
    place = added['overlays'][0]['transform'] if added else {}
    asset = next((a for a in assets if added and a['id'] == added['overlays'][0]['assetId']), {})
    # Its longer side a fifth of 1080 px, its top right corner 24 px inside the Reels safe area: right 876, top 274.
    r.check('Add image, with no image in the project, imports the picked file and puts it in the top right corner of a new cover, selected',
            added and asset.get('path', '').endswith('logo.png') and abs(place['scale'] - 0.2) < 1e-6
            and abs(place['x'] - (876 - 108) / 1080 + 0.5) < 1e-3 and abs(place['y'] - (274 + 108) / 1920 + 0.5) < 1e-3
            and r.s.run("return window.__nuzky.cover.getState().selected") == 'overlay:0'
            and r.s.run("return document.querySelector('[data-testid=cover-image-inspector]')?.innerText.includes('logo.png') ?? false"),
            {'cover': added, 'asset': asset})
    seen = wait(lambda: (v := r.s.run(LOGO)) and v['revision'] == v['now'] and v['share'] > 0 and v, 10)
    r.check('the cover on screen shows the logo there, over the frame', seen and abs(seen['x'] - (0.5 + place['x'])) < 0.01
            and abs(seen['y'] - (0.5 + place['y'])) < 0.01, seen)
    time.sleep(0.5)
    r.shot('cover-logo')

    r.s.run(DRAG, place['x'], place['y'], 60)
    moved = wait(lambda: (t := cover(r)['overlays'][0]['transform'])['y'] > place['y'] + 0.02 and t, 5)
    r.check('dragging the logo on the cover moves it', moved, cover(r)['overlays'])
    r.key('z', ctrlKey=True)
    r.check('Undo puts it back in one step', wait(lambda: cover(r)['overlays'][0]['transform'] == place, 5), cover(r)['overlays'])
    r.s.run(TYPE + "t.blur();", 'input[aria-label="Size value"]', '40')
    r.check('Size makes it larger', wait(lambda: abs(cover(r)['overlays'][0]['transform']['scale'] - 0.4) < 1e-6, 5), cover(r)['overlays'])
    bigger = cover(r)['overlays']

    r.key('Delete')
    gone = wait(lambda: not cover(r).get('overlays'), 5)
    undo = r.s.run("""const t = window.__nuzky.store.getState().toasts.find((t) => t.text === 'Image removed from the cover');
        if (!t?.action) return false; t.action.run(); return true;""")
    r.check('Delete takes the selected logo off the cover and the toast brings it back with Undo',
            gone and undo and wait(lambda: cover(r).get('overlays') == bigger, 5), cover(r))

    # With an image in the project, Add image offers Import image… and then it.
    r.s.run("window.__nuzky.store.setState({toasts: []})")
    click(r, 'Add image')
    items = wait(lambda: r.s.run("return [...document.querySelectorAll('[role=menu] [role=menuitem]')].map((i) => i.textContent.trim())"), 5)
    r.shot('cover-add-image-menu')
    r.s.run("[...document.querySelectorAll('[role=menu] [role=menuitem]')].find((i) => i.textContent.trim() === 'logo.png')?.click()")
    two = wait(lambda: len(cover(r).get('overlays', [])) == 2, 5)
    focused = wait(lambda: r.s.run("return document.activeElement?.closest('[data-testid=cover-image-inspector]') ? document.activeElement.getAttribute('aria-label') : null"), 5)
    r.check('with an image in the project, Add image lists Import image… and then it, adds the one chosen and puts the focus on its Size',
            items and items[0] == 'Import image…' and items[-1] == 'logo.png' and two and focused == 'Size'
            and r.s.run("return window.__nuzky.cover.getState().selected") == 'overlay:1', {'items': items, 'focus': focused, 'cover': cover(r)})
    click(r, 'Delete image (Delete)', '[data-testid=cover-image-inspector]')
    r.check("the image's own Delete takes it off the cover", wait(lambda: cover(r).get('overlays') == bigger, 5), cover(r))
    rows = r.s.run("return [...document.querySelectorAll('[aria-label=\"Images on the cover\"] button')].map((b) => b.textContent.trim())")
    r.check('the cover lists its image by name', rows == ['logo.png'], rows)
    r.s.run("window.__nuzky.store.setState({toasts: []})")
    time.sleep(0.5)
    r.shot('cover-logo-settings')

    # The YouTube thumbnail not made yet starts from the cover, with the logo as large for its size in its top right corner.
    click(r, '16:9 YouTube', '[aria-label="Cover format"]')
    wide = wait(lambda: (v := r.s.run(LOGO)) and abs(v['aspect'] - 16 / 9) < 0.01 and v['share'] > 0 and v['revision'] == v['now'] and v, 10)
    r.check('the YouTube draft shows the logo in its top right corner and saves nothing', wide and wide['x'] > 0.8 and wide['y'] < 0.3
            and cover(r, 'youtube_16x9') is None, wide)
    r.shot('youtube-logo')
    click(r, '9:16 Cover', '[aria-label="Cover format"]')
    r.s.run("window.__nuzky.cover.setState({selected: null})")
    on_screen = wait(lambda: (v := r.s.run(LOGO)) and abs(v['aspect'] - 9 / 16) < 0.01 and v['share'] > 0 and v['revision'] == v['now'] and v, 10)

    png = r.work / 'cover.png'
    started = r.s.call('window.__nuzky.api.startCoverExport(arguments[0], arguments[1], arguments[2], arguments[3])',
                       'cover_9x16', str(png), False, r.state()['epoch'])
    done = started['ok'] and wait(lambda: (j := r.s.run('return window.__nuzky.store.getState().jobs[arguments[0]] ?? null', started['value']))
                                  and j['status'] != 'running' and j, 60)
    exported = None
    if png.exists():
        import numpy
        pixels = numpy.asarray(Image.open(png).convert('RGB'), dtype=int)
        ys, xs = numpy.nonzero((pixels[..., 0] > 230) & (pixels[..., 1] < 40) & (pixels[..., 2] > 230))
        exported = {'x': (xs.mean() + 0.5) / 1080, 'y': (ys.mean() + 0.5) / 1920} if len(xs) else None
    r.check('the exported cover has the logo where the editor shows it', done and done['status'] == 'done' and exported and on_screen
            and abs(exported['x'] - on_screen['x']) < 0.005 and abs(exported['y'] - on_screen['y']) < 0.005,
            {'job': done, 'export': exported, 'editor': on_screen})
    time.sleep(0.5)
    rect = r.s.run("""const b = document.querySelector('[data-testid=cover-canvas]').getBoundingClientRect();
        return [b.left, b.top, b.right, b.bottom, window.innerWidth];""")
    r.shot('cover-logo-as-exported')
    if png.exists():
        screen = Image.open(r.work / 'cover-logo-as-exported.png').convert('RGB')
        shown_cover = screen.crop(tuple(round(v * screen.width / rect[4]) for v in rect[:4]))
        half = (shown_cover.width // 2, shown_cover.height // 2)
        differs = changed_share(shown_cover.resize(half, Image.BOX), Image.open(png).convert('RGB').resize(half, Image.BOX))
        r.check('the cover on screen is the exported cover, logo included', differs < 0.03, differs)
    # The file keeps the shortest form of each number, the app the longer one of the same value.
    same = lambda a, b: a and b and len(a) == len(b) and all(x['assetId'] == y['assetId'] and all(
        abs(x['transform'][k] - y['transform'][k]) < 1e-6 for k in ('x', 'y', 'scale', 'rotation', 'opacity')) for x, y in zip(a, b))
    saved = wait(lambda: (c := next((t for t in json.loads(r.saved_project().read_text()).get('thumbnails', []) if t['format'] == 'cover_9x16'), None))
                 and same(c.get('overlays'), bigger) and c, 5)
    r.check('the saved project keeps the logo on the cover', saved, saved)

    # The file goes away: the cover says which and to put it back; back, it draws again.
    away = r.work / 'logo-moved.png'
    logo.rename(away)
    click(r, '16:9 YouTube', '[aria-label="Cover format"]')
    click(r, '9:16 Cover', '[aria-label="Cover format"]')
    missing = wait(lambda: (v := r.s.run(LOGO)) and v['alert'] and v, 10)
    r.check('a missing logo file says which, to put it back or to take it off the cover', missing
            and 'logo.png is missing. Put the file back where it was, or take the image off the cover.' in missing['alert'], missing)
    r.shot('cover-logo-missing')
    away.rename(logo)
    click(r, '16:9 YouTube', '[aria-label="Cover format"]')
    click(r, '9:16 Cover', '[aria-label="Cover format"]')
    r.check('put back, the logo draws again', wait(lambda: (v := r.s.run(LOGO)) and not v['alert'] and v['share'] > 0, 10))

    # Removing the image from the project takes it off the cover; Undo brings both back.
    r.s.call("window.__nuzky.store.getState().edit({type: 'removeAsset', assetId: arguments[0]})", asset.get('id'))
    off = wait(lambda: (c := cover(r)) and not c.get('overlays') and (v := r.s.run(LOGO)) and v['revision'] == v['now'] and v['share'] == 0, 10)
    r.key('z', ctrlKey=True)
    r.check('removing the image from the project takes it off the cover, and Undo puts it back',
            off and wait(lambda: cover(r).get('overlays') == bigger, 5), cover(r))

    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    wait(lambda: r.s.run('return window.innerWidth') == 1024, 5)
    r.s.run("[...document.querySelectorAll('[aria-label=\"Images on the cover\"] button')][0]?.click()")
    wait(lambda: r.s.run("return !!document.querySelector('[data-testid=cover-image-inspector]')"), 5)
    time.sleep(1)
    r.shot('cover-logo-1024')
    r.check('no error toast', not r.errors(), r.errors())


def states(r):
    """A test pattern without a face, copied so the flow can take it away, and the face detector damaged:
    the size of the real one, so it counts as installed until it is loaded."""
    video = r.work / 'pattern.mp4'
    shutil.copy(FIXTURES / 'talk.mp4', video)
    project = r.work / 'data/nuzky/projects/pattern.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(video)], env=r.env, check=True, capture_output=True)
    models = r.work / 'data/nuzky/models'
    models.mkdir(parents=True, exist_ok=True)
    for name in VISION[1:]:
        try:
            os.link(MODELS / name, models / name)
        except OSError:
            shutil.copy(MODELS / name, models / name)
    (models / VISION[0]).write_bytes(b'\0' * (MODELS / VISION[0]).stat().st_size)


@flow('cover_states', 'Cover states: a damaged model says to download it again, a video without a face and a missing file say what to do',
      before=states)
def cover_states(r):
    webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
    time.sleep(1)
    click(r, 'Make a cover')
    click(r, 'Pick for me')
    failed = job(r, 'cover-pick:', 120)
    alert = wait(lambda: r.s.run("return document.querySelector('[data-testid=cover-frames] [role=alert]')?.innerText ?? null"), 5)
    r.check('a damaged model fails Pick for me with a plain reason and Download again', failed and failed['status'] == 'failed'
            and alert and 'damaged' in alert and 'Download again' in alert, {'job': failed, 'alert': alert})
    r.shot('cover-model-damaged')

    # Put back as Download again would; Pick for me then runs on a video without a face.
    models = r.work / 'data/nuzky/models'
    (models / VISION[0]).unlink()
    shutil.copy(MODELS / VISION[0], models / VISION[0])
    click(r, 'Pick for me')
    done = job(r, 'cover-pick:', 240)
    note = wait(lambda: r.s.run("""return [...document.querySelectorAll('[data-testid=cover-frames] p')].map((p) => p.innerText)
        .find((t) => t.startsWith('No face')) ?? null"""), 10)
    r.check('without a face Pick for me still picks a frame and says there is no face', done and done['status'] == 'done'
            and cover(r) and note, {'job': done, 'note': note, 'cover': cover(r)})
    r.shot('cover-no-face')

    # The file goes away; the cover says which and what to do.
    if WINDOWS:
        # Windows refuses to move a file a program holds open, as the preview holds the one it shows: there it
        # goes while Nuzky is closed.
        leave_editor(r)
        close_window()
        wait(lambda: r.app.poll() is not None, 15)
        r.s.close()
        (r.work / 'pattern.mp4').rename(r.work / 'pattern-moved.mp4')
        r.s = Session()
        wait(lambda: r.s.run('return !!window.__nuzky?.store.getState().snap', retries=3), 60)
        webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
        r.s.run("window.__nuzky.openProject({path: window.__nuzky.store.getState().snap.path, name: 'pattern'})")
        wait(lambda: r.s.run('return window.__nuzky.store.getState().view') == 'editor', 10)
    else:
        (r.work / 'pattern.mp4').rename(r.work / 'pattern-moved.mp4')
        r.key('Escape')
    click(r, 'Edit the cover')
    missing = wait(lambda: r.s.run("return document.querySelector('[data-testid=cover-stage] [role=alert]')?.innerText ?? null"), 10)
    r.check('a missing file says which and to put it back', missing and 'pattern.mp4 is missing' in missing, missing)
    r.shot('cover-media-missing')
    refused = r.s.call("window.__nuzky.api.startCoverPick('cover_9x16', arguments[0])", r.state()['epoch'])
    r.check('Pick for me refuses with the same reason', not refused['ok'] and 'MEDIA_MISSING' in refused['error'], refused)
    (r.work / 'pattern-moved.mp4').rename(r.work / 'pattern.mp4')

"""The home screen and the launcher. Nuzky starts on every project, with the last one open behind it: pictures
rendered like the export, a project's cover when it has one, collections, Rename, Duplicate, the Trash with Undo,
and search through names and through what was said. Over the editor, the launcher opens recent projects, starts new
ones and searches the same way. Opening another project while an agent edits asks whether to keep or undo its
changes."""
import json, os, subprocess, time
from pathlib import Path

from e2e.harness import CLI, FIXTURES, LINUX, Bridge, changed_share, close_window, deny, flow, link_models, wait, webdriver

DAY = 86_400
CARDS = """return [...document.querySelectorAll('[role=listbox][aria-label=Projects] [role=option][data-path]')].map((c) => ({
  path: c.dataset.path, name: c.querySelector('[data-name]')?.textContent ?? c.querySelector('input')?.value,
  meta: c.querySelector('[data-meta]').textContent, selected: c.getAttribute('aria-selected') === 'true',
  disabled: c.getAttribute('aria-disabled') === 'true'}));"""
SECTIONS = "return [...document.querySelectorAll('[role=listbox][aria-label=Projects] section > h2')].map((h) => h.textContent);"
# A poster that shows the test pattern has many colours; an empty or black frame has a few.
POSTER_COLOURS = """const img = document.querySelector(`[data-path="${CSS.escape(arguments[0])}"] img`);
if (!img || !img.complete || !img.naturalWidth) return 0;
const c = document.createElement('canvas'); c.width = img.naturalWidth; c.height = img.naturalHeight;
const g = c.getContext('2d'); g.drawImage(img, 0, 0);
const d = g.getImageData(0, 0, c.width, c.height).data; const seen = new Set();
for (let i = 0; i < d.length; i += 16) seen.add((d[i] >> 4) << 8 | (d[i + 1] >> 4) << 4 | d[i + 2] >> 4);
return seen.size;"""
# A card's picture: the share of its pixels near the colour `arguments[1]`, the shapes of the image and its frame, the
# image's address and, with `arguments[2]`, the picture as a PNG data URL.
PICTURE = """const img = document.querySelector(`[data-path="${CSS.escape(arguments[0])}"] img`);
if (!img || !img.complete || !img.naturalWidth) return null;
const c = document.createElement('canvas'); c.width = img.naturalWidth; c.height = img.naturalHeight;
const g = c.getContext('2d'); g.drawImage(img, 0, 0);
const d = g.getImageData(0, 0, c.width, c.height).data; const [red, green, blue] = arguments[1]; let n = 0;
for (let i = 0; i < d.length; i += 4) if (Math.abs(d[i] - red) < 40 && Math.abs(d[i + 1] - green) < 40 && Math.abs(d[i + 2] - blue) < 40) n++;
const frame = img.parentElement.getBoundingClientRect();
return {share: n / (d.length / 4), image: img.naturalWidth / img.naturalHeight, frame: frame.width / frame.height, src: img.src,
        png: arguments[2] ? c.toDataURL('image/png') : null};"""
MAGENTA, CYAN, YELLOW, ORANGE = (255, 0, 255), (0, 255, 255), (255, 255, 0), (255, 128, 0)
# What the poster worker logs each time it draws a cover before the person is cut out of it.
DRAFT_DRAWN = 'shows its cover without the person cut out until their mask is made'
VISION = ('yunet-2026may.onnx', 'face-landmarks-v2.onnx', 'face-blendshapes-v2.onnx', 'selfie-segmenter.onnx',
          'birefnet-lite.onnx')
TYPE = """const el = document.querySelector(arguments[0]); el.focus();
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, arguments[1]);
el.dispatchEvent(new Event('input', {bubbles: true})); return true;"""
PRESS = """const el = arguments[0] ? document.querySelector(arguments[0]) : document.activeElement;
el.dispatchEvent(new KeyboardEvent('keydown', Object.assign({key: arguments[1], bubbles: true, cancelable: true}, arguments[2])));"""
CLICK_TEXT = """const scope = arguments[1] ? document.querySelector(arguments[1]) : document;
const el = [...scope.querySelectorAll('button, [role=menuitem], [role=menuitemradio], [role=option]')]
  .find((b) => b.textContent.trim().startsWith(arguments[0]));
if (!el) return false; el.click(); return true;"""
VIEW = "const s = window.__nuzky.store.getState(); return {view: s.view, launcher: s.launcherOpen, name: s.snap.project.name, path: s.snap.path, timeUs: s.timeUs, tab: s.panelTab, aiRun: s.aiRun, canvas: [s.snap.project.canvas.width, s.snap.project.canvas.height]};"


def projects_dir(r):
    return r.work / 'data/nuzky/projects'


def make(r, file, name, media, age_days):
    """A project of `media` named `name`, last changed `age_days` ago, as the CLI makes it."""
    path = projects_dir(r) / file
    subprocess.run([str(CLI), 'new', str(path), *map(str, media)], env=r.env, check=True, capture_output=True)
    project = json.loads(path.read_text())
    project['name'] = name
    path.write_text(json.dumps(project))
    stamp = time.time() - age_days * DAY
    os.utime(path, (stamp, stamp))
    return path


def set_cover(r, path, cover, image=None, before=(), logo=None):
    """An agent makes the project's cover, from `image` when given, with the image `logo` on it as `logo['transform']`
    places it, after the edits `before`, and leaves the project as old as it was."""
    stamp = path.stat().st_mtime
    agent = Bridge(r, path)
    try:
        run = agent.call('begin_run', {'label': 'Cover'})['run_id']
        if image:
            cover['image'] = agent.call('import_media', {'run_id': run, 'paths': [str(image)], 'request_id': 'image'})['asset_ids'][0]
        if logo:
            asset = agent.call('import_media', {'run_id': run, 'paths': [str(logo['path'])], 'request_id': 'logo'})['asset_ids'][0]
            cover['overlays'] = [{'assetId': asset, 'transform': logo['transform']}]
        edits = [*before, {'type': 'setThumbnail', 'thumbnail': cover}]
        agent.call('apply_edits', {'run_id': run, 'request_id': 'cover', 'edits': edits})
        agent.call('end_run', {'run_id': run, 'action': 'keep'})
    finally:
        agent.close()
    if not json.loads(path.read_text()).get('thumbnails'):
        raise RuntimeError(f'the cover of {path.name} was not saved')
    os.utime(path, (stamp, stamp))


def behind(path, background):
    """The edit that puts `background` behind the person in the first clip of the project at `path`."""
    clip = json.loads(path.read_text())['tracks'][0]['clips'][0]['id']
    return {'type': 'updateClip', 'clipId': clip, 'background': background}


def seed(r):
    link_models(r, ('ggml-small.bin', 'ggml-silero-v5.1.2.bin', *VISION))
    make(r, 'talk.nuzky', 'Talk to camera', [FIXTURES / 'talk.mp4'], 0)
    croatia = make(r, 'croatia.nuzky', 'Croatia day 2', [FIXTURES / 'wide-head.mp4'], 3)
    reel = make(r, 'reel.nuzky', 'Morning reel', [FIXTURES / 'portrait.mp4'], 40)
    # The reel, on a 16:9 canvas, has a 9:16 cover made from a generated image. The landscape interview, on a 16:9
    # canvas too, has only a 9:16 cover, on a solid colour, which cuts the person out of its frame and so waits for
    # their mask; its clip blurs what is behind the person, who is not found in its frames yet either.
    image = r.work / 'generated-cover.png'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'color=c=#ff00ff:s=1080x1920', '-frames:v', '1', str(image)],
                   check=True)
    # On the reel's cover a logo: a yellow square in the middle half of a transparent picture. Its file is moved away
    # before Nuzky starts, so the cover first cannot be drawn.
    from PIL import Image
    logo = Image.new('RGBA', (400, 400), (0, 0, 0, 0))
    logo.paste((255, 255, 0, 255), (100, 100, 300, 300))
    logo.save(r.work / 'logo.png')
    plain = {'x': 0, 'y': 0, 'scale': 1, 'rotation': 0, 'opacity': 1}
    set_cover(r, reel, {'format': 'cover_9x16', 'timeUs': 0, 'frame': plain, 'texts': []}, image,
              logo={'path': r.work / 'logo.png', 'transform': dict(plain, x=0.15, y=-0.2, scale=0.5)},
              before=[{'type': 'setCanvas', 'width': 1920, 'height': 1080}])
    (r.work / 'logo.png').rename(r.work / 'logo-away.png')
    set_cover(r, croatia, {'format': 'cover_9x16', 'timeUs': 1_000_000, 'frame': plain, 'texts': [],
                           'background': {'picture': False, 'blur': 0, 'dim': 0, 'color': '#00ffff'}},
              before=[{'type': 'setCanvas', 'width': 1920, 'height': 1080}, behind(croatia, {'type': 'blur', 'strength': 1})])
    broken = projects_dir(r) / 'broken.nuzky'
    broken.write_text('{"name": "half a proj')
    empty = projects_dir(r) / 'empty.nuzky'
    empty.write_text(json.dumps({'version': 1, 'name': 'Untitled project', 'canvas': {'width': 1080, 'height': 1920, 'fps': 30, 'background': '#000000'},
                                 'assets': [], 'tracks': [{'id': 'main', 'kind': 'video', 'name': 'Main', 'muted': False, 'hidden': False, 'clips': []}]}))
    for path, days in ((broken, 60), (empty, 20)):
        os.utime(path, (time.time() - days * DAY,) * 2)


def cards(r):
    return r.s.run(CARDS, retries=3) or []


def card(r, name):
    return next((c for c in cards(r) if c['name'] == name), None)


def view(r):
    return r.s.run(VIEW, retries=3)


def click(r, text, scope=None):
    return r.s.run(CLICK_TEXT, text, scope)


def press(r, key, selector=None, **modifiers):
    r.s.run(PRESS, selector, key, modifiers)


def focus_card(r, name):
    r.s.run("document.querySelector(`[data-path=\"${CSS.escape(arguments[0])}\"]`).focus()", card(r, name)['path'])


def toast_texts(r):
    return [t['text'] for t in r.state()['toasts']]


def resize(r, width, height):
    """The window at another size; False when the window manager does not allow it."""
    try:
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
    except RuntimeError:
        return False
    return wait(lambda: r.s.run('return [window.innerWidth, window.innerHeight]') == [width, height], 5)


def poster_cached(r, path):
    """Whether the poster cache holds a picture of the project at `path`; named by the FNV-1a of the path."""
    name = 0xcbf29ce484222325
    for byte in path.encode():
        name = ((name ^ byte) * 0x100000001b3) % 2 ** 64
    return any((r.work / 'cache/nuzky/posters').glob(f'{name:016x}-*.png'))


def drafts_drawn(r):
    """How many times the app drew a cover before the person was cut out of it, from its log file."""
    logs = r.work / 'data/nuzky/logs'
    return sum(p.read_text(errors='replace').count(DRAFT_DRAWN) for p in logs.glob('nuzky*.log'))


def wait_library(r):
    return wait(lambda: len(cards(r)) > 0, 20)


@flow('home', 'Nuzky starts on the home screen: every project with its picture, collections, rename, duplicate, the '
      'Trash with Undo and search through names and through what was said', before=seed, home=True)
def home(r):
    r.check('Nuzky starts on the home screen, the newest project open behind it',
            wait(lambda: view(r)['view'] == 'home', 10) and view(r)['name'] == 'Talk to camera', view(r))
    # The editor and the dialogs come in chunks of their own, fetched once Home has painted.
    order = r.s.run("""const paint = performance.getEntriesByType('paint').find((e) => e.name === 'first-contentful-paint');
        const chunk = (name) => performance.getEntriesByType('resource')
          .find((e) => e.name.includes(`/components/${name}.tsx`) || e.name.includes(`/assets/${name}-`));
        return {paint: paint?.startTime ?? null, editor: chunk('Editor')?.startTime ?? null, dialogs: chunk('Dialogs')?.startTime ?? null,
                resources: performance.getEntriesByType('resource').length};""")
    r.check('the home screen paints before the code of the editor and the dialogs loads', order['paint'] is not None
            and all(order[name] is not None and order[name] > order['paint'] for name in ('editor', 'dialogs')), order)
    r.check('the back button names the open project',
            r.s.run("return [...document.querySelectorAll('header button')].some((b) => b.textContent.includes('Talk to camera'))"))
    wait_library(r)
    names = sorted(c['name'] for c in cards(r))
    r.check('every project shows, the damaged one by its file name', names == ['Croatia day 2', 'Morning reel', 'Talk to camera', 'broken.nuzky'], names)
    r.check('an empty untitled project nobody has open is cleared away', not (projects_dir(r) / 'empty.nuzky').exists())
    r.check('the damaged project says why and cannot open', (card(r, 'broken.nuzky') or {}).get('meta') == 'Damaged file'
            and card(r, 'broken.nuzky')['disabled'], card(r, 'broken.nuzky'))
    r.check('the open project says so', (card(r, 'Talk to camera') or {}).get('meta') == 'Open now', card(r, 'Talk to camera'))
    sections = r.s.run(SECTIONS)
    order = [c['name'] for c in cards(r)]
    r.check('a few projects show newest first, without date groups', sections == [] and order[0] == 'Talk to camera'
            and order.index('Croatia day 2') < order.index('Morning reel'), (sections, order))
    talk = card(r, 'Talk to camera')['path']
    colours = wait(lambda: r.s.run(POSTER_COLOURS, talk) > 40 and r.s.run(POSTER_COLOURS, talk), 40)
    r.check('a project without a cover shows a frame, rendered like the export', colours, colours)
    upright = lambda p: abs(p['image'] - 9 / 16) < 0.02 and abs(p['frame'] - 9 / 16) < 0.02
    wide = lambda p: abs(p['image'] - 16 / 9) < 0.02 and abs(p['frame'] - 16 / 9) < 0.02
    reel = card(r, 'Morning reel')['path']
    # The test pattern has a magenta bar too, so the frame is told apart by its many colours and less magenta.
    is_frame = lambda: r.s.run(POSTER_COLOURS, reel) > 40 and (p := r.s.run(PICTURE, reel, MAGENTA)) and p['share'] < 0.5 and p
    frame = wait(is_frame, 40)
    r.check('a cover whose logo file is gone leaves the card its frame, kept like any poster', frame and wide(frame)
            and poster_cached(r, reel), {'frame': frame, 'cached': poster_cached(r, reel)})
    # The logo is back but does not open, so the cover cannot be drawn, as its export cannot: the card keeps its frame, in
    # the frame's shape, though the list expected the cover's.
    logo = r.work / 'logo.png'
    logo.write_bytes(b'not a picture')
    r.s.call('window.__nuzky.refreshLibrary()')
    again = wait(lambda: (p := is_frame()) and p['src'] != frame['src'] and p, 20)
    r.check('a cover whose logo does not open leaves the card its frame, not cut to the shape of the cover', again
            and wide(again), again or r.s.run(PICTURE, reel, MAGENTA))
    # The logo that opens is back in its place: the next list shows the cover, with the logo where its export has it.
    (r.work / 'logo-away.png').replace(logo)
    r.s.call('window.__nuzky.refreshLibrary()')
    cover = lambda: (p := r.s.run(PICTURE, reel, MAGENTA)) and p['share'] > 0.85 and upright(p) and p
    shown = wait(cover, 20)
    r.check('once its logo opens, a reel with a cover made from a generated image shows that cover in its 9:16 shape',
            shown, r.s.run(PICTURE, reel, MAGENTA))
    exported = r.work / 'reel-cover.png'
    made = subprocess.run([str(CLI), 'thumbnail', str(projects_dir(r) / 'reel.nuzky'), 'cover_9x16', str(exported)], env=r.env,
                          capture_output=True, text=True)
    picture = r.s.run(PICTURE, reel, YELLOW, True)
    if made.returncode == 0 and picture:
        import base64, io
        from PIL import Image
        on_card = Image.open(io.BytesIO(base64.b64decode(picture['png'].split(',', 1)[1]))).convert('RGB')
        differs = changed_share(on_card, Image.open(exported).convert('RGB').resize(on_card.size, Image.BOX))
    else:
        differs = None
    r.check('the card shows the logo on the cover as the exported cover has it', picture and picture['share'] > 0.02
            and differs is not None and differs < 0.006, {'logo': picture and picture['share'], 'differs': differs,
                                                         'export': made.stderr[-300:]})
    # The cover's own image stops opening. Its export fails, so the card shows the frame rather than a cover without its
    # picture, and the cover again once the image opens.
    image = r.work / 'generated-cover.png'
    good = image.read_bytes()
    image.write_bytes(b'not a picture')
    r.s.call('window.__nuzky.refreshLibrary()')
    frame = wait(lambda: (p := is_frame()) and wide(p) and p, 20)
    r.check('a cover whose image does not open leaves the card its frame', frame, r.s.run(PICTURE, reel, MAGENTA))
    image.write_bytes(good)
    r.s.call('window.__nuzky.refreshLibrary()')
    r.check('once its image opens again, the card shows the cover', wait(cover, 20), r.s.run(PICTURE, reel, MAGENTA))
    # The reel's cover becomes its frame at 1 s, and its clip gets a colour behind the person, who is not found in its
    # frames yet: the card shows the clip as recorded, as the cover editor does, and once the person is found, the
    # next list draws the cover again with the colour.
    file, plain = projects_dir(r) / 'reel.nuzky', {'x': 0, 'y': 0, 'scale': 1, 'rotation': 0, 'opacity': 1}
    set_cover(r, file, {'format': 'cover_9x16', 'timeUs': 1_000_000, 'frame': plain, 'texts': []},
              before=[behind(file, {'type': 'color', 'color': '#ff8000'})])
    # Changed as any edit changes it; set_cover keeps the seeded age.
    os.utime(file, (file.stat().st_mtime + 60,) * 2)
    r.s.call('window.__nuzky.refreshLibrary()')
    recorded = wait(lambda: r.s.run(POSTER_COLOURS, reel) > 40 and (p := r.s.run(PICTURE, reel, ORANGE)) and upright(p) and p, 20)
    found = subprocess.run([str(CLI), 'frame', str(file), '1', str(r.work / 'reel-frame.png')], env=r.env, capture_output=True,
                           text=True)
    r.s.call('window.__nuzky.refreshLibrary()')
    coloured = wait(lambda: (p := r.s.run(PICTURE, reel, ORANGE)) and recorded and p['share'] > recorded['share'] + 0.1 and p, 20)
    r.check('a cover drawn before the person is found in its clip shows the colour behind them once they are found',
            recorded and found.returncode == 0 and coloured,
            {'recorded': recorded, 'found': found.stderr[-300:], 'shown': r.s.run(PICTURE, reel, ORANGE)})
    croatia = card(r, 'Croatia day 2')['path']
    cover = lambda: (p := r.s.run(PICTURE, croatia, CYAN)) and p['share'] > 0.5 and upright(p) and p
    shown = wait(cover, 40)
    r.check('a landscape video with only a 9:16 cover shows it in that shape, as the cover editor draws it until the '
            'person is cut out', shown and r.s.run(POSTER_COLOURS, croatia) > 40,
            {'cyan': r.s.run(PICTURE, croatia, CYAN), 'colours': r.s.run(POSTER_COLOURS, croatia)})
    r.check('a cover drawn before the person is cut out is not kept', poster_cached(r, reel) and poster_cached(r, talk)
            and not poster_cached(r, croatia))
    # Each list asks for the draft again, in case the mask was made; until then nothing is drawn again and the card's
    # image stays as it is.
    before = (r.s.run(PICTURE, croatia, CYAN) or {}).get('src')
    for _ in range(3):
        r.s.call('window.__nuzky.refreshLibrary()')
        time.sleep(1)
    after = (r.s.run(PICTURE, croatia, CYAN) or {}).get('src')
    r.check('listing the projects again does not draw the draft cover again', drafts_drawn(r) == 1, drafts_drawn(r))
    r.check('the card keeps its image while the same draft comes back', before and after == before, {'before': before, 'after': after})
    # The person is found in the clip's frames and cut out of the cover's frame, as the cover editor's jobs do, which
    # leaves the project as it was. What is behind them is blurred now and the mask is made for those pixels: the next
    # list draws the cover from that frame, with the person cut out, and keeps it.
    file = projects_dir(r) / 'croatia.nuzky'
    stamp, draft = file.stat().st_mtime, shown
    masked = subprocess.run([str(CLI), 'mask', str(file), '1', str(r.work / 'croatia-mask.png')], env=r.env, capture_output=True,
                            text=True)
    r.s.call('window.__nuzky.refreshLibrary()')
    redrawn = lambda: (p := r.s.run(PICTURE, croatia, CYAN)) and draft and abs(p['share'] - draft['share']) > 0.02 and upright(p) and p
    shown = wait(redrawn, 20)
    r.check('once the person is cut out, the next list draws the cover with them cut out and keeps it',
            masked.returncode == 0 and file.stat().st_mtime == stamp and shown and wait(lambda: poster_cached(r, croatia), 5),
            {'mask': masked.stderr[-300:], 'draft': draft, 'shown': r.s.run(PICTURE, croatia, CYAN),
             'unchanged': file.stat().st_mtime == stamp})
    r.shot('home')
    if resize(r, 1024, 640):
        time.sleep(0.5)
        r.shot('home-1024')
        resize(r, 1440, 900)
    else:
        r.check('the window can be resized to its smallest size for a screenshot', False)

    # Collections: a new one, a project moved into it from its menu.
    click(r, 'New collection', 'nav[aria-label=Collections]')
    r.s.run(TYPE, "input[aria-label='New collection name']", 'Client work')
    press(r, 'Enter', "input[aria-label='New collection name']")
    r.check('a new collection shows and opens, empty with a hint',
            wait(lambda: r.s.run("return document.querySelector('main h1')?.textContent") == 'Client work'
                 and 'No projects in Client work yet' in r.s.run('return document.body.textContent'), 5))
    r.shot('empty-collection')
    click(r, 'All projects', 'nav[aria-label=Collections]')
    wait(lambda: len(cards(r)) == 4, 5)
    r.s.run("""const c = document.querySelector(`[data-path="${CSS.escape(arguments[0])}"]`); const b = c.getBoundingClientRect();
        c.dispatchEvent(new MouseEvent('contextmenu', {bubbles: true, clientX: b.left + 60, clientY: b.top + 80}));""", card(r, 'Croatia day 2')['path'])
    r.check('right-click opens the project menu', wait(lambda: r.s.run("return !!document.querySelector('[role=menu]')"), 3))
    click(r, 'Move to collection', '[role=menu]')
    time.sleep(0.3)
    r.shot('context-menu')
    menus = r.s.run("return [...document.querySelectorAll('[role=menu]')].map((m) => m.getAttribute('aria-label'))")
    r.check('Move to collection opens the list of collections', menus == ['Croatia day 2', 'Move to collection'], menus)
    # The pointer moves on to Rename: the list closes and the keys work in the project menu again.
    r.s.run("[...document.querySelectorAll('[role=menu]')][0].querySelectorAll('[role=menuitem]').forEach((i) => "
            "i.textContent.startsWith('Rename') && i.dispatchEvent(new PointerEvent('pointerover', {bubbles: true})))")
    focus = lambda: r.s.run("const m = document.querySelectorAll('[role=menu]'); return [m.length, document.activeElement === m[0]]")
    r.check('moving the pointer off the list closes it and the project menu keeps the keyboard', wait(lambda: focus() == [1, True], 3), focus())
    click(r, 'Move to collection', '[role=menu]')
    wait(lambda: len(r.s.run("return [...document.querySelectorAll('[role=menu]')]")) == 2, 3)
    r.s.run("[...document.querySelectorAll('[role=menu]')][1].querySelectorAll('[role=menuitemradio]').forEach((i) => i.textContent.includes('Client work') && i.click())")
    croatia = card(r, 'Croatia day 2')['path']
    library = r.work / 'data/nuzky/library.json'
    r.check('the project is in the collection, saved in library.json',
            wait(lambda: library.exists() and croatia in json.loads(library.read_text())['members'], 5)
            and wait(lambda: 'Client work' in card(r, 'Croatia day 2')['meta'], 5), card(r, 'Croatia day 2'))
    count = r.s.run("return [...document.querySelectorAll('nav[aria-label=Collections] [data-drop]')].map((b) => b.textContent)")
    r.check('the sidebar counts it', count == ['Client work1'], count)

    # Rename and duplicate a project that is not open, from the keyboard.
    focus_card(r, 'Croatia day 2')
    r.key('F2')
    r.check('F2 renames in place', wait(lambda: r.s.run("return document.activeElement?.getAttribute('aria-label')") == 'Project name', 3))
    r.s.run(TYPE, "input[aria-label='Project name']", 'Croatia day two')
    press(r, 'Enter', "input[aria-label='Project name']")
    r.check('the new name is saved in the project file',
            wait(lambda: json.loads((projects_dir(r) / 'croatia.nuzky').read_text())['name'] == 'Croatia day two', 5)
            and wait(lambda: card(r, 'Croatia day two'), 5))
    focus_card(r, 'Croatia day two')
    r.key('d', ctrlKey=True)
    copy = wait(lambda: card(r, 'Croatia day two copy'), 5)
    r.check('Ctrl+D duplicates into the same collection and focuses the copy', copy and 'Client work' in copy['meta']
            and wait(lambda: r.s.run('return document.activeElement?.dataset.path') == copy['path'], 3), copy)

    # The Trash, with Undo; after a while the file really goes to the system Trash, the videos stay.
    r.s.run("document.querySelector(`[data-path=\"${CSS.escape(arguments[0])}\"]`).focus()", copy['path'])
    r.key('Delete')
    first_move = time.time()
    r.check('Delete moves the copy to the Trash, the toast says the videos stay and offers Undo',
            wait(lambda: not card(r, 'Croatia day two copy'), 5)
            and any('Moved “Croatia day two copy” to the Trash. Your videos stay where they are.' == t for t in toast_texts(r)), toast_texts(r))
    r.shot('trash-toast')
    r.s.run("[...document.querySelectorAll('[data-toast]')].find((t) => t.textContent.includes('to the Trash')).querySelector('button').click()")
    r.check('Undo brings it back', wait(lambda: card(r, 'Croatia day two copy'), 5))
    # Moved again 8 s later: the first move's timer must not take this one. The check 33 s after the first move
    # falls 3 s after that timer and 5 s before the second one.
    time.sleep(8)
    r.s.run("document.querySelector(`[data-path=\"${CSS.escape(arguments[0])}\"]`).focus()", copy['path'])
    r.key('Delete')
    moved = time.time()
    # And a project an agent opens while it waits for the Trash stays, with a message.
    reel = card(r, 'Morning reel')['path']
    r.s.run("document.querySelector(`[data-path=\"${CSS.escape(arguments[0])}\"]`).focus()", reel)
    r.key('Delete')
    agent = Bridge(r, projects_dir(r) / 'reel.nuzky')
    try:
        time.sleep(max(0, first_move + 33 - time.time()))
        r.check('30 s after the first move, the copy moved again is still waiting for Undo', os.path.exists(copy['path'])
                and not card(r, 'Croatia day two copy'))
        # On macOS and Windows the Trash of the home the run gives the app (src-tauri/src/library.rs), never the user's.
        trashed = r.work / ('data/Trash/files' if LINUX else 'home/.Trash')
        r.check('30 s after the second move the copy is in the system Trash and gone from the projects',
                wait(lambda: trashed.exists() and any(p.suffix == '.nuzky' for p in trashed.iterdir()), max(5, moved + 45 - time.time()))
                and not os.path.exists(copy['path']), sorted(os.listdir(trashed)) if trashed.exists() else None)
        r.check('a project an agent opened meanwhile stays, and a toast says why',
                wait(lambda: any('“Morning reel” was opened in another Nuzky window or by an AI agent' in t for t in toast_texts(r)), 10)
                and os.path.exists(reel) and wait(lambda: card(r, 'Morning reel'), 5), toast_texts(r))
    finally:
        agent.close()
    r.check('the videos it used are untouched', (FIXTURES / 'wide-head.mp4').exists())
    open_card = card(r, 'Talk to camera')['path']
    r.s.run("document.querySelector(`[data-path=\"${CSS.escape(arguments[0])}\"]`).focus()", open_card)
    r.key('Delete')
    r.check('the open project cannot go to the Trash, a toast says why',
            wait(lambda: any('Open another project first' in t for t in toast_texts(r)), 3) and os.path.exists(open_card), toast_texts(r))

    # Search: names at once, then what was said once the project is transcribed.
    models = r.s.call('window.__nuzky.api.speechModels().then((m) => JSON.stringify(m))')
    small = next(m['id'] for m in json.loads(models['value']) if 'small' in m['id'] and m['downloaded'])
    job = r.s.call("window.__nuzky.api.startTranscript(arguments[0], 'en', false, window.__nuzky.store.getState().snap.sessionEpoch)", small)
    r.check('the open project is transcribed', job['ok'] and wait(lambda: any(j['kind'] == 'transcript' and j['status'] == 'done' for j in r.state()['jobs']), 240),
            r.state()['jobs'])
    r.s.run(TYPE, 'main input[type=search]', 'croatia')
    names = wait(lambda: [c['name'] for c in cards(r)] == ['Croatia day two'] and [c['name'] for c in cards(r)], 3)
    r.check('typing a name shows only the projects that match', names, [c['name'] for c in cards(r)])
    r.s.run(TYPE, 'main input[type=search]', 'laptop')
    quote = """const s = document.querySelector('section[aria-label="Said in your videos"]');
        return s && [...s.querySelectorAll('button')].map((b) => ({text: b.textContent, mark: b.querySelector('mark')?.textContent}));"""
    hits = wait(lambda: (r.s.run(quote) or None), 20)
    r.check('a word said in a video finds the project, with the sentence around it', hits and 'Talk to camera' in hits[0]['text']
            and hits[0]['mark'].lower().startswith('laptop'), hits)
    # The interview of Croatia day 2 has no sound, so only the reel has speech to transcribe.
    r.check('projects that are not transcribed are named, so nothing is silently missed',
            "Morning reel isn't transcribed" in (r.s.run("return document.querySelector('section[aria-label=\"Said in your videos\"]').textContent") or ''))
    r.shot('search-said')

    # In a collection, what was said in its projects is found even when ten newer projects elsewhere,
    # each saying "the" more than three times, fill the limit on results.
    talk = r.s.run("return window.__nuzky.library.getState().projects.find((p) => p.name === 'Talk to camera').path")
    for i in range(10):
        make(r, f'again-{i}.nuzky', f'Talk again {i}', [FIXTURES / 'talk.mp4'], -0.01)
    client = r.s.run("return window.__nuzky.library.getState().collections.find((c) => c.name === 'Client work').id")
    r.s.call('window.__nuzky.api.setCollection([arguments[0]], arguments[1]).then(() => window.__nuzky.refreshLibrary())', talk, client)
    click(r, 'Client work', 'nav[aria-label=Collections]')
    r.s.run(TYPE, 'main input[type=search]', 'the')
    hits = wait(lambda: (r.s.run(quote) or None), 20)
    r.check('in a collection, a word said in its project is found although newer projects elsewhere say it more',
            hits and any('Talk to camera' in h['text'] for h in hits), hits)
    r.s.run(TYPE, 'main input[type=search]', 'laptop')
    wait(lambda: (r.s.run(quote) or [{}])[0].get('mark', '').lower().startswith('laptop'), 20)
    r.s.run("document.querySelector('section[aria-label=\"Said in your videos\"] button').click()")
    v = wait(lambda: view(r)['view'] == 'editor' and view(r), 5)
    r.check('a found sentence opens its project there, with the transcript', v and v['name'] == 'Talk to camera' and v['timeUs'] > 1_000_000
            and v['tab'] == 'transcript', v)
    unexpected = [e for e in r.errors() if 'so it was not moved to the Trash' not in e]
    r.check('no other error toast', not unexpected, unexpected)

    # A cover made just before going home: the list shown at once has the project as it was, before its save lands.
    # The card shows the cover once it is saved, without a focus event or a restart, and nothing is listed after that.
    save_state = 'return window.__nuzky.store.getState().saveState'
    wait(lambda: r.s.run(save_state) == 'saved', 10)
    r.s.run("const api = window.__nuzky.api; window.__list = api.library; window.__listed = 0; api.library = () => (window.__listed++, window.__list());")
    hook = {'text': 'NEW', 'behind': False, 'transform': plain, 'style': {'fontSize': 400, 'color': '#ffffff', 'bold': True,
            'strokeWidth': 0, 'strokeColor': '#000000', 'background': '#ffffff'}}
    cover = {'format': 'cover_9x16', 'timeUs': 1_000_000, 'frame': plain, 'texts': [hook],
             'background': {'picture': True, 'blur': 0, 'dim': 0, 'color': '#000000'}}
    edited = r.s.call("window.__nuzky.store.getState().edit({type: 'setThumbnail', thumbnail: arguments[0]}).then((s) => s?.revision)", cover)
    saving = r.s.run(save_state)
    close_window()
    home_state = wait(lambda: view(r)['view'] == 'home' and r.s.run(save_state), 5)
    r.check('closing the window right after the cover changed goes home while the project is still saving',
            edited['ok'] and edited['value'] and saving == 'saving' and home_state == 'saving', (edited, saving, home_state))
    WHITE = (255, 255, 255)
    shown = wait(lambda: (p := r.s.run(PICTURE, talk, WHITE)) and p['share'] > 0.05 and upright(p) and p, 20)
    r.check('once the save lands, the card shows the new cover', shown and r.s.run(save_state) == 'saved',
            {'card': r.s.run(PICTURE, talk, WHITE), 'saved': r.s.run(save_state)})
    listed = r.s.run('return window.__listed')
    # The editor's state changes without a save, as with a toast or the progress of an export.
    r.s.run("window.__nuzky.store.getState().toast({kind: 'info', text: 'Nothing saved'})")
    time.sleep(3)
    r.check('then nothing is listed again while nothing is saved', listed and r.s.run('return window.__listed') == listed,
            (listed, r.s.run('return window.__listed')))
    r.s.run('window.__nuzky.api.library = window.__list')
    r.shot('home-after-cover')

    # Collections that cannot be read are never written over.
    index = r.work / 'data/nuzky/library.json'
    before = index.read_bytes()
    allow = deny(index, 0, 'R,W')
    try:
        made = r.s.call("window.__nuzky.api.createCollection('Lost')")
    finally:
        allow()
    r.check('a new collection is refused while the collections cannot be read, and they stay as they were',
            not made['ok'] and index.read_bytes() == before, made)

    # A list that was on its way when a project went to the Trash does not bring it back.
    names = "return Object.fromEntries(window.__nuzky.library.getState().projects.map((p) => [p.name, p.path]))"
    croatia = r.s.run(names)['Croatia day two']
    r.s.call("""(async () => {
        const c = window.__nuzky, real = c.api.library;
        let release;
        const gate = new Promise((ok) => (release = ok));
        c.api.library = () => real().then((list) => gate.then(() => list));
        const listed = c.refreshLibrary();
        await new Promise((ok) => setTimeout(ok, 500));
        c.api.library = real;
        await c.trashProjects([arguments[0]]);
        release();
        await listed;
    })()""", croatia)
    shown = r.s.run(names)
    r.check('a list on its way when a project goes to the Trash does not bring it back', 'Croatia day two' not in shown, sorted(shown))
    r.s.call('window.__nuzky.api.restoreProjects([arguments[0]]).then(() => window.__nuzky.refreshLibrary())', croatia)

    # Closing the window moves what waits for the Trash. A project an agent opened meanwhile stays,
    # and the window stays open to say why.
    r.s.run("window.__nuzky.store.setState({view: 'home', toasts: []})")
    reel = r.s.run(names)['Morning reel']
    r.s.call('window.__nuzky.trashProjects([arguments[0]])', reel)
    agent = Bridge(r, Path(reel))
    try:
        close_window()

        def told():
            try:
                return any('“Morning reel” was opened in another Nuzky window or by an AI agent' in t for t in toast_texts(r))
            except Exception:
                return False
        r.check('closing the window while an agent holds a project waiting for the Trash: the window stays open and says why',
                wait(told, 10) and os.path.exists(reel))
    finally:
        agent.close()


# Records when the app maximizes its window; a UI flow keeps its window as it is (src-tauri/src/lib.rs, maximize_window).
MAXIMIZED = """const api = window.__nuzky.api, maximize = api.maximizeWindow;
window.__maximized = [];
api.maximizeWindow = () => (window.__maximized.push('asked'), maximize().then(() => window.__maximized.push('done')));"""


def launcher_seed(r):
    make(r, 'talk.nuzky', 'Talk to camera', [FIXTURES / 'talk.mp4'], 0)
    make(r, 'reel.nuzky', 'Morning reel', [FIXTURES / 'portrait.mp4'], 1)
    make(r, 'croatia.nuzky', 'Croatia day 2', [FIXTURES / 'wide.mp4'], 2)


@flow('launcher', 'Over the editor, Projects or Ctrl+K opens the launcher: open, start or find a project. Switching '
      'while an agent edits asks whether to keep or undo its changes, and an agent holding a project shows it busy',
      before=launcher_seed)
def launcher(r):
    r.check('the flow starts in the editor of the newest project', view(r)['view'] == 'editor' and view(r)['name'] == 'Talk to camera', view(r))
    r.s.run(MAXIMIZED)
    r.s.run('document.activeElement?.blur()')
    r.key('k', ctrlKey=True)
    r.check('Ctrl+K opens the launcher with the open project and the recent ones',
            wait(lambda: r.s.run("return [...document.querySelectorAll('#launcher-list [role=option]')].map((o) => o.textContent)"), 5))
    options = wait(lambda: len(r.s.run("return [...document.querySelectorAll('#launcher-list [role=option]')]")) == 4
                   and r.s.run("return [...document.querySelectorAll('#launcher-list [role=option]')].map((o) => o.textContent)"), 10)
    r.check('Continue, the two other projects and All projects', options and 'Talk to camera' in options[0] and 'Morning reel' in options[1]
            and 'All projects' in options[3], options)
    r.check('the launcher offers empty projects by name',
            'Empty project' in (r.s.run("return document.querySelector('[role=dialog][aria-label=Projects]').textContent") or ''))
    r.check('focus is in the search field', r.s.run("return document.activeElement?.getAttribute('role')") == 'combobox')
    time.sleep(0.3)
    r.shot('launcher')
    if resize(r, 1024, 640):
        time.sleep(0.5)
        r.shot('launcher-1024')
        resize(r, 1440, 900)
    r.s.run(TYPE, '[role=combobox]', 'croat')
    r.check('typing narrows to the project', wait(lambda: 'Croatia day 2' in (r.s.run("return document.querySelector('#launcher-list [role=option][aria-selected=true]')?.textContent") or ''), 5))
    press(r, 'Enter', '[role=combobox]')
    r.check('Enter opens it in the editor', wait(lambda: view(r)['name'] == 'Croatia day 2' and not view(r)['launcher'], 10), view(r))
    r.check('switching projects in the editor leaves the window as it is', r.s.run('return window.__maximized') == [])
    r.check('opening a project counts as opening it: it is first in the list next time',
            wait(lambda: r.s.call('window.__nuzky.api.library().then((l) => l.projects[0].name)')['value'] == 'Croatia day 2', 5))

    # An agent edits the open project; opening another one asks what happens to its changes.
    croatia = r.work / 'data/nuzky/projects/croatia.nuzky'
    bridge = Bridge(r, croatia)
    try:
        # The user's own change, then the agent's. Undo changes while the new project cannot start
        # takes back only the agent's, and its toast offers no Undo that would take the user's.
        r.s.call("window.__nuzky.store.getState().edit({type: 'renameProject', name: 'Croatia, day 2'})")
        wait(lambda: view(r)['name'] == 'Croatia, day 2', 5)
        run = bridge.call('begin_run', {'label': 'Split for a look'})
        wait(lambda: view(r)['aiRun'], 10)
        bridge.call('apply_edits', {'run_id': run['run_id'], 'request_id': 'look', 'edits': [{'type': 'splitClip', 'clipId': r.track()[0]['id'], 'atUs': 1_000_000}]})
        wait(lambda: len(r.track()) == 2, 10)
        r.s.call('window.__nuzky.newProjectFromMedia(arguments[0])', [str(r.work / 'missing.mp4')])
        wait(lambda: r.s.run("return document.getElementById('switch-title')?.textContent"), 5)
        click(r, 'Undo changes and start')
        stayed = wait(lambda: any('None of those files could be opened' in e for e in r.errors()) and len(r.track()) == 1 and view(r), 10)
        r.check("Undo changes and start with videos that cannot open: the agent's edit is gone, the project and the user's change stay",
                stayed and stayed['name'] == 'Croatia, day 2', view(r))
        run_toasts = """return [...document.querySelectorAll('[data-toast]')].filter((t) => t.textContent.includes('Split for a look'))
            .map((t) => ({text: t.textContent, undo: [...t.querySelectorAll('button')].some((b) => b.textContent === 'Undo')}));"""
        ended = wait(lambda: r.s.run(run_toasts), 5)
        r.check('its toast says the AI edit was undone and offers no Undo, which would take back the change before it',
                ended and all('Undid the AI edit' in t['text'] and not t['undo'] for t in ended), ended)

        # Undo changes that cannot be saved: the run stays, and Stop and edit later still takes it back.
        run = bridge.call('begin_run', {'label': 'Split once more'})
        wait(lambda: view(r)['aiRun'], 10)
        bridge.call('apply_edits', {'run_id': run['run_id'], 'request_id': 'more', 'edits': [{'type': 'splitClip', 'clipId': r.track()[0]['id'], 'atUs': 3_000_000}]})
        wait(lambda: len(r.track()) == 2, 10)
        r.s.run("window.__nuzky.store.setState({toasts: []})")
        r.s.call('window.__nuzky.newProjectFromMedia(arguments[0])', [str(r.work / 'missing.mp4')])
        wait(lambda: r.s.run("return document.getElementById('switch-title')?.textContent"), 5)
        allow = deny(projects_dir(r), 0o555)
        try:
            click(r, 'Undo changes and start')
            refused = wait(lambda: r.errors(), 10)
        finally:
            allow()
        r.check('Undo changes that cannot be saved says so and leaves the run open', refused and view(r)['aiRun'], refused)
        click(r, 'Stop and edit')
        more = """return [...document.querySelectorAll('[data-toast]')].filter((t) => t.textContent.includes('Split once more'))
            .map((t) => ({text: t.textContent, undo: [...t.querySelectorAll('button')].some((b) => b.textContent === 'Undo')}));"""
        ended = wait(lambda: r.s.run(more), 10)
        r.check('Stop and edit afterwards still takes the AI edit back, and offers no Undo of the change before it',
                ended and all('Undid the AI edit' in t['text'] and not t['undo'] for t in ended) and len(r.track()) == 1
                and view(r)['name'] == 'Croatia, day 2', ended)
        r.s.run("window.__nuzky.store.setState({toasts: []})")

        run = bridge.call('begin_run', {'label': 'Trim the start'})
        r.check('the AI run shows', wait(lambda: view(r)['aiRun'], 10))
        clip = r.track()[0]
        bridge.call('apply_edits', {'run_id': run['run_id'], 'request_id': 'split', 'edits': [{'type': 'splitClip', 'clipId': clip['id'], 'atUs': 2_000_000}]})
        r.check("the agent's edit is on the timeline and saved", wait(lambda: len(r.track()) == 2, 10)
                and wait(lambda: len(json.loads(croatia.read_text())['tracks'][0]['clips']) == 2, 5))
        r.key('k', ctrlKey=True)
        wait(lambda: r.s.run("return !!document.querySelector('[role=combobox]')"), 5)
        r.s.run(TYPE, '[role=combobox]', 'morning')
        wait(lambda: 'Morning reel' in (r.s.run("return document.querySelector('#launcher-list [role=option][aria-selected=true]')?.textContent") or ''), 5)
        press(r, 'Enter', '[role=combobox]')
        question = wait(lambda: r.s.run("return document.getElementById('switch-title')?.textContent"), 5)
        r.check('opening another project asks what happens to the AI edit', question == 'Stop the AI edit and open “Morning reel”?', question)
        r.check('Keep changes and open is the default', r.s.run('return document.activeElement?.textContent') == 'Keep changes and open')
        time.sleep(0.3)
        r.shot('switch-confirm')
        click(r, 'Undo changes and open')
        r.check('Undo changes and open takes the AI edit back, saved, and opens the other project',
                wait(lambda: view(r)['name'] == 'Morning reel', 10)
                and wait(lambda: len(json.loads(croatia.read_text())['tracks'][0]['clips']) == 1, 5), view(r))
    finally:
        bridge.close()

    # An agent holding a project that is not open here: the home screen shows it busy and does not open it.
    talk = r.work / 'data/nuzky/projects/talk.nuzky'
    holder = Bridge(r, talk)
    try:
        holder.call('get_state', {})
        r.key('k', ctrlKey=True)
        wait(lambda: r.s.run("return !!document.querySelector('[role=combobox]')"), 5)
        click(r, 'All projects', '#launcher-list')
        r.check('All projects goes to the home screen', wait(lambda: view(r)['view'] == 'home', 5))
        busy = wait(lambda: (card(r, 'Talk to camera') or {}).get('meta') == 'In use elsewhere' and card(r, 'Talk to camera'), 10)
        r.check('a project an agent has open shows In use elsewhere and cannot open', busy and busy['disabled'], card(r, 'Talk to camera'))
        r.shot('home-busy')
    finally:
        holder.close()

    # New project in a format, and from videos in the first video's format.
    count = len(list(projects_dir(r).glob('*.nuzky')))
    r.s.run("document.querySelector('[data-new-project]').click()")
    wait(lambda: r.s.run("return !!document.querySelector('[role=menu]')"), 3)
    time.sleep(0.2)
    r.shot('new-project-menu')
    click(r, 'Empty 16:9', '[role=menu]')
    v = wait(lambda: view(r)['view'] == 'editor' and view(r)['name'] == 'Untitled project' and view(r), 10)
    r.check('New project, 16:9, opens an empty 16:9 project', v and v['canvas'] == [1920, 1080], v)
    r.check('opening the editor from the home screen asks the app to maximize the window',
            wait(lambda: r.s.run('return window.__maximized') == ['asked', 'done'], 5), r.s.run('return window.__maximized'))
    r.check('it is a new file', len(list(projects_dir(r).glob('*.nuzky'))) == count + 1)
    r.s.call('window.__nuzky.newProjectFromMedia(arguments[0])', [str(FIXTURES / 'portrait.mp4')])
    v = wait(lambda: view(r)['canvas'] == [1080, 1920] and len(r.track()) == 1 and view(r), 15)
    r.check('a new project from a portrait video is 9:16 with the video on the timeline', v, view(r))
    r.check('the empty project left behind is cleared away once the list is shown again',
            r.s.call('window.__nuzky.api.library().then((l) => l.projects.filter((p) => p.state === "empty").length)')['value'] == 0
            and len(list(projects_dir(r).glob('*.nuzky'))) == count + 1)

    # Asked one after the other while the first is still reading its videos: the one asked last stays open.
    reel = str(projects_dir(r) / 'reel.nuzky')
    r.s.run("window.__nuzky.newProjectFromMedia(arguments[0]); window.__nuzky.openProject({path: arguments[1], name: 'Morning reel'});",
            [str(FIXTURES / name) for name in ('talk.mp4', 'wide.mp4', 'portrait.mp4') * 3], reel)
    opened = wait(lambda: view(r)['name'] == 'Morning reel', 15)
    time.sleep(3)
    r.check('a new project from videos, then Morning reel at once: Morning reel is what stays open',
            opened and view(r)['name'] == 'Morning reel', view(r))
    unexpected = [e for e in r.errors() if 'None of those files could be opened' not in e]
    r.check('no other error toast', not unexpected, unexpected)


FIRST_RUN_TEXT = "return document.querySelector('main')?.textContent ?? ''"
FIRST_STEPS = ('Captions in your language', 'Cut pauses and retakes', 'Export for Reels & TikTok')
OPEN_VIDEO = 'Open a talking head video'
NAMED_LANGUAGES = """const steps = [...document.querySelectorAll('main ol li')].map((li) => li.textContent).join(' ');
const names = new Intl.DisplayNames(['en'], {type: 'language'});
return [...new Set(['en', 'cs', 'de', 'es', 'fr', 'it', 'pt', 'pl', 'ru', 'uk', 'ja', 'zh', 'ko', 'nl', 'sv', 'tr', 'ar', 'hi', navigator.language]
  .map((code) => names.of(code)))].filter((name) => steps.includes(name));"""
# The native file dialog cannot be driven, so it answers with the given path. Tauri's `invoke` cannot be replaced, so
# the answer comes from the fetch that carries the call to the app.
ANSWER_FILE_DIALOG = """const fetch = window.fetch, picked = arguments[0];
window.fetch = (url, init) => decodeURIComponent(String(url)).includes('plugin:dialog|open')
  ? Promise.resolve(new Response(JSON.stringify([picked]), {headers: {'Content-Type': 'application/json', 'Tauri-Response': 'ok'}}))
  : fetch(url, init);"""
CLIPPED = """const pane = document.querySelector('main .overflow-y-auto'), p = pane.getBoundingClientRect();
const out = [...pane.querySelectorAll('h2, button, ol li, p')].filter((el) => {
  const b = el.getBoundingClientRect(); return b.width && (b.left < p.left || b.right > p.right); }).map((el) => el.textContent);
return pane.scrollWidth > pane.clientWidth ? [...out, `the pane scrolls sideways: ${pane.scrollWidth} > ${pane.clientWidth}`] : out;"""
CONNECT_OPEN = "return !!document.querySelector('[role=dialog][aria-labelledby=connect-title]')"
SETUP_OPEN = "return !!document.querySelector('[role=dialog][aria-labelledby=setup-title]')"


@flow('first-run', 'The first start leads from a talking head video to captions, cuts and a vertical export; the video '
      'starts a project, and a format still shapes the project Nuzky made instead of leaving a second empty file', home=True)
def first_run(r):
    text = lambda: r.s.run(FIRST_RUN_TEXT) or ''
    r.check('the first start offers Open a talking head video and the three steps',
            wait(lambda: OPEN_VIDEO in text() and all(step in text() for step in FIRST_STEPS), 10), text())
    named = r.s.run(NAMED_LANGUAGES)
    r.check('no step names a language', named == [], named)
    r.shot('first-run')
    if r.check('the window goes to its smallest size', resize(r, 1024, 640)):
        time.sleep(0.5)
        clipped = r.s.run(CLIPPED)
        r.check('nothing of the first start is cut off at the sides', clipped == [], clipped)
        r.shot('first-run-1024')
        resize(r, 1440, 900)

    click(r, '16:9', 'main')
    v = wait(lambda: view(r)['view'] == 'editor' and view(r), 5)
    r.check('a format opens the editor with that format', v and v['canvas'] == [1920, 1080], v)
    r.check('no second project file', wait(lambda: len(list(projects_dir(r).glob('*.nuzky'))) == 1, 3),
            sorted(p.name for p in projects_dir(r).iterdir()))
    r.s.run("window.__nuzky.store.setState({view: 'home'})")
    r.check('the project is still empty, so home still shows the first start', wait(lambda: OPEN_VIDEO in text(), 5), text())

    r.check('the first start asks Want AI to do the cutting?', 'Want AI to do the cutting?' in text(), text())
    r.check('connect an agent is a button', click(r, 'connect an agent', 'main'))
    if r.check('it opens the Connect dialog', wait(lambda: r.s.run(CONNECT_OPEN), 5)):
        r.key('Escape')
        r.check('Esc closes the Connect dialog', wait(lambda: not r.s.run(CONNECT_OPEN), 5))
    r.check('Get set up is a button', click(r, 'Get set up', 'main'))
    r.check('it opens Get set up over the first start', wait(lambda: r.s.run(SETUP_OPEN), 5) and view(r)['view'] == 'home', view(r))
    r.s.run("document.querySelector('[aria-labelledby=setup-title] button[aria-label=Close]').click()")
    r.check('× closes Get set up', wait(lambda: not r.s.run(SETUP_OPEN), 5))
    r.check('back home, the first start still shows', wait(lambda: OPEN_VIDEO in text(), 5), text())

    r.s.run(ANSWER_FILE_DIALOG, str(FIXTURES / 'talk.mp4'))
    r.check('Open a talking head video is a button', click(r, OPEN_VIDEO, 'main'))
    v = wait(lambda: view(r)['view'] == 'editor' and len(r.track()) == 1 and view(r), 15)
    assets = [a['name'] for a in r.state()['assets']]
    r.check('it opens the editor with the video on the timeline', v and assets and 'talk' in assets[0], (view(r), assets))
    r.s.run("window.__nuzky.store.setState({view: 'home'})")
    r.check('after the first video, home shows the project instead of the first start',
            wait(lambda: OPEN_VIDEO not in text() and any(c['path'] == view(r)['path'] and c['meta'] == 'Open now' for c in cards(r)), 10), (text(), cards(r)))
    time.sleep(0.5)
    r.shot('home-after-first-video')

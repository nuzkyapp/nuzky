"""Opening a CapCut draft as a project: the home screen offers CapCut's drafts newest first, a draft opens as a new
project whose clips play where CapCut had them (checked in the preview's pixels against FFmpeg's frames of the media),
a dialog lists what did not come over, Finish with AI asks the AI panel's agent to finish it, a draft opened before
opens its project again, an encrypted one says why it cannot open, and the draft itself is never changed."""
import hashlib, json, subprocess, time
from pathlib import Path

from e2e.ai_panel import FAKE, runs
from e2e.harness import MACOS, flow, preview_crop, preview_rect, wait
from e2e.home import projects_dir, resize, toast_texts, view

DRAFTS = 'Movies/CapCut/User Data/Projects/com.lveditor.draft'
# A colour for every second of the recording, so a pixel says which second of it plays.
PALETTE = ['FF0000', '00FF00', '0000FF', 'FFFF00', '00FFFF', 'FF00FF', '800000', '008000', '000080', '808000', '008080', '800080']
# Grey, which every colour matrix draws the same.
PHOTO = '808080'
FINISH = 'Finish this draft in my style.'
DRAFT_CARDS = """return [...document.querySelectorAll('section[aria-label="CapCut drafts"] button[data-draft]')].map((b) => ({
  path: b.dataset.draft, name: b.querySelector('[data-name]').textContent, text: b.innerText, label: b.getAttribute('aria-label')}));"""
SECTION = """const s = document.querySelector('section[aria-label="CapCut drafts"]');
return s && !s.hidden ? s.querySelector('h2').textContent : null;"""
DIALOG = 'dialog[aria-labelledby=opened-draft-title]'
DIALOG_STATE = f"""const d = document.querySelector('{DIALOG}'); if (!d?.open) return null;
return {{title: d.querySelector('h2').textContent, lines: [...d.querySelectorAll('li')].map((l) => l.innerText.replace(/\\n/g, ' ')),
         text: d.innerText}};"""
PROJECT = "return window.__nuzky.store.getState().snap.project;"
CLICK_CARD = """const b = [...document.querySelectorAll('section[aria-label="CapCut drafts"] button[data-draft]')]
  .find((b) => b.querySelector('[data-name]').textContent === arguments[0]); if (!b) return false; b.focus(); b.click(); return true;"""
MENU_ITEM = """const m = [...document.querySelectorAll('[role=menu] [role=menuitem]')].find((m) => m.textContent.trim().startsWith(arguments[0]));
if (!m) return false; m.click(); return true;"""
CLICK = """const b = [...document.querySelectorAll(arguments[1] + ' button')].find((b) => b.textContent.trim().startsWith(arguments[0]));
if (!b) return false; b.click(); return true;"""


def run(*command):
    subprocess.run(command, check=True, capture_output=True)


def media(folder):
    """A 12 s recording, a colour each second with a white square in the middle and a tone; a photo; a song."""
    folder.mkdir(parents=True)
    sources = ''.join(f'color=c=0x{c}:s=540x960:r=30:d=1[c{i}];' for i, c in enumerate(PALETTE))
    joined = ''.join(f'[c{i}]' for i in range(len(PALETTE)))
    run('ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'sine=frequency=440:duration=12', '-filter_complex',
        f'{sources}{joined}concat=n={len(PALETTE)}:v=1:a=0,drawbox=x=220:y=430:w=100:h=100:color=white:t=fill[v]',
        '-map', '[v]', '-map', '0:a', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-c:a', 'aac',
        # Tagged, so Nuzky and FFmpeg turn it into the same RGB.
        '-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', str(folder / 'talk.mp4'))
    run('ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', f'color=c=0x{PHOTO}:s=540x960', '-frames:v', '1', str(folder / 'photo.png'))
    run('ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'sine=frequency=220:duration=10', str(folder / 'song.mp3'))
    # A file that is there but is no video, as a render CapCut never finished.
    (folder / 'broken.mp4').write_bytes(b'not a video')


def segment(material, start, duration, source=None, speed=1.0, **extra):
    s = {'material_id': material, 'target_timerange': {'start': start, 'duration': duration}, 'speed': speed, 'volume': 1.0,
         'clip': {'scale': {'x': 1.0, 'y': 1.0}, 'transform': {'x': 0.0, 'y': 0.0}, 'rotation': 0.0}}
    if source is not None:
        s['source_timerange'] = {'start': source, 'duration': round(duration * speed)}
    s.update(extra)
    return s


def text(id, words, kind='subtitle', shadow=False):
    style = {'fill': {'content': {'solid': {'color': [1.0, 1.0, 1.0]}}}, 'size': 12, 'range': [0, len(words)],
             'strokes': [{'content': {'solid': {'color': [0.0, 0.0, 0.0]}}, 'width': 0.06}]}
    return {'id': id, 'type': kind, 'has_shadow': shadow, 'line_max_width': 0.82, 'alignment': 1,
            'content': json.dumps({'text': words, 'styles': [style]})}


def draft(folder, name, files, modified_s):
    """A CapCut for macOS draft: four clips back to back on the main track, the first zooming in and colour adjusted,
    the second a file that is not on this computer, the third twice as fast after a Woosh, the fourth after a dissolve;
    a photo over the start, a hidden copy of the recording on top, captions (one with a shadow), a title, and the
    song muted with a fade in."""
    folder.mkdir(parents=True)
    talk, photo, song = files / 'talk.mp4', files / 'photo.png', files / 'song.mp3'
    main = [
        segment('talk', 0, 2_000_000, 0, volume=0.5, extra_material_refs=['bright'], common_keyframes=[{'property_type': 'KFTypeScaleX', 'keyframe_list': [
            {'time_offset': 0, 'values': [1.0], 'curveType': 'Line'}, {'time_offset': 2_000_000, 'values': [1.5], 'curveType': 'Line'}]}]),
        segment('gone', 2_000_000, 1_000_000, 0, extra_material_refs=['woosh']),
        segment('talk', 3_000_000, 2_000_000, 5_000_000, speed=2.0, extra_material_refs=['mix']),
        segment('talk', 5_000_000, 2_000_000, 9_000_000),
    ]
    overlay = segment('photo', 500_000, 2_000_000, 0)
    overlay['clip'] = {'scale': {'x': 0.4, 'y': 0.4}, 'transform': {'x': -0.5, 'y': 0.5}, 'rotation': 0.0}
    captions = [segment(f'cap{i}', i * 2_000_000, 2_000_000) for i in range(3)]
    for c in captions:
        c['clip']['transform']['y'] = -0.5
    title = segment('title', 0, 3_000_000)
    title['clip']['transform']['y'] = 0.6
    content = {
        'fps': 30.0, 'canvas_config': {'width': 540, 'height': 960, 'ratio': 'original'},
        'platform': {'app_source': 'cc', 'app_version': '9.5.0', 'os': 'mac'},
        'tracks': [
            {'type': 'video', 'attribute': 0, 'segments': main},
            {'type': 'video', 'attribute': 0, 'segments': [overlay]},
            {'type': 'video', 'attribute': 0, 'segments': [segment('broken', 2_000_000, 1_000_000, 0)]},
            {'type': 'video', 'attribute': 2, 'segments': [segment('talk', 0, 7_000_000, 0)]},
            {'type': 'text', 'attribute': 0, 'segments': captions},
            {'type': 'text', 'attribute': 0, 'segments': [title]},
            {'type': 'audio', 'attribute': 1, 'segments': [segment('song', 0, 7_000_000, 0, volume=0.3, extra_material_refs=['fade'])]},
            {'type': 'effect', 'attribute': 0, 'segments': []},
        ],
        'materials': {
            'videos': [{'id': 'talk', 'type': 'video', 'path': str(talk), 'material_name': 'talk.mp4', 'width': 540, 'height': 960},
                       {'id': 'gone', 'type': 'video', 'path': str(files / 'gone/gone.mov'), 'material_name': 'gone.mov', 'width': 540, 'height': 960},
                       {'id': 'broken', 'type': 'video', 'path': str(files / 'broken.mp4'), 'material_name': 'broken.mp4', 'width': 540, 'height': 960},
                       {'id': 'photo', 'type': 'photo', 'path': str(photo), 'material_name': 'photo.png', 'width': 540, 'height': 960}],
            'audios': [{'id': 'song', 'type': 'music', 'path': str(song), 'name': 'song.mp3'}],
            'transitions': [{'id': 'woosh', 'type': 'transition', 'name': 'Woosh', 'duration': 466_666, 'is_overlap': True},
                            {'id': 'mix', 'type': 'transition', 'name': 'Mix', 'duration': 400_000, 'is_overlap': True}],
            'effects': [{'id': 'bright', 'type': 'brightness', 'value': 0.3}],
            'audio_fades': [{'id': 'fade', 'type': 'audio_fade', 'fade_in_duration': 1_000_000, 'fade_out_duration': 0}],
            'texts': [text('cap0', 'Hello there'), text('cap1', 'big zoom', shadow=True), text('cap2', 'and more'),
                      text('title', 'My title', kind='text')],
        },
    }
    (folder / 'draft_info.json').write_text(json.dumps(content))
    (folder / 'draft_meta_info.json').write_text(json.dumps({'draft_name': name, 'tm_draft_modified': int(modified_s * 1e6), 'tm_duration': 7_000_000}))
    (folder / 'draft_cover.jpg').write_bytes((files / 'photo.png').read_bytes())


def fingerprint(folder):
    return {str(p.relative_to(folder)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(folder.rglob('*')) if p.is_file()}


def setup(r):
    files = r.work / 'media'
    media(files)
    home = Path(r.env['HOME']) if MACOS else r.work / 'home'
    r.env['HOME'] = str(home)
    root = home / DRAFTS
    now = time.time()
    r.reel, r.older = root / '0928', root / '0915'
    draft(r.reel, 'Talking reel', files, now - 3600)
    draft(r.older, 'Older reel', files, now - 86400 * 3)
    (root / '1009').mkdir(parents=True)
    (root / '1009/draft_content.json').write_text('Hk3vQ0bS8vZ2Yz4uXw9pR1tL7mN5kJ2hG6fD0sA8qW3eR5tY7uI9oP1aS3dF5gH7jK9lZ1xC3vB5nM7')
    (root / '1009/draft_meta_info.json').write_text(json.dumps({'draft_name': 'From JianYing', 'tm_draft_modified': int((now - 86400) * 1e6), 'tm_duration': 5_000_000}))
    (root / '0825').mkdir()
    (root / '0825/draft_info.json').write_text(json.dumps({'tracks': [], 'materials': {}}))
    (root / '0825/draft_meta_info.json').write_text(json.dumps({'draft_name': 'Empty', 'tm_draft_modified': int(now * 1e6), 'tm_duration': 0}))
    r.elsewhere = r.work / 'Backup/Short reel'
    draft(r.elsewhere, 'Short reel', files, now - 86400 * 30)
    draft(r.work / 'Backup/Second reel', 'Second reel', files, now - 86400 * 31)
    r.prints = {d: fingerprint(d) for d in (r.reel, r.older, r.elsewhere)}
    # The fake stands in for Claude Code, so Finish with AI never spends anyone's subscription.
    r.env.update(NUZKY_AGENT_CLAUDE=str(FAKE), NUZKY_AGENT_CODEX=str(r.work / 'no-codex'), NUZKY_FAKE_AGENT_LOG=str(r.work / 'fake'))
    r.files = files


def reference(r, seconds):
    """The pixel at 30 % across and 75 % down of the recording's frame at `seconds`, as FFmpeg decodes it."""
    from PIL import Image
    out = r.work / f'ref-{seconds}.png'
    run('ffmpeg', '-v', 'error', '-y', '-ss', str(seconds), '-i', str(r.files / 'talk.mp4'), '-frames:v', '1', str(out))
    image = Image.open(out).convert('RGB')
    return image.getpixel((int(image.width * 0.3), int(image.height * 0.75)))


def preview_pixel(r, us, name, at=(0.3, 0.75)):
    r.seek(us)
    time.sleep(1.5)
    r.shot(name)
    crop = preview_crop(r.work / f'{name}.png', preview_rect(r))
    return crop.getpixel((int(crop.width * at[0]), int(crop.height * at[1])))


def near(a, b, tolerance=28):
    return all(abs(x - y) <= tolerance for x, y in zip(a, b))


def hex_rgb(colour):
    return tuple(int(colour[i:i + 2], 16) for i in (0, 2, 4))


@flow('capcut-open', 'Open a CapCut draft as a project: the home screen offers the drafts newest first, the clips play where '
                     'CapCut had them, one list says what did not come over, Finish with AI, reopening, an encrypted draft, '
                     'and the draft stays unchanged', before=setup, home=True)
def capcut_open(r):
    r.check('the first start offers CapCut\'s drafts under it', wait(lambda: r.s.run(SECTION) == 'Continue a CapCut draft', 15), r.s.run(SECTION))
    aligned = r.s.run("""const s = document.querySelector('section[aria-label="CapCut drafts"]').getBoundingClientRect();
        const t = document.querySelector('main ol').getBoundingClientRect();
        return Math.abs(s.left - t.left) < 2 && Math.abs(s.right - t.right) < 2""")
    r.check('as wide as the first-start steps and lined up with them', aligned)
    found = wait(lambda: r.s.run(DRAFT_CARDS), 10) or []
    names = [c['name'] for c in found]
    r.check('newest first, by name, without the empty one', names == ['Talking reel', 'From JianYing', 'Older reel'], names)
    first = found[0]['text'].split('\n') if found else []
    r.check('a card shows its length, name and when it changed', '0:07' in first and 'Talking reel' in first and '1 h ago' in first, first)
    cover = "return document.querySelector('section[aria-label=\"CapCut drafts\"] button[data-draft] img')?.naturalWidth ?? 0"
    r.check('and the picture CapCut keeps of the draft', wait(lambda: r.s.run(cover) > 0, 10), r.s.run(cover))
    r.shot('drafts-first-start')
    # One Tab stop, the arrow keys move between the cards.
    tabs = r.s.run("return [...document.querySelectorAll('section[aria-label=\"CapCut drafts\"] button[data-draft]')].map((b) => b.tabIndex)")
    r.s.run("document.querySelector('section[aria-label=\"CapCut drafts\"] button[data-draft]').focus()")
    r.key('ArrowRight')
    moved = r.s.run("return document.activeElement?.querySelector('[data-name]')?.textContent ?? null")
    r.check('the cards are one Tab stop and the arrow keys move between them', tabs == [0, -1, -1] and moved == 'From JianYing', [tabs, moved])

    r.check('clicking a draft opens it', r.s.run(CLICK_CARD, 'Talking reel'))
    dialog = wait(lambda: r.s.run(DIALOG_STATE), 20)
    v = view(r)
    r.check('as a new project in the editor, named as in CapCut', v['view'] == 'editor' and v['name'] == 'Talking reel' and v['canvas'] == [540, 960], v)
    r.check('a dialog says it opened from CapCut and what came over',
            dialog and dialog['title'] == 'Opened “Talking reel” from CapCut' and '7 clips, 1 sound' in dialog['text'] and '3 captions' in dialog['text'] and '1 title' in dialog['text'], dialog)
    lines = dialog['lines'] if dialog else []
    wanted = ['Missing media: gone.mov', 'Media Nuzky cannot open: broken.mp4', 'Transition: Woosh', 'Text shadows', 'Effects', 'Color adjustments']
    r.check('and lists in one list everything that did not come over, missing media first',
            all(any(l.startswith(w) for l in lines) for w in wanted) and {l.split(':')[0] for l in lines[:2]} == {'Missing media', 'Media Nuzky cannot open'}, lines)
    time.sleep(0.4)
    r.shot('opened-dialog')
    r.check('Done closes it', r.s.run(CLICK, 'Done', DIALOG) and wait(lambda: r.s.run(DIALOG_STATE) is None, 5))

    p = r.s.run(PROJECT)
    tracks = p['tracks']
    main = tracks[0]['clips']
    times = [(c['startUs'], c['durationUs'], c['content']['sourceInUs'], c['content']['speed']) for c in main]
    r.check('the main track holds the four clips where CapCut had them, with their trim and speed',
            times == [(0, 2_000_000, 0, 1.0), (2_000_000, 1_000_000, 0, 1.0), (3_000_000, 2_000_000, 5_000_000, 2.0), (5_000_000, 2_000_000, 9_000_000, 1.0)], times)
    keys = [(k['tUs'], round(k['transform']['scale'], 3)) for k in main[0]['keyframes']]
    r.check('the zoom is keyframes, timed from the clip', keys == [(0, 1.0), (2_000_000, 1.5)] and main[0]['content']['volume'] == 0.5, keys)
    r.check('the dissolve stays on its cut and the unknown transition is left out',
            main[3].get('transitionIn') == {'kind': 'dissolve', 'durationUs': 400_000} and main[2].get('transitionIn') is None, [c.get('transitionIn') for c in main])
    by_name = {}
    for t in tracks:
        by_name.setdefault(t['name'], []).append(t)
    captions = by_name.get('Captions', [{}])[0]
    r.check('captions are editable text on the Captions track, at their place', [c['content']['text'] for c in captions.get('clips', [])] == ['Hello there', 'big zoom', 'and more']
            and all(abs(c['content']['transform']['y'] - 0.25) < 1e-4 for c in captions['clips']), captions)
    r.check('the title is text on a track of its own', [c['content']['text'] for t in by_name.get('Text', []) for c in t['clips']] == ['My title'], by_name.get('Text'))
    overlays = by_name.get('Overlay', [])
    r.check('the photo is an image clip over the start, and the hidden track stays hidden',
            len(overlays) == 3 and not overlays[0]['hidden'] and overlays[0]['clips'][0]['startUs'] == 500_000 and overlays[2]['hidden'], overlays)
    audio = by_name.get('Audio', [{}])[0]
    song = audio.get('clips', [{}])[0].get('content', {})
    r.check('the song is muted, at its volume with its fade in', audio.get('muted') and audio.get('keepInPlace') and abs(song.get('volume', 0) - 0.3) < 1e-6 and song.get('fadeInUs') == 1_000_000, audio)
    r.check('the project remembers the draft and what did not come over',
            p['imported']['editor'] == 'CapCut' and Path(p['imported']['path']) == r.reel.resolve() and p['imported']['leftOut'] == dialog['lines'] if dialog else False, p.get('imported'))

    # What plays is what CapCut cut: a pixel of the preview against FFmpeg's frame of the recording at that moment.
    for at, source in [(500_000, 0.5), (3_250_000, 5.5), (4_500_000, 8.0), (6_000_000, 10.0)]:
        got, want = preview_pixel(r, at, f'preview-{at}'), reference(r, source)
        r.check(f'at {at / 1e6:g} s the preview shows the recording at {source:g} s', near(got, want), {'preview': got, 'ffmpeg': want})
    black = preview_pixel(r, 2_500_000, 'preview-missing')
    r.check('the missing file and the one that does not open keep their place, showing nothing, and the hidden copy stays hidden',
            near(black, (0, 0, 0)), black)
    photo = preview_pixel(r, 1_000_000, 'preview-photo', at=(0.25, 0.25))
    r.check('the photo sits over the top left quarter', near(photo, hex_rgb(PHOTO)), photo)

    r.s.run("window.__nuzky.dock.setState({open: true})")
    starter = "return [...document.querySelectorAll('aside[aria-label=AI] button')].some((b) => b.textContent.trim() === 'Finish this draft in my style')"
    r.check('the AI panel offers Finish this draft in my style first', wait(lambda: r.s.run(starter), 10))
    r.s.run(CLICK, 'Finish this draft in my style', 'aside[aria-label=AI]')
    draft_text = r.s.run("return window.__nuzky.agent.getState().draft")
    r.check('which puts the whole request in the field', draft_text.startswith(FINISH) and 'CapCut' in draft_text, draft_text)
    r.shot('ai-starter')
    r.s.run("window.__nuzky.agent.setState({draft: ''}); window.__nuzky.dock.setState({open: false})")

    # Back home: the projects come first now, the drafts below them, and the one opened says so.
    r.s.run("window.__nuzky.store.setState({view: 'home'})")
    r.check('once there are projects, the drafts are titled CapCut drafts', wait(lambda: r.s.run(SECTION) == 'CapCut drafts', 10), r.s.run(SECTION))
    below = r.s.run("""const grid = document.querySelector('[role=listbox][aria-label=Projects]')?.getBoundingClientRect();
        const s = document.querySelector('section[aria-label="CapCut drafts"]').getBoundingClientRect();
        return !!grid && grid.height > 0 && s.top >= grid.bottom""")
    r.check('and sit below the projects', below)
    opened = wait(lambda: next((c for c in r.s.run(DRAFT_CARDS) if c['name'] == 'Talking reel' and 'In Nuzky' in c['text']), None), 10)
    r.check('the draft opened before says In Nuzky', opened, r.s.run(DRAFT_CARDS))
    clear = wait(lambda: r.s.run("""const card = [...document.querySelectorAll('[role=option][data-path]')].find((c) => c.innerText.includes('missing'));
        const chips = card && [...card.querySelectorAll('span')].filter((x) => /missing|^\\d+:\\d\\d$/.test(x.textContent.trim()) && x.children.length <= 1);
        if (!chips || chips.length < 2) return null;
        const [a, b] = chips.map((c) => c.getBoundingClientRect());
        return a.right <= b.left || b.right <= a.left || a.bottom <= b.top || b.bottom <= a.top"""), 10)
    r.check('the project\'s missing file chip and its length do not cover each other', clear)
    r.shot('drafts-home')
    count = len(list(projects_dir(r).glob('*.nuzky')))
    path = view(r)['path']
    r.s.run(CLICK_CARD, 'Talking reel')
    r.check('and opens that project again instead of another copy',
            wait(lambda: view(r)['view'] == 'editor', 10) and view(r)['path'] == path and len(list(projects_dir(r).glob('*.nuzky'))) == count, view(r))

    r.s.run("window.__nuzky.store.setState({view: 'home'})")
    wait(lambda: r.s.run(DRAFT_CARDS), 10)
    r.s.run(CLICK_CARD, 'From JianYing')
    encrypted = 'JianYing encrypts its drafts, so Nuzky cannot open this one.'
    r.check('an encrypted draft says why it cannot open and what to do instead',
            wait(lambda: any(t.startswith(encrypted) and 'start a project from it' in t for t in toast_texts(r)), 10), toast_texts(r))
    r.check('and nothing else opens', view(r)['path'] == path, view(r))
    r.s.run("const s = window.__nuzky.store.getState(); s.toasts.forEach((t) => s.dismissToast(t.id));")

    # The smallest window: another draft, its dialog, and Finish with AI.
    if r.check('the window goes to its smallest size', resize(r, 1024, 640)):
        time.sleep(0.5)
        r.shot('drafts-home-1024')
    r.s.run(CLICK_CARD, 'Older reel')
    r.check('another draft opens with its own dialog', wait(lambda: (d := r.s.run(DIALOG_STATE)) and 'Older reel' in d['title'], 20), r.s.run(DIALOG_STATE))
    time.sleep(0.4)
    r.shot('opened-dialog-1024')
    fits = r.s.run(f"""const d = document.querySelector('{DIALOG}').getBoundingClientRect();
        const b = [...document.querySelectorAll('{DIALOG} button')].find((x) => x.textContent.trim() === 'Done').getBoundingClientRect();
        return [d.top >= 0 && d.bottom <= innerHeight, b.bottom <= d.bottom]""")
    r.check('the dialog and its buttons fit the smallest window', fits == [True, True], fits)
    r.s.run(CLICK, 'Finish with AI', DIALOG)
    sent = wait(lambda: next((x for x in runs(r) if x['prompt'].startswith(FINISH)), None), 30)
    r.check('Finish with AI closes it and asks the AI panel\'s agent to finish the draft',
            sent and r.s.run(DIALOG_STATE) is None and r.s.run("return window.__nuzky.dock.getState().open"), [x['prompt'][:80] for x in runs(r)])
    resize(r, 1440, 900)

    # A draft kept elsewhere: the native folder dialog cannot be driven, so the flow hands its path to what it calls.
    r.s.call('window.__nuzky.openDraftFolder(arguments[0])', str(r.elsewhere))
    r.check('a draft folder added by hand opens at once', wait(lambda: view(r)['name'] == 'Short reel', 20), view(r))
    wait(lambda: r.s.run(DIALOG_STATE), 10)
    r.s.run(CLICK, 'Done', DIALOG)
    r.s.call('window.__nuzky.openDraftFolder(arguments[0])', str(r.elsewhere.parent))
    r.s.run("window.__nuzky.store.setState({view: 'home'})")
    r.check('a folder of drafts joins the cards', wait(lambda: {'Short reel', 'Second reel'} <= {c['name'] for c in r.s.run(DRAFT_CARDS)}, 10), r.s.run(DRAFT_CARDS))
    r.s.call('window.__nuzky.findDrafts()')
    learn = "return (window.__nuzky.style.getState().capcutDrafts ?? []).map((d) => d.name)"
    r.check('and Learn my style lists them too', wait(lambda: 'Second reel' in r.s.run(learn), 10), r.s.run(learn))

    # Saved again in CapCut: the menu brings it over once more, beside the project made before.
    count = len(list(projects_dir(r).glob('*.nuzky')))
    r.s.run("""const b = [...document.querySelectorAll('section[aria-label="CapCut drafts"] button[data-draft]')].find((b) => b.querySelector('[data-name]').textContent === 'Talking reel');
        b.focus(); b.dispatchEvent(new KeyboardEvent('keydown', {key: 'ContextMenu', bubbles: true, cancelable: true}));""")
    r.check('a draft\'s menu offers to open it again as a new project', wait(lambda: r.s.run(MENU_ITEM, 'Open as a new project'), 5))
    r.check('which makes another project of it', wait(lambda: view(r)['name'] == 'Talking reel' and len(list(projects_dir(r).glob('*.nuzky'))) == count + 1, 20), view(r))
    wait(lambda: r.s.run(DIALOG_STATE), 10)
    r.s.run(CLICK, 'Done', DIALOG)

    r.check('the drafts themselves are byte for byte as they were', all(fingerprint(d) == before for d, before in r.prints.items()))
    r.check('no other error toast', not r.errors(), r.errors())

"""With nothing selected the inspector shows what the video still needs, read from the project, and each step
opens the place that does it with the keyboard on its control."""
import json, subprocess, time

from e2e.harness import FIXTURES, flow, link_models, wait
from e2e.layout import resize

STEP = "[...document.querySelectorAll('ul[aria-label=\"Project status\"] > li')].find((li) => li.querySelector('span').firstChild.textContent === arguments[0])"
DETAIL = f'const li = {STEP}; return li ? (li.querySelector("div").children[1]?.textContent ?? "") : null'
BUTTON = f'const b = {STEP}?.querySelector("button"); return b ? {{text: b.textContent, disabled: b.getAttribute("aria-disabled") === "true" || b.disabled}} : null'
FOCUSED = """const a = document.activeElement;
    return {tab: window.__nuzky.store.getState().panelTab, attrs: [...a.attributes].map((x) => x.name), text: a.textContent.trim(),
            placeholder: a.placeholder ?? null};"""


def step(r, title):
    return r.s.run(DETAIL, title)


def press_step(r, title, button='button'):
    r.s.run(f'{STEP}.querySelector(arguments[1]).click()', title, button)


def lands(r, tab, attr):
    """The tab opens and its control takes the keyboard; the Transcript's buttons show only once the transcript arrives."""
    return wait(lambda: (f := r.s.run(FOCUSED))['tab'] == tab and attr(f), 5) or r.s.run(FOCUSED)


@flow('project-status', 'With nothing selected the inspector lists what the video still needs, from the project, '
      'and each step opens the tab that does it with the keyboard on its control', before=link_models)
def project_status(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    r.s.run("window.__nuzky.speech.setState({model: 'small', language: 'en'})")
    r.s.run('window.__nuzky.store.getState().select([])')
    r.check('nothing selected shows the project status', wait(lambda: step(r, 'Speech') == 'Not transcribed', 10),
            {t: step(r, t) for t in ('Speech', 'Captions', 'Music', 'Cover')})
    r.check('captions, music and cover say what is missing',
            (step(r, 'Captions'), step(r, 'Music'), step(r, 'Cover')) == ('None', 'None', 'None'),
            {t: step(r, t) for t in ('Captions', 'Music', 'Cover')})
    zooms = r.s.run(BUTTON, 'Zooms')
    r.check('Suggest zooms waits for a transcript and says so', zooms and zooms['disabled'] and step(r, 'Zooms') == 'Needs a transcript',
            {'button': zooms, 'state': step(r, 'Zooms')})
    press_step(r, 'Music')
    landed = lands(r, 'audio', lambda f: f['placeholder'] == 'Search free music')
    r.check('Add opens free music in Audio with the keyboard in its search', landed is True, landed)
    r.shot('status-new')

    press_step(r, 'Speech')
    landed = lands(r, 'transcript', lambda f: 'data-transcribe' in f['attrs'])
    r.check('Transcribe opens Transcript on Transcribe timeline', landed is True, landed)

    # Captions also transcribe; the speech step then counts what Remove retakes and pauses would cut.
    press_step(r, 'Captions')
    landed = lands(r, 'captions', lambda f: 'data-generate-captions' in f['attrs'])
    r.check('Add opens Captions on Generate captions', landed is True, landed)
    r.s.run('document.activeElement.click()')
    job = wait(lambda: next((j for j in r.state()['jobs'] if j['kind'] == 'captions' and j['status'] != 'running'), None), 300, 1)
    r.check('captions finish', job and job['status'] == 'done', job)
    captions = sum(len(t['clips']) for t in r.state()['tracks'] if t['kind'] == 'text')
    r.check('Captions counts the captions and names their style',
            wait(lambda: step(r, 'Captions') == f'{captions} caption{"" if captions == 1 else "s"} · Reel', 10),
            {'shown': step(r, 'Captions'), 'captions': captions})
    r.check('a finished step tells a screen reader it is done', r.s.run(f'return {STEP}.querySelector("span").textContent', 'Captions') == 'Captions, done')
    view = json.loads(r.s.call('window.__nuzky.api.transcriptView(500000).then(JSON.stringify)')['value'])
    shown = wait(lambda: (d := step(r, 'Speech')) not in ('Checking…', 'Not transcribed') and d, 10)
    expected = [f'{n} {w}{"" if n == 1 else "s"}' for n, w in ((view['retakes'], 'retake'), (view['fillers'], 'filler word'),
                (len(view['hesitations']), 'hesitation'), (len(view['pauses']), 'pause')) if n]
    r.check('Speech shows what removing retakes and pauses would cut', shown == (', '.join(expected) or 'Nothing left to cut'),
            {'shown': shown, 'transcript': {k: view[k] for k in ('retakes', 'fillers')} | {'pauses': len(view['pauses'])}})
    if expected:
        press_step(r, 'Speech')
        landed = lands(r, 'transcript', lambda f: 'data-remove-retakes' in f['attrs'])
        r.check('Clean up opens Transcript on Remove retakes and pauses', landed is True, landed)
    zooms = r.s.run(BUTTON, 'Zooms')
    r.check('Suggest zooms is ready once transcribed', zooms and not zooms['disabled'], zooms)
    press_step(r, 'Zooms')
    landed = lands(r, 'transcript', lambda f: 'data-suggest-zooms' in f['attrs'])
    r.check('Suggest opens Transcript on Suggest zooms', landed is True, landed)

    music = r.work / 'music.wav'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'sine=frequency=440:duration=3', str(music)], check=True)
    r.import_media(music)
    asset = next(a['id'] for a in r.state()['assets'] if a['name'] == 'music.wav')
    r.s.call("window.__nuzky.store.getState().edit({type: 'addClip', assetId: arguments[0], startUs: 0, trackId: null})", asset)
    wait(lambda: any(t['kind'] == 'audio' and t['clips'] for t in r.state()['tracks']), 10)
    r.s.run('window.__nuzky.store.getState().select([])')
    r.check('Music names the sound on the timeline', wait(lambda: step(r, 'Music') == 'music.wav', 10), step(r, 'Music'))
    r.s.run('window.__nuzky.sounds.setState({tab: "effects"})')
    press_step(r, 'Music', '[aria-label="Edit music"]')
    landed = lands(r, 'audio', lambda f: f['attrs'] and r.s.run('return window.__nuzky.sounds.getState().tab') == 'project')
    r.check('Edit opens the sounds in the project', landed is True, landed)

    press_step(r, 'Cover')
    opened = wait(lambda: r.s.run('return window.__nuzky.cover.getState().open'), 5)
    landed = wait(lambda: 'data-make-cover-ai' in r.s.run(FOCUSED)['attrs'], 5) or r.s.run(FOCUSED)
    r.check('Make opens the cover editor on Make it with AI', opened == 'cover_9x16' and landed is True, {'open': opened, 'focused': landed})
    r.s.run("document.querySelector('[data-testid=cover-tile]').click()")
    r.check('the cover tile closes the cover editor', wait(lambda: not r.s.run('return window.__nuzky.cover.getState().open'), 5))
    # A cover as the cover editor saves one when a frame is chosen.
    made = r.s.call("""window.__nuzky.store.getState().edit({type: 'setThumbnail', thumbnail: {format: 'cover_9x16', timeUs: 1000000,
        frame: {x: 0, y: 0, scale: 1, rotation: 0, opacity: 1}, background: {picture: true, blur: 0, dim: 0, color: '#000000'},
        texts: [], outline: null}})""")
    r.s.run('window.__nuzky.store.getState().select([])')
    r.check('Cover says which format has a cover', wait(lambda: step(r, 'Cover') == '9:16', 10), {'edit': made, 'shown': step(r, 'Cover')})
    press_step(r, 'Cover')
    landed = wait(lambda: (f := r.s.run(FOCUSED))['text'].startswith('9:16 Cover') and 'aria-pressed' in f['attrs'], 5) or r.s.run(FOCUSED)
    r.check('Edit opens the cover editor with the keyboard on its format', landed is True, landed)
    r.s.run("document.querySelector('[data-testid=cover-tile]').click()")
    wait(lambda: not r.s.run('return window.__nuzky.cover.getState().open'), 5)

    r.s.run('window.__nuzky.store.getState().select([])')
    time.sleep(0.5)
    r.shot('status')
    if resize(r, 1024, 640):
        time.sleep(0.5)
        r.shot('status-small')
        fits = r.s.run("""const list = document.querySelector('ul[aria-label="Project status"]');
            return [...list.querySelectorAll('button')].every((b) => b.getBoundingClientRect().right <= list.getBoundingClientRect().right + 0.5 && b.scrollWidth <= b.clientWidth)""")
        r.check('in the smallest window, 1024 x 640, every step button shows whole', fits)
        resize(r, 1440, 900)
    r.check('no error toast', not r.errors(), r.errors())

"""When an agent needs the face and person models and they are missing, the AI panel asks in the conversation with
one Download button; Not now is remembered for that message, Download goes on by itself, an agent outside the panel
gets the same question as a dialog, and Get set up downloads the rest. The models come from a loopback server."""
import json, re, subprocess, time

from e2e.ai_panel import agent, click, panel, send
from e2e.harness import CLI, FIXTURES, MODELS, ROOT, Bridge, flow, wait
from e2e.setup import BOX
from e2e.speech_model import Models

# Every model with the path the loopback server answers it on: the address's host, then its path.
URLS = dict(re.findall(r'file: "([^"]+)",\s*label: "[^"]+",\s*url: "https://([^"]+)"', (ROOT / 'crates/vision/src/models.rs').read_text()))
FRAMES = ('yunet-2026may.onnx', 'face-landmarks-v2.onnx', 'face-blendshapes-v2.onnx', 'selfie-segmenter.onnx')
CARD = 'aside[aria-label=AI] [aria-labelledby=vision-question-title]'
DIALOG = 'dialog[aria-labelledby="vision-model-title"]'
PROMPT = 'Pick cover frames'
DECLINED = 'Bez modelů vyberu snímek sám.'


def before(r):
    project = r.work / 'data/nuzky/projects/face.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(FIXTURES / 'face-thumb.mp4')], env=r.env, check=True, capture_output=True)
    (r.work / 'data/nuzky/models').mkdir(parents=True)
    r.models = Models({f'/{url}': MODELS / name for name, url in URLS.items()})
    r.env.update(NUZKY_MODEL_URL=f'http://127.0.0.1:{r.models.http.server_port}', NUZKY_AGENT_CLAUDE=str(ROOT / 'tests/e2e/fake_agent'),
                 NUZKY_AGENT_CODEX=str(r.work / 'no-codex'), NUZKY_FAKE_AGENT_LOG=str(r.work / 'fake'))


def card(r):
    return r.s.run("""const c = document.querySelector(arguments[0]); const f = document.activeElement;
        return c && {text: c.innerText, buttons: [...c.querySelectorAll('button')].map((b) => b.textContent.trim()),
                     focusInField: f?.tagName === 'TEXTAREA'};""", CARD)


def download(r):
    return r.s.run("""const s = [...document.querySelectorAll('aside[aria-label=AI] [role=status]')].find((e) => e.innerText.includes('Downloading face and person models'));
        return s ? s.innerText : null;""")


def dialog(r):
    return r.s.run("const d = document.querySelector(arguments[0]); return {open: !!d?.open, text: d?.innerText ?? ''};", DIALOG)


def attempts(r):
    file = r.work / 'fake/frames.jsonl'
    return [json.loads(line) for line in file.read_text().splitlines()] if file.exists() else []


def last_answer(r):
    return r.s.run("return window.__nuzky.agent.getState().items.filter((i) => i.kind === 'agent').at(-1)?.text ?? null")


@flow('vision_models', 'An agent asks for the face and person models in the AI panel, remembers Not now, downloads with one '
      'button and goes on; outside the panel a dialog asks, and Get set up downloads the rest', before=before)
def vision_models(r):
    models = r.work / 'data/nuzky/models'
    try:
        r.s.run('document.body.focus()')
        r.key('j', ctrlKey=True)
        wait(lambda: panel(r), 5)
        wait(lambda: r.s.run("return window.__nuzky.agent.getState().agents?.some((a) => a.name === 'Claude Code' && a.path)"), 10)
        send(r, PROMPT)
        c = wait(lambda: card(r), 20)
        if not r.check('the conversation asks for the models the frames need, 8 MB, with Download and Not now',
                       c and 'Download the face and person models?' in c['text'] and c['buttons'] == ['Download 8 MB', 'Not now'], c):
            return
        r.check('the question takes no focus and opens no dialog', c['focusInField'] and not dialog(r)['open'], {'card': c, 'dialog': dialog(r)})
        r.shot('question')

        click(r, 'Not now', CARD)
        r.check('Not now removes the question and the agent carries on without the models',
                wait(lambda: agent(r)['status'] == 'idle', 15) and not card(r) and last_answer(r) == DECLINED, panel(r)['text'])
        log = attempts(r)
        r.check('both attempts were declined, the second at once without asking again', len(log) == 2
                and all(not a['ok'] and 'MODEL_DECLINED' in a['error'] for a in log) and log[1]['seconds'] < 1, log)
        r.check('nothing was downloaded', not r.models.requests and not any(models.iterdir()), r.models.requests)

        r.models.rate = 1024 * 1024
        send(r, PROMPT)
        r.check('a new message asks again', wait(lambda: card(r), 20), panel(r)['text'])
        click(r, 'Download 8 MB', CARD)
        r.check('Download turns the question into the download with its percentage and Stop',
                wait(lambda: (d := download(r)) and re.search(r'\d+%', d) and 'Stop' in d, 10), download(r))
        r.check('the top bar shows the download with its own Stop', r.s.run(
            "return !!document.querySelector('header button[aria-label=\"Stop downloading the face and person models\"]')"))
        r.shot('downloading')
        r.models.rate = 0
        r.check('the agent goes on by itself once they are in', wait(lambda: agent(r)['status'] == 'idle', 120)
                and (last_answer(r) or '').startswith('Nejlepší snímek je v'), panel(r)['text'])
        log = attempts(r)[2:]
        r.check('the frames came on the first attempt of that message', len(log) == 1 and log[0]['ok'], log)
        fetched = sorted({q['path'] for q in r.models.requests})
        r.check('only the four models the frames need were downloaded, and are installed', fetched == sorted(f'/{URLS[name]}' for name in FRAMES)
                and all((models / name).stat().st_size == (MODELS / name).stat().st_size for name in FRAMES), fetched)
        r.check('the download line is gone', not download(r) and not card(r), panel(r)['text'])
        r.shot('done')

        # An agent outside the panel gets a dialog; the person mask needs the 188 MB model.
        r.key('j', ctrlKey=True)
        wait(lambda: not panel(r), 5)
        bridge = Bridge(r, r.saved_project())
        try:
            job = bridge.call('segment_subject', {'time_us': 7_500_000})
            d = wait(lambda: (d := dialog(r))['open'] and d, 15)
            r.check('with the panel closed a dialog asks for the subject mask model, 189 MB', d and 'face and person models' in d['text']
                    and '189 MB' in d['text'], d)
            time.sleep(0.4)  # past the 200 ms fade-in
            r.shot('dialog')
            r.s.run("document.querySelector(arguments[0]).dispatchEvent(new Event('cancel'))", DIALOG)
            status = wait(lambda: (s := bridge.call('job', {'job_id': job['job_id'], 'action': 'get'}))['status'] != 'running' and s, 10)
            r.check('Esc tells the outside agent it declined', status and status['status'] == 'failed' and 'MODEL_DECLINED' in status['error'], status)
        finally:
            bridge.close()

        # Get set up downloads the rest and counts the step done.
        r.s.run("window.__nuzky.setup.getState().openSetup(); window.__nuzky.setup.getState().showStep('vision')")
        step = lambda: r.s.run(f"""const p = document.querySelector('{BOX} [role=tabpanel]');
            return p && {{title: p.querySelector('h3').textContent, text: p.innerText,
                          buttons: [...document.querySelectorAll('{BOX} footer button')].map((b) => b.textContent.trim())}};""")
        s = wait(lambda: (s := step()) and 'MB' in s['text'] and s, 10)
        r.check('the step offers what is left, 189 MB', s and s['title'] == 'Face and person models' and s['buttons'][0] == 'Download 189 MB', s)
        time.sleep(0.4)
        r.shot('setup')
        r.s.run(f"[...document.querySelectorAll('{BOX} footer button')].find((b) => b.textContent.startsWith('Download')).click()")
        r.check('Get set up downloads them and marks the step done', wait(lambda: 'Downloaded' in (step() or {}).get('text', '')
                and r.s.run("return window.__nuzky.setup.getState().states.vision") == 'done', 180), step())
        r.check('every model is installed', all((models / name).exists() for name in URLS), sorted(p.name for p in models.iterdir()))
        r.shot('setup-done')
    finally:
        r.models.http.shutdown()
        r.models.http.server_close()
    r.check('no error toast', not r.errors(), r.errors())

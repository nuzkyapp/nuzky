"""The AI panel asks once per turn for missing speech models, stops promptly, resumes and finishes the edit."""
import json, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from e2e.ai_panel import agent, click, panel, send, setup
from e2e.harness import MODELS, flow, link_models, press, wait, webdriver

MODEL = 'ggml-large-v3-turbo-q5_0.bin'
VAD = 'ggml-silero-v5.1.2.bin'
SIZE = 574041195
DIALOG = 'dialog[aria-labelledby="speech-model-title"]'
DECLINED = 'Bez přepisu můžu vystřihnout dlouhá ticha.'
PROMPT = 'Cut retakes and long pauses'


class Models:
    """Serves the fixtures' models under their Hugging Face paths and records every request. `rate` limits bytes per
    second; a path in `fail` answers 503 once."""
    def __init__(self, paths=None):
        self.rate, self.requests, self.fail = 0, [], set()
        server = self
        paths = paths or {f'/ggerganov/whisper.cpp/resolve/main/{MODEL}': MODELS / MODEL,
                          f'/ggml-org/whisper-vad/resolve/main/{VAD}': MODELS / VAD}

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                ranged = self.headers.get('Range')
                server.requests.append({'path': self.path, 'range': ranged})
                if self.path in server.fail:
                    server.fail.discard(self.path)
                    self.send_error(503)
                    return
                if self.path not in paths:
                    self.send_error(404)
                    return
                file = paths[self.path]
                size = file.stat().st_size
                start = int(ranged.removeprefix('bytes=').removesuffix('-')) if ranged else 0
                self.send_response(206 if ranged else 200)
                self.send_header('Content-Length', str(size - start))
                self.send_header('Accept-Ranges', 'bytes')
                if ranged:
                    self.send_header('Content-Range', f'bytes {start}-{size - 1}/{size}')
                self.end_headers()
                try:
                    with file.open('rb') as f:
                        f.seek(start)
                        while chunk := f.read(256 * 1024):
                            self.wfile.write(chunk)
                            if server.rate:
                                time.sleep(len(chunk) / server.rate)
                except (BrokenPipeError, ConnectionResetError):
                    pass

            def log_message(self, *args):
                pass

        self.http = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        threading.Thread(target=self.http.serve_forever, daemon=True).start()


def before(r):
    setup(r)
    link_models(r, names=())
    r.models = Models()
    r.env['NUZKY_MODEL_URL'] = f'http://127.0.0.1:{r.models.http.server_port}'


def dialog(r):
    return r.s.run("""const d = document.querySelector(arguments[0]);
        return {open: !!d?.open, count: document.querySelectorAll(arguments[0]).length, text: d?.innerText ?? '',
                focused: !!d && document.activeElement === d,
                question: window.__nuzky.store.getState().speechModelQuestion};""", DIALOG)


def small_window(r, name, selector):
    """Screenshots the smallest window and says whether `selector` is wholly inside it."""
    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    wait(lambda: r.s.run('return window.innerWidth') <= 1030, 5)
    time.sleep(0.4)
    inside = r.s.run("""const b = document.querySelector(arguments[0]).getBoundingClientRect();
        return b.width > 0 && b.left >= 0 && b.top >= 0 && b.right <= innerWidth && b.bottom <= innerHeight;""", selector)
    r.shot(name)
    webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
    wait(lambda: r.s.run('return window.innerWidth') >= 1400, 5)
    return inside


def speech_job(r):
    return r.s.run("return Object.values(window.__nuzky.store.getState().jobs).filter((j) => j.kind === 'speech-model').at(-1) ?? null;")


def attempts(r):
    file = r.work / 'fake/transcribe.jsonl'
    return [json.loads(line) for line in file.read_text().splitlines()] if file.exists() else []


@flow('speech_model', 'An agent asks before downloading speech models, remembers a refusal, resumes after Stop and edits', before=before)
def speech_model(r):
    models = r.work / 'data/nuzky/models'
    try:
        r.s.run('document.body.focus()')
        r.key('j', ctrlKey=True)
        wait(lambda: panel(r), 5)
        wait(lambda: r.s.run("return window.__nuzky.agent.getState().agents?.some((a) => a.name === 'Claude Code' && a.path)"), 10)
        send(r, PROMPT)
        if not r.check('the agent asks to download the missing model', wait(lambda: dialog(r)['open'], 15), dialog(r)):
            return
        d = dialog(r)
        r.check('the dialog names the download, shows 548 MB and takes focus itself',
                d['count'] == 1 and 'Download the speech model?' in d['text'] and '548 MB' in d['text'] and d['focused']
                and d['question']['sizeMb'] == 548 and 'id' in d['question'], d)
        time.sleep(0.4)  # past the 200 ms fade-in
        r.shot('dialog')
        press('space')
        time.sleep(0.5)
        r.check('a Space typed while it opens starts nothing, neither the download nor playback', dialog(r)['open'] and not speech_job(r)
                and not r.models.requests and not r.s.run("return window.__nuzky.store.getState().playing"))
        press('Escape')
        r.check('Escape closes the question', wait(lambda: not dialog(r)['open'] and dialog(r)['question'] is None, 3))
        reopened = []

        def finished():
            d = dialog(r)
            if d['open'] or d['question'] is not None:
                reopened.append(d)
            return agent(r)['status'] == 'idle'

        r.check('the agent finishes without another question', wait(finished, 15, 0.2) and not reopened, reopened)
        log = attempts(r)
        r.check('all four attempts were declined and retries failed in under a second',
                len(log) == 4 and [a['attempt'] for a in log] == [1, 2, 3, 4]
                and all(not a['ok'] and 'MODEL_DECLINED' in a['error'] for a in log)
                and all(a['seconds'] < 1 for a in log[1:]), log)
        r.check('the answer offers silence cuts and no model was downloaded', DECLINED in panel(r)['text']
                and not (models / MODEL).exists() and not (models / VAD).exists()
                and not list(models.glob('*.part')) and not r.models.requests, panel(r)['text'])
        r.shot('declined')

        r.models.rate = 20 * 1024 * 1024
        send(r, PROMPT)
        r.check('a new turn asks again', wait(lambda: dialog(r)['open'] and dialog(r)['focused'], 15))
        press('Return')
        r.check('Enter downloads and closes the question', wait(lambda: not dialog(r)['open'] and dialog(r)['question'] is None, 3))
        r.check('the speech-model job reports progress', wait(lambda: (j := speech_job(r)) and j['status'] == 'running'
                and j['progress'] >= 0.01 and j['label'] == 'Speech model' and j['id'].startswith('speech-model:'), 15), speech_job(r))
        r.check('the top bar shows the download percentage and Stop', r.s.run("""const h = document.querySelector('header');
            return h.innerText.includes('Downloading speech model') && /[1-9][0-9]*%/.test(h.innerText)
                && h.querySelector('button[aria-label="Stop downloading the speech model"]')?.textContent.trim() === 'Stop';"""))
        r.shot('downloading')
        r.check('in a 1024 x 640 window the download and its Stop still fit the top bar',
                small_window(r, 'downloading-small', 'header button[aria-label="Stop downloading the speech model"]'))
        job_id = speech_job(r)['id']
        started = time.monotonic()
        click(r, 'Stop downloading the speech model', 'header')
        r.check('Stop cancels this download within three seconds', wait(lambda: (j := speech_job(r))
                and j['id'] == job_id and j['status'] == 'cancelled', 3, 0.1)
                and time.monotonic() - started <= 3, speech_job(r))
        r.check('the stopped turn finishes with the no-transcript answer', wait(lambda: agent(r)['status'] == 'idle', 15)
                and r.s.run("return window.__nuzky.agent.getState().items.filter((i) => i.kind === 'agent').at(-1)?.text") == DECLINED)
        log = attempts(r)[4:]
        r.check('stopping also declines the agent job and its retries', len(log) == 4
                and all(not a['ok'] and 'MODEL_DECLINED' in a['error'] for a in log), log)
        part = models / (MODEL + '.part')
        r.check('Stop keeps a partial model to resume', not (models / MODEL).exists()
                and part.exists() and 0 < part.stat().st_size < SIZE)

        r.models.rate = 0
        before_track = r.track()
        send(r, PROMPT)
        r.check('the next turn asks before resuming', wait(lambda: dialog(r)['open'], 15))
        r.check('in a 1024 x 640 window the dialog fits', small_window(r, 'dialog-small', DIALOG))
        click(r, 'Download', DIALOG)
        r.check('Download closes the question', wait(lambda: not dialog(r)['open'] and dialog(r)['question'] is None, 3))
        r.check('the download finishes and the agent completes without more input',
                wait(lambda: agent(r)['status'] == 'idle' and r.s.run("return window.__nuzky.agent.getState().items.filter((i) => i.kind === 'agent').at(-1)?.text.startsWith('Hotovo')"), 180))
        r.check('the speech-model job finished', speech_job(r)['status'] == 'done', speech_job(r))
        r.check('both complete models exist and no partial file remains', (models / MODEL).exists()
                and (models / MODEL).stat().st_size == SIZE and (models / VAD).exists() and not list(models.glob('*.part')))
        r.check('the speech model resumed with a Range request', any(q['path'].endswith('/' + MODEL)
                and q['range'] and int(q['range'].removeprefix('bytes=').removesuffix('-')) > 0 for q in r.models.requests), r.models.requests)
        after_track = r.track()
        r.check('the transcript edit shortened the main track and its run card is visible',
                after_track[-1]['startUs'] + after_track[-1]['durationUs'] < before_track[-1]['startUs'] + before_track[-1]['durationUs']
                and 'run' in agent(r)['kinds'] and r.s.run("return document.querySelector('aside[aria-label=AI] span[title=\"Cut retakes\"]')?.getClientRects().length > 0"), panel(r)['text'])
        log = attempts(r)[8:]
        r.check('the resumed turn transcribed successfully on its first attempt', len(log) == 1 and log[0]['ok'], log)
        r.shot('done')
    finally:
        r.models.http.shutdown()
        r.models.http.server_close()

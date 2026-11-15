"""Help improve Nuzky. A new profile is not asked at its first launch; the next launch asks once, with both choices
unticked. With Problems and usage ticked, Nuzky sends its own server the computer, errors, failed jobs and what was used
without the names of the project and its files or the folders, and the report of a crash; AI conversations adds the
panel's messages and answers; Settings › Privacy turns each off and deletes what was sent. Between the app and the real
account server (account/, in its end-to-end mode) a proxy records what arrives."""
import json, re, shutil, subprocess, threading, time, urllib.error, urllib.request, uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from e2e.harness import CLI, FIXTURES, ROOT, Session, close_window, deny, flow, leave_editor, wait
from e2e.home import resize

FAKE = Path(__file__).resolve().parent / 'fake_agent'
NAME = 'Svatba Jany'
DIALOG = 'dialog[data-improve]'
QUESTION = """const d = document.querySelector(arguments[0]);
return d?.open ? {text: d.innerText, ticked: [...d.querySelectorAll('input[type=checkbox]')].map((i) => i.checked),
  share: [...d.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Share')?.getAttribute('aria-disabled')} : null;"""
TICK = """const l = [...document.querySelectorAll(arguments[0] + ' label')].find((l) => l.textContent.includes(arguments[1]));
l.querySelector('input').click();"""
CLICK = """const b = [...document.querySelectorAll(arguments[0] + ' button')].find((b) => b.textContent.trim() === arguments[1]);
if (!b || b.getAttribute('aria-disabled') === 'true') return false; b.click(); return true;"""
STATUS = 'return window.__nuzky.telemetry.getState()'
TOASTS = 'return window.__nuzky.store.getState().toasts.map((t) => t.text)'
PRIVACY = '[role=dialog][data-settings] [data-setting=improve]'
SERVER = ROOT / 'account/.next/standalone/server.js'


def needs():
    return None if SERVER.exists() else 'the account server is not built: run npm ci and npm run build in account/'


def setup(r):
    media = r.work / f'{NAME}.mp4'
    shutil.copyfile(FIXTURES / 'talk.mp4', media)
    subprocess.run([str(CLI), 'new', str(r.work / f'data/nuzky/projects/{NAME}.nuzky'), str(media)], env=r.env, check=True, capture_output=True)
    (r.work / 'data/nuzky/telemetry/settings.json').unlink()
    r.received = []
    log = (r.work / 'account-server.log').open('w')
    r.server = subprocess.Popen(['node', str(ROOT / 'account/scripts/e2e-server.mjs'), '--data', str(r.work / 'account-server')],
                                stdout=subprocess.PIPE, stderr=log, text=True)
    account = json.loads(r.server.stdout.readline())['url']

    class Server(BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers['Content-Length']))
            forward = urllib.request.Request(account + self.path, data=body, method='POST',
                                             headers={'Content-Type': self.headers.get('Content-Type') or ''})
            try:
                with urllib.request.urlopen(forward, timeout=30) as answer:
                    status, reply = answer.status, answer.read()
            except urllib.error.HTTPError as error:
                status, reply = error.code, error.read()
            r.received.append({'path': self.path, 'type': self.headers.get('Content-Type'), 'body': json.loads(body), 'raw': body.decode(),
                               'status': status, 'answer': json.loads(reply or b'null')})
            self.send_response(status)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(reply)))
            self.end_headers()
            self.wfile.write(reply)

        def log_message(self, *args):
            pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), Server)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    r.env.update(NUZKY_TELEMETRY_URL=f'http://127.0.0.1:{server.server_port}/api/telemetry', NUZKY_TELEMETRY_ROUND_MS='1500',
                 NUZKY_AGENT_CLAUDE=str(FAKE), NUZKY_FAKE_AGENT_LOG=str(r.work / 'fake'))


def events(r, kind=None):
    found = [e for req in r.received if req['path'] == '/api/telemetry' for e in req['body']['events']]
    return [e for e in found if kind is None or e['kind'] == kind]


def SETTINGS(r):
    return r.work / 'data/nuzky/telemetry/settings.json'


def settings(r):
    return json.loads(SETTINGS(r).read_text())


def private(r):
    """What was sent that names the project, its file, or the folders of this run."""
    return [req['raw'][:300] for req in r.received if NAME in req['raw'] or str(r.work) in req['raw'] or 'Users/jana' in req['raw']]


@flow('telemetry', 'Help improve Nuzky asks once after an update with both off, sends only what was ticked, without names '
      'or folders, sends a report after a crash, and turns off and deletes from Settings › Privacy', before=setup, needs=needs)
def telemetry(r):
    r.check('a new profile is not asked at its first launch',
            not r.s.run(STATUS)['open'] and not r.s.run(QUESTION, DIALOG) and r.s.run(STATUS)['status']['asked'] is False)

    # The next launch is a profile that used Nuzky before, and the one before it ended without quitting.
    (r.work / 'data/nuzky/logs/running-1').write_text('')
    leave_editor(r)
    close_window()
    r.s.close()
    r.s = Session()
    if not wait(lambda: r.s.run('return !!window.__nuzky?.store.getState().snap', retries=3), 60):
        raise RuntimeError('the app did not start again')
    resize(r, 1440, 900)
    r.check('nothing was sent while both were off', not r.received, r.received)
    q = wait(lambda: r.s.run(QUESTION, DIALOG), 10)
    r.check('the next launch asks by itself, both unticked and Share disabled',
            q and 'Can you help me make Nuzky better?' in q['text'] and 'Tomáš Mach' in q['text'] and q['ticked'] == [False, False]
            and q['share'] == 'true', q)
    r.check('the crash notice offers its report', any('closed unexpectedly' in t for t in r.s.run(TOASTS)), r.s.run(TOASTS))
    time.sleep(0.5)  # The dialog fades in.
    r.shot('question')
    r.s.run(CLICK, DIALOG, 'Example of a problem report')
    r.check('the example of a problem report names no file', 'MEDIA_UNREADABLE' in r.s.run(QUESTION, DIALOG)['text'])
    resize(r, 1024, 640)
    r.shot('question-small-window')
    resize(r, 1440, 900)

    r.s.run(TICK, DIALOG, 'Problems and usage')
    wait(lambda: r.s.run(QUESTION, DIALOG)['share'] is None, 3)
    r.check('Share keeps what was ticked', r.s.run(CLICK, DIALOG, 'Share') and wait(lambda: not r.s.run(QUESTION, DIALOG), 3)
            and wait(lambda: SETTINGS(r).exists(), 3) and settings(r)['problems'] and not settings(r)['conversations']
            and settings(r)['asked'], SETTINGS(r).exists() and settings(r))
    r.check('a toast thanks and says where to change it', wait(lambda: any('Thank you' in t for t in r.s.run(TOASTS)), 3), r.s.run(TOASTS))

    r.s.run("window.__nuzky.api.logUiError(arguments[0])", f'Cannot read /Users/jana/Videos/{NAME}.mp4: Invalid data found')
    r.s.run('window.__nuzky.api.pause()')
    sent = lambda: events(r, 'error') and events(r, 'start') and events(r, 'usage')
    r.check('the computer, the error and what was used arrive', wait(sent, 20), [e['kind'] for e in events(r)])
    first = r.received[0]
    r.check('as JSON with a random install id, the version and the system',
            first['type'] == 'application/json' and uuid.UUID(first['body']['install']).version == 4
            and first['body']['version'] and first['body']['system'], {k: first['body'][k] for k in ('install', 'version', 'system')})
    error = next(e for e in events(r, 'error') if e['data']['source'] == 'ui')
    r.check("the error keeps the file's extension and says nothing of its name or folder",
            error['data']['message'] == 'Cannot read ….mp4: Invalid data found', error)
    r.check('usage counts actions by name', any('pause' in e['data']['actions'] for e in events(r, 'usage')), events(r, 'usage'))
    r.check('no AI conversation goes while it is off', not events(r, 'aiChat'))

    locked = r.work / 'locked'
    locked.mkdir()
    allow = deny(locked, 0o500)
    r.s.call('window.__nuzky.api.startExport(arguments[0], arguments[1], arguments[2], arguments[3])', str(locked / f'{NAME} final.mp4'),
             {'resolution': 720, 'fps': 30, 'quality': 'small'}, r.state()['epoch'], False)
    failed = lambda: [e['data'] for e in events(r, 'job') if e['data']['kind'] == 'export' and e['data']['status'] == 'failed']
    r.check("a failed export arrives with its error, the last actions and the files' formats",
            wait(failed, 60) and failed()[0]['error'] and 'job export started' in failed()[0]['actions']
            and any(line.startswith('File 1 (video') for line in failed()[0]['files']), failed()[:1])
    allow()

    r.check('sharing Problems and usage sends the crash report by itself, and the crash notice goes',
            wait(lambda: events(r, 'report'), 15) and wait(lambda: any('Crash report sent' in t for t in r.s.run(TOASTS)), 5)
            and not any('closed unexpectedly' in t for t in r.s.run(TOASTS)), r.s.run(TOASTS))
    report = events(r, 'report')[0]['data']['text']
    r.check('the report holds the version and the files\' formats', 'Nuzky' in report and 'File 1 (video' in report, report[:400])

    r.s.run("window.__nuzky.store.setState({view: 'editor'})")
    r.s.run("window.__nuzky.openSettings('privacy')")
    view = lambda: r.s.run(f"return document.querySelector('{PRIVACY}')?.innerText ?? ''")
    switch = lambda name: r.s.run(f"return document.querySelector('{PRIVACY} [role=switch][aria-label=\"{name}\"]')?.getAttribute('aria-checked')")
    shown = lambda name: r.s.run(f"return [...document.querySelectorAll('{PRIVACY} button')].find((b) => b.textContent.trim() === arguments[0])?.getAttribute('aria-disabled')", name)
    r.check('Settings › Privacy shows Problems and usage on, AI conversations off, and what was sent can be shown',
            wait(lambda: switch('Problems and usage') == 'true', 5) and switch('AI conversations') == 'false'
            and 'without your name, email or account' in view() and wait(lambda: shown('Show what was sent…') is None, 5), view())
    r.s.run(f"document.querySelector('{PRIVACY} [role=switch][aria-label=\"AI conversations\"]').click()")
    r.check('its switch turns AI conversations on', wait(lambda: settings(r)['conversations'], 3), settings(r))
    time.sleep(0.5)  # The screenshot can trail a change made just before it.
    r.shot('privacy')
    r.key('Escape')

    r.s.run("window.__nuzky.store.setState({view: 'editor'})")
    r.s.run('document.body.focus()')
    r.key('j', ctrlKey=True)
    wait(lambda: r.s.run("return !!document.querySelector('aside[aria-label=AI] textarea')"), 5)
    r.s.run("""const t = document.querySelector('aside[aria-label=AI] textarea'); t.focus();
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(t, arguments[0]);
        t.dispatchEvent(new Event('input', {bubbles: true}));""", 'Please cut the slow start')
    r.key('Enter')
    chat = lambda: events(r, 'aiChat') and events(r, 'aiTurn')
    r.check("the panel's message, answer and tools arrive", wait(chat, 60), [e['kind'] for e in events(r)])
    said = events(r, 'aiChat')[0]['data']
    turn = events(r, 'aiTurn')[0]['data']
    r.check('with the message as written and the tools the agent called',
            said['prompt'] == 'Please cut the slow start' and said['answer'] and said['tools'] and said['agent'] == 'Claude Code'
            and turn['tools'] and turn['end'] == 'done', {'chat': said, 'turn': turn})
    r.check("nothing sent names the project or its file, or this run's folders", not private(r), private(r))
    sent = [req for req in r.received if req['path'] == '/api/telemetry']
    r.check('the account server stored every event of every batch',
            all(req['status'] == 200 and req['answer'] == {'stored': len(req['body']['events']), 'skipped': 0} for req in sent),
            [(req['status'], req['answer']) for req in sent])

    r.s.run("window.__nuzky.openSettings('privacy')")
    wait(lambda: view(), 5)
    install = settings(r)['install']
    r.s.run(CLICK, PRIVACY, 'Delete what was sent…')
    r.check('Delete asks first, with the keyboard on Cancel', 'Delete everything this computer sent' in view()
            and r.s.run('return document.activeElement?.textContent.trim()') == 'Cancel', view())
    r.s.run(CLICK, PRIVACY, 'Delete')
    deleted = lambda: [req for req in r.received if req['path'] == '/api/telemetry/delete']
    r.check('Delete asks the server to delete this install and turns both off as a new install',
            wait(deleted, 10) and deleted()[0]['body'] == {'install': install}
            and wait(lambda: not settings(r)['problems'] and not settings(r)['conversations'] and settings(r)['install'] != install, 5)
            and switch('Problems and usage') == 'false' and 'Deleted everything this computer sent.' in view(), settings(r))
    r.check('the account server deleted what this install sent', deleted()[0]['status'] == 200
            and deleted()[0]['answer']['deleted'] == sum(len(req['body']['events']) for req in sent), deleted()[0]['answer'])
    r.shot('privacy-deleted')
    r.server.terminate()

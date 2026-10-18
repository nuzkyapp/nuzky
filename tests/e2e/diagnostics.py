"""When something breaks, the person sees what happened with a way on and can send what Nuzky logged. An export to a
folder it may not write to fails with Open logs folder and a way to export again, which works once the folder is
writable; the log names the failure without the folders. Copy diagnostics gives the version, system, graphics and the end of the log.
After the app is killed, the next start says it closed unexpectedly and Report a bug… copies the diagnostics and
opens the bug form; after a normal quit it says nothing. A bug while drawing the window shows Reload instead of an
empty window. Nothing leaves the computer: the flow records the pages and folders the app would open, and the
clipboard is the page's own, never the system's."""
import os, re, signal, subprocess, time, urllib.parse
from pathlib import Path

from e2e.harness import MACOS, Session, close_window, flow, wait

# The page's clipboard keeps what is copied, so the flow neither reads nor overwrites the person's own.
CLIPBOARD = """window.__copied = [];
Object.defineProperty(navigator, 'clipboard', {configurable: true, value: {writeText: async (t) => { window.__copied.push(t); }}});"""
COPIED = 'return window.__copied ?? []'
TOASTS = "return window.__nuzky.store.getState().toasts.map((t) => ({kind: t.kind, text: t.text, action: t.action?.label ?? null}));"
START_EXPORT = 'window.__nuzky.api.startExport(arguments[0], arguments[1], arguments[2], arguments[3])'
DIALOG_BUTTONS = "return [...document.querySelectorAll('[role=dialog] button')].map((b) => b.textContent.trim())"
CLICK = """const b = [...document.querySelectorAll(arguments[0] + ' button')].find((b) => b.textContent.trim() === arguments[1]);
if (!b) return false; b.click(); return true;"""
ALERT = "return document.querySelector('[role=dialog] [role=alert]')?.textContent ?? ''"
MENU = """const m = document.querySelector('[role=menu][aria-label="Send feedback"]');
return m && [...m.querySelectorAll('[role^=menuitem]')].map((i) => i.textContent);"""
CHOOSE = """const i = [...document.querySelectorAll('[role=menu][aria-label="Send feedback"] [role^=menuitem]')]
  .find((i) => i.textContent === arguments[0]);
if (!i) return false; i.click(); return true;"""
CRASHED = 'Nuzky closed unexpectedly. Your project is as it was last saved.'


def record_opened(r):
    r.opened = r.work / 'opened.txt'
    r.env['NUZKY_OPENED_URLS'] = str(r.opened)


def opened(r):
    return r.opened.read_text().splitlines() if r.opened.exists() else []


def log_text(r):
    logs = r.work / 'data/nuzky/logs'
    return ''.join(p.read_text(errors='replace') for p in sorted(logs.glob('nuzky*.log')))


def app_pid(r):
    """The app of this run. On Linux WebKitWebDriver starts it, so it is the nuzky-app with this run's data folder."""
    if MACOS:
        return r.app.pid
    for proc in Path('/proc').iterdir():
        try:
            if proc.name.isdigit() and (proc / 'exe').resolve().name == 'nuzky-app' and \
                    f'XDG_DATA_HOME={r.work / "data"}'.encode() in (proc / 'environ').read_bytes().split(b'\0'):
                return int(proc.name)
        except OSError:
            continue
    return None


def restart(r):
    r.s.close()
    r.s = Session()
    if not wait(lambda: r.s.run('return !!window.__nuzky?.store.getState().snap', retries=3), 60):
        raise RuntimeError('the app did not start again')
    r.s.run(CLIPBOARD)


def secret_free(r, text):
    """Neither the flow's folders nor the person's home are in `text`."""
    return str(r.work) not in text and str(Path.home()) not in text and 'locked/' not in text


@flow('diagnostics', 'A failed export offers Open logs folder and a way to export again, Copy diagnostics gives the version, system, graphics '
      'and log without folders, a crash is reported on the next start, and a broken window offers Reload',
      before=record_opened)
def diagnostics(r):
    r.s.run(CLIPBOARD)
    clip = r.work / 'clip.mp4'
    subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'testsrc=size=640x360:rate=30', '-f', 'lavfi', '-i',
                    'sine=frequency=440', '-t', '2', '-pix_fmt', 'yuv420p', str(clip)], check=True)
    r.import_media(clip)
    r.add_clip('clip.mp4')
    clips = r.track()

    locked = r.work / 'locked'
    locked.mkdir()
    locked.chmod(0o500)
    target = locked / 'out.mp4'
    # Started as Export… does after the save panel, which cannot be driven, with the dialog open on it.
    options = {'resolution': 720, 'fps': 30, 'quality': 'small'}
    started = r.s.call(START_EXPORT, str(target), options, r.state()['epoch'], False)
    r.s.run('window.__nuzky.store.setState({exportJobId: arguments[0], exportOpen: true})', started.get('value'))
    alert = wait(lambda: r.s.run(ALERT), 60)
    r.check('the export fails saying the folder cannot be written to and what to do, without a code or the folder',
            alert == "Export failed: Nuzky isn't allowed to save in that folder. Export to another folder.Open logs folder" and not re.search(r'\b[A-Z_]{4,}:', alert) and secret_free(r, alert), alert)
    buttons = r.s.run(DIALOG_BUTTONS)
    # Started from the dialog, the last button is Retry; this export never told the dialog where it went.
    r.check('the failure offers Open logs folder, and Close and Export… to try again', 'Open logs folder' in buttons and buttons[-2:] == ['Close', 'Export…'], buttons)
    time.sleep(0.6)  # The dialog has faded in.
    r.shot('export-failed')
    log = log_text(r)
    r.check('the log records the failed export with the file name and without its folders',
            re.search(r'^\S+Z ERROR nuzky_app::jobs: Exporting out\.mp4 failed: .+$', log, re.M) and secret_free(r, log), log[-600:])

    r.s.run(CLICK, '[role=dialog]', 'Open logs folder')
    r.check('Open logs folder opens the logs folder in the data folder',
            wait(lambda: opened(r), 5) and Path(opened(r)[-1]).resolve() == (r.work / 'data/nuzky/logs').resolve(), opened(r))

    locked.chmod(0o700)
    again = r.s.call(START_EXPORT, str(target), options, r.state()['epoch'], False)
    r.s.run('window.__nuzky.store.setState({exportJobId: arguments[0]})', again.get('value'))
    done = wait(lambda: 'Saved to' in (r.s.run("return document.querySelector('[role=dialog]')?.textContent ?? ''") or ''), 120)
    r.check('once the folder can be written the same export succeeds', done and target.stat().st_size > 1000)
    r.s.run(CLICK, '[role=dialog]', 'Done')

    # A speech model download that fails at once: Nuzky may not write to its models folder, so nothing goes online.
    models = r.work / 'data/nuzky/models'
    models.mkdir(parents=True, exist_ok=True)
    models.chmod(0o500)
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === 'Transcript').click()")
    wait(lambda: r.s.run(CLICK, 'body', 'Transcribe timeline'), 5)
    failed = 'Transcript failed: The speech model could not be downloaded. Check the internet connection and free disk space, then try again.'
    toast = wait(lambda: next((t for t in r.s.run(TOASTS) if t['text'] == failed), None), 30)
    r.check('a failed model download says so in plain words, with Try again', toast and toast['action'] == 'Try again', r.s.run(TOASTS))
    panel = r.s.run("return [...document.querySelectorAll('[role=alert]')].map((a) => a.textContent).find((t) => t.includes('speech model')) ?? ''")
    r.check('the panel shows the same failure with Open logs folder', panel.startswith(failed[len('Transcript failed: '):]) and panel.endswith('Open logs folder'), panel)
    time.sleep(0.3)
    r.shot('transcript-failed')
    jobs = lambda: [j for j in r.state()['jobs'] if j['kind'] == 'transcript']
    r.s.run(CLICK, '[data-toast]', 'Try again')
    r.check('Try again runs the transcript again', wait(lambda: len(jobs()) == 2 and all(j['status'] == 'failed' for j in jobs()), 30), jobs())
    models.chmod(0o700)
    r.s.run("window.__nuzky.store.setState({toasts: []})")

    # An error that names a private folder, a caption and a secret, as the interface would report one.
    r.s.call('window.__nuzky.api.logUiError(arguments[0])',
             f'Cannot draw {r.work}/Private Trip/clip.mp4 with caption "Ahoj Jano" and token=5eed5eed5eed')
    log = log_text(r)
    r.check('the log keeps an error of the interface without its folder, caption or token',
            re.search(r'^\S+Z ERROR ui: Cannot draw clip\.mp4 with caption "…" and token=\[hidden\]$', log, re.M)
            and secret_free(r, log) and 'Ahoj' not in log and '5eed5eed' not in log, log[-400:])
    r.s.run("const b = document.querySelector('header [data-feedback]'); b.focus(); b.click();")
    menu = wait(lambda: r.s.run(MENU), 3)
    r.check('Send feedback offers Copy diagnostics and Open logs folder',
            menu == ['Report a bug…', 'Share an idea…', 'Copy diagnostics', 'Open logs folder', 'Message me on X…'], menu)
    time.sleep(0.3)
    r.shot('feedback-menu')
    r.s.run(CHOOSE, 'Copy diagnostics')
    copied = wait(lambda: r.s.run(COPIED), 5)
    text = copied[-1] if copied else ''
    version = r.s.run('return window.__nuzky.updates.getState().version')
    r.check('Copy diagnostics copies the version, system and graphics backend',
            text.startswith(f'Nuzky {version}\nSystem: ') and re.search(r'^Graphics: .+\((metal|vulkan|dx12|gl)\)$', text, re.M), text[:300])
    r.check('and the end of the log with the failed export and the cleaned interface error, without folders',
            re.search(r'^Last \d+ lines of the log:$', text, re.M) and 'Exporting out.mp4 failed' in text
            and 'Cannot draw clip.mp4' in text and 'Ahoj' not in text and secret_free(r, text), text[-600:])
    r.check('a toast says they were copied',
            wait(lambda: any(t['text'] == 'Diagnostics copied. Paste them into your bug report.' for t in r.s.run(TOASTS)), 3), r.s.run(TOASTS))

    # Killed like a crash: the next start says so and offers to report it.
    os.kill(app_pid(r), signal.SIGKILL)
    restart(r)
    notice = wait(lambda: next((t for t in r.s.run(TOASTS) if t['text'] == CRASHED), None), 10)
    r.check('after a crash the next start says Nuzky closed unexpectedly, with Report a bug…',
            notice and notice['kind'] == 'warning' and notice['action'] == 'Report a bug…', r.s.run(TOASTS))
    r.check('and the project opens with its clip', wait(lambda: r.track() == clips, 10), r.track())
    time.sleep(0.3)
    r.shot('crash-notice')
    before = len(opened(r))
    r.s.run(CLICK, '[data-toast]', 'Report a bug…')
    wait(lambda: len(opened(r)) > before, 5)
    url = urllib.parse.urlsplit(opened(r)[-1] if len(opened(r)) > before else '')
    query = dict(urllib.parse.parse_qsl(url.query))
    r.check('Report a bug… opens the bug form with this version and system',
            f'{url.scheme}://{url.netloc}{url.path}' == 'https://github.com/nuzkyapp/nuzky/issues/new'
            and query.get('template') == 'bug_report.yml' and query.get('version') == version and query.get('system'), (url, query))
    r.check('and copies the diagnostics to paste into it', wait(lambda: (r.s.run(COPIED) or [''])[-1].startswith(f'Nuzky {version}'), 3))

    close_window()
    # Stopping the session would end the app with SIGTERM, which also clears the marker; it has to quit by itself.
    r.check('the window closes and Nuzky quits', wait(lambda: (r.app.poll() is not None) if MACOS else app_pid(r) is None, 15))
    restart(r)
    time.sleep(1)
    r.check('after quitting normally the next start says nothing about a crash',
            all(t['text'] != CRASHED for t in r.s.run(TOASTS)), r.s.run(TOASTS))

    # A bug while drawing the window: the project vanishes from under the editor.
    r.s.run("window.__nuzky.store.setState({view: 'editor'})")
    time.sleep(0.5)
    r.s.run("const st = window.__nuzky.store; st.setState({snap: {...st.getState().snap, project: null}})")
    screen = wait(lambda: r.s.run("return document.querySelector('[role=alertdialog]')?.textContent ?? ''"), 5)
    r.check('a bug while drawing shows what happened instead of an empty window',
            screen.startswith('Something went wrong in this window') and 'Report a bug…' in screen, screen)
    r.check('Reload has the focus', r.s.run("return document.activeElement?.textContent === 'Reload'"))
    before = len(opened(r))
    r.s.run(CLICK, '[role=alertdialog]', 'Report a bug…')
    r.check('Report a bug… there copies the diagnostics, says so in the card and opens the bug form',
            wait(lambda: 'Diagnostics copied. Paste them into your bug report.' in r.s.run("return document.querySelector('[role=alertdialog]').textContent"), 5)
            and len(opened(r)) == before + 1 and 'template=bug_report.yml' in opened(r)[-1], opened(r)[before:])
    r.shot('window-error')
    r.check('the log records it', wait(lambda: 'ERROR ui: The window stopped drawing' in log_text(r), 5), log_text(r)[-400:])
    r.s.run(CLICK, '[role=alertdialog]', 'Reload')

    def reloaded():
        try:
            return r.s.run('return !!window.__nuzky?.store.getState().snap?.project') and r.track() == clips
        except RuntimeError:  # The page is reloading.
            return False
    r.check('Reload brings the project back', wait(reloaded, 30), r.s.run(TOASTS))

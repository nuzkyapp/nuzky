"""Shared parts of the end-to-end flows: the app under WebDriver, isolated directories, checks and proof.

A flow is a function decorated with @flow in its own file under tests/e2e/. It gets a Run, drives the real
app through the dev-only `window.__nuzky` hook and the keyboard, and records what the user would see
with r.check(...). Run flows with scripts/repro.py.

On Linux WebKitWebDriver drives the app and on Windows Microsoft Edge WebDriver drives its WebView2. macOS has
no WebDriver for an app's webview, so there the debug app answers the same requests itself, over a private
socket, when the harness starts it with a secret (src-tauri/src/test_bridge.rs).
"""
import base64, datetime, functools, json, os, queue, secrets, shutil, signal, socket, subprocess, sys, tempfile, threading, time
import traceback, urllib.error, urllib.request
from pathlib import Path

MACOS, WINDOWS = sys.platform == 'darwin', sys.platform == 'win32'
LINUX = not (MACOS or WINDOWS)
ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / 'tmp-test'
MODELS = FIXTURES / 'xdg/data/nuzky/models'
OUT = ROOT / 'tmp-test/repro'
TARGET = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target'))
APP, CLI, ANALYZE = (TARGET / 'debug' / (name + ('.exe' if WINDOWS else '')) for name in ('nuzky-app', 'nuzky', 'nuzky-analyze'))
WEBKIT_DRIVER = shutil.which('WebKitWebDriver') or '/usr/bin/WebKitWebDriver'
AI_EDITING = 'AI is editing. Stop it to edit yourself.'
FLOWS = {}
RUN = None  # The flow in progress; on macOS its app, socket and secret.


def edge_driver():
    """The Microsoft Edge WebDriver of the installed WebView2 version, which scripts/setup-windows-deps.ps1 fetches;
    a driver of another version refuses the webview."""
    import winreg
    key = r'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    for hive in (winreg.HKEY_LOCAL_MACHINE, winreg.HKEY_CURRENT_USER):
        try:
            with winreg.OpenKey(hive, key) as runtime:
                version = winreg.QueryValueEx(runtime, 'pv')[0]
            break
        except OSError:
            continue
    else:
        return Path('msedgedriver.exe (WebView2 is not installed)')
    deps = Path(os.environ.get('NUZKY_DEPS') or Path(os.environ['LOCALAPPDATA']) / 'Nuzky/build-deps')
    return deps / f'msedgedriver-{version}/msedgedriver.exe'


DRIVER = edge_driver() if WINDOWS else WEBKIT_DRIVER
DEBUG_PORT = 4445  # The webview's on Windows, which Edge WebDriver attaches to.


def flow(name, description, before=None, home=False, first_launch=False):
    """`home`: the flow starts on the home screen; others start in the editor of the restored project.
    `first_launch`: Get set up stays open when a fresh profile opens it by itself; other flows start without it."""
    def register(fn):
        fn.home = home
        fn.first_launch = first_launch
        FLOWS[name] = (description, before, fn)
        return fn
    return register


# --- WebDriver client for the app's webview, straight to WebKitWebDriver --------------------------------

def webdriver(method, path, body=None, retries=0):
    """Retries only queries, which are safe to send twice when a connection drops."""
    if MACOS:
        return _bridge(method, path, body, retries)
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request('http://127.0.0.1:4444' + path, data=data, method=method,
                                     headers={'Content-Type': 'application/json'})
    for attempt in range(retries + 1):
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                return json.loads(response.read() or b'{}').get('value')
        except urllib.error.HTTPError as e:
            raise RuntimeError(f'WebDriver {e.code}: {e.read()[:500]!r}') from None
        except (ConnectionResetError, ConnectionRefusedError):
            if attempt == retries:
                raise
            time.sleep(0.3)


def _bridge(method, path, body, retries):
    """The same requests to the app's own socket on macOS, one JSON line each way."""
    request = (json.dumps({'token': RUN.token, 'method': method, 'path': path, 'body': body}) + '\n').encode()
    for attempt in range(retries + 1):
        try:
            with socket.socket(socket.AF_UNIX) as connection:
                connection.settimeout(130)
                connection.connect(str(RUN.socket))
                connection.sendall(request)
                reply = json.loads(connection.makefile().readline() or '{"error": "the app closed the connection"}')
            break
        except (ConnectionResetError, ConnectionRefusedError, FileNotFoundError):
            if attempt == retries:
                raise
            time.sleep(0.3)
    if 'error' in reply:
        raise RuntimeError(f'WebDriver: {reply["error"][:500]}')
    return reply['value']


class Session:
    def __init__(self):
        if MACOS:
            # The app itself, started for each session as WebKitWebDriver starts it on Linux.
            RUN.socket.unlink(missing_ok=True)
            RUN.app = start([str(APP), *RUN.args], RUN.work / 'app.log', dict(RUN.env, NUZKY_TEST_BRIDGE=RUN.token))
            if not wait(lambda: RUN.app.poll() is not None or RUN.socket.exists(), 60) or RUN.app.poll() is not None:
                raise RuntimeError('the app did not start, see app.log')
            caps = {}
        elif WINDOWS:
            # Started here too: Edge WebDriver would give the webview a new profile for each session, so nothing the
            # page keeps would outlive a restart. It attaches to the webview's debugging port instead.
            webview = ' '.join(filter(None, [RUN.env.get('WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS'), f'--remote-debugging-port={DEBUG_PORT}']))
            RUN.app = start([str(APP), *RUN.args], RUN.work / 'app.log', RUN.launch_env(
                TAURI_WEBVIEW_AUTOMATION='true', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=webview))
            if not started(RUN.app, DEBUG_PORT, 60):
                raise RuntimeError('the app did not start, see app.log')
            caps = {'capabilities': {'alwaysMatch': {'browserName': 'webview2', 'ms:edgeOptions': {
                'debuggerAddress': f'127.0.0.1:{DEBUG_PORT}'}}}}
        else:
            # What tauri-driver sends on Linux. Talking to WebKitWebDriver directly avoids tauri-driver's connection
            # pool, whose stale connections now and then dropped a request mid-flow.
            caps = {'capabilities': {'alwaysMatch': {'browserName': 'wry',
                                                     'webkitgtk:browserOptions': {'binary': str(APP), 'args': RUN.args}}}}
        self.path = '/session/' + webdriver('POST', '/session', caps)['sessionId']
        if WINDOWS:
            # The page as large as under gamescope, whatever the display's scale; Windows would fit the window to
            # the screen in fractions of a page pixel.
            webdriver('POST', self.path + '/window/rect', {'width': 1440, 'height': 900})

    def run(self, script, *args, retries=0):
        return webdriver('POST', self.path + '/execute/sync', {'script': script, 'args': list(args)}, retries)

    def call(self, expression, *args):
        """Awaits a promise in the page; returns {'ok': True, 'value': …} or {'ok': False, 'error': …}."""
        script = ('const done = arguments[arguments.length - 1];'
                  f'Promise.resolve().then(() => {expression})'
                  '.then((v) => done({ok: true, value: typeof v === "string" || typeof v === "number" ? v : null}))'
                  '.catch((e) => done({ok: false, error: String(e)}))')
        return webdriver('POST', self.path + '/execute/async', {'script': script, 'args': list(args)})

    def screenshot(self, path):
        path.write_bytes(base64.b64decode(webdriver('GET', self.path + '/screenshot', retries=3)))

    def close(self):
        try:
            webdriver('DELETE', self.path)
        except Exception:
            pass
        if MACOS or WINDOWS:
            stop(RUN.app)


def wait(fn, timeout=30, step=0.2):
    end = time.time() + timeout
    while True:
        try:
            value = fn()
        except PermissionError:
            # Windows refuses to open a file while the app replaces it, and to replace one the app holds open.
            if not WINDOWS:
                raise
            value = None
        if value or time.time() > end:
            return value
        time.sleep(step)


# --- One flow: its directories, processes, checks and proof -------------------------------------------

STATE = """const st = window.__nuzky.store.getState(); const p = st.snap.project;
return {
  assets: p.assets.map((a) => ({id: a.id, name: a.name})),
  tracks: p.tracks.map((t) => ({id: t.id, kind: t.kind, name: t.name, clips: [...t.clips]
    .sort((a, b) => a.startUs - b.startUs)
    .map((c) => ({id: c.id, startUs: c.startUs, durationUs: c.durationUs, assetId: c.content.assetId ?? null,
                  text: c.content.text ?? null}))})),
  thumbs: Object.keys(st.thumbs), aiRun: st.aiRun, epoch: st.snap.sessionEpoch,
  toasts: st.toasts.map((t) => ({kind: t.kind, text: t.text})),
  jobs: Object.values(st.jobs).map((j) => ({kind: j.kind, status: j.status, message: j.message})),
};"""
KEY = """const target = document.activeElement || document.body;
target.dispatchEvent(new KeyboardEvent('keydown', Object.assign({key: arguments[0], bubbles: true, cancelable: true}, arguments[1])));"""


class Run:
    def __init__(self, name, runtime):
        self.name, self.work = name, OUT / name
        shutil.rmtree(self.work, ignore_errors=True)
        (self.work / 'data/nuzky/projects').mkdir(parents=True)
        self.env = dict(os.environ, XDG_DATA_HOME=str(self.work / 'data'), XDG_CACHE_HOME=str(self.work / 'cache'),
                        XDG_RUNTIME_DIR=runtime, DBUS_SESSION_BUS_ADDRESS='disabled:', GDK_BACKEND='x11',
                        # Flows never ask GitHub for a new version; tests/e2e/updates.py serves its own.
                        NUZKY_NO_UPDATE_CHECK='1',
                        # Nor does the app read the ChatGPT sign-in from the user's keychain or reach OpenAI; on macOS
                        # a separate home does not separate the keychain. tests/e2e/chatgpt.py serves its own issuer.
                        NUZKY_CHATGPT_ISSUER='http://127.0.0.1:9', NUZKY_TEST_KEYCHAIN=str(self.work / 'keychain.json'),
                        # Nor does it run the person's own agents; a flow that needs one puts a stand-in there.
                        NUZKY_AGENT_CLAUDE=str(self.work / 'no-agent'), NUZKY_AGENT_CODEX=str(self.work / 'no-agent'),
                        NUZKY_AGENT_AGY=str(self.work / 'no-agent'), PYTHONDONTWRITEBYTECODE='1')
        # A data folder the person chose for their own work would win over XDG_DATA_HOME.
        self.env.pop('NUZKY_DATA_DIR', None)
        self.checks, self.shots, self.s, self.app, self.driver = [], [], None, None, None
        # Arguments for the app, such as the system language on macOS (`-AppleLanguages (cs)`).
        self.args = []
        if MACOS:
            # macOS keeps Nuzky's data and cache under ~/Library and WebKit's storage under the home folder, whatever
            # XDG_* say, so the app gets a home of its own whose Library folders are this run's.
            home = self.work / 'home'
            for folder in (home / 'Library/Application Support', home / 'Library/Caches', self.work / 'cache/nuzky'):
                folder.mkdir(parents=True)
            os.symlink(self.work / 'data/nuzky', home / 'Library/Application Support/nuzky')
            os.symlink(self.work / 'cache/nuzky', home / 'Library/Caches/nuzky')
            self.env.update(HOME=str(home), CFFIXED_USER_HOME=str(home))
            self.socket, self.token = Path(runtime) / 'nuzky/webdriver.sock', secrets.token_hex(32)
        if WINDOWS:
            # Windows names a user's folders under USERPROFILE, so the app gets a home of its own as on macOS: its
            # cache, the webview's storage and CapCut's drafts live in this run's AppData, with the cache joined to
            # this run's folder. NUZKY_DATA_DIR moves the data folder, which roams elsewhere. The CLI the flows run
            # gets the same home, so it shares the app's cache and never writes the person's own.
            home = self.work / 'home'
            for folder in (home / 'AppData/Local', home / 'AppData/Roaming', self.work / 'cache/nuzky'):
                folder.mkdir(parents=True)
            subprocess.run(['cmd', '/c', 'mklink', '/J', str(home / 'AppData/Local/nuzky'), str(self.work / 'cache/nuzky')],
                           check=True, capture_output=True)
            self.env.update(HOME=str(home), USERPROFILE=str(home), NUZKY_DATA_DIR=str(self.work / 'data/nuzky'))

    def launch_env(self, **extra):
        """r.env for a process this run starts; on Windows the home a flow set in HOME is the one apps read."""
        env = dict(self.env, **extra)
        if WINDOWS:
            env['USERPROFILE'] = env['HOME']
            # Without them Windows names none of the app's folders and Tauri falls back to its own.
            for folder in ('AppData/Local', 'AppData/Roaming'):
                (Path(env['HOME']) / folder).mkdir(parents=True, exist_ok=True)
            for variable in ('NUZKY_AGENT_CLAUDE', 'NUZKY_AGENT_CODEX', 'NUZKY_AGENT_AGY'):
                path = Path(env[variable])
                if path.suffix:
                    continue
                if path.parent == STAND_INS.parent and path.name in ('fake_agent', 'fake_codex', 'fake_agy'):
                    path = windows_stand_in(self.work / 'stand-ins' / path.name, path.name)
                elif path.is_file():
                    # A flow's copy of the stand-in, which it may take away.
                    path = windows_stand_in(path, path.name, script=path)
                # A stand-in a flow installs later goes here.
                env[variable] = str(path.with_suffix('.cmd'))
            if 'NUZKY_TEST_BROWSER' in env:
                browser = Path(env['NUZKY_TEST_BROWSER'])
                env['NUZKY_TEST_BROWSER'] = str(windows_stand_in(self.work / 'stand-ins' / browser.name, browser.name, script=browser))
        return env

    def check(self, name, ok, detail=None):
        self.checks.append({'check': name, 'ok': bool(ok), 'detail': detail})
        print(f"  {'ok  ' if ok else 'FAIL'} {name}" + ('' if ok or detail is None else f': {detail}'), flush=True)
        return bool(ok)

    def shot(self, name):
        self.s.screenshot(self.work / f'{name}.png')
        if f'{name}.png' not in self.shots:
            self.shots.append(f'{name}.png')

    def state(self):
        return self.s.run(STATE, retries=3)

    def track(self, track_id='main'):
        return next(t['clips'] for t in self.state()['tracks'] if t['id'] == track_id)

    def key(self, key, **modifiers):
        self.s.run(KEY, key, modifiers)

    def focus_clip(self, clip_id):
        self.s.run("document.querySelector(`[data-clip-id='${arguments[0]}']`).focus();"
                   "window.__nuzky.store.getState().select([arguments[0]]);", clip_id)

    def seek(self, us):
        self.s.run('window.__nuzky.store.getState().seek(arguments[0])', us)

    def import_media(self, *paths):
        count = len(self.state()['assets']) + len(paths)
        self.s.call('window.__nuzky.importPaths(arguments[0])', [str(p) for p in paths])
        if not wait(lambda: len(self.state()['assets']) == count):
            raise RuntimeError(f'import did not finish: {self.state()["assets"]}')

    def add_clip(self, name):
        count = len(self.track())
        added = self.s.call("""(() => { const st = window.__nuzky.store.getState();
            const asset = st.snap.project.assets.find((a) => a.name === arguments[0]);
            return st.edit({type: 'addClip', trackId: 'main', assetId: asset.id, startUs: null}); })()""", name)
        if not added['ok'] or not wait(lambda: len(self.track()) == count + 1):
            raise RuntimeError(f'{name} was not placed: {added}')

    def errors(self):
        return [t['text'] for t in self.state()['toasts'] if t['kind'] == 'error']

    def saved_project(self):
        return next((self.work / 'data/nuzky/projects').glob('*.nuzky'))


STAND_INS = ROOT / 'tests/e2e/fake_agent'


def windows_stand_in(path, name, script=STAND_INS, **env):
    """The stand-in program `name`, such as fake_codex or fake_browser, at `path` on Windows, which runs neither a
    shebang nor git's links: a .cmd that runs the Python `script` as `name`, as npm's .cmd runs Codex with Node.
    Returns its path. `env`: how it answers, such as FAKE_AGENT_SIGNED_OUT."""
    launcher = path.with_suffix('.cmd')
    launcher.parent.mkdir(parents=True, exist_ok=True)
    lines = ['@echo off', f'set FAKE_AGENT_AS={name}', 'set FAKE_AGENT_LAUNCHER=%~f0', *(f'set {k}={v}' for k, v in env.items()),
             f'"{sys.executable}" -X utf8 "{script}" %*']
    launcher.write_text('\r\n'.join(lines) + '\r\n')
    return launcher


def start(command, log, env=None):
    group = {'creationflags': subprocess.CREATE_NEW_PROCESS_GROUP} if WINDOWS else {'start_new_session': True}
    return subprocess.Popen(command, cwd=ROOT, env=env, stdout=open(log, 'w'), stderr=subprocess.STDOUT, **group)


def stop(process, timeout=5):
    if WINDOWS:
        # The whole tree, as the process group on Unix: the app Edge WebDriver started, the node npx started.
        subprocess.run(['taskkill', '/T', '/F', '/PID', str(process.pid)], capture_output=True)
        try:
            process.wait(timeout)
        except subprocess.TimeoutExpired:
            pass
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout)
    except ProcessLookupError:
        pass
    except PermissionError:
        # macOS refuses to signal a group whose leader has ended unwaited, such as the app after its window closed.
        process.wait(timeout)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)


def started(process, port, timeout):
    """True once `port` answers while `process` still runs; a server that exited left the port to someone else."""
    wait(lambda: process.poll() is not None or port_busy(port), timeout)
    return process.poll() is None and port_busy(port)


def port_busy(port):
    """Vite may listen on ::1 only, so every address of localhost counts."""
    try:
        socket.create_connection(('localhost', port), timeout=1).close()
        return True
    except OSError:
        return False


def start_driver(r):
    """The WebDriver, which on Linux starts the app with its own environment, so r.env as it is now."""
    # Tauri 2 lets WebDriver drive its webview only with this set, as tauri-driver does.
    host = [] if WINDOWS else ['--host=127.0.0.1']
    r.driver = start([str(DRIVER), '--port=4444', *host], r.work / 'driver.log', r.launch_env(TAURI_WEBVIEW_AUTOMATION='true'))
    if not started(r.driver, 4444, 20):
        raise RuntimeError(f'{Path(DRIVER).name} did not start, see driver.log')


def run_flow(name):
    description, before, body = FLOWS[name]
    print(f'\n==> {name}: {description}', flush=True)
    # Unix socket paths must stay short, and macOS's TMPDIR is long.
    runtime = tempfile.mkdtemp(prefix='nuzky-repro-', dir='/tmp' if MACOS else None)
    global RUN
    r = RUN = Run(name, runtime)
    error = None
    try:
        if before:
            before(r)
        if not MACOS:
            # A WebDriver that is not ours would launch the app outside this run's directories.
            if not wait(lambda: not port_busy(4444), 10):
                raise RuntimeError('port 4444 is in use by another session')
            start_driver(r)
        r.s = Session()
        if not wait(lambda: r.s.run('return !!window.__nuzky?.store.getState().snap', retries=3), 60):
            raise RuntimeError('the app did not load a project')
        # Decided once the projects are listed, which the home screen asks for as it opens.
        if not body.first_launch:
            wait(lambda: r.s.run('return window.__nuzky.tour.getState().firstLaunch !== null', retries=3), 15)
            r.s.run('const s = window.__nuzky.setup.getState(); if (s.open) s.closeSetup();')
        if not body.home:
            r.s.run("window.__nuzky.store.setState({view: 'editor'})")
        time.sleep(1)
        body(r)
    except Exception:
        error = traceback.format_exc()
        print(error, flush=True)
    finally:
        if r.s:
            r.s.close()
        if r.driver:
            stop(r.driver)
        if r.app:
            stop(r.app)
        shutil.rmtree(runtime, ignore_errors=True)
    revision = subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    dirty = bool(subprocess.run(['git', 'status', '--porcelain'], cwd=ROOT, capture_output=True, text=True).stdout)
    result = {'flow': name, 'description': description, 'passed': not error and bool(r.checks) and all(c['ok'] for c in r.checks),
              'checks': r.checks, 'error': error, 'screenshots': r.shots, 'command': f'python3 scripts/repro.py {name}',
              'revision': revision, 'uncommitted_changes': dirty, 'finished': datetime.datetime.now().isoformat(timespec='seconds')}
    (r.work / 'result.json').write_text(json.dumps(result, indent=2, ensure_ascii=False, default=lambda o: sorted(o) if isinstance(o, set) else str(o)) + '\n')


# --- Helpers shared by flows ----------------------------------------------------------------------------

def preview_rect(r):
    return r.s.run("""const b = document.querySelector('section[aria-label=Preview] canvas').getBoundingClientRect();
        return {left: b.left, top: b.top, width: b.width, height: b.height, viewport: window.innerWidth};""")


def preview_crop(screenshot, rect):
    from PIL import Image
    image = Image.open(screenshot).convert('RGB')
    scale = image.width / rect['viewport']
    left, top = int(rect['left'] * scale), int(rect['top'] * scale)
    return image.crop((left, top, left + int(rect['width'] * scale), top + int(rect['height'] * scale)))


def preview_redraw(r, name, before, timeout=6):
    """Screenshots the preview until it differs from `before` (a crop), so the picture follows the last edit.
    Returns the crop and whether the preview changed in time."""
    rect = preview_rect(r)
    end = time.time() + timeout
    while True:
        r.shot(name)
        crop = preview_crop(r.work / f'{name}.png', rect)
        if changed_share(crop, before) > 0:
            return crop, True
        if time.time() > end:
            return crop, False
        time.sleep(0.5)


def preview_brightness(r, screenshot, fraction):
    """Mean brightness, 0 to 255, of the top `fraction` of the preview."""
    from PIL import ImageStat
    crop = preview_crop(screenshot, preview_rect(r))
    band = crop.crop((0, 0, crop.width, max(1, int(crop.height * fraction)))).convert('L')
    return ImageStat.Stat(band).mean[0]


def export(r, name):
    """Exports the timeline at 720p to `name` in the flow's folder and checks that the job finishes."""
    target = r.work / name
    started = r.s.call('window.__nuzky.api.startExport(arguments[0], arguments[1], arguments[2], arguments[3])',
                       str(target), {'resolution': 720, 'fps': 30, 'quality': 'small'}, r.state()['epoch'], True)
    if not started['ok']:
        raise RuntimeError(f'export {name} did not start: {started}')
    job = wait(lambda: (j := r.s.run("return window.__nuzky.store.getState().jobs[arguments[0]] ?? null", started['value']))
               and j['status'] != 'running' and j, 180)
    r.check(f'the export {name} finishes', job and job['status'] == 'done', job)
    return target


def ffprobe(path):
    out = subprocess.run(['ffprobe', '-v', 'error', '-show_entries', 'format=duration:stream=codec_name', '-of', 'json', path],
                         capture_output=True, text=True, check=True).stdout
    data = json.loads(out)
    return float(data['format']['duration']), {s['codec_name'] for s in data['streams']}


def changed_share(a, b):
    """Share of pixels whose brightness clearly differs, for two image paths or Pillow images. Encoding noise
    stays below the threshold; the neighbouring frame of the test pattern changes about 1 % of the picture."""
    from PIL import Image, ImageChops, ImageStat
    first, second = (Image.open(x) if isinstance(x, (str, Path)) else x for x in (a, b))
    first, second = first.convert('L'), second.convert('L')
    if first.size != second.size:
        return 1.0
    return ImageStat.Stat(ImageChops.difference(first, second).point(lambda v: 255 if v > 48 else 0)).mean[0] / 255


def _windows(d):
    found, stack = [], [d.screen().root]
    while stack:
        w = stack.pop()
        try:
            name = w.get_wm_name()
            stack.extend(w.query_tree().children)
        except Exception:
            continue
        if name:
            found.append((w, name.decode() if isinstance(name, bytes) else name))
    return found


# WebDriver's codes for the X11 key names the flows use.
WEBDRIVER_KEYS = {'Return': '\ue007', 'Tab': '\ue004', 'space': ' ', 'BackSpace': '\ue003', 'Escape': '\ue00c',
                  'F8': '\ue038', 'Home': '\ue011', 'Delete': '\ue017', 'End': '\ue010', 'Left': '\ue012', 'Up': '\ue013',
                  'Right': '\ue014', 'Down': '\ue015'}


def actions(*sources):
    """WebDriver input on Windows, which the webview handles as a person's, whether or not its window has focus."""
    webdriver('POST', RUN.s.path + '/actions', {'actions': list(sources)})


def press(key):
    """A real key press into the app window, an X11 key name or a character: a focused button acts on Enter and
    focus moves on Tab as they do for a person, which key events made by the page do not do. Through XTest under
    gamescope, through the window itself on macOS and through WebDriver's actions on Windows; WebKitWebDriver has
    no key input."""
    if MACOS:
        webdriver('POST', '/session/nuzky/nuzky/keys', {'keys': [key]})
        return
    if WINDOWS:
        value = WEBDRIVER_KEYS.get(key, key)
        actions({'type': 'key', 'id': 'keyboard', 'actions': [{'type': 'keyDown', 'value': value}, {'type': 'keyUp', 'value': value}]})
        return
    from Xlib import X, XK, display
    from Xlib.ext import xtest
    d = display.Display()
    for window, name in _windows(d):
        if name == 'Nuzky':
            window.set_input_focus(X.RevertToParent, X.CurrentTime)
    keysym = XK.string_to_keysym(key)
    code = d.keysym_to_keycode(keysym)
    shift = d.keycode_to_keysym(code, 0) != keysym
    if shift:
        xtest.fake_input(d, X.KeyPress, d.keysym_to_keycode(XK.XK_Shift_L))
    for kind in (X.KeyPress, X.KeyRelease):
        xtest.fake_input(d, kind, code)
    if shift:
        xtest.fake_input(d, X.KeyRelease, d.keysym_to_keycode(XK.XK_Shift_L))
    d.sync()
    d.close()


def set_clipboard(text):
    """Puts the person's own text back on the system clipboard of macOS or Windows after a flow copied to it."""
    if MACOS:
        subprocess.run(['pbcopy'], input=text, text=True, check=True)
    else:
        win32.set_clipboard(text)


def clipboard():
    """The text the app put on the clipboard: on macOS and Windows the system's (a flow that copies puts the person's
    text back afterwards), on Linux the one of gamescope's own X server, asked for as UTF-8 like any app would."""
    if MACOS:
        return subprocess.run(['pbpaste'], capture_output=True, text=True, check=True).stdout
    if WINDOWS:
        return win32.clipboard()
    from Xlib import X, display
    d = display.Display()
    try:
        window = d.screen().root.create_window(0, 0, 1, 1, 0, X.CopyFromParent)
        into = d.intern_atom('NUZKY_CLIPBOARD')
        window.convert_selection(d.intern_atom('CLIPBOARD'), d.intern_atom('UTF8_STRING'), into, X.CurrentTime)
        d.flush()
        deadline = time.time() + 5
        while time.time() < deadline:
            while d.pending_events():
                event = d.next_event()
                if event.type == X.SelectionNotify:
                    found = event.property != X.NONE and window.get_full_property(into, X.AnyPropertyType)
                    return found.value.decode() if found else ''
            time.sleep(0.05)
        return None
    finally:
        d.close()


def pointer(kind, x, y, clicks=1):
    """macOS and Windows: the left mouse button `down` or `up` at a point of the page. On Windows the webview counts
    the clicks itself, from their timing."""
    if WINDOWS:
        button = {'type': 'pointerDown' if kind == 'down' else 'pointerUp', 'button': 0}
        actions({'type': 'pointer', 'id': 'mouse', 'parameters': {'pointerType': 'mouse'}, 'actions': [
            {'type': 'pointerMove', 'duration': 0, 'origin': 'viewport', 'x': round(x), 'y': round(y)}, button]})
        return
    webdriver('POST', '/session/nuzky/nuzky/pointer', {'kind': kind, 'x': x, 'y': y, 'clicks': clicks})


def close_window(d=None):
    """Closes the app window like the title-bar button."""
    if MACOS:
        webdriver('DELETE', '/session/nuzky/window')
        return None
    if WINDOWS:
        for window in win32.windows('Nuzky'):
            win32.close(window)
        return None
    from Xlib import X, display
    from Xlib.protocol import event
    d = d or display.Display()
    protocols, delete = d.intern_atom('WM_PROTOCOLS'), d.intern_atom('WM_DELETE_WINDOW')
    for w, name in _windows(d):
        if name == 'Nuzky':
            w.send_event(event.ClientMessage(window=w, client_type=protocols, data=(32, [delete, X.CurrentTime, 0, 0, 0])))
    d.sync()
    return d


def leave_editor(r):
    """In the editor the close button goes back to the projects; a flow that quits closes the window once more."""
    view = lambda: r.s.run('return window.__nuzky.store.getState().view', retries=3)
    if view() == 'editor':
        close_window()
        if not wait(lambda: view() == 'home', 10):
            raise RuntimeError('closing the window in the editor did not show the projects')


def close_window_and_confirm(r):
    """Closes the app window like the title-bar button and confirms the native quit question with Enter."""
    if MACOS:
        close_window()

        def question():
            try:
                return any('Quit Nuzky' in text for text in webdriver('GET', r.s.path + '/alert/text'))
            except RuntimeError:
                return False
        # AppKit draws the dialog's glass only on screen, so the proof here is its text, not a picture.
        if not wait(question, 5, 0.1):
            return False
        (r.work / 'quit-question.txt').write_text('\n'.join(webdriver('GET', r.s.path + '/alert/text')) + '\n')
        webdriver('POST', r.s.path + '/alert/accept')
        return True
    if WINDOWS:
        close_window()
        dialog = wait(lambda: next(iter(win32.windows('Quit Nuzky?')), None), 5, 0.1)
        if not dialog:
            return False
        time.sleep(0.8)
        win32.capture(dialog).save(r.work / 'quit-question.png')
        r.shots.append('quit-question.png')
        win32.press_default(dialog)
        return True
    from Xlib import X, XK
    from Xlib.protocol import event
    d = close_window()
    windows = lambda: _windows(d)
    dialog = wait(lambda: next((w for w, name in windows() if 'Quit Nuzky' in name), None), 5, 0.1)
    if not dialog:
        return False
    time.sleep(0.8)
    geometry = dialog.get_geometry()
    raw = dialog.get_image(0, 0, geometry.width, geometry.height, X.ZPixmap, 0xffffffff)
    from PIL import Image
    Image.frombytes('RGB', (geometry.width, geometry.height), raw.data, 'raw', 'BGRX').save(r.work / 'quit-question.png')
    r.shots.append('quit-question.png')
    code = d.keysym_to_keycode(XK.string_to_keysym('Return'))
    for kind in (event.KeyPress, event.KeyRelease):
        dialog.send_event(kind(time=X.CurrentTime, root=d.screen().root, window=dialog, same_screen=1, child=X.NONE,
                               root_x=0, root_y=0, event_x=0, event_y=0, state=0, detail=code), propagate=True)
    d.sync()
    return True


class win32:
    """The few Windows calls the flows make where X11 serves them on Linux: windows by title, the clipboard,
    the app's processes and their memory."""

    @staticmethod
    @functools.cache
    def _api():
        import ctypes
        from ctypes import wintypes
        user32, kernel32 = ctypes.WinDLL('user32', use_last_error=True), ctypes.WinDLL('kernel32', use_last_error=True)
        user32.EnumWindows.argtypes = [ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM), wintypes.LPARAM]
        user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
        user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
        user32.GetClipboardData.restype = wintypes.HANDLE
        kernel32.GlobalLock.argtypes, kernel32.GlobalLock.restype = [wintypes.HGLOBAL], ctypes.c_void_p
        kernel32.GlobalUnlock.argtypes = [wintypes.HGLOBAL]
        kernel32.OpenProcess.restype = wintypes.HANDLE
        kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel32.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
        kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
        return ctypes, wintypes, user32, kernel32

    @staticmethod
    def windows(title):
        """The visible top-level windows with exactly this title of this run's app, never of a Nuzky the person has open."""
        ctypes, wintypes, user32, _ = win32._api()
        found = []

        def each(hwnd, _):
            text, pid = ctypes.create_unicode_buffer(256), wintypes.DWORD()
            user32.GetWindowTextW(hwnd, text, 256)
            user32.GetWindowThreadProcessId(wintypes.HWND(hwnd), ctypes.byref(pid))
            if text.value == title and user32.IsWindowVisible(hwnd) and RUN.app and pid.value == RUN.app.pid:
                found.append(hwnd)
            return True
        user32.EnumWindows(ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)(each), 0)
        return found

    @staticmethod
    def close(hwnd):
        """The title-bar close button's message."""
        win32._api()[2].PostMessageW(hwnd, 0x0010, 0, 0)

    @staticmethod
    def capture(hwnd):
        """The window as it is on screen. The frame DWM reports is in physical pixels, as ImageGrab takes them."""
        ctypes, wintypes, _, _ = win32._api()
        from PIL import ImageGrab
        rect = wintypes.RECT()
        ctypes.windll.dwmapi.DwmGetWindowAttribute(wintypes.HWND(hwnd), 9, ctypes.byref(rect), ctypes.sizeof(rect))
        return ImageGrab.grab((rect.left, rect.top, rect.right, rect.bottom), all_screens=True)

    @staticmethod
    def press_default(hwnd):
        """Return into the dialog's focused control, as a person presses it, so the dialog acts on its default button."""
        ctypes, wintypes, user32, _ = win32._api()

        class GuiThreadInfo(ctypes.Structure):
            _fields_ = [('cbSize', wintypes.DWORD), ('flags', wintypes.DWORD), ('hwndActive', wintypes.HWND),
                        ('hwndFocus', wintypes.HWND), ('hwndCapture', wintypes.HWND), ('hwndMenuOwner', wintypes.HWND),
                        ('hwndMoveSize', wintypes.HWND), ('hwndCaret', wintypes.HWND), ('rcCaret', wintypes.RECT)]
        info = GuiThreadInfo(cbSize=ctypes.sizeof(GuiThreadInfo))
        user32.GetGUIThreadInfo(user32.GetWindowThreadProcessId(wintypes.HWND(hwnd), None), ctypes.byref(info))
        target = info.hwndFocus or hwnd
        user32.PostMessageW(target, 0x0100, 0x0D, 0x001C0001)
        user32.PostMessageW(target, 0x0101, 0x0D, 0xC01C0001)

    @staticmethod
    def clipboard():
        """The text on the clipboard; another program may hold it for a moment."""
        ctypes, _, user32, kernel32 = win32._api()
        if not wait(lambda: user32.OpenClipboard(None), 2, 0.05):
            return None
        try:
            handle = user32.GetClipboardData(13)
            if not handle:
                return ''
            try:
                return ctypes.wstring_at(kernel32.GlobalLock(handle))
            finally:
                kernel32.GlobalUnlock(handle)
        finally:
            user32.CloseClipboard()

    @staticmethod
    def set_clipboard(text):
        ctypes, _, user32, kernel32 = win32._api()
        data = (text + '\0').encode('utf-16-le')
        kernel32.GlobalAlloc.restype = ctypes.c_void_p
        handle = kernel32.GlobalAlloc(0x0002, len(data))
        ctypes.memmove(kernel32.GlobalLock(ctypes.c_void_p(handle)), data, len(data))
        kernel32.GlobalUnlock(ctypes.c_void_p(handle))
        if not wait(lambda: user32.OpenClipboard(None), 2, 0.05):
            raise RuntimeError('the clipboard stayed busy')
        try:
            user32.EmptyClipboard()
            user32.SetClipboardData.argtypes = [ctypes.c_uint, ctypes.c_void_p]
            user32.SetClipboardData(13, handle)
        finally:
            user32.CloseClipboard()

    @staticmethod
    def processes():
        """Every process as {pid: (parent pid, full path of its program or None)}."""
        ctypes, wintypes, _, kernel32 = win32._api()

        class Entry(ctypes.Structure):
            _fields_ = [('dwSize', wintypes.DWORD), ('cntUsage', wintypes.DWORD), ('th32ProcessID', wintypes.DWORD),
                        ('th32DefaultHeapID', ctypes.c_size_t), ('th32ModuleID', wintypes.DWORD), ('cntThreads', wintypes.DWORD),
                        ('th32ParentProcessID', wintypes.DWORD), ('pcPriClassBase', ctypes.c_long), ('dwFlags', wintypes.DWORD),
                        ('szExeFile', ctypes.c_wchar * 260)]
        snapshot, entry, found = kernel32.CreateToolhelp32Snapshot(2, 0), Entry(dwSize=ctypes.sizeof(Entry)), {}
        try:
            more = kernel32.Process32FirstW(wintypes.HANDLE(snapshot), ctypes.byref(entry))
            while more:
                path, size = ctypes.create_unicode_buffer(1024), wintypes.DWORD(1024)
                process = kernel32.OpenProcess(0x1000, False, entry.th32ProcessID)
                ok = process and kernel32.QueryFullProcessImageNameW(process, 0, path, ctypes.byref(size))
                if process:
                    kernel32.CloseHandle(process)
                found[entry.th32ProcessID] = (entry.th32ParentProcessID, path.value if ok else None)
                more = kernel32.Process32NextW(wintypes.HANDLE(snapshot), ctypes.byref(entry))
        finally:
            kernel32.CloseHandle(snapshot)
        return found

    @staticmethod
    def app_pid():
        """The app of this run: Edge WebDriver starts it, and one run at a time uses this build."""
        app = str(APP.resolve()).lower()
        return next((pid for pid, (_, path) in win32.processes().items() if path and path.lower() == app), None)

    @staticmethod
    def alive(pid):
        """Whether the process still runs."""
        ctypes, wintypes, _, kernel32 = win32._api()
        process = kernel32.OpenProcess(0x1000, False, pid)
        if not process:
            return False
        try:
            code = wintypes.DWORD()
            return bool(kernel32.GetExitCodeProcess(wintypes.HANDLE(process), ctypes.byref(code))) and code.value == 259
        finally:
            kernel32.CloseHandle(process)

    @staticmethod
    def memory_mb(pid):
        """The process's resident memory, its working set, or None once it is gone."""
        ctypes, wintypes, _, kernel32 = win32._api()

        class Counters(ctypes.Structure):
            _fields_ = [('cb', wintypes.DWORD), ('PageFaultCount', wintypes.DWORD)] + [
                (name, ctypes.c_size_t) for name in ('PeakWorkingSetSize', 'WorkingSetSize', 'QuotaPeakPagedPoolUsage',
                                                     'QuotaPagedPoolUsage', 'QuotaPeakNonPagedPoolUsage', 'QuotaNonPagedPoolUsage',
                                                     'PagefileUsage', 'PeakPagefileUsage')]
        process = kernel32.OpenProcess(0x1000, False, pid)
        if not process:
            return None
        try:
            counters = Counters(cb=ctypes.sizeof(Counters))
            ok = ctypes.WinDLL('psapi').GetProcessMemoryInfo(wintypes.HANDLE(process), ctypes.byref(counters), counters.cb)
            return counters.WorkingSetSize / 1048576 if ok else None
        finally:
            kernel32.CloseHandle(process)


class Bridge:
    """The agent's side: `nuzky mcp` attached to the open app over its local socket."""

    def __init__(self, r, project=None, command=None):
        """`command`: what an agent's config runs instead, such as `nuzky-app mcp --current`."""
        command = command or [str(CLI), 'mcp', '--project', str(project), '--allow-write']
        self.process = subprocess.Popen(command, env=r.launch_env(),
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(r.work / 'bridge.log', 'w'),
                                        text=True, start_new_session=True)
        self.lines, self.id = queue.Queue(), 0
        threading.Thread(target=lambda: [self.lines.put(line) for line in self.process.stdout], daemon=True).start()
        try:
            self.rpc('initialize', {'protocolVersion': '2025-03-26', 'capabilities': {}, 'clientInfo': {'name': 'repro', 'version': '1'}})
            self.send({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
        except BaseException:
            stop(self.process)
            raise

    def send(self, message):
        self.process.stdin.write(json.dumps(message) + '\n')
        self.process.stdin.flush()

    def rpc(self, method, params):
        self.id += 1
        self.send({'jsonrpc': '2.0', 'id': self.id, 'method': method, 'params': params})
        while True:
            message = json.loads(self.lines.get(timeout=30))
            if message.get('id') == self.id:
                return message

    def call(self, tool, arguments):
        response = self.rpc('tools/call', {'name': tool, 'arguments': arguments})
        if 'error' in response or response['result'].get('isError'):
            raise RuntimeError(f'{tool} failed: {response}')
        return response['result']['structuredContent']

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(10)
        except subprocess.TimeoutExpired:
            stop(self.process)


def deny(path, mode, access='WD,AD'):
    """Takes away the right to write `path` (or with access 'R', to read it) until the returned function gives it back:
    `mode` on Unix, and on Windows, which ignores a folder's read-only flag, a deny entry for the user in its access list.
    Its default denies adding files and folders; icacls' W also denies the right to wait on a handle, without which a
    folder cannot even be listed."""
    if WINDOWS:
        user = os.environ['USERNAME']
        subprocess.run(['icacls', str(path), '/deny', f'{user}:({access})'], check=True, capture_output=True)
        return lambda: subprocess.run(['icacls', str(path), '/remove:d', user], check=True, capture_output=True)
    before = path.stat().st_mode & 0o777
    os.chmod(path, mode)
    return lambda: os.chmod(path, before)


@functools.cache
def _launcher():
    """tests/e2e/launcher.rs, built once per run of the flows."""
    built = TARGET / 'e2e/launcher.exe'
    built.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(['rustc', '--edition', '2024', '-O', '-o', str(built), str(ROOT / 'tests/e2e/launcher.rs')],
                   env=dict(os.environ, NUZKY_PYTHON=sys.executable), check=True, capture_output=True)
    return built


def program(r, script, name=None):
    """A path the app can run as the fake program `script` (a Python file in tests/e2e), called `name` or as the file
    is. Unix runs the script itself; Windows runs a copy of tests/e2e/launcher.rs named for it, in this run's folder,
    since the app looks for yt-dlp and keeps its own copy only as an .exe."""
    name = name or script.name
    if not WINDOWS:
        return script
    exe = r.work / 'bin' / f'{name}.exe'
    exe.parent.mkdir(exist_ok=True)
    shutil.copy(_launcher(), exe)
    r.env[f"NUZKY_FAKE_{name.upper().replace('-', '_')}"] = str(script)
    return exe


def running_with(text):
    """The ids of the processes whose command line holds `text`, apart from the one that looks."""
    if WINDOWS:
        query = f"$_.CommandLine -like '*{text}*' -and $_.ProcessId -ne $PID"
        listed = subprocess.run(['powershell', '-NoProfile', '-Command',
                                 f'(Get-CimInstance Win32_Process | Where-Object {{ {query} }}).ProcessId'],
                                capture_output=True, text=True).stdout
    else:
        listed = subprocess.run(['pgrep', '-f', text], capture_output=True, text=True).stdout
    return listed.split()


def kill_hard(pid):
    """Ends a process at once, as a crash does; Windows has no SIGKILL, and its TerminateProcess is the same."""
    os.kill(pid, getattr(signal, 'SIGKILL', signal.SIGTERM))


def comma_locale(r):
    """Starts the app with numbers written with a decimal comma, as on a Czech system, when such a locale is installed;
    the person model once saw nobody under one. Returns the locale, or None. Windows programs do not take theirs
    from the environment."""
    if WINDOWS:
        return None
    installed = subprocess.run(['locale', '-a'], capture_output=True, text=True).stdout.split()
    found = next((name for name in ('cs_CZ.utf8', 'de_DE.utf8', 'fr_FR.utf8') if name in installed), None)
    if found:
        # LC_ALL would win over it.
        r.env.pop('LC_ALL', None)
        r.env['LC_NUMERIC'] = found
    return found


# The word timing models recognition downloads on first use; linked so no flow reaches the network.
ALIGN_MODELS = ('wav2vec2-xls-r-300m-cs-250-q8_0.gguf', 'wav2vec2-base-960h.gguf')


def link_models(r, names=('ggml-small.bin', 'ggml-silero-v5.1.2.bin')):
    models = r.work / 'data/nuzky/models'
    models.mkdir(parents=True)
    for name in (*names, *ALIGN_MODELS):
        try:
            os.link(MODELS / name, models / name)
        except OSError:
            shutil.copy(MODELS / name, models / name)

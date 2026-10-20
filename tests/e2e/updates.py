"""Update checks and installing. The foot of the home sidebar says when a newer Nuzky was released, from a latest.json
served here in place of GitHub's. The automatic check runs 10 s after start and at most once a day, also across launches,
says nothing when it fails and can be turned off; Check for updates always answers in a toast. Install and restart
downloads the package latest.json names, signed here with a throwaway key, with its progress and a Stop, and keeps it;
with nothing running Nuzky restarts into it after a 5 s countdown, otherwise Restart to update waits for the work. This
debug build installs into a folder that stands in for the installed copy (NUZKY_UPDATE_INSTALL) and is never started
again: the flow reloads the page where a release would restart."""
import gzip, json, os, re, subprocess, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from e2e.harness import FIXTURES, ROOT, Bridge, close_window, flow, wait, webdriver

ROW = """const b = document.querySelector('[data-version]');
return b && {text: b.textContent, focused: document.activeElement === b};"""
MENU = """const m = document.querySelector('[role=menu][aria-label=Updates]');
return m && [...m.querySelectorAll('[role^=menuitem]')].map((i) => ({label: i.textContent, checked: i.getAttribute('aria-checked')}));"""
FIRST_ITEM = """const i = document.querySelector('[role=menu][aria-label=Updates] [role^=menuitem]');
return i && {label: i.textContent, disabled: i.getAttribute('aria-disabled'), reason: i.title,
             active: i.parentElement.getAttribute('aria-activedescendant') === i.id};"""
CHOOSE = """const i = [...document.querySelectorAll('[role=menu][aria-label=Updates] [role^=menuitem]')]
  .find((i) => i.textContent.startsWith(arguments[0]));
if (!i) return false; i.click(); return true;"""
# A click without a pointer, as Enter or Space on the focused row sends it.
OPEN_MENU = "const b = document.querySelector('[data-version]'); b.focus(); b.click();"
OPEN_EDITOR_MENU = "const b = document.querySelector('[data-update-button]'); b.focus(); b.click();"
IN_MENU = "return !!document.activeElement?.closest('[role=menu][aria-label=Updates]')"
TOASTS = "return window.__nuzky.store.getState().toasts.map((t) => ({kind: t.kind, text: t.text}));"
CHOICES = """return window.__nuzky.store.getState().toasts.map((t) => ({kind: t.kind, text: t.text,
  choices: [t.secondary?.label, t.action?.label].filter(Boolean)}));"""
# A click focuses the button it lands on, as WebKitGTK does for a person's click.
TOAST_BUTTON = """const b = [...document.querySelectorAll('[data-toast] button')].find((b) => b.textContent === arguments[0]);
if (!b) return false; b.focus(); b.click(); return true;"""
PROGRESS = """const p = document.querySelector('[data-update-progress]');
return p && {text: p.textContent, stop: p.querySelector('button')?.textContent ?? null, width: p.getBoundingClientRect().width,
             stopFocused: document.activeElement === p.querySelector('button')};"""
INSTALL = 'return window.__nuzky.updates.getState().install'
RELOAD = "window.__oldPage = true; location.reload()"
EDITOR_BUTTON = """const b = document.querySelector('[data-update-button]');
return b && {label: b.getAttribute('aria-label') ?? b.innerText.trim(), focused: document.activeElement === b,
             disabled: b.getAttribute('aria-disabled') === 'true'};"""
# What the toasts show and the order Tab reaches their buttons in.
SHOWN = """return [...document.querySelectorAll('[data-toast]')].map((t) => ({text: t.querySelector('span').textContent,
  buttons: [...t.querySelectorAll('button')].map((b) => b.getAttribute('aria-label') ?? b.textContent)}));"""
NAME = 'return window.__nuzky.store.getState().snap.project.name'
RENAME = "window.__nuzky.store.getState().edit({type: 'renameProject', name: arguments[0]})"
VIEW = 'return window.__nuzky.store.getState().view'
DRAFT = 'return window.__nuzky.agent.getState().draft'
OPEN_DOWNLOAD = "Download from GitHub…"
LOADED = "return !window.__oldPage && !!window.__nuzky?.store.getState().snap"
RELEASE_PAGE = 'https://github.com/nuzkyapp/nuzky/releases/latest'
# The keys of the release pipeline's latest.json, and the one a build without a bundle looks for on Linux.
RELEASE_KEYS = ['darwin-aarch64-app', 'darwin-aarch64', 'windows-x86_64-nsis', 'windows-x86_64', 'linux-x86_64-appimage',
                'linux-x86_64-deb']
UNBUNDLED_KEY = 'linux-x86_64'
CHUNK = 64 * 1024


class Server:
    """latest.json as the release would serve it, counting the requests, and the package it names."""
    def __init__(self):
        self.status, self.body, self.requests, self.gzip, self.delay = 200, b'', 0, False, 0
        self.package, self.package_status, self.pace, self.sent, self.package_requests = b'', 200, 0, 0, 0
        server = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                if self.path == '/package':
                    return server.send_package(self)
                server.requests += 1
                time.sleep(server.delay)
                self.send_response(server.status)
                self.send_header('Content-Type', 'application/json')
                if server.gzip:
                    self.send_header('Content-Encoding', 'gzip')
                self.send_header('Content-Length', str(len(server.body)))
                self.end_headers()
                self.wfile.write(server.body)

            def log_message(self, *args):
                pass

        self.http = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        threading.Thread(target=self.http.serve_forever, daemon=True).start()
        self.root = f'http://127.0.0.1:{self.http.server_port}'
        self.url = self.root + '/latest.json'

    def serve(self, version, status=200, pad=0, packed=False, signature='', keys=(), url=None):
        """`packed`: gzip, so a large file travels in a few kilobytes. `keys`: the installs a package is offered for,
        at `url`, by default this server."""
        self.status, self.gzip = status, packed
        file = {'version': version, 'pub_date': '2026-10-09T00:00:00Z'}
        if keys:
            file['platforms'] = {key: {'url': url or self.root + '/package', 'signature': signature} for key in keys}
        if pad:
            file['notes'] = 'x' * pad
        body = json.dumps(file).encode() if status == 200 else b'Not Found'
        self.body = gzip.compress(body) if packed else body

    def send_package(self, handler):
        """In chunks `pace` seconds apart, so the download takes a moment; `sent` counts what the newest request got.
        A stopped download's request may still be writing when the next one starts."""
        self.package_requests += 1
        handler.send_response(self.package_status)
        handler.send_header('Content-Length', str(len(self.package) if self.package_status == 200 else 0))
        handler.end_headers()
        self.newest, self.sent = handler, 0
        try:
            for start in range(0, len(self.package) if self.package_status == 200 else 0, CHUNK):
                handler.wfile.write(self.package[start:start + CHUNK])
                handler.wfile.flush()
                if self.newest is handler:
                    self.sent += len(self.package[start:start + CHUNK])
                time.sleep(self.pace)
        except (BrokenPipeError, ConnectionResetError):
            pass


def sign(r, path, version):
    """Signs `path` as the release job does, with the flow's throwaway key; returns the signature for latest.json."""
    subprocess.run([str(ROOT / 'node_modules/.bin/tauri'), 'signer', 'sign', '-f', str(r.signing_key), '-p', '', '--app-version', version, str(path)],
                   check=True, capture_output=True)
    return path.with_name(path.name + '.sig').read_text()


def serve_releases(r):
    r.updates = Server()
    r.updates.serve('99.0.0')
    r.env.pop('NUZKY_NO_UPDATE_CHECK', None)
    r.env['NUZKY_UPDATE_URL'] = r.updates.url
    keys = r.work / 'update-key'
    keys.mkdir()
    r.signing_key = keys / 'test.key'
    subprocess.run([str(ROOT / 'node_modules/.bin/tauri'), 'signer', 'generate', '--ci', '-p', '', '-w', str(r.signing_key)], check=True, capture_output=True)
    r.env['NUZKY_UPDATE_PUBKEY'] = (keys / 'test.key.pub').read_text().strip()
    r.updates.package = os.urandom(3 * 1024 * 1024)
    (keys / 'package').write_bytes(r.updates.package)
    r.signature = sign(r, keys / 'package', '99.0.0')
    # A valid signature from the same key, of other bytes: what a swapped download would carry.
    (keys / 'other').write_bytes(r.updates.package[:-1] + b'x')
    r.other_signature = sign(r, keys / 'other', '99.0.0')
    r.opened = r.work / 'opened.txt'
    r.env['NUZKY_OPENED_URLS'] = str(r.opened)
    # Stands in for the installed copy; while it exists this copy can install.
    r.installed = r.work / 'installed-copy'
    r.installed.mkdir()
    r.env['NUZKY_UPDATE_INSTALL'] = str(r.installed)
    r.note = r.work / 'data/nuzky/update-note.json'


def row(r):
    return r.s.run(ROW) or {}


def toasts(r):
    return r.s.run(TOASTS)


def manual_check(r):
    """Check for updates from the row's menu; returns the toast it answered with. Earlier toasts go first, as
    an identical toast replaces the one before it."""
    r.s.run('window.__nuzky.store.setState({toasts: []})')
    r.s.run(OPEN_MENU)
    if not wait(lambda: r.s.run(MENU), 3) or not r.s.run(CHOOSE, 'Check for updates'):
        return None
    wait(lambda: toasts(r), 15)
    return (toasts(r) or [None])[-1]


@flow('updates', 'the home sidebar says when a newer version is out; checks run daily, quietly, and can be turned off',
      before=serve_releases, home=True)
def updates(r):
    server = r.updates
    try:
        version = r.s.run('return window.__nuzky.updates.getState().version')
        r.check('the sidebar foot names this version at start, before anything was asked',
                row(r).get('text') == f'Nuzky {version}' and server.requests == 0, (row(r), server.requests))

        r.check('about 10 s after start it finds the newer version and says so in the sidebar',
                wait(lambda: row(r).get('text') == 'Update available99.0.0', 20) and server.requests == 1, (row(r), server.requests))
        r.check('the automatic check shows no toast', toasts(r) == [], toasts(r))
        r.shot('update-available')

        r.s.run(OPEN_MENU)
        menu = wait(lambda: r.s.run(MENU), 3)
        r.check("its menu offers to install it and restart, what's new, a check and the automatic check, which is on",
                menu == [{'label': 'Install Nuzky 99.0.0 and restart', 'checked': None}, {'label': "What's new in Nuzky 99.0.0…", 'checked': None},
                         {'label': 'Check for updates', 'checked': None}, {'label': 'Check automatically', 'checked': 'true'}], menu)
        r.check('opened from the keyboard, the menu has the focus', wait(lambda: r.s.run(IN_MENU), 3))
        r.shot('update-menu')
        r.key('Escape')
        r.check('Esc closes the menu and focus returns to the row', wait(lambda: not r.s.run(MENU) and row(r).get('focused'), 3), row(r))

        webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
        if wait(lambda: r.s.run('return window.innerWidth') == 1024, 5):
            r.check('in a 1024 x 640 window the notice still sits at the foot of the sidebar, in full',
                    r.s.run("""const b = document.querySelector('[data-version]').getBoundingClientRect();
                               const nav = document.querySelector('nav[aria-label=Collections]').getBoundingClientRect();
                               return b.bottom <= window.innerHeight - 6 && b.top >= nav.bottom && b.width > 180;"""))
            r.shot('update-available-small')
        webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})

        r.s.run("window.__nuzky.store.setState({view: 'editor'})")
        r.check('in the editor a small button next to Projects says so',
                wait(lambda: r.s.run(EDITOR_BUTTON), 3) == {'label': 'Nuzky 99.0.0 is available', 'focused': False, 'disabled': False},
                r.s.run(EDITOR_BUTTON))
        r.shot('editor-update')
        r.s.run("const b = document.querySelector('[data-update-button]'); b.focus(); b.click();")
        menu = wait(lambda: r.s.run(MENU), 3)
        r.check('it opens the same menu, with the focus in it',
                (menu or [{}])[0].get('label') == 'Install Nuzky 99.0.0 and restart' and wait(lambda: r.s.run(IN_MENU), 3), menu)
        r.shot('editor-update-menu')
        r.key('Escape')
        r.check('Esc closes it and focus returns to the button', wait(lambda: not r.s.run(MENU) and (r.s.run(EDITOR_BUTTON) or {}).get('focused'), 3))
        r.s.run("window.__nuzky.store.setState({view: 'home'})")

        server.serve(version)
        answer = manual_check(r)
        r.check('Check for updates answers that this is the latest version, and the notice goes',
                answer == {'kind': 'success', 'text': f'Nuzky {version} is the latest version.'}
                and row(r).get('text') == f'Nuzky {version}' and server.requests == 2, (answer, row(r), server.requests))
        r.s.run("window.__nuzky.store.setState({view: 'editor'})")
        r.check('and the editor shows no button', wait(lambda: r.s.run("return !document.querySelector('[data-update-button]') && !!document.querySelector('header')"), 3))
        r.s.run("window.__nuzky.store.setState({view: 'home'})")

        server.serve(version, status=404)
        answer = manual_check(r)
        r.check('when the release cannot be read, Check for updates says so and what to try',
                answer == {'kind': 'error', 'text': "Couldn't check for updates. Check your internet connection or try again later."}
                and row(r).get('text') == f'Nuzky {version}', (answer, row(r)))

        server.serve('99.0.0', pad=70_000)
        answer = manual_check(r)
        r.check('a file larger than latest.json can be is not read, even when it names a newer version',
                (answer or {}).get('kind') == 'error' and row(r).get('text') == f'Nuzky {version}', (answer, row(r)))
        server.serve('99.0.0', pad=20_000_000, packed=True)
        answer = manual_check(r)
        r.check('nor is one that arrives small and unpacks large',
                len(server.body) < 65_536 and (answer or {}).get('kind') == 'error' and row(r).get('text') == f'Nuzky {version}',
                (len(server.body), answer, row(r)))
        r.shot('check-failed')

        server.serve('99.0.0')
        asked = server.requests
        r.s.run(RELOAD)
        wait(lambda: r.s.run(LOADED, retries=3), 30)
        time.sleep(13)
        r.check('after a restart within a day it does not ask again', server.requests == asked and row(r).get('text') == f'Nuzky {version}',
                (server.requests, asked, row(r)))

        r.s.run(OPEN_MENU)
        wait(lambda: r.s.run(IN_MENU), 3)
        r.key('End')
        r.key('Enter')
        r.check('chosen from the keyboard, Check automatically turns off and focus returns to the row',
                wait(lambda: not r.s.run(MENU) and row(r).get('focused'), 3)
                and r.s.run('return window.__nuzky.updates.getState().auto') is False, row(r))
        r.s.run(OPEN_MENU)
        menu = wait(lambda: r.s.run(MENU), 3)
        r.check('the menu shows it off', (menu or [{}])[-1].get('checked') == 'false', menu)
        r.key('Escape')
        # A day passes.
        r.s.run("const s = JSON.parse(localStorage.getItem('nuzky.updates')); s.checkedAt = Date.now() - 25 * 3600e3;"
                "localStorage.setItem('nuzky.updates', JSON.stringify(s));" + RELOAD)
        wait(lambda: r.s.run(LOADED, retries=3), 30)
        time.sleep(13)
        r.check('turned off, it does not ask even a day later', server.requests == asked, (server.requests, asked))

        r.s.run(OPEN_MENU)
        wait(lambda: r.s.run(MENU), 3)
        r.s.run(CHOOSE, 'Check automatically')
        r.check('turned on again a day after the last check, it asks at once and finds the newer version',
                wait(lambda: row(r).get('text') == 'Update available99.0.0', 15) and server.requests == asked + 1 and toasts(r) == [],
                (row(r), server.requests, toasts(r)))
        # Another day passes, on a slow connection; Check for updates is chosen while the automatic check runs.
        asked, server.delay = server.requests, 3
        r.s.run("const s = JSON.parse(localStorage.getItem('nuzky.updates')); s.checkedAt = Date.now() - 25 * 3600e3;"
                "localStorage.setItem('nuzky.updates', JSON.stringify(s));" + RELOAD)
        wait(lambda: r.s.run(LOADED, retries=3), 30)
        wait(lambda: server.requests == asked + 1, 15)
        r.s.call('window.__nuzky.checkForUpdates(true)')
        r.check('Check for updates during the automatic check answers once it is done, without asking twice',
                wait(lambda: toasts(r), 10) and toasts(r)[-1] == {'kind': 'info', 'text': 'Nuzky 99.0.0 is available.'}
                and server.requests == asked + 1, (toasts(r), server.requests, asked))
        server.delay = 0
        r.check('nothing went wrong on the way', [t for t in toasts(r) if t['kind'] == 'error'] == [], toasts(r))
        r.check('its toast offers to install it', (r.s.run(CHOICES) or [{}])[-1].get('choices') == ['Install and restart'], r.s.run(CHOICES))
        install(r, server, version)
    finally:
        server.http.shutdown()


def opened(r):
    return r.opened.read_text().splitlines() if r.opened.exists() else []


def choices_after(r, start):
    """Runs `start` with no toasts shown and returns the toast it ended with, with its choices."""
    r.s.run('window.__nuzky.store.setState({toasts: []})')
    start()
    wait(lambda: r.s.run(CHOICES), 30)
    return (r.s.run(CHOICES) or [None])[-1]


def from_row(r, item):
    """Chooses the menu item starting with `item` from the sidebar foot's menu."""
    r.s.run(OPEN_MENU)
    wait(lambda: r.s.run(MENU), 3)
    return r.s.run(CHOOSE, item)


def offered(r):
    """Check for updates, as the user would after a failure that changed what is offered."""
    r.s.run(OPEN_MENU if r.s.run(VIEW) == 'home' else OPEN_EDITOR_MENU)
    wait(lambda: r.s.run(MENU), 3)
    return choices_after(r, lambda: r.s.run(CHOOSE, 'Check for updates'))


def editor(r):
    r.s.run("window.__nuzky.store.setState({view: 'editor'})")
    wait(lambda: r.s.run('return !!document.querySelector("header")'), 5)


def home(r):
    r.s.run("window.__nuzky.store.setState({view: 'home'})")
    wait(lambda: r.s.run('return !!document.querySelector("[data-version]")'), 5)


def start_export(r):
    started = r.s.call('window.__nuzky.api.startExport(arguments[0], arguments[1], arguments[2], arguments[3])',
                       str(r.work / 'long.mp4'), {'resolution': 720, 'fps': 30, 'quality': 'small'}, r.state()['epoch'], True)
    wait(lambda: r.s.run("return window.__nuzky.store.getState().jobs[arguments[0]]?.status", started.get('value')) == 'running', 10)
    return started['value']


def stop_export(r, job):
    r.s.call('window.__nuzky.api.cancelJob(arguments[0])', job)
    wait(lambda: r.s.run("return window.__nuzky.store.getState().jobs[arguments[0]]?.status", job) != 'running', 15)


def ready_toast(r):
    return next((t for t in r.s.run(SHOWN) or [] if t['text'].startswith('Nuzky 99.0.0 is ready.')), None)


def install(r, server, version):
    """The release now carries signed packages, as the release job makes them."""
    keys = RELEASE_KEYS + [UNBUNDLED_KEY]
    server.serve('99.0.0', signature=r.signature, keys=keys)

    # A copy that cannot replace itself, such as a build from source, gets the installers on GitHub as before.
    r.installed.rmdir()
    answer = offered(r)
    r.s.run(OPEN_MENU)
    menu = wait(lambda: r.s.run(MENU), 3)
    r.check('a copy that cannot install itself is offered the download from GitHub, in the toast and first in the menu',
            answer == {'kind': 'info', 'text': 'Nuzky 99.0.0 is available.', 'choices': [OPEN_DOWNLOAD]}
            and (menu or [{}])[0].get('label') == 'Download Nuzky 99.0.0…', (answer, menu))
    r.shot('cannot-install')
    r.s.run(CHOOSE, 'Download Nuzky')
    r.check('which opens the release page and downloads nothing', wait(lambda: opened(r) == [RELEASE_PAGE], 5) and server.package_requests == 0,
            (opened(r), server.package_requests))
    r.installed.mkdir()
    answer = offered(r)
    r.check('a copy that can install itself is offered Install and restart', (answer or {}).get('choices') == ['Install and restart'], answer)

    # latest.json can name any address; only GitHub's release downloads are fetched (the debug build also takes 127.0.0.1).
    server.serve('99.0.0', signature=r.signature, keys=keys, url=f'http://localhost:{server.http.server_port}/package')
    answer = choices_after(r, lambda: from_row(r, 'Install'))
    r.check('a package that is not on the release downloads is refused before anything downloads, and GitHub is offered',
            answer == {'kind': 'error', 'text': "The update isn't on Nuzky's GitHub releases, so it wasn't downloaded. Download the new version from GitHub instead.",
                       'choices': [OPEN_DOWNLOAD]} and server.package_requests == 0, (answer, server.package_requests))
    r.s.run(OPEN_MENU)
    r.check('and the menu then offers the download from GitHub first', ((wait(lambda: r.s.run(MENU), 3) or [{}])[0]).get('label') == 'Download Nuzky 99.0.0…')
    r.key('Escape')

    server.serve('99.0.0', signature=r.other_signature, keys=keys)
    offered(r)
    answer = choices_after(r, lambda: from_row(r, 'Install'))
    r.check("a download whose signature doesn't match is not installed; the toast says so and offers GitHub",
            answer == {'kind': 'error', 'text': "The download isn't signed by Nuzky, so it wasn't installed. Download the new version from GitHub instead.",
                       'choices': [OPEN_DOWNLOAD]} and row(r).get('text') == 'Update available99.0.0', (answer, row(r)))
    r.shot('update-bad-signature')
    r.s.run(TOAST_BUTTON, OPEN_DOWNLOAD)
    r.check('its Download from GitHub… opens the release page', wait(lambda: opened(r) == [RELEASE_PAGE] * 2, 5), opened(r))
    r.s.call(RENAME, 'After the update failed')
    r.check('and the app keeps working', wait(lambda: r.s.run(NAME) == 'After the update failed', 5), r.s.run(NAME))

    # The download from the editor, with its Stop.
    server.serve('99.0.0', signature=r.signature, keys=keys)
    offered(r)
    editor(r)
    r.import_media(FIXTURES / 'filmstrip-long.mp4')
    r.add_clip('filmstrip-long.mp4')
    server.pace = 0.08
    r.s.run(OPEN_EDITOR_MENU)
    wait(lambda: r.s.run(IN_MENU), 3)
    r.key('Enter')
    progress = wait(lambda: (p := r.s.run(PROGRESS)) and re.search(r'\d%', p['text']) and p, 10)
    r.check('Install and restart from the keyboard downloads in the background, with its percentage and Stop, which has the focus',
            progress and re.fullmatch(r'Downloading update\d{1,2}%Stop', progress['text']) and progress['stopFocused'], progress)
    r.shot('update-downloading-editor')
    later = wait(lambda: (p := r.s.run(PROGRESS)) and p['text'] != progress['text'] and p, 5)
    r.check('the capsule keeps its width while the percentage grows', later and progress and later['width'] == progress['width'], (progress, later))
    r.s.run('document.activeElement.click()')
    stopped = wait(lambda: r.s.run(INSTALL) is None and (b := r.s.run(EDITOR_BUTTON)) and b['focused'] and b, 5)
    r.check('Stop ends the download at once, without an error; the update stays offered and its button has the focus',
            stopped and stopped['focused'] and 0 < server.sent < len(server.package) and not r.errors(), (stopped, server.sent, r.errors()))

    home(r)
    r.s.run('window.__nuzky.store.setState({toasts: []})')
    server.pace = 0.04
    from_row(r, 'Install')
    downloading = wait(lambda: re.fullmatch(r'Downloading\d{1,2}%', row(r).get('text', '')), 10)
    r.check('on the home screen the sidebar foot shows the download and its percentage', downloading, row(r))
    r.s.run(OPEN_MENU)
    menu = wait(lambda: r.s.run(MENU), 3) or []
    check_item = next((i for i in menu if i['label'] == 'Check for updates'), None)
    reason = r.s.run("""return [...document.querySelectorAll('[role=menu][aria-label=Updates] [role^=menuitem]')]
                        .find((i) => i.textContent === 'Check for updates')?.title""")
    r.check('while it downloads the menu offers Stop downloading, and Check for updates waits and says why',
            menu[:1] == [{'label': 'Stop downloading', 'checked': None}] and check_item and reason == 'The update is downloading.', (menu, reason))
    r.key('Escape')
    # The menu fades out and gives the focus back to the row.
    time.sleep(0.4)
    r.s.run("document.querySelector('[data-version]').focus()")
    stop = r.s.run("""const b = document.querySelector('[data-version]').parentElement.querySelector('button:not([data-version])');
                      return b && {text: b.textContent, opacity: getComputedStyle(b).opacity}""")
    r.check('with the row focused its Stop shows', stop and stop['text'] == 'Stop' and stop['opacity'] == '1', stop)
    r.shot('update-downloading')
    r.s.run("document.querySelector('[data-version]').parentElement.querySelector('button:not([data-version])').click()")
    r.check('and stops the download', wait(lambda: r.s.run(INSTALL) is None and row(r).get('text') == 'Update available99.0.0', 5), row(r))

    # A download that fails, and Try again while an agent edits: the package is kept for when the AI is done.
    server.pace = 0
    server.package_status = 404
    answer = choices_after(r, lambda: from_row(r, 'Install'))
    r.check('a download that fails offers Download from GitHub… and Try again',
            answer == {'kind': 'error', 'text': "Couldn't download the update. Check your internet connection and try again.",
                       'choices': [OPEN_DOWNLOAD, 'Try again']}, answer)
    shown = (r.s.run(SHOWN) or [{}])[-1]
    r.check('Tab reaches its actions before Dismiss', shown.get('buttons') == [OPEN_DOWNLOAD, 'Try again', 'Dismiss'], shown)
    r.shot('update-download-failed')
    server.package_status = 200
    asked = server.package_requests
    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Tighten the intro'})
        wait(lambda: r.state()['aiRun'], 10)
        r.s.run(TOAST_BUTTON, 'Try again')
        r.check('Try again downloads it while an agent edits', wait(lambda: (r.s.run(INSTALL) or {}).get('stage') == 'ready', 30)
                and server.package_requests == asked + 1 and server.sent == len(server.package), (r.s.run(INSTALL), server.package_requests))
        time.sleep(1)
        r.check('with work running nothing counts down to a restart', ready_toast(r) is None, r.s.run(SHOWN))
        r.check('the sidebar foot says it restarts when the AI edit finishes', row(r).get('text') == 'Restart when an AI edit finishes', row(r))
        r.s.run(OPEN_MENU)
        item = wait(lambda: r.s.run(FIRST_ITEM), 3)
        r.check('its menu says so in the item, which is disabled, and the keyboard starts on the next one',
                item == {'label': 'Restart when an AI edit finishes', 'disabled': 'true', 'reason': 'Restart when an AI edit finishes.', 'active': False}, item)
        time.sleep(0.5)
        r.shot('restart-waits-home')
        r.key('Escape')
        editor(r)
        button = r.s.run(EDITOR_BUTTON)
        r.check('in the editor the top bar says the same, disabled',
                button and button['label'].strip() == 'Restart when an AI edit finishes' and button['disabled'], button)
        r.shot('restart-waits-editor')
        webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
        if wait(lambda: r.s.run('return window.innerWidth') == 1024, 5):
            r.shot('restart-waits-editor-small')
        webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
        wait(lambda: r.s.run('return window.innerWidth') == 1440, 5)
        bridge.call('end_run', {'run_id': run['run_id'], 'action': 'keep'})
        wait(lambda: not r.state()['aiRun'], 10)
    finally:
        bridge.close()
    button = wait(lambda: (b := r.s.run(EDITOR_BUTTON)) and not b['disabled'] and b, 5)
    r.check('once the AI is done it offers Restart to update', button and button['label'].strip() == 'Restart to update', button)
    r.shot('restart-to-update')
    time.sleep(6)
    r.check('and never restarts by itself', not (r.installed / 'installed').exists() and (r.s.run(INSTALL) or {}).get('stage') == 'ready', r.s.run(INSTALL))

    export = start_export(r)
    button = wait(lambda: (b := r.s.run(EDITOR_BUTTON)) and b['disabled'] and b, 5)
    r.check('during an export it says it restarts when the export finishes', button and button['label'].strip() == 'Restart when the export finishes', button)
    r.s.run("document.querySelector('[data-update-button]').click()")
    time.sleep(0.5)
    r.check('and a click does nothing', not (r.installed / 'installed').exists() and (r.s.run(INSTALL) or {}).get('stage') == 'ready', r.s.run(INSTALL))
    stop_export(r, export)

    # Windows starts its installer after the hook saved the project; when Windows refuses to start it, Nuzky goes on.
    (r.installed / 'refuse').touch()
    asked = server.package_requests
    r.s.call(RENAME, 'Before the installer')
    answer = choices_after(r, lambda: r.s.run("document.querySelector('[data-update-button]').click()"))
    r.check('when the installer cannot start, the toast says to try again, with GitHub beside it',
            answer == {'kind': 'error', 'text': "Couldn't install the update. Try again.", 'choices': [OPEN_DOWNLOAD, 'Try again']}, answer)
    saved = json.loads(r.saved_project().read_text())['name']
    r.s.call(RENAME, 'After the installer failed')
    r.check('the project was saved before it, and Nuzky goes on editing and saving',
            saved == 'Before the installer' and wait(lambda: json.loads(r.saved_project().read_text())['name'] == 'After the installer failed', 5)
            and r.s.run(VIEW) == 'editor', (saved, r.s.run(NAME)))
    (r.installed / 'refuse').unlink()

    # A failure trying again cannot fix drops the package; the next one downloads anew, here with nothing running.
    r.installed.rmdir()
    r.s.run(TOAST_BUTTON, 'Try again')
    answer = wait(lambda: (t := r.s.run(CHOICES)) and t[-1]['text'].startswith('Nuzky can') and t[-1], 10)
    r.check('Try again uses the package it has without downloading it again; a failure trying again cannot fix drops it',
            server.package_requests == asked and (answer or {}).get('text') == "Nuzky can't update this copy by itself. Download the new version from GitHub.",
            (server.package_requests, answer))
    r.installed.mkdir()
    offered(r)
    r.s.run('window.__nuzky.store.setState({toasts: []})')
    r.s.run(OPEN_EDITOR_MENU)
    wait(lambda: r.s.run(MENU), 3)
    r.s.run(CHOOSE, 'Install')
    toast = wait(lambda: ready_toast(r), 30)
    r.check('downloaded with nothing running, a toast counts down to the restart, with Not now and Restart now',
            toast == {'text': 'Nuzky 99.0.0 is ready. Restarting in 5 s.', 'buttons': ['Not now', 'Restart now', 'Dismiss']}, toast)
    r.shot('ready-countdown')
    later = wait(lambda: (t := ready_toast(r)) and t['text'] != toast['text'] and t, 3)
    r.check('and counts down', later and later['text'] == 'Nuzky 99.0.0 is ready. Restarting in 4 s.', later)
    r.s.run(TOAST_BUTTON, 'Not now')
    button = wait(lambda: (b := r.s.run(EDITOR_BUTTON)) and b['focused'] and b, 3) or r.s.run(EDITOR_BUTTON)
    r.check('Not now leaves Restart to update in the top bar, with the focus', button and button['label'].strip() == 'Restart to update' and button['focused'], button)
    time.sleep(6)
    r.check('and nothing restarts by itself', not (r.installed / 'installed').exists() and ready_toast(r) is None, r.s.run(SHOWN))

    # Once more, letting the countdown run out; an export starts while it installs.
    r.installed.rmdir()
    r.s.run("document.querySelector('[data-update-button]').click()")
    wait(lambda: (r.s.run(CHOICES) or [{}])[-1].get('kind') == 'error', 10)
    r.installed.mkdir()
    offered(r)
    (r.installed / 'hold').touch()
    r.s.run('window.__nuzky.store.setState({toasts: []})')
    asked = server.package_requests
    r.s.run(OPEN_EDITOR_MENU)
    wait(lambda: r.s.run(MENU), 3)
    r.s.run(CHOOSE, 'Install')
    wait(lambda: ready_toast(r), 30)
    progress = wait(lambda: (p := r.s.run(PROGRESS)) and p['text'] == 'Installing update…' and p, 10)
    r.check('when the countdown runs out it installs, saying so in place of the button, with nothing to choose',
            progress and progress['stop'] is None and server.package_requests == asked + 1, progress)
    r.shot('installing')
    close_window()
    time.sleep(1)
    r.check('closing the window meanwhile neither quits nor leaves the editor', r.s.run(VIEW, retries=3) == 'editor')
    export = start_export(r)
    (r.installed / 'hold').unlink()
    toast = wait(lambda: next((t for t in r.s.run(SHOWN) or [] if t['text'].startswith('Nuzky 99.0.0 is installed')), None), 10)
    r.check('work that started while it installed keeps Nuzky running, and the toast says when to restart',
            toast == {'text': 'Nuzky 99.0.0 is installed. Restart when the export finishes.', 'buttons': ['Restart', 'Dismiss']}
            and (r.installed / 'installed').read_bytes() == server.package and not r.note.exists(), toast)
    r.shot('installed-waits')
    button = r.s.run(EDITOR_BUTTON)
    r.check('the top bar waits for the export too', button and button['label'].strip() == 'Restart when the export finishes' and button['disabled'], button)
    stop_export(r, export)

    # Restart to update, with the AI panel's draft and the editor to come back to.
    r.s.run("window.__nuzky.agent.setState({draft: 'Cut the long pauses'})")
    r.s.call(RENAME, 'Survives the update')
    button = wait(lambda: (b := r.s.run(EDITOR_BUTTON)) and not b['disabled'] and b, 5)
    r.s.run("document.querySelector('[data-update-button]').click()")
    note = wait(lambda: r.note.exists() and json.loads(r.note.read_text()), 10)
    r.check('Restart to update saves the project and leaves the restarted Nuzky where to go on, without installing again',
            button and note == {'from': version, 'view': 'editor', 'draft': 'Cut the long pauses'}
            and json.loads(r.saved_project().read_text())['name'] == 'Survives the update' and server.package_requests == asked + 1, (button, note))
    # Started as the newer version: this debug build stays the same, so the note and the last check say they came from
    # an older one.
    r.note.write_text(json.dumps(dict(note, **{'from': '0.0.9'})))
    r.s.run("const s = JSON.parse(localStorage.getItem('nuzky.updates')); s.by = '0.0.9';"
            "localStorage.setItem('nuzky.updates', JSON.stringify(s)); window.__nuzky.agent.setState({draft: ''});" + RELOAD)
    wait(lambda: r.s.run(LOADED, retries=3), 30)
    updated = {'kind': 'success', 'text': f'Updated to Nuzky {version}.'}
    r.check('the restarted Nuzky says it updated and opens on the editor with the project and the draft, offering no update',
            wait(lambda: updated in (toasts(r) or []), 5) and r.s.run(VIEW) == 'editor' and r.s.run(NAME) == 'Survives the update'
            and r.s.run(DRAFT) == 'Cut the long pauses' and not r.note.exists() and not r.s.run(EDITOR_BUTTON),
            (toasts(r), r.s.run(VIEW), r.s.run(DRAFT), r.s.run(EDITOR_BUTTON)))
    wait(lambda: r.s.run("return !!document.querySelector('header')"), 10)
    time.sleep(0.5)
    r.shot('updated')
    r.s.run(RELOAD)
    wait(lambda: r.s.run(LOADED, retries=3), 30)
    time.sleep(1)
    r.check('only once', updated not in toasts(r) and r.s.run(VIEW) == 'home', (toasts(r), r.s.run(VIEW)))
    r.check('no other error on the way', not [t for t in toasts(r) if t['kind'] == 'error'], toasts(r))
    toast_layout(r)


def toast_layout(r):
    """Toasts with a short action keep it beside the text, as before; Tab reaches actions before Dismiss."""
    for width, height in ((1440, 900), (1024, 640)):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        editor(r)
        r.s.run("""const st = window.__nuzky.store.getState(); window.__nuzky.store.setState({toasts: []});
                   st.toast({kind: 'error', text: 'Export failed: The disk is full. Free some space and export again.', action: {label: 'Details', run: () => {}}});
                   st.toast({kind: 'info', text: 'Nuzky 99.0.0 is ready. Restarting in 5 s.', action: {label: 'Restart now', run: () => {}},
                             secondary: {label: 'Not now', run: () => {}}, sticky: true});""")
        time.sleep(0.5)
        place = r.s.run("""const t = document.querySelector('[data-toast]'), text = t.querySelector('span').getBoundingClientRect();
                           const b = [...t.querySelectorAll('button')].find((b) => b.textContent === 'Details').getBoundingClientRect();
                           return {beside: b.left >= text.right && b.top < text.bottom, buttons: [...t.querySelectorAll('button')].map((b) => b.getAttribute('aria-label') ?? b.textContent)}""")
        r.check(f'at {width} x {height} a failed export keeps Details beside its text, then Dismiss',
                place == {'beside': True, 'buttons': ['Details', 'Dismiss']}, place)
        r.shot(f'toasts-{width}')
    webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})

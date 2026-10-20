"""Update checks and installing. The foot of the home sidebar says when a newer Nuzky was released, from a latest.json
served here in place of GitHub's. The automatic check runs 10 s after start and at most once a day, also across launches,
says nothing when it fails and can be turned off; Check for updates always answers in a toast. Install and restart
downloads the package latest.json names, signed here with a throwaway key, with its progress and a Stop; this debug
build checks the signature and then refuses to replace itself, as a build from source does."""
import gzip, json, os, re, subprocess, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from e2e.harness import FIXTURES, ROOT, Bridge, flow, wait, webdriver

ROW = """const b = document.querySelector('[data-version]');
return b && {text: b.textContent, focused: document.activeElement === b};"""
MENU = """const m = document.querySelector('[role=menu][aria-label=Updates]');
return m && [...m.querySelectorAll('[role^=menuitem]')].map((i) => ({label: i.textContent, checked: i.getAttribute('aria-checked')}));"""
INSTALL_ITEM = """const i = [...document.querySelectorAll('[role=menu][aria-label=Updates] [role^=menuitem]')]
  .find((i) => i.textContent.startsWith('Install'));
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
TOAST_BUTTON = """const b = [...document.querySelectorAll('[data-toast] button')].find((b) => b.textContent === arguments[0]);
if (!b) return false; b.click(); return true;"""
PROGRESS = """const p = document.querySelector('[data-update-progress]');
return p && {text: p.textContent, stop: p.querySelector('button')?.textContent ?? null,
             stopFocused: document.activeElement === p.querySelector('button')};"""
INSTALL = 'return window.__nuzky.updates.getState().install'
RELOAD = "window.__oldPage = true; location.reload()"
EDITOR_BUTTON = """const b = document.querySelector('[data-update-button]');
return b && {label: b.getAttribute('aria-label'), focused: document.activeElement === b};"""
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
        self.package, self.package_status, self.pace, self.sent = b'', 200, 0, 0
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

    def serve(self, version, status=200, pad=0, packed=False, signature='', keys=()):
        """`packed`: gzip, so a large file travels in a few kilobytes. `keys`: the installs a package is offered for."""
        self.status, self.gzip = status, packed
        file = {'version': version, 'pub_date': '2026-10-09T00:00:00Z'}
        if keys:
            file['platforms'] = {key: {'url': self.root + '/package', 'signature': signature} for key in keys}
        if pad:
            file['notes'] = 'x' * pad
        body = json.dumps(file).encode() if status == 200 else b'Not Found'
        self.body = gzip.compress(body) if packed else body

    def send_package(self, handler):
        """In chunks `pace` seconds apart, so the download takes a moment; `sent` counts what the newest request got.
        A stopped download's request may still be writing when the next one starts."""
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
                wait(lambda: r.s.run(EDITOR_BUTTON), 3) == {'label': 'Nuzky 99.0.0 is available', 'focused': False}, r.s.run(EDITOR_BUTTON))
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


def install_from_row(r):
    """Install and restart from the sidebar foot's menu; returns the toast it ended with."""
    r.s.run('window.__nuzky.store.setState({toasts: []})')
    r.s.run(OPEN_MENU)
    wait(lambda: r.s.run(MENU), 3)
    r.s.run(CHOOSE, 'Install')
    wait(lambda: r.s.run(CHOICES), 30)
    return (r.s.run(CHOICES) or [None])[-1]


def install(r, server, version):
    """The release now carries signed packages, as the release job makes them."""
    keys = RELEASE_KEYS + [UNBUNDLED_KEY]
    server.serve('99.0.0', signature=r.signature, keys=keys)
    r.s.run('window.__nuzky.store.setState({toasts: []})')

    # Updating restarts Nuzky, which would cancel an export.
    r.s.run("window.__nuzky.store.setState({view: 'editor'})")
    r.import_media(FIXTURES / 'filmstrip-long.mp4')
    r.add_clip('filmstrip-long.mp4')
    export = r.s.call('window.__nuzky.api.startExport(arguments[0], arguments[1], arguments[2], arguments[3])',
                      str(r.work / 'long.mp4'), {'resolution': 720, 'fps': 30, 'quality': 'small'}, r.state()['epoch'], True)
    job = "return window.__nuzky.store.getState().jobs[arguments[0]]?.status"
    wait(lambda: r.s.run(job, export.get('value')) == 'running', 10)
    r.s.run(OPEN_EDITOR_MENU)
    item = wait(lambda: r.s.run(INSTALL_ITEM), 3)
    r.check('while an export runs, Install is disabled and says why',
            item and item['disabled'] == 'true' and item['reason'] == 'Updating restarts Nuzky. Wait for the export to finish.', item)
    # The webview paints the menu a moment after it opens.
    time.sleep(0.5)
    r.shot('install-refused-export')
    r.key('Enter')
    time.sleep(1)
    r.check('chosen from the keyboard anyway, it downloads nothing', item and item['active'] and server.sent == 0 and r.s.run(INSTALL) is None,
            (item, server.sent, r.s.run(INSTALL)))
    r.key('Escape')
    r.s.call('window.__nuzky.api.cancelJob(arguments[0])', export['value'])
    wait(lambda: r.s.run(job, export['value']) != 'running', 15)

    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Tighten the intro'})
        wait(lambda: r.state()['aiRun'], 10)
        r.s.run(OPEN_EDITOR_MENU)
        item = wait(lambda: r.s.run(INSTALL_ITEM), 3)
        r.check('while an AI agent edits, Install is disabled and says why',
                item and item['disabled'] == 'true' and item['reason'] == 'Updating restarts Nuzky. Wait for the AI to finish.', item)
        r.key('Escape')
        bridge.call('end_run', {'run_id': run['run_id'], 'action': 'keep'})
        wait(lambda: not r.state()['aiRun'], 10)
    finally:
        bridge.close()

    server.pace = 0.08
    r.s.run(OPEN_EDITOR_MENU)
    wait(lambda: r.s.run(IN_MENU), 3)
    r.key('Enter')
    progress = wait(lambda: (p := r.s.run(PROGRESS)) and '%' in p['text'] and p, 10)
    r.check('Install and restart from the keyboard downloads in the background, with its percentage and Stop, which has the focus',
            progress and re.fullmatch(r'Downloading update\d{1,2}%Stop', progress['text']) and progress['stopFocused'], progress)
    r.shot('update-downloading-editor')
    r.s.run('document.activeElement.click()')
    stopped = wait(lambda: r.s.run(INSTALL) is None and r.s.run(EDITOR_BUTTON), 5)
    r.check('Stop ends the download at once, without an error, and the update stays offered',
            stopped and 0 < server.sent < len(server.package) and not r.errors(), (server.sent, r.errors()))

    r.s.run("window.__nuzky.store.setState({view: 'home', toasts: []})")
    server.pace = 0.04
    r.s.run(OPEN_MENU)
    wait(lambda: r.s.run(MENU), 3)
    r.s.run(CHOOSE, 'Install')
    downloading = wait(lambda: re.fullmatch(r'Downloading\d{1,2}%', row(r).get('text', '')), 10)
    r.check('on the home screen the sidebar foot shows the download and its percentage', downloading, row(r))
    r.shot('update-downloading')
    answer = wait(lambda: r.s.run(CHOICES), 30) and r.s.run(CHOICES)[-1]
    r.check('a package with the right signature downloads whole and passes the check; this debug build then says it cannot '
            'replace itself and offers GitHub',
            server.sent == len(server.package) and row(r).get('text') == 'Update available99.0.0'
            and answer == {'kind': 'error', 'text': "Nuzky can't update this copy by itself. Download the new version from GitHub.",
                           'choices': ['Download']}, (server.sent, answer, row(r)))

    server.pace = 0
    server.serve('99.0.0', signature=r.other_signature, keys=keys)
    answer = install_from_row(r)
    r.check("a download whose signature doesn't match is not installed; the toast says so and offers GitHub",
            answer == {'kind': 'error', 'text': "The download isn't signed by Nuzky, so it wasn't installed. Download the new version from GitHub instead.",
                       'choices': ['Download']} and row(r).get('text') == 'Update available99.0.0', (answer, row(r)))
    r.shot('update-bad-signature')
    r.s.run(TOAST_BUTTON, 'Download')
    r.check("its Download opens the release page", wait(lambda: opened(r) == [RELEASE_PAGE], 5), opened(r))
    r.s.call("window.__nuzky.store.getState().edit({type: 'renameProject', name: 'After the update failed'})")
    name = 'return window.__nuzky.store.getState().snap.project.name'
    r.check('and the app keeps working', wait(lambda: r.s.run(name) == 'After the update failed', 5), r.s.run(name))

    server.serve('99.0.0', signature=r.signature, keys=keys)
    server.package_status = 404
    answer = install_from_row(r)
    r.check('a download that fails offers Try again and GitHub',
            answer == {'kind': 'error', 'text': "Couldn't download the update. Check your internet connection and try again.",
                       'choices': ['Download', 'Try again']}, answer)
    r.shot('update-download-failed')
    server.package_status = 200
    r.s.run(TOAST_BUTTON, 'Try again')
    answer = wait(lambda: (t := r.s.run(CHOICES)) and t[-1]['text'].startswith('Nuzky can') and t[-1], 30)
    r.check('Try again downloads it again', answer and server.sent == len(server.package), (answer, server.sent))

    server.serve('99.0.0', signature=r.signature, keys=RELEASE_KEYS)
    answer = install_from_row(r)
    r.check('a release with no package for this kind of install says to download it from GitHub',
            answer == {'kind': 'error', 'text': "Nuzky can't update this copy by itself. Download the new version from GitHub.",
                       'choices': ['Download']}, answer)

    # The debug build cannot restart into a new version (the release AppImage was checked by hand); what Install
    # leaves for the restarted Nuzky is set here.
    r.s.run("localStorage.setItem('nuzky.updates.installing', arguments[0]);" + RELOAD, version)
    wait(lambda: r.s.run(LOADED, retries=3), 30)
    updated = {'kind': 'success', 'text': f'Updated to Nuzky {version}.'}
    r.check('started as the version it installed, Nuzky says so', wait(lambda: updated in (toasts(r) or []), 5), toasts(r))
    r.shot('updated')
    r.s.run(RELOAD)
    wait(lambda: r.s.run(LOADED, retries=3), 30)
    time.sleep(1)
    r.check('only once', updated not in toasts(r), toasts(r))
    r.check('no other error on the way', not [t for t in toasts(r) if t['kind'] == 'error'], toasts(r))

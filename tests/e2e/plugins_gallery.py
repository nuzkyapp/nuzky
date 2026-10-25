"""The plugin gallery, from a local server standing in for github.com/nuzkyapp/nuzky-plugins (NUZKY_GALLERY_URL, read
only by debug builds). The server logs every request, so the flow sees that Nuzky asks for the list and the chosen
plugin's files and nothing else."""
import hashlib, io, json, shutil, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from e2e.harness import FIXTURES, ROOT, flow, press, wait, webdriver

COMMIT = 'c0ffee' * 6 + 'c0ff'
EXAMPLE = ROOT / 'skills/nuzky-plugin/examples/remove-short-clips'
LOOK = ROOT / 'skills/nuzky-plugin/examples/vhs'
# A top level that does not even parse: installing must not run it.
BROKEN_SCRIPT = b'function run(ctx) { return [ ;'
DESCRIPTION = 'Flashes my logo on every beat. The best plugin ever, it can do anything!'
REFUSED = 'Nuzky won’t install this plugin. It includes files plugins aren’t allowed to have.'
SHARED = 'Shared by other people. Nuzky’s maintainer checks each one before it’s listed.'
TAB = "document.querySelector('[role=tablist][aria-label=Plugins] [data-tab=\"' + arguments[0] + '\"]').click()"
ROW = "return !!document.querySelector(`[data-gallery-plugin=\"${arguments[0]}\"]`)"
MARK = "return document.querySelector(`[data-gallery-plugin=\"${arguments[0]}\"]`)?.lastElementChild?.textContent ?? null"
OPEN = "document.querySelector(`[data-gallery-plugin=\"${arguments[0]}\"]`).click()"
DETAIL = """const d = document.querySelector(`[data-gallery-detail="${arguments[0]}"]`);
return d && {summary: [...d.querySelectorAll('[data-gallery-summary] li')].map((l) => l.textContent),
  new: [...d.querySelectorAll('[data-gallery-summary] li[data-new]')].map((l) => l.textContent),
  files: [...d.querySelectorAll('[data-gallery-files] li')].map((l) => l.innerText.replace(/\\s+/g, ' ')),
  refused: [...d.querySelectorAll('[data-gallery-files] li[data-refused]')].map((l) => l.firstElementChild.textContent),
  author: d.querySelector('[data-gallery-author]')?.textContent ?? null,
  buttons: [...d.querySelectorAll('button')].map((b) => ({text: b.textContent, off: b.getAttribute('aria-disabled') === 'true'})),
  sections: [...d.children].map((s) => s.querySelector('h3, h2')?.textContent ?? (s.querySelector('button, p') ? 'action' : '')),
  text: d.innerText};"""
CLICK = """const b = [...document.querySelectorAll(`[data-gallery-detail] button`)].find((b) => b.textContent === arguments[0]);
if (!b) return false; b.click(); return true;"""
FOCUS = """const b = [...document.querySelectorAll(`[data-gallery-detail] button`)].find((b) => b.textContent === arguments[0]);
if (!b) return false; b.focus(); return document.activeElement === b;"""
FOCUSED = "return document.activeElement?.closest('[data-gallery-detail]') ? document.activeElement.textContent : null"
PROBLEM = "return document.querySelector('#plugins-panel-gallery [role=alert], [data-gallery-detail] [role=alert]')?.innerText ?? null"
SHARED_LINE = "return [...document.querySelectorAll('#plugins-panel-gallery p')].some((p) => p.textContent === arguments[0])"
LOADING = "return !!document.querySelector('#plugins-panel-gallery [role=status][aria-label=\"Loading the gallery\"]')"
AUTO = "return window.__nuzky.plugins.getState().auto"
AUTO_ON = ("window.__nuzky.plugins.setState((s) => ({auto: {import: [...s.auto.import, arguments[0]], "
           "export: [...s.auto.export, arguments[0]]}}))")
TOAST = """const t = window.__nuzky.store.getState().toasts.find((t) => t.text === arguments[0]);
return t ? {kind: t.kind, action: t.action?.label ?? null} : null"""


def png():
    from PIL import Image
    out = io.BytesIO()
    Image.new('RGB', (64, 64), (255, 220, 0)).save(out, 'PNG')
    return out.getvalue()


def summary():
    """What the detail must say of Beat flash, in this order, from its manifest and files alone."""
    return ['Can change anything in this project, as one step you can undo.',
            f'Adds 1 picture of its own to your video and Media ({len(png())} B).', 'Reads the beats of the music.',
            "Can't reach the internet, your files or other apps.",
            'Installing runs nothing. Opening it shows a preview: its script runs without changing your project.',
            'It changes your project only when you press its button, or by itself after import or before export if '
            'you turn that on.']


def beat_flash(version, script, needs=('beats',)):
    manifest = {'nuzkyPlugin': 2, 'id': 'beat-flash', 'name': 'Beat flash', 'version': version, 'action': 'Flash',
                'description': DESCRIPTION, 'needs': list(needs), 'files': ['flash.png']}
    return manifest, {'plugin.json': json.dumps(manifest).encode(), 'plugin.js': script, 'flash.png': png()}


def shared_example():
    """The example plugin as someone shares it, a newer version of the one the user made."""
    manifest = json.loads((EXAMPLE / 'plugin.json').read_text())
    manifest['version'] = '9.0.0'
    return manifest, {'plugin.json': json.dumps(manifest).encode(), 'plugin.js': (EXAMPLE / 'plugin.js').read_bytes()}


def shared_look():
    """The guide's VHS example as someone shares it: a look and no script."""
    files = {path: (LOOK / path).read_bytes() for path in ('plugin.json', 'README.md', 'effects/vhs.wgsl')}
    return json.loads(files['plugin.json']), files


def runs_by_itself(r, id):
    auto = r.s.run(AUTO)
    return id in auto['import'] or id in auto['export']


def odd_files():
    """A plugin whose listing includes a file no plugin's folder may hold."""
    manifest = {'nuzkyPlugin': 2, 'id': 'odd-files', 'name': 'Odd files', 'version': '1.0.0',
                'description': 'Comes with my notes.'}
    return manifest, {'plugin.json': json.dumps(manifest).encode(), 'plugin.js': b'function run() { return []; }',
                      'notes.txt': b'my notes'}


class Gallery:
    """index.json at /main/ and each plugin's files at /<commit>/plugins/<id>/, as raw.githubusercontent.com serves
    the gallery repository. `served` may differ from what the index lists, as a tampered file would. `status` answers
    the list with another status, `hold` keeps the list until it is set, and `slow` sends half of a file and the rest
    once its event is set."""
    def __init__(self):
        self.version, self.plugins, self.served, self.log, self.port = 1, [], {}, [], 0
        self.status, self.hold, self.slow = 200, None, {}
        self.start()

    def start(self):
        gallery = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                gallery.log.append({'path': self.path, 'headers': {k.lower(): v for k, v in self.headers.items()}})
                if self.path == '/main/index.json':
                    if gallery.hold:
                        gallery.hold.wait(30)
                    status, body = gallery.status, gallery.index() if gallery.status == 200 else b''
                else:
                    body = gallery.served.get(self.path)
                    status = 200 if body is not None else 404
                self.send_response(status)
                self.send_header('Content-Length', str(len(body or b'')))
                self.end_headers()
                slow = gallery.slow.get(self.path)
                if slow and body:
                    self.wfile.write(body[:len(body) // 2])
                    self.wfile.flush()
                    slow.wait(30)
                    body = body[len(body) // 2:]
                try:
                    self.wfile.write(body or b'')
                except OSError:
                    pass

            def log_message(self, *args):
                pass

        self.http = ThreadingHTTPServer(('127.0.0.1', self.port), Handler)
        self.port = self.http.server_port
        threading.Thread(target=self.http.serve_forever, daemon=True).start()
        self.root = f'http://127.0.0.1:{self.port}'

    def stop(self):
        self.http.shutdown()
        self.http.server_close()

    def add(self, manifest, files, served=None):
        self.plugins = [p for p in self.plugins if p['manifest']['id'] != manifest['id']]
        self.plugins.append({'author': 'Ann Example', 'manifest': manifest, 'files': [
            {'path': path, 'size': len(data), 'sha256': hashlib.sha256(data).hexdigest()} for path, data in files.items()]})
        for path, data in (served or files).items():
            self.served[f"/{COMMIT}/plugins/{manifest['id']}/{path}"] = data

    def index(self):
        return json.dumps({'nuzkyGallery': self.version, 'commit': COMMIT, 'plugins': self.plugins}).encode()

    def files_asked(self):
        return [entry['path'] for entry in self.log if entry['path'] != '/main/index.json']


def prepare(r):
    r.gallery = Gallery()
    r.env['NUZKY_GALLERY_URL'] = r.gallery.root
    r.gallery.add(*beat_flash('1.0.0', BROKEN_SCRIPT))
    manifest, files = beat_flash('1.0.0', BROKEN_SCRIPT)
    manifest['id'] = manifest['name'] = 'tampered'
    files['plugin.json'] = json.dumps(manifest).encode()
    r.gallery.add(manifest, files, served=dict(files, **{'plugin.js': b'function run(ctx) { return []; } // changed'}))
    r.gallery.add(*shared_example())
    r.gallery.add(*odd_files())
    r.gallery.add(*shared_look())


def trashed(r, name):
    """The folder the app moved to this run's Trash, moved out of it so the next one of that name fits."""
    for trash in (r.work / 'home/.Trash', r.work / 'data/Trash/files'):
        found = sorted(trash.glob(name + '*')) if trash.is_dir() else []
        if found:
            kept = r.work / f'from-trash-{name}-{len(list(r.work.glob("from-trash-*")))}'
            shutil.move(found[0], kept)
            return kept
    return None


def staging_left(r):
    return [p.name for p in (r.work / 'data/nuzky').iterdir() if p.name.startswith('gallery-part-')]


def record(r):
    path = r.work / 'data/nuzky/plugin-gallery.json'
    return json.loads(path.read_text()) if path.exists() else {}


def detail(r, id):
    return r.s.run(DETAIL, id)


def resize(r, width, height):
    webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
    return wait(lambda: r.s.run('return [window.innerWidth, window.innerHeight]') == [width, height], 5)


def shot(r, name):
    time.sleep(0.3)  # The rows' colours fade over 120 ms.
    r.shot(name)


def state(r, id):
    return r.s.run("return window.__nuzky.gallery.getState().list?.find((p) => p.id === arguments[0])?.state ?? null", id)


def reopen(r):
    """Leaves the Gallery and opens it again, which reads the list again."""
    r.s.run(TAB, 'installed')
    r.s.run(TAB, 'gallery')


@flow('plugins_gallery', 'The Gallery lists plugins other people shared and says in Nuzky\'s own words what each may '
      'do; Install downloads only that plugin\'s checked files without running them, shows its progress with Stop and '
      'then offers Open, a tampered file installs nothing, files plugins may not have are refused and marked, '
      'replacing the user\'s own or edited plugin asks first by its name and Esc cancels, an update marks what is new, '
      'and offline, busy, newer and empty galleries each say so', before=prepare)
def plugins_gallery(r):
    g = r.gallery
    plugins = r.work / 'data/nuzky/plugins'
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((b) => b.textContent === 'Plugins').click()")
    browse = "return [...document.querySelectorAll('#plugins-panel-installed button')].find((b) => b.textContent.includes('Browse the gallery'))"
    r.check('with no plugins, Installed offers Browse the gallery', wait(lambda: r.s.run(browse), 10))
    shot(r, 'installed-empty')
    r.check('nothing asks the gallery before the Gallery opens', g.log == [], list(g.log))

    shutil.copytree(EXAMPLE, plugins / 'remove-short-clips')
    g.hold = threading.Event()
    r.s.run(browse[len('return '):] + '.click()')
    r.check('Browse the gallery opens the Gallery, which shows skeleton rows under who shares the plugins while it loads',
            wait(lambda: r.s.run(LOADING), 5) and r.s.run(SHARED_LINE, SHARED))
    shot(r, 'gallery-loading')
    g.hold.set()
    g.hold = None
    r.check('the Gallery lists the shared plugins', wait(lambda: all(r.s.run(ROW, id) for id in
            ('beat-flash', 'tampered', 'remove-short-clips', 'odd-files', 'vhs')), 10))
    r.check('one line over the list says who shares and checks the plugins', r.s.run(SHARED_LINE, SHARED))
    r.s.run(TAB, 'installed')
    r.check('Installed now lists the user\'s own plugin',
            wait(lambda: r.s.run("return !!document.querySelector('[data-plugin=\"remove-short-clips\"]')"), 10))
    r.s.run(TAB, 'gallery')
    r.check('the plugin with the name of the user\'s own says so in its row',
            wait(lambda: r.s.run(MARK, 'remove-short-clips') == 'Same name as yours', 10), r.s.run(MARK, 'remove-short-clips'))
    shot(r, 'gallery')

    r.s.run(OPEN, 'beat-flash')
    shown = wait(lambda: detail(r, 'beat-flash'), 5)
    r.check('the detail says what Nuzky lets the plugin do, from its manifest, in order',
            shown and shown['summary'] == summary() and 'What Nuzky lets it do' in shown['text'], shown and shown['summary'])
    r.check('none of those lines is the author\'s text, which comes right after the button and before the files',
            shown and not any('best plugin' in line for line in shown['summary']) and shown['author'] == DESCRIPTION
            and shown['sections'] == ['What Nuzky lets it do', 'action', 'From the author', 'Files'], shown and shown['sections'])
    r.check('the detail lists the files with their sizes',
            shown and [f.split(' ')[0] for f in shown['files']] == ['plugin.json', 'plugin.js', 'flash.png'], shown and shown['files'])
    r.check('opening the detail asks the gallery nothing more', g.files_asked() == [], g.log)
    r.s.run(OPEN, 'vhs')
    look = wait(lambda: detail(r, 'vhs'), 5)
    r.check('a plugin that only adds a look says so, and neither that it changes the project nor that it has a button',
            look and look['summary'] == ['Adds a look you can put on clips: VHS.', "Can't reach the internet, your files or other apps.",
                                         'Installing changes no project. Its looks show only on clips you put them on.'],
            look and look['summary'])
    shot(r, 'gallery-look')
    r.s.run(OPEN, 'beat-flash')
    wait(lambda: detail(r, 'beat-flash'), 5)
    shot(r, 'gallery-detail')
    r.check('the window goes to its smallest size', resize(r, 1024, 640))
    shot(r, 'gallery-detail-1024')
    page = r.s.run("""const d = document.querySelector('[data-gallery-detail]'), a = d.querySelector('[data-gallery-author]');
        d.scrollTop = d.clientHeight - 48;
        const box = d.getBoundingClientRect(), text = a.getBoundingClientRect();
        return {fits: text.top >= box.top && text.bottom <= box.bottom, after: a.closest('section').previousElementSibling.querySelector('[data-gallery-main]') !== null};""")
    r.check('at 1024 × 640 the author\'s text follows the button and shows whole one page down', page == {'fits': True, 'after': True}, page)
    shot(r, 'gallery-detail-1024-scrolled')
    resize(r, 1440, 900)

    flash = f'/{COMMIT}/plugins/beat-flash/flash.png'
    g.slow[flash] = threading.Event()
    r.check('Install has the keyboard focus', r.s.run(FOCUS, 'Install'))
    press('Return')
    progress = wait(lambda: r.s.run("return document.querySelector('[data-gallery-progress]')?.innerText ?? null"), 10)
    r.check('Enter on Install shows the download\'s percentage with Stop',
            progress and 'Installing…' in progress and '%' in progress and 'Stop' in progress, progress)
    r.check('Stop takes the focus Install had', wait(lambda: r.s.run(FOCUSED) == 'Stop', 3), r.s.run(FOCUSED))
    shot(r, 'gallery-installing')
    press('Return')
    r.check('Enter on Stop ends the install with nothing installed',
            wait(lambda: not r.s.run("return !!document.querySelector('[data-gallery-progress]')"), 3)
            and state(r, 'beat-flash') == 'available' and not (plugins / 'beat-flash').exists())
    r.check('after Stop the focus is on Install again', wait(lambda: r.s.run(FOCUSED) == 'Install', 3), r.s.run(FOCUSED))
    g.slow.pop(flash).set()
    r.check('nothing is left of the stopped download', wait(lambda: staging_left(r) == [], 10), staging_left(r))
    asked_before = len(g.log)

    press('Return')
    installed = wait(lambda: state(r, 'beat-flash') == 'installed', 15)
    r.check('Enter on Install again installs the plugin', installed, state(r, 'beat-flash'))
    toast = r.s.run(TOAST, 'Installed “Beat flash”.')
    r.check('a toast says it is installed and offers Open', toast == {'kind': 'success', 'action': 'Open'}, toast)
    _, files = beat_flash('1.0.0', BROKEN_SCRIPT)
    on_disk = {path: hashlib.sha256((plugins / 'beat-flash' / path).read_bytes()).hexdigest()
               for path in files if (plugins / 'beat-flash' / path).is_file()}
    expected = {path: hashlib.sha256(data).hexdigest() for path, data in files.items()}
    r.check('the plugin\'s folder holds exactly its files with the listed SHA-256, the broken script included',
            on_disk == expected and sorted(p.name for p in (plugins / 'beat-flash').iterdir()) == sorted(files), on_disk)
    r.check('Nuzky records where the plugin came from outside its folder',
            record(r).get('beat-flash', {}).get('commit') == COMMIT and record(r)['beat-flash']['files'] == expected, record(r))
    asked = [e['path'] for e in g.log[asked_before:]]
    r.check('the server was asked only for that plugin\'s files, by the listed commit',
            sorted(asked) == sorted(f'/{COMMIT}/plugins/beat-flash/{path}' for path in files), asked)
    r.check('no request carries a query or a cookie',
            all('?' not in e['path'] and 'cookie' not in e['headers'] for e in g.log), g.log)
    r.check('nothing is left of the download beside the plugins folder', staging_left(r) == [], staging_left(r))
    r.check('installing does not open or run the plugin', r.s.run(
        'const p = window.__nuzky.plugins.getState(); return p.open === null && p.preview === null'))
    r.check('the installed plugin shows under Installed, its script never run', wait(lambda: r.s.run(
        "return window.__nuzky.plugins.getState().list?.some((p) => p.id === 'beat-flash' && !p.error)"), 5))
    shown = detail(r, 'beat-flash')
    r.check('its detail says this version is installed and offers Open',
            shown and 'This version is installed.' in shown['text'] and {'text': 'Open', 'off': False} in shown['buttons'], shown)
    r.check('the focus went from Install to Open', r.s.run(FOCUSED) == 'Open', r.s.run(FOCUSED))
    shot(r, 'gallery-installed')
    r.s.run(CLICK, 'Open')
    r.check('Open opens the installed plugin under Installed', wait(lambda: r.s.run(
        "return window.__nuzky.plugins.getState().open === 'beat-flash' && window.__nuzky.gallery.getState().tab === 'installed'"), 5))
    r.s.run(TAB, 'gallery')
    wait(lambda: r.s.run(ROW, 'tampered'), 10)

    before = len(g.log)
    r.s.run(OPEN, 'tampered')
    wait(lambda: detail(r, 'tampered'), 5)
    r.s.run(CLICK, 'Install')
    failed = wait(lambda: r.s.run(PROBLEM), 15)
    r.check('a file that differs from the list installs nothing and says so in plain words',
            failed == 'Nothing was installed. A file didn’t download correctly. Try again.', failed)
    r.check('the button then says Try again', any(b['text'] == 'Try again' for b in detail(r, 'tampered')['buttons']))
    shot(r, 'gallery-corrupt')
    r.check('the tampered plugin is not installed and nothing is left of it',
            not (plugins / 'tampered').exists() and staging_left(r) == [] and 'tampered' not in record(r), staging_left(r))
    r.check('it asked only for the tampered plugin\'s files',
            all(e['path'].startswith(f'/{COMMIT}/plugins/tampered/') for e in g.log[before:]), g.log[before:])

    before = len(g.log)
    r.s.run(OPEN, 'odd-files')
    shown = wait(lambda: detail(r, 'odd-files'), 5)
    r.check('a plugin with a file plugins may not have says Nuzky won\'t install it, with no button',
            shown and REFUSED in shown['text'] and shown['buttons'] == [], shown)
    r.check('its Files mark that file as not allowed', shown and shown['refused'] == ['notes.txt'], shown and shown['files'])
    r.check('the refused plugin\'s row says it can\'t be installed', 'Can’t be installed' in r.s.run(
        "return document.querySelector('[data-gallery-plugin=\"odd-files\"]').textContent"))
    shot(r, 'gallery-refused')
    r.check('it asked nothing for the refused plugin', len(g.log) == before, g.log[before:])

    r.s.run(OPEN, 'vhs')
    wait(lambda: detail(r, 'vhs'), 5)
    r.s.run(CLICK, 'Install')
    r.check('the look installs with its checked shader and no script', wait(lambda: state(r, 'vhs') == 'installed', 15)
            and (plugins / 'vhs/effects/vhs.wgsl').read_bytes() == (LOOK / 'effects/vhs.wgsl').read_bytes()
            and not (plugins / 'vhs/plugin.js').exists())

    own = (plugins / 'remove-short-clips' / 'plugin.json').read_text()
    name = json.loads(own)['name']
    # The user had their own plugin run by itself; the gallery's code replacing it must not.
    r.s.run(AUTO_ON, 'remove-short-clips')
    r.s.run(OPEN, 'remove-short-clips')
    shown = wait(lambda: detail(r, 'remove-short-clips'), 5)
    note = f'Your own plugin “{name}” has the same name.'
    r.check('the plugin with the name of the user\'s own names it and offers Replace your plugin',
            shown and note in shown['text'] and any(b['text'] == 'Replace your plugin' for b in shown['buttons']), shown)
    shot(r, 'gallery-clash')
    r.s.run(CLICK, 'Replace your plugin')
    question = f'Your plugin “{name}” goes to the Trash. You can put it back from there.'
    asking = wait(lambda: question in (detail(r, 'remove-short-clips') or {}).get('text', ''), 5)
    r.check('Replace asks first, naming the user\'s plugin that goes to the Trash, and keeps the note',
            asking and note in detail(r, 'remove-short-clips')['text'], detail(r, 'remove-short-clips'))
    r.check('Cancel has the focus, so Enter keeps the user\'s plugin', r.s.run(FOCUSED) == 'Cancel', r.s.run(FOCUSED))
    shot(r, 'gallery-replace')
    r.check('asking alone changes nothing', (plugins / 'remove-short-clips' / 'plugin.json').read_text() == own
            and trashed(r, 'remove-short-clips') is None)
    press('Escape')
    r.check('Esc cancels the question and keeps the user\'s plugin',
            wait(lambda: question not in detail(r, 'remove-short-clips')['text'], 3)
            and (plugins / 'remove-short-clips/plugin.json').read_text() == own, detail(r, 'remove-short-clips'))
    r.check('the focus goes back to Replace your plugin', wait(lambda: r.s.run(FOCUSED) == 'Replace your plugin', 3),
            r.s.run(FOCUSED))
    r.s.run(CLICK, 'Replace your plugin')
    wait(lambda: question in detail(r, 'remove-short-clips')['text'], 5)
    r.s.run(CLICK, 'Cancel')
    r.check('Cancel keeps the user\'s plugin', wait(lambda: any(b['text'] == 'Replace your plugin' for b in
            detail(r, 'remove-short-clips')['buttons']), 5) and (plugins / 'remove-short-clips/plugin.json').read_text() == own)
    r.s.run(CLICK, 'Replace your plugin')
    wait(lambda: question in detail(r, 'remove-short-clips')['text'], 5)
    r.s.run(CLICK, 'Replace your plugin')
    r.check('confirming installs the gallery\'s version', wait(lambda: state(r, 'remove-short-clips') == 'installed', 15)
            and json.loads((plugins / 'remove-short-clips/plugin.json').read_text())['version'] == '9.0.0')
    kept = wait(lambda: trashed(r, 'remove-short-clips'), 5)
    r.check('the user\'s plugin is in the Trash', kept and (kept / 'plugin.json').read_text() == own, kept)
    r.check('replacing it turns off its runs after import and before export', not runs_by_itself(r, 'remove-short-clips'),
            r.s.run(AUTO))

    edited = b'function run(ctx) { return []; } // my AI changed this'
    (plugins / 'beat-flash/plugin.js').write_bytes(edited)
    reopen(r)
    r.check('a plugin edited after installing shows Edited in its row',
            wait(lambda: state(r, 'beat-flash') == 'changed', 10) and r.s.run(MARK, 'beat-flash') == 'Edited',
            [state(r, 'beat-flash'), r.s.run(MARK, 'beat-flash')])
    r.s.run(OPEN, 'beat-flash')
    shown = wait(lambda: detail(r, 'beat-flash'), 5)
    r.check('its detail says the user or their AI edited it and offers Replace your edited copy',
            shown and 'You or your AI edited your copy.' in shown['text']
            and any(b['text'] == 'Replace your edited copy' for b in shown['buttons']), shown)
    r.s.run(CLICK, 'Replace your edited copy')
    r.check('Replace asks first, saying the edited copy goes to the Trash with its changes', wait(lambda:
            'Your edited copy goes to the Trash, with your changes.' in detail(r, 'beat-flash')['text'], 5))
    shot(r, 'gallery-edited-replace')
    r.s.run(CLICK, 'Replace your edited copy')
    r.check('confirming installs the gallery\'s version over the edited copy', wait(lambda: state(r, 'beat-flash') == 'installed', 15)
            and (plugins / 'beat-flash/plugin.js').read_bytes() == BROKEN_SCRIPT)
    kept = wait(lambda: trashed(r, 'beat-flash'), 5)
    r.check('the edited copy is in the Trash with its changes', kept and (kept / 'plugin.js').read_bytes() == edited, kept)

    newer = beat_flash('1.1.0', b'function run(ctx) { return {edits: [], message: "Flashed."}; }', needs=('beats', 'loudness'))
    g.add(*newer)
    reopen(r)
    r.check('opened again, the Gallery reads its list again and offers the update',
            wait(lambda: state(r, 'beat-flash') == 'update', 10), state(r, 'beat-flash'))
    r.check('the row says Update', r.s.run(MARK, 'beat-flash') == 'Update', r.s.run(MARK, 'beat-flash'))
    r.s.run(OPEN, 'beat-flash')
    shown = wait(lambda: detail(r, 'beat-flash'), 5)
    r.check('the detail says which version is installed', shown and 'You have version 1.0.0' in shown['text'], shown)
    r.check('only what the installed version could not do is marked New',
            shown and shown['new'] == ['NewReads how loud the sound is.'], shown and shown['summary'])
    shot(r, 'gallery-update')
    r.s.run(AUTO_ON, 'beat-flash')
    r.s.run(CLICK, 'Update')
    r.check('Update installs the new version', wait(lambda: state(r, 'beat-flash') == 'installed', 15)
            and (plugins / 'beat-flash/plugin.js').read_bytes() == newer[1]['plugin.js'])
    old = wait(lambda: trashed(r, 'beat-flash'), 5)
    r.check('the old version is in the Trash', old and (old / 'plugin.js').read_bytes() == BROKEN_SCRIPT, old)
    r.check('installed, nothing is marked New', not detail(r, 'beat-flash')['new'], detail(r, 'beat-flash'))
    r.check('updating turns off its runs after import and before export', not runs_by_itself(r, 'beat-flash'), r.s.run(AUTO))

    # A pass of automatic runs is going on while the user updates a plugin it was about to run: the new code must not
    # run in it. A slow plugin of the user's own, named to run first, holds the pass while Update installs.
    slow = plugins / 'a-slow-one'
    slow.mkdir()
    (slow / 'plugin.json').write_text(json.dumps({'nuzkyPlugin': 2, 'id': 'a-slow-one', 'name': 'A slow one',
                                                  'version': '1.0.0', 'action': 'Wait', 'description': 'Takes a while.'}))
    (slow / 'plugin.js').write_text('function run(ctx) { const end = Date.now() + 4000; while (Date.now() < end) {} return []; }')
    replaced = beat_flash('1.2.0', b'function run(ctx) { return {edits: [], message: "THE NEW CODE RAN."}; }')
    g.add(*replaced)
    reopen(r)
    wait(lambda: state(r, 'beat-flash') == 'update', 10)
    r.s.run(OPEN, 'beat-flash')
    wait(lambda: detail(r, 'beat-flash'), 5)
    r.s.run(AUTO_ON, 'a-slow-one')
    r.s.run(AUTO_ON, 'beat-flash')
    still = r.work / 'still.png'
    still.write_bytes(png())
    r.import_media(still)
    holding = wait(lambda: r.s.run("const p = window.__nuzky.plugins.getState(); "
                                   "return p.pass === 'import' && p.running === 'a-slow-one' && p.step?.total === 2"), 10)
    r.check('an import starts a pass that runs the slow plugin first, then the plugin about to be updated', holding,
            r.s.run("const p = window.__nuzky.plugins.getState(); return [p.pass, p.running, p.step]"))
    r.s.run(CLICK, 'Update')
    r.check('Update installs while the pass goes on', wait(lambda: state(r, 'beat-flash') == 'installed', 15)
            and (plugins / 'beat-flash/plugin.js').read_bytes() == replaced[1]['plugin.js']
            and r.s.run("return window.__nuzky.plugins.getState().pass") == 'import')
    trashed(r, 'beat-flash')
    r.check('the pass ends', wait(lambda: r.s.run("return window.__nuzky.plugins.getState().pass") is None, 20))
    texts = [t['text'] for t in r.state()['toasts']]
    r.check('the plugin replaced during the pass did not run in it, neither its new code nor its old',
            not any(('Beat flash' in text and 'after import' in text) or 'THE NEW CODE RAN' in text for text in texts)
            and any(text.startswith('A slow one ran after import') for text in texts), texts)
    r.check('and runs by itself no more', not runs_by_itself(r, 'beat-flash'), r.s.run(AUTO))
    r.s.run("window.__nuzky.plugins.setState((s) => ({auto: {import: s.auto.import.filter((id) => id !== 'a-slow-one'), "
            "export: s.auto.export.filter((id) => id !== 'a-slow-one')}}))")

    # An update that installs nothing leaves its switches as they were.
    tampered = beat_flash('1.3.0', b'function run(ctx) { return []; }')
    g.add(*tampered, served=dict(tampered[1], **{'plugin.js': b'function run(ctx) { return []; } // not as listed'}))
    reopen(r)
    wait(lambda: state(r, 'beat-flash') == 'update', 10)
    r.s.run(OPEN, 'beat-flash')
    wait(lambda: detail(r, 'beat-flash'), 5)
    r.s.run(AUTO_ON, 'beat-flash')
    r.s.run(CLICK, 'Update')
    failed = wait(lambda: r.s.run(PROBLEM), 15)
    r.check('an update whose file differs installs nothing and leaves the plugin running by itself as before',
            failed and failed.startswith('Nothing was installed') and runs_by_itself(r, 'beat-flash')
            and (plugins / 'beat-flash/plugin.js').read_bytes() == replaced[1]['plugin.js'], [failed, r.s.run(AUTO)])
    r.s.run("window.__nuzky.plugins.setState((s) => ({auto: {import: s.auto.import.filter((id) => id !== 'beat-flash'), "
            "export: s.auto.export.filter((id) => id !== 'beat-flash')}}))")

    g.stop()
    reopen(r)
    offline = wait(lambda: r.s.run(PROBLEM), 20)
    r.check('without the server the Gallery says it can\'t reach it, with Try again',
            offline and 'Can’t reach the gallery' in offline and 'Try again' in offline, offline)
    shot(r, 'gallery-offline')
    g.start()
    r.s.run("[...document.querySelectorAll('[role=alert] button')].find((b) => b.textContent === 'Try again').click()")
    r.check('Try again lists the plugins again', wait(lambda: r.s.run(ROW, 'beat-flash'), 10))

    g.status = 403
    reopen(r)
    busy = wait(lambda: r.s.run(PROBLEM), 10)
    r.check('GitHub refusing says it is busy, with Try again', busy and 'GitHub is busy' in busy and 'Try again' in busy, busy)
    shot(r, 'gallery-busy')
    g.status = 200

    g.version = 2
    reopen(r)
    newer_gallery = wait(lambda: r.s.run(PROBLEM), 10)
    r.check('a gallery made for a newer Nuzky says so in one sentence, with nothing to press while no update is out',
            newer_gallery == 'This gallery needs a newer Nuzky' and not r.s.run(
                "return document.querySelector('#plugins-panel-gallery [role=alert] button')"), newer_gallery)
    updates = r.s.run("const u = window.__nuzky.updates.getState(); return {enabled: u.enabled, latest: u.latest, canInstall: u.canInstall}")
    r.s.run("window.__nuzky.updates.setState({enabled: true, latest: '9.9.9', canInstall: true})")
    offered = wait(lambda: r.s.run(
        "return document.querySelector('#plugins-panel-gallery [role=alert] button')?.textContent ?? null") == 'Update to Nuzky 9.9.9', 3)
    r.check('with a newer Nuzky out, it offers the update', offered)
    shot(r, 'gallery-newer')
    r.s.run("window.__nuzky.updates.setState(arguments[0])", updates)
    g.version = 1

    listed = g.plugins
    g.plugins = []
    reopen(r)
    empty = wait(lambda: r.s.run("return document.querySelector('#plugins-panel-gallery h3')?.textContent ?? null") == 'The gallery is empty', 10)
    r.check('an empty gallery says so', empty)
    shot(r, 'gallery-empty')
    g.plugins = listed

    r.s.run(TAB, 'installed')
    wait(lambda: r.s.run("return !!document.querySelector('[aria-label=\"More for Beat flash\"]')"), 5)
    r.s.run("document.querySelector('[aria-label=\"More for Beat flash\"]').click()")
    r.s.run("[...document.querySelectorAll('[role=menuitem]')].find((b) => b.textContent.includes('Move to Trash')).click()")
    gone = wait(lambda: not (plugins / 'beat-flash').exists(), 5)
    r.check('Move to Trash removes the installed gallery plugin', gone)
    r.check('it lands in the Trash and the gallery\'s record forgets it',
            wait(lambda: trashed(r, 'beat-flash'), 5) and wait(lambda: 'beat-flash' not in record(r), 5), record(r))
    g.stop()
    r.check('no error toast', not r.errors(), r.errors())

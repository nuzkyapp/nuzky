import base64, http.client, json, socket, time, urllib.request
from pathlib import Path
from urllib.parse import parse_qs, quote, urlsplit

from e2e.harness import FIXTURES, LINUX, Bridge, MACOS, WEBKIT_DRIVER, ffprobe, flow, start, started, stop, wait
from e2e.ai_panel import FAKE

BUTTON = 'button[aria-label="Edit from your phone or another computer"], button[title="Edit from your phone or another computer"]'


def setup(r):
    # The fake stands in for Claude Code; exports the page asks for go to this run's Videos folder, not the person's.
    r.env.update(NUZKY_AGENT_CLAUDE=str(FAKE), NUZKY_FAKE_AGENT_LOG=str(r.work / 'fake'), XDG_CONFIG_HOME=str(r.work / 'config'))
    (r.work / 'config').mkdir()
    (r.work / 'config/user-dirs.dirs').write_text(f'XDG_VIDEOS_DIR="{r.work / "videos"}"\n')


class Page:
    """What the page in a phone's browser sends, over plain HTTP as the browser does."""

    def __init__(self, url):
        parts = urlsplit(url)
        self.host, self.key, self.cookie = parts.netloc, parse_qs(parts.query)['k'][0], None

    def request(self, method, path, body=None, headers=None, timeout=30):
        connection = http.client.HTTPConnection(self.host, timeout=timeout)
        sent = {'Host': self.host, **({'Cookie': self.cookie} if self.cookie else {}), **(headers or {})}
        connection.request(method, path, body=body, headers=sent)
        response = connection.getresponse()
        return response.status, {k.lower(): v for k, v in response.getheaders()}, response.read()

    def act(self, **action):
        return self.request('POST', '/action', json.dumps(action), {'X-Nuzky': '1', 'Content-Type': 'application/json'})[0]

    def upload(self, path, name):
        return self.request('POST', f'/upload?name={quote(name)}', Path(path).read_bytes(), {'X-Nuzky': '1'}, timeout=120)[0]

    def state(self):
        """The newest state the editor published, as a newly opened page gets it first."""
        connection = http.client.HTTPConnection(self.host, timeout=10)
        event = None
        try:
            connection.request('GET', '/events', headers={'Host': self.host, 'Cookie': self.cookie, 'User-Agent': 'iPhone'})
            response = connection.getresponse()
            while line := response.fp.readline().decode():
                if line.startswith('event: '):
                    event = line[7:].strip()
                elif line.startswith('data: ') and event == 'state':
                    return json.loads(line[6:])
        except OSError:
            return None
        finally:
            connection.close()


class Phone:
    """The page in a real browser: WebKit, the engine of Safari on an iPhone, at an iPhone's size. Linux only, where
    WebKitGTK's MiniBrowser runs under the WebDriver inside the run's headless gamescope."""

    BROWSER = Path('/usr/libexec/webkit2gtk-4.1/MiniBrowser')

    def __init__(self, r):
        with socket.socket() as s:
            s.bind(('127.0.0.1', 0))
            self.port = s.getsockname()[1]
        self.driver = start([WEBKIT_DRIVER, f'--port={self.port}', '--host=127.0.0.1'], r.work / 'phone-driver.log', r.launch_env())
        if not started(self.driver, self.port, 20):
            raise RuntimeError('the phone\'s WebDriver did not start, see phone-driver.log')
        caps = {'capabilities': {'alwaysMatch': {'webkitgtk:browserOptions': {'binary': str(self.BROWSER), 'args': ['--automation', '--private']}}}}
        self.path = '/session/' + self.call('POST', '/session', caps)['sessionId']
        self.call('POST', self.path + '/window/rect', {'width': 402, 'height': 874})

    def call(self, method, path, body=None):
        request = urllib.request.Request(f'http://127.0.0.1:{self.port}{path}', data=json.dumps(body).encode() if body is not None else None,
                                         method=method, headers={'Content-Type': 'application/json'})
        with urllib.request.urlopen(request, timeout=120) as response:
            return json.loads(response.read() or b'{}').get('value')

    def open(self, url):
        self.call('POST', self.path + '/url', {'url': url})

    def run(self, script):
        return self.call('POST', self.path + '/execute/sync', {'script': script, 'args': []})

    def tap(self, css):
        """A click the browser makes itself, with the pointer events a finger's tap fires."""
        element = self.call('POST', self.path + '/element', {'using': 'css selector', 'value': css})
        self.call('POST', f"{self.path}/element/{next(iter(element.values()))}/click", {})

    def shown(self, css):
        return self.run(f"const e = document.querySelector('{css}'); return !!e && !e.closest('[hidden]') && e.getClientRects().length > 0")

    def shot(self, path):
        path.write_bytes(base64.b64decode(self.call('GET', self.path + '/screenshot')))

    def close(self):
        try:
            self.call('DELETE', self.path)
        finally:
            stop(self.driver)


THUMBS = "return performance.getEntriesByType('resource').filter((e) => e.name.includes('/thumb/')).length"
LOADED = """const f = document.getElementById('frame');
return !f.hidden && f.complete && f.naturalWidth > 0 && document.querySelectorAll('.clip:not(.pending)').length === 2"""


def on_phone(r, url, page):
    phone = Phone(r)
    try:
        began = time.monotonic()
        phone.open(url)
        loaded = wait(lambda: phone.run(LOADED), 30, step=0.05)
        took = time.monotonic() - began
        timing = phone.run("""return performance.getEntriesByType('resource').concat(performance.getEntriesByType('navigation'))
            .map((e) => [e.name.replace(location.origin, ''), Math.round(e.startTime), Math.round(e.duration), e.encodedBodySize])""")
        r.check('the page opens on a phone with the preview and the clips', loaded, timing)
        print(f'  phone page ready in {took:.2f} s: {timing}', flush=True)
        wait(lambda: phone.run(THUMBS) >= 2, 10)
        fetched = phone.run(THUMBS)
        # Each seek publishes a new state and a new frame, as playing does several times a second.
        for us in range(250_000, 2_500_000, 250_000):
            page.act(action='seek', us=us)
            time.sleep(0.4)
        r.check('the strip keeps its thumbnails while the playhead moves, so the clips do not blink',
                fetched == 2 and phone.run(THUMBS) == fetched, [fetched, phone.run(THUMBS)])
        phone.shot(r.work / 'share-phone.png')
        r.shots.append('share-phone.png')
    finally:
        phone.close()


def page_state(page, check, timeout=60):
    return wait(lambda: (s := page.state()) and check(s) and s, timeout, step=0.5)


@flow('share', 'Edit from your phone: the code opens the page and nothing opens it without the code, phone videos land at the '
      'end of the timeline in the order sent, a message from the page reaches the AI panel and its run shows there, '
      'a choice and Stop work from the page, it seeks and shows the preview, exports and downloads that export, and '
      'Stop sharing closes it', before=setup)
def share(r):
    r.s.run(f"document.querySelector('{BUTTON}').click()")
    sharing = wait(lambda: r.s.run('return window.__nuzky.remote.getState().sharing'), 10)
    r.check('the dialog shows a QR code of a link with a secret', sharing and sharing['qr'].startswith('<?xml') and '/?k=' in sharing['url'], sharing and sharing['url'])
    # The dialog fades in.
    time.sleep(0.5)
    r.shot('share-dialog')
    page = Page(sharing['url'])

    status, headers, _ = page.request('GET', '/')
    r.check('without the code the page does not open', status == 403, status)
    status, headers, _ = page.request('GET', '/?k=' + 'a' * len(page.key))
    r.check('a wrong code does not open it either', status == 403 and 'set-cookie' not in headers, status)
    status, headers, _ = page.request('GET', f'/?k={page.key}', headers={'Host': f'nuzky.example:{page.host.split(":")[1]}'})
    r.check('another host name does not reach it, as a site rebinding its name would', status == 403, status)
    status, headers, _ = page.request('GET', f'/?k={page.key}')
    page.cookie = headers.get('set-cookie', '').split(';')[0]
    r.check('the code is traded for a cookie the page then uses', status == 303 and page.cookie.endswith(page.key)
            and 'HttpOnly' in headers.get('set-cookie', '') and 'SameSite=Lax' in headers.get('set-cookie', ''), headers)
    status, headers, html = page.request('GET', '/')
    r.check('with it the page opens', status == 200 and b'/app.js' in html and 'script-src' in headers.get('content-security-policy', ''), status)
    status, _, _ = page.request('POST', '/action', json.dumps({'action': 'export'}), {'Content-Type': 'application/json'})
    r.check('a request without the page\'s header is refused, as another site\'s form would be', status == 403, status)
    status, _, _ = page.request('POST', '/action', json.dumps({'action': 'export'}), {'X-Nuzky': '1', 'Origin': 'http://evil.example'})
    r.check('a request from another site is refused', status == 403, status)
    status, _, said = page.request('POST', '/upload?name=notes.txt', b'not media', {'X-Nuzky': '1'})
    r.check('a file that is not media is refused, saying why', status == 400 and b'videos, photos and sound' in said, [status, said])

    # Two videos from a phone, the second of them HEVC with a variable frame rate, as an iPhone films.
    r.check('the page sends a video', page.upload(FIXTURES / 'talk.mp4', 'IMG_4021.MOV') == 200)
    r.check('and another', page.upload(FIXTURES / 'phone_hevc_vfr.mov', 'IMG_4022.MOV') == 200)
    names = lambda: [next(a['name'] for a in r.state()['assets'] if a['id'] == c['assetId']) for c in r.track()]
    placed = wait(lambda: names() == ['IMG_4021.MOV', 'IMG_4022.MOV'], 60)
    clips = r.track()
    r.check('both land on the timeline in the order sent, the second right after the first', placed
            and clips[1]['startUs'] == clips[0]['startUs'] + clips[0]['durationUs'], [names(), clips])
    kept = list((r.work / 'data/nuzky/shared').glob('*/IMG_4021.MOV'))
    r.check('the video is kept in Nuzky\'s own folder under its name, with no unfinished file left',
            len(kept) == 1 and kept[0].stat().st_size == (FIXTURES / 'talk.mp4').stat().st_size
            and not list((r.work / 'data/nuzky/shared').glob('*/.nuzky-part-*')), kept)
    seen = page_state(page, lambda s: len(s['clips']) == 2)
    r.check('the page sees the two clips', seen and [c['assetId'] for c in seen['clips']] == [c['assetId'] for c in clips], seen)
    status, headers, png = page.request('GET', f"/thumb/{clips[0]['assetId']}")
    r.check('the page gets each clip\'s thumbnail', status == 200 and png.startswith(b'\x89PNG'), status)

    r.check('the page moves the playhead', page.act(action='seek', us=1_500_000) == 200
            and wait(lambda: r.s.run('return window.__nuzky.store.getState().timeUs') == 1_500_000, 10))
    # The computer draws the frame at the new playhead a moment later.
    frame = wait(lambda: (got := page.request('GET', '/frame.jpg'))[0] == 200 and got, 10)
    r.check('the page gets the preview frame', frame and frame[2].startswith(b'\xff\xd8'), frame and frame[0])
    r.check('the page cannot seek before the start', page.act(action='seek', us=-1) == 400)
    if LINUX:
        on_phone(r, sharing['url'], page)

    duration = sum(c['durationUs'] for c in clips)
    r.check('a message from the page goes to the AI', page.act(action='send', text='cut the pause') == 200)
    working = page_state(page, lambda s: s['working'] or s['lock'], 30)
    r.check('the page sees the AI at work', working, working)
    done = page_state(page, lambda s: not s['working'] and not s['lock'] and any(i['kind'] == 'run' for i in s['items']), 60)
    run = next((i for i in (done or {}).get('items', []) if i['kind'] == 'run'), None)
    r.check('the page shows the message, the answer and what the run changed', done and done['items'][0] == {'kind': 'user', 'text': 'cut the pause'}
            and run and run['label'] == 'Tighten the start' and run['changes'], done and done['items'])
    r.check('the run cut the project on the computer', sum(c['durationUs'] for c in r.track()) == duration - 2_000_000, r.track())
    sent = r.s.run("return window.__nuzky.agent.getState().items.find((i) => i.kind === 'user' && i.text === 'cut the pause') ?? null")
    # The clip selected on the computer, the one just added, is not what the page's message is about.
    r.check('the AI panel on the computer shows the message, without the selection there', sent and 'clip' not in (sent['context'] or ''), sent)
    r.shot('share-after-run')

    page.act(action='send', text='options')
    offered = page_state(page, lambda s: any(i['kind'] == 'options' and i['open'] for i in s['items']), 30)
    options = next((i for i in (offered or {}).get('items', []) if i['kind'] == 'options'), None)
    r.check('the page offers the agent\'s choices', options and options['options'] == ['The first take', 'The last take'], offered and offered['items'])
    if options:
        page.act(action='choose', id=options['id'], label='The last take')
        chosen = page_state(page, lambda s: any(i['kind'] == 'options' and i['chosen'] == 'The last take' for i in s['items']), 30)
        answer = r.s.run("return window.__nuzky.agent.getState().items.find((i) => i.kind === 'user' && i.text === 'The last take') ?? null")
        r.check('a choice on the page answers the agent, without the selection there either', chosen and answer
                and 'clip' not in (answer['context'] or ''), answer)
        page_state(page, lambda s: not s['working'], 30)

    page.act(action='send', text='slow')
    page_state(page, lambda s: s['working'], 30)
    stopped_at = time.monotonic()
    page.act(action='stop')
    stopped = page_state(page, lambda s: not s['working'] and not s['lock'], 15)
    r.check('Stop on the page stops the agent within seconds', stopped and time.monotonic() - stopped_at < 10, time.monotonic() - stopped_at)

    r.check('the page exports', page.act(action='export') == 200)
    exported = page_state(page, lambda s: s['exported'] or s['exportError'], 240)
    # macOS names its Videos folder Movies in the home and reads no user-dirs.dirs.
    target = next((r.work / ('home/Movies' if MACOS else 'videos')).glob('*.mp4'), None)
    r.check('the export goes to the Videos folder and the page sees it done', exported and exported['exported'] and target
            and exported['exported']['bytes'] == target.stat().st_size, exported)
    status, headers, video = page.request('GET', '/video?download=1', timeout=120)
    saved = r.work / 'downloaded.mp4'
    saved.write_bytes(video)
    length, codecs = ffprobe(saved)
    r.check('the page downloads that export as a file', status == 200 and 'attachment' in headers.get('content-disposition', '')
            and video == target.read_bytes() and codecs == {'h264', 'aac'}, [status, headers, codecs])
    r.check('the download lasts as long as the timeline', abs(length - exported['durationUs'] / 1e6) < 0.1, [length, exported['durationUs']])
    status, headers, part = page.request('GET', '/video', headers={'Range': 'bytes=100-199'})
    r.check('the export plays in ranges, as a phone\'s player asks', status == 206 and part == video[100:200]
            and headers.get('content-range') == f'bytes 100-199/{len(video)}', [status, headers.get('content-range')])

    r.s.run(f"document.querySelector('{BUTTON}').click()")
    r.check('the dialog says who is connected', wait(lambda: 'connected' in (r.s.run(
        "return document.querySelector('[role=dialog][aria-labelledby=share-title]')?.textContent ?? ''") or ''), 10))
    r.s.run("[...document.querySelectorAll('[role=dialog] button')].find((b) => b.textContent.trim() === 'Stop sharing').click()")
    def closed():
        try:
            page.request('GET', '/', timeout=2)
            return False
        except (ConnectionError, socket.timeout, OSError):
            return True
    r.check('Stop sharing closes the page\'s address', wait(closed, 10))


def open_page(r):
    """Sharing on, and the page's cookie, as a phone that scanned the code."""
    sharing = wait(lambda: r.s.run('return window.__nuzky.remote.getState().sharing'), 10)
    page = Page(sharing['url'])
    page.cookie = page.request('GET', f'/?k={page.key}')[1].get('set-cookie', '').split(';')[0]
    return sharing, page


@flow('share-editor', 'Edit from your phone from the home screen: the page plays, splits, undoes, redoes, deletes, duplicates '
      'and detaches the sound of clips, picks the agent and lists past chats, starts a new project in a format and opens '
      'the earlier one again, and nothing is edited while the AI edits', before=setup, home=True)
def share_editor(r):
    # The editor stays drawn under the home screen with its own button; this one is the home screen's.
    on_home = f"[...document.querySelectorAll('{BUTTON}')].find((b) => b.closest('[aria-label=Projects]'))"
    r.check('the home screen offers sharing', r.s.run(f"return !!{on_home}"))
    r.s.run(f"{on_home}.click()")
    sharing, page = open_page(r)
    r.check('it opens the dialog with the code', sharing and '/?k=' in sharing['url'])
    time.sleep(0.5)
    r.shot('share-home')
    r.s.run("window.__nuzky.remote.setState({dialog: false})")

    r.check('the page sends a video', page.upload(FIXTURES / 'talk.mp4', 'IMG_4021.MOV') == 200)
    seen = page_state(page, lambda s: len(s['clips']) == 1 and s['projects'])
    main = next((t for t in (seen or {}).get('tracks', []) if t['main']), None)
    r.check('the page gets every track, the clip with what it can do, the agents and the projects', seen and main
            and main['clips'][0]['kind'] == 'video' and main['clips'][0]['label'] == 'IMG_4021.MOV'
            and any(a['id'] == 'claude' and a['ready'] for a in seen['agents'])
            and [p['state'] for p in seen['projects']].count('open') == 1, seen)
    clip = main['clips'][0] if main else {'id': '', 'durationUs': 0}
    first_project = next(p for p in seen['projects'] if p['state'] == 'open')

    page.act(action='play')
    r.check('Play on the page plays on the computer', wait(lambda: r.s.run('return window.__nuzky.store.getState().playing'), 10))
    page.act(action='play')
    r.check('and pauses it', wait(lambda: not r.s.run('return window.__nuzky.store.getState().playing'), 10))

    page.act(action='seek', us=clip['durationUs'] // 2)
    wait(lambda: r.s.run('return window.__nuzky.store.getState().timeUs') == clip['durationUs'] // 2, 10)
    page.act(action='split', clipId=clip['id'])
    r.check('Split cuts the clip at the playhead', wait(lambda: len(r.track()) == 2, 10), r.track())
    page.act(action='undo')
    r.check('Undo puts it back', wait(lambda: len(r.track()) == 1, 10))
    page.act(action='redo')
    r.check('Redo cuts it again', wait(lambda: len(r.track()) == 2, 10))
    second = r.track()[1]['id']
    page.act(action='delete', clipId=second)
    r.check('Delete removes the clip it names', wait(lambda: [c['id'] for c in r.track()] == [clip['id']], 10), r.track())
    page.act(action='duplicate', clipId=clip['id'])
    r.check('Duplicate copies it', wait(lambda: len(r.track()) == 2, 10))
    copy = r.track()[1]
    page.act(action='seek', us=copy['startUs'] + copy['durationUs'] // 2)
    wait(lambda: r.s.run('return window.__nuzky.store.getState().timeUs') == copy['startUs'] + copy['durationUs'] // 2, 10)
    page.act(action='split', clipId=clip['id'])
    time.sleep(1)
    r.check('Split on a clip the playhead has left cuts nothing, not the clip under the playhead', len(r.track()) == 2, r.track())
    page.act(action='detachAudio', clipId=clip['id'])
    audio = lambda: [c for t in r.state()['tracks'] if t['kind'] == 'audio' for c in t['clips']]
    r.check('Detach audio moves its sound to an audio track', wait(lambda: len(audio()) == 1, 10), r.state()['tracks'])
    detached = page_state(page, lambda s: any(t['kind'] == 'audio' and t['clips'] for t in s['tracks']))
    r.check('and the page shows that track and that the clip has no sound to detach any more', detached and not next(
        c for t in detached['tracks'] if t['main'] for c in t['clips'] if c['id'] == clip['id'])['can']['detachAudio'], detached and detached['tracks'])

    r.check('a clip the project does not have changes nothing', page.act(action='delete', clipId='nope') == 200
            and len(r.track()) == 2)
    page.act(action='agent', agent='codex')
    time.sleep(1)
    r.check('an agent that is not installed cannot be chosen', r.s.run('return window.__nuzky.agent.getState().agent') == 'claude')
    page.act(action='chats')
    r.check('the page lists past chats', page_state(page, lambda s: s['chats'] is not None, 15))

    # An agent connected over MCP edits: the page must not edit beside it.
    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Phone'})['run_id']
        wait(lambda: r.state()['aiRun'], 10)
        before = len(r.track())
        page.act(action='duplicate', clipId=clip['id'])
        page.act(action='newProject', format='16:9')
        locked = page_state(page, lambda s: s['editBlocked'])
        time.sleep(1)
        r.check('while the AI edits, the page edits nothing and says why', locked and len(r.track()) == before
                and r.s.run('return window.__nuzky.store.getState().snap.project.canvas.width') == 1080
                and locked['tracks'][0]['clips'][0]['why'].get('delete'), locked and locked['editBlocked'])
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
        wait(lambda: not r.state()['aiRun'], 10)
    finally:
        bridge.close()

    page.act(action='newProject', format='16:9')
    started_new = page_state(page, lambda s: s['canvas'] == {'width': 1920, 'height': 1080} and not s['clips'], 30)
    r.check('New project in 16:9 starts an empty one and the page follows it', started_new, started_new and started_new['canvas'])
    r.check('the computer shows the editor with it', r.s.run('return window.__nuzky.store.getState().view') == 'editor')
    listed = page_state(page, lambda s: len(s['projects']) == 2, 30)
    r.check('both projects are listed, the new one open', listed and [p['id'] for p in listed['projects'] if p['state'] == 'open'] != [first_project['id']],
            listed and listed['projects'])
    page.act(action='openProject', id=first_project['id'])
    back = page_state(page, lambda s: len(s['clips']) == 2 and s['canvas']['width'] == 1080, 30)
    r.check('Open brings the earlier project back with its clips', back, back and back['canvas'])
    r.check('a name the page made up opens nothing', page.act(action='openProject', id='/etc/passwd') == 200
            and page_state(page, lambda s: len(s['clips']) == 2))
    r.check('a format Nuzky does not offer starts nothing', page.act(action='newProject', format='7:3') == 200)
    time.sleep(1)
    r.check('so the project stays open', r.s.run('return window.__nuzky.store.getState().snap.project.canvas.width') == 1080)
    if LINUX:
        tap_on_phone(r, sharing['url'])


def tap_on_phone(r, url):
    phone = Phone(r)
    try:
        phone.open(url)
        drawn = wait(lambda: phone.run("return document.querySelectorAll('#tracks .clip').length") >= 3, 30)
        r.check('on a phone the page shows every track: both video clips and the detached sound', drawn,
                phone.run("return [...document.querySelectorAll('#tracks .clip')].map((c) => c.className)"))
        r.check('the sound on its own track shows as sound, without the video\'s picture', phone.run(
            "return [...document.querySelectorAll('#tracks .clip.audio')].map((c) => c.style.backgroundImage)") == [''],
            phone.run("return [...document.querySelectorAll('#tracks .clip')].map((c) => [c.className, c.style.backgroundImage])"))
        BRIGHT = """const f = document.getElementById('frame'); if (f.hidden || !f.naturalWidth) return 0;
            const c = document.createElement('canvas'); c.width = 16; c.height = 16; const g = c.getContext('2d'); g.drawImage(f, 0, 0, 16, 16);
            return g.getImageData(0, 0, 16, 16).data.reduce((a, v, i) => (i % 4 === 3 ? a : a + v), 0) / (16 * 16 * 3)"""
        r.check('the preview shows the open project\'s picture, not black', wait(lambda: phone.run(BRIGHT) > 40, 10), phone.run(BRIGHT))
        dark = []
        for us in range(1_000_000, 6_000_000, 1_000_000):
            r.s.run('window.__nuzky.store.getState().seek(arguments[0])', us)
            for _ in range(6):
                time.sleep(0.1)
                dark += [b for b in [phone.run(BRIGHT)] if b <= 40]
        r.check('the preview never goes black while new frames come', not dark, dark)
        phone.shot(r.work / 'phone-editor.png')
        r.shots.append('phone-editor.png')
        main = r.track()

        bridge = Bridge(r, r.saved_project())
        try:
            run = bridge.call('begin_run', {'label': 'Phone'})['run_id']
            wait(lambda: phone.shown('#lock'), 10)
            phone.tap(f".clip[data-id='{main[0]['id']}']")
            time.sleep(0.5)
            r.check('while an agent edits, tapping a clip keeps the AI bar in view', phone.shown('#lock') and not phone.shown('#clipbar'))
            bridge.call('end_run', {'run_id': run, 'action': 'keep'})
            wait(lambda: not phone.shown('#lock'), 10)
        finally:
            bridge.close()
        phone.tap(f".clip[data-id='{main[0]['id']}']")
        r.check('tapping a clip swaps the AI for its actions', wait(lambda: phone.shown('#clipbar') and not phone.shown('#text'), 5))
        phone.shot(r.work / 'phone-clip.png')
        r.shots.append('phone-clip.png')
        phone.tap(".act[data-act='duplicate']")
        r.check('Duplicate there copies the clip on the computer', wait(lambda: len(r.track()) == len(main) + 1, 10), r.track())
        phone.tap('#clip-done')
        phone.tap('#dock .agentchip')
        r.check('the agent chip opens the agents with the model and effort', wait(lambda: phone.shown('#agents')
                and 'Claude Code' in phone.run("return document.getElementById('agents').textContent"), 5))
        phone.shot(r.work / 'phone-agents.png')
        r.shots.append('phone-agents.png')
        phone.tap('#agents .sheet-head .pill')
        phone.tap('#projects')
        # The empty 16:9 project went away once nobody had it open, as the home screen drops it.
        r.check('the projects button lists the projects and which is open', wait(lambda: phone.shown('#projects-sheet')
                and 'Open now' in phone.run("return document.getElementById('projects-sheet').textContent"), 5))
        phone.shot(r.work / 'phone-projects.png')
        r.shots.append('phone-projects.png')
        phone.tap("[aria-label='New 1:1 project']")
        r.check('a format there starts a new project on the computer', wait(
            lambda: r.s.run('const c = window.__nuzky.store.getState().snap.project.canvas; return [c.width, c.height]') == [1080, 1080], 30))
        r.check('the empty project shows no time left over from the last one', wait(
            lambda: phone.run("return document.getElementById('time-now').textContent + ' / ' + document.getElementById('time-total').textContent") == '0:00.0 / 0:00.0', 10),
            phone.run("return document.getElementById('time-now').textContent + ' / ' + document.getElementById('time-total').textContent"))
        r.s.call("window.__nuzky.store.getState().edit({type: 'addText', startUs: 0, text: 'Only text', style: {fontSize: 80, color: '#ffffff'}})")
        r.check('a project with only text on it shows its timeline and can play', wait(lambda: phone.shown('#tl')
                and phone.run("return !document.getElementById('play').disabled"), 10))
    finally:
        phone.close()

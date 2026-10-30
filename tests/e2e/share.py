import http.client, json, socket, time
from pathlib import Path
from urllib.parse import parse_qs, quote, urlsplit

from e2e.harness import FIXTURES, ffprobe, flow, wait
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
    status, headers, jpeg = page.request('GET', '/frame.jpg')
    r.check('the page gets the preview frame', status == 200 and jpeg.startswith(b'\xff\xd8'), status)
    r.check('the page cannot seek before the start', page.act(action='seek', us=-1) == 400)

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
        r.check('a choice on the page answers the agent', chosen and r.s.run(
            "return window.__nuzky.agent.getState().items.some((i) => i.kind === 'user' && i.text === 'The last take')"), chosen and chosen['items'])
        page_state(page, lambda s: not s['working'], 30)

    page.act(action='send', text='slow')
    page_state(page, lambda s: s['working'], 30)
    stopped_at = time.monotonic()
    page.act(action='stop')
    stopped = page_state(page, lambda s: not s['working'] and not s['lock'], 15)
    r.check('Stop on the page stops the agent within seconds', stopped and time.monotonic() - stopped_at < 10, time.monotonic() - stopped_at)

    r.check('the page exports', page.act(action='export') == 200)
    exported = page_state(page, lambda s: s['exported'] or s['exportError'], 240)
    target = next((r.work / 'videos').glob('*.mp4'), None)
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

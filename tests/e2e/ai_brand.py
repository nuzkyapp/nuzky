"""A brand deal as the maintainer asked for it: in a talking head with captions, the user tells the AI panel "find the
logo of the company I'm talking about, put it into the video and make the captions in its colours". Claude Code (the
fake agent) searches the web, gets the brand's page from a stand-in for its site, then the logo the page lists first,
reads the logo's colours with inspect_image, puts the logo in while the brand is talked about and recolours every
caption in one run; one Undo on the run card takes all of it back. Checked in the panel, the saved project and the
preview's pixels."""
import io, json, subprocess, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import numpy as np
from PIL import Image

from e2e.ai_panel import FAKE, MESSAGES, UNFOLD, agent, click, panel, send
from e2e.caption_text import REEL
from e2e.harness import CLI, FIXTURES, changed_share, flow, preview_crop, preview_rect, wait, webdriver
from e2e.stickers import bounds

GREEN, DARK = '#1db954', '#191414'
# Acme's logo on transparency, two shapes in each colour and no text.
LOGO = f"""<svg xmlns="http://www.w3.org/2000/svg" width="400" height="200" viewBox="0 0 400 200">
<circle cx="100" cy="100" r="92" fill="{GREEN}"/><path d="M100 36 L152 164 H126 L100 96 L74 164 H48 Z" fill="{DARK}"/>
<rect x="212" y="58" width="176" height="32" rx="16" fill="{DARK}"/><rect x="212" y="110" width="124" height="32" rx="16" fill="{GREEN}"/>
</svg>"""
PAGE = """<!DOCTYPE html><html><head><title>Acme</title><meta property="og:image" content="/assets/acme-share.jpg">
<link rel="icon" href="/favicon.png"></head><body><header><img class="site-logo" src="/assets/acme-logo.svg" alt="Acme"></header>
<p>Acme makes the tools.</p></body></html>"""
ASK = "Find the logo of the company I'm talking about, put it into the video and make the captions in its colours"
TEXTS = ['This video is', 'sponsored by Acme', 'link in the bio']
# Through the app's own edit, 2 s each, so "sponsored by Acme" shows from 2 s to 4 s.
ADD = """window.__nuzky.store.getState().edit({type: 'addCaptions', style: arguments[0],
  segments: arguments[1].map((text, i) => ({startUs: i * 2000000, endUs: (i + 1) * 2000000, text}))})"""
SHOWS, BEFORE = 3_000_000, 1_000_000
# Each step row's text and the detail its tooltip keeps when repeats of the step share one row.
ROWS = """return [...document.querySelectorAll('aside[aria-label=AI] ol[aria-label=Steps] li')].map((li) =>
  [li.innerText.replace(/\\s+/g, ' ').trim(), li.querySelector('span[title]')?.title ?? null])"""
CAPTIONS = """return window.__nuzky.store.getState().snap.project.tracks.find((t) => t.name === 'Captions')?.clips.length ?? 0"""


def picture(size, colour, kind):
    data = io.BytesIO()
    Image.new('RGB', size, colour).save(data, kind)
    return data.getvalue()


class Site:
    """Acme's site: its home page with the logo, a share picture and an icon, and every request it got."""

    def __init__(self):
        self.requests = []
        files = {'/acme': ('text/html; charset=utf-8', PAGE.encode()), '/assets/acme-logo.svg': ('image/svg+xml', LOGO.encode()),
                 '/assets/acme-share.jpg': ('image/jpeg', picture((1200, 630), (240, 120, 30), 'JPEG')),
                 '/favicon.png': ('image/png', picture((64, 64), (29, 185, 84), 'PNG'))}
        site = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                site.requests.append(self.path)
                kind, body = files.get(self.path, ('text/plain', b''))
                self.send_response(200 if body else 404)
                self.send_header('Content-Type', kind)
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass

        self.http = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.base = f'http://127.0.0.1:{self.http.server_port}'
        threading.Thread(target=self.http.serve_forever, daemon=True).start()

    def stop(self):
        self.http.shutdown()
        self.http.server_close()


def setup(r):
    if not (FIXTURES / 'talking-head.mov').exists():
        raise RuntimeError('tmp-test/talking-head.mov is missing: run scripts/fixtures.sh')
    project = r.work / 'data/nuzky/projects/brand.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(FIXTURES / 'talking-head.mov')], env=r.env, check=True, capture_output=True)
    r.site = Site()
    r.env.update(NUZKY_AGENT_CLAUDE=str(FAKE), NUZKY_AGENT_CODEX=str(r.work / 'no-codex'), NUZKY_FAKE_AGENT_LOG=str(r.work / 'fake'),
                 NUZKY_WEB_TEST_URL=r.site.base)


def saved(r):
    return json.loads(r.saved_project().read_text())


def captions(project):
    return [c for t in project['tracks'] if t['name'] == 'Captions' for c in t['clips']]


def timing(project):
    return [(c['content']['text'], c['startUs'], c['durationUs']) for c in captions(project)]


def colours(project):
    return {(c['content']['style']['color'], c['content']['style']['strokeColor']) for c in captions(project)}


def preview(r, name, at_us, until=lambda crop: True):
    """The preview at `at_us`, shot again until `until` holds or 5 s pass, as a frame can trail the seek."""
    r.seek(at_us)
    time.sleep(1)

    def shoot():
        r.shot(name)
        return preview_crop(r.work / f'{name}.png', preview_rect(r))
    crop = shoot()
    end = time.time() + 5
    while not until(crop) and time.time() < end:
        time.sleep(0.5)
        crop = shoot()
    return crop


def box(crop, corners, canvas):
    k = crop.width / canvas['width']
    xs, ys = [x * k for x, _ in corners], [y * k for _, y in corners]
    return tuple(round(v) for v in (min(xs), min(ys), max(xs), max(ys)))


def green(crop, corners, canvas):
    """The share of the box in the brand's green; the talking head has none of it."""
    a = np.asarray(crop.crop(box(crop, corners, canvas)), dtype=int)
    return float(((abs(a[..., 0] - 0x1d) < 40) & (abs(a[..., 1] - 0xb9) < 40) & (abs(a[..., 2] - 0x54) < 40)).mean())


@flow('ai_panel_brand', 'A brand deal: the AI panel finds the logo of the company the user talks about on the web, puts it '
      'in while the brand is talked about and colours the captions in its colours, and one Undo takes it all back', before=setup)
def ai_panel_brand(r):
    site = r.site
    try:
        r.check('the talking head gets three white captions', r.s.call(ADD, REEL, TEXTS)['ok'] and wait(lambda: r.s.run(CAPTIONS) == 3, 5))
        before = wait(lambda: (p := saved(r)) and len(captions(p)) == 3 and p, 5)
        canvas = before['canvas']
        r.s.run('document.body.focus()')
        r.key('j', ctrlKey=True)
        wait(lambda: panel(r) and r.s.run('return window.__nuzky.agent.getState().agents'), 10)
        r.s.run('window.__nuzky.store.getState().select([])')
        plain = preview(r, 'before-run', SHOWS)
        send(r, ASK)
        wait(lambda: agent(r)['status'] == 'idle' and 'run' in agent(r)['kinds'], 60)
        r.s.run(UNFOLD)
        host = site.base.split('/')[2]
        rows = wait(lambda: (lambda rows: rows if ['Looking at a picture', None] in rows else None)(r.s.run(ROWS)), 5) or r.s.run(ROWS)
        answer = f'Hotovo: logo Acme z {host} je ve videu od 2 do 5 s a titulky mají jeho barvu {GREEN}.'
        r.check('the panel shows the web search with its query, getting the picture from its site and looking at it, and the turn '
                'ends with the answer and no error',
                ['Searching the web Acme logo svg', 'Acme logo svg'] in rows and ['Looking at a picture', None] in rows
                and ['Getting a picture from the web 2 times', host] in rows
                and 'error' not in agent(r)['kinds'] and r.s.run(MESSAGES)[-1:] == [answer] and not r.errors(),
                {'rows': rows, 'kinds': agent(r)['kinds'], 'messages': r.s.run(MESSAGES), 'errors': r.errors()})
        r.check('Nuzky got the page, then only the logo it lists first', site.requests == ['/acme', '/assets/acme-logo.svg'], site.requests)

        after = wait(lambda: (p := saved(r)) and colours(p) != colours(before) and p, 5) or saved(r)
        new = [a for a in after['assets'] if a['id'] not in {b['id'] for b in before['assets']}]
        logo = new[0] if len(new) == 1 else None
        kept = (r.work / 'data/nuzky/web-images').resolve()
        r.check("the project has one new picture, the SVG drawn 2048 px wide in Nuzky's own web-images folder",
                logo and logo['kind'] == 'image' and Path(logo['path']).resolve().parent == kept
                and (logo['width'], logo['height']) == (2048, 1024) == Image.open(logo['path']).size, new)
        placed = [(t['id'], t['kind'], c['id'], c['startUs'], c['durationUs']) for t in after['tracks'] for c in t['clips']
                  if logo and c['content'].get('assetId') == logo['id']]
        r.check('it shows from 2 s for 3 s on a track over the video', len(placed) == 1 and placed[0][0] != 'main'
                and placed[0][1] == 'video' and placed[0][3:] == (2_000_000, 3_000_000), placed)
        r.check("every caption is in the logo's green, outlined in black, with its text and timing as before",
                colours(after) == {(GREEN, '#000000')} and timing(after) == timing(before), captions(after))

        sticker = (bounds(r, placed[0][2], SHOWS) or {}).get('corners') if placed else None
        line = bounds(r, next(c['id'] for c in captions(after) if c['content']['text'] == TEXTS[1]), SHOWS)['corners']
        w, h = canvas['width'], canvas['height']
        safe = (w * 60 / 1080, h * 250 / 1920, w - w * 180 / 1080, h - h * 500 / 1920)
        r.check("the logo sits inside the reel's safe area, clear of the caption under it",
                sticker and all(safe[0] <= x <= safe[2] and safe[1] <= y <= safe[3] for x, y in sticker)
                and max(y for _, y in sticker) < min(y for _, y in line), {'logo': sticker, 'caption': line, 'safe': safe})
        r.s.run('window.__nuzky.store.getState().select([])')
        shown = preview(r, 'brand-1440', SHOWS, lambda crop: not sticker or green(crop, sticker, canvas) > 0.15)
        r.check("the preview shows the logo's green where the logo is and in the caption, where there was none before",
                sticker and green(shown, sticker, canvas) > 0.15 and green(shown, line, canvas) > 0.03
                and green(plain, sticker, canvas) < 0.01 and green(plain, line, canvas) < 0.005,
                sticker and {'logo': green(shown, sticker, canvas), 'caption': green(shown, line, canvas),
                             'before run': [green(plain, sticker, canvas), green(plain, line, canvas)]})
        early = preview(r, 'brand-before-start', BEFORE, lambda crop: not sticker or green(crop, sticker, canvas) < 0.01)
        r.check('before 2 s the logo is not there', sticker and green(early, sticker, canvas) < 0.01, sticker and green(early, sticker, canvas))

        webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
        wait(lambda: panel(r)['windowWidth'] <= 1030 and panel(r)['slot'] == 'inspector', 5)
        r.s.run(UNFOLD)
        small = preview(r, 'brand-1024', SHOWS, lambda crop: not sticker or green(crop, sticker, canvas) > 0.15)
        r.check('in the smallest window the preview shows the logo too', sticker and green(small, sticker, canvas) > 0.15,
                sticker and green(small, sticker, canvas))
        webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
        wait(lambda: panel(r)['windowWidth'] >= 1430, 5)

        click(r, 'Undo')
        undone = wait(lambda: (p := saved(r)) and colours(p) == colours(before) and p, 10) or saved(r)
        r.check('one Undo on the run card takes the picture and the logo away and gives the captions their white back',
                colours(undone) == {('#ffffff', '#000000')} and [a['id'] for a in undone['assets']] == [a['id'] for a in before['assets']]
                and not any(logo and c['content'].get('assetId') == logo['id'] for t in undone['tracks'] for c in t['clips'])
                and timing(undone) == timing(before), {'assets': undone['assets'], 'captions': captions(undone)})
        back = preview(r, 'undone', SHOWS, lambda crop: not sticker or green(crop, sticker, canvas) < 0.01)
        same = lambda corners: changed_share(back.crop(box(back, corners, canvas)), plain.crop(box(plain, corners, canvas)))
        r.check('and the preview is as before the run where the logo and the caption were', sticker and same(sticker) < 0.01 and same(line) < 0.01,
                sticker and {'logo changed': same(sticker), 'caption changed': same(line)})
        r.check('no error toast along the way', not r.errors(), r.errors())
    finally:
        site.stop()

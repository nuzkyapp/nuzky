"""Your style and My caption styles follow the Nuzky account between computers, against the real account server: this
computer's own style and caption style reach the account when it signs in, another computer's changes arrive here,
caption styles both computers changed meanwhile all stay, a style changed on both keeps the newer change, and a signed
out computer leaves the account alone. The other computer is this flow, signed in to the same account over HTTP."""
import base64, hashlib, json, secrets, time, urllib.error, urllib.parse, urllib.request

from e2e.account import EMAIL, PASSWORD, browse, click, close_settings, needs, open_account, setup, shot, view
from e2e.harness import ROOT, flow, wait
from e2e.home import resize
from e2e.style import answer

PRESET = json.loads((ROOT / 'assets/presets/captions.json').read_text())[0]
MINE = '# How I edit\n\n## Your rules\n- Keep every laugh.\n'
THEIRS = '# How I edit\n\n## Your rules\n- Keep every laugh.\n- Never cut a sentence in half.\n'
NEWER_HERE = '# How I edit\n\n## Your rules\n- Keep every laugh.\n- Start on the first word.\n'


class Stay(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


class OtherComputer:
    """Nuzky on another computer, signed in to the same account as the app."""

    def __init__(self, server):
        self.server = server
        opener = urllib.request.build_opener(Stay)
        verifier = secrets.token_urlsafe(32)
        challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b'=').decode()
        redirect = 'http://127.0.0.1:9/auth/callback'
        query = urllib.parse.urlencode({
            'response_type': 'code', 'client_id': 'nuzky-desktop', 'redirect_uri': redirect, 'state': 'other',
            'scope': 'openid email profile offline_access nuzky', 'nonce': 'other', 'code_challenge': challenge,
            'code_challenge_method': 'S256'})
        try:
            opener.open(f'{server}/api/auth/oauth2/authorize?{query}', timeout=10)
            raise RuntimeError('authorize did not send the browser to sign in')
        except urllib.error.HTTPError as error:
            sign_in = urllib.parse.urlsplit(error.headers['Location']).query
        signed_in = self.request('POST', '/api/auth/sign-in/email', {'email': EMAIL, 'password': PASSWORD, 'oauth_query': sign_in},
                                 {'Origin': server})
        code = urllib.parse.parse_qs(urllib.parse.urlsplit(signed_in['url']).query)['code'][0]
        form = urllib.parse.urlencode({'grant_type': 'authorization_code', 'code': code, 'redirect_uri': redirect,
                                       'client_id': 'nuzky-desktop', 'code_verifier': verifier}).encode()
        with urllib.request.urlopen(f'{server}/api/auth/oauth2/token', form, timeout=10) as response:
            self.token = json.loads(response.read())['access_token']

    def request(self, method, path, body=None, headers=None):
        request = urllib.request.Request(self.server + path, json.dumps(body).encode() if body is not None else None, method=method,
                                         headers={'Content-Type': 'application/json', 'Accept': 'application/json', **(headers or {})})
        try:
            with urllib.request.urlopen(request, timeout=10) as response:
                return json.loads(response.read())
        except urllib.error.HTTPError as error:
            return {'status': error.code, **json.loads(error.read() or b'{}')}

    def read(self):
        return self.request('GET', '/api/auth/sync', headers={'Authorization': f'Bearer {self.token}'})['documents']

    def write(self, kind, body):
        revision = self.read()[kind]['revision']
        answer = self.request('PUT', f'/api/auth/sync/{kind}', {'revision': revision, 'body': body},
                              {'Authorization': f'Bearer {self.token}'})
        if 'revision' not in answer or 'status' in answer:
            raise RuntimeError(f'the other computer could not store {kind}: {answer}')

    def style(self):
        return (self.read()['style'].get('body') or {}).get('text')

    def captions(self):
        return [s['name'] for s in (self.read()['caption-styles'].get('body') or {}).get('styles', [])]


def caption(name, color='#ffffff'):
    return {**PRESET, 'name': name, 'style': {**PRESET['style'], 'color': color}}


def edit_md(r):
    path = r.work / 'data/nuzky/EDIT.md'
    return path.read_text() if path.exists() else None


def mine(r):
    return [s['name'] for s in answer(r, 'window.__nuzky.api.captionStyles()')]


def set_style(r, text):
    version = answer(r, 'window.__nuzky.api.styleView()')['version']
    answer(r, 'window.__nuzky.api.styleAct(arguments[0])', {'type': 'setText', 'text': text, 'baseVersion': version})


def synced_line(r):
    text = view(r).get('text', '')
    return 'Your style and caption styles' in text and 'Synced just now' in text


@flow('account-sync', 'Your style and caption styles follow the Nuzky account between computers: what this computer has reaches the '
      "account on signing in, another computer's changes arrive, caption styles changed on both all stay, a style changed on "
      'both keeps the newer change and a signed out computer leaves the account alone', before=setup, needs=needs)
def account_sync(r):
    try:
        resize(r, 1440, 900)
        set_style(r, MINE)
        r.check('this computer has a caption style of its own',
                r.s.call('window.__nuzky.api.saveCaptionStyle(arguments[0])', caption('Mine'))['ok'] and mine(r) == ['Mine'])

        open_account(r)
        browse(r, 'signup')
        click(r, 'Continue with email')
        r.check('signing up signs Nuzky in', wait(lambda: EMAIL in view(r).get('text', '') and 'Sign out' in view(r)['text'], 20), view(r))
        other = OtherComputer(r.endpoints['url'])
        r.check("signing in sends this computer's style and caption style to the account",
                wait(lambda: other.style() == MINE and other.captions() == ['Mine'], 15), other.read())
        r.check('Settings › Account says they synced just now', wait(lambda: synced_line(r), 10), view(r))
        shot(r, 'synced')

        other.write('style', {'text': THEIRS, 'accepted': {}, 'frozen': {}, 'atMs': int(time.time() * 1000)})
        other.write('caption-styles', {'styles': [caption('Mine'), caption('Team yellow', '#ffd400')]})
        click(r, 'Sync now')
        r.check("Sync now brings the other computer's style into EDIT.md", wait(lambda: edit_md(r) == THEIRS, 15), edit_md(r))
        versions = answer(r, 'window.__nuzky.api.styleView()')['versions']
        r.check('the style it replaced stays in Versions under the one from another computer',
                [v['label'] for v in versions[:2]] == ['From another computer', 'Edited by hand'], versions[:3])
        r.check("and the other computer's caption style joins this computer's", wait(lambda: mine(r) == ['Mine', 'Team yellow'], 10), mine(r))
        close_settings(r)
        r.s.run("[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === 'Captions')?.click()")
        tiles = lambda: r.s.run("return [...document.querySelectorAll('[data-style-tile]')].map((b) => b.textContent.trim())")
        r.check('the Captions tab shows the synced style', wait(lambda: 'Team yellow' in (tiles() or []), 10), tiles())
        wait(lambda: r.s.run('return document.fonts.status') == 'loaded', 10)
        r.s.run("[...document.querySelectorAll('[data-style-tile]')].find((b) => b.textContent.trim() === 'Team yellow')"
                ".scrollIntoView({block: 'center'})")
        shot(r, 'captions-tab')

        r.check('a caption style saved here', r.s.call('window.__nuzky.api.saveCaptionStyle(arguments[0])', caption('Mine 2'))['ok'])
        other.write('caption-styles', {'styles': [caption('Mine'), caption('Team yellow', '#ffd400'), caption('Theirs 2')]})
        r.check("and one saved on the other computer meanwhile both end up on both computers",
                wait(lambda: sorted(other.captions()) == ['Mine', 'Mine 2', 'Team yellow', 'Theirs 2']
                     and sorted(mine(r)) == ['Mine', 'Mine 2', 'Team yellow', 'Theirs 2'], 20), {'here': mine(r), 'there': other.captions()})

        other.write('style', {'text': THEIRS + '- An older change.\n', 'accepted': {}, 'frozen': {}, 'atMs': int(time.time() * 1000) - 60_000})
        set_style(r, NEWER_HERE)
        r.check('when both changed the style, the newer change here wins on both computers',
                wait(lambda: other.style() == NEWER_HERE, 15) and edit_md(r) == NEWER_HERE, {'here': edit_md(r), 'there': other.style()})

        open_account(r)
        click(r, 'Sign out')
        wait(lambda: 'Continue with email' in view(r).get('text', ''), 10)
        other.write('style', {'text': THEIRS, 'accepted': {}, 'frozen': {}, 'atMs': int(time.time() * 1000)})
        set_style(r, MINE)
        time.sleep(5)
        r.check('signed out, Nuzky neither sends nor takes changes', other.style() == THEIRS and edit_md(r) == MINE,
                {'here': edit_md(r), 'there': other.style()})
        close_settings(r)
    finally:
        if r.server.poll() is None:
            r.server.terminate()
            r.server.wait(timeout=10)

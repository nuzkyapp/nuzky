"""The optional Nuzky account against the real account server (account/, its end-to-end mode with a mock Google and
mail written to files): Settings › Account signs up by email with the mailed code and in with Google in the browser,
keeps the tokens out of the page and the logs, renews them, outlives a restart, signs out on the server too, ends the
sign-in of a deleted account and stays signed in while the server cannot be reached."""
import json, shutil, subprocess, time, urllib.error, urllib.parse, urllib.request
from pathlib import Path

from e2e.harness import ROOT, Session, close_window, flow, leave_editor, wait
from e2e.home import resize

SERVER = ROOT / 'account/.next/standalone/server.js'
EMAIL, PASSWORD = 'creator@example.test', 'cutting-reels-42'
GOOGLE = 'friend@example.test'
PANE = '[role=dialog][data-settings] [data-setting=account]'
VIEW = """const d = document.querySelector(arguments[0]);
return d && {text: d.innerText, alert: d.querySelector('[role=alert]')?.textContent ?? null,
             focused: document.activeElement?.textContent?.trim(),
             tab: document.querySelector('#settings-tab-account')?.innerText ?? null};"""
CLICK = """const d = document.querySelector(arguments[0]);
const b = d && [...d.querySelectorAll('button')].find((b) => b.textContent.trim() === arguments[1]);
if (!b || b.getAttribute('aria-disabled') === 'true') return false;
b.click(); return true;"""


def needs():
    return None if SERVER.exists() else 'the account server is not built: run npm ci and npm run build in account/'


def setup(r):
    data = r.work / 'account-server'
    log = (r.work / 'account-server.log').open('w')
    r.server = subprocess.Popen(['node', str(ROOT / 'account/scripts/e2e-server.mjs'), '--data', str(data),
                                 # Every renewal check finds the token within its last five minutes.
                                 '--access-token-seconds', '60'],
                                stdout=subprocess.PIPE, stderr=log, text=True)
    r.endpoints = json.loads(r.server.stdout.readline())
    r.env.update(NUZKY_ACCOUNT_ISSUER=r.endpoints['url'],
                 NUZKY_TEST_BROWSER=str(Path(__file__).with_name('fake_account_browser')),
                 NUZKY_FAKE_ACCOUNT_PLAN=str(r.work / 'browser-plan.json'),
                 NUZKY_FAKE_ACCOUNT_VISITS=str(r.work / 'browser-visits.jsonl'),
                 NUZKY_FAKE_BROWSER_PAGE=str(r.work / 'browser.html'))


def browse(r, mode, email=EMAIL, password=PASSWORD):
    """What the next sign-in does in the browser."""
    plan = {'mode': mode, 'email': email, 'password': password, 'server': r.endpoints['url'], 'mail': r.endpoints['mail']}
    (r.work / 'browser-plan.json').write_text(json.dumps(plan))
    (r.work / 'browser.html').unlink(missing_ok=True)
    (r.work / 'browser-visits.jsonl').unlink(missing_ok=True)


def visits(r):
    path = r.work / 'browser-visits.jsonl'
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []


def view(r):
    return r.s.run(VIEW, PANE) or {}


def click(r, label):
    if not r.s.run(CLICK, PANE, label):
        raise RuntimeError(f'{label} is not available in Settings › Account')


def open_account(r):
    r.s.run("document.querySelector('header button[aria-label^=\"Settings\"]').click()")
    if not wait(lambda: r.s.run("return !!document.querySelector('#settings-tab-account')"), 5):
        raise RuntimeError('Settings shows no account at the top of its panes')
    r.s.run("document.querySelector('#settings-tab-account').click()")
    if not wait(lambda: (t := view(r).get('text')) and 'Checking your sign-in' not in t, 15):
        raise RuntimeError('Settings › Account did not finish checking the sign-in')


def close_settings(r):
    r.key('Escape')
    wait(lambda: not r.s.run("return !!document.querySelector('[role=dialog][data-settings]')"), 3)


def stored(r):
    path = r.work / 'account-keychain.json'
    return json.loads(path.read_text()) if path.exists() else {}


def registration(r):
    return (r.work / 'data/nuzky/account.json').read_text()


def check_private(r, name):
    tokens = stored(r)
    page = r.s.run('return document.documentElement.outerHTML + JSON.stringify({...localStorage})')
    logs = ''.join(p.read_text(errors='replace') for p in r.work.glob('*.log') if p.name.startswith(('app', 'driver')))
    leaked = [key for key in ('access_token', 'refresh_token') if tokens.get(key)
              and any(tokens[key] in surface for surface in (page, registration(r), logs))]
    r.check(name, tokens and not leaked, {'leaked': leaked, 'stored': sorted(tokens)})


def settled(r):
    """The refresh token once syncing, which renews it, has stopped for a while."""
    last, since, deadline = None, time.time(), time.time() + 30
    while time.time() - since < 4 and time.time() < deadline:
        now = stored(r).get('refresh_token')
        if now != last or 'Syncing' in view(r).get('text', ''):
            last, since = now, time.time()
        time.sleep(0.2)
    return last


def refresh_refused(r, refresh_token):
    """Whether the server refuses `refresh_token` as an app would send it."""
    form = urllib.parse.urlencode({'grant_type': 'refresh_token', 'client_id': 'nuzky-desktop', 'refresh_token': refresh_token})
    try:
        urllib.request.urlopen(r.endpoints['url'] + '/api/auth/oauth2/token', form.encode(), timeout=10)
        return False
    except urllib.error.HTTPError as error:
        return error.code == 400 and json.loads(error.read()).get('error') == 'invalid_grant'


def delete_in_browser(r):
    """Delete account on the account's page, with the browser's own session."""
    jar = json.loads((r.work / 'fake-account-cookies.json').read_text())
    request = urllib.request.Request(r.endpoints['url'] + '/api/auth/delete-user', b'{}', method='POST', headers={
        'Content-Type': 'application/json', 'Accept': 'application/json', 'Origin': r.endpoints['url'],
        'Cookie': '; '.join(f'{k}={v}' for k, v in jar.items())})
    with urllib.request.urlopen(request, timeout=10) as response:
        return response.status


def shot(r, name):
    time.sleep(0.5)  # The off-screen window paints the pane before capture.
    r.shot(name)


@flow('account', 'Settings › Account signs up by email with the mailed code and in with Google through the real account '
      'server, keeps the tokens private, renews them, outlives a restart, signs out on the server, ends a deleted '
      "account's sign-in and stays signed in offline", before=setup, needs=needs)
def account(r):
    try:
        resize(r, 1440, 900)
        open_account(r)
        r.check('the account sits at the top of the panes as Sign in, Nuzky account',
                (view(r).get('tab') or '').split('\n') == ['Sign in', 'Nuzky account'], view(r))
        r.check('signed out, the pane offers Continue with Google and Continue with email',
                all(w in view(r)['text'] for w in ('Sign in to Nuzky', 'Continue with Google', 'Continue with email', 'Both open your browser.'))
                and not stored(r), view(r))
        shot(r, 'signed-out')
        close_settings(r)
        r.s.run("document.querySelector('header button[aria-label^=\"Settings\"]').click()")
        wait(lambda: r.s.run("return !!document.querySelector('[data-settings] input[type=search]')"), 5)
        r.s.run("document.querySelector('[data-settings] input[type=search]').focus()")
        r.s.run("""const field = document.querySelector('[data-settings] input[type=search]');
            Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(field, 'sign in');
            field.dispatchEvent(new Event('input', {bubbles: true}));""")
        r.check('searching "sign in" finds the Nuzky account',
                wait(lambda: r.s.run("return [...document.querySelectorAll('[data-settings] [data-results] button')].map((b) => b.innerText.split('\\n')[0])")[:1] == ['Nuzky account'], 5),
                r.s.run("return [...document.querySelectorAll('[data-settings] [data-results] button')].map((b) => b.innerText)"))
        r.key('Enter')
        r.check('Enter on it shows Settings › Account with Continue with Google on the keyboard',
                wait(lambda: view(r).get('focused') == 'Continue with Google', 5), view(r))
        time.sleep(2.6)  # The light on the row search found fades before the next picture.

        browse(r, 'ignore')
        click(r, 'Continue with email')
        r.check('while the browser signs in, the pane waits with Cancel',
                wait(lambda: 'Continue in your browser…' in view(r).get('text', ''), 5), view(r))
        shot(r, 'waiting')
        click(r, 'Cancel')
        r.check('Cancel goes back to the two buttons without an error or tokens',
                wait(lambda: 'Continue with email' in view(r).get('text', ''), 5) and not view(r)['alert'] and not stored(r), view(r))

        browse(r, 'signup')
        click(r, 'Continue with email')
        r.check('signing up with the mailed code signs Nuzky in as that email',
                wait(lambda: EMAIL in view(r).get('text', '') and 'Sign out' in view(r)['text'], 20), view(r))
        authorize = urllib.parse.parse_qs(urllib.parse.urlsplit(visits(r)[0]['url']).query)
        r.check('after signing in the keyboard is on Open, so Enter cannot sign out', view(r)['focused'] == 'Open', view(r))
        r.check('Nuzky asks for a code with PKCE S256 for its own client and a loopback callback, without Google or prompt',
                authorize.get('client_id') == ['nuzky-desktop'] and authorize.get('code_challenge_method') == ['S256']
                and authorize.get('scope') == ['openid email profile offline_access nuzky']
                and authorize['redirect_uri'][0].startswith('http://127.0.0.1:') and authorize['redirect_uri'][0].endswith('/auth/callback')
                and 'provider' not in authorize and 'prompt' not in authorize, authorize)
        r.check("the browser ends on the server's page saying the sign-in worked",
                "You're signed in" in (r.work / 'browser.html').read_text() and visits(r)[-1]['url'].endswith('/done'), visits(r)[-2:])
        r.check('the keychain holds exactly the access and refresh tokens and their expiry',
                set(stored(r)) == {'access_token', 'refresh_token', 'expires_at'})
        r.check('account.json keeps who signed in and no token',
                json.loads(registration(r)).get('email') == EMAIL and 'token' not in registration(r), registration(r))
        check_private(r, 'no token appears in the page, localStorage, account.json or the app log after signing in')
        r.check('the account at the top of the panes now names the email', EMAIL in (view(r).get('tab') or ''), view(r))
        shot(r, 'signed-in')

        first = stored(r)
        close_settings(r)
        open_account(r)
        r.check('showing the account again renews the token about to expire, rotating the refresh token',
                wait(lambda: stored(r).get('refresh_token') not in (None, first['refresh_token']), 10)
                and EMAIL in view(r)['text'] and not view(r)['alert'], view(r))
        r.check('renewal replaces the access token too', stored(r)['access_token'] != first['access_token'])
        check_private(r, 'no token appears in the page or the logs after renewing')

        close_settings(r)
        leave_editor(r)
        close_window()
        r.s.close()
        if (r.work / 'app.log').exists():
            shutil.copyfile(r.work / 'app.log', r.work / 'app-before-restart.log')
        r.s = Session()
        if not wait(lambda: r.s.run('return !!window.__nuzky?.store.getState().snap', retries=3), 60):
            raise RuntimeError('the app did not start again')
        r.s.run("window.__nuzky.store.setState({view: 'editor'})")
        resize(r, 1440, 900)
        open_account(r)
        r.check('after a restart Nuzky is still signed in as the email', EMAIL in view(r)['text'] and 'Sign out' in view(r)['text'], view(r))

        r.s.run("document.querySelector('#settings-tab-privacy').click()")
        privacy = lambda: r.s.run("return document.querySelector('[data-setting=privacy]')?.innerText ?? ''")
        r.check('Privacy names the Nuzky account as signed in', wait(lambda: 'Nuzky account' in privacy(), 5)
                and 'Signed in' in privacy().split('Nuzky account', 1)[1].split('Claude Code', 1)[0], privacy())
        r.check('and says Your style and caption styles sync with the account, not that they stay here',
                'Your style and caption styles sync with your Nuzky account.' in privacy()
                and 'Your style and every edit stay' not in privacy(), privacy())
        shot(r, 'privacy')
        r.s.run("document.querySelector('#settings-tab-account').click()")
        wait(lambda: 'Sign out' in view(r).get('text', ''), 5)

        spent = settled(r)
        click(r, 'Sign out')
        r.check('Sign out remembers the email and offers both ways to sign in again',
                wait(lambda: f'Last signed in as {EMAIL}.' in view(r).get('text', '') and 'Continue with Google' in view(r)['text'], 10)
                and not stored(r), view(r))
        r.check('the server no longer renews the signed-out refresh token', refresh_refused(r, spent))

        browse(r, 'google', email=GOOGLE)
        click(r, 'Continue with Google')
        r.check('Continue with Google signs in with the Google account and its name',
                wait(lambda: GOOGLE in view(r).get('text', '') and 'friend' in view(r)['text'].split(GOOGLE)[0], 20), view(r))
        google_authorize = urllib.parse.parse_qs(urllib.parse.urlsplit(visits(r)[0]['url']).query)
        r.check("Google goes to Google's chooser though the browser is signed in by email, without stopping on Nuzky's page",
                google_authorize.get('provider') == ['google'] and google_authorize.get('prompt') == ['login']
                and not any(v['status'] == 200 and urllib.parse.urlsplit(v['url']).path == '/sign-in' for v in visits(r)), visits(r))
        check_private(r, 'no token appears in the page or the logs after signing in with Google')
        shot(r, 'signed-in-google')

        r.check('Delete account on the account page succeeds', delete_in_browser(r) == 200)
        close_settings(r)
        open_account(r)
        r.check("the next check ends a deleted account's sign-in and says so",
                wait(lambda: 'Your sign-in ended. Sign in again to keep using your account.' in view(r).get('text', ''), 10)
                and f'Last signed in as {GOOGLE}.' in view(r)['text'] and not stored(r), view(r))
        shot(r, 'ended')

        browse(r, 'signin')
        click(r, 'Use another account')
        r.check('Use another account signs in again by email',
                wait(lambda: EMAIL in view(r).get('text', '') and 'Sign out' in view(r)['text'], 20), view(r))
        another = urllib.parse.parse_qs(urllib.parse.urlsplit(visits(r)[0]['url']).query)
        r.check('Use another account asks the server for the sign-in page', another.get('prompt') == ['login'] and 'provider' not in another, another)

        r.server.terminate()
        r.server.wait(timeout=10)
        before = stored(r)
        close_settings(r)
        open_account(r)
        r.check('without the account server Nuzky stays signed in, keeps its tokens and shows no error',
                EMAIL in view(r)['text'] and 'Sign out' in view(r)['text'] and not view(r)['alert'] and stored(r) == before, view(r))

        if resize(r, 1024, 640):
            shot(r, 'signed-in-small-window')
        click(r, 'Sign out')
        r.check('signing out without the server signs out here and says the server could not end the sign-in',
                wait(lambda: "Signed out on this computer, but Nuzky couldn't reach the account server" in view(r).get('text', ''), 15)
                and 'Continue with email' in view(r)['text'] and not stored(r), view(r))
        shot(r, 'signed-out-offline')
        close_settings(r)
    finally:
        if r.server.poll() is None:
            r.server.terminate()
            r.server.wait(timeout=10)

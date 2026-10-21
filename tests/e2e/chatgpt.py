"""ChatGPT account sign-in, renewal and sign-out against a local OAuth server with real RS256 tokens."""
import base64, hashlib, json, re, secrets, shutil, subprocess, threading, time, uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qsl, urlencode, urlsplit

from e2e.harness import Session, close_window, leave_editor, flow, wait

CLIENT = 'oaiapp_nuzky_e2e'
EMAIL = 'editor@example.test'
SCOPES = 'openid profile email offline_access resource.invoke chatgpt.tokens.use.direct'
RESOURCE = 'https://api.openai.com/v1'
DIALOG = '[role=dialog][aria-labelledby=chatgpt-title]'
VIEW = """const d = document.querySelector(arguments[0]);
return d && {text: d.innerText, alert: d.querySelector('[role=alert]')?.textContent ?? null,
             focused: document.activeElement?.textContent?.trim()};"""


def encoded(value):
    return base64.urlsafe_b64encode(value).decode().rstrip('=')


class Server:
    def __init__(self, work):
        self.key = work / 'signing-key.pem'
        subprocess.run(['openssl', 'genpkey', '-algorithm', 'RSA', '-pkeyopt', 'rsa_keygen_bits:2048',
                        '-out', str(self.key)], check=True, capture_output=True)
        self.key.chmod(0o600)
        modulus = subprocess.run(['openssl', 'rsa', '-in', str(self.key), '-noout', '-modulus'],
                                 check=True, capture_output=True, text=True).stdout.strip().split('=', 1)[1]
        self.jwks = {'keys': [{'kty': 'RSA', 'kid': 'e2e-key', 'alg': 'RS256', 'use': 'sig',
                              'n': encoded(bytes.fromhex(modulus)), 'e': encoded((65537).to_bytes(3, 'big'))}]}
        self.authorize, self.tokens, self.issued, self.revokes, self.errors = [], [], [], [], []
        self.denied, self.plan, self.jwks_requests = False, True, 0
        server = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                self.respond('GET')

            def do_POST(self):
                self.respond('POST')

            def respond(self, method):
                try:
                    length = int(self.headers.get('Content-Length', 0))
                    status, location, body = server.serve(method, self.path, self.rfile.read(length).decode())
                except Exception as error:
                    server.errors.append(str(error))
                    status, location, body = 400, None, {'error': 'invalid_request'}
                body = json.dumps(body).encode()
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                if location:
                    self.send_header('Location', location)
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass

        self.http = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.url = f'http://127.0.0.1:{self.http.server_port}'
        self.worker = threading.Thread(target=self.http.serve_forever, daemon=True)
        self.worker.start()

    def serve(self, method, target, body):
        url = urlsplit(target)
        if (method, url.path) == ('GET', '/api/accounts/authorize'):
            query = dict(parse_qsl(url.query))
            require(re.fullmatch(r'http://127\.0\.0\.1:[1-9][0-9]*/auth/callback', query['redirect_uri']),
                    'the redirect is a loopback callback')
            self.authorize.append(query)
            callback = {'state': query['state']}
            if self.denied:
                callback['error'] = 'access_denied'
            else:
                callback.update(code='e2e-code', scope=SCOPES)
                if query['client_id'] == 'dynamic_agent_client':
                    callback['client_id'] = CLIENT
            return 302, query['redirect_uri'] + '?' + urlencode(callback), {}
        if (method, url.path) == ('GET', '/.well-known/jwks.json'):
            self.jwks_requests += 1
            return 200, None, self.jwks
        form = dict(parse_qsl(body))
        if (method, url.path) == ('POST', '/api/accounts/oauth/token'):
            self.tokens.append(form)
            require(form['client_id'] == CLIENT and form['resource'] == RESOURCE, 'token client and resource match')
            refresh = form['grant_type'] == 'refresh_token'
            if refresh:
                require(form['refresh_token'] == self.issued[-1]['refresh_token'] and 'scope' not in form,
                        'refresh uses the current token without requesting scope')
            else:
                auth = self.authorize[-1]
                require(form['grant_type'] == 'authorization_code' and form['code'] == 'e2e-code',
                        'the authorization code is exchanged')
                require(form['redirect_uri'] == auth['redirect_uri'], 'the token redirect matches authorization')
                require(auth['code_challenge_method'] == 'S256'
                        and encoded(hashlib.sha256(form['code_verifier'].encode()).digest()) == auth['code_challenge'],
                        'the PKCE S256 verifier matches the challenge')
            now = int(time.time())
            claims = {'iss': self.url, 'aud': CLIENT, 'azp': CLIENT, 'sub': 'e2e-editor', 'email': EMAIL,
                      'exp': now + 3600, 'nonce': self.authorize[-1]['nonce']}
            signed = '.'.join(encoded(json.dumps(part).encode()) for part in
                              ({'alg': 'RS256', 'kid': 'e2e-key'}, claims))
            signature = subprocess.run(['openssl', 'dgst', '-sha256', '-sign', str(self.key)],
                                       input=signed.encode(), check=True, capture_output=True).stdout
            scope = SCOPES if self.plan else 'openid profile email offline_access resource.invoke'
            if refresh:
                scope += f' refreshed-{len(self.issued) + 1}'
            response = {'access_token': secrets.token_urlsafe(32), 'refresh_token': secrets.token_urlsafe(32),
                        'id_token': signed + '.' + encoded(signature), 'expires_in': 60, 'scope': scope}
            self.issued.append(dict(response, issued_at=now))
            return 200, None, response
        if (method, url.path) == ('POST', '/api/accounts/oauth/revoke'):
            self.revokes.append(form)
            return 200, None, {}
        raise ValueError(f'unexpected request: {method} {url.path}')


def require(ok, message):
    if not ok:
        raise ValueError(message)


def setup(r):
    r.chatgpt = Server(r.work)
    r.env.update(NUZKY_CHATGPT_ISSUER=r.chatgpt.url,
                 NUZKY_TEST_BROWSER=str(Path(__file__).with_name('fake_browser')),
                 NUZKY_FAKE_BROWSER_PAGE=str(r.work / 'browser.html'),
                 # Claude Code is the panel's agent, so the panel's ChatGPT does not ask the fake for its models.
                 NUZKY_AGENT_CLAUDE=str(Path(__file__).with_name('fake_agent')), NUZKY_FAKE_AGENT_LOG=str(r.work / 'fake'),
                 NUZKY_AGENT_CODEX=str(r.work / 'no-codex'))


def view(r):
    return r.s.run(VIEW, DIALOG) or {}


def shot(r, name):
    time.sleep(0.5)  # The off-screen window must paint the dialog before capture.
    r.shot(name)


def click(r, label):
    found = r.s.run("""const d = document.querySelector(arguments[0]);
        const b = [...d.querySelectorAll('button')].find((b) => b.textContent.trim() === arguments[1]);
        if (!b || b.getAttribute('aria-disabled') === 'true') return false;
        b.click(); return true;""", DIALOG, label)
    require(found, f'{label} is available in the dialog')


def open_dialog(r):
    # After a restart the editor renders a moment after the home screen, and Ctrl+J before it would close the panel
    # it restores, so the AI button's state decides.
    ai_button = "document.querySelector('header button[title*=\"AI (Ctrl+J)\"]')"
    require(wait(lambda: r.s.run(f'return !!{ai_button}'), 10), 'the editor is ready')
    if r.s.run(f"return {ai_button}.getAttribute('aria-pressed')") != 'true':
        r.s.run('document.activeElement?.blur()')
        r.key('j', ctrlKey=True)
    require(wait(lambda: r.s.run('return !!document.querySelector("aside[aria-label=AI]")'), 5), 'AI panel opens')
    r.s.run("""document.querySelector('aside[aria-label=AI] button[title="Choose the agent"]').click()""")
    require(wait(lambda: r.s.run('return !!document.querySelector("[role=menu][aria-label=Agent]")'), 5), 'agent menu opens')
    r.s.run("""[...document.querySelectorAll('[role=menu][aria-label=Agent] button')]
        .find((b) => b.textContent.trim() === 'ChatGPT account…').click()""")
    require(wait(lambda: view(r).get('text') and 'Checking your ChatGPT sign-in' not in view(r)['text'], 10),
            'ChatGPT dialog finishes checking the account')


def stored(r):
    path = r.work / 'keychain.json'
    return json.loads(path.read_text()) if path.exists() else {}


def check_tokens(r, name):
    tokens, issued = stored(r), r.chatgpt.issued[-1]
    r.check(name, set(tokens) == {'client_id', 'access_token', 'refresh_token', 'id_token', 'expires_at', 'scopes'}
            and all(tokens.get(k) == issued[k] for k in ('access_token', 'refresh_token', 'id_token'))
            and tokens['client_id'] == CLIENT and tokens['scopes'] == issued['scope'].split()
            and issued['issued_at'] + 60 <= tokens['expires_at'] <= int(time.time()) + 60,
            {'fields': sorted(tokens), 'responses': len(r.chatgpt.issued)})


def check_private(r, name):
    page = r.s.run('return document.documentElement.outerHTML + JSON.stringify({...localStorage})')
    registration = (r.work / 'data/nuzky/chatgpt.json').read_text()
    logs = ''.join(path.read_text() for path in r.work.glob('*.log') if path.name.startswith(('app', 'driver')))
    leaked = [key for response in r.chatgpt.issued for key in ('access_token', 'refresh_token', 'id_token')
              if any(response[key] in surface for surface in (page, registration, logs))]
    r.check(name, not leaked, {'leaked_fields': leaked})


def continue_sign_in(r):
    (r.work / 'browser.html').unlink(missing_ok=True)
    click(r, 'Continue with ChatGPT')
    require(wait(lambda: (r.work / 'browser.html').exists(), 25), 'the browser receives the callback page')


@flow('chatgpt', 'ChatGPT sign-in keeps tokens private, renews after restart, signs out and shows refused consent', before=setup)
def chatgpt(r):
    server = r.chatgpt
    try:
        open_dialog(r)
        r.check('signed out, Continue with ChatGPT is shown and focused',
                'Continue with ChatGPT' in view(r)['text'] and view(r)['focused'] == 'Continue with ChatGPT'
                and not (r.work / 'keychain.json').exists(), view(r))
        shot(r, 'signed-out')

        continue_sign_in(r)
        r.check('Continue with ChatGPT shows the email and Using ChatGPT plan',
                wait(lambda: EMAIL in view(r).get('text', '') and 'Using ChatGPT plan' in view(r)['text'], 10), view(r))
        shot(r, 'signed-in')
        auth = server.authorize[0]
        host = auth.get('ext_agent_host_id', '')
        host_valid = bool(re.fullmatch(r'urn:uuid:[0-9a-f-]{36}', host))
        if host_valid:
            host_valid = str(uuid.UUID(host[9:])) == host[9:]
        r.check('first authorization registers Nuzky with a host UUID, loopback callback, S256 and the full scope',
                len(server.authorize) == 1 and auth.get('client_id') == 'dynamic_agent_client'
                and auth.get('agent_name_hint') == 'Nuzky' and host_valid
                and bool(re.fullmatch(r'http://127\.0\.0\.1:[1-9][0-9]*/auth/callback', auth.get('redirect_uri', '')))
                and auth.get('code_challenge_method') == 'S256' and auth.get('scope') == SCOPES
                and auth.get('resource') == RESOURCE and auth.get('response_type') == 'code'
                and bool(auth.get('state')) and bool(auth.get('nonce')))
        r.check('the token endpoint accepts the code and PKCE verifier and the app reads the RS256 signing key',
                len(server.tokens) == 1 and server.tokens[0]['grant_type'] == 'authorization_code'
                and server.jwks_requests == 1 and not server.errors, server.errors)
        r.check("the browser receives You're signed in to ChatGPT", "You're signed in to ChatGPT" in (r.work / 'browser.html').read_text())
        check_tokens(r, 'the keychain file holds exactly the issued tokens, expiry and scopes')
        reg = json.loads((r.work / 'data/nuzky/chatgpt.json').read_text())
        r.check('chatgpt.json keeps the host and issued client id with no token fields',
                reg.get('host_id') == host and reg.get('client_id') == CLIENT
                and not any(key in reg for key in ('access_token', 'refresh_token', 'id_token')))
        check_private(r, 'no issued token appears in the DOM, localStorage, registration or app log after sign-in')

        first = stored(r)
        leave_editor(r)
        close_window()
        r.s.close()
        if (r.work / 'app.log').exists():
            shutil.copyfile(r.work / 'app.log', r.work / 'app-before-restart.log')
        r.s = Session()
        require(wait(lambda: r.s.run('return !!window.__nuzky?.store.getState().snap', retries=3), 60), 'the app restarts')
        r.s.run("window.__nuzky.store.setState({view: 'editor'})")
        open_dialog(r)
        # The AI panel read the stored sign-in already, so the dialog shows it at once and renews it meanwhile.
        wait(lambda: len(server.issued) == 2 and stored(r).get('refresh_token') == server.issued[1]['refresh_token'], 10)
        r.check('after restart the dialog shows the same email and Using ChatGPT plan',
                EMAIL in view(r)['text'] and 'Using ChatGPT plan' in view(r)['text'], view(r))
        r.check('opening the dialog refreshes with the first refresh token without another authorization',
                len(server.authorize) == 1 and len(server.tokens) == 2
                and server.tokens[1]['grant_type'] == 'refresh_token'
                and server.tokens[1].get('refresh_token') == first['refresh_token'] and not server.errors, server.errors)
        check_tokens(r, 'renewal replaces the stored tokens, expiry and scopes with the server response')
        r.check('renewal rotates both the refresh token and the access token',
                all(stored(r).get(key) != first[key] for key in ('refresh_token', 'access_token')))
        check_private(r, 'no issued token appears in the DOM, localStorage, registration or either app log after renewal')
        shot(r, 'renewed-after-restart')

        current = stored(r)
        click(r, 'Sign out')
        r.check('Sign out remembers the email and offers Continue with ChatGPT',
                wait(lambda: f'Last signed in as {EMAIL}.' in view(r).get('text', '')
                     and 'Continue with ChatGPT' in view(r)['text'], 10), view(r))
        r.check('Sign out revokes the current refresh token with its hint and client id and removes the keychain file',
                server.revokes == [{'token': current['refresh_token'], 'token_type_hint': 'refresh_token', 'client_id': CLIENT}]
                and not (r.work / 'keychain.json').exists())
        shot(r, 'signed-out-again')

        server.denied = True
        continue_sign_in(r)
        r.check('browser denial shows a sign-in cancelled alert and stores no tokens',
                wait(lambda: view(r).get('alert') == 'Signing in was cancelled in the browser. Continue with ChatGPT to try again.', 10)
                and not (r.work / 'keychain.json').exists() and len(server.tokens) == 2, view(r))
        shot(r, 'access-denied')

        server.denied, server.plan = False, False
        continue_sign_in(r)
        r.check("without direct plan scope the dialog says Nuzky can't use your ChatGPT plan yet",
                wait(lambda: EMAIL in view(r).get('text', '') and "Nuzky can't use your ChatGPT plan yet" in view(r)['text'], 10)
                and 'Using ChatGPT plan' not in view(r)['text'], view(r))
        check_tokens(r, 'the restricted sign-in stores only the scopes the server granted')
        check_private(r, 'no issued token appears in the DOM, localStorage, registration or app logs after refused plan use')
        shot(r, 'plan-not-allowed')
        r.key('Escape')
        r.check('Esc closes the ChatGPT dialog', wait(lambda: not view(r), 3))
        r.check('the fake server rejected no protocol request', not server.errors, server.errors)
    finally:
        server.http.shutdown()
        server.http.server_close()
        server.worker.join(timeout=5)

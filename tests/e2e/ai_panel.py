import json, os, shutil, subprocess, threading, time, uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from e2e.harness import CLI, FIXTURES, MACOS, Session, clipboard, close_window, leave_editor, flow, preview_crop, preview_rect, press, wait, webdriver
from e2e.history import alive as window_alive

FAKE = Path(__file__).resolve().parent / 'fake_agent'
# What the agents search for when asked to "search the web", as SEARCHES in fake_agent.
SEARCHES = ['Spotify logo svg', 'Spotify brand colors']
# OpenAI's web search of its own index, which every ChatGPT request offers.
SEARCH_TOOL = {'type': 'web_search', 'external_web_access': False}
PANEL = """const p = document.querySelector('aside[aria-label=AI]'); if (!p) return null; const b = p.getBoundingClientRect();
return {left: Math.round(b.left), top: Math.round(b.top), width: Math.round(b.width), right: Math.round(window.innerWidth - b.right),
        bottom: Math.round(window.innerHeight - b.bottom), text: p.innerText, slot: p.dataset.dockSlot ?? null,
        inspector: !!document.querySelector('aside[aria-label=Inspector]'), windowWidth: window.innerWidth};"""
CLICK = """const scope = arguments[1] ? document.querySelector(arguments[1]) : document;
const all = [...scope.querySelectorAll('button, [role^=menuitem]')];
const b = all.find((b) => b.textContent.trim() === arguments[0] || b.getAttribute('aria-label') === arguments[0])
  ?? all.find((b) => b.textContent.trim().startsWith(arguments[0]));
if (!b) return null; b.click(); return b.getAttribute('aria-disabled') ?? 'false';"""
CHIPS = "return [...document.querySelectorAll('aside[aria-label=AI] button[aria-label^=\"Don\\'t send\"]')].map((b) => b.getAttribute('aria-label'))"
PRESSED = "return document.querySelector('aside[aria-label=AI]').querySelector(`button[aria-label=\"${arguments[0]}\"]`)?.getAttribute('aria-pressed')"
TEXT = "return document.querySelector('aside[aria-label=AI]').querySelector(`button[aria-label=\"${arguments[0]}\"]`)?.textContent.trim()"
DRAG = """const g = document.querySelector('aside[aria-label=AI] button[aria-label="Move panel"]').getBoundingClientRect();
const at = (type, x, y) => new PointerEvent(type, {bubbles: true, button: 0, clientX: x, clientY: y, pointerId: 1});
document.querySelector('aside[aria-label=AI] button[aria-label="Move panel"]').dispatchEvent(at('pointerdown', g.left + 6, g.top + 6));
window.dispatchEvent(at('pointermove', arguments[0], arguments[1]));
if (arguments[2]) window.dispatchEvent(at('pointerup', arguments[0], arguments[1]));"""
GRIP_AT = """const g = document.querySelector('aside[aria-label=AI] button[aria-label="Move panel"]').getBoundingClientRect();
return g.left <= arguments[0] && arguments[0] <= g.right && g.top <= arguments[1] && arguments[1] <= g.bottom;"""


def setup(r):
    project = r.work / 'data/nuzky/projects/panel.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(FIXTURES / 'talk.mp4')], env=r.env, check=True, capture_output=True)
    # The fake stands in for Claude Code; Codex is deliberately missing, whatever this machine has.
    r.env.update(NUZKY_AGENT_CLAUDE=str(FAKE), NUZKY_AGENT_CODEX=str(r.work / 'no-codex'), NUZKY_FAKE_AGENT_LOG=str(r.work / 'fake'))


def runs(r):
    log = r.work / 'fake/runs.jsonl'
    return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []


def panel(r):
    return r.s.run(PANEL, retries=3)


def agent(r):
    return r.s.run("const a = window.__nuzky.agent.getState(); return {status: a.status, draft: a.draft, kinds: a.items.map((i) => i.kind)};")


def click(r, label, scope='aside[aria-label=AI]'):
    return r.s.run(CLICK, label, scope)


def send(r, text):
    """Fills the field as typing does (WebKitWebDriver cannot send keys to it), then presses Enter."""
    r.s.run("""const t = document.querySelector('aside[aria-label=AI] textarea'); t.focus();
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(t, arguments[0]);
        t.dispatchEvent(new Event('input', {bubbles: true}));""", text)
    r.key('Enter')


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


MODEL = "return document.querySelector('aside[aria-label=AI] button[title=\"Choose the model and effort\"]')?.textContent.trim()"
MODELS = "return [...document.querySelectorAll('[role=menu][aria-label=Model] [role^=menuitem]')].map((b) => b.textContent.trim())"
EFFORT = """return [...document.querySelectorAll('[role=menu][aria-label=Model] [role=group][aria-label=Effort] button')]
  .map((b) => [b.textContent.trim(), b.getAttribute('aria-pressed')])"""
MENU = '[role=menu][aria-label=Model]'


def open_models(r):
    """Focuses the button first, as a real click does, so a focus moving away would close the menu here too."""
    r.s.run("const b = document.querySelector('aside[aria-label=AI] button[title=\"Choose the model and effort\"]'); b.focus(); b.click()")


def flag(argv, name):
    return argv[argv.index(name) + 1] if name in argv else None


MESSAGES = "return window.__nuzky.agent.getState().items.filter((i) => i.kind === 'agent').map((i) => i.text)"
# The blocks of the answer that starts "Here is what went wrong:", each list as the number or bullet shown before each item.
LISTS = """const p = [...document.querySelectorAll('aside[aria-label=AI] p')].find((p) => p.textContent === 'Here is what went wrong:');
return p && [...p.parentElement.children].map((e) => e.tagName === 'P' ? e.textContent
  : [...e.children].map((li, k) => [e.tagName === 'OL' ? e.start + k : '•', li.textContent]));"""


def idle(r):
    return wait(lambda: agent(r)['status'] == 'idle', 20)


def setup_jobs(r):
    setup(r)
    # A copy of the fake, so the flow can take the agent away; an older Claude Code, which names no models.
    shutil.copy(FAKE, r.work / 'claude')
    r.env.update(NUZKY_AGENT_CLAUDE=str(r.work / 'claude'), FAKE_AGENT_OLD='1')


def setup_codex(r):
    setup(r)
    r.env.update(NUZKY_AGENT_CLAUDE=str(r.work / 'no-claude'), NUZKY_AGENT_CODEX=str(FAKE.with_name('fake_codex')))


class ChatGptPanelServer:
    def __init__(self):
        self.requests, self.errors = [], []
        server = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                if self.path != '/v1/models':
                    self.send_error(404)
                    return
                models = [{'slug': f'gpt-e2e-{name}', 'display_name': f'GPT E2E {name.title()}', 'visibility': visibility}
                          for name, visibility in [('sol', 'list'), ('luna', 'list'), ('hidden', 'hidden')]]
                models[1].update(supported_reasoning_levels=[{'effort': 'low'}, {'effort': 'high'}], default_reasoning_level='low')
                body = json.dumps({'models': models}).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_POST(self):
                if self.path != '/v1/responses':
                    self.send_error(404)
                    return
                try:
                    body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                    server.requests.append({'body': body, 'authorization': self.headers.get('Authorization')})
                    if SEARCH_TOOL not in body.get('tools', []):
                        server.errors.append(f"a request without OpenAI's web search: {body.get('tools')}")
                    events, pause = server.answer(body['input'])
                    self.send_response(200)
                    self.send_header('Content-Type', 'text/event-stream')
                    self.send_header('Connection', 'close')
                    self.end_headers()
                    time.sleep(pause)
                    for event in events:
                        self.wfile.write(f"event: {event['type']}\ndata: {json.dumps(event)}\n\n".encode())
                        self.wfile.flush()
                        if event['type'] == 'response.output_text.delta':
                            time.sleep(0.3)
                    self.close_connection = True
                except Exception as error:
                    server.errors.append(str(error))

            def log_message(self, *args):
                pass

        self.http = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.url = f'http://127.0.0.1:{self.http.server_port}'
        self.worker = threading.Thread(target=self.http.serve_forever, daemon=True)
        self.worker.start()

    def answer(self, items):
        last = max(i for i, item in enumerate(items) if item.get('role') == 'user')
        message = ' '.join(part.get('text', '') for part in items[last]['content']).lower()
        if 'limit' in message:
            return [{'type': 'response.output_text.delta', 'delta': 'Checking your plan now.'},
                    {'type': 'response.failed', 'response': {'error': {
                        'code': 'subscription_sharing_usage_limit_exceeded', 'message': 'The plan usage limit was reached.'}}}], 0
        if 'lose the connection' in message:
            # The stream breaks off while a search runs: it began, but neither it nor the answer completes.
            return [{'type': 'response.output_item.added', 'item': {'type': 'web_search_call', 'id': 'ws-lost', 'status': 'in_progress'}}], 0
        if 'search the web' in message:
            return self.search(items[last + 1:])
        if 'split' not in message:
            raise ValueError(f'unexpected user message: {message}')
        outputs = [json.loads(item['output']) for item in items[last + 1:] if item['type'] == 'function_call_output']
        count, events = len(outputs), []
        if count in (0, 4):
            text = 'I will split the clip at 2 s.' if count == 0 else 'The clip is split at 2 s.'
            events += [{'type': 'response.output_text.delta', 'delta': text},
                       {'type': 'response.output_item.done', 'item': {'type': 'message', 'id': f'msg-{count}',
                        'role': 'assistant', 'content': [{'type': 'output_text', 'text': text}]}}]
        if count < 4:
            name = ['get_state', 'begin_run', 'apply_edits', 'end_run'][count]
            arguments = {}
            if count == 1:
                arguments = {'label': 'Split the clip at 2 s'}
            elif count >= 2:
                arguments = {'run_id': outputs[1]['run_id']}
                if count == 2:
                    clip = next(t['clips'][0] for t in outputs[0]['tracks'] if t['id'] == 'main')
                    arguments.update(request_id='e2e-split', edits=[{'type': 'splitClip', 'clipId': clip['id'], 'atUs': 2_000_000}])
                else:
                    arguments['action'] = 'keep'
            events.append({'type': 'response.output_item.done', 'item': {'type': 'function_call', 'id': f'item-{count}',
                           'call_id': f'call-{count}', 'namespace': 'nuzky', 'name': name, 'arguments': json.dumps(arguments)}})
        events.append({'type': 'response.completed', 'response': {'id': f'response-{count}'}})
        return events, 1 if count in (1, 2, 3) else 0

    @staticmethod
    def search(after):
        """Searches the web twice and reads the project, then answers once the searches came back as they went, with their ids."""
        if not after:
            events = []
            for n, query in enumerate(SEARCHES):
                found = {'type': 'web_search_call', 'id': f'ws-{n}'}
                events += [{'type': 'response.output_item.added', 'item': dict(found, status='in_progress')},
                           {'type': 'response.output_item.done', 'item': dict(found, status='completed', action={'type': 'search', 'query': query})}]
            events += [{'type': 'response.output_item.done', 'item': {'type': 'function_call', 'id': 'item-read', 'call_id': 'call-read',
                                                                       'namespace': 'nuzky', 'name': 'get_state', 'arguments': '{}'}},
                       {'type': 'response.completed', 'response': {'id': 'response-search'}}]
            return events, 0
        searches = [item for item in after if item['type'] == 'web_search_call']
        if searches != [{'type': 'web_search_call', 'id': f'ws-{n}', 'status': 'completed', 'action': {'type': 'search', 'query': q}}
                        for n, q in enumerate(SEARCHES)]:
            raise ValueError(f'the searches did not come back as they went, with their ids: {after}')
        text = "Spotify's logo is green, #1ED760."
        # Long enough for the flow to see the searches while it works.
        return [{'type': 'response.output_text.delta', 'delta': text},
                {'type': 'response.output_item.done', 'item': {'type': 'message', 'id': 'msg-search', 'role': 'assistant',
                                                               'content': [{'type': 'output_text', 'text': text}]}},
                {'type': 'response.completed', 'response': {'id': 'response-searched'}}], 1.5


# The creator's style ChatGPT has to read before the guide.
STYLE = '# Editing style\n\nKeep pauses under 120 ms.\n'


def setup_chatgpt(r):
    from e2e.chatgpt import SCOPES
    setup(r)
    r.env.update(NUZKY_AGENT_CLAUDE=str(r.work / 'no-claude'), NUZKY_AGENT_CODEX=str(r.work / 'no-codex'))
    (r.work / 'data/nuzky/EDIT.md').write_text(STYLE)
    (r.work / 'data/nuzky/chatgpt.json').write_text(json.dumps({
        'host_id': f'urn:uuid:{uuid.uuid4()}', 'client_id': 'oaiapp_e2e', 'subject': 'e2e-editor', 'email': 'editor@example.test'}))
    (r.work / 'keychain.json').write_text(json.dumps({
        'client_id': 'oaiapp_e2e', 'access_token': 'e2e-access', 'refresh_token': 'e2e-refresh', 'id_token': 'e2e-id',
        'expires_at': int(time.time()) + 3600, 'scopes': SCOPES.split()}))
    r.chatgpt = ChatGptPanelServer()
    # Signing in again only has to start: no browser opens.
    r.env.update(NUZKY_CHATGPT_ISSUER=r.chatgpt.url, NUZKY_TEST_BROWSER='/usr/bin/true')


@flow('ai_panel_chatgpt', 'ChatGPT on the signed-in plan streams a split through local tools, shows its run and Undo, '
      'lists account models and recovers from a usage limit and a missing sign-in', before=setup_chatgpt)
def ai_panel_chatgpt(r):
    server = r.chatgpt

    def check(name, ok, detail=None):
        if not r.check(name, ok, detail):
            r.shot('failed')
            raise RuntimeError(name)

    try:
        r.s.run('document.body.focus()')
        r.key('j', ctrlKey=True)
        wait(lambda: panel(r) and 'GPT E2E Sol' in panel(r)['text'], 15)
        text = (panel(r) or {}).get('text', '')
        check('Ctrl+J opens the panel using ChatGPT, its plan, usage link and first model',
              r.s.run("return window.__nuzky.agent.getState().agent === 'chatgpt'")
              and all(s in text for s in ('Using ChatGPT plan', 'Manage usage', 'GPT E2E Sol')), text)
        click(r, 'GPT E2E Sol')
        menu = '[role=menu][aria-label=Model]'
        listed = r.s.run("return [...document.querySelectorAll(arguments[0] + ' [role^=menuitem]')].map((b) => b.textContent.trim())", menu)
        check('the model menu lists exactly the two visible models in server order', listed == ['GPT E2E Sol', 'GPT E2E Luna'], listed)
        click(r, 'GPT E2E Luna', menu)
        check("picking the second model changes the button, with that model's default effort",
              wait(lambda: r.s.run(MODEL) == 'GPT E2E Luna·Low', 3), r.s.run(MODEL))
        click(r, 'High', menu)
        r.key('Escape')
        check('picking High shows on the button', wait(lambda: r.s.run(MODEL) == 'GPT E2E Luna·High', 3), r.s.run(MODEL))
        r.shot('chatgpt-model-selected')
        webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
        check('in the smallest window the plan line and the model still show',
              wait(lambda: panel(r)['windowWidth'] <= 1030 and 'Using ChatGPT plan' in panel(r)['text'] and 'GPT E2E Luna' in panel(r)['text'], 5),
              panel(r))
        open_models(r)
        fits = """const m = document.querySelector('[role=menu][aria-label=Model]')?.getBoundingClientRect();
            const p = document.querySelector('aside[aria-label=AI]').getBoundingClientRect();
            return !!m && m.left >= p.left && m.right <= p.right;"""
        check('the open menu fits inside the panel there', wait(lambda: r.s.run(fits), 3), r.s.run(MODELS))
        time.sleep(0.5)  # WebKitGTK's screenshot can trail a change made just before it.
        r.shot('chatgpt-small-window-menu')
        r.key('Escape')
        r.shot('chatgpt-small-window')
        webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
        wait(lambda: panel(r)['windowWidth'] >= 1400, 5)

        before = r.track()
        send(r, 'Split the clip at 2 s')
        check('the editor locks and the live steps show Read the project and Editing',
              wait(lambda: r.state()['aiRun'], 15)
              and wait(lambda: all(s in panel(r)['text'] for s in ('Read the project', 'Editing')) and r.state()['aiRun'], 10), panel(r)['text'])
        wait(lambda: agent(r)['status'] == 'idle' and 'run' in agent(r)['kinds'], 20)
        saved = json.loads(r.saved_project().read_text())
        main = next(t['clips'] for t in saved['tracks'] if t['id'] == 'main')
        check('the done run names the split, saves two clips meeting at 2 s and shows no error toast',
              'Split talk.mp4' in panel(r)['text'] and len(main) == 2
              and main[0]['startUs'] + main[0]['durationUs'] == main[1]['startUs'] == 2_000_000
              and main[-1]['startUs'] + main[-1]['durationUs'] == before[0]['durationUs']
              and not r.errors(), {'panel': panel(r)['text'], 'clips': main, 'errors': r.errors()})
        r.shot('chatgpt-run-done')
        click(r, 'Undo')
        check('Undo on the card restores the original single clip', wait(lambda: r.track() == before, 10), r.track())

        send(r, 'limit please')
        idle(r)
        text = panel(r)['text']
        check('a streamed usage limit keeps the words and shows the plan card with Manage usage and no error code',
              'Checking your plan now.' in r.s.run(MESSAGES) and "ChatGPT can't run on your plan right now." in text
              and r.s.run("return [...document.querySelectorAll('aside[aria-label=AI] button')].filter((b) => b.textContent.trim() === 'Manage usage').length") == 2
              and 'subscription_sharing_usage_limit_exceeded' not in text and 'USAGE_LIMIT' not in text, text)
        r.shot('chatgpt-usage-limit')
        (r.work / 'keychain.json').unlink()
        send(r, 'Hello after sign-out')
        idle(r)
        check('a missing sign-in shows Sign in, returns the message to the field and stops saying the plan is in use',
              "ChatGPT isn't signed in." in panel(r)['text'] and 'Sign in…' in panel(r)['text']
              and wait(lambda: 'Using ChatGPT plan' not in panel(r)['text'], 5)
              and agent(r)['draft'] == 'Hello after sign-out'
              and r.s.run("return document.querySelector('aside[aria-label=AI] textarea').value") == 'Hello after sign-out', panel(r)['text'])
        r.shot('chatgpt-not-signed-in')
        click(r, 'Sign in…')
        dialog = "return document.querySelector('[role=dialog][aria-labelledby=chatgpt-title]')?.innerText ?? ''"
        check('Sign in… starts signing in again, with the ChatGPT dialog showing it',
              wait(lambda: 'Sign in to ChatGPT in your browser' in r.s.run(dialog), 10), r.s.run(dialog))
        click(r, 'Cancel', '[role=dialog][aria-labelledby=chatgpt-title]')
        check('cancelled, the dialog offers Continue with ChatGPT', wait(lambda: 'Continue with ChatGPT' in r.s.run(dialog), 10), r.s.run(dialog))
        r.key('Escape')
        wait(lambda: not r.s.run(dialog), 3)
        r.s.run("window.__nuzky.agent.setState({draft: ''})")
        click(r, 'New chat')
        check('signed out, the empty panel offers Continue with ChatGPT and, without Claude Code or Codex, how to install them',
              wait(lambda: all(s in panel(r)['text'] for s in ('Use your ChatGPT plan', 'Continue with ChatGPT', 'Or use Claude Code or Codex'))
                   and 'Using ChatGPT plan' not in panel(r)['text'], 5), panel(r)['text'])
        r.shot('chatgpt-sign-in')
        r.s.run("""document.querySelector('aside[aria-label=AI] button[title="Choose the agent"]').click()""")
        items = "return [...document.querySelectorAll('[role=menu][aria-label=Agent] [role^=menuitem]')].map((b) => b.textContent.trim())"
        check('the agent menu offers ChatGPT as not signed in, beside the two missing agents',
              wait(lambda: (r.s.run(items) or [])[:3] == ['Claude CodeNot installed', 'CodexNot installed', 'ChatGPTNot signed in'], 3),
              r.s.run(items))
        r.shot('chatgpt-agent-menu')
        r.key('Escape')

        unsupported = set('background conversation max_output_tokens max_tool_calls metadata moderation multi_agent prompt '
                          'prompt_cache_retention safety_identifier temperature top_logprobs top_p truncation user previous_response_id'.split())
        guide = (Path(__file__).resolve().parents[2] / 'skills/nuzky-edit/SKILL.md').read_text().split('---', 2)[2]
        heading = next(line for line in guide.splitlines() if line.startswith('# '))
        requests = server.requests
        bodies = [request['body'] for request in requests]
        check("all requests use the selected model, bearer token, preview fields, guide, namespaced Nuzky tools and only OpenAI's own web search",
              len(bodies) == 6 and not server.errors and all(
                  request['authorization'] == 'Bearer e2e-access' and body.get('model') == 'gpt-e2e-luna'
                  and body.get('reasoning') == {'effort': 'high'}
                  and body.get('store') is False and body.get('stream') is True and not unsupported.intersection(body)
                  and 'You are the AI panel inside Nuzky' in body.get('instructions', '') and heading in body['instructions']
                  and len(body.get('tools', [])) == 2 and body['tools'][0].get('type') == 'namespace' and body['tools'][1] == SEARCH_TOOL
                  and body['tools'][0].get('name') == 'nuzky' and all(t.get('type') == 'function' for t in body['tools'][0]['tools'])
                  and {'get_state', 'begin_run', 'apply_edits', 'end_run'} <= {t['name'] for t in body['tools'][0]['tools']}
                  for request, body in zip(requests, bodies)), {'requests': len(bodies), 'server_errors': server.errors})
        style_first = lambda text: STYLE in text and heading in text and text.index(STYLE) < text.index(heading)
        check("ChatGPT reads the creator's style before the guide and that it decides how to cut, caption and zoom",
              all(style_first(body.get('instructions', '')) and 'decides how you cut, caption and zoom' in body['instructions']
                  for body in bodies), bodies[0].get('instructions', '')[:2000])
        calls = [i for i in bodies[-1]['input'] if i['type'] == 'function_call']
        check('the first prompt carries the playhead and later inputs pair calls without item ids with their outputs',
              any('[Nuzky] Playhead' in p['text'] for i in bodies[0]['input'] if i.get('role') == 'user' for p in i['content'])
              and [call['name'] for call in calls] == ['get_state', 'begin_run', 'apply_edits', 'end_run']
              and all('id' not in i for body in bodies[1:] for i in body['input'] if i['type'] == 'function_call')
              and all(any(i['type'] == 'function_call_output' and i['call_id'] == call['call_id'] and isinstance(i['output'], str)
                          for i in bodies[-1]['input']) for call in calls), bodies[-1]['input'])

    finally:
        server.http.shutdown()
        server.http.server_close()
        server.worker.join(timeout=5)
        (r.work / 'requests.json').write_text(json.dumps(server.requests, indent=2) + '\n')


def setup_web_search(r):
    setup_chatgpt(r)
    # Claude Code and Codex stand in beside the ChatGPT sign-in, so one flow asks each of them.
    r.env.update(NUZKY_AGENT_CLAUDE=str(FAKE), NUZKY_AGENT_CODEX=str(FAKE.with_name('fake_codex')))


STEP_ROWS = "return [...document.querySelectorAll('aside[aria-label=AI] ol[aria-label=Steps] li')].map((li) => li.innerText.replace(/\\s+/g, ' ').trim())"
UNFOLD = "[...document.querySelectorAll('aside[aria-label=AI] button')].filter((b) => /^\\d+ steps?$/.test(b.textContent.trim())).forEach((b) => b.click())"


@flow('ai_panel_web_search', "Claude Code, Codex and ChatGPT search the web with their provider's own search, and the panel "
      'shows each search as a step of its own with what it searched for', before=setup_web_search)
def ai_panel_web_search(r):
    server = r.chatgpt
    searched = [f'Searching the web {query}' for query in SEARCHES]
    try:
        r.s.run('document.body.focus()')
        r.key('j', ctrlKey=True)
        wait(lambda: panel(r) and r.s.run('return window.__nuzky.agent.getState().agents'), 10)
        for agent_id, name, answer, steps in [
                ('claude', 'Claude Code', 'Logo Spotify je zelené, #1ED760.', searched),
                ('codex', 'Codex', 'Logo Spotify je zelené, #1ED760.', searched),
                ('chatgpt', 'ChatGPT', "Spotify's logo is green, #1ED760.", searched + ['Read the project'])]:
            if r.s.run('return window.__nuzky.agent.getState().agent') != agent_id:
                r.s.run("""document.querySelector('aside[aria-label=AI] button[title="Choose the agent"]').click()""")
                click(r, name, '[role=menu][aria-label=Agent]')
            r.check(f'{name} is the agent', wait(lambda: r.s.run('return window.__nuzky.agent.getState().agent') == agent_id
                                                 and (agent_id != 'chatgpt' or 'GPT E2E Sol' in panel(r)['text']), 10), panel(r)['text'])
            send(r, 'Search the web for the Spotify logo and its colours')
            live = wait(lambda: all(row in (r.s.run(STEP_ROWS) or []) for row in searched), 15)
            r.check(f'{name}: while it works, each search shows as a step of its own with its query', live, r.s.run(STEP_ROWS))
            r.shot(f'{agent_id}-searching')
            wait(lambda: agent(r)['status'] == 'idle', 20)
            r.s.run(UNFOLD)
            rows = wait(lambda: (lambda rows: rows if rows == steps else None)(r.s.run(STEP_ROWS)), 5) or r.s.run(STEP_ROWS)
            r.check(f'{name}: the turn ends without an error, its answer shown and every search kept as its own row',
                    rows == steps and 'error' not in agent(r)['kinds'] and r.s.run(MESSAGES)[-1:] == [answer] and not r.errors(),
                    {'rows': rows, 'kinds': agent(r)['kinds'], 'messages': r.s.run(MESSAGES), 'errors': r.errors()})
            r.shot(f'{agent_id}-searched')
            webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
            wait(lambda: panel(r)['windowWidth'] <= 1030 and panel(r)['slot'] == 'inspector', 5)
            # In the inspector's place the panel draws anew, with its steps folded.
            r.s.run(UNFOLD)
            wait(lambda: r.s.run(STEP_ROWS) == steps, 3)
            time.sleep(0.5)  # WebKitGTK's screenshot can trail a change made just before it.
            r.shot(f'{agent_id}-searched-small')
            webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
            wait(lambda: panel(r)['windowWidth'] >= 1430, 5)

        claude = next(x for x in runs(r) if x['argv'][:1] != ['exec'])
        codex = next(x for x in runs(r) if x['argv'][:1] == ['exec'])
        r.check("Claude Code got only its own WebSearch beside Nuzky's tools, and Codex only its search of OpenAI's index",
                not claude['missing'] and flag(claude['argv'], '--tools') == 'WebSearch'
                and flag(claude['argv'], '--allowedTools') == 'mcp__nuzky,WebSearch'
                and not codex['missing'] and 'web_search="cached"' in codex['argv'],
                {'claude': claude['argv'], 'codex': codex['argv']})
        bodies = [request['body'] for request in server.requests]
        back = [i for i in bodies[-1]['input'] if i['type'] == 'web_search_call'] if bodies else []
        r.check("ChatGPT offered OpenAI's web search without live pages, and its searches went back in the next request with their ids",
                len(bodies) == 2 and not server.errors and all(SEARCH_TOOL in body['tools'] for body in bodies)
                and [i['action']['query'] for i in back] == SEARCHES and [i.get('id') for i in back] == ['ws-0', 'ws-1'],
                {'requests': len(bodies), 'server_errors': server.errors, 'back': back})

        send(r, 'Search the web, then lose the connection')
        wait(lambda: agent(r)['status'] == 'idle' and 'error' in agent(r)['kinds'], 20)
        r.s.run(UNFOLD)
        rows = r.s.run(STEP_ROWS)
        running = r.s.run("return window.__nuzky.agent.getState().items.flatMap((i) => i.kind === 'steps' ? i.steps : []).filter((s) => s.status === 'running').length")
        r.check('a search the connection broke off in the middle of says Interrupted when the turn fails, with no step left running',
                'error' in agent(r)['kinds'] and rows[-1:] == ['Searching the web Interrupted'] and running == 0,
                {'rows': rows, 'running': running, 'kinds': agent(r)['kinds']})
        r.shot('chatgpt-search-broke-off')
    finally:
        server.http.shutdown()
        server.http.server_close()
        server.worker.join(timeout=5)
        (r.work / 'requests.json').write_text(json.dumps(server.requests, indent=2) + '\n')


@flow('ai_panel_codex', 'The AI panel uses Codex: live edits, undo, choices, conversation after Stop, and prompt login and usage hints',
      before=setup_codex)
def ai_panel_codex(r):
    r.s.run('document.body.focus()')
    r.key('j', ctrlKey=True)
    wait(lambda: panel(r), 5)
    agents = wait(lambda: r.s.run('return window.__nuzky.agent.getState().agents'), 10) or []
    r.check('Claude Code is not installed and Codex is installed with version 0.162.1',
            [(a['name'], bool(a['path']), a['version']) for a in agents] ==
            [('Claude Code', False, None), ('Codex', True, '0.162.1')], agents)
    r.check('the panel uses Codex', r.s.run("return window.__nuzky.agent.getState().agent === 'codex'")
            and 'Codex' in panel(r)['text'], panel(r)['text'])
    r.check("Codex starts on the first model it lists and that model's default effort",
            wait(lambda: r.s.run(MODEL) == 'Test First·Medium', 10), r.s.run(MODEL))

    before = r.track()
    send(r, 'Cut the pause at the start and add captions')
    r.check('the editor locks while Codex edits', wait(lambda: r.state()['aiRun'], 15))
    r.check('its steps show live with plain titles', wait(lambda: 'Read the project' in panel(r)['text'], 10), panel(r)['text'])
    wait(lambda: agent(r)['status'] == 'idle' and 'run' in agent(r)['kinds'], 30)
    text = panel(r)['text']
    r.check('a card lists the cut and the captions',
            'Cut out 1 passage, 2.0 s in total' in text and 'Added 2 captions' in text, text)
    r.check("Codex's whole answer shows", 'Hotovo.' in text, text)
    r.check('a message after a call the panel does not show is a message of its own',
            r.s.run(MESSAGES)[:2] == ['Vystřihnu pauzu na začátku a přidám titulky.', 'Začínám.'], r.s.run(MESSAGES))
    saved = json.loads(r.saved_project().read_text())
    main = next(t['clips'] for t in saved['tracks'] if t['id'] == 'main')
    captions = [c for t in saved['tracks'] if t['name'] == 'Captions' for c in t['clips']]
    r.check('the cut and both captions are in the saved project',
            main[-1]['startUs'] + main[-1]['durationUs'] == before[-1]['startUs'] + before[-1]['durationUs'] - 2_000_000
            and [c['content']['text'] for c in captions] == ['Dneska vám', 'ukážu jak'], saved['tracks'])
    first = runs(r)[0]
    r.check("Codex ran with Nuzky's tools only, outside home, without resuming",
            not first['missing'] and not first['cwd'].startswith(r.env['HOME']) and not first['resume'], first)
    r.shot('codex-run-done')
    click(r, 'Undo')
    r.check('Undo on the card takes back the whole run', wait(lambda: r.track() == before
            and not any(t['name'] == 'Captions' for t in r.state()['tracks']), 10), r.state()['tracks'])

    r.check('the first run went with that model and effort',
            flag(first['argv'], '-m') == 'test-first' and 'model_reasoning_effort="medium"' in first['argv'], first['argv'])
    open_models(r)
    r.check("the menu lists Codex's two listed models in its order, and Effort what the first takes, without Ultra",
            wait(lambda: r.s.run(MODELS) == ['Test First', 'Test Second'], 3) and r.s.run(EFFORT) == [['Med', 'true'], ['Max', 'false']],
            {'models': r.s.run(MODELS), 'effort': r.s.run(EFFORT)})
    click(r, 'Test Second', MENU)
    r.check('a model that does not take Medium shows its own default, High',
            wait(lambda: r.s.run(EFFORT) == [['Low', 'false'], ['High', 'true']], 3), r.s.run(EFFORT))
    time.sleep(0.5)  # WebKitGTK's screenshot can trail a change made just before it.
    r.shot('codex-model-menu')
    r.key('Escape')

    send(r, 'What options do I have?')
    r.check('the next message resumes the same Codex thread', wait(lambda: len(runs(r)) == 2, 10)
            and runs(r)[1]['resume'] and runs(r)[1]['session'] == first['session'], runs(r)[-1])
    r.check('it goes with the newly picked model and its effort',
            flag(runs(r)[1]['argv'], '-m') == 'test-second' and 'model_reasoning_effort="high"' in runs(r)[1]['argv'], runs(r)[1]['argv'])
    buttons = "return [...document.querySelectorAll('aside[aria-label=AI] [role=group] button')].map((b) => b.textContent)"
    r.check('the choices show as buttons', wait(lambda: r.s.run(buttons) ==
            ['The first takeShorter, a slip in the middle', 'The last takeClean, 2 s longer']
            and agent(r)['status'] == 'idle', 15), r.s.run(buttons))

    send(r, 'slow edit please')
    r.check('a slow Codex edit locks the editor', wait(lambda: r.state()['aiRun'] and 'Pracuju' in panel(r)['text'], 15))
    pid = runs(r)[-1]['pid']
    started = time.monotonic()
    click(r, 'Stop')
    gone = wait(lambda: not alive(pid), 5, 0.05)
    r.check('Stop ends Codex within seconds and unlocks the editor', gone and wait(lambda: not r.state()['aiRun'], 3),
            {'seconds': round(time.monotonic() - started, 2)})
    idle(r)
    send(r, 'hello after Stop')
    r.check('a stopped Codex turn keeps the same conversation', wait(lambda: len(runs(r)) == 4, 10)
            and runs(r)[3]['resume'] and runs(r)[3]['session'] == first['session']
            and wait(lambda: 'Rozumím: hello after stop' in panel(r)['text'], 10), runs(r)[-1])
    idle(r)

    # Codex reads one message per turn: a message while it works stops it and goes on in the same conversation.
    r.s.run("[...document.querySelectorAll('aside[aria-label=AI] button')].filter((b) => b.textContent.trim() === 'Undo').at(-1).click()")
    wait(lambda: r.track() == before, 10)
    said = lambda: panel(r)['text'].count('Pracuju')
    working = said()
    send(r, 'steer me please')
    r.check('a steerable Codex edit locks the editor', wait(lambda: r.state()['aiRun'] and said() > working, 15))
    pid = runs(r)[-1]['pid']
    started = time.monotonic()
    send(r, 'Do this instead')
    r.check('the message stops Codex within seconds and resumes the conversation with it',
            wait(lambda: not alive(pid), 5, 0.05) and wait(lambda: len(runs(r)) == 6, 10) and runs(r)[5]['resume']
            and runs(r)[5]['session'] == first['session'] and runs(r)[5]['prompt'].startswith('Do this instead')
            and wait(lambda: 'Rozumím: do this instead' in panel(r)['text'], 10),
            {'seconds': round(time.monotonic() - started, 2), 'run': runs(r)[-1]})
    idle(r)
    kinds = r.s.run("return window.__nuzky.agent.getState().items.map((i) => i.kind === 'run' ? `run:${i.label}:${i.stopped}` : i.kind === 'user' ? `user:${i.text}` : i.kind)")
    steered = kinds[kinds.index('user:steer me please'):]
    r.check('the interrupted run keeps its split in its own card, not called stopped, before the next answer',
            'run:Slow edit:false' in steered and 'stopped' not in steered and len(r.track()) == len(before) + 1
            and not any('AI edit done' in t for t in r.s.run("return document.body.innerText").split('\n')), steered)
    r.shot('codex-steered')

    # Before Codex has kept anything of its turn, stopping it would lose that message, so the new one waits.
    send(r, 'think first')
    wait(lambda: len(runs(r)) == 7, 10)
    send(r, 'Then this')
    r.check('a message sent before Codex did anything waits for its answer, then goes in the same conversation',
            wait(lambda: 'Rozumím: think first' in panel(r)['text'] and 'Rozumím: then this' in panel(r)['text'], 15)
            and len(runs(r)) == 8 and runs(r)[7]['resume'] and runs(r)[7]['prompt'].startswith('Then this')
            and panel(r)['text'].index('Rozumím: think first') < panel(r)['text'].index('Rozumím: then this'),
            {'runs': [(x['pid'], x['prompt'][:20]) for x in runs(r)[6:]], 'text': panel(r)['text'][-300:]})
    idle(r)

    # Stop and edit while a message waits stops for good: the message goes back to the field.
    r.s.run("[...document.querySelectorAll('aside[aria-label=AI] button')].filter((b) => b.textContent.trim() === 'Undo').at(-1).click()")
    wait(lambda: r.track() == before, 10)
    send(r, 'think first, then a slow edit')
    wait(lambda: len(runs(r)) == 9, 10)
    send(r, 'Then that')
    wait(lambda: r.s.run("return !!window.__nuzky.agent.getState().next"), 5)
    send(r, 'And this')
    r.check('a second message while one waits says why it was not sent', wait(lambda: 'Codex answers your last message first' in panel(r)['text'], 3)
            and agent(r)['draft'] == 'And this', {'draft': agent(r)['draft']})
    r.s.run("window.__nuzky.agent.setState({draft: ''})")
    r.check('the waiting Codex opens its run', wait(lambda: r.state()['aiRun'], 15))
    click(r, 'Stop and edit', 'header')
    r.check('Stop and edit sends nothing more and puts the waiting message back into the field',
            wait(lambda: not r.state()['aiRun'] and agent(r)['status'] == 'idle' and agent(r)['draft'] == 'Then that', 10)
            and not wait(lambda: len(runs(r)) > 9, 3), {'runs': len(runs(r)), 'agent': agent(r)})
    r.s.run("window.__nuzky.agent.setState({draft: ''})")
    r.s.run('window.__nuzky.store.getState().undo()')
    wait(lambda: r.track() == before, 5)

    started = time.monotonic()
    send(r, 'signin please')
    hint = wait(lambda: "Codex isn't signed in." in panel(r)['text'] and agent(r)['draft'] == 'signin please', 7, 0.05)
    text = panel(r)['text']
    gone = wait(lambda: len(runs(r)) == 10 and not alive(runs(r)[-1]['pid']), 1, 0.05)
    r.check('not signed in gives the login steps and the message back within 5 s, with Codex already stopped',
            hint and 'codex login' in text and 'Sign in with ChatGPT' in text and gone and time.monotonic() - started < 5,
            {'seconds': round(time.monotonic() - started, 2), 'draft': agent(r)['draft'], 'text': text, 'gone': gone})
    r.shot('codex-not-signed-in')
    idle(r)
    r.s.run("window.__nuzky.agent.setState({draft: ''})")
    send(r, 'limit please')
    r.check('the Codex usage hint includes its reset time', wait(lambda: 'Codex usage limit reached.' in panel(r)['text']
            and '9:00 PM' in panel(r)['text'], 10), panel(r)['text'])
    r.check('no error toast along the way', not r.errors(), r.errors())


def setup_codex_image(r):
    setup_codex(r)
    # The app knows this home; the fake saves its images under another one, as a Codex in a terminal may.
    r.env['CODEX_HOME'] = str(r.work / 'codex')


def orange_share(r, name):
    """The share of the preview that is the generated image's orange, which the test video has none of."""
    r.shot(name)
    crop = preview_crop(r.work / f'{name}.png', preview_rect(r)).convert('RGB')
    pixels = list(crop.getdata())
    return sum(1 for (red, green, blue) in pixels if red > 220 and 100 < green < 160 and blue < 40) / len(pixels)


@flow('ai_panel_codex_image', "Codex in the AI panel generates an image and puts it into the video; the project keeps "
      "Nuzky's own copy, so it stays when Codex's folder is emptied", before=setup_codex_image)
def ai_panel_codex_image(r):
    r.s.run('document.body.focus()')
    r.key('j', ctrlKey=True)
    wait(lambda: panel(r), 5)
    r.check('the panel uses Codex', wait(lambda: r.s.run("return window.__nuzky.agent.getState().agent === 'codex'"), 10))
    r.seek(500_000)
    before = orange_share(r, 'codex-image-before')
    send(r, 'Generate an orange image and put it in the video')
    wait(lambda: agent(r)['status'] == 'idle' and 'run' in agent(r)['kinds'], 30)
    text = panel(r)['text']
    r.check('Codex says the image is in the video', 'Hotovo, obrázek je v prvních 3 s.' in text, text)
    generated = Path((r.work / 'fake/generated.txt').read_text())
    saved = json.loads(r.saved_project().read_text())
    images = [a for a in saved['assets'] if a['kind'] == 'image']
    kept = (r.work / 'data/nuzky/generated-images').resolve()
    r.check("the project's image is Nuzky's copy, not the file in Codex's folder",
            len(images) == 1 and Path(images[0]['path']).parent.parent == kept
            and Path(images[0]['path']).read_bytes() == generated.read_bytes(), images)
    clips = [c for t in saved['tracks'] if t['id'] != 'main' for c in t['clips'] if images and c['content'].get('assetId') == images[0]['id']]
    r.check('the image shows from 0 for 3 s on an overlay track',
            [(c['startUs'], c['durationUs']) for c in clips] == [(0, 3_000_000)], clips)
    shutil.rmtree(generated.parent.parent)
    r.check("the copy stays when Codex's folder is emptied", images and Path(images[0]['path']).is_file())
    share = wait(lambda: (lambda s: s > 0.05 and s)(orange_share(r, 'codex-image')), 10)
    r.check('the preview shows the orange image where it showed none before', before < 0.005 and share,
            {'before': before, 'share': share})
    r.check('no error toast along the way', not r.errors(), r.errors())


# The share of the cover editor's canvas that is the generated image's orange.
COVER_ORANGE = """const c = document.querySelector('[data-testid=cover-canvas]');
if (!c || !c.width || c.width === 300) return null;
const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let n = 0;
for (let i = 0; i < d.length; i += 4) if (d[i] > 220 && d[i + 1] > 100 && d[i + 1] < 160 && d[i + 2] < 40) n++;
return n / (d.length / 4);"""
# The same for pure blue, the colour the image is saved anew in.
COVER_BLUE = COVER_ORANGE.replace("d[i] > 220 && d[i + 1] > 100 && d[i + 1] < 160 && d[i + 2] < 40", "d[i] < 40 && d[i + 1] < 40 && d[i + 2] > 220")
COVER_STATE = """const p = window.__nuzky.store.getState().snap.project;
const c = (p.thumbnails ?? []).find((t) => t.format === 'cover_9x16');
const image = c?.image ? p.assets.find((a) => a.id === c.image) : null;
return c && {image: c.image ?? null, path: image?.path ?? null, name: image?.name ?? null, timeUs: c.timeUs,
             texts: c.texts.map((t) => t.text)};"""


@flow('ai_panel_codex_cover', "Codex in the AI panel makes the cover from a generated image; the cover editor shows the "
      "image, the playhead leaves it, Use frame goes back to the video and Undo brings the image back",
      before=setup_codex_image)
def ai_panel_codex_cover(r):
    r.s.run('document.body.focus()')
    r.key('j', ctrlKey=True)
    wait(lambda: panel(r), 5)
    r.check('the panel uses Codex', wait(lambda: r.s.run("return window.__nuzky.agent.getState().agent === 'codex'"), 10))
    send(r, 'Make a cover with image gen: me looking aside at the GrokBot logo')
    wait(lambda: agent(r)['status'] == 'idle' and 'run' in agent(r)['kinds'], 30)
    r.check('Codex says the cover is made', 'Hotovo, náhled je z vygenerovaného obrázku.' in panel(r)['text'], panel(r)['text'])
    saved = json.loads(r.saved_project().read_text())
    cover = next((t for t in saved.get('thumbnails', []) if t['format'] == 'cover_9x16'), None)
    image = next((a for a in saved['assets'] if cover and a['id'] == cover.get('image')), None)
    kept = (r.work / 'data/nuzky/generated-images').resolve()
    r.check("the saved cover is made from Nuzky's copy of the generated image, with the hook",
            image and image['kind'] == 'image' and Path(image['path']).parent.parent == kept
            and [t['text'] for t in cover['texts']] == ['GROKBOT'], cover)

    click(r, 'Edit the cover', None)
    share = wait(lambda: (r.s.run(COVER_ORANGE) or 0) > 0.5 and r.s.run(COVER_ORANGE), 15)
    bar = r.s.run("return document.querySelector('[data-testid=cover-bar]')?.innerText ?? ''")
    r.check('the cover editor shows the generated image, and the bar Use frame instead of a time',
            share and 'Use frame' in bar and 'Frame' not in bar.replace('Use frame', ''), {'share': share, 'bar': bar})
    time.sleep(1)
    r.shot('codex-cover')
    r.seek(2_000_000)
    time.sleep(1)
    r.check('moving the playhead leaves the cover made from the image',
            r.s.run(COVER_STATE)['image'] == cover['image'] and (r.s.run(COVER_ORANGE) or 0) > 0.5, r.s.run(COVER_STATE))
    # The image saved anew under the same path, as an editor would: the next drawing, after a text edit, shows it, as
    # the export would.
    from PIL import Image
    retext = """window.__nuzky.store.getState().edit((p) => { const c = p.thumbnails.find((t) => t.format === 'cover_9x16');
        return {type: 'setThumbnail', thumbnail: {...c, texts: c.texts.map((t) => ({...t, text: arguments[0]}))}}; })"""
    Image.new('RGB', (540, 960), (0, 0, 255)).save(r.s.run(COVER_STATE)['path'], 'PNG')
    r.s.call(retext, 'GROKBOT!')
    r.check('an image saved anew on disk shows in the cover editor at the next drawing, as the export draws it',
            wait(lambda: (r.s.run(COVER_BLUE) or 0) > 0.5, 10), {'blue': r.s.run(COVER_BLUE), 'orange': r.s.run(COVER_ORANGE)})
    Image.new('RGB', (540, 960), (255, 128, 0)).save(r.s.run(COVER_STATE)['path'], 'PNG')
    r.s.call(retext, 'GROKBOT')
    wait(lambda: (r.s.run(COVER_ORANGE) or 0) > 0.5, 10)
    r.seek(2_000_000)
    frames = r.s.run("return document.querySelector('[data-testid=cover-frames]')?.innerText ?? ''")
    r.check('Pick for me says that choosing a frame replaces the image', 'Choosing a frame replaces it.' in frames, frames)
    apart = """const bar = document.querySelector('[data-testid=cover-bar]');
        const use = [...bar.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Use frame').getBoundingClientRect();
        const formats = bar.querySelector('[aria-label="Cover format"]').getBoundingClientRect();
        return use.width > 0 && use.right <= formats.left;"""
    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    time.sleep(1)
    r.check('Use frame stays clear of the format switch in the smallest window', r.s.run(apart))
    r.shot('codex-cover-small')
    webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
    time.sleep(1)
    r.check('and at 1440 x 900', r.s.run(apart))
    click(r, 'Use frame', '[data-testid=cover-bar]')
    r.check('Use frame makes the cover from the frame at the playhead, keeping the hook',
            wait(lambda: r.s.run(COVER_STATE)['image'] is None and r.s.run(COVER_ORANGE) is not None and r.s.run(COVER_ORANGE) < 0.01, 10)
            and r.s.run(COVER_STATE)['timeUs'] > 0 and r.s.run(COVER_STATE)['texts'] == ['GROKBOT'], r.s.run(COVER_STATE))
    r.shot('codex-cover-frame')
    # The message field keeps the keys while it has focus.
    r.s.run('document.activeElement.blur()')
    r.key('Home')
    r.check('Home then takes the cover to the first frame', wait(lambda: r.s.run(COVER_STATE)['timeUs'] == 0, 5),
            r.s.run(COVER_STATE))
    r.s.run('window.__nuzky.store.getState().undo()')
    wait(lambda: r.s.run(COVER_STATE)['timeUs'] == 2_000_000, 5)
    r.s.run('window.__nuzky.store.getState().undo()')
    r.check('Undo brings the generated image back',
            wait(lambda: r.s.run(COVER_STATE)['image'] == cover['image'] and (r.s.run(COVER_ORANGE) or 0) > 0.5, 10),
            r.s.run(COVER_STATE))
    r.check('no error toast along the way', not r.errors(), r.errors())


@flow('ai_panel_jobs', "A job of the panel's agent outlasts the turn it ends to ask: it shows with Stop while the user "
      'decides, the next turn reads its result or cancels it, and Stop or a new chat stops it', before=setup_jobs)
def ai_panel_jobs(r):
    r.s.run('document.body.focus()')
    r.key('j', ctrlKey=True)
    wait(lambda: panel(r), 5)
    wait(lambda: r.s.run('return window.__nuzky.agent.getState().agents'), 10)
    fake = r.work / 'fake'
    status = "return [...document.querySelectorAll('aside[aria-label=AI] [role=status]')].map((e) => e.textContent).join(' | ')"
    stop = "return [...document.querySelectorAll('aside[aria-label=AI] button')].some((b) => b.textContent.trim() === 'Stop')"
    # The latest question's choice: earlier questions keep theirs, turned off.
    answer = """const b = [...document.querySelectorAll('aside[aria-label=AI] [role=group] button')]
        .filter((b) => b.textContent.trim() === arguments[0] && !b.getAttribute('aria-disabled')).pop(); b?.click(); return !!b;"""
    said = lambda: (panel(r) or {}).get('text', '')

    open_models(r)
    r.check("an older Claude Code that names no models still offers its aliases, with every effort",
            wait(lambda: r.s.run(MODELS) == ['DefaultClaude Code setting', 'Fable', 'Opus', 'Sonnet', 'Haiku'], 10)
            and [e for e, _ in r.s.run(EFFORT)] == ['Low', 'Med', 'High', 'X-High', 'Max'],
            {'models': r.s.run(MODELS), 'effort': r.s.run(EFFORT)})
    r.key('Escape')

    def ask(text='Export it, a long job'):
        """The agent starts an export and ends its turn with a question; returns the job."""
        (fake / 'job.json').unlink(missing_ok=True)
        idle(r)
        send(r, text)
        wait(lambda: (fake / 'job.json').exists() and agent(r)['status'] == 'idle', 30)
        job = json.loads((fake / 'job.json').read_text())
        r.check('the export was still running when the agent ended its turn to ask', job['running_at_end'], job)
        return job['id']

    first = ask()
    r.check('while the user decides, the panel shows the export with Stop, and the answer can be sent',
            wait(lambda: 'Exporting' in r.s.run(status) and r.s.run(stop), 5) and agent(r)['status'] == 'idle', r.s.run(status))
    r.shot('job-between-turns')
    # The user takes a moment to answer.
    time.sleep(1)
    r.check('the user answers the question', r.s.run(answer, 'Wait and continue'))
    # The answer streams in, so wait for the file name after "done", not just the word.
    r.check('the next turn reads the finished result of that same job',
            wait(lambda: any(f'Job {first}: done {p.name}' in said() for p in fake.glob('export-*.mp4')), 60), said())
    exported = [p for p in fake.glob('export-*.mp4') if p.name in said()]
    r.check('the export is in its file', len(exported) == 1 and exported[0].stat().st_size > 0,
            {'files': sorted(p.name for p in fake.iterdir()), 'said': said()})
    r.check('with the job done, the panel no longer shows it', wait(lambda: 'Exporting' not in r.s.run(status) and not r.s.run(stop), 5),
            r.s.run(status))
    r.shot('job-result')

    second = ask()
    r.check('the user answers the question', r.s.run(answer, 'Stop the export'))
    r.check('the next turn can cancel the job the turn before started', wait(lambda: f'Job {second}: cancelled' in said(), 30), said())

    third = ask()
    wait(lambda: r.s.run(stop), 5)
    started = time.monotonic()
    click(r, 'Stop')
    gone = wait(lambda: 'Exporting' not in r.s.run(status) and not r.s.run(stop), 5, 0.05)
    r.check('Stop in the panel stops the job between turns within a few seconds', gone, {'seconds': round(time.monotonic() - started, 2)})
    r.check('the user answers the question', r.s.run(answer, 'Wait and continue'))
    r.check('the agent then finds it cancelled', wait(lambda: f'Job {third}: cancelled' in said(), 30), said())

    ask('Export it into a folder, a long job')
    # The folder goes away while the user decides, so the export cannot be saved.
    shutil.rmtree(fake / 'folder')
    r.check('an export that fails between turns says so in the panel, with no Stop',
            wait(lambda: 'Exporting failed' in r.s.run(status) and not r.s.run(stop), 30), r.s.run(status))
    r.shot('job-failed')

    fourth = ask()
    r.s.run("""document.querySelector('aside[aria-label=AI] button[aria-label^="New chat"]').click()""")
    r.check('a new chat leaves nothing of the old one running in the panel', wait(lambda: not r.s.run(stop), 5), r.s.run(status))
    send(r, 'Wait and continue')
    r.check('and stops the job the old one started', wait(lambda: f'Job {fourth}: cancelled' in said(), 30), said())

    ask()
    # The agent goes away while its answer starts, so no turn starts.
    (fake / 'vanish').touch()
    send(r, 'Wait and continue')
    r.check('when the answer cannot start, the panel still shows the export with Stop',
            wait(lambda: 'error' in agent(r)['kinds'], 10) and wait(lambda: 'Exporting' in r.s.run(status) and r.s.run(stop), 5),
            {'status': r.s.run(status), 'agent': agent(r)})
    click(r, 'Stop')
    r.check('and Stop stops it', wait(lambda: 'Exporting' not in r.s.run(status) and not r.s.run(stop), 5), r.s.run(status))
    left = lambda: sorted(p.name for p in fake.iterdir() if p.name.startswith(('export-', '.nuzky-part')))
    # The panel lets go of a stopped job at once; the export may still be removing its part.
    r.check('the stopped exports left no file behind', bool(exported) and wait(lambda: left() == [exported[0].name], 10), left())
    r.check('no error toast along the way', not r.errors(), r.errors())


@flow('ai_panel_chats', 'Project chats list newest first, reopen with the keyboard, resume the right session, survive a restart '
      'and keep the message when the agent has lost its session', before=setup)
def ai_panel_chats(r):
    def check(name, ok, detail=None):
        if not r.check(name, ok, detail):
            r.shot('failed')
            raise RuntimeError(name)

    cut = 'Cut the pause at the start and add captions'
    field = "return document.querySelector('aside[aria-label=AI] textarea')?.value"
    items = 'return window.__nuzky.agent.getState().items'
    files = lambda: sorted((r.work / 'data/nuzky/chats').glob('*/*'))
    r.s.run('document.body.focus()')
    r.key('j', ctrlKey=True)
    check('Ctrl+J opens the panel with Claude Code ready',
          wait(lambda: panel(r) and r.s.run('return window.__nuzky.agent.getState().agents'), 10)
          and r.s.run("return window.__nuzky.agent.getState().agent === 'claude'"), panel(r))
    send(r, cut)
    check('the first chat shows the cut message, answer and completed run card',
          wait(lambda: idle(r) and 'run' in agent(r)['kinds'] and 'Hotovo.' in panel(r)['text'], 30)
          and cut in panel(r)['text'] and 'Cut out 1 passage, 2.0 s in total' in panel(r)['text'], panel(r)['text'])
    first = runs(r)[0]
    click(r, 'New chat')
    check('New chat clears the conversation', wait(lambda: not r.s.run(items), 5), agent(r))
    send(r, 'Hello there')
    check('the second chat echoes Hello there and enables Past chats',
          wait(lambda: 'Rozumím: hello there' in r.s.run(MESSAGES) and agent(r)['status'] == 'idle', 15)
          and wait(lambda: r.s.run("""const b = document.querySelector('aside[aria-label=AI] button[aria-label="Past chats in this project"]');
              return b && !b.disabled && b.getAttribute('aria-disabled') !== 'true';"""), 5), panel(r)['text'])

    click(r, 'Past chats in this project')
    rows = "return [...document.querySelectorAll('ul[aria-label=\"Past chats\"] button')].map((b) => b.innerText)"
    check('Past chats lists two Claude Code chats, newest Hello there open, then the cut',
          wait(lambda: len(r.s.run(rows)) == 2, 5)
          and r.s.run(rows)[0].startswith('Hello there\nOpen now') and r.s.run(rows)[1].startswith(cut)
          and 'Open now' not in r.s.run(rows)[1] and all('Claude Code' in row for row in r.s.run(rows)), r.s.run(rows))
    r.shot('chat-list')
    check('keyboard focus starts on the open chat',
          r.s.run('return document.activeElement?.getAttribute("aria-current")') == 'true')
    r.key('ArrowDown')
    check('ArrowDown focuses the cut chat', r.s.run('return document.activeElement?.innerText') == r.s.run(rows)[1])
    press('Return')
    check('Enter reopens the cut chat with its messages, run card, date and Claude Code, without Undo',
          wait(lambda: r.s.run("""const p = document.querySelector('aside[aria-label=AI]');
              if (!p) return false;
              const a = window.__nuzky.agent.getState(), text = p.innerText;
              return !p.querySelector('ul[aria-label="Past chats"]') && text.includes(arguments[0])
                && a.items.some((i) => i.kind === 'run') && text.includes('Hotovo.')
                && text.includes('Cut out 1 passage, 2.0 s in total') && !text.includes('Hello there')
                && !!p.querySelector('[role=separator]')?.textContent.trim()
                && ![...p.querySelectorAll('button')].some((b) => b.textContent.trim() === 'Undo')
                && a.agent === 'claude';""", cut), 10), panel(r)['text'])
    r.shot('reopened')
    check('the reopened chat becomes the one last used, first in the list',
          wait(lambda: r.s.run("const a = window.__nuzky.agent.getState(); return a.chats?.[0]?.id === a.chat"), 5),
          r.s.run("return window.__nuzky.agent.getState().chats"))
    send(r, 'Hello again')
    check('Hello again resumes the cut chat session and its answer shows',
          wait(lambda: len(runs(r)) == 3 and 'Rozumím: hello again' in r.s.run(MESSAGES)
               and agent(r)['status'] == 'idle', 15)
          and runs(r)[-1]['resume'] and runs(r)[-1]['session'] == first['session'], runs(r)[-1])
    kept = r.s.run(items)
    check('both chats are saved and the continued chat keeps every message and run card',
          wait(lambda: len(files()) == 2 and any(json.loads(p.read_text()).get('items') == kept for p in files()), 10),
          [str(p.relative_to(r.work)) for p in files()])

    leave_editor(r)
    close_window()
    check('the window closes', wait(lambda: not window_alive(r), 15))
    r.s.close()
    r.s = Session()
    check('the app starts again with the project',
          wait(lambda: r.s.run('return !!window.__nuzky?.store.getState().snap', retries=3), 60))
    r.s.run("window.__nuzky.store.setState({view: 'editor'})")
    ai_button = "document.querySelector('header button[title*=\"AI (Ctrl+J)\"]')"
    check('the editor is ready after restart', wait(lambda: r.s.run(f'return !!{ai_button}'), 10))
    if r.s.run(f"return {ai_button}.getAttribute('aria-pressed')") != 'true':
        r.s.run('document.body.focus()')
        r.key('j', ctrlKey=True)
    check('restart automatically reopens the last continued chat with Hello again, the original cut and nothing lost',
          wait(lambda: panel(r) and 'Hello again' in panel(r)['text'] and cut in panel(r)['text']
               and r.s.run(items) == kept, 10), {'panel': panel(r), 'items': r.s.run(items)})
    (Path(r.env['NUZKY_FAKE_AGENT_LOG']) / 'sessions' / first['session']).unlink()
    send(r, 'Still there?')
    check('a missing session shows the recovery card and returns Still there? to the field',
          wait(lambda: agent(r)['status'] == 'idle' and 'Claude Code no longer has this chat' in panel(r)['text']
               and agent(r)['draft'] == r.s.run(field) == 'Still there?', 15), panel(r)['text'])
    check('focus moves to New chat with this message, so Enter takes the way on',
          wait(lambda: r.s.run('return document.activeElement?.textContent.trim()') == 'New chat with this message', 3),
          r.s.run('return document.activeElement?.outerHTML'))
    count = len(runs(r))
    r.s.run("document.querySelector('aside[aria-label=AI] textarea').focus()")
    r.key('Enter')
    check('Enter in the field explains that this chat cannot continue and starts no fake run',
          wait(lambda: 'Start a new chat to send this' in panel(r)['text'], 5)
          and len(runs(r)) == count and agent(r)['status'] == 'idle' and r.s.run(field) == 'Still there?', panel(r)['text'])
    r.shot('session-gone')
    click(r, 'New chat with this message')
    check('New chat with this message clears the conversation and keeps the draft without sending it',
          wait(lambda: not r.s.run(items), 5) and agent(r)['draft'] == r.s.run(field) == 'Still there?'
          and len(runs(r)) == count, agent(r))
    beside = sorted((r.work / 'data/nuzky/projects').glob('*chat*'))
    saved = files()
    check('chat storage has exactly two JSON files in one 16-hex project folder and none beside the project',
          not beside and len(saved) == 2 and len({p.parent for p in saved}) == 1
          and all(p.suffix == '.json' and len(p.parent.name) == 16
                  and all(c in '0123456789abcdef' for c in p.parent.name) for p in saved)
          and {json.loads(p.read_text())['title'] for p in saved} == {cut, 'Hello there'},
          {'beside': beside, 'chats': [str(p.relative_to(r.work)) for p in saved]})


@flow('ai_panel', "The AI panel runs the user's agent with only Nuzky's tools: it edits live, says exactly what changed, "
      'offers choices, stops at once, keeps a failed message, and docks, floats and resizes where the user put it',
      before=setup)
def ai_panel(r):
    r.check('the panel starts closed, with AI in the top bar', panel(r) is None and r.s.run(
        "return !![...document.querySelectorAll('header button')].find((b) => b.textContent.trim() === 'AI')"))
    r.s.run('document.body.focus()')
    r.key('j', ctrlKey=True)
    p = wait(lambda: panel(r), 5)
    r.check('Ctrl+J opens it as a column at the right edge, under the top bar to the bottom, beside the inspector',
            p and p['right'] == 6 and p['top'] == 48 and p['bottom'] == 6 and p['width'] == 360 and p['inspector'], p)
    r.check('the field takes focus', wait(lambda: r.s.run('return document.activeElement?.tagName') == 'TEXTAREA', 3))
    agents = wait(lambda: r.s.run('return window.__nuzky.agent.getState().agents'), 10) or []
    r.check('Claude Code is found and Codex is not installed', [(a['name'], bool(a['path'])) for a in agents] == [('Claude Code', True), ('Codex', False)], agents)
    click(r, 'Add captions')
    r.check('a starting point goes into the field to send or change, not straight to the agent',
            wait(lambda: agent(r)['draft'] == 'Add captions', 3) and not runs(r), agent(r))
    r.s.run("window.__nuzky.agent.setState({draft: ''})")
    clip = r.track()[0]
    r.s.run('window.__nuzky.store.getState().select([arguments[0]])', clip['id'])
    r.check('the selection shows as a chip that goes with the message', wait(lambda: '1 clip' in panel(r)['text'], 5), panel(r)['text'])
    click(r, "Don't send the playhead")
    click(r, 'Show the agent the frame at the playhead')
    r.check('the camera button shows the agent the frame: the playhead comes back as a Frame chip with ×',
            wait(lambda: 'Frame 00:00' in panel(r)['text'] and 'Playhead' not in panel(r)['text'], 5)
            and r.s.run(CHIPS) == ["Don't send the selected clips", "Don't send the frame"]
            and r.s.run(PRESSED, "Don't show the agent the frame at the playhead") == 'true'
            and r.s.run(TEXT, "Don't show the agent the frame at the playhead") == 'Frame',
            panel(r)['text'])
    r.shot('panel-open')
    r.check('× on the Frame chip stops sending the frame and brings back the playhead chip',
            click(r, "Don't send the frame") == 'false'
            and wait(lambda: r.s.run(CHIPS) == ["Don't send the selected clips", "Don't send the playhead"], 5)
            and not r.s.run('return window.__nuzky.agent.getState().frame')
            and r.s.run(TEXT, 'Show the agent the frame at the playhead') == '', r.s.run(CHIPS))
    click(r, 'Show the agent the frame at the playhead')

    # A cut with captions: live, explained by what changed, undone in one step.
    before = r.track()
    send(r, 'Cut the pause at the start and add captions')
    r.check('the editor locks while the agent edits', wait(lambda: r.state()['aiRun'], 15))
    r.check('its steps show live with plain titles', wait(lambda: 'Read the project' in panel(r)['text'], 10), panel(r)['text'])
    r.shot('working')
    wait(lambda: agent(r)['status'] == 'idle' and 'run' in agent(r)['kinds'], 30)
    text = panel(r)['text']
    r.check('a card lists exactly what changed, from the project itself',
            'Shortened the video from' in text and 'Cut out 1 passage, 2.0 s in total' in text and 'Added 2 captions' in text, text)
    r.check("the agent's answer streams in", 'Hotovo.' in text, text)
    r.check('a message after a call the panel does not show (here begin_run) is a message of its own, not glued to the last one',
            r.s.run(MESSAGES)[:2] == ['Vystřihnu pauzu na začátku a přidám titulky.', 'Začínám.'] and 'titulky.Začínám' not in text,
            r.s.run(MESSAGES))
    r.s.run("[...document.querySelectorAll('aside[aria-label=AI] button')].filter((b) => /^\\d+ steps?$/.test(b.textContent.trim())).forEach((b) => b.click())")
    r.check('unfolded steps name the scan and the frames it checked',
            wait(lambda: 'Scanning picture and sound' in panel(r)['text'] and 'Checking frames' in panel(r)['text'] and '00:00 · 00:01' in panel(r)['text'], 5), panel(r)['text'])
    rows = r.s.run("return [...document.querySelectorAll('aside[aria-label=AI] ol[aria-label=Steps] li')].map((li) => li.innerText.replace(/\\s+/g, ' ').trim())")
    r.check('the same step three times in a row is one row that says so, not three rows',
            rows.count('Read the project 3 times') == 1 and rows.count('Read the project') == 1, rows)
    r.check('the card takes the place of the "AI edit done" toast', not any('AI edit done' in t['text'] for t in r.state()['toasts']), r.state()['toasts'])
    saved = json.loads(r.saved_project().read_text())
    captions = [t for t in saved['tracks'] if t['name'] == 'Captions']
    r.check('the cut and the captions are in the saved project', captions and len(captions[0]['clips']) == 2, [t['name'] for t in saved['tracks']])
    first = runs(r)[0]
    r.check("the agent ran with Nuzky's tools only: built-ins and hooks off, in a folder outside home",
            not first['missing'] and not first['cwd'].startswith(os.environ['HOME']) and not first['resume'], first)
    r.check('the message carried the selection, the playhead and the frame',
            f"Selected clips: {clip['id']}" in first['prompt'] and '[Nuzky] Playhead' in first['prompt'] and 'points at the frame at the playhead' in first['prompt'],
            first['prompt'])
    r.shot('run-done')
    # A real key press, as the clipboard takes text only from something the user did. On macOS it is the person's
    # own clipboard, so their text goes back right after.
    theirs = clipboard() if MACOS else None
    found = r.s.run("""const b = document.querySelector('aside[aria-label=AI] button[aria-label="Copy message"]');
        b?.focus(); return !!b;""")
    try:
        press('Return')
        copied = wait(lambda: clipboard() == 'Cut the pause at the start and add captions', 5)
        got = clipboard()
        shown = r.s.run("return !!document.querySelector('aside[aria-label=AI] button[aria-label=Copied]')")
        r.shot('message-copied')
        reply = r.s.run(MESSAGES)[-1]
        reply_found = r.s.run("""const b = [...document.querySelectorAll('aside[aria-label=AI] button[aria-label="Copy reply"]')].at(-1);
            b?.focus(); return !!b;""")
        press('Return')
        reply_copied = wait(lambda: clipboard() == reply, 5)
        reply_got = clipboard()
        reply_shown = r.s.run("return document.activeElement?.getAttribute('aria-label') === 'Copied'")
        r.shot('reply-copied')
    finally:
        if theirs is not None:
            subprocess.run(['pbcopy'], input=theirs, text=True, check=True)
    r.check('Copy beside my message puts exactly what I sent on the clipboard and says it is copied',
            found and copied and shown, {'button': found, 'clipboard': got, 'copied shown': shown})
    r.check("Copy under the agent's reply puts exactly its text on the clipboard and says it is copied",
            reply_found and reply.startswith('Hotovo.') and reply_copied and reply_shown,
            {'button': reply_found, 'reply': reply, 'clipboard': reply_got, 'copied shown': reply_shown})
    click(r, 'Added 2 captions')
    r.check('a line of the card selects the clips it is about', wait(lambda: len(r.s.run('return window.__nuzky.store.getState().selection')) == 2, 5))
    click(r, 'Undo')
    r.check('Undo on the card takes back the whole run', wait(lambda: len(r.track()) == len(before) and not any(t['name'] == 'Captions' for t in r.state()['tracks']), 10), r.state()['tracks'])
    undo = "return [...document.querySelectorAll('aside[aria-label=AI] button')].some((b) => b.textContent.trim() === 'Undo')"
    r.check('then the card says the run was undone and offers no Undo', wait(lambda: 'Undone: Tighten the start' in panel(r)['text'] and not r.s.run(undo), 5), panel(r)['text'])

    # Model and effort: Claude Code's own setting until the user picks, then both go with the next message.
    r.check('Claude Code starts on its own model and effort, so the first run had neither flag',
            r.s.run(MODEL) == 'Default' and '--model' not in first['argv'] and '--effort' not in first['argv'],
            {'button': r.s.run(MODEL), 'argv': first['argv']})
    open_models(r)
    r.check("the menu offers Default and each model Claude Code names, by its version and once, with no effort picked and the line saying why",
            wait(lambda: r.s.run(MODELS) == ['DefaultClaude Code setting', 'Opus 5.5', 'Fable 5.1', 'Sonnet 5.5'], 10)
            and [e for e, _ in r.s.run(EFFORT)] == ['Low', 'Med', 'High', 'X-High', 'Max']
            and all(pressed == 'false' for _, pressed in r.s.run(EFFORT))
            and 'Uses your Claude Code setting.' in r.s.run(f"return document.querySelector('{MENU}').innerText"),
            {'models': r.s.run(MODELS), 'effort': r.s.run(EFFORT)})
    click(r, 'High', MENU)
    r.check('an effort picked on Default shows on the button and keeps the menu open',
            wait(lambda: ['High', 'true'] in r.s.run(EFFORT), 3) and r.s.run(MODEL) == 'Default·High', {'button': r.s.run(MODEL), 'effort': r.s.run(EFFORT)})
    click(r, 'Opus 5.5', MENU)
    r.check('picking Opus keeps the effort picked before',
            wait(lambda: ['High', 'true'] in r.s.run(EFFORT), 3) and r.s.run(MODEL) == 'Opus 5.5·High', {'button': r.s.run(MODEL), 'effort': r.s.run(EFFORT)})
    click(r, 'X-High', MENU)
    wait(lambda: ['X-High', 'true'] in r.s.run(EFFORT), 3)
    time.sleep(0.5)  # WebKitGTK's screenshot can trail a change made just before it.
    r.shot('model-menu')
    r.key('Escape')
    r.check('Esc closes the menu and the button shows the choice',
            wait(lambda: not r.s.run(MODELS) and r.s.run(MODEL) == 'Opus 5.5·Extra high', 3), r.s.run(MODEL))

    # Choices: the next message continues the conversation; a picked option is the next message.
    send(r, 'What options do I have?')
    r.check('the next message continues the same conversation', wait(lambda: len(runs(r)) == 2, 10) and runs(r)[1]['resume'] and runs(r)[1]['session'] == first['session'], runs(r)[-1])
    r.check('it goes with the chosen model and effort',
            flag(runs(r)[1]['argv'], '--model') == 'opus' and flag(runs(r)[1]['argv'], '--effort') == 'xhigh', runs(r)[1]['argv'])
    buttons = "return [...document.querySelectorAll('aside[aria-label=AI] [role=group] button')].map((b) => [b.textContent, b.getAttribute('aria-pressed'), b.getAttribute('aria-disabled')])"
    r.check('the choices show as buttons', wait(lambda: len(r.s.run(buttons)) == 2 and agent(r)['status'] == 'idle', 15), r.s.run(buttons))
    r.shot('options')
    click(r, 'The last takeClean, 2 s longer') or click(r, 'The last take')
    r.check('a picked choice goes as the next message', wait(lambda: len(runs(r)) == 3 and runs(r)[2]['prompt'].startswith('The last take'), 10)
            and wait(lambda: 'Rozumím: the last take' in panel(r)['text'], 10), runs(r)[-1]['prompt'] if runs(r) else None)
    state = r.s.run(buttons)
    r.check('it is marked chosen and the other is off', state[1][1] == 'true' and state[0][2] == 'true', state)
    idle(r)

    # Agents write loose lists, a blank line between items; each item once came out as its own list numbered 1.
    send(r, 'Sum up the problems as a numbered list')
    blocks = wait(lambda: idle(r) and r.s.run(LISTS), 15)
    r.check('a loose numbered list in an answer is one list numbered 1, 2, 3 with its wrapped line inside the item, a command indented under an item keeps its line and indent, '
            'a list after a paragraph goes on from its own first number, and loose bullets are one list too', blocks == [
                'Here is what went wrong:',
                [[1, 'Captions have no shadow. TextStyle has no shadow, so white text disappears on a bright sky.'],
                 [2, 'Caption position cannot be set in bulk. Each caption moves alone.'],
                 [3, 'Font size meant rebuilding captions. The size lived in the preset.']],
                'What I would do next:',
                [[4, 'Add a shadow.'], [5, 'Move captions together.']],
                [['•', 'Keep the preset'], ['•', 'Keep the font, then run\n    npm run dev']],
                'That is all.'], blocks)
    r.shot('numbered-list')

    # A message while it works steers the same agent process, inside the same run.
    send(r, 'steer me please')
    r.check('a steerable edit locks the editor', wait(lambda: r.state()['aiRun'] and 'Pracuju. Napiš mi' in panel(r)['text'], 15))
    pid, count = runs(r)[-1]['pid'], len(runs(r))
    r.s.run("""const t = document.querySelector('aside[aria-label=AI] textarea');
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(t, arguments[0]);
        t.dispatchEvent(new Event('input', {bubbles: true}));""", 'Also split it at 4 s')
    r.check('while it works, typed text offers Send now beside Stop',
            wait(lambda: r.s.run("return !!document.querySelector('aside[aria-label=AI] button[aria-label=\"Send now\"]')"), 3)
            and 'Tell Claude Code more while it works' in r.s.run("return document.querySelector('aside[aria-label=AI] textarea').placeholder"))
    r.shot('steer-typed')
    r.key('Enter')
    r.check('the agent takes the message in the same process and answers it',
            wait(lambda: 'Beru v potaz: Also split it at 4 s' in panel(r)['text'], 15) and len(runs(r)) == count and runs(r)[-1]['pid'] == pid,
            {'runs': len(runs(r)), 'text': panel(r)['text']})
    r.check('the message shows in the conversation where it was sent', wait(lambda: agent(r)['status'] == 'idle', 15)
            and r.s.run("const i = window.__nuzky.agent.getState().items; return i.map((x) => x.kind === 'user' ? x.text : x.kind)").count('Also split it at 4 s') == 1,
            r.s.run("return window.__nuzky.agent.getState().items.map((x) => x.kind)"))
    r.check('it is answered once, by that process', len(runs(r)) == count and 'Rozumím: also split' not in panel(r)['text'],
            {'runs': len(runs(r)), 'text': panel(r)['text']})
    text = panel(r)['text']
    r.check('both splits are one run with one card', text.count('Steerable edit') == 1 and 'Split talk.mp4\n00:02' in text and '00:04' in text
            and len(r.track()) == len(before) + 2, text)
    r.shot('steered')
    r.s.run("[...document.querySelectorAll('aside[aria-label=AI] button')].filter((b) => b.textContent.trim() === 'Undo').at(-1).click()")
    r.check("the card's Undo takes back both splits at once", wait(lambda: len(r.track()) == len(before), 10), r.track())

    # Stop, in the panel and in the top bar.
    for where in ('Stop', 'Stop and edit'):
        send(r, 'slow edit please')
        r.check(f'a slow edit locks the editor ({where})', wait(lambda: r.state()['aiRun'] and 'Pracuju' in panel(r)['text'], 15))
        pid = runs(r)[-1]['pid']
        started = time.time()
        click(r, where, 'aside[aria-label=AI]' if where == 'Stop' else 'header')
        gone = wait(lambda: not alive(pid), 5, 0.05)
        took = round(time.time() - started, 2)
        r.check(f'{where} ends the agent within seconds and unlocks the editor', gone and wait(lambda: not r.state()['aiRun'], 3), {'seconds': took})
        r.check(f'what it did so far stays, and the stopped card says what ({where})',
                wait(lambda: idle(r) and 'Stopped: Slow edit' in panel(r)['text'] and 'Split talk.mp4\n00:02' in panel(r)['text'], 5), panel(r)['text'])
        if where == 'Stop':
            r.shot('stopped')
        r.s.run('window.__nuzky.store.getState().undo()')
        wait(lambda: len(r.track()) == len(before), 5)

    # Problems say what to do next and never lose the message.
    send(r, 'signin please')
    r.check('not signed in says what to do and gives the message back', wait(lambda: "isn't signed in." in panel(r)['text'], 10)
            and agent(r)['draft'].startswith('signin please'), {'draft': agent(r)['draft']})
    r.shot('not-signed-in')
    r.s.run("window.__nuzky.agent.setState({draft: ''})")
    send(r, 'limit please')
    r.check("a used-up plan says so, with the agent's reset time", wait(lambda: 'usage limit reached.' in panel(r)['text'] and 'resets 9pm' in panel(r)['text'], 10), panel(r)['text'])
    r.check('the earlier problem went away with the new message, so its Try again cannot send this one', "isn't signed in." not in panel(r)['text'], panel(r)['text'])
    r.s.run("window.__nuzky.agent.setState({draft: ''})")
    field = "return [window.__nuzky.agent.getState().draft, document.querySelector('aside[aria-label=AI] textarea').value, document.querySelector('aside[aria-label=AI]').innerText.includes('Wait for the answer')]"
    r.check('the field stays as the user left it, with no stale reason under it', wait(lambda: r.s.run(field) == ['', '', False], 3), r.s.run(field))
    r.check('no error toast along the way', not r.errors(), r.errors())

    # Its place: docked left, in the inspector's place, floating; resized; remembered; closed.
    r.s.run(DRAG, 30, 200, False)
    r.check('dragging it to the left edge shows where it would dock', wait(lambda: 'Dock left' in r.s.run('return document.body.innerText'), 3))
    r.check('the panel itself follows the pointer while it is dragged', r.s.run(GRIP_AT, 30, 200), panel(r))
    r.shot('dock-preview')
    r.s.run("window.dispatchEvent(new PointerEvent('pointerup', {clientX: 30, clientY: 200, pointerId: 1}))")
    r.check('dropped there, it docks as a column at the left edge', wait(lambda: panel(r)['left'] == 6 and panel(r)['width'] == 360, 3), panel(r))
    r.s.run("document.querySelector('[role=separator][aria-label=\"Resize AI panel\"]').focus()")
    r.key('ArrowRight')
    r.check('the gap beside it resizes it from the keyboard', wait(lambda: panel(r)['width'] == 384, 3), panel(r))
    r.shot('docked-left')
    r.s.run('window.__oldPage = true; location.reload()')
    # Nuzky starts on the home screen; the panel is in the editor behind it.
    wait(lambda: r.s.run("const c = window.__nuzky; if (window.__oldPage || !c?.store.getState().snap) return false; "
                         "c.store.setState({view: 'editor'}); return true", retries=3), 30)
    r.check('after a restart it opens where it was, as wide as it was', wait(lambda: (panel(r) or {}).get('left') == 6 and panel(r)['width'] == 384, 10), panel(r))
    inspector = r.s.run("const b = document.querySelector('aside[aria-label=Inspector]').getBoundingClientRect(); return [b.left + b.width / 2, b.top + b.height / 2]")
    r.s.run(DRAG, inspector[0], inspector[1], True)
    # As wide as the inspector, which with the column gone from the left edge takes 28 % of the window again.
    r.check("dropped on the inspector it takes the inspector's place and width, running down beside the timeline",
            wait(lambda: panel(r)['slot'] == 'inspector' and not panel(r)['inspector'] and panel(r)['width'] == round(panel(r)['windowWidth'] * 0.28)
                 and panel(r)['bottom'] == 6, 3), panel(r))
    r.shot('in-inspector-place')
    r.s.run("document.querySelector('aside[aria-label=AI] button[aria-label=\"Move panel\"]').dispatchEvent(new KeyboardEvent('keydown', {key: 'Enter', bubbles: true}))")
    r.check('the grip also opens the places as a menu', wait(lambda: click(r, 'Floating') is not None, 3))
    p = wait(lambda: (lambda p: p if p and p['slot'] is None and p['inspector'] else None)(panel(r)), 3)
    r.check('floating, it sits over the editor and the inspector is back', p, panel(r))
    r.s.run("document.querySelector('aside[aria-label=AI]').parentElement.querySelector('[role=separator][aria-label=\"Resize AI panel\"]').focus()")
    r.key('ArrowRight')
    r.check('a floating panel resizes from the keyboard too', wait(lambda: panel(r)['width'] == p['width'] + 24, 3), {'before': p['width'], 'after': panel(r)['width']})
    r.shot('floating')
    before = panel(r)
    r.s.run(DRAG, 600, 300, False)
    r.check('a floating panel moves with the pointer, without a docking outline over the open editor',
            wait(lambda: r.s.run(GRIP_AT, 600, 300), 3) and 'Floating' not in r.s.run('return document.body.innerText'), panel(r))
    r.shot('moving')
    r.key('Escape')
    r.check('Esc puts the dragged panel back where it was', wait(lambda: (panel(r)['left'], panel(r)['top']) == (before['left'], before['top']), 3), panel(r))
    r.s.run(DRAG, 600, 300, False)
    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    wait(lambda: panel(r)['windowWidth'] <= 1030, 5)
    r.key('Escape')
    r.check('a drag cancelled after the window shrank leaves the panel inside the window',
            wait(lambda: panel(r)['left'] >= 6 and panel(r)['right'] >= 6, 3), panel(r))
    webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
    wait(lambda: panel(r)['windowWidth'] >= 1430, 5)
    r.s.run(DRAG, 600, 300, False)
    shown = wait(lambda: (lambda q: q if r.s.run(GRIP_AT, 600, 300) else None)(panel(r)), 3)
    r.s.run("window.dispatchEvent(new PointerEvent('pointerup', {clientX: 600, clientY: 300, pointerId: 1}))")
    r.check('let go, it stays exactly where it was shown while dragging', shown and wait(lambda: (panel(r)['left'], panel(r)['top'], panel(r)['width']) == (shown['left'], shown['top'], shown['width']), 3),
            {'shown': shown and {k: shown[k] for k in ('left', 'top', 'width')}, 'after': {k: panel(r)[k] for k in ('left', 'top', 'width')}})
    r.s.run("window.__nuzky.store.setState({view: 'home'})")
    r.check('on the home screen the floating panel does not show over the projects', wait(lambda: (panel(r) or {}).get('width') == 0, 3), panel(r))
    r.check('the projects have no AI button', not r.s.run(
        "return [...document.querySelectorAll('div[aria-label=Projects] header button')].some((b) => b.textContent.trim() === 'AI')"))
    r.shot('home-no-ai')
    r.key('j', ctrlKey=True)
    time.sleep(0.5)
    r.check('Ctrl+J on the projects stays there and opens no project',
            r.s.run('return window.__nuzky.store.getState().view') == 'home' and (panel(r) or {}).get('width') == 0, panel(r))
    r.key('Escape')
    r.check("Esc goes back to the editor with the panel where it was",
            wait(lambda: r.s.run('return window.__nuzky.store.getState().view') == 'editor' and (panel(r) or {}).get('width') == p['width'] + 24, 3), panel(r))
    click(r, 'Close AI (Ctrl+J)')
    r.check('× closes it entirely', wait(lambda: panel(r) is None, 3))
    click(r, 'AI', 'header')
    r.check('AI in the top bar opens it again where it was', wait(lambda: (panel(r) or {}).get('slot') is None and panel(r)['width'] == p['width'] + 24, 3), panel(r))

    # The smallest window has no room for a column beside the editor: it takes the inspector's place.
    r.s.run("document.querySelector('aside[aria-label=AI] button[aria-label=\"Move panel\"]').dispatchEvent(new KeyboardEvent('keydown', {key: 'Enter', bubbles: true}))")
    wait(lambda: click(r, 'Dock right') is not None, 3)
    r.check('docked right again from the menu', wait(lambda: panel(r)['right'] == 6 and panel(r)['inspector'], 3), panel(r))
    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    small = wait(lambda: panel(r)['windowWidth'] <= 1030, 5)
    r.check("in a 1024 x 640 window it takes the inspector's place, and goes back once there is room",
            small and wait(lambda: panel(r)['slot'] == 'inspector', 3), panel(r))
    r.shot('small-window')
    webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
    r.check('back in a wide window it is a column at the right edge again', wait(lambda: panel(r)['right'] == 6 and panel(r)['inspector'], 5), panel(r))

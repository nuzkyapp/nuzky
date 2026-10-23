"""Connect your AI in Get set up: for Claude Code, Codex and Gemini it says whether each is installed, with its version, signed
in and connected to Nuzky. A missing agent shows its official install command with Copy and the install guide, and
Check again finds it once installed; a signed-out one says how to sign in; Connect puts Nuzky into the agent's own
settings and checks that the agent's own command reaches this Nuzky, and when it does not, says why and offers Try
again. Skip leads to cutting by hand in the transcript, which works without any agent."""
import json, time
from pathlib import Path

from e2e.connect import MCP, SETTINGS
from e2e.corrections import Hands
from e2e.harness import FIXTURES, LINUX, clipboard, flow, link_models, press, set_clipboard, wait
from e2e.setup import BOX, FOCUS, dialog, rail

HERE = Path(__file__).parent
CLAUDE = '{\n  "numStartups": 3,\n  "mcpServers": {\n    "other": {\n      "type": "stdio",\n      "command": "other-server"\n    }\n  }\n}\n'
CARDS = """return Object.fromEntries([...document.querySelectorAll('#setup-panel [role=group]')].map((g) => [g.getAttribute('aria-label'), {
  text: g.innerText, command: g.querySelector('code')?.textContent ?? null, alert: g.querySelector('[role=alert]')?.textContent ?? null,
  buttons: [...g.querySelectorAll('button')].map((b) => b.getAttribute('aria-label') || b.textContent.trim())}]));"""
FOCUS_BUTTON = """const b = [...document.querySelectorAll(arguments[1] + ' button')].find((b) => (b.getAttribute('aria-label') || b.textContent.trim()) === arguments[0]);
if (!b) return false; b.focus(); return document.activeElement === b;"""
# The file dialog answers with the video, as a picked file; multiple: false answers one path.
ANSWER_FILE_DIALOG = """const fetch = window.fetch, picked = arguments[0];
window.fetch = (url, init) => decodeURIComponent(String(url)).includes('plugin:dialog|open')
  ? Promise.resolve(new Response(JSON.stringify(picked), {headers: {'Content-Type': 'application/json', 'Tauri-Response': 'ok'}}))
  : fetch(url, init);"""
WORDS = "[role=listbox][aria-label=Transcript] [role=option][data-t]"


def before(r):
    """The agents' settings live in a home folder of this run, and the agents Nuzky finds are stand-ins in its bin."""
    folder = r.work / 'home'
    folder.mkdir(exist_ok=True)
    (folder / '.claude.json').write_text(CLAUDE)
    (folder / '.gemini/config').mkdir(parents=True)
    (folder / '.gemini/antigravity-cli').mkdir(parents=True)
    (folder / '.gemini/config/mcp_config.json').write_text(MCP)
    (folder / '.gemini/antigravity-cli/settings.json').write_text(SETTINGS)
    r.env['HOME'] = str(folder)
    for variable in ('CLAUDE_CONFIG_DIR', 'CODEX_HOME'):
        r.env.pop(variable, None)
    (r.work / 'bin').mkdir()
    r.env.update(NUZKY_AGENT_CLAUDE=str(r.work / 'bin/claude'), NUZKY_AGENT_CODEX=str(r.work / 'bin/codex'),
                 NUZKY_AGENT_AGY=str(r.work / 'bin/agy'))
    link_models(r)


def agent(r, name, gone=False, **answers):
    """Installs the stand-in for `name`, answering as `answers` say (FAKE_AGENT_SIGNED_OUT, FAKE_AGENT_MCP), or
    takes it away."""
    path = r.work / 'bin' / name
    if gone:
        path.unlink(missing_ok=True)
        return
    fake = HERE / {'claude': 'fake_agent', 'codex': 'fake_codex', 'agy': 'fake_agy'}[name]
    path.write_text('#!/bin/sh\n' + ''.join(f'export {k}={v}\n' for k, v in answers.items()) + f'exec "{fake}" "$@"\n')
    path.chmod(0o755)


def cards(r):
    return r.s.run(CARDS, retries=3) or {}


def card_says(r, name, text):
    return text in (cards(r).get(name) or {}).get('text', '')


def press_button(r, label, scope=BOX):
    """Focuses the button and presses Return on it, as a keyboard user does."""
    if not r.s.run(FOCUS_BUTTON, label, scope):
        return False
    press('Return')
    return True


@flow('setup-ai', 'Connect your AI says whether Claude Code, Codex and Gemini are installed, signed in and connected; Copy, Check '
      'again, Connect and a check that the agent reaches Nuzky, a failed connection with its reason, and Skip to cutting '
      'by hand in the transcript', before=before, home=True)
def setup_ai(r):
    theirs = clipboard() if not LINUX else None
    try:
        agents(r, Path(r.env['HOME']))
        skip_to_transcript(r)
    finally:
        if theirs is not None:
            set_clipboard(theirs)


def agents(r, home):
    r.s.run("window.__nuzky.setup.getState().openSetup()")
    wait(lambda: dialog(r), 5)
    press('End')
    d = wait(lambda: (dialog(r) or {}).get('panel') == 'Connect your AI' and dialog(r), 5)
    r.check('the rail lists only the steps that are built', rail(d) == [['Captions model'], ['Face and person models'], ['Music from YouTube'], ['Connect your AI']], rail(d))
    r.check('the header counts four steps', d and d['count'] == '0 of 4 done', d)

    c = wait(lambda: (x := cards(r)) and all('Not installed' in v['text'] for v in x.values()) and x, 10)
    r.check('with no agents installed, all three say Not installed', c and r.s.run("return [...document.querySelectorAll('#setup-panel [role=group]')].map((g) => g.getAttribute('aria-label'))") == ['Claude Code', 'Codex', 'Gemini'], cards(r))
    r.check('each shows its official install command, with Copy and the install guide',
            c and c['Claude Code']['command'] == 'curl -fsSL https://claude.ai/install.sh | bash'
            and c['Codex']['command'] == 'curl -fsSL https://chatgpt.com/codex/install.sh | sh'
            and c['Gemini']['command'] == 'curl -fsSL https://antigravity.google/cli/install.sh | bash'
            and all(c[name]['buttons'] == ['Install guide', 'Copy the command'] for name in ('Claude Code', 'Codex'))
            and c['Gemini']['buttons'] == ['Install guide', 'Details', 'Copy the command'], c)
    r.check('Gemini says why it runs in a terminal', c and "Google doesn't let other apps run Gemini" in c['Gemini']['text'], c)
    d = dialog(r)
    r.check('the footer offers Check again before Skip', d and d['buttons'] == ['Check again', 'Skip'], d)
    r.check('Copy for Claude Code', press_button(r, 'Copy the command', f'{BOX} [role=group][aria-label="Claude Code"]'))
    r.check('puts its install command on the clipboard', wait(lambda: clipboard() == 'curl -fsSL https://claude.ai/install.sh | bash', 3), clipboard())
    r.check('and says Copied', wait(lambda: 'Copied' in cards(r)['Claude Code']['buttons'], 2), cards(r))
    time.sleep(0.3)
    r.shot('setup-ai-missing')
    r.s.run("document.querySelector('#setup-panel [role=group][aria-label=Gemini]').scrollIntoView({block: 'end'})")
    time.sleep(0.3)
    r.shot('setup-ai-gemini-missing')

    # Installed, signed out.
    agent(r, 'claude', FAKE_AGENT_SIGNED_OUT=1)
    r.check('after installing Claude Code, Check again', press_button(r, 'Check again'))
    c = wait(lambda: card_says(r, 'Claude Code', 'Not signed in') and cards(r), 10)
    r.check('finds it with its version, not signed in', c and '2.1.292' in c['Claude Code']['text'], cards(r))
    r.check('and shows how to sign in', c and 'To sign in, run this in' in c['Claude Code']['text'] and c['Claude Code']['command'] == 'claude', c)
    r.check('Codex is still not installed', c and 'Not installed' in c['Codex']['text'], c)
    r.check('nothing offers to connect an agent that is not signed in', c and 'Connect' not in c['Claude Code']['buttons'], c)
    r.shot('setup-ai-signed-out')

    # Signed in, but it starts its servers where they cannot find Nuzky: the connection fails and says why.
    agent(r, 'claude', FAKE_AGENT_MCP='broken')
    press_button(r, 'Check again')
    c = wait(lambda: card_says(r, 'Claude Code', 'Signed in') and cards(r), 10)
    r.check('signed in, Claude Code offers Connect', c and c['Claude Code']['buttons'] == ['Connect'], cards(r))
    r.check('Connect, from the keyboard', press_button(r, 'Connect'))
    c = wait(lambda: card_says(r, 'Claude Code', "Couldn't connect") and cards(r), 30)
    r.check("when Claude Code's own check does not reach Nuzky, it says Couldn't connect, why and what to do",
            c and "Claude Code's check of Nuzky's server failed: it couldn't reach this Nuzky. Try again." == c['Claude Code']['alert'], cards(r))
    r.check('Connect turns into Try again and keeps the focus',
            c and c['Claude Code']['buttons'] == ['Try again'] and r.s.run(FOCUS) == 'Try again', (cards(r), r.s.run(FOCUS)))
    saved = json.loads((home / '.claude.json').read_text())
    r.check("the nuzky entry went into Claude Code's own settings, beside the other server",
            saved['mcpServers']['nuzky']['args'] == ['mcp', '--current', '--allow-write'] and saved['mcpServers']['other']['command'] == 'other-server', saved)
    r.check('after a backup', any(p.name.startswith('.claude.json.nuzky-backup-') for p in home.iterdir()), sorted(p.name for p in home.iterdir()))
    r.check('a failed check does not finish the step', rail(dialog(r))[3] == ['Connect your AI'], dialog(r))
    time.sleep(0.3)
    r.shot('setup-ai-failed')

    # Its servers reach Nuzky now: Try again connects and the step is done.
    agent(r, 'claude')
    press('Return')
    c = wait(lambda: card_says(r, 'Claude Code', 'Connected') and cards(r), 30)
    r.check('Try again: Claude Code is connected and its check passed', c and 'Claude Code sees Nuzky' in c['Claude Code']['text'], cards(r))
    r.check('it says to restart Claude Code if it is open', c and 'restart it so it sees Nuzky' in c['Claude Code']['text'], c)
    d = wait(lambda: (x := dialog(r)) and x['steps'][3]['lines'] == ['Connect your AI', 'Done'] and x, 5)
    r.check('the step is done and the header counts it', d and d['count'] == '1 of 4 done', dialog(r))
    r.check('the focus goes to Done', wait(lambda: r.s.run(FOCUS) == 'Done', 3), r.s.run(FOCUS))
    r.check('Check again stays for Codex and Gemini, still missing, before Done', d and d['buttons'] == ['Check again', 'Done'], d)
    time.sleep(0.3)
    r.shot('setup-ai-connected')

    # The person takes Nuzky out of Claude Code's settings meanwhile: Check again no longer says Connected.
    saved = json.loads((home / '.claude.json').read_text())
    del saved['mcpServers']['nuzky']
    (home / '.claude.json').write_text(json.dumps(saved, indent=2))
    press_button(r, 'Check again')
    c = wait(lambda: card_says(r, 'Claude Code', 'Signed in') and cards(r), 10)
    r.check('with its entry gone, Check again shows Claude Code signed in with Connect, not Connected', c and c['Claude Code']['buttons'] == ['Connect'], cards(r))
    press_button(r, 'Connect')
    r.check('Connect puts it back and its check passes', wait(lambda: card_says(r, 'Claude Code', 'Claude Code sees Nuzky'), 30), cards(r))

    # Codex: Nuzky reads its entry with Codex's own command and starts that server as Codex would.
    agent(r, 'codex')
    press('Escape')
    wait(lambda: not dialog(r), 3)
    r.s.run("window.__nuzky.setup.getState().openSetup()")
    c = wait(lambda: card_says(r, 'Claude Code', 'Connected') and card_says(r, 'Codex', 'Signed in') and cards(r), 30)
    r.check('opened again, Claude Code is checked again and still connected', c and 'Claude Code sees Nuzky' in c['Claude Code']['text'], cards(r))
    r.check('and Codex, installed since, offers Connect', c and c['Codex']['buttons'] == ['Connect'] and '0.162.1' in c['Codex']['text'], c)
    press_button(r, 'Connect', f'{BOX} [role=group][aria-label=Codex]')
    c = wait(lambda: card_says(r, 'Codex', 'Connected') and cards(r), 30)
    r.check('Codex connects and its check passes', c and 'Codex sees Nuzky' in c['Codex']['text'], cards(r))
    config = (home / '.codex/config.toml').read_text()
    r.check("Codex's entry passes on the runtime folder Codex would not hand its server", 'env_vars = ["XDG_RUNTIME_DIR"]' in config, config)
    r.check('with Gemini still missing, Check again stays before Done', (dialog(r) or {}).get('buttons') == ['Check again', 'Done'], dialog(r))

    agent(r, 'agy')
    press('Escape')
    wait(lambda: not dialog(r), 3)
    r.s.run("window.__nuzky.setup.getState().openSetup()")
    c = wait(lambda: card_says(r, 'Claude Code', 'Connected') and card_says(r, 'Codex', 'Connected')
             and card_says(r, 'Gemini', 'Antigravity CLI 1.4.3') and cards(r), 30)
    r.check('opened again, Gemini shows its installed version and offers Connect',
            c and c['Gemini']['buttons'] == ['Connect', 'Details'], cards(r))
    r.check('Connect Gemini from the keyboard', press_button(r, 'Connect', f'{BOX} [role=group][aria-label=Gemini]'))
    # "Checking that Gemini sees Nuzky" comes first; how to ask shows once the check passed.
    c = wait(lambda: card_says(r, 'Gemini', 'ask it, for example') and cards(r), 30)
    r.check('Gemini connects, lists both backups, turns on Agent access and allows its tools',
            c and all(line in c['Gemini']['text'] for line in ('Backed up ~/.gemini/config/mcp_config.json',
                'Backed up ~/.gemini/antigravity-cli/settings.json', 'Added Nuzky and turned on Agent access',
                "Let Gemini use Nuzky's tools without asking")), cards(r))
    commands = r.s.run("return [...document.querySelectorAll('#setup-panel [role=group][aria-label=Gemini] code')].map((c) => c.textContent)")
    r.check('Gemini shows how to ask in a terminal, with the agy Nuzky found and an example',
            commands == [str(r.work / 'bin/agy'), 'Cut the pauses and retakes in the video open in Nuzky, then add captions']
            and c and 'choose Google OAuth' in c['Gemini']['text'] and 'close that window and paste the command in a new one' in c['Gemini']['text'], commands)
    r.check('the focus goes to copying agy, the next thing to do, not to Done', r.s.run(FOCUS) == 'Copy the command', r.s.run(FOCUS))
    r.check('with all three connected, only Done is left in the footer', c and (dialog(r) or {}).get('buttons') == ['Done'], dialog(r))
    r.s.run("document.querySelector('#setup-panel [role=group][aria-label=Gemini]').scrollIntoView({block: 'start'})")
    time.sleep(0.3)
    r.shot('setup-ai-gemini')
    press_button(r, 'Done')
    r.check('Done closes it', wait(lambda: not dialog(r), 3), dialog(r))


def skip_to_transcript(r):
    """With no agent, Skip leads to cutting by hand: a video opens on its transcript, and a word deleted there is cut."""
    agent(r, 'claude', gone=True)
    agent(r, 'codex', gone=True)
    agent(r, 'agy', gone=True)
    r.s.run("window.__nuzky.setup.setState({states: {}, current: 'ai'}); window.__nuzky.setup.getState().openSetup()")
    wait(lambda: card_says(r, 'Codex', 'Not installed'), 10)

    # On Windows, where agents cannot attach to Nuzky yet, the step says so first and shows only the way by hand.
    r.s.run("Object.defineProperty(navigator, 'userAgent', {value: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)', configurable: true})")
    press('Home')
    press('End')
    d = wait(lambda: (x := dialog(r)) and x['panel'] == 'Connect your AI' and x, 5)
    text = r.s.run("return document.querySelector('#setup-panel').innerText") or ''
    r.check("on Windows the step says at once that AI editing isn't available there yet, with no agents to set up",
            [line for line in text.split('\n') if line.strip()][1].startswith("AI editing isn't available on Windows yet")
            and 'Claude Code, Codex and Gemini edit in Nuzky on macOS and Linux' in text
            and 'Cut without AI' in text and not cards(r) and 'Set up Claude Code, Codex or Gemini instead' not in text, text)
    r.check('and offers Open a video… before Skip', d and d['buttons'] == ['Open a video…', 'Skip'], d)
    r.shot('setup-ai-windows')
    r.s.run("delete navigator.userAgent")
    press('Home')
    press('End')
    wait(lambda: card_says(r, 'Codex', 'Not installed'), 10)
    r.check('Skip', press_button(r, 'Skip'))
    d = wait(lambda: (x := dialog(r)) and x['steps'][3]['lines'] == ['Connect your AI', 'Skipped'] and x, 5)
    text = r.s.run("return document.querySelector('#setup-panel').innerText") or ''
    r.check('skipped, the step shows how to cut without AI', 'Cut without AI' in text and 'Remove retakes and pauses' in text and 'Transcribe timeline' in text
            and 'Set up Claude Code, Codex or Gemini instead' in text, text)
    r.check('Open a video… is the way on, and Done is secondary', d and d['buttons'] == ['Open a video…', 'Done'], d)
    r.check('the focus goes to Open a video…', wait(lambda: r.s.run(FOCUS) == 'Open a video…', 3), r.s.run(FOCUS))
    time.sleep(0.3)
    r.shot('setup-ai-skipped')

    r.s.run(ANSWER_FILE_DIALOG, str(FIXTURES / 'talk.mp4'))
    r.s.run("localStorage.setItem('nuzky.speechModel', 'small'); localStorage.setItem('nuzky.speechLanguage', 'en');"
            "window.__nuzky.speech.setState({model: 'small', language: 'en'})")
    press('Return')
    v = wait(lambda: (s := r.s.run("const s = window.__nuzky.store.getState(); return {view: s.view, tab: s.panelTab}")) and s['view'] == 'editor' and s, 20)
    r.check('Open a video… closes Get set up and opens the video in the editor on its transcript',
            v and v['tab'] == 'transcript' and not dialog(r) and len(r.track()) == 1, (v, dialog(r)))
    r.check('Transcribe timeline', wait(lambda: r.s.run("const b = [...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Transcribe timeline');"
                                                       "if (!b) return false; b.click(); return true;"), 10))
    words = wait(lambda: (n := r.s.run(f"return document.querySelectorAll('{WORDS}').length")) and n > 5 and n, 180)
    r.check('the transcript shows the words said', words, words)
    before = sum(c['durationUs'] for c in r.track())
    word = r.s.run(f"const w = document.querySelectorAll('{WORDS}')[2]; w.scrollIntoView({{block: 'center'}}); return {{id: w.id, text: w.textContent}}")
    r.check(f'a click on the word "{word and word["text"]}" selects it', word and Hands(r).click(f'#{word["id"]}')
            and wait(lambda: r.s.run("return document.getElementById(arguments[0])?.getAttribute('aria-selected')", word['id']) == 'true', 3))
    r.key('Delete')
    after = wait(lambda: (n := sum(c['durationUs'] for c in r.track())) < before and n, 10)
    r.check('Delete cuts it out of the video', after, {'before us': before, 'after us': after})
    time.sleep(0.3)
    r.shot('setup-ai-cut-by-hand')
    r.check('no error toast along the way', not r.errors(), r.errors())

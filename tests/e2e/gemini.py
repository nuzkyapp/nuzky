"""Gemini in the AI panel: Google lets no other app run it, so choosing it shows why and leads to it in a terminal.
Install Google's Antigravity CLI (agy), Connect, which puts Nuzky into agy's settings and lets agy use Nuzky's tools
without asking, then ask agy in a terminal. Nuzky never runs agy for anything but its version. The agent agy starts from
that entry edits the open project live, and the panel says so while it does."""
import json, time
from pathlib import Path

from e2e.harness import Bridge, flow, press, wait, webdriver

HERE = Path(__file__).parent
MCP = '{\n  "mcpServers": {\n    "other": {\n      "command": "other-server",\n      "args": ["--x"]\n    }\n  }\n}\n'
SETTINGS = '{\n  "theme": "dark",\n  "permissions": {\n    "allow": ["command(git)"],\n    "deny": ["mcp(other/*)"]\n  }\n}\n'
GUIDE = """const a = document.querySelector('aside[aria-label=AI]');
const steps = a && [...a.querySelectorAll('ol[aria-label="How to use Gemini"] > li')];
return a && {text: a.innerText, title: a.querySelector('h3')?.textContent ?? null, focus: document.activeElement?.textContent ?? null,
  steps: steps.map((s) => ({text: s.innerText, buttons: [...s.querySelectorAll('button')].map((b) => b.getAttribute('aria-label') || b.textContent.trim()),
    commands: [...s.querySelectorAll('code')].map((c) => c.textContent)})),
  field: !!a.querySelector('textarea')};"""
MENU = """return [...document.querySelectorAll('[role=menu] [role^=menuitem]')].map((i) => ({text: i.textContent,
  checked: i.getAttribute('aria-checked'), disabled: i.getAttribute('aria-disabled')}));"""


def before(r):
    """agy's settings live in a home folder of this run; the agy Nuzky finds is a stand-in, not installed at first."""
    home = r.work / 'home'
    (home / '.gemini/config').mkdir(parents=True)
    (home / '.gemini/antigravity-cli').mkdir(parents=True)
    (home / '.gemini/config/mcp_config.json').write_text(MCP)
    (home / '.gemini/antigravity-cli/settings.json').write_text(SETTINGS)
    r.env['HOME'] = str(home)
    (r.work / 'bin').mkdir()
    r.env['NUZKY_AGENT_AGY'] = str(r.work / 'bin/agy')


def guide(r):
    return r.s.run(GUIDE, retries=3) or {}


def click(r, label, scope='aside[aria-label=AI]'):
    return r.s.run("""const b = [...document.querySelectorAll(arguments[1] + ' button')].find((b) => (b.getAttribute('aria-label') || b.textContent.trim()) === arguments[0]);
        if (!b) return false; b.focus(); b.click(); return true;""", label, scope)


def menu(r):
    r.s.run("document.querySelector('aside[aria-label=AI] button[title=\"Choose the agent\"]').click()")
    return wait(lambda: r.s.run(MENU) or None, 5) or []


@flow('gemini', "Gemini in the AI panel: why it runs in a terminal, installing Antigravity CLI, Connect with agy's tools allowed, "
      'and the agent agy starts from that entry editing the open project live', before=before)
def gemini(r):
    home = r.work / 'home'
    r.check('AI in the top bar opens the panel', r.s.run("""const b = [...document.querySelectorAll('header button')].find((b) => b.textContent.trim() === 'AI');
        b.click(); return true;""") and wait(lambda: r.s.run("return !!document.querySelector('aside[aria-label=AI]')"), 5))

    # Without Claude Code or Codex the panel offers ChatGPT, and Gemini under it.
    text = wait(lambda: (t := r.s.run("return document.querySelector('aside[aria-label=AI]').innerText")) and 'Use your ChatGPT plan' in t and t, 10)
    r.check('without Claude Code or Codex, the ChatGPT view offers to set up Gemini under them', text and 'Or use Claude Code or Codex' in text
            and 'Gemini' in text.split('Or use Claude Code or Codex')[1] and 'Set up' in text.split('Or use Claude Code or Codex')[1], text)
    items = menu(r)
    row = next((i for i in items if i['text'].startswith('Gemini')), None)
    r.check('the agent menu lists Gemini after ChatGPT, not installed', row and row['text'] == 'GeminiNot installed'
            and [i['text'].split('Not')[0].split('Signed')[0] for i in items].index('Gemini') == 3, items)
    press('Escape')
    r.check('Set up beside Gemini switches the panel to it', click(r, 'Set up'))
    g = wait(lambda: (x := guide(r)) and x.get('steps') and x, 10)
    r.check('the panel shows how to use Gemini from a terminal, with no field to write in',
            g and g['title'] == 'Use Gemini from a terminal' and not g['field'], g)
    r.check('in two sentences it says why: Google lets no other app run Gemini on the account, so it runs in a terminal and edits here',
            g and "Google doesn't let other apps run Gemini on your Google account" in g['text'] and 'edits the video open in Nuzky, live' in g['text'], g)
    r.check('the focus is on the title, not on a button a stray Enter would press', g and g['focus'] == 'Use Gemini from a terminal', g)
    r.check('step 1, not installed, shows the official install command', g and 'Not installed' in g['steps'][0]['text']
            and g['steps'][0]['commands'] == ['curl -fsSL https://antigravity.google/cli/install.sh | bash']
            and g['steps'][0]['buttons'] == ['Install guide', 'Copy the command', 'Check again'], g)
    r.check('steps 2 and 3 wait', g and g['steps'][1]['buttons'] == [] and g['steps'][2]['commands'] == [], g)
    click(r, 'Details')
    g = guide(r)
    r.check("Details gives Google's reasons: Gemini CLI turned off, Antigravity's terms, no sign-in for other apps",
            '18 June 2026' in g['text'] and "Google's terms call that a breach" in g['text'] and 'unlike ChatGPT, Gemini has no way to let another app use your plan' in g['text'], g)
    time.sleep(0.3)
    r.shot('gemini-not-installed')

    # Installed in a terminal: coming back to Nuzky's window is enough.
    agy = r.work / 'bin/agy'
    agy.symlink_to(HERE / 'fake_agy')
    r.s.run("window.dispatchEvent(new Event('focus'))")
    g = wait(lambda: (x := guide(r)) and 'Installed · 1.4.3' in x['steps'][0]['text'] and x, 10)
    r.check('back in Nuzky, step 1 is done with the version agy reports', g, guide(r))
    r.check('step 2 offers Connect and says what it does', g and g['steps'][1]['buttons'] == ['Connect']
            and 'without asking before each step' in g['steps'][1]['text'], g)
    r.check('the agent menu now says Gemini is not connected', any(i['text'] == 'GeminiNot connected' and i['checked'] == 'true' for i in menu(r)), r.s.run(MENU))
    press('Escape')
    time.sleep(0.3)
    r.shot('gemini-installed')

    r.check('Connect', click(r, 'Connect'))
    g = wait(lambda: (x := guide(r)) and x['steps'][2]['commands'] and x, 30)
    r.check('Connect backs up both settings files, adds Nuzky, allows its tools and checks that the server reaches Nuzky',
            g and all(line in g['steps'][1]['text'] for line in ('\nGemini sees Nuzky', 'Backed up ~/.gemini/config/mcp_config.json', 'Backed up ~/.gemini/antigravity-cli/settings.json',
                                                                  'Added Nuzky and turned on Agent access', "Let Gemini use Nuzky's tools without asking")), guide(r))
    r.check('step 3 says how to ask in a terminal, with the agy Nuzky found and an example',
            g and g['steps'][2]['commands'] == [str(agy), 'Cut the pauses and retakes in the video open in Nuzky, then add captions']
            and 'choose Google OAuth' in g['steps'][2]['text'] and 'close that window and paste the command in a new one' in g['steps'][2]['text'], g)
    r.check('once connected, the why gives way to how to ask', g and "Google doesn't let other apps" not in g['text'], g)
    r.check('the focus moves on from the vanished Connect to copying agy', r.s.run("return document.activeElement?.getAttribute('aria-label')") == 'Copy the command',
            r.s.run("return document.activeElement?.outerHTML.slice(0, 120)"))
    time.sleep(0.3)
    r.shot('gemini-connected')

    config = json.loads((home / '.gemini/config/mcp_config.json').read_text())
    entry = config['mcpServers']['nuzky']
    r.check("agy's servers keep the other one and gain only Nuzky, attached to the open project",
            config['mcpServers']['other'] == json.loads(MCP)['mcpServers']['other'] and entry['args'] == ['mcp', '--current', '--allow-write']
            and set(entry) == {'command', 'args'}, config)
    settings = (home / '.gemini/antigravity-cli/settings.json').read_text()
    r.check("agy's settings keep every other rule and gain only mcp(nuzky/*)",
            settings == SETTINGS.replace('["command(git)"]', '["command(git)", "mcp(nuzky/*)"]'), settings)
    backups = sorted(p.name for p in home.rglob('*.nuzky-backup-*'))
    r.check('both files were backed up first', len(backups) == 2, backups)

    # The server agy starts from that entry: an agent in a terminal edits the open project, and the panel says so.
    agent = Bridge(r, command=[entry['command'], *entry['args']])
    try:
        name = r.s.run('return window.__nuzky.store.getState().snap.project.name')
        r.check('the agent agy starts reads the project open in the app', agent.call('get_state', {})['name'] == name)
        run = agent.call('begin_run', {'label': 'Cut the pauses'})
        g = wait(lambda: (x := guide(r)) and 'An agent outside Nuzky is editing' in x['text'] and x, 10)
        r.check("while it edits, the panel says an agent outside Nuzky is editing and how to stop it", g and 'Cut the pauses. Stop and edit in the top bar stops it.' in g['text'], guide(r))
        time.sleep(0.3)
        r.shot('gemini-editing')
        agent.call('end_run', {'run_id': run['run_id'], 'action': 'discard'})
        r.check('the note goes when the run ends', wait(lambda: 'is editing' not in guide(r)['text'], 10))
    finally:
        agent.close()

    # Opened again later, it shows how to ask, and turning Agent access off shows the way back.
    r.s.run("window.__nuzky.api.setAgentAccess(false, true)")
    click(r, 'Close AI (Ctrl+J)')
    r.s.run("""const b = [...document.querySelectorAll('header button')].find((b) => b.textContent.trim() === 'AI'); b.click();""")
    g = wait(lambda: (x := guide(r)) and x.get('steps') and 'Agent access is off' in x['steps'][1]['text'] and x, 10)
    r.check('with Agent access off, step 2 says so and offers Turn on', g and g['steps'][1]['buttons'] == ['Turn on Agent access'], guide(r))
    r.check('Turn on', click(r, 'Turn on Agent access'))
    g = wait(lambda: (x := guide(r)) and x['steps'][1]['buttons'] == [] and 'Connected' in x['steps'][1]['text'] and x['steps'][2]['commands'] and x, 15)
    r.check('turns Agent access on, also when Nuzky starts, and step 3 is back',
            g and r.s.call('window.__nuzky.api.agentAccess().then(JSON.stringify)')['value'] == '{"on":true,"atLaunch":true}', guide(r))
    r.check('the agent menu says Gemini is connected', any(i['text'] == 'GeminiConnected' for i in menu(r)), r.s.run(MENU))
    press('Escape')

    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    wait(lambda: r.s.run('return window.innerWidth') == 1024, 5)
    time.sleep(0.5)
    r.shot('gemini-1024')
    webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
    # Back in its column the panel is drawn anew and quietly checks the connection again; that check ends first.
    wait(lambda: (x := guide(r)) and x.get('steps') and x['steps'][2]['commands'], 10)
    time.sleep(1)

    # agy turned Nuzky off meanwhile (`agy mcp disable nuzky`): back in Nuzky, the guide no longer says Connected.
    mcp = home / '.gemini/config/mcp_config.json'
    config = json.loads(mcp.read_text())
    config['mcpServers']['nuzky']['disabled'] = True
    mcp.write_text(json.dumps(config, indent=2))
    r.s.run("window.dispatchEvent(new Event('focus'))")
    g = wait(lambda: (x := guide(r)) and x['steps'][1]['buttons'] == ['Connect'] and x, 10)
    r.check('with Nuzky turned off in agy, step 2 offers Connect again and step 3 hides', g and not g['steps'][2]['commands'], guide(r))
    r.check('Connect', click(r, 'Connect'))
    r.check('Connect turns it back on', wait(lambda: (x := guide(r)) and x['steps'][2]['commands'], 30)
            and 'disabled' not in mcp.read_text(), (mcp.read_text(), guide(r)))

    # A settings file agy cannot start with: the check says so, though Nuzky's server would answer.
    settings_file = home / '.gemini/antigravity-cli/settings.json'
    good = settings_file.read_text()
    settings_file.write_text('{"permissions":')
    click(r, 'Close AI (Ctrl+J)')
    r.s.run("""const b = [...document.querySelectorAll('header button')].find((b) => b.textContent.trim() === 'AI'); b.click();""")
    g = wait(lambda: (x := guide(r)) and x.get('steps') and "Couldn't connect" in x['steps'][1]['text'] and x, 15)
    r.check("with agy's settings broken, step 2 says it couldn't connect and why, with Try again",
            g and "settings.json isn't valid JSON, so agy can't start" in g['steps'][1]['text'] and g['steps'][1]['buttons'] == ['Try again'], guide(r))
    # A rule of the person's own that denies a Nuzky tool wins over the one Connect added, so the check says so.
    settings_file.write_text(good.replace('"deny": ["mcp(other/*)"]', '"deny": ["mcp(other/*)", "mcp(nuzky/apply_edits)"]'))
    click(r, 'Close AI (Ctrl+J)')
    r.s.run("""const b = [...document.querySelectorAll('header button')].find((b) => b.textContent.trim() === 'AI'); b.click();""")
    g = wait(lambda: (x := guide(r)) and x.get('steps') and "Couldn't connect" in x['steps'][1]['text'] and x, 15)
    r.check("with a rule in agy's settings that denies a Nuzky tool, step 2 names it",
            g and "denies or asks before Nuzky's tools (mcp(nuzky/apply_edits))" in g['steps'][1]['text'], guide(r))
    settings_file.write_text(good)
    r.check('no error toast', not r.errors(), r.errors())

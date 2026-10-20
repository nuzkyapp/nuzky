"""Connect your agent: one click puts Nuzky into Claude Code's, Codex's and Gemini's own settings, after a backup and without
touching anything else there, a copy gives any other agent the same server and a first message, and Agent access
decides whether an agent attached to the open project may edit it, without restarting the agent."""
import json, subprocess, time

from e2e.harness import AI_EDITING, MACOS, Bridge, clipboard, flow, press, wait, webdriver

CLAUDE = '{\n  "numStartups": 12,\n  "projects": {\n    "/home/me/site": {\n      "allowedTools": []\n    }\n  },\n  "mcpServers": {\n    "other": {\n      "type": "stdio",\n      "command": "other-server"\n    }\n  }\n}\n'
CODEX = '# my Codex settings\nmodel = "gpt-6.1-sol" # keep this\n\n[mcp_servers.other]\ncommand = "other"\n\n[profiles.fast]\nmodel_reasoning_effort = "low"\n'
MCP = '{\n  "mcpServers": {\n    "other": {\n      "command": "other-server",\n      "args": ["--x"]\n    }\n  }\n}\n'
SETTINGS = '{\n  "theme": "dark",\n  "permissions": {\n    "allow": ["command(git)"],\n    "deny": ["mcp(other/*)"]\n  }\n}\n'
ROWS = """return [...document.querySelectorAll('[role=dialog] [role=group]')]
  .filter((row) => ['Claude Code', 'Codex', 'Gemini'].includes(row.getAttribute('aria-label')))
  .map((row) => ({text: row.textContent, button: row.querySelector('button')?.textContent}));"""
ACCESS = """const row = document.querySelector('[role=dialog] [role=group][aria-label="Agent access"]');
const toggle = row && row.querySelector('[role=switch]');
return toggle && {on: toggle.getAttribute('aria-checked') === 'true', atLaunch: row.querySelector('input[type=checkbox]').checked,
  locked: toggle.getAttribute('aria-disabled') === 'true', why: toggle.title, text: row.textContent};"""


def home(r):
    """The agents' settings live in a home folder of this run, never the user's own."""
    folder = r.work / 'home'
    (folder / '.codex').mkdir(parents=True)
    (folder / '.claude.json').write_text(CLAUDE)
    (folder / '.codex/config.toml').write_text(CODEX)
    (folder / '.gemini/config').mkdir(parents=True)
    (folder / '.gemini/antigravity-cli').mkdir(parents=True)
    (folder / '.gemini/config/mcp_config.json').write_text(MCP)
    (folder / '.gemini/antigravity-cli/settings.json').write_text(SETTINGS)
    r.env['HOME'] = str(folder)
    for variable in ('CLAUDE_CONFIG_DIR', 'CODEX_HOME'):
        r.env.pop(variable, None)


def click(r, label, scope='document'):
    return r.s.run(f"""const b = [...{scope}.querySelectorAll('button')].find((b) => b.textContent.trim() === arguments[0]);
        if (!b) return false; b.focus(); b.click(); return true;""", label)


def copy(r, label):
    """Presses the copy button for real, as the clipboard takes text only from something the user did, and returns
    the clipboard and the toast."""
    r.s.run("window.__nuzky.store.setState({toasts: []})")
    r.s.run("[...document.querySelectorAll('[role=dialog] button')].find((b) => b.textContent.trim() === arguments[0]).focus()", label)
    press('Return')
    toast = wait(lambda: r.state()['toasts'], 5) or [{}]
    return clipboard(), toast[0]


def tool(agent, name, arguments):
    """A tool call that may fail, as the agent sees it."""
    return agent.rpc('tools/call', {'name': name, 'arguments': arguments})['result']


def server_of(text):
    return json.loads(text[text.index('{'):text.rindex('}') + 1])['mcpServers']['nuzky']


@flow('connect', 'Connect an agent outside Nuzky (in the AI panel) writes only the nuzky entry into Claude Code, Codex and Gemini settings, '
      'after a backup, a copy gives other agents the same server, and Agent access lets the agent edit the open project live',
      before=home)
def connect(r):
    folder = r.work / 'home'
    # On macOS the copies land on the person's own clipboard; their text goes back at the end.
    theirs = clipboard() if MACOS else None
    agent = None
    try:
        # Connect agent lives in the AI panel's agent menu: AI in the top bar, the agent's name, then the item.
        r.check('AI in the top bar opens the panel', click(r, 'AI', "document.querySelector('header')")
                and wait(lambda: r.s.run("return !!document.querySelector('aside[aria-label=AI]')"), 5))
        r.check('its agent menu offers to connect an agent outside Nuzky',
                r.s.run("document.querySelector('aside[aria-label=AI] button[title=\"Choose the agent\"]').click(); return true")
                and wait(lambda: r.s.run("""const item = [...document.querySelectorAll('[role=menu] [role=menuitem]')]
                    .find((b) => b.textContent.includes('Connect an agent outside Nuzky')); if (!item) return false; item.click(); return true;"""), 5))
        rows = wait(lambda: r.s.run(ROWS) or None, 10)
        r.check('the dialog lists Claude Code, Codex and Gemini, not yet connected',
                rows and len(rows) == 3 and all(name in row['text'] and 'Not connected' in row['text']
                for name, row in zip(('Claude Code', 'Codex', 'Gemini'), rows)), rows)
        access = wait(lambda: r.s.run(ACCESS), 5)
        r.check('Agent access starts off and is not turned on at launch', access and not access['on'] and not access['atLaunch']
                and "can't edit" in access['text'], access)
        time.sleep(0.3)  # the dialog fades in over 200 ms
        r.shot('connect-dialog')

        # Copies for agents Nuzky does not connect itself: the server for their settings and a first message.
        text, toast = copy(r, 'Copy MCP config')
        server = server_of(text) if text and '{' in text else {}
        r.check('Copy MCP config copies a server that runs this app attached to the open project',
                server.get('args') == ['mcp', '--current', '--allow-write'] and server.get('command', '').endswith('nuzky-app'), text)
        r.check('it says to add only the nuzky entry to a file that has servers, then gives a first message: read nuzky://guide, '
                'edit only through the tools, a run is one undo step',
                'add only the "nuzky" entry' in text and 'nuzky://guide' in text and 'begin_run' in text and 'one undo step' in text, text)
        r.check('the toast says what was copied and that Agent access is off',
                toast.get('kind') == 'success' and 'Turn on Agent access' in toast.get('text', ''), toast)
        for label, place in (('Copy for Claude', 'Claude Desktop config (Settings, Developer, Edit Config)'), ('Copy for Cursor', '~/.cursor/mcp.json')):
            text, _ = copy(r, label)
            r.check(f'{label} copies the same server and says where it goes', place in text and server_of(text) == server, text)
        r.check('copying changes no file', (folder / '.claude.json').read_text() == CLAUDE
                and (folder / '.codex/config.toml').read_text() == CODEX
                and (folder / '.gemini/config/mcp_config.json').read_text() == MCP
                and (folder / '.gemini/antigravity-cli/settings.json').read_text() == SETTINGS
                and not list(folder.rglob('*.nuzky-backup-*'))
                and not (r.work / 'data/nuzky/agent-access.json').exists())

        # An agent set up from that copy, the way its settings start it.
        agent = Bridge(r, command=[server['command'], *server['args']])
        refused = tool(agent, 'get_state', {})
        r.check('with Agent access off the agent is refused, saying how to turn it on',
                refused.get('isError') and 'AGENT_ACCESS_OFF' in json.dumps(refused) and 'Agent access is off' in json.dumps(refused), refused)
        r.s.run("document.querySelector('[role=dialog] [role=switch]').focus()")
        press('space')
        r.check('Space on the switch turns Agent access on, not at launch', wait(lambda: (r.s.run(ACCESS) or {}).get('on'), 5)
                and not r.s.run(ACCESS)['atLaunch'], r.s.run(ACCESS))
        open_name = r.s.run('return window.__nuzky.store.getState().snap.project.name')
        state = tool(agent, 'get_state', {})
        r.check('the same agent, not restarted, now reads the project open in the app',
                not state.get('isError') and state['structuredContent']['name'] == open_name, state)
        run = agent.call('begin_run', {'label': 'Connected agent'})
        r.check("the agent's run shows in the app", wait(lambda: r.state()['aiRun'], 10))
        locked = r.s.run(ACCESS)
        r.check('while it edits, the switch is locked and says why', locked['locked'] and locked['why'] == AI_EDITING, locked)
        agent.call('end_run', {'run_id': run['run_id'], 'action': 'discard'})
        r.check('and the run ends there', wait(lambda: not r.state()['aiRun'], 10))
        r.check('the switch unlocks', wait(lambda: not r.s.run(ACCESS)['locked'], 5))
        r.s.run("document.querySelector('[role=dialog] [role=switch]').focus()")
        press('space')
        r.check('Space again turns it off', wait(lambda: not r.s.run(ACCESS)['on'], 5), r.s.run(ACCESS))
        # The app lets go of the agent within its next 20 ms round, so a call may still pass just before.
        r.check('and the agent is refused again', wait(lambda: 'AGENT_ACCESS_OFF' in json.dumps(tool(agent, 'get_state', {})), 5),
                tool(agent, 'get_state', {}))

        focused = """const el = document.activeElement; return el && el.closest('[role=dialog]')
            ? {text: el.textContent.trim(), row: el.closest('[role=group]')?.getAttribute('aria-label') ?? null} : null;"""
        for name, next_row in (('Claude Code', 'Codex'), ('Codex', 'Gemini'), ('Gemini', None)):
            next_focus = {'text': 'Connect' if next_row else 'Done', 'row': next_row}
            r.s.run("""const row = document.querySelector(`[role=dialog] [role=group][aria-label="${arguments[0]}"]`);
                row.querySelector('button').focus(); row.querySelector('button').click();""", name)
            r.check(f'{name} shows connected', wait(lambda: any(name in x['text'] and 'Connected' in x['text'] for x in r.s.run(ROWS)), 10),
                    r.s.run(ROWS))
            # A connected row has no button, so keyboard focus moves on inside the dialog instead of getting lost.
            r.check(f"after {name}, focus moves to {next_row + ' Connect' if next_row else 'Done'}", wait(lambda: r.s.run(focused) == next_focus, 3), r.s.run(focused))
        toast = wait(lambda: next((t for t in r.state()['toasts'] if t['text'].startswith('Gemini is connected.')), None), 5)
        r.check('after Gemini connects, the toast says to run agy in a terminal',
                toast and toast['kind'] == 'success' and 'Run agy in a terminal' in toast['text'], toast)
        access = r.s.run(ACCESS)
        saved = r.work / 'data/nuzky/agent-access.json'
        r.check('connecting turns Agent access on, and on at every launch, so a connected agent keeps working',
                access['on'] and access['atLaunch'] and saved.exists() and json.loads(saved.read_text()) == {'atLaunch': True}, access)
        state = tool(agent, 'get_state', {})
        r.check('the agent from the copy can edit again, still not restarted', not state.get('isError'), state)
        r.shot('connected')
        r.s.run("document.querySelector('[role=dialog] [role=switch]').focus()")
        press('space')
        r.check('with access off, a connected agent does not look ready', wait(lambda: all(
            'Connected. Agent access is off' in x['text'] for x in r.s.run(ROWS)), 5), r.s.run(ROWS))
        press('space')
        r.check('and with access back on it does', wait(lambda: all(
            'Agent access is off' not in x['text'] and 'Connected' in x['text'] for x in r.s.run(ROWS)), 5), r.s.run(ROWS))
        webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
        wait(lambda: r.s.run('return window.innerWidth') == 1024, 5)
        time.sleep(0.5)
        box = r.s.run("const b = document.querySelector('[role=dialog]').getBoundingClientRect(); "
                      "return {top: b.top, bottom: b.bottom, height: innerHeight};")
        r.check('in the smallest window, 1024 x 640, the whole dialog fits', box['top'] >= 0 and box['bottom'] <= box['height'], box)
        r.shot('connected-1024')
        webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
    finally:
        if agent:
            agent.close()
        if theirs is not None:
            subprocess.run(['pbcopy'], input=theirs, text=True, check=True)

    claude = (folder / '.claude.json').read_text()
    config = json.loads(claude)
    entry = config['mcpServers'].pop('nuzky')
    r.check("Claude Code's other settings are untouched", config == json.loads(CLAUDE), config)
    backups = sorted(folder.glob('.claude.json.nuzky-backup-*'))
    r.check("Claude Code's settings were backed up first", len(backups) == 1 and backups[0].read_text() == CLAUDE, backups)
    codex = (folder / '.codex/config.toml').read_text()
    block = f'[mcp_servers.nuzky]\ncommand = {json.dumps(entry["command"])}\nargs = ["mcp", "--current", "--allow-write"]\nenv_vars = ["XDG_RUNTIME_DIR"]\n\n'
    r.check("Codex gets the same command and keeps every comment and setting", block in codex and codex.replace(block, '') == CODEX,
            codex)
    backups = sorted((folder / '.codex').glob('config.toml.nuzky-backup-*'))
    r.check("Codex's settings were backed up first", len(backups) == 1 and backups[0].read_text() == CODEX, backups)
    gemini = (folder / '.gemini/config/mcp_config.json').read_text()
    config = json.loads(gemini)
    gemini_entry = config['mcpServers'].pop('nuzky')
    r.check("Gemini gets only the same command and args and keeps the other server's exact text",
            gemini_entry == {'command': entry['command'], 'args': entry['args']}
            and config == json.loads(MCP) and gemini.startswith(MCP.removesuffix('\n  }\n}\n') + ','), gemini)
    settings = (folder / '.gemini/antigravity-cli/settings.json').read_text()
    r.check("Gemini's settings keep every other byte and gain only mcp(nuzky/*)",
            settings == SETTINGS.replace('["command(git)"]', '["command(git)", "mcp(nuzky/*)"]'), settings)
    for relative, original in (('.gemini/config/mcp_config.json', MCP), ('.gemini/antigravity-cli/settings.json', SETTINGS)):
        path = folder / relative
        backups = sorted(path.parent.glob(path.name + '.nuzky-backup-*'))
        r.check(f"Gemini's {path.name} was backed up once with its original text",
                len(backups) == 1 and backups[0].read_text() == original, backups)
    r.check('the entry runs this app attached to the open project, as the copy does',
            entry['args'] == ['mcp', '--current', '--allow-write'] and entry['command'] == server['command'], entry)

    # What Claude Code starts from that entry: an MCP server that edits the project open here.
    connected = Bridge(r, command=[entry['command'], *entry['args']])
    try:
        state = connected.call('get_state', {})
        r.check('the agent Claude Code starts sees the project open in the app', state['name'] == open_name, [state['name'], open_name])
    finally:
        connected.close()
    r.check('Done closes the dialog', click(r, 'Done') and wait(lambda: not r.s.run("return !!document.querySelector('[role=dialog]')"), 5))
    r.check('no error toast', not r.errors(), r.errors())


def asked_at_launch(r):
    (r.work / 'data/nuzky/agent-access.json').write_text('{"atLaunch": true}')


@flow('connect_at_launch', 'With "Turn on when Nuzky starts" set, Agent access is on as Nuzky starts and an agent attached to the '
      'open project edits it at once', before=asked_at_launch)
def connect_at_launch(r):
    access = r.s.call('window.__nuzky.api.agentAccess().then(JSON.stringify)')
    r.check('Agent access is on from the start', access['ok'] and json.loads(access['value']) == {'on': True, 'atLaunch': True}, access)
    text = r.s.call("window.__nuzky.api.agentSetup('other')")['value']
    agent = Bridge(r, command=[server_of(text)['command'], *server_of(text)['args']])
    try:
        name = r.s.run('return window.__nuzky.store.getState().snap.project.name')
        r.check('an agent attached with --current reads the open project', agent.call('get_state', {})['name'] == name)
    finally:
        agent.close()

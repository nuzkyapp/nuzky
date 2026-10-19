"""Send feedback. The foot of the home sidebar and the editor's top bar open one menu. Report a bug asks what happened,
shows what the report includes, then opens the repository's bug form in the browser with the version and its commit,
the system, the description and a summary of the report filled in, and saves the whole report in the logs folder to
drag into the issue. Share an idea opens the idea form with the version, and Message me on X opens the author's profile.
Copy diagnostics and Open logs folder have their own flow (diagnostics). Nothing is sent; the flow records the pages
the app would open instead of a browser and keeps what is copied in the page."""
import platform, re, sys, time, urllib.parse
from pathlib import Path

from e2e.harness import MACOS, flow, wait, webdriver

MENU = """const m = document.querySelector('[role=menu][aria-label="Send feedback"]');
return m && [...m.querySelectorAll('[role^=menuitem]')].map((i) => i.textContent);"""
CHOOSE = """const i = [...document.querySelectorAll('[role=menu][aria-label="Send feedback"] [role^=menuitem]')]
  .find((i) => i.textContent.startsWith(arguments[0]));
if (!i) return false; i.click(); return true;"""
# A click without a pointer, as Enter or Space on the focused control sends it.
OPEN = "const b = document.querySelector(arguments[0]); b.focus(); b.click();"
IN_MENU = "return !!document.activeElement?.closest('[role=menu][aria-label=\"Send feedback\"]')"
FOCUSED = "return document.activeElement === document.querySelector(arguments[0])"
TOASTS = "return window.__nuzky.store.getState().toasts.map((t) => ({kind: t.kind, text: t.text}));"
ROW, BUTTON = 'nav + div [data-feedback]', 'header [data-feedback]'
DIALOG = 'dialog[aria-labelledby=bug-report-title]'
DIALOG_TEXT = "const d = document.querySelector(arguments[0]); return d?.open ? d.textContent : null"
TYPE = """const t = document.querySelector('#bug-report-text');
Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(t, arguments[0]);
t.dispatchEvent(new Event('input', {bubbles: true}));"""
CLICK = """const b = [...document.querySelectorAll(arguments[0] + ' button')].find((b) => b.textContent.trim().startsWith(arguments[1]));
if (!b) return false; b.click(); return true;"""
REPORT = "return document.querySelector('[data-report]')?.textContent ?? ''"
FITS = "const r = document.querySelector(arguments[0]).getBoundingClientRect(); return r.top >= 0 && r.bottom <= window.innerHeight;"
DESCRIPTION = 'After I split the clip, the sound plays half a second late.\nIt happens on every phone video.'
ITEMS = ['Report a bug…', 'Share an idea…', 'Copy diagnostics', 'Open logs folder', 'Message me on X…']
# The page's clipboard keeps what is copied, so the flow neither reads nor overwrites the person's own.
CLIPBOARD = """window.__copied = [];
Object.defineProperty(navigator, 'clipboard', {configurable: true, value: {writeText: async (t) => { window.__copied.push(t); }}});"""


def record_opened(r):
    r.opened = r.work / 'opened.txt'
    r.env['NUZKY_OPENED_URLS'] = str(r.opened)


def opened(r):
    return r.opened.read_text().splitlines() if r.opened.exists() else []


def system():
    """What the app should name this computer as, as a person reading the issue would want it."""
    if MACOS:
        return f'macOS {platform.mac_ver()[0]}'
    if sys.platform.startswith('linux'):
        try:
            name = platform.freedesktop_os_release()['PRETTY_NAME']
            return f'Linux ({name})'
        except (OSError, KeyError):
            return 'Linux'
    return platform.system()


def choose(r, opener, item):
    """Opens the menu from `opener` and picks `item`; returns the page it opened."""
    before = len(opened(r))
    r.s.run(OPEN, opener)
    if not wait(lambda: r.s.run(MENU), 3) or not r.s.run(CHOOSE, item):
        return None
    wait(lambda: len(opened(r)) > before, 5)
    return opened(r)[before] if len(opened(r)) > before else None


def issue(url):
    parts = urllib.parse.urlsplit(url or '')
    query = urllib.parse.parse_qs(parts.query)
    return f'{parts.scheme}://{parts.netloc}{parts.path}', {k: v[0] for k, v in query.items()}


@flow('feedback', 'Send feedback opens a prefilled GitHub issue for a bug or an idea, or the X profile, with nothing sent',
      before=record_opened, home=True)
def feedback(r):
    r.s.run(CLIPBOARD)
    version = r.s.run('return window.__nuzky.updates.getState().version')
    this_system = f'{system()} · {platform.machine().replace("arm64", "aarch64")}'
    form = 'https://github.com/nuzkyapp/nuzky/issues/new'

    r.check('the foot of the home sidebar offers Send feedback, above the version',
            r.s.run("""const f = document.querySelector(arguments[0]), v = document.querySelector('nav + div p, [data-version]');
                       return !!f && f.textContent.trim() === 'Send feedback' && !!v && f.getBoundingClientRect().bottom <= v.getBoundingClientRect().top;""", ROW))
    r.s.run(OPEN, ROW)
    menu = wait(lambda: r.s.run(MENU), 3)
    r.check('it opens a menu: Report a bug, Share an idea, Copy diagnostics, Open logs folder and Message me on X', menu == ITEMS, menu)
    r.check('opened from the keyboard, the menu has the focus', wait(lambda: r.s.run(IN_MENU), 3))
    time.sleep(0.3)  # The menu shows once it has been placed.
    r.shot('home-menu')
    r.key('Escape')
    r.check('Esc closes the menu and focus returns to the row', wait(lambda: not r.s.run(MENU) and r.s.run(FOCUSED, ROW), 3))
    r.check('opening the menu opened nothing', opened(r) == [], opened(r))

    before = len(opened(r))
    r.s.run(OPEN, ROW)
    wait(lambda: r.s.run(MENU), 3)
    r.s.run(CHOOSE, 'Report a bug')
    r.check('Report a bug asks what happened, with the focus in the text field and nothing opened yet',
            wait(lambda: r.s.run(DIALOG_TEXT, DIALOG), 5) and r.s.run('return document.activeElement?.id') == 'bug-report-text'
            and len(opened(r)) == before, r.s.run(DIALOG_TEXT, DIALOG))
    r.check('folded, it says what the report includes',
            wait(lambda: 'What the report includesVersion, system, graphics, ' in (r.s.run(DIALOG_TEXT, DIALOG) or ''), 10)
            and 'your last actions, the log' in r.s.run(DIALOG_TEXT, DIALOG), r.s.run(DIALOG_TEXT, DIALOG))
    r.s.run(TYPE, DESCRIPTION)
    time.sleep(0.4)  # The dialog has faded in.
    r.shot('report-form')
    r.s.run(CLICK, DIALOG, 'What the report includes')
    report = wait(lambda: (lambda t: t if t.startswith('Nuzky') else None)(r.s.run(REPORT)), 10) or ''
    r.check('unfolded, it shows the whole report: the version with its commit, system, graphics, memory and the log',
            report.startswith(f'Nuzky {version} (') and f'\nSystem: {this_system}\nGraphics: ' in report
            and re.search(r'^Memory: \d+ GB$', report, re.M) and re.search(r'^Last \d+ lines of the log:$', report, re.M), report[:400])
    time.sleep(0.2)
    r.shot('report-unfolded')
    r.s.run(CLICK, DIALOG, 'Continue on GitHub')
    name = wait(lambda: r.s.run("return document.querySelector('[data-report-name]')?.textContent"), 20)
    page, query = issue(opened(r)[-1] if len(opened(r)) > before else None)
    r.check('Continue on GitHub opens the bug form with the version and commit, system, description and a summary',
            page == form and query.get('template') == 'bug_report.yml' and re.fullmatch(rf'{re.escape(version)} \(\w+(-dirty)?\)', query.get('version', ''))
            and query.get('system') == this_system and query.get('what-happened') == DESCRIPTION
            and query.get('diagnostics', '').startswith(f'Nuzky {version} (') and len(opened(r)) == before + 1, (page, query))
    saved = r.work / 'data/nuzky/logs' / (name or 'missing')
    r.check('and saves the whole report, starting with the description, in the logs folder',
            name and re.fullmatch(r'nuzky-report-[\d-]+\.txt', name) and saved.exists()
            and saved.read_text().startswith(f'What happened:\n{DESCRIPTION}\n\nNuzky {version} ('), name)
    time.sleep(0.2)
    r.shot('report-saved')
    r.s.run(CLICK, DIALOG, 'Show in folder')
    shown = wait(lambda: len(opened(r)) > before + 1, 5) and opened(r)[-1]
    r.check('Show in folder shows the saved report in the file manager',
            shown and shown.startswith('reveal ') and Path(shown[len('reveal '):]).resolve() == saved.resolve(), shown)
    r.s.run(CLICK, DIALOG, 'Done')
    r.check('Done closes it', wait(lambda: r.s.run(DIALOG_TEXT, DIALOG) is None, 3))

    page, query = issue(choose(r, ROW, 'Share an idea'))
    r.check('Share an idea opens the feature request form with this version',
            page == form and query == {'template': 'feature_request.yml', 'version': version}, (page, query))

    url = choose(r, ROW, 'Message me on X')
    r.check('Message me on X opens the profile on X', url == 'https://x.com/mach_builds', url)
    r.check('each choice opened exactly one page and showed no error',
            len(opened(r)) == 4 and [t for t in r.s.run(TOASTS) if t['kind'] == 'error'] == [], (opened(r), r.s.run(TOASTS)))

    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    if wait(lambda: r.s.run('return window.innerWidth') == 1024, 5):
        r.check('in a 1024 x 640 window Send feedback still sits whole in the sidebar foot',
                r.s.run("""const b = document.querySelector(arguments[0]).getBoundingClientRect();
                           const nav = document.querySelector('nav[aria-label=Collections]').getBoundingClientRect();
                           return b.bottom <= window.innerHeight - 6 && b.top >= nav.bottom && b.width > 180;""", ROW))
        r.shot('home-small')
        r.s.run(OPEN, ROW)
        wait(lambda: r.s.run(MENU), 3)
        r.s.run(CHOOSE, 'Report a bug')
        wait(lambda: r.s.run(DIALOG_TEXT, DIALOG), 5)
        r.s.run(CLICK, DIALOG, 'What the report includes')
        wait(lambda: r.s.run(REPORT).startswith('Nuzky'), 10)
        time.sleep(0.4)
        r.check('unfolded in a 1024 x 640 window, the Report a bug dialog fits whole', r.s.run(FITS, DIALOG))
        r.shot('report-small')
        r.key('Escape')
        wait(lambda: r.s.run(DIALOG_TEXT, DIALOG) is None, 3)
    webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})

    r.s.run("window.__nuzky.store.setState({view: 'editor'})")
    r.check('in the editor a button next to Projects says Send feedback',
            wait(lambda: r.s.run("return document.querySelector(arguments[0])?.getAttribute('aria-label')", BUTTON), 3) == 'Send feedback')
    r.s.run(OPEN, BUTTON)
    menu = wait(lambda: r.s.run(MENU), 3)
    r.check('it opens the same menu, with the focus in it', menu == ITEMS and wait(lambda: r.s.run(IN_MENU), 3), menu)
    time.sleep(0.3)
    r.shot('editor-menu')
    r.key('Escape')
    r.check('Esc closes it and focus returns to the button', wait(lambda: not r.s.run(MENU) and r.s.run(FOCUSED, BUTTON), 3))
    before = len(opened(r))
    r.s.run(OPEN, BUTTON)
    wait(lambda: r.s.run(MENU), 3)
    r.s.run(CHOOSE, 'Report a bug')
    r.check('Report a bug from the editor asks the same', wait(lambda: r.s.run(DIALOG_TEXT, DIALOG), 5))
    r.key('Escape')
    r.check('Esc closes it without opening anything, and focus goes back to the button',
            wait(lambda: r.s.run(DIALOG_TEXT, DIALOG) is None and r.s.run(FOCUSED, BUTTON), 3) and len(opened(r)) == before,
            (r.s.run(DIALOG_TEXT, DIALOG) is None, r.s.run('return document.activeElement?.outerHTML.slice(0, 160)'), opened(r)[before:]))
    r.shot('editor')

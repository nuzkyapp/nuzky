"""Send feedback. The foot of the home sidebar and the editor's top bar open one menu: Report a bug and Share an idea
open the repository's issue form in the browser with this version (and for a bug the system) filled in, Report a bug
also copies the diagnostics to paste there, and Message me on X opens the author's profile. Copy diagnostics and Open
logs folder have their own flow (diagnostics). Nothing is sent; the flow records the pages the app would open instead of
a browser and keeps what is copied in the page."""
import platform, sys, time, urllib.parse

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

    page, query = issue(choose(r, ROW, 'Report a bug'))
    r.check('Report a bug opens the bug report form with this version, system and architecture',
            page == form and query == {'template': 'bug_report.yml', 'version': version, 'system': this_system}, (page, query))
    copied = wait(lambda: r.s.run('return window.__copied'), 3)
    r.check('and copies the diagnostics to paste into it', copied and copied[-1].startswith(f'Nuzky {version}\nSystem: {this_system}\n'), copied)

    page, query = issue(choose(r, ROW, 'Share an idea'))
    r.check('Share an idea opens the feature request form with this version',
            page == form and query == {'template': 'feature_request.yml', 'version': version}, (page, query))

    url = choose(r, ROW, 'Message me on X')
    r.check('Message me on X opens the profile on X', url == 'https://x.com/mach_builds', url)
    r.check('each choice opened exactly one page and showed no error',
            len(opened(r)) == 3 and [t for t in r.s.run(TOASTS) if t['kind'] == 'error'] == [], (opened(r), r.s.run(TOASTS)))

    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    if wait(lambda: r.s.run('return window.innerWidth') == 1024, 5):
        r.check('in a 1024 x 640 window Send feedback still sits whole in the sidebar foot',
                r.s.run("""const b = document.querySelector(arguments[0]).getBoundingClientRect();
                           const nav = document.querySelector('nav[aria-label=Collections]').getBoundingClientRect();
                           return b.bottom <= window.innerHeight - 6 && b.top >= nav.bottom && b.width > 180;""", ROW))
        r.shot('home-small')
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
    page, query = issue(choose(r, BUTTON, 'Report a bug'))
    r.check('Report a bug from the editor opens the same prefilled form',
            page == form and query.get('template') == 'bug_report.yml' and query.get('system') == this_system, (page, query))
    time.sleep(0.5)
    r.check('focus goes back to the button after a choice', r.s.run(FOCUSED, BUTTON))
    r.shot('editor')

"""Buttons that open a web page: Install beside Claude Code, Codex and OpenCode in the AI panel and Install guide in
Get set up's Connect your AI open the agent's own guide in the browser. When the browser cannot open, Get set up says
so in the agent's card, since toasts sit under dialogs, and the AI panel in an error toast; both name the page, and a
click on Copy link puts its address on the clipboard. The flow records the pages the app would open instead of a
browser."""
import time

from e2e.corrections import Hands
from e2e.harness import LINUX, clipboard, flow, press, set_clipboard, wait, webdriver
from e2e.setup import BOX, dialog
from e2e.setup_ai import cards, press_button

CLAUDE_CODE, CODEX = 'https://code.claude.com/docs/en/setup', 'https://learn.chatgpt.com/docs/codex/cli'
OPENCODE, ANTIGRAVITY = 'https://opencode.ai/docs', 'https://antigravity.google/docs/getting-started'
PANEL_TEXT = "return document.querySelector('aside[aria-label=AI]')?.innerText ?? ''"
INSTALLS = """return [...document.querySelectorAll('aside[aria-label=AI] button')].filter((b) => b.textContent.trim() === 'Install')
  .map((b) => b.parentElement.firstChild.textContent.trim());"""
CLICK_INSTALL = """const b = [...document.querySelectorAll('aside[aria-label=AI] button')]
  .find((b) => b.textContent.trim() === 'Install' && b.parentElement.firstChild.textContent.trim() === arguments[0]);
if (!b) return false; b.click(); return true;"""
TOASTS = "return window.__nuzky.store.getState().toasts.map((t) => ({kind: t.kind, text: t.text, action: t.action?.label ?? null}));"
MARK_COPY = """const b = [...document.querySelectorAll('[data-toast] button')].find((b) => b.textContent.trim() === 'Copy link');
if (!b) return false; b.dataset.copyLink = ''; return true;"""
FAILED = "Couldn't open your browser. Go to {} in it."
CARD_ALERT = "return document.querySelector(`${arguments[0]} [role=group][aria-label=\"Claude Code\"] [role=alert]`)?.textContent ?? null"


def before(r):
    r.opened = r.work / 'opened.txt'
    r.env['NUZKY_OPENED_URLS'] = str(r.opened)


def opened(r):
    return r.opened.read_text().splitlines() if r.opened.is_file() else []


def page_after(r, act):
    """Does `act` and returns the page the app opened for it, or None when it opened nothing."""
    before = len(opened(r))
    if not act():
        return None
    wait(lambda: len(opened(r)) > before, 5)
    return opened(r)[before] if len(opened(r)) > before else None


@flow('web-pages', "Install in the AI panel and Install guide in Get set up open the agent's guide in the browser, and a "
      'browser that does not open is named in place with Copy link', before=before)
def web_pages(r):
    theirs = clipboard() if not LINUX else None
    try:
        ai_panel(r)
        get_set_up(r)
        no_browser(r)
    finally:
        if theirs is not None:
            set_clipboard(theirs)


def ai_panel(r):
    r.s.run("[...document.querySelectorAll('header button')].find((b) => b.textContent.trim() === 'AI').click()")
    rows = wait(lambda: 'Use your ChatGPT plan' in r.s.run(PANEL_TEXT) and r.s.run(INSTALLS), 10)
    r.check('without an agent installed, the AI panel offers Install for Claude Code, Codex and OpenCode',
            rows == ['Claude Code', 'Codex', 'OpenCode'], (rows, r.s.run(PANEL_TEXT)))
    time.sleep(0.3)
    r.shot('ai-panel-install')
    for name, page in (('Claude Code', CLAUDE_CODE), ('Codex', CODEX), ('OpenCode', OPENCODE)):
        got = page_after(r, lambda: r.s.run(CLICK_INSTALL, name))
        r.check(f"Install beside {name} opens its guide in the browser", got == page, got)
    r.check('opening them showed no error', r.errors() == [], r.errors())


def get_set_up(r):
    r.s.run("window.__nuzky.setup.getState().openSetup()")
    wait(lambda: dialog(r), 5)
    press('End')
    c = wait(lambda: (dialog(r) or {}).get('panel') == 'Connect your AI' and (x := cards(r)) and len(x) == 3
             and all('Not installed' in v['text'] for v in x.values()) and x, 10)
    r.check("Get set up's Connect your AI offers Install guide for each missing agent",
            c and all('Install guide' in c[name]['buttons'] for name in ('Claude Code', 'Codex', 'Gemini')), c)
    for name, page in (('Claude Code', CLAUDE_CODE), ('Codex', CODEX), ('Gemini', ANTIGRAVITY)):
        got = page_after(r, lambda: press_button(r, 'Install guide', f'{BOX} [role=group][aria-label="{name}"]'))
        r.check(f"Install guide for {name}, from the keyboard, opens its guide in the browser", got == page, got)
    r.check('opening them showed no error', r.errors() == [], r.errors())



def no_browser(r):
    """The browser cannot open: the app's record of what it opens is a folder it cannot write to. In the smallest
    window, where a dialog covers the toasts."""
    r.opened.unlink(missing_ok=True)
    r.opened.mkdir()
    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    wait(lambda: r.s.run('return innerWidth') <= 1030, 5)
    hands = Hands(r)
    press_button(r, 'Install guide', f'{BOX} [role=group][aria-label="Claude Code"]')
    said = wait(lambda: r.s.run(CARD_ALERT, BOX), 5)
    r.check("in Get set up, the card says the browser didn't open, names the page and offers Copy link, with no toast under the dialog",
            said == FAILED.format('code.claude.com/docs/en/setup') + ' Copy link' and r.s.run(TOASTS) == [], (said, r.s.run(TOASTS)))
    r.s.run("document.querySelector(arguments[0] + ' [data-copy-link]')?.scrollIntoView({block: 'nearest'})", BOX)
    time.sleep(0.3)
    r.shot('browser-failed-setup')
    r.check('a click on Copy link in the open dialog', said and hands.click(f'{BOX} [data-copy-link]'))
    r.check("puts the guide's address on the clipboard and says Copied", wait(lambda: clipboard() == CLAUDE_CODE, 3)
            and wait(lambda: r.s.run(CARD_ALERT, BOX).endswith(' Copied'), 3), (clipboard(), r.s.run(CARD_ALERT, BOX)))

    press('Escape')
    wait(lambda: not dialog(r), 3)
    r.s.run(CLICK_INSTALL, 'Codex')
    toast = wait(lambda: next((t for t in r.s.run(TOASTS) if t['text'] == FAILED.format('learn.chatgpt.com/docs/codex/cli')), None), 5)
    r.check('in the AI panel, an error toast says so, names the page and offers Copy link',
            toast and toast['kind'] == 'error' and toast['action'] == 'Copy link', r.s.run(TOASTS))
    time.sleep(0.3)
    r.shot('browser-failed-panel')
    r.check('a click on Copy link', r.s.run(MARK_COPY) and hands.click('[data-toast] [data-copy-link]'))
    r.check("puts the guide's address on the clipboard", wait(lambda: clipboard() == CODEX, 3), clipboard())
    r.check('and says it was copied', wait(lambda: any(t['text'] == 'Link copied. Paste it into your browser.' and t['kind'] == 'success'
                                                       for t in r.s.run(TOASTS)), 3), r.s.run(TOASTS))

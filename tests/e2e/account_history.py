"""The account keeps every synced version of Your style and My caption styles: the History dialog lists them with the
computer each came from, shows what restoring one would change, and Restore brings it back on this computer and the
others. The other computer is this flow, signed in to the same account over HTTP, as in account_sync."""
import time

from e2e.account import browse, click, close_settings, needs, open_account, setup, shot, view
from e2e.account_sync import MINE, THEIRS, OtherComputer, caption, edit_md, mine, set_style
from e2e.harness import flow, wait
from e2e.home import resize
from e2e.style import answer

DIALOG = '[role=dialog][data-history]'
IMAC = {'computer': 'Studio iMac', 'computerId': 'urn:uuid:imac', 'label': 'Added your rule'}
LIST = f"""return [...document.querySelectorAll('{DIALOG} [role=option]')].map((o) => o.innerText.replace(/\\n/g, ' | '));"""
PREVIEW = f"""const d = document.querySelector('{DIALOG}');
return d && {{text: d.innerText, back: [...d.querySelectorAll('[data-diff=back]')].map((l) => l.textContent),
              goes: [...d.querySelectorAll('[data-diff=goes]')].map((l) => l.textContent)}};"""
CHOOSE = f"""const o = [...document.querySelectorAll('{DIALOG} [role=option]')].find((o) => o.innerText.includes(arguments[0])
  && o.innerText.includes(arguments[1]));
if (!o) return false; o.click(); return true;"""
BUTTON = f"""const b = [...document.querySelectorAll('{DIALOG} button')].find((b) => b.textContent.trim() === arguments[0]);
if (!b || b.getAttribute('aria-disabled') === 'true' || b.disabled) return false; b.click(); return true;"""


def options(r):
    return r.s.run(LIST) or []


def preview(r):
    return r.s.run(PREVIEW) or {}


def tab(r, name):
    r.s.run(f"[...document.querySelectorAll('{DIALOG} [data-seg]')].find((b) => b.textContent.trim() === arguments[0]).click()", name)


@flow('account-history', 'The History dialog lists every synced version of Your style and caption styles with the computer it came '
      'from, shows what Restore changes, and Restore brings an older style and a deleted caption style back on every computer',
      before=setup, needs=needs)
def account_history(r):
    try:
        resize(r, 1440, 900)
        set_style(r, MINE)
        r.s.call('window.__nuzky.api.saveCaptionStyle(arguments[0])', caption('Team yellow', '#ffd400'))
        open_account(r)
        browse(r, 'signup')
        click(r, 'Continue with email')
        wait(lambda: 'Sign out' in view(r).get('text', ''), 20)
        other = OtherComputer(r.endpoints['url'])
        r.check('signing in syncs this computer\'s style and caption style', wait(lambda: other.style() == MINE and other.captions() == ['Team yellow'], 15))

        other.write('style', {'text': THEIRS, 'accepted': {}, 'frozen': {}, 'atMs': int(time.time() * 1000), 'label': 'Added your rule'}, IMAC)
        other.write('caption-styles', {'styles': []}, {**IMAC, 'label': 'Deleted Team yellow'})
        click(r, 'Sync now')
        r.check("the other computer's style and deletion arrive", wait(lambda: edit_md(r) == THEIRS and mine(r) == [], 15), edit_md(r))

        click(r, 'History')
        r.check('History in Settings › Account opens the dialog', wait(lambda: r.s.run(f"return !!document.querySelector('{DIALOG}')"), 5))
        listed = wait(lambda: (o := options(r)) and len(o) >= 4 and o, 10)
        r.check('it lists both kinds, newest first, with the computer each came from and which is now',
                listed and 'Studio iMac' in listed[0] and '· now' in listed[0] and any('this computer' in o for o in listed), listed)
        r.check('this computer\'s first style is chosen and its preview shows what Restore changes',
                r.s.run(CHOOSE, 'Your style', 'this computer') and wait(lambda: preview(r).get('goes') == ['- Never cut a sentence in half.'], 10), preview(r))
        shot(r, 'history-style')
        r.check('Restore this version is offered for an older version', r.s.run(BUTTON, 'Restore this version'))
        r.check('Restore brings the older style back here', wait(lambda: edit_md(r) == MINE, 10), edit_md(r))
        versions = answer(r, 'window.__nuzky.api.styleView()')['versions']
        r.check('as a version named after what it restored', versions[0]['label'].startswith('Restored: '), versions[:2])
        r.check('and the other computer gets it as the newest version', wait(lambda: other.style() == MINE, 15), other.style())

        open_account(r)
        click(r, 'History')
        wait(lambda: len(options(r)) >= 5, 10)
        tab(r, 'Caption styles')
        r.check('Caption styles lists only caption style versions', wait(lambda: all('Your style' not in o for o in options(r)) and len(options(r)) >= 2, 5), options(r))
        r.s.run(CHOOSE, 'Team yellow', 'this computer') or r.s.run(CHOOSE, '', 'this computer')
        r.check('the version from before the deletion says Team yellow comes back',
                wait(lambda: 'Team yellow' in preview(r).get('text', '') and 'Comes back' in preview(r).get('text', ''), 10), preview(r))
        shot(r, 'history-captions')
        r.s.run(BUTTON, 'Restore this version')
        r.check('Restore brings the deleted caption style back here', wait(lambda: mine(r) == ['Team yellow'], 10), mine(r))
        r.check('and on the other computer', wait(lambda: other.captions() == ['Team yellow'], 15), other.captions())
        r.check('the dialog closes after restoring', wait(lambda: not r.s.run(f"return !!document.querySelector('{DIALOG}')"), 5))
        close_settings(r)
    finally:
        if r.server.poll() is None:
            r.server.terminate()
            r.server.wait(timeout=10)

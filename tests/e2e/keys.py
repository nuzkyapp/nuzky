"""Settings › Keyboard: a key recorded for Split splits and the old one does not, a key another action holds moves only
after Use it here, tooltips name the new key, a set of another editor (DaVinci Resolve) puts its keys in place, and a
Premiere Pro file takes its keys over that set; a file of no editor changes nothing and says so."""
import time

from e2e.harness import FIXTURES, flow, press, wait
from e2e.home import resize

OPEN = "return !!document.querySelector('[role=dialog][data-settings]')"
# Settings is open on its Keyboard pane.
DIALOG = "return document.querySelector('[role=dialog][data-settings] #settings-title')?.textContent === 'Keyboard'"
KEYBOARD_TAB = "document.querySelector('#settings-tab-keyboard').click()"
# The menu's item is what the pointer would hit at its middle, and the menu has the keyboard.
MENU_ON_TOP = """const item = [...document.querySelectorAll('[role=menu] [role^=menuitem]')].find((el) => el.textContent.includes('DaVinci Resolve'));
if (!item) return false;
const b = item.getBoundingClientRect();
return item.contains(document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2)) && !!document.activeElement?.closest('[role=menu]');"""
KEYS = 'return JSON.parse(JSON.stringify(window.__nuzky.keys.getState()))'
SAVED = "return JSON.parse(localStorage.getItem('nuzky.keys'))"
RECORD = "const b = document.querySelector(`[data-keys='${arguments[0]}']`); b.focus(); b.click();"
CONFLICT = "return document.querySelector(`[data-keys-list] [role=alert]`)?.textContent ?? null"
CLICK_TEXT = """const b = [...document.querySelectorAll('[role=dialog] button, [role=menu] [role^=menuitem]')]
  .find((el) => el.textContent.trim() === arguments[0]); b?.click(); return !!b;"""
SPLIT_TIP = "return document.querySelector('[aria-label^=\"Split at playhead\"]')?.getAttribute('aria-label')"
STATUS = "return document.querySelector('[role=dialog] [role=status]')?.textContent ?? null"
LAUNCHER = 'return window.__nuzky.store.getState().launcherOpen'
# The native file dialog cannot be driven, so it answers with the given path (as in home.py).
ANSWER_FILE_DIALOG = """const fetch = window.__realFetch ?? (window.__realFetch = window.fetch), picked = arguments[0];
window.fetch = (url, init) => decodeURIComponent(String(url)).includes('plugin:dialog|open')
  ? Promise.resolve(new Response(JSON.stringify(picked), {headers: {'Content-Type': 'application/json', 'Tauri-Response': 'ok'}}))
  : fetch(url, init);"""


def kys(items):
    """A Premiere Pro keyboard set as Premiere writes it: each item a command, its modifiers and its key."""
    body = ''.join(
        f'<item.{i} Version="1"><commandname>{command}</commandname><modifier.shift>false</modifier.shift>'
        f'<modifier.alt>false</modifier.alt><modifier.ctrl>{"true" if ctrl else "false"}</modifier.ctrl>'
        f'<virtualkey>{0x80000000 | ord(key)}</virtualkey></item.{i}>'
        for i, (command, ctrl, key) in enumerate(items))
    return (f'<?xml version="1.0" encoding="UTF-8"?><PremiereData Version="3"><shortcuts Version="5"><platform>windows'
            f'</platform><context.global Version="1"><itemcount>{len(items)}</itemcount>{body}</context.global>'
            '</shortcuts></PremiereData>')


def clips(r):
    return len(r.track())


def split_with(r, key, **modifiers):
    """Presses the keys with the playhead over the middle of the one clip and says whether it split; undoes a split."""
    r.s.run('window.__nuzky.store.getState().select([])')
    r.s.run('document.activeElement?.blur()')
    r.seek(1_500_000)
    time.sleep(0.3)
    r.key(key, **modifiers)
    split = wait(lambda: clips(r) == 2, 3)
    if split:
        r.key('z', ctrlKey=True)
        wait(lambda: clips(r) == 1, 5)
    return split


@flow('keys', 'Settings › Keyboard: record a key for Split, move a key from another action, a set of another editor, '
      'a Premiere Pro file, and tooltips that name the keys as they are')
def keys(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    r.check('S splits with the keys Nuzky starts with', split_with(r, 's'))

    r.key(',', ctrlKey=True)
    r.check('Ctrl+, opens Settings', wait(lambda: r.s.run(OPEN), 5))
    r.s.run(KEYBOARD_TAB)
    r.check('its Keyboard pane lists the keys', wait(lambda: r.s.run(DIALOG) and r.s.run("return !!document.querySelector('[data-keys=split]')"), 5))
    time.sleep(0.5)
    r.shot('dialog')
    r.s.run(RECORD, 'split')
    r.check('clicking Split\'s keys waits for new ones', 'Press the new keys' in (r.s.run("return document.activeElement.textContent") or ''))
    time.sleep(0.5)
    r.shot('recording')
    r.key('b', ctrlKey=True)
    r.check('Ctrl+B is now Split\'s only key', wait(lambda: r.s.run(KEYS)['changed'].get('split') == ['Ctrl+B'], 3), r.s.run(KEYS))

    r.s.run(RECORD, 'split')
    r.key('k', ctrlKey=True)
    conflict = wait(lambda: r.s.run(CONFLICT), 3)
    r.check('Ctrl+K asks first, as Projects holds it', conflict and 'Ctrl+K is the key of Projects' in conflict and 'Projects will have no key' in conflict, conflict)
    r.check('nothing moves before the answer', r.s.run(KEYS)['changed'].get('split') == ['Ctrl+B'], r.s.run(KEYS))
    time.sleep(0.5)
    r.shot('conflict')
    r.s.run(CLICK_TEXT, 'Use it here')
    state = r.s.run(KEYS)
    r.check('Use it here gives Ctrl+K to Split and leaves Projects without a key',
            state['changed'].get('split') == ['Ctrl+K'] and state['changed'].get('projects') == [], state)
    r.check('the keys stay for the next launch', r.s.run(SAVED) == state, r.s.run(SAVED))

    r.key('Escape')
    r.check('Esc closes it', wait(lambda: not r.s.run(OPEN), 3))
    r.check('the Split button\'s tooltip names Ctrl+K', r.s.run(SPLIT_TIP) == 'Split at playhead (Ctrl+K)', r.s.run(SPLIT_TIP))
    r.check('S no longer splits', not split_with(r, 's'))
    r.check('Ctrl+K splits and opens no launcher', split_with(r, 'k', ctrlKey=True) and not r.s.run(LAUNCHER))

    # Another editor's set: DaVinci Resolve splits with Ctrl+\ and deletes left of the playhead with Ctrl+Shift+[.
    r.key(',', ctrlKey=True)
    r.check('Ctrl+, opens Settings again on Keyboard, the pane shown last', wait(lambda: r.s.run(DIALOG), 5))
    # Real key presses: Enter on Keys like, then type-ahead and Enter in its menu, which must lie over the dialog.
    r.s.run("document.querySelector('[role=dialog] [data-preset]').focus()")
    press('Return')
    time.sleep(0.5)
    r.shot('menu')
    r.check('Keys like opens its menu over the dialog, with the keyboard in it', r.s.run(MENU_ON_TOP), r.s.run(MENU_ON_TOP))
    press('d')
    press('Return')
    r.check('choosing DaVinci Resolve from the keyboard takes its set', wait(lambda: r.s.run(KEYS)['preset'] == 'resolve', 3), r.s.run(KEYS))
    state = r.s.run(KEYS)
    r.check('choosing it drops the changes made by hand', state == {'preset': 'resolve', 'changed': {}}, state)
    status = r.s.run(STATUS)
    r.check('and says so, with Undo', status and 'Your 2 changed keys are gone.' in status, status)
    time.sleep(0.5)
    r.shot('resolve')
    r.key('Escape')
    wait(lambda: not r.s.run(OPEN), 3)
    r.check('Ctrl+\\ splits in the Resolve set', split_with(r, '\\', ctrlKey=True))
    r.s.run("const field = document.createElement('input'); field.id = 'typing'; document.body.append(field); field.focus()")
    r.key('!', code='Digit1', shiftKey=True)
    time.sleep(0.5)
    r.check("typing ! in a text field types, though Shift+1 is Projects' key in this set", not r.s.run(LAUNCHER))
    r.s.run("document.getElementById('typing').remove()")
    r.s.run("window.__nuzky.store.setState({launcherOpen: true})")
    wait(lambda: r.s.run("return document.activeElement?.getAttribute('role') === 'combobox'"), 3)
    r.key('!', code='Digit1', shiftKey=True)
    time.sleep(0.5)
    r.check("typing ! in the launcher's search keeps it open", r.s.run(LAUNCHER))
    r.key('Escape')
    wait(lambda: not r.s.run(LAUNCHER), 3)
    whole = r.track()[0]['durationUs']
    r.seek(1_500_000)
    time.sleep(0.3)
    r.key('{', code='BracketLeft', ctrlKey=True, shiftKey=True)
    left = wait(lambda: (t := r.track()) and abs(t[0]['durationUs'] - (whole - 1_500_000)) < 50_000 and t, 3)
    r.check('Ctrl+Shift+[ deletes the part left of the playhead, named by the key\'s place', left, r.track())
    if left:
        r.key('z', ctrlKey=True)

    # A Premiere Pro file: Split on X, Duplicate on W (which Premiere's set gives Delete right), Export on Ctrl+M.
    path = r.work / 'mine.kys'
    path.write_text(kys([('cmd.sequence.razorateditline', False, 'X'), ('cmd.edit.duplicate', False, 'W'),
                         ('cmd.file.export.movie', True, 'M')]))
    r.key(',', ctrlKey=True)
    wait(lambda: r.s.run(DIALOG), 5)
    r.s.run(ANSWER_FILE_DIALOG, str(path))
    r.s.run(CLICK_TEXT, 'Import file…')
    status = wait(lambda: r.s.run(STATUS), 5)
    state = r.s.run(KEYS)
    r.check('the file says what it took and which action lost its key',
            status == "Imported 3 keys from “mine.kys” over Premiere Pro's keys. Delete right of the playhead has no key now.Undo", status)
    r.check('its keys go over Premiere Pro\'s set', state['preset'] == 'premiere' and state['changed'].get('split') == ['X']
            and state['changed'].get('duplicate') == ['W'] and state['changed'].get('deleteRight') == [], state)
    time.sleep(0.5)
    r.shot('imported')
    if resize(r, 1024, 640):
        time.sleep(0.5)
        r.shot('imported-1024')
        resize(r, 1440, 900)

    bad = r.work / 'notes.txt'
    bad.write_text('just some notes\n')
    r.s.run(ANSWER_FILE_DIALOG, str(bad))
    r.s.run(CLICK_TEXT, 'Import file…')
    status = wait(lambda: (s := r.s.run(STATUS)) and 'notes.txt' in s and s, 5)
    r.check('a file of no editor says so and changes nothing',
            status and status.startswith("Couldn't import “notes.txt”. This file has no keyboard shortcuts") and r.s.run(KEYS) == state, status)
    r.key('Escape')
    wait(lambda: not r.s.run(OPEN), 3)
    r.check('X splits after the import', split_with(r, 'x'))
    r.check('no error toast', not r.errors(), r.errors())

    # The gear in the editor's top bar opens it too.
    r.shot('topbar')
    r.s.run("document.querySelector('header button[aria-label^=\"Settings\"]')?.click()")
    r.check("the top bar's gear opens it", wait(lambda: r.s.run(DIALOG), 5))
    r.key('Escape')
    wait(lambda: not r.s.run(OPEN), 3)

    # The home screen's Settings row opens the same window.
    r.s.run("window.__nuzky.store.setState({view: 'home'})")
    time.sleep(1)
    r.s.run("[...document.querySelectorAll('nav[aria-label=Collections] button')].find((b) => b.textContent.trim() === 'Settings')?.click()")
    r.check("the home screen's Settings row opens it", wait(lambda: r.s.run(DIALOG), 5))
    time.sleep(0.5)
    r.shot('home')

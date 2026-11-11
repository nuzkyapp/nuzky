"""Actions in the launcher: Ctrl+K (Cmd+K) finds the editor's actions by English or Czech words, parts of words and words
with a typo, shows each one's key, Enter runs it or moves the keyboard to its control, a row that cannot run says why,
Esc closes it, typing in a text field types, and another editor's keys keep their own jobs."""
import time

from e2e.harness import FIXTURES, Bridge, flow, press, wait
from e2e.home import TYPE, resize

OPEN = 'return window.__nuzky.store.getState().launcherOpen'
ROWS = """return [...document.querySelectorAll('#launcher-list [role=group][aria-label=Actions] [role=option]')].map((o) => ({
  text: o.textContent, chosen: o.getAttribute('aria-selected') === 'true', disabled: o.getAttribute('aria-disabled') === 'true'}))"""
CHOSEN = "return document.querySelector('#launcher-list [role=option][aria-selected=true]')?.textContent ?? null"
GROUPS = "return [...document.querySelectorAll('#launcher-list [role=group]')].filter((g) => g.querySelector('[role=option]')).map((g) => g.getAttribute('aria-label'))"
FOCUSED = "const el = document.activeElement; return el ? {tag: el.tagName, data: Object.keys(el.dataset), text: el.textContent, role: el.getAttribute('role')} : null"
TOASTS = 'return window.__nuzky.store.getState().toasts.map((t) => t.text)'


def open_launcher(r, **modifiers):
    r.s.run('document.activeElement?.blur()')
    r.key('k', **(modifiers or {'ctrlKey': True}))
    return wait(lambda: r.s.run(OPEN) and r.s.run("return document.activeElement?.getAttribute('role') === 'combobox'"), 5)


def search(r, text):
    """Types into the launcher's field and waits for the first row to settle."""
    r.s.run(TYPE, '[role=combobox]', text)
    time.sleep(0.3)
    return r.s.run(CHOSEN)


def clips(r):
    return len(r.track())


@flow('launcher-actions', "Ctrl+K finds the editor's actions in English and Czech, by part of a word and with a typo, shows their "
      'keys, runs one on Enter or moves to its control, says why one cannot run, closes on Esc, leaves typing alone, '
      "and takes the launcher's key of another editor's set")
def launcher_actions(r):
    resize(r, 1440, 900)
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    r.s.run('window.__nuzky.store.getState().select([])')
    r.seek(1_500_000)

    r.check('Ctrl+K opens the launcher with the keyboard in its field', open_launcher(r))
    placeholder = r.s.run("return document.querySelector('[role=combobox]').placeholder")
    r.check('its field says it finds actions', placeholder == 'Search actions, projects and what was said', placeholder)
    r.check('with nothing typed it lists projects, not actions', not r.s.run(ROWS), r.s.run(ROWS))

    # Real key presses into the field: a typo of Split.
    for ch in 'splt':
        press(ch)
    first = wait(lambda: (c := r.s.run(CHOSEN)) and 'Split at the playhead' in c and c, 5)
    r.check('typing "splt" finds Split first, despite the typo', first, r.s.run(ROWS))
    rows = r.s.run(ROWS)
    r.check('the row says where Split lives and shows its key, S', rows and rows[0]['text'] == 'Split at the playheadTimelineS', rows)
    r.check('the key hints say Enter runs it', '↵ run' in (r.s.run("return document.querySelector('[role=dialog][aria-label=Projects]').textContent") or ''))
    r.check('Actions come before the other results', (r.s.run(GROUPS) or [None])[0] == 'Actions', r.s.run(GROUPS))
    time.sleep(0.3)
    r.shot('typo')
    press('Return')
    r.check('Enter closes the launcher and splits the clip under the playhead', wait(lambda: clips(r) == 2 and not r.s.run(OPEN), 5), r.track())
    r.key('z', ctrlKey=True)
    wait(lambda: clips(r) == 1, 5)

    # Czech, without accents and with them, and part of a word.
    open_launcher(r, metaKey=True)
    r.check('Cmd+K opens it too', r.s.run(OPEN))
    r.check('"rozdelit" finds Split', 'Split at the playhead' in (search(r, 'rozdelit') or ''), r.s.run(ROWS))
    r.check('"přiblížit časovou osu" finds Zoom in', 'Zoom in' in (search(r, 'přiblížit časovou osu') or ''), r.s.run(ROWS))
    r.check('"titul" finds Add captions', 'Add captions' in (search(r, 'titul') or ''), r.s.run(ROWS))
    chosen = search(r, 'přidat titulky')
    r.check('"přidat titulky" finds Add captions first', chosen and 'Add captions' in chosen, r.s.run(ROWS))
    r.check('nothing was said, so only the action shows', r.s.run(GROUPS) == ['Actions'] and 'Said in your videos' not in r.s.run(
        "return document.querySelector('#launcher-list').textContent"), r.s.run(GROUPS))
    time.sleep(0.3)
    r.shot('czech')
    press('Return')
    focused = wait(lambda: (f := r.s.run(FOCUSED)) and 'generateCaptions' in f['data'] and f, 5)
    r.check('Enter on Add captions opens the Captions tab with the keyboard on Generate captions',
            focused and r.s.run('return window.__nuzky.store.getState().panelTab') == 'captions' and not r.s.run(OPEN), r.s.run(FOCUSED))
    time.sleep(0.4)
    r.shot('captions')

    # Zoom in by its name; ↓ moves to the next row, Enter runs what is chosen.
    zoom = r.s.run('return window.__nuzky.store.getState().zoom')
    open_launcher(r)
    search(r, 'zoom')
    rows = r.s.run(ROWS)
    r.check('"zoom" lists Zoom in and Zoom out with their keys', rows and 'Zoom in' in rows[0]['text'] and rows[0]['text'].endswith('+')
            and any('Zoom out' in row['text'] and row['text'].endswith('-') for row in rows), rows)
    press('Down')
    r.check('↓ moves to Zoom out', 'Zoom out' in (r.s.run(CHOSEN) or ''), r.s.run(CHOSEN))
    press('Up')
    press('Return')
    r.check('Enter zooms the timeline in', wait(lambda: r.s.run('return window.__nuzky.store.getState().zoom') > zoom, 5))

    # Export with a swapped pair of letters opens the Export dialog.
    open_launcher(r)
    r.check('"exprot" finds Export', 'Export' in (search(r, 'exprot') or ''), r.s.run(ROWS))
    press('Return')
    r.check('Enter opens the Export dialog', wait(lambda: r.s.run('return window.__nuzky.store.getState().exportOpen'), 5))
    r.s.run("window.__nuzky.store.setState({exportOpen: false})")
    time.sleep(0.3)

    # Nothing selected: Duplicate says why and does nothing.
    r.s.run('window.__nuzky.store.getState().select([])')
    open_launcher(r)
    search(r, 'duplicate')
    rows = r.s.run(ROWS)
    r.check('Duplicate with nothing selected says why, and is off', rows and rows[0]['disabled'] and 'Select a clip first.' in rows[0]['text'], rows)
    time.sleep(0.3)
    r.shot('disabled')
    toasts = r.s.run(TOASTS)
    press('Return')
    r.check('Enter on it lights its reason up, duplicates nothing and keeps the launcher open',
            wait(lambda: r.s.run("return !!document.querySelector('#launcher-list [data-nudged]')"), 2)
            and clips(r) == 1 and r.s.run(OPEN) and r.s.run(TOASTS) == toasts, r.s.run(TOASTS))
    search(r, 'rotate')
    rows = r.s.run(ROWS)
    r.check("a button's own reason reads as a sentence: Rotate 90° says to select a clip first", rows and rows[0]['text'] == 'Rotate 90°Select a clip first.', rows)
    press('Escape')
    r.check('Esc closes the launcher', wait(lambda: not r.s.run(OPEN), 3))

    # A button that opens a menu is pressed, with the keyboard left on it.
    open_launcher(r)
    r.check('"pomer stran" finds Ratio', 'Ratio' in (search(r, 'pomer stran') or ''), r.s.run(ROWS))
    press('Return')
    r.check('Enter opens the Ratio menu', wait(lambda: r.s.run("return !!document.querySelector('[role=menu][aria-label=\"Canvas ratio\"]')"), 3))
    press('Escape')
    wait(lambda: not r.s.run("return !!document.querySelector('[role=menu][aria-label=\"Canvas ratio\"]')"), 3)

    # Typing in a text field types; with Ctrl the launcher opens, and Esc gives the field back as it was.
    r.s.run("const field = document.createElement('input'); field.id = 'typing'; field.value = 'My title'; document.body.append(field); field.focus()")
    r.key('k')
    time.sleep(0.3)
    r.check('K typed in a text field opens nothing', not r.s.run(OPEN))
    r.key('k', ctrlKey=True)
    r.check('Ctrl+K from a text field opens the launcher', wait(lambda: r.s.run(OPEN), 3))
    press('k')
    time.sleep(0.3)
    r.check("K typed in the launcher's field types and keeps it open",
            r.s.run(OPEN) and r.s.run("return document.querySelector('[role=combobox]').value") == 'k')
    press('Escape')
    r.check('Esc gives the keyboard back to the text field, its text kept',
            wait(lambda: not r.s.run(OPEN) and r.s.run("return document.activeElement?.id === 'typing' && document.activeElement.value === 'My title'"), 3))
    r.s.run("document.getElementById('typing').remove()")

    # While an agent edits, an edit says so and does nothing.
    bridge = Bridge(r, r.s.run('return window.__nuzky.store.getState().snap.path'))
    try:
        run = bridge.call('begin_run', {'label': 'Trim pauses'})
        wait(lambda: r.s.run('return window.__nuzky.store.getState().lockShown'), 10)
        open_launcher(r)
        search(r, 'split')
        rows = r.s.run(ROWS)
        r.check('while the AI edits, Split says so and is off', rows and rows[0]['disabled'] and 'AI is editing. Stop it to edit yourself.' in rows[0]['text'], rows)
        time.sleep(0.3)
        r.shot('ai-lock')
        press('Return')
        time.sleep(0.5)
        r.check('Enter on it splits nothing', clips(r) == 1, r.track())
        press('Escape')
        wait(lambda: not r.s.run(OPEN), 3)
        bridge.call('end_run', {'run_id': run['run_id'], 'action': 'keep'})
        wait(lambda: not r.s.run('return window.__nuzky.store.getState().aiRun'), 10)
    finally:
        bridge.close()

    # Premiere Pro's keys: Ctrl+K splits there and Ctrl+O opens the launcher, whose rows name Premiere's keys.
    r.s.run("window.__nuzky.keys.setState({preset: 'premiere', changed: {}})")
    r.s.run('document.activeElement?.blur()')
    r.key('k', ctrlKey=True)
    r.check("with Premiere Pro's keys Ctrl+K still splits and opens no launcher", wait(lambda: clips(r) == 2, 3) and not r.s.run(OPEN), r.track())
    r.key('z', ctrlKey=True)
    wait(lambda: clips(r) == 1, 5)
    r.s.run('document.activeElement?.blur()')
    r.key('o', ctrlKey=True)
    r.check("and Ctrl+O, the launcher's key there, opens it", wait(lambda: r.s.run(OPEN), 5))
    search(r, 'split')
    rows = r.s.run(ROWS)
    r.check("Split's row names Premiere's key, Ctrl+K", rows and rows[0]['text'].endswith('CtrlK'), rows)
    time.sleep(0.3)
    r.shot('premiere')
    if resize(r, 1024, 640):
        time.sleep(0.5)
        r.shot('premiere-1024')
        resize(r, 1440, 900)
    press('Escape')
    r.s.run("window.__nuzky.keys.setState({preset: 'nuzky', changed: {}})")
    r.check('no error toast', not r.errors(), r.errors())

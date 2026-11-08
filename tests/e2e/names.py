"""Names the user teaches Nuzky: a word corrected in the transcript is saved as a name only when the user chooses Save as
a name, Settings shows, spells anew and deletes the names, they outlast a restart and reach an agent, and the next
transcript spells them the user's way while the transcript made before stays as it was."""
import json, time

from e2e.corrections import Hands, field, shown, tab
from e2e.harness import FIXTURES, MODELS, Bridge, Session, close_window, flow, leave_editor, link_models, press, wait
from e2e.reel import words as plain

TAKES = (FIXTURES / 'names-1.mp4', FIXTURES / 'names-2.mp4')
MODEL = 'large-v3-turbo-q5_0'
NAMES = "return window.__nuzky.names.getState().names"
ROWS = "return [...document.querySelectorAll('[data-setting=names] span.truncate')].map((s) => s.textContent)"
TOAST_BUTTON = """const b = [...document.querySelectorAll('[data-toast] button')].find((b) => b.textContent.trim() === arguments[0]);
if (!b) return false; b.click(); return true;"""
# Types into a field as pasting does, which also takes letters the test keyboard lacks, such as ů.
FILL = """const f = document.querySelector(arguments[0]); f.focus();
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(f, arguments[1]);
f.dispatchEvent(new Event('input', {bubbles: true})); return true;"""
CLICK = """const b = [...document.querySelectorAll(arguments[0])].find((b) => (b.getAttribute('aria-label') ?? b.textContent.trim()) === arguments[1]
  && b.getAttribute('aria-disabled') !== 'true'); if (!b) return false; b.click(); return true;"""


def setup(r):
    missing = [str(p) for p in (*TAKES, MODELS / f'ggml-{MODEL}.bin') if not p.exists()]
    if missing:
        raise RuntimeError(f'run scripts/fixtures.sh first, missing: {missing}')
    link_models(r, ('ggml-silero-v5.1.2.bin', f'ggml-{MODEL}.bin'))


def saved(r):
    """The names as Nuzky keeps them on disk, None while there is no file."""
    path = r.work / 'data/nuzky/glossary.json'
    return json.loads(path.read_text()) if path.exists() else None


def transcribe(r):
    jobs = lambda: [j for j in r.state()['jobs'] if j['kind'] == 'transcript']
    count = len(jobs())
    tab(r, 'transcript')
    r.s.run("[...document.querySelectorAll('button')].find((b) => /^Transcribe/.test(b.textContent.trim())).click()")
    return wait(lambda: next((j for j in jobs()[count:] if j['status'] != 'running'), None), 600, 1)


def toast_with(r, start):
    return wait(lambda: next((t['text'] for t in r.state()['toasts'] if t['text'].startswith(start)), None), 5)


def dismiss_toasts(r):
    r.s.run('const s = window.__nuzky.store.getState(); s.toasts.forEach((t) => s.dismissToast(t.id));')


def correct(r, hands, word, text):
    hands.double_click(f'[data-t="{word["t"]}"]')
    if not wait(lambda: (field(r) or {}).get('focused'), 5):
        return False
    hands.type(text + '\n')
    return wait(lambda: any(w['t'] == word['t'] and w['text'] == text for w in shown(r)), 5)


def settings_names(r):
    r.s.run("window.__nuzky.openSettings('models', 'names')")
    wait(lambda: r.s.run("return !!document.querySelector('[data-setting=names]')"), 5)
    time.sleep(0.5)
    return r.s.run(ROWS)


@flow('names', 'A corrected word is saved as a name only when the user chooses so; Settings shows, renames and deletes names, '
      'they outlast a restart and reach an agent, and the next transcript spells them that way', before=setup)
def names(r):
    r.import_media(TAKES[0])
    r.add_clip(TAKES[0].name)
    r.s.run("window.__nuzky.speech.setState({model: arguments[0], language: 'en'})", MODEL)
    job = transcribe(r)
    r.check('the first take is transcribed', job and job['status'] == 'done', job)
    words = wait(lambda: len(shown(r)) > 10 and shown(r), 30) or []
    texts = [w['text'] for w in words]
    code = next((i for i, w in enumerate(texts) if plain(w) == ['code'] and i > 0), None)
    if not r.check('without names, recognition misheard "Claude" before "Code"',
                   code is not None and plain(texts[code - 1]) != ['claude'], texts):
        return
    misheard, ordinary = words[code - 1], next(w for w in words if plain(w['text']) == ['helped'])
    hands = Hands(r)

    # An ordinary correction, its toast left alone: nothing is saved.
    r.check('correcting "helped" to "helps" changes the word', correct(r, hands, ordinary, 'helps'), shown(r))
    r.check('its toast offers Save as a name', toast_with(r, 'Corrected “helped” to “helps”')
            and r.s.run("return [...document.querySelectorAll('[data-toast] button')].some((b) => b.textContent.trim() === 'Save as a name')"))
    dismiss_toasts(r)
    r.s.run('document.activeElement?.blur()')
    r.key('z', ctrlKey=True)
    r.check('undo takes the ordinary correction back', wait(lambda: any(w['text'] == 'helped' for w in shown(r)), 10), shown(r))
    r.check('a correction the user did not save as a name saves nothing', saved(r) is None and r.s.run(NAMES) in (None, []),
            {'disk': saved(r), 'store': r.s.run(NAMES)})

    # The misheard name, saved from its toast.
    heard = misheard['text']
    r.check(f'correcting "{heard}" to "Claude" changes the word', correct(r, hands, misheard, 'Claude'), shown(r))
    r.check('the toast says what changed', toast_with(r, f'Corrected “{heard}” to “Claude”'), r.state()['toasts'])
    time.sleep(0.6)
    r.shot('save-as-name')
    r.check('Save as a name in the toast saves it', r.s.run(TOAST_BUTTON, 'Save as a name') and wait(lambda: saved(r) == ['Claude'], 5), saved(r))
    r.check('and says what that does', toast_with(r, 'Saved “Claude”. Transcripts made from now on'), r.state()['toasts'])
    r.check('Undo in that toast forgets the name again', r.s.run(TOAST_BUTTON, 'Undo') and wait(lambda: saved(r) == [], 5), saved(r))
    dismiss_toasts(r)
    before = [w['text'] for w in shown(r)]

    # Settings: add it there, spell it anew, add more, delete one.
    r.check('Settings › Models lists no names', settings_names(r) == [], r.s.run(ROWS))
    r.s.run(FILL, '[data-setting=names] input[aria-label="Name to add"]', 'Claude')
    press('Return')
    r.check('Enter in the field adds the name', wait(lambda: saved(r) == ['Claude'] and r.s.run(ROWS) == ['Claude'], 5),
            {'disk': saved(r), 'rows': r.s.run(ROWS)})
    r.s.run(CLICK, '[data-setting=names] button', 'Spell “Claude” differently')
    wait(lambda: r.s.run("return !!document.querySelector('[data-setting=names] input[aria-label^=Spell]')"), 5)
    r.s.run(FILL, '[data-setting=names] input[aria-label^=Spell]', 'Claude Code')
    press('Return')
    r.check('Enter keeps the new spelling', wait(lambda: saved(r) == ['Claude Code'] and r.s.run(ROWS) == ['Claude Code'], 5),
            {'disk': saved(r), 'rows': r.s.run(ROWS)})
    for name in ('Codex', 'Nůžky', 'Delete me'):
        r.s.run(FILL, '[data-setting=names] input[aria-label="Name to add"]', name)
        r.s.run(CLICK, '[data-setting=names] button', 'Add')
        wait(lambda: name in (saved(r) or []), 5)
    r.s.run(FILL, '[data-setting=names] input[aria-label="Name to add"]', 'codex')
    r.s.run(CLICK, '[data-setting=names] button', 'Add')
    r.check('adding a name that differs only in case keeps one, spelled as typed last',
            wait(lambda: saved(r) == ['Claude Code', 'Nůžky', 'Delete me', 'codex'], 5), saved(r))
    r.s.run(CLICK, '[data-setting=names] button', 'Spell “codex” differently')
    wait(lambda: r.s.run("return !!document.querySelector('[data-setting=names] input[aria-label^=Spell]')"), 5)
    r.s.run(FILL, '[data-setting=names] input[aria-label^=Spell]', 'Codex')
    press('Return')
    r.s.run(CLICK, '[data-setting=names] button', 'Delete “Delete me”')
    r.check('deleting a name removes only that one, accents kept',
            wait(lambda: saved(r) == ['Claude Code', 'Nůžky', 'Codex'] and r.s.run(ROWS) == saved(r), 5),
            {'disk': saved(r), 'rows': r.s.run(ROWS)})
    r.shot('names-in-settings')
    r.check('the transcript made before is unchanged by saving names', [w['text'] for w in shown(r)] == before, shown(r))

    # A restart keeps them, and an agent reads them.
    press('Escape')
    leave_editor(r)
    close_window()
    r.check('the window closes', wait(lambda: not alive(r), 15))
    r.s.close()
    r.s = Session()
    r.check('the app starts again with the project', wait(lambda: r.s.run('return !!window.__nuzky?.store.getState().snap', retries=3), 60))
    r.s.run("window.__nuzky.store.setState({view: 'editor'})")
    r.check('after a restart, Settings lists the same names', settings_names(r) == ['Claude Code', 'Nůžky', 'Codex'], r.s.run(ROWS))
    press('Escape')
    bridge = Bridge(r, r.saved_project())
    try:
        got = bridge.call('get_transcript', {})
        r.check("get_transcript gives an agent the user's names", got.get('names') == ['Claude Code', 'Nůžky', 'Codex'], got.get('names'))
    finally:
        bridge.close()

    # The next transcript reads them as a hint; the first take keeps the words it had.
    r.import_media(TAKES[1])
    r.add_clip(TAKES[1].name)
    job = transcribe(r)
    r.check('the second take is transcribed', job and job['status'] == 'done', job)
    after = wait(lambda: len(shown(r)) > len(before) and [w['text'] for w in shown(r)], 30) or []
    r.check('the first take reads as before, with its correction', after[:len(before)] == before, after)
    second = after[len(before):]
    pairs = [' '.join(plain(a) + plain(b)) for a, b in zip(second, second[1:])]
    r.check('the second take spells "Claude Code" and "Codex" as saved',
            'claude code' in pairs and any(w.strip('.,') == 'Codex' for w in second)
            and any(w.startswith('Claude') for w in second), second)
    tab(r, 'transcript')
    time.sleep(0.5)
    r.shot('next-transcript')
    r.check('no error toast', not r.errors(), r.errors())


def alive(r):
    try:
        r.s.run('return 1')
        return True
    except Exception:
        return False

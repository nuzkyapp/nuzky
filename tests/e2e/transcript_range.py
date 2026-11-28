"""Cutting several words at once from the transcript (#452): the user drags across them, Shift-clicks the last one or
triple-clicks a word for its whole sentence, and Delete cuts them all from the timeline as one undo step. On macOS the
pointer events the test bridge makes report no pressed button, so the drag shows only that the selection follows the
pointer while it is dragging, not which button the system reports. Shift is added to the click's pointerdown, since
the bridge's clicks carry no modifier keys."""
import re, time

from e2e.home import resize
from e2e.harness import FIXTURES, LINUX, MODELS, WINDOWS, actions, flow, link_models, pointer, press, wait

TAKE = FIXTURES / 'reel-2.mp4'
MODEL = 'large-v3-turbo-q5_0'
LIST = '[role=listbox][aria-label=Transcript]'
WORDS = f"""return [...document.querySelectorAll('{LIST} [role=option][data-t]')]
    .filter((e) => !e.hasAttribute('aria-label'))
    .map((e) => ({{t: Number(e.dataset.t), text: e.textContent, selected: e.getAttribute('aria-selected') === 'true'}}));"""
SELECTED = f"return [...document.querySelectorAll('{LIST} [role=option][aria-selected=true]')].map((e) => Number(e.dataset.t))"
BAR = f"return document.querySelector('{LIST}').nextElementSibling?.querySelector('span')?.textContent ?? null"
FITS = f"""const bar = document.querySelector('{LIST}').nextElementSibling, list = document.querySelector('{LIST}');
if (!bar) return null;
let panel = bar.parentElement; while (panel && getComputedStyle(panel).overflow === 'visible') panel = panel.parentElement;
return {{bar: Math.round(bar.getBoundingClientRect().bottom), panel: Math.round(panel.getBoundingClientRect().bottom), list: Math.round(list.getBoundingClientRect().height)}};"""
OPEN_CHAPTERS = """const b = document.querySelector('[data-chapters-toggle]');
if (b?.getAttribute('aria-expanded') === 'false') b.click();
return b?.getAttribute('aria-expanded') === 'true' && !!document.querySelector('[data-chapters]');"""
HINT = f"""const p = document.querySelector('{LIST}').nextElementSibling?.querySelector('p');
return p && p.offsetParent && {{text: p.textContent, whole: p.scrollWidth <= p.clientWidth}};"""
LENGTH = "return window.__nuzky.store.getState().snap.project.tracks[0].clips.reduce((s, c) => Math.max(s, c.startUs + c.durationUs), 0)"


def setup(r):
    missing = [str(p) for p in (TAKE, MODELS / f'ggml-{MODEL}.bin') if not p.exists()]
    if missing:
        raise RuntimeError(f'run scripts/fixtures.sh first, missing: {missing}')
    link_models(r, ('ggml-silero-v5.1.2.bin', f'ggml-{MODEL}.bin'))


def at(r, t, x=0.5):
    """A point of the word `t` on the page."""
    return r.s.run('const b = document.querySelector(`[data-t="${arguments[0]}"]`).getBoundingClientRect();'
                   'return [b.left + b.width * arguments[1], b.top + b.height / 2];', t, x)


class Mouse:
    """The real mouse: button down on one point, moved there with the button held, up there."""

    def __init__(self, r):
        self.r = r
        if LINUX:
            from Xlib import display
            self.d = display.Display()
            r.s.run("window.__pointerAt = null;"
                    "addEventListener('pointermove', (e) => { window.__pointerAt = [e.clientX, e.clientY]; }, true);")
            (x0, y0), (x1, y1) = (500, 300), (900, 600)
            (c0, d0), (c1, d1) = self.seen(x0, y0), self.seen(x1, y1)
            self.scale = (x1 - x0) / (c1 - c0)
            self.origin = (x0 - c0 * self.scale, y0 - d0 * self.scale)

    def fake(self, kind, detail=0, **where):
        from Xlib.ext import xtest
        xtest.fake_input(self.d, kind, detail, **where)
        self.d.sync()

    def seen(self, x, y):
        from Xlib import X
        self.fake(X.MotionNotify, x=x, y=y)
        time.sleep(0.3)
        return self.r.s.run('return window.__pointerAt')

    def screen(self, x, y):
        return (round(self.origin[0] + x * self.scale), round(self.origin[1] + y * self.scale)) if LINUX else (x, y)

    def drag(self, points):
        """Presses on the first point, moves through the others with the button held and lets go on the last."""
        if WINDOWS:
            steps = [{'type': 'pointerMove', 'duration': 0, 'origin': 'viewport', 'x': round(points[0][0]), 'y': round(points[0][1])},
                     {'type': 'pointerDown', 'button': 0}]
            steps += [{'type': 'pointerMove', 'duration': 60, 'origin': 'viewport', 'x': round(x), 'y': round(y)} for x, y in points[1:]]
            actions({'type': 'pointer', 'id': 'mouse', 'parameters': {'pointerType': 'mouse'}, 'actions': [*steps, {'type': 'pointerUp', 'button': 0}]})
            return
        if LINUX:
            from Xlib import X
            x, y = self.screen(*points[0])
            self.fake(X.MotionNotify, x=x, y=y)
            time.sleep(0.1)
            self.fake(X.ButtonPress, 1)
            for p in points[1:]:
                time.sleep(0.08)
                x, y = self.screen(*p)
                self.fake(X.MotionNotify, x=x, y=y)
            time.sleep(0.1)
            self.fake(X.ButtonRelease, 1)
            return
        pointer('down', *points[0])
        for p in points[1:]:
            time.sleep(0.08)
            pointer('drag', *p)
        time.sleep(0.1)
        pointer('up', *points[-1])

    def click(self, point, shift=False, times=1):
        """`times` clicks within the double-click time; Shift held through them when `shift`."""
        if times > 1:
            if WINDOWS:
                press_release = [{'type': 'pointerDown', 'button': 0}, {'type': 'pause', 'duration': 30}, {'type': 'pointerUp', 'button': 0}]
                actions({'type': 'pointer', 'id': 'mouse', 'parameters': {'pointerType': 'mouse'}, 'actions': [
                    {'type': 'pointerMove', 'duration': 0, 'origin': 'viewport', 'x': round(point[0]), 'y': round(point[1])}, *press_release * times]})
                return
            for n in range(1, times + 1):
                if LINUX:
                    from Xlib import X
                    x, y = self.screen(*point)
                    self.fake(X.MotionNotify, x=x, y=y)
                    self.fake(X.ButtonPress, 1)
                    time.sleep(0.03)
                    self.fake(X.ButtonRelease, 1)
                else:
                    pointer('down', *point, n)
                    time.sleep(0.03)
                    pointer('up', *point, n)
                time.sleep(0.06)
            return
        if shift:
            # Shift held through the click, as a person holds it.
            self.r.s.run("window.__shift = (e) => Object.defineProperty(e, 'shiftKey', {value: true});"
                         "addEventListener('pointerdown', window.__shift, true);")
        try:
            self.drag([point])
        finally:
            if shift:
                self.r.s.run("removeEventListener('pointerdown', window.__shift, true)")


def sentences(shown):
    """The transcript's words split into sentences after each word that ends one."""
    out, current = [], []
    for w in shown:
        current.append(w)
        if re.search(r'[.!?…]["”»)]*$', w['text']):
            out.append(current)
            current = []
    return out + ([current] if current else [])


@flow('transcript-range', 'Several words cut at once from the transcript: words show a text cursor and one selected word '
      'says how to select more; dragging across words, Shift-clicking or triple-clicking a word for its sentence selects '
      'them, and Delete cuts them all from the timeline as one undo step', before=setup)
def transcript_range(r):
    r.import_media(TAKE)
    r.add_clip('reel-2.mp4')
    r.s.run("window.__nuzky.speech.setState({model: arguments[0], language: 'cs'})", MODEL)
    r.s.run("document.querySelector('[data-tab=transcript]').click()")
    if not r.check('the Transcript tab offers to transcribe', wait(lambda: r.s.run("return !!document.querySelector('[data-transcribe]')"), 10)):
        return
    r.s.run("document.querySelector('[data-transcribe]').click()")
    words = lambda: r.s.run(WORDS, retries=3) or []
    if not r.check('the transcript shows the words', wait(lambda: len(words()) > 12, 600, 1), words()):
        return
    shown = words()
    sentence = next((s for s in sentences(shown)[1:] if len(s) >= 4), None)
    if not r.check('the take has a sentence of four words or more after the first', sentence, [w['text'] for w in shown]):
        return
    said = [w['t'] for w in sentence]
    text = ' '.join(w['text'] for w in sentence)
    length = r.s.run(LENGTH)
    mouse = Mouse(r)

    cursor = r.s.run('return getComputedStyle(document.querySelector(`[data-t="${arguments[0]}"]`)).cursor', said[1])
    r.check('a word shows the text cursor, as text that can be selected', cursor == 'text', cursor)
    mouse.click(at(r, said[1]))
    hint = lambda: r.s.run(HINT)
    r.check('with one word selected the bar says how to select more, the whole line in view',
            wait(lambda: hint() == {'text': 'Drag or Shift-click to select more words', 'whole': True}, 3), hint())
    r.shot('one-word')

    mouse.click(at(r, said[1]), times=3)
    selected = wait(lambda: r.s.run(SELECTED) if len(r.s.run(SELECTED)) > 1 else None, 3) or r.s.run(SELECTED)
    words_selected = [t for t in selected if t in {w['t'] for w in shown}]
    r.check(f'triple-clicking "{sentence[1]["text"]}" selects its sentence "{text}"', words_selected == said, selected)
    r.check('no word is left open for correcting', not r.s.run(f"return !!document.querySelector('{LIST} input')"))
    r.check('the bar counts the words of the sentence and no longer says how to select more',
            wait(lambda: (r.s.run(BAR) or '').startswith(f'{len(said)} words') and hint() is None, 3), [r.s.run(BAR), hint()])
    r.shot('sentence')
    press('Delete')
    cut = wait(lambda: r.s.run(LENGTH) < length - 500_000, 5)
    after = ' '.join(w['text'] for w in words())
    r.check('Delete cuts the whole sentence from the timeline and the transcript', cut and text not in after,
            {'before': length, 'after': r.s.run(LENGTH), 'transcript': after})
    r.shot('sentence-cut')
    r.s.run("window.__nuzky.store.getState().undo()")
    r.check('one undo brings the whole sentence back', wait(lambda: r.s.run(LENGTH) == length and text in ' '.join(w['text'] for w in words()), 5),
            r.s.run(LENGTH))

    # The field a double-click opens on a short word at the end of a line can wrap to the next line, away from the
    # pointer: the third click then presses on the gap, which closes the field, and lands there. The page plays that
    # third click itself, as no layout here wraps the field on cue.
    mouse.click(at(r, said[2]), times=2)
    field = f"return document.querySelector('{LIST} input')?.value ?? null"
    if r.check('double-clicking a word opens it for correcting', wait(lambda: r.s.run(field) == sentence[2]['text'], 3), r.s.run(field)):
        r.s.run(f"document.querySelector('{LIST}').focus()")
        wait(lambda: r.s.run(field) is None, 3)
        r.s.run(f"document.querySelector('{LIST}').dispatchEvent(new MouseEvent('click', {{bubbles: true, detail: 3}}))")
        selected = wait(lambda: r.s.run(SELECTED) if len(r.s.run(SELECTED)) > 1 else None, 3) or r.s.run(SELECTED)
        r.check('a third click that lands beside the word after its field closed still selects the sentence',
                [t for t in selected if t in said] == said and len(selected) >= len(said), selected)

    # A triple-click inside a word opened for correcting selects its text there, as in any text field.
    mouse.click(at(r, said[2]), times=2)
    if r.check('double-clicking the word again opens it for correcting', wait(lambda: r.s.run(field) == sentence[2]['text'], 3), r.s.run(field)):
        time.sleep(1)
        box = r.s.run(f"const b = document.querySelector('{LIST} input').getBoundingClientRect(); return [b.left + b.width / 2, b.top + b.height / 2]")
        mouse.click(box, times=3)
        time.sleep(0.5)
        r.check('a later triple-click inside the field keeps it open on the word', r.s.run(field) == sentence[2]['text'], r.s.run(SELECTED))
        press('Escape')
        wait(lambda: r.s.run(field) is None, 3)

    first, last = shown[1], shown[5]
    picked = [w['t'] for w in shown if first['t'] <= w['t'] <= last['t']]
    mouse.drag([at(r, first['t'], 0.3), *(at(r, w['t']) for w in shown[2:5]), at(r, last['t'], 0.7)])
    selected = wait(lambda: r.s.run(SELECTED) if len(r.s.run(SELECTED)) > 1 else None, 3) or r.s.run(SELECTED)
    r.check(f'dragging from "{first["text"]}" to "{last["text"]}" selects every word between', set(picked) <= set(selected) and min(selected) == first['t']
            and max(selected) == last['t'], selected)
    r.shot('dragged')

    mouse.click(at(r, first['t']))
    mouse.click(at(r, last['t']), shift=True)
    selected = r.s.run(SELECTED)
    r.check(f'clicking "{first["text"]}" and Shift-clicking "{last["text"]}" selects every word between', set(picked) <= set(selected)
            and min(selected) == first['t'] and max(selected) == last['t'], selected)

    mouse.click(at(r, first['t']))
    if r.check('the window shrinks to its smallest size, 1024 × 640', resize(r, 1024, 640)):
        fits = wait(lambda: (lambda f: f if f['bar'] <= f['panel'] else None)(r.s.run(FITS)), 3) or r.s.run(FITS)
        r.check('there the line on how to select more gives way, so the bar with Delete stays inside the panel',
                fits['bar'] <= fits['panel'] and not hint(), {**fits, 'hint': hint()})
        r.shot('one-word-1024')

    # Open chapters take the tab's height too: in a 1024 × 780 window with four of them the text is short again.
    asset = r.state()['assets'][0]['id']
    chapters = [{'title': t, 'assetId': asset, 'sourceUs': us} for t, us in (('Úvod', 0), ('Kamera', 4_000_000), ('Start', 8_000_000), ('Stativ', 11_000_000))]
    r.s.run('return window.__nuzky.store.getState().edit(arguments[0]).then((s) => !!s)', {'type': 'setChapters', 'chapters': chapters})
    if r.check('the window is 1024 × 780', resize(r, 1024, 780)) and r.check(
            'Chapters opens above the text', wait(lambda: r.s.run(OPEN_CHAPTERS), 5, 0.5)):
        time.sleep(0.5)
        mouse.click(at(r, first['t']))
        fits = wait(lambda: (lambda f: f if f and f['bar'] <= f['panel'] else None)(r.s.run(FITS)), 3) or r.s.run(FITS)
        r.check('with chapters open a word is still there to select, and the bar with Delete stays inside the panel',
                fits and fits['bar'] <= fits['panel'], {'fits': fits, 'hint': hint(), 'selected': r.s.run(SELECTED)})
        r.shot('one-word-chapters')
    resize(r, 1440, 900)

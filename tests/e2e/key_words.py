"""Key words marked by hand. With key words off, clicking a word of a caption in its inspector, or selecting one in the
transcript and pressing K, colours it in the key word yellow, in the preview, the engine's frame and the export alike, as
one undo step. The mark stays through a new style and a split, and generating the captions again with more words per
caption keeps it on the same words."""
import json, time

from e2e.harness import FIXTURES, changed_share, flow, press, preview_crop, preview_rect, wait
from e2e.karaoke import FPS, MODEL, engine_frame, export_frame, frame_at, karaoke_setup, lit, same_place, tile_js

WORDS = """const p = window.__nuzky.store.getState().snap.project;
return p.tracks.filter((t) => t.kind === 'text').flatMap((t) => t.clips).map((c) => ({id: c.id, startUs: c.startUs,
  durationUs: c.durationUs, text: c.content.text, keywords: c.content.style.keywords ?? null, words: c.content.words ?? []}));"""
JOBS = "return Object.entries(window.__nuzky.store.getState().jobs).map(([id, j]) => ({id, kind: j.kind, status: j.status}))"


def said(captions):
    """Every caption word with the timeline moment its middle is said and its mark: {(moment, text): marked}. Only the
    caption a word is heard in counts: a split leaves every word in both halves."""
    return {(c['startUs'] + (w['startUs'] + w['endUs']) // 2, w['text']): w.get('marked') for c in captions for w in c['words']
            if 0 <= (w['startUs'] + w['endUs']) // 2 < c['durationUs']}


def mark_at(captions, moment, text, tolerance=40_000):
    """The mark of the caption word saying `text` (ignoring case and punctuation) at `moment`."""
    plain = lambda s: ''.join(ch for ch in s.lower() if ch.isalnum())
    return next((m for (at, t), m in said(captions).items() if abs(at - moment) <= tolerance and plain(t) == plain(text)), 'missing')


def generate(r, label):
    before = {j['id'] for j in r.s.run(JOBS)}
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === 'Captions')?.click()")
    button = f"return [...document.querySelectorAll('button')].find((b) => b.textContent.includes({label!r}))"
    if not wait(lambda: r.s.run(button + ' ? true : false'), 10, 0.5):
        return None
    r.s.run(button + '.click()')
    return wait(lambda: next((j for j in r.s.run(JOBS) if j['kind'] == 'captions' and j['id'] not in before and j['status'] != 'running'), None), 900, 1)


@flow('key-words', 'A caption word marked by hand, in the inspector or the transcript, shows in the key word colour with key '
      'words off, in the preview and the export, undoes in one step and stays through a style, a split and regenerating',
      before=karaoke_setup, reuse_transcripts=True)
def key_words(r):
    r.import_media(FIXTURES / 'reel-2.mp4')
    r.add_clip('reel-2.mp4')
    r.s.run(f"window.__nuzky.speech.setState({{model: '{MODEL}', language: 'cs', captionWords: 2}})")
    job = generate(r, 'Generate captions')
    if not r.check('Czech captions are generated in the Reel style', job and job['status'] == 'done', job):
        return
    captions = r.s.run(WORDS)
    r.check('the captions have key words off', captions and all(c['keywords'] is None for c in captions), [c['keywords'] for c in captions[:3]])
    caption = next((c for c in captions[1:] if len(c['words']) == 2 and c['durationUs'] >= 500_000
                    and all(sum(ch.isalnum() for ch in w['text']) >= 3 for w in c['words'])), None)
    if not r.check('a caption of two words is there', caption, [c['text'] for c in captions]):
        return
    word = caption['words'][1]
    moment = caption['startUs'] + (word['startUs'] + word['endUs']) // 2
    # The frame where the word is said: whatever captions are made again, the one showing it is on screen then.
    n, at = frame_at(moment)
    print(f"  caption \"{caption['text']}\" at {caption['startUs'] / 1e6:.3f} s, marking \"{word['text']}\"", flush=True)
    time.sleep(1.5)  # `nuzky frame` reads the saved file, saved about a second after an edit.
    r.check('before marking, the caption has no key word colour', lit(engine_frame(r, at, 'engine-before'))[1] == 0)

    # The inspector lists the caption's words; clicking the second makes it key.
    r.seek(at)
    r.focus_clip(caption['id'])
    group = "document.querySelector('[role=group][aria-label=\"Key words of this caption\"]')"
    if not r.check('the caption inspector lists its words under Key words', wait(lambda: r.s.run(f'return !!{group}'), 5)):
        r.shot('inspector-missing')
        return
    r.s.run(f"{group}.scrollIntoView({{block: 'center'}})")
    r.shot('inspector-before')
    buttons = r.s.run(f"return [...{group}.querySelectorAll('button')].map((b) => [b.textContent, b.getAttribute('aria-pressed')])")
    r.check('the words show as written, none pressed', buttons == [[w['text'], 'false'] for w in caption['words']], buttons)
    r.s.run(f"{group}.querySelectorAll('button')[1].click()")
    marked = wait(lambda: mark_at(r.s.run(WORDS), moment, word['text']) is True, 5)
    r.check(f'clicking "{word["text"]}" marks it key in the project', marked)
    r.check('its button shows it pressed', wait(lambda: r.s.run(f"return {group}.querySelectorAll('button')[1].getAttribute('aria-pressed')") == 'true', 5))
    rect = preview_rect(r)

    def preview_lit(name):
        r.shot(name)
        found = lit(preview_crop(r.work / f'{name}.png', rect))
        return found if found[1] > 0.0003 else None
    preview = wait(lambda: preview_lit('preview-marked'), 8, 1) or lit(preview_crop(r.work / 'preview-marked.png', rect))
    time.sleep(1.5)
    engine = lit(engine_frame(r, at, 'engine-marked'))
    r.check('the preview colours the word with key words off', preview[1] > 0.0003, preview)
    r.check('the engine colours it too, where the preview does', engine[1] > 0.0003 and same_place(preview[0], engine[0]),
            {'preview': preview, 'engine': engine})
    r.shot('inspector-marked')

    r.s.run('window.__nuzky.store.getState().undo()')
    r.check('one undo takes the mark back', wait(lambda: mark_at(r.s.run(WORDS), moment, word['text']) is None, 5))
    r.s.run('window.__nuzky.store.getState().redo()')
    r.check('redo marks it again', wait(lambda: mark_at(r.s.run(WORDS), moment, word['text']) is True, 5))

    # A new style keeps the mark, so does a split through the caption.
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === 'Captions')?.click()")
    wait(lambda: r.s.run(tile_js('Outline') + ' ? true : false'), 10, 0.5)
    r.s.run(tile_js('Outline') + '.click()')
    r.check('the Outline style keeps the mark', wait(lambda: r.s.run(tile_js('Outline') + ".getAttribute('aria-pressed')") == 'true', 5)
            and mark_at(r.s.run(WORDS), moment, word['text']) is True)
    split = caption['startUs'] + (caption['words'][0]['endUs'] + word['startUs']) // 2
    r.s.call("window.__nuzky.store.getState().edit({type: 'splitClip', clipId: arguments[0], atUs: arguments[1]})", caption['id'], split)
    r.check('a split between the two words keeps the mark on its piece',
            wait(lambda: len(r.s.run(WORDS)) == len(captions) + 1, 5) and mark_at(r.s.run(WORDS), moment, word['text']) is True)

    # The transcript. By its time: a take may say the same word twice. Words are the options without a label of their own.
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === 'Transcript')?.click()")
    view = json.loads(r.s.call('window.__nuzky.api.transcriptView(500000).then(JSON.stringify)')['value'])

    def transcript_word(start_us):
        k = min(range(len(view['words'])), key=lambda i: abs(view['words'][i]['startUs'] - start_us))
        span = f"document.querySelectorAll('[role=listbox][aria-label=Transcript] [role=option]:not([aria-label])')[{k}]"
        select = (f"const s = {span}; s.scrollIntoView({{block: 'center'}});"
                  "s.dispatchEvent(new PointerEvent('pointerdown', {bubbles: true, button: 0, buttons: 1}));"
                  "s.dispatchEvent(new PointerEvent('pointerup', {bubbles: true, button: 0}));")
        return view['words'][k]['text'], span, select
    button = "[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Key word')"

    # The split caption: K acts on the half the word is heard in, so the first K takes the mark back.
    _, _, select_split = transcript_word(caption['startUs'] + word['startUs'])
    r.s.run(select_split)
    r.check(f'after the split, Key word shows "{word["text"]}" as key', wait(lambda: r.s.run(f"return {button}?.getAttribute('aria-pressed')") == 'true', 5))
    press('k')
    r.check('K on it takes the mark back in the half it is heard in', wait(lambda: mark_at(r.s.run(WORDS), moment, word['text']) is None, 5))
    press('k')
    r.check('and K again marks it there once more', wait(lambda: mark_at(r.s.run(WORDS), moment, word['text']) is True, 5))

    # A word of another caption.
    other = next((c for c in captions if c['startUs'] > caption['startUs'] + caption['durationUs'] and c['words']
                  and sum(ch.isalnum() for ch in c['words'][0]['text']) >= 3), None)
    if not r.check('another caption follows', other):
        return
    picked = other['words'][0]
    picked_at = other['startUs'] + (picked['startUs'] + picked['endUs']) // 2
    shown, span, select_word = transcript_word(other['startUs'] + picked['startUs'])
    if not r.check(f'the transcript shows "{picked["text"]}"', wait(lambda: r.s.run(f'return ({span})?.textContent.trim()') == picked['text'].strip(), 10),
                   [shown, picked['text']]):
        r.shot('transcript-missing')
        return
    r.s.run(select_word)
    r.check('the bar under the selection offers Key word, not pressed', wait(lambda: r.s.run(f"return {button}?.getAttribute('aria-pressed')") == 'false', 5))
    press('k')
    r.check(f'K marks "{picked["text"]}" key in its caption', wait(lambda: mark_at(r.s.run(WORDS), picked_at, picked['text']) is True, 5))
    r.check('Key word shows pressed', wait(lambda: r.s.run(f"return {button}.getAttribute('aria-pressed')") == 'true', 5))
    shade = f"const s = getComputedStyle({span}); return [s.backgroundColor, s.color]"
    r.check('at once the selected word shows black on the key word colour',
            wait(lambda: r.s.run(shade) == ['rgb(255, 212, 0)', 'rgb(0, 0, 0)'], 5), r.s.run(shade))
    r.shot('transcript-pressed')
    # The word under the playhead is drawn in the accent; one word on, it shows in the key word colour.
    press('Right')
    colour = f"return getComputedStyle({span}).color"
    r.check('the transcript shows the word in the key word colour', wait(lambda: r.s.run(colour) == 'rgb(255, 212, 0)', 5), r.s.run(colour))
    wait(lambda: r.s.run(f"return {button}.getAttribute('aria-pressed')") == 'false', 5)
    time.sleep(0.3)  # The button's colour fades over 120 ms.
    r.shot('transcript-key')
    r.s.run(select_word)
    press('k')
    # Manual alone leaves the word plain, so taking the mark back leaves nothing behind.
    r.check('K again makes it plain and follows the pick again', wait(lambda: mark_at(r.s.run(WORDS), picked_at, picked['text']) is None, 5))
    press('k')
    r.check('and K once more key again', wait(lambda: mark_at(r.s.run(WORDS), picked_at, picked['text']) is True, 5))

    # Generating again with three words per caption keeps both marks on their words.
    r.s.run("window.__nuzky.speech.setState({captionWords: 3})")
    job = generate(r, 'Regenerate captions')
    if not r.check('captions are generated again, three words each', job and job['status'] == 'done', job):
        return
    again = r.s.run(WORDS)
    r.check('regrouped captions are there', any(len(c['words']) == 3 for c in again), [c['text'] for c in again[:4]])
    r.check(f'"{word["text"]}" stays marked after regenerating', mark_at(again, moment, word['text']) is True)
    r.check(f'"{picked["text"]}" stays marked after regenerating', mark_at(again, picked_at, picked['text']) is True)
    r.check('no other word got a mark', sum(1 for m in said(again).values() if m) == 2, [k for k, m in said(again).items() if m])

    # The export colours the word where the engine does.
    time.sleep(1.5)
    engine = lit(engine_frame(r, at, 'engine-regenerated'))
    r.check('the engine colours the word after regenerating', engine[1] > 0.0003, engine)
    target = r.work / 'key-words.mp4'
    started = r.s.call('window.__nuzky.api.startExport(arguments[0], arguments[1], arguments[2], arguments[3])', str(target),
                       {'resolution': 720, 'fps': FPS, 'quality': 'recommended'}, r.state()['epoch'], False)
    r.check('the export starts', started['ok'], started)
    done = wait(lambda: next((j for j in r.state()['jobs'] if j['kind'] == 'export' and j['status'] != 'running'), None), 300, 1)
    if r.check('the export finishes', done and done['status'] == 'done', done):
        exported = export_frame(r, target, n, 'export-marked')
        found = lit(exported)
        r.check(f'export frame {n} colours the word where the engine does', found[1] > 0.0003 and same_place(found[0], engine[0]),
                {'export': found, 'engine': engine})
        changed = changed_share(r.work / 'engine-regenerated.png', exported)
        r.check(f'export frame {n} matches the rendered one', changed < 0.002, f'{changed:.3%} of pixels differ')
    r.check('no error toast', not r.errors(), r.errors())

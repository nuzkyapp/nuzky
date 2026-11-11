import json, subprocess, time

from e2e.ai_panel import FAKE
from e2e.caption_text import SECTION, reveal, views
from e2e.corrections import generate_captions, tab
from e2e.harness import CLI, FIXTURES, changed_share, export, flow, link_models, press, preview_crop, preview_rect, preview_redraw, wait, webdriver

TRACKS = 'return window.__nuzky.store.getState().snap.project.tracks'
LANGUAGES = "return [...document.querySelectorAll('[role=radiogroup][aria-label=\"Captions to style\"] [role=radio]')]" \
            ".map((r) => ({name: r.getAttribute('aria-label'), on: r.getAttribute('aria-checked') === 'true', text: r.innerText}))"
ADD = 'document.querySelector("[data-translate-captions]")'
SAID = ["Today I'll show you how to edit a video.", 'First we import the clips.', 'Then we add the captions.']


def setup(r):
    if not (FIXTURES / 'czech-talk.mp4').exists():
        raise RuntimeError('tmp-test/czech-talk.mp4 is missing: run scripts/fixtures.sh')
    link_models(r, ('ggml-large-v3-turbo-q5_0.bin', 'ggml-silero-v5.1.2.bin'))
    project = r.work / 'data/nuzky/projects/bilingual.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(FIXTURES / 'czech-talk.mp4')], env=r.env, check=True, capture_output=True)
    # The stand-in answers for Claude Code, so no flow spends anyone's subscription; Codex is missing on purpose.
    r.env.update(NUZKY_AGENT_CLAUDE=str(FAKE), NUZKY_AGENT_CODEX=str(r.work / 'no-codex'), NUZKY_FAKE_AGENT_LOG=str(r.work / 'fake'))


def tracks(r):
    return r.s.run(TRACKS, retries=3) or []


def speech(r):
    return next((t for t in tracks(r) if t['name'] == 'Captions' and not t.get('translation')), None)


def english(r):
    return next((t for t in tracks(r) if t['name'] == 'Captions' and t.get('translation') == 'en'), None)


def texts(track):
    return [c['content']['text'] for c in track['clips']] if track else []


def frame(path, at_s, out):
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-ss', f'{at_s:.3f}', '-i', str(path), '-frames:v', '1', str(out)], check=True)
    return out


@flow('bilingual', 'Czech captions made from the speech get English captions under them, translated by the AI the user '
      'connected, as a track of their own with its own look that shows, hides, edits and exports apart from the Czech',
      before=setup)
def bilingual(r):
    r.s.run("window.__nuzky.speech.setState({model: 'large-v3-turbo-q5_0', language: 'cs'})")
    job = generate_captions(r)
    r.check('Whisper makes the Czech captions', job and job['status'] == 'done', job)
    czech = texts(speech(r))
    heard = ' '.join(czech).lower()
    found = [w for w in ('dneska', 'ukážu', 'video', 'klipy', 'titulky') if w in heard]
    r.check('the captions are the Czech that was said', len(found) >= 3, {'found': found, 'captions': czech})
    on_disk = r.saved_project().read_text()
    r.check('a project with one captions track saves without a translation', '"translation"' not in on_disk)

    rows = wait(lambda: (rows := r.s.run(LANGUAGES)) and rows[0]['text'].startswith('Czech') and rows, 10)
    r.check('Languages names the speech captions by the language heard', rows and rows[0]['name'] == 'Czech captions', rows)
    ready = wait(lambda: r.s.run(f'return {ADD}?.getAttribute("aria-disabled") !== "true" && {ADD}?.textContent'), 15)
    r.check('Add English captions is ready', ready and 'Add English captions' in ready,
            r.s.run(f'return {ADD} && [{ADD}.textContent, {ADD}.title]'))
    r.shot('languages-before')

    r.s.run(f'{ADD}.click()')
    working = wait(lambda: r.s.run("return document.querySelector('[data-translating]')?.innerText ?? ''").startswith('Translating to English'), 10)
    r.check('the row says the AI is translating, with Stop', working and r.s.run(
        "return [...document.querySelectorAll('[data-translating] button')].some((b) => b.textContent.trim() === 'Stop')"),
        r.s.run('return window.__nuzky.agent.getState().translating'))
    r.shot('translating')
    en = wait(lambda: r.s.run('return window.__nuzky.agent.getState().status') == 'idle' and english(r), 60)
    r.check('the AI answers with an English captions track', en, r.s.run('return window.__nuzky.agent.getState().items.map((i) => i.kind)'))
    sent = [json.loads(line) for line in (r.work / 'fake/runs.jsonl').read_text().splitlines()]
    r.check('the panel asked the AI to translate into English', any('Translate the captions into English.' in json.dumps(s) for s in sent))
    # The stand-in translates sentence by sentence from SAID; Whisper decides how many sentences it heard.
    shown = ' '.join(texts(en))
    r.check('the English captions say the translated sentences', any(shown == ' '.join(SAID[:n]) for n in (2, 3)), texts(en))
    r.check('the Czech captions stay as they were', texts(speech(r)) == czech, texts(speech(r)))
    cs = speech(r)
    r.check('English starts with the first Czech caption and keeps no spoken words',
            en['clips'][0]['startUs'] == cs['clips'][0]['startUs'] and all(not c['content'].get('words') for c in en['clips']),
            [(c['startUs'], c['content']['text']) for c in en['clips']])
    cs_style, en_style = cs['clips'][0]['content']['style'], en['clips'][0]['content']['style']
    r.check('English is 70 % of the Czech size and sits under it',
            abs(en_style['fontSize'] - cs_style['fontSize'] * 0.7) < 0.01 and en['clips'][0]['content']['transform']['y'] > cs['clips'][0]['content']['transform']['y'],
            {'cs': cs_style['fontSize'], 'en': en_style['fontSize']})

    # One Undo takes the AI's whole run back, and Redo brings it.
    r.s.call('window.__nuzky.store.getState().undo()')
    r.check('Undo takes the English captions back', wait(lambda: english(r) is None, 5))
    r.s.call('window.__nuzky.store.getState().redo()')
    r.check('Redo brings them back', wait(lambda: english(r) is not None, 5))
    en = english(r)

    # Picking English in Languages makes the tab about English; Edit style shows an English caption in the inspector,
    # whose place the AI panel took to translate.
    r.s.run("document.querySelector('[role=radio][aria-label=\"English captions\"]').click()")
    picked = wait(lambda: r.s.run("return [...document.querySelectorAll('h3')].some((h) => h.textContent === 'Style · English')"), 5)
    r.check('picking English makes the tab about the English captions', picked and r.s.run(
        "return !!document.querySelector('[data-translate-again]') && !document.querySelector('[data-generate-captions]')"))
    inspector = f"return [!!document.querySelector('aside[aria-label=AI]'), !!document.querySelector('{SECTION}')]"
    r.check("the AI panel that translated sits in the inspector's place", r.s.run(inspector) == [True, False], r.s.run(inspector))
    r.seek(en['clips'][0]['startUs'])
    r.s.run("document.querySelector('[data-edit-caption-style]').click()")
    selection = lambda: r.s.run('return window.__nuzky.store.getState().selection')
    r.check('Edit style puts the AI panel away and shows the English caption at the playhead in the inspector',
            wait(lambda: r.s.run(inspector) == [False, True] and selection() == [en['clips'][0]['id']], 5), [r.s.run(inspector), selection()])
    r.s.run(f"document.querySelector('{SECTION} button[title=\"Apply Yellow to all captions\"]').click()")
    yellow = wait(lambda: (t := english(r)) and t['clips'][0]['content']['style']['color'] != en_style['color'] and t, 5)
    r.check('a style applies to the English captions', yellow, english(r)['clips'][0]['content']['style'])
    r.check('and leaves the Czech look alone', speech(r)['clips'][0]['content']['style'] == cs_style)

    # A caption's inspector styles either track: Language goes from a Czech caption to the English one on screen with it,
    # whose inspector has no karaoke or key words and puts a style on at 70 % of its size.
    k = yellow['clips'][0]['content']['style']['fontSize'] / 74  # Yellow is 74 px at 1080p; here at 70 % on this canvas.
    first = speech(r)['clips'][0]
    r.focus_clip(first['id'])
    picker = f"[...document.querySelectorAll('{SECTION} [role=group][aria-label=Language] button')]"
    segments = lambda: r.s.run(f"return {picker}.map((b) => [b.textContent.trim(), b.getAttribute('aria-pressed')])")
    r.check("a Czech caption's inspector offers Language with Czech and English, Czech picked",
            wait(lambda: segments() == [['Czech', 'true'], ['English', 'false']], 10), segments())
    r.s.run(f"{picker}.find((b) => b.textContent.trim() === 'English')?.focus()")
    press('Return')
    under = next(c for c in english(r)['clips'] if c['startUs'] <= first['startUs'] < c['startUs'] + c['durationUs'])
    r.check('Enter on English selects the English caption on screen when the Czech one starts', wait(lambda: selection() == [under['id']], 5), selection())
    focused = lambda: r.s.run("return [document.activeElement?.textContent.trim(), document.activeElement?.getAttribute('aria-pressed')]")
    r.check('and the keyboard stays on English, now picked', wait(lambda: focused() == ['English', 'true'], 5), focused())
    sections = r.s.run(f"return [...document.querySelectorAll('{SECTION} section h3')].map((h) => h.textContent)")
    karaoke = r.s.run(f"return !!document.querySelector('{SECTION} button[title=\"Apply Karaoke to all captions\"]')")
    r.check("the English caption's inspector has no Karaoke or Key words and no style that lights words",
            'Style' in sections and 'Karaoke' not in sections and 'Key words' not in sections and not karaoke, {'sections': sections, 'karaoke tile': karaoke})
    views(r, 'inspector-english', reveal(f"document.querySelector('{SECTION} [role=group][aria-label=Language]').parentElement.parentElement.parentElement"))
    r.s.run(f"document.querySelector('{SECTION} button[title=\"Apply Box to all captions\"]').click()")
    boxed = wait(lambda: (t := english(r)) and all(c['content']['style']['background'] == '#000000b3' for c in t['clips']) and t, 5)
    sizes = [c['content']['style']['fontSize'] for c in (boxed or english(r))['clips']]
    r.check('Box from the inspector goes on every English caption at 70 % of its 62 px, without a highlight',
            boxed and all(abs(s - 62 * k) < 0.01 for s in sizes) and not any(c['content']['style'].get('highlight') for c in boxed['clips']),
            {'sizes': sizes, 'expected': 62 * k})
    r.check('and leaves the Czech look alone too', speech(r)['clips'][0]['content']['style'] == cs_style)
    r.s.run('window.__nuzky.store.getState().select([])')

    # Both show in the preview; hiding English takes only it away.
    middle = en['clips'][0]['startUs'] + en['clips'][0]['durationUs'] // 2
    r.seek(middle)
    time.sleep(1.5)
    r.shot('both')
    both = preview_crop(r.work / 'both.png', preview_rect(r))
    r.s.run("document.querySelector('[role=radio][aria-label=\"English captions\"] button[aria-label^=Hide]').click()")
    r.check('the eye hides the English track', wait(lambda: english(r)['hidden'], 5))
    hidden, redrawn = preview_redraw(r, 'english-hidden', both)
    r.check('the preview drops the English caption', redrawn and changed_share(both, hidden) > 0.002, f'{changed_share(both, hidden):.2%}')
    r.s.run("document.querySelector('[role=radio][aria-label=\"English captions\"] button[aria-label^=Show]').click()")
    wait(lambda: not english(r)['hidden'], 5)

    # Editing either track leaves the other.
    edit = "window.__nuzky.store.getState().edit({type: 'updateClip', clipId: arguments[0], text: arguments[1]})"
    first_en = english(r)['clips'][0]
    r.s.call(edit, first_en['id'], 'Hi, today I will show you')
    wait(lambda: english(r)['clips'][0]['content']['text'] == 'Hi, today I will show you', 5)
    r.check('an English edit leaves the Czech', texts(speech(r)) == czech)
    english_now = texts(english(r))
    r.s.call(edit, speech(r)['clips'][0]['id'], 'Dnes')
    wait(lambda: texts(speech(r))[0] == 'Dnes', 5)
    r.check('a Czech edit leaves the English', texts(english(r)) == english_now, texts(english(r)))

    # Making the Czech captions again works beside English and leaves it as the user edited it.
    r.s.run("document.querySelector('[role=radio][aria-label=\"Czech captions\"]').click()")
    job = generate_captions(r)
    r.check('Regenerate works beside the English track', job and job['status'] == 'done', job)
    r.check('and leaves the English as it was', texts(english(r)) == english_now, texts(english(r)))
    r.check('Regenerate makes the Czech again', texts(speech(r)) == czech, texts(speech(r)))

    # The export has the English burned in exactly while the track shows.
    at = middle / 1e6
    with_en = frame(export(r, 'bilingual.mp4'), at, r.work / 'export-both.png')
    r.s.call("window.__nuzky.store.getState().edit({type: 'updateTrack', trackId: arguments[0], hidden: true})", english(r)['id'])
    wait(lambda: english(r)['hidden'], 5)
    without = frame(export(r, 'czech-only.mp4'), at, r.work / 'export-czech.png')
    r.check('the export draws the English captions and leaves them out when hidden', changed_share(with_en, without) > 0.002,
            f'{changed_share(with_en, without):.2%}')
    r.s.call("window.__nuzky.store.getState().edit({type: 'updateTrack', trackId: arguments[0], hidden: false})", english(r)['id'])
    wait(lambda: not english(r)['hidden'], 5)

    saved = json.loads(r.saved_project().read_text())
    stored = [t for t in saved['tracks'] if t['name'] == 'Captions']
    r.check('the project keeps both tracks, the English marked as a translation',
            [t.get('translation') for t in stored] == [None, 'en'], [t.get('translation') for t in stored])
    tab(r, 'captions')
    r.s.run("document.querySelector('[role=radio][aria-label=\"English captions\"]').click()")
    r.seek(middle)
    time.sleep(1.5)
    r.shot('bilingual')
    height = "return document.querySelector('[data-translate-again]').getBoundingClientRect().height"
    r.check('Translate again is a full 32 px button', r.s.run(height) >= 32, r.s.run(height))
    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    wait(lambda: r.s.run('return [innerWidth, innerHeight]') == [1024, 640], 5)
    time.sleep(1)
    r.shot('bilingual-small')
    rows = r.s.run("return [...document.querySelectorAll('[role=radio]')].map((e) => e.getBoundingClientRect())"
                   ".every((b) => b.width >= 200 && b.bottom <= innerHeight)")
    r.check('at the smallest window both language rows show whole', rows)
    r.check('no error toast', not r.errors(), r.errors())

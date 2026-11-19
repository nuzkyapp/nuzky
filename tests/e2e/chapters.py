"""YouTube chapters in the app. A Czech talk of about a minute and a half is cut and its second half sped up; the
Transcript tab's Chapters asks the AI panel's agent (the fake one, which picks three sentences over MCP) and shows each
chapter at the time Nuzky worked out on the cut video, with its title to edit. Copy puts the list YouTube takes on the
clipboard, a cut that brings two chapters within 10 s leaves one out and says why, and Export saves the list beside the
video with the captions of both languages as .srt, unchanged."""
import json, subprocess, time
from pathlib import Path

from e2e.export_srt import matches, open_dialog, close_dialog, save, DIALOG
from e2e.harness import CLI, FIXTURES, MACOS, Bridge, flow, link_models, wait, webdriver
from e2e.reel import MODEL, poll

FAKE = Path(__file__).resolve().parent / 'fake_agent'
REGION = "document.querySelector('[data-chapters]')"
ROWS = f"""return [...({REGION}?.querySelectorAll('[data-chapter]') ?? [])].map((li) => ({{
  time: li.querySelector('[data-chapter-time]').textContent, title: li.querySelector('input').value,
  leftOut: li.querySelector('[data-left-out]')?.textContent ?? null}}))"""
SEGMENTS = """return [...document.querySelectorAll('[data-chapter-segment]')]
  .map((b) => [b.textContent, parseFloat(b.style.left), parseFloat(b.style.width)])"""
CLICK = """const b = [...(arguments[1] ? document.querySelector(arguments[1]) : document).querySelectorAll('button')]
  .find((b) => b.textContent.trim() === arguments[0] || b.getAttribute('aria-label') === arguments[0]);
if (!b) return null; b.click(); return b.getAttribute('aria-disabled') ?? 'false';"""
EDIT = 'return window.__nuzky.store.getState().edit(arguments[0]).then((s) => !!s)'
BOX = """return (([l]) => l && {checked: l.querySelector('input').checked, disabled: l.querySelector('input').disabled, title: l.title})(
  [...document.querySelectorAll('[role=dialog] label')].filter((l) => l.textContent.includes(arguments[0])))"""
TOGGLE = "[...document.querySelectorAll('[role=dialog] label')].find((l) => l.textContent.includes(arguments[0])).querySelector('input').click()"
TRANSLATE = 'document.querySelector("[data-translate-captions]")'


def setup(r):
    link_models(r, ('ggml-silero-v5.1.2.bin', f'ggml-{MODEL}.bin'))
    # The three Czech takes said twice over a test picture: about 95 s.
    takes = [FIXTURES / f'reel-{n}.mp4' for n in (1, 2, 3)] * 2
    video = r.work / 'talk.mp4'
    inputs = [arg for take in takes for arg in ('-i', str(take))]
    sound = ''.join(f'[{n}:a]' for n in range(len(takes))) + f'concat=n={len(takes)}:v=0:a=1[a]'
    # Its length given, so the transcript of an earlier run still belongs to it (reuse_transcripts).
    length = sum(float(subprocess.run(['ffprobe', '-v', 'error', '-select_streams', 'a:0', '-show_entries', 'stream=duration',
                                       '-of', 'csv=p=0', str(take)], capture_output=True, text=True, check=True).stdout) for take in takes)
    subprocess.run(['ffmpeg', '-v', 'error', '-y', *inputs, '-f', 'lavfi', '-i', 'testsrc2=size=1280x720:rate=30',
                    '-filter_complex', sound, '-map', f'{len(takes)}:v', '-map', '[a]', '-t', f'{length:.3f}',
                    '-c:v', 'libx264', '-preset', 'veryfast', '-pix_fmt', 'yuv420p', '-c:a', 'aac', str(video)], check=True)
    project = r.work / 'data/nuzky/projects/talk.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(video)], env=r.env, check=True, capture_output=True)
    (r.work / 'home/Movies').mkdir(parents=True, exist_ok=True)
    r.env.update(NUZKY_AGENT_CLAUDE=str(FAKE), NUZKY_AGENT_CODEX=str(r.work / 'no-codex'), NUZKY_FAKE_AGENT_LOG=str(r.work / 'fake'))


def project(r):
    return r.s.run('return window.__nuzky.store.getState().snap.project', retries=3)


def chapter_list(r):
    return json.loads(r.s.call('window.__nuzky.api.chapterList().then(JSON.stringify)')['value'])


def rows(r):
    return r.s.run(ROWS, retries=3) or []


def words(r):
    return json.loads(r.s.call('window.__nuzky.api.transcriptView(500000).then((v) => JSON.stringify(v.words))')['value'])


def small_window(r, name):
    for width, height, shot in ((1024, 640, name), (1440, 900, None)):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        if shot:
            time.sleep(0.5)
            r.shot(shot)
            apart = r.s.run(f"""const region = {REGION}.getBoundingClientRect();
                const text = document.querySelector('[role=listbox][aria-label=Transcript]').getBoundingClientRect();
                return region.bottom <= text.top + 1;""")
            r.check('in the smallest window the chapters scroll inside their region and never cover the text', apart)
            reach = r.s.run(f"""const body = {REGION}.querySelector('.overflow-y-auto'); body.scrollTop = body.scrollHeight;
                const button = {REGION}.querySelector('[data-suggest-chapters]').getBoundingClientRect();
                const region = {REGION}.getBoundingClientRect(), pane = {REGION}.parentElement.getBoundingClientRect();
                const shown = button.top >= region.top && button.bottom <= Math.min(region.bottom, pane.bottom) + 1;
                body.scrollTop = 0; return shown;""")
            r.check('and every chapter and Suggest again can be scrolled to inside the pane', reach)


@flow('chapters', 'Chapters in the Transcript tab: the AI names three chapters of a cut and sped-up talk, Nuzky times them '
      'on the video as it plays, titles are edited and copied for YouTube, a cut that brings two within 10 s leaves one '
      'out with the reason, and Export saves the list beside the video and its captions in two languages',
      before=setup, reuse_transcripts=True)
def chapters(r):
    r.s.call("window.__nuzky.store.getState().edit({type: 'setCanvas', width: 1920, height: 1080, background: null, backgroundBlur: null})")
    r.s.call("window.__nuzky.store.getState().edit({type: 'renameProject', name: 'Morning routine'})")
    bridge = Bridge(r, r.saved_project())
    try:
        job = poll(bridge, bridge.call('transcribe', {'language': 'cs', 'model': MODEL}), 600)
        r.check('the talk is transcribed', job['status'] == 'done', job)
    finally:
        bridge.close()
    edit_the_talk(r)
    suggest(r)
    rename_and_copy(r)
    left_out(r)
    if MACOS:
        # Only macOS lets a flow answer the system's save dialog (src-tauri/src/test_bridge.rs).
        export(r)
    r.check('no error toast', not r.errors(), r.errors())


def edit_the_talk(r):
    """Cuts 4 s out of the first take and plays everything after the first minute at 1.5x."""
    before = project(r)['tracks'][0]['clips'][0]['durationUs']
    r.s.run(EDIT, {'type': 'rippleDeleteRanges', 'ranges': [{'startUs': 6_000_000, 'endUs': 10_000_000}], 'keepTrackIds': None})
    r.s.run(EDIT, {'type': 'splitClip', 'clipId': project(r)['tracks'][0]['clips'][-1]['id'], 'atUs': 56_000_000})
    tail = project(r)['tracks'][0]['clips'][-1]
    r.s.run(EDIT, {'type': 'updateClip', 'clipId': tail['id'], 'speed': 1.5, 'keepPitch': True})
    main = project(r)['tracks'][0]['clips']
    r.check('the talk is cut and its end sped up', len(main) == 3 and main[-1]['content']['speed'] == 1.5
            and sum(c['durationUs'] for c in main) < before - 4_000_000, [(c['durationUs'], c['content']['speed']) for c in main])


def suggest(r):
    r.s.run("window.__nuzky.store.setState({panelTab: 'transcript'})")
    r.check('a talk over 30 s offers Chapters in its Transcript tab', wait(lambda: r.s.run(CLICK, 'Chapters', 'aside') == 'false', 15))
    shown = wait(lambda: r.s.run(f'return {REGION}?.innerText ?? null'), 5)
    r.check('Chapters opens a region that says what Suggest chapters does', shown and 'Suggest chapters' in shown
            and 'Your AI names them' in shown, shown)
    r.shot('chapters-empty')

    r.s.run(CLICK, 'Suggest chapters', '[data-chapters]')
    finding = wait(lambda: 'Your AI is finding chapters' in (r.s.run(f'return {REGION}?.innerText') or ''), 10)
    r.check('Suggest chapters asks the AI panel and says the AI is at work', finding and r.s.run('return window.__nuzky.dock.getState().open'))
    r.check('while the AI edits, the editor is locked', wait(lambda: r.state()['aiRun'], 10))
    r.check('the agent\'s three chapters show up as rows', wait(lambda: len(rows(r)) == 3, 60), rows(r))
    r.check('finding ends with the agent\'s answer', wait(lambda: not r.s.run('return window.__nuzky.chapters.getState().finding'), 20))
    sent = (r.work / 'fake/runs.jsonl').read_text()
    r.check('the panel asked the AI for YouTube chapters', 'Suggest YouTube chapters for this video' in sent)

    listed, shown, said = chapter_list(r), rows(r), words(r)
    r.check('the first chapter is at 00:00 and the times go up', [c['time'] for c in shown][0] == '00:00'
            and [c['startUs'] for c in listed['chapters']] == sorted(c['startUs'] for c in listed['chapters']), listed)
    starts = {w['startUs'] for w in said}
    r.check('each chapter starts where its first word is heard in the cut and sped-up video',
            all(c['startUs'] in starts for c in listed['chapters'][1:]), [c['startUs'] for c in listed['chapters']])
    r.check('each row shows the time in whole seconds that YouTube reads',
            [c['time'] for c in shown] == [f"{c['startUs'] // 60_000_000:02}:{c['startUs'] // 1_000_000 % 60:02}" for c in listed['chapters']],
            [shown, listed['chapters']])
    r.check('the chapters are 10 s or more apart and the list has every one', listed['problem'] is None
            and len(listed['text'].split('\n')) == 3 and all(c['leftOut'] is None for c in listed['chapters']), listed)
    answer = wait(lambda: next((i['text'] for i in reversed(r.s.run('return window.__nuzky.agent.getState().items'))
                                if i['kind'] == 'agent' and 'Kapitoly' in i.get('text', '')), None), 10)
    r.check('the agent got the list Nuzky timed', answer and listed['text'] in answer, answer)
    r.s.run("window.__nuzky.dock.setState({open: false})")
    r.s.run("document.querySelector('[aria-label=\"Fit timeline to view\"]').click()")
    time.sleep(0.5)
    r.shot('chapters')
    small_window(r, 'chapters-small')

    second = listed['chapters'][1]['startUs']
    r.s.run(f"{REGION}.querySelectorAll('[data-chapter-time]')[1].click()")
    playhead = lambda: r.s.run('return window.__nuzky.store.getState().timeUs')
    r.check('clicking a time moves the playhead to the chapter', wait(lambda: playhead() == second, 5), playhead())
    band(r, listed)


def band(r, listed):
    """The timeline splits the video into the chapters, as YouTube's player bar does."""
    shown = r.s.run(SEGMENTS)
    zoom = r.s.run('return window.__nuzky.store.getState().zoom')
    duration = r.s.run('return window.__nuzky.store.getState().snap.project.tracks[0].clips.reduce((e, c) => Math.max(e, c.startUs + c.durationUs), 0)')
    starts = [c['startUs'] for c in listed['chapters']]
    ends = starts[1:] + [duration]
    expected = [(c['title'], round(s / 1e6 * zoom), round((e - s) / 1e6 * zoom - 2)) for c, s, e in zip(listed['chapters'], starts, ends)]
    r.check('the timeline shows one segment per chapter, from its start to the next, with its title',
            [(t, round(x), round(w)) for t, x, w in shown] == expected, [shown, expected])
    r.s.run("document.querySelectorAll('[data-chapter-segment]')[2].click()")
    r.check('clicking a segment moves the playhead to its chapter', wait(lambda: r.s.run('return window.__nuzky.store.getState().timeUs') == starts[2], 5))
    r.check('the segment under the playhead is marked', wait(lambda: 'bg-accent' in r.s.run("return document.querySelectorAll('[data-chapter-segment]')[2].className"), 5))
    r.s.run("document.querySelectorAll('[data-chapter-segment]')[1].dispatchEvent(new MouseEvent('dblclick', {bubbles: true}))")
    focused = wait(lambda: r.s.run("return document.activeElement?.getAttribute('aria-label')") == 'Title of chapter 2', 5)
    r.check('double-clicking a segment opens its title to type in', focused, r.s.run("return document.activeElement?.outerHTML.slice(0, 120)"))
    r.key('Escape')
    toggle = "document.querySelector('[data-chapters-timeline]')"
    r.s.run(f"{toggle}.click()")
    r.check('Chapters on the timeline closes the list', wait(lambda: not r.s.run(f'return !!{REGION}'), 5)
            and r.s.run(f"return {toggle}.getAttribute('aria-pressed')") == 'false')
    r.s.run(f"{toggle}.click()")
    r.check('and opens it again in the Transcript tab', wait(lambda: r.s.run(f'return !!{REGION}'), 5))


def rename_and_copy(r):
    original = rows(r)[1]['title']
    field = f"{REGION}.querySelectorAll('[data-chapter] input')[1]"
    r.s.run(f"{field}.focus()")
    r.s.run(f"""const f = {field}; const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
        set.call(f, 'Proč to funguje'); f.dispatchEvent(new Event('input', {{bubbles: true}}));""")
    r.key('Enter')
    renamed = wait(lambda: [c['title'] for c in project(r)['chapters']][1] == 'Proč to funguje', 5)
    r.check('a title typed and confirmed with Enter is saved in the project', renamed, project(r).get('chapters'))
    r.check('the list for YouTube has the new title', 'Proč to funguje' in chapter_list(r)['text'], chapter_list(r))
    r.s.call('window.__nuzky.store.getState().undo()')
    r.check('one undo brings the title back', wait(lambda: [c['title'] for c in project(r)['chapters']][1] == original, 5))
    r.s.call('window.__nuzky.store.getState().redo()')
    wait(lambda: [c['title'] for c in project(r)['chapters']][1] == 'Proč to funguje', 5)

    # Esc gives the title back without saving.
    r.s.run(f"{field}.focus()")
    r.s.run(f"""const f = {field}; const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
        set.call(f, 'Nic'); f.dispatchEvent(new Event('input', {{bubbles: true}}));""")
    r.key('Escape')
    time.sleep(0.3)
    r.check('Esc in a title gives it back and keeps the region open', r.s.run(f'return {field}.value') == 'Proč to funguje'
            and [c['title'] for c in project(r)['chapters']][1] == 'Proč to funguje' and r.s.run(f'return !!{REGION}'))

    r.s.run('window.__copied = null; navigator.clipboard.writeText = async (t) => { window.__copied = t; };')
    toasts = len(r.state()['toasts'])
    r.s.run(CLICK, 'Copy for YouTube', '[data-chapters]')
    copied = wait(lambda: r.s.run('return window.__copied'), 5)
    r.check('Copy puts the list YouTube takes on the clipboard', copied == chapter_list(r)['text'] and copied.startswith('00:00 '), copied)
    toast = wait(lambda: r.state()['toasts'][toasts:], 5)
    r.check('a toast says it was copied and where to paste it', toast and 'Paste them into the video\'s description on YouTube' in toast[0]['text'], toast)


def left_out(r):
    """A cut just before the third chapter brings it within 10 s of the second."""
    listed = chapter_list(r)
    second, third = (c['startUs'] for c in listed['chapters'][1:])
    cut = {'type': 'rippleDeleteRanges', 'ranges': [{'startUs': second + 2_000_000, 'endUs': third - 1_000_000}], 'keepTrackIds': None}
    r.s.run(EDIT, cut)
    shown = wait(lambda: (rows(r)[2]['leftOut'] or '').startswith('Left out') and rows(r), 10)
    r.check('the chapter the cut brought within 10 s says it is left out and why', shown
            and 'less than 10 s from the chapter next to it' in shown[2]['leftOut'], rows(r))
    problem = r.s.run(f'return {REGION}.innerText')
    copy = r.s.run(f"return [...{REGION}.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Copy for YouTube')?.getAttribute('aria-disabled')")
    r.check('with fewer than 3 left, the list says to suggest them again and Copy is off', 'and only 2 are left.' in problem
            and copy == 'true', [problem, copy])
    marks = r.s.run("return [document.querySelectorAll('[data-chapter-segment]').length, document.querySelectorAll('[data-chapter-left-out]').length]")
    r.check('on the timeline the left-out chapter is a dashed mark, not a segment', marks == [2, 1], marks)
    time.sleep(0.8)  # the cut slides the clips into place
    r.shot('chapters-left-out')
    r.s.call('window.__nuzky.store.getState().undo()')
    r.check('undoing the cut brings the chapter back', wait(lambda: chapter_list(r)['problem'] is None and all(c['leftOut'] is None for c in rows(r)), 10), rows(r))

    # With nothing heard, the transcript is empty and the chapters still say why they are out.
    main = next(t for t in project(r)['tracks'] if t['id'] == 'main')
    r.s.run(EDIT, {'type': 'updateTrack', 'trackId': main['id'], 'muted': True})
    shown = wait(lambda: (now := rows(r)) and len(now) == 3 and now[1]['time'].strip() == '--:--' and now, 10)
    r.check('a muted talk keeps its chapters in the Transcript tab, each saying its words are gone', shown
            and all('its words were cut or moved' in (c['leftOut'] or '') for c in shown[1:]), rows(r))
    r.s.call('window.__nuzky.store.getState().undo()')
    wait(lambda: chapter_list(r)['problem'] is None, 10)


def export(r):
    folder = r.work / 'exports'
    folder.mkdir()
    # Captions in two languages: Whisper's Czech from the transcript and the AI's English.
    r.s.run("window.__nuzky.store.setState({panelTab: 'captions'})")
    r.s.run("[...document.querySelectorAll('button')].find((b) => /enerate captions/.test(b.textContent)).click()")
    czech = wait(lambda: any(t['name'] == 'Captions' and not t.get('translation') and t['clips'] for t in project(r)['tracks']), 300)
    r.check('Whisper makes the Czech captions', czech)
    ready = wait(lambda: r.s.run(f'return {TRANSLATE}?.getAttribute("aria-disabled") !== "true"'), 30)
    r.s.run(f'{TRANSLATE}.click()')
    english = ready and wait(lambda: r.s.run('return window.__nuzky.agent.getState().status') == 'idle'
                             and any(t.get('translation') == 'en' for t in project(r)['tracks']), 120)
    r.check('the AI adds English captions', english)
    r.s.run("window.__nuzky.dock.setState({open: false})")
    expected = chapter_list(r)['text']

    open_dialog(r)
    r.s.run(TOGGLE, 'Also save as .srt')
    r.s.run(TOGGLE, 'Also save as .chapters.txt')
    boxes = wait(lambda: (s := r.s.run(BOX, 'Also save as .srt'), c := r.s.run(BOX, 'Also save as .chapters.txt'))[0]['checked'] and c['checked'] and (s, c), 5)
    r.check('Also save as .chapters.txt sits under the captions and can be checked', boxes, boxes)
    r.shot('export-chapters')
    save(r, 'Export…', to=folder / 'Morning routine.mp4')
    job = wait(lambda: (j := next((j for j in r.state()['jobs'] if j['kind'] == 'export'), None)) and j['status'] != 'running' and j, 300)
    r.check('the video exports', job and job['status'] == 'done', job)
    names = sorted(p.name for p in folder.iterdir())
    r.check('the video, one .srt per language and the chapters land in one folder', names == [
        'Morning routine.chapters.txt', 'Morning routine.cs.srt', 'Morning routine.en.srt', 'Morning routine.mp4'], names)
    text = (folder / 'Morning routine.chapters.txt').read_text(encoding='utf-8') if (folder / 'Morning routine.chapters.txt').exists() else ''
    r.check('the chapters file holds the list YouTube takes, as Copy gives it', text == expected + '\n', [text, expected])
    tracks = project(r)['tracks']
    matches(r, folder / 'Morning routine.cs.srt', next(t for t in tracks if t['name'] == 'Captions' and not t.get('translation')), 'Czech')
    matches(r, folder / 'Morning routine.en.srt', next(t for t in tracks if t.get('translation') == 'en'), 'English')
    r.check('the dialog says the chapters are beside the video', wait(lambda: 'Chapters saved beside it' in r.s.run(DIALOG), 5), r.s.run(DIALOG))
    r.shot('export-chapters-saved')
    close_dialog(r)

    # An existing chapters file is replaced only once someone says so.
    (folder / 'Morning routine.chapters.txt').write_text('my own notes')
    open_dialog(r)
    save(r, 'Export…', to=folder / 'Morning routine.mp4')
    asked = wait(lambda: 'already exist' in r.s.run(DIALOG) and r.s.run(DIALOG), 10)
    r.check('an existing chapters file is kept and the dialog asks', asked and 'Morning routine.chapters.txt' in asked
            and (folder / 'Morning routine.chapters.txt').read_text() == 'my own notes', asked)
    boxes = [r.s.run(BOX, 'Also save as .srt'), r.s.run(BOX, 'Also save as .chapters.txt')]
    r.check('while it asks, which files go beside the video cannot change', all(b['disabled'] for b in boxes), boxes)
    close_dialog(r)

    # Replace agreed for the captions alone never covers a chapters file someone saves meanwhile: the dialog asks again.
    chapters_file = folder / 'Morning routine.chapters.txt'
    chapters_file.unlink()
    open_dialog(r)
    save(r, 'Export…', to=folder / 'Morning routine.mp4')
    asked = wait(lambda: 'already exist' in r.s.run(DIALOG) and r.s.run(DIALOG), 10)
    r.check('with only the captions there, the dialog asks about them alone', asked and 'Morning routine.chapters.txt' not in asked, asked)
    chapters_file.write_text("a colleague's notes")
    r.s.run("[...document.querySelectorAll('[role=dialog] button')].find((b) => b.textContent.trim() === 'Replace').click()")
    again = wait(lambda: 'Morning routine.chapters.txt already exists' in r.s.run(DIALOG) and r.s.run(DIALOG), 10)
    r.check('a chapters file that appeared before Replace is asked about on its own and kept', again
            and chapters_file.read_text() == "a colleague's notes", r.s.run(DIALOG))
    close_dialog(r)

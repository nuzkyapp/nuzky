import re
import subprocess
import time
from pathlib import Path

from e2e.bilingual import ADD, english, setup, speech
from e2e.corrections import generate_captions
from e2e.harness import MACOS, flow, wait, webdriver

DIALOG = "return document.querySelector('[role=dialog]')?.innerText ?? ''"
BUTTON = """return (([b]) => b && {disabled: b.getAttribute('aria-disabled') === 'true' || b.disabled, title: b.title})(
  [...document.querySelectorAll('[role=dialog] button')].filter((b) => b.textContent.trim() === arguments[0]))"""
CLICK = "[...document.querySelectorAll('[role=dialog] button')].find((b) => b.textContent.trim() === arguments[0]).click()"
SRT_BOX = """return (([l]) => l && {checked: l.querySelector('input').checked, disabled: l.querySelector('input').disabled, title: l.title})(
  [...document.querySelectorAll('[role=dialog] label')].filter((l) => l.textContent.includes('Also save as .srt')))"""
TOGGLE = "[...document.querySelectorAll('[role=dialog] label')].find((l) => l.textContent.includes('Also save as .srt')).querySelector('input').click()"


def stamp_ms(stamp):
    h, m, rest = stamp.split(':')
    s, ms = rest.split(',')
    return ((int(h) * 60 + int(m)) * 60 + int(s)) * 1000 + int(ms)


def cues(text):
    """An SRT file's captions as (start ms, end ms, text), checking that they are numbered from 1."""
    found = []
    for i, block in enumerate(b for b in text.replace('\r\n', '\n').strip().split('\n\n') if b.strip()):
        number, times, *lines = block.split('\n')
        assert number == str(i + 1), block
        start, end = times.split(' --> ')
        found.append((stamp_ms(start), stamp_ms(end), '\n'.join(lines)))
    return found


def expected(track):
    """What a captions track says, as the export must write it: in time order, times rounded to milliseconds."""
    clips = sorted(track['clips'], key=lambda c: c['startUs'])
    return [((c['startUs'] + 500) // 1000, (c['startUs'] + c['durationUs'] + 500) // 1000, c['content']['text'].strip()) for c in clips]


def ffmpeg_cues(path):
    """The captions FFmpeg reads from the file, written back out as SRT."""
    out = subprocess.run(['ffmpeg', '-v', 'error', '-i', str(path), '-f', 'srt', '-'], capture_output=True, text=True, check=True).stdout
    return cues(out)


def matches(r, path, track, label):
    text = path.read_text(encoding='utf-8') if path.exists() else ''
    ours = cues(text) if text else []
    r.check(f'{label}: {path.name} has every caption of the track with its times and text', ours == expected(track) and ours,
            {'file': ours[:3], 'track': expected(track)[:3]})
    r.check(f'{label}: FFmpeg reads the same captions from it', text and ffmpeg_cues(path) == ours)


def sheet(r):
    return webdriver('GET', r.s.path + '/nuzky/sheet')


def native(r, label):
    """Clicks `label` and returns the folder and name the system's save dialog offers, then cancels it."""
    r.s.run(CLICK, label)
    offered = wait(lambda: sheet(r), 5)
    time.sleep(0.8)  # AppKit lets the sheet slide in
    webdriver('POST', r.s.path + '/nuzky/sheet/cancel')
    wait(lambda: sheet(r) is None, 5)
    if offered and offered.get('folder'):
        # rfd starts the dialog on the folder with the name added (rfd's set_path), which AppKit shows as the folder.
        folder = Path(offered['folder'])
        offered['folder'] = str(folder.parent if folder.name == offered['name'] else folder)
    return offered


def save(r, label, to=None):
    """Clicks `label` and saves in the save dialog it opens, at `to` or under the name and in the folder offered, and
    returns what it offered. The dialog runs in another process on macOS, which a flow cannot press, so the app answers it
    as Save does (`answerNextSave`); `native` checks what the system's dialog offers."""
    count = r.s.run('window.__nuzky.answerNextSave(arguments[0] ?? undefined); return window.__nuzky.savesAsked.length',
                    to and str(to))
    r.s.run(CLICK, label)
    asked = wait(lambda: r.s.run('return window.__nuzky.savesAsked[arguments[0]] ?? null', count), 5)
    path = Path(asked) if asked else None
    return path and {'folder': str(path.parent), 'name': path.name}


def open_dialog(r):
    r.key('e', ctrlKey=True)
    shown = wait(lambda: 'Export video' in r.s.run(DIALOG), 5)
    time.sleep(0.5)  # the dialog fades in over 200 ms
    return shown


def close_dialog(r):
    r.key('Escape')
    wait(lambda: not r.s.run(DIALOG), 5)


def before(r):
    setup(r)
    # Tauri's Videos folder is ~/Movies on macOS; the save dialog opens there only when it exists.
    (r.work / 'home/Movies').mkdir(parents=True, exist_ok=True)


def export_srt(r):
    movies, folder = r.work / 'home/Movies', r.work / 'exports'
    folder.mkdir()
    r.s.call("window.__nuzky.store.getState().edit({type: 'renameProject', name: 'Morning routine'})")
    r.check('without captions the export dialog opens', open_dialog(r), r.s.run(DIALOG))
    box, only = r.s.run(SRT_BOX), r.s.run(BUTTON, 'Export .srt only…')
    no_captions = 'No captions yet. Make them in the Captions tab first.'
    r.check('without captions Also save as .srt and Export .srt only… are off and say why',
            box and box['disabled'] and box['title'] == no_captions and only and only['disabled'] and only['title'] == no_captions, [box, only])
    close_dialog(r)

    # One captions track: Whisper's Czech captions. The file is named like the video, and the dialog names no file.
    r.s.run("window.__nuzky.speech.setState({model: 'large-v3-turbo-q5_0', language: 'cs'})")
    job = generate_captions(r)
    r.check('Whisper makes the Czech captions', job and job['status'] == 'done' and speech(r), job)
    open_dialog(r)
    r.s.run(TOGGLE)
    box = wait(lambda: (b := r.s.run(SRT_BOX)) and b['checked'] and b, 5)
    dialog = r.s.run(DIALOG)
    r.check('with one captions track Also save as .srt can be checked and no file name is shown',
            box and not re.search(r'\w\.srt', dialog), dialog)
    r.shot('srt-one-track')
    offered = native(r, 'Export .srt only…')
    r.check("Export .srt only… opens the system's save dialog in Videos, named after the project, as .srt",
            offered and Path(offered['folder']).resolve() == movies.resolve() and offered['name'] == 'Morning routine.srt', offered)
    r.check('Cancel there keeps the export dialog as it was', r.s.run(BUTTON, 'Export .srt only…') and not list(movies.iterdir()))
    # Saved in a folder of the user's own, which every export of the project then starts in.
    one = folder / 'Morning routine.srt'
    offered = save(r, 'Export .srt only…', to=one)
    r.check('the app asks for the same folder and name again', offered and Path(offered['folder']).resolve() == movies.resolve()
            and offered['name'] == 'Morning routine.srt', offered)
    r.check('it saves one file under the name chosen, without a language', wait(lambda: 'Saved to' in r.s.run(DIALOG), 10)
            and sorted(p.name for p in folder.iterdir()) == ['Morning routine.srt'], sorted(p.name for p in folder.iterdir()))
    matches(r, one, speech(r), 'one track')
    r.check('the dialog says where it went, with Show in folder and Done', 'Saved to ' + str(one) in r.s.run(DIALOG)
            and r.s.run(BUTTON, 'Show in folder') and r.s.run(BUTTON, 'Done'), r.s.run(DIALOG))
    r.shot('srt-only-saved')
    close_dialog(r)

    # Two tracks: the AI translates the captions into English. Each language goes into a file of its own.
    ready = wait(lambda: r.s.run(f'return {ADD}?.getAttribute("aria-disabled") !== "true"'), 15)
    r.s.run(f'{ADD}.click()')
    en = ready and wait(lambda: r.s.run('return window.__nuzky.agent.getState().status') == 'idle' and english(r), 60)
    r.check('the AI adds English captions', en, r.s.run('return window.__nuzky.agent.getState().items.map((i) => i.kind)'))
    r.s.run("window.__nuzky.dock.setState({open: false})")
    open_dialog(r)
    shown = wait(lambda: 'Morning routine.cs.srt · Morning routine.en.srt' in r.s.run(DIALOG), 10)
    r.check('Also save as .srt is remembered, and with two languages the dialog names both files', shown and r.s.run(SRT_BOX)['checked'],
            r.s.run(DIALOG))
    r.shot('srt-two-languages')
    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    wait(lambda: r.s.run('return [innerWidth, innerHeight]') == [1024, 640], 5)
    time.sleep(1)
    r.shot('srt-two-languages-small')
    fits = "return (({top, bottom}) => top >= 0 && bottom <= innerHeight)(document.querySelector('[role=dialog]').getBoundingClientRect())"
    r.check('at the smallest window the whole dialog shows', r.s.run(fits))
    webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
    wait(lambda: r.s.run('return [innerWidth, innerHeight]') == [1440, 900], 5)
    # The Czech captions hidden, as for a video that goes to YouTube with its captions in a file: they are still saved.
    hide = "window.__nuzky.store.getState().edit({type: 'updateTrack', trackId: arguments[0], hidden: arguments[1]})"
    r.s.call(hide, speech(r)['id'], True)
    wait(lambda: speech(r)['hidden'], 5)
    offered = save(r, 'Export…')
    r.check('Export… asks in the folder the captions went to, named after the project', offered
            and Path(offered['folder']).resolve() == folder.resolve() and offered['name'] == 'Morning routine.mp4', offered)
    job = wait(lambda: (j := next((j for j in r.state()['jobs'] if j['kind'] == 'export'), None)) and j['status'] != 'running' and j, 300)
    r.check('the video exports', job and job['status'] == 'done', job)
    video, cs, en_file = folder / 'Morning routine.mp4', folder / 'Morning routine.cs.srt', folder / 'Morning routine.en.srt'
    r.check('the video and one .srt per language land in one folder, named after the video',
            video.exists() and cs.exists() and en_file.exists(), sorted(p.name for p in folder.iterdir()))
    matches(r, cs, speech(r), 'Czech, hidden in the picture')
    matches(r, en_file, english(r), 'English')
    r.check('the dialog names the captions beside the video', wait(
        lambda: 'Captions saved beside it: Morning routine.cs.srt · Morning routine.en.srt' in r.s.run(DIALOG), 5), r.s.run(DIALOG))
    r.shot('video-and-srt-saved')
    close_dialog(r)
    r.s.call(hide, speech(r)['id'], False)

    # Existing captions files are replaced only once someone says so.
    cs.write_text('my own notes')
    open_dialog(r)
    save(r, 'Export .srt only…')
    asked = wait(lambda: 'Morning routine.cs.srt and Morning routine.en.srt already exist. Replace them?' in r.s.run(DIALOG), 10)
    r.check('existing .srt files are kept and the dialog asks about both', asked and cs.read_text() == 'my own notes', r.s.run(DIALOG))
    r.shot('srt-replace')
    r.s.run(CLICK, 'Replace')
    r.check('Replace writes them', wait(lambda: 'Saved to' in r.s.run(DIALOG), 10) and cs.read_text() != 'my own notes')
    matches(r, cs, speech(r), 'replaced')
    close_dialog(r)

    # The cover goes to the same folder as the video and its captions.
    r.s.run("[...document.querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === 'Make a cover' || b.title === 'Make a cover').click()")
    r.check('the Cover tile opens the cover editor', wait(lambda: r.s.run("return !!document.querySelector('[data-testid=cover-stage]')"), 10))
    r.key('e', ctrlKey=True)
    wait(lambda: 'Export cover' in r.s.run(DIALOG), 5)
    time.sleep(0.5)
    offered = save(r, 'Export…')
    r.check('the cover is offered the same folder', offered and Path(offered['folder']).resolve() == folder.resolve()
            and offered['name'] == 'Morning routine cover.png', offered)
    r.check('the cover is saved beside them', wait(lambda: (folder / 'Morning routine cover.png').exists(), 60), sorted(p.name for p in folder.iterdir()))
    r.check('nothing else was written to Videos', not list(movies.iterdir()), sorted(p.name for p in movies.iterdir()))
    r.check('no error toast', not r.errors(), r.errors())


if MACOS:
    # Only macOS lets a flow save through the system's save dialog (src-tauri/src/test_bridge.rs).
    flow('export_srt', 'Export saves the captions as .srt, alone or beside the video, one file per language with the times '
         'and text of the project, and the video, its captions and its cover go to one folder', before=before)(export_srt)

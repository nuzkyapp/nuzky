"""Record screen in Media on macOS, with a stand-in for `screencapture` that records nothing (fake_screencapture): nothing
records until the user picks the whole screen or a window, the top bar shows the recording with Stop and Delete, Stop
adds it to Media as one undo step, Delete leaves nothing behind, and a recording the system ends lands in Media too."""
import json, subprocess, time
from pathlib import Path

from e2e.harness import MACOS, Bridge, flow, wait, webdriver

MENU = "document.querySelector('[role=menu][aria-label=\"Record screen\"]')"
INDICATOR = "document.querySelector('[data-screen-recording]')"



def small_window(r, name):
    """A screenshot in the smallest window, 1024 × 640, then back to 1440 × 900."""
    for width, height, shot in ((1024, 640, name), (1440, 900, None)):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        if shot:
            time.sleep(0.5)
            r.shot(shot)


def setup(r):
    video = r.work / 'screen.mov'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=s=1920x1080:r=30:d=3', '-c:v', 'libx264',
                    '-preset', 'veryfast', '-pix_fmt', 'yuv420p', '-an', str(video)], check=True)
    r.env.update(NUZKY_SCREENCAPTURE=str(Path(__file__).with_name('fake_screencapture')),
                 NUZKY_FAKE_SCREENCAPTURE_LOG=str(r.work / 'screencapture.log'), NUZKY_FAKE_SCREENCAPTURE_VIDEO=str(video),
                 NUZKY_FAKE_SCREENCAPTURE_END=str(r.work / 'system-stop'))
    (r.work / 'screencapture.log').write_text('')


def runs(r):
    return [json.loads(line) for line in (r.work / 'screencapture.log').read_text().splitlines()]


def recordings(r):
    return sorted((r.work / 'data/nuzky/recordings').glob('*.mov')) if (r.work / 'data/nuzky/recordings').is_dir() else []


def assets(r):
    return r.s.run('return window.__nuzky.store.getState().snap.project.assets', retries=3)


def open_menu(r):
    r.s.run("document.querySelector('[data-record-screen]').click()")
    return wait(lambda: r.s.run(f"const m = {MENU}; return m && [...m.querySelectorAll('[role=menuitem]')].map((i) => i.textContent)"), 10)


def choose(r, index):
    r.s.run(f"{MENU}.querySelectorAll('[role=menuitem]')[arguments[0]].click()", index)
    return wait(lambda: r.s.run(f'return {INDICATOR}?.textContent ?? null'), 10)


def record_screen(r):
    time.sleep(2)
    r.check('nothing records the screen before the user asks', runs(r) == [] and not r.s.run(f'return !!{INDICATOR}'), runs(r))
    tile = r.s.run("const b = document.querySelector('[data-record-screen]'); return b && [b.getAttribute('aria-label'), b.getAttribute('aria-disabled')]")
    r.check('Media offers Record screen', tile and tile[0] == 'Record screen' and tile[1] != 'true', tile)
    r.shot('media')

    items = open_menu(r)
    r.check('its menu offers the whole screen and the windows of other apps', items and items[0] == 'Whole screen', items)
    time.sleep(0.3)
    r.shot('menu')
    shown = choose(r, 0)
    r.check('the top bar shows the recording with Stop', shown and shown.startswith('Recording screen') and 'Stop' in shown, shown)
    started = runs(r)
    target = started and Path(started[-1][-1])
    r.check('screencapture records video of the whole screen into Nuzky\'s own folder',
            len(started) == 1 and started[0][:2] == ['-x', '-v'] and not any(a.startswith('-l') for a in started[0])
            and target.parent.resolve() == (r.work / 'data/nuzky/recordings').resolve(), started)
    r.check('the Media tile now stops it', r.s.run("return document.querySelector('[data-record-screen]').getAttribute('aria-label')") == 'Stop recording')
    time.sleep(1.2)
    r.shot('recording')

    count = len(assets(r))
    r.s.run(f"[...{INDICATOR}.querySelectorAll('button')].find((b) => b.textContent === 'Stop').click()")
    added = wait(lambda: (a := assets(r)) and len(a) == count + 1 and a[-1], 20)
    r.check('Stop adds the recording to Media as a silent video', added and added['kind'] == 'video' and not added['hasAudio']
            and Path(added['path']).resolve() == target.resolve() and target.is_file(), added)
    r.check('it is named after what it showed', added and added['name'].strip() != '', added and added['name'])
    r.check('the top bar no longer shows a recording', wait(lambda: not r.s.run(f'return !!{INDICATOR}'), 5))
    r.check('nothing is placed on the timeline', all(not t['clips'] for t in r.state()['tracks']), r.state()['tracks'])
    time.sleep(0.5)
    r.shot('added')
    small_window(r, 'added-1024')
    r.key('z', ctrlKey=True)
    r.check('one undo takes it out of Media', wait(lambda: len(assets(r)) == count, 5), [a['name'] for a in assets(r)])

    # A window, then Delete: nothing is added and the file is gone.
    items = open_menu(r)
    windows = items[2:] if items and len(items) > 2 else []
    if r.check('the menu lists at least one window to record', windows, items):
        shown = choose(r, 2)
        args = runs(r)[-1]
        r.check('a window is recorded by its id', shown and any(a.startswith('-l') and a[2:].isdigit() for a in args), [shown, args])
        r.check('the top bar names the window\'s app', shown and windows[0].split(' · ')[0] in shown, [shown, windows[0]])
        r.s.run(f"{INDICATOR}.querySelector('[aria-label=\"Delete recording\"]').click()")
        r.check('Delete stops it without adding anything', wait(lambda: not r.s.run(f'return !!{INDICATOR}'), 20)
                and len(assets(r)) == count, [a['name'] for a in assets(r)])
        r.check('and deletes what it recorded', wait(lambda: not Path(args[-1]).exists(), 5), recordings(r))

    # The system's own Stop in the menu bar ends screencapture; the recording lands in Media all the same.
    open_menu(r)
    choose(r, 0)
    (r.work / 'system-stop').touch()
    r.check('a recording the system ends by itself is added to Media', wait(lambda: len(assets(r)) == count + 1, 20)
            and not r.s.run(f'return !!{INDICATOR}'), [a['name'] for a in assets(r)])
    r.check('no error toast', not r.errors(), r.errors())

    # While the AI edits, the project refuses the recording the system ended: it stays, with Add and ×, until added.
    (r.work / 'system-stop').unlink()
    open_menu(r)
    choose(r, 0)
    count = len(assets(r))
    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Edit'})['run_id']
        wait(lambda: r.state()['aiRun'], 10)
        (r.work / 'system-stop').touch()
        kept = wait(lambda: r.s.run(f'return {INDICATOR}?.textContent ?? null') if 'not added' in (r.s.run(f'return {INDICATOR}?.textContent ?? ""') or '') else None, 20)
        path = Path(runs(r)[-1][-1])
        r.check('a recording refused while the AI edits stays, with Add', kept and 'Add' in kept and path.is_file()
                and len(assets(r)) == count, [kept, path.is_file()])
        r.shot('not-added')
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
        wait(lambda: not r.state()['aiRun'], 10)
    finally:
        bridge.close()
    r.s.run(f"[...{INDICATOR}.querySelectorAll('button')].find((b) => b.textContent === 'Add').click()")
    added = wait(lambda: (a := assets(r)) and len(a) == count + 1 and a[-1], 20)
    r.check('Add then puts it in Media', added and Path(added['path']).resolve() == path.resolve()
            and not r.s.run(f'return !!{INDICATOR}'), added)


if MACOS:
    flow('screen_recording', 'Record screen in Media records nothing until the user picks the whole screen or a window; the '
         'top bar shows it with Stop and Delete, Stop adds it to Media as one undo step, Delete leaves nothing, and a '
         'recording the system ends lands in Media', before=setup)(record_screen)

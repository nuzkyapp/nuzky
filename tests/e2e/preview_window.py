import subprocess, time

from e2e.harness import CLI, LINUX, close_window, flow, wait, webdriver

BUTTON = "document.querySelector('[data-preview-window]')"
# What the canvas shows: its size, the time of its frame and a checksum of its pixels.
FRAME = """const c = document.querySelector('canvas');
if (!c || !c.dataset.us) return null;
const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
let h = 0;
for (let i = 0; i < d.length; i += 61) h = (h * 31 + d[i]) | 0;
return {size: [c.width, c.height], us: Number(c.dataset.us), sum: h};"""
MAIN_FRAME = FRAME.replace("document.querySelector('canvas')", "document.querySelector('section[aria-label=Preview] canvas')")
WINDOW_STATE = 'return window.__nuzkyWindow.state.getState()'
OFF_CENTRE = """const play = [...document.querySelectorAll('section[aria-label=Preview] button')].find((b) => /^(Play|Pause)/.test(b.getAttribute('aria-label') || ''));
const p = play.getBoundingClientRect(), o = play.closest('.bar').getBoundingClientRect();
return Math.round((p.left + p.width / 2) - (o.left + o.width / 2))"""
# Records what the window asks of itself instead of doing it: full screen would take over the screen of whoever runs
# the flows, and on macOS bring out the window the run keeps off every screen.
STUB = """const w = window.__nuzkyWindow.win;
window.__fullscreen = [];
w.isFullscreen = () => Promise.resolve(window.__fullscreen.at(-1) ?? false);
w.setFullscreen = (on) => { window.__fullscreen.push(on); window.__nuzkyWindow.state.setState({full: on}); return Promise.resolve(); };"""
CAPTION = {'type': 'addCaptions', 'segments': [{'startUs': 0, 'endUs': 6_000_000, 'text': 'Druhý monitor', 'words': []}],
           'style': {'fontFamily': None, 'fontSize': 140, 'color': '#ffffff', 'bold': True, 'strokeWidth': 11,
                     'strokeColor': '#000000', 'background': None}}


def setup(r):
    video = r.work / 'tone.mp4'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=s=1080x1920:r=30:d=6', '-f', 'lavfi',
                    '-i', 'sine=f=440:d=6', '-shortest', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-c:a', 'aac',
                    str(video)], check=True)
    project = r.work / 'data/nuzky/projects/second-screen.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(video)], env=r.env, check=True, capture_output=True)


def handles(r):
    return webdriver('GET', r.s.path + '/window/handles', retries=3)


def switch(r, handle):
    webdriver('POST', r.s.path + '/window', {'handle': handle})


def pressed(r):
    return r.s.run(f'const b = {BUTTON}; return b && b.getAttribute("aria-pressed")')


@flow('preview_window', 'Shift+F or the button beside Full screen shows the preview in a window of its own, for another '
      'screen: the same frames while paused, seeking, editing and playing, its keys play and seek the editor, F makes it '
      'full screen, frames are drawn for the larger of the two previews, and closing it leaves the editor and the '
      'project as they were', before=setup)
def preview_window(r):
    main = webdriver('GET', r.s.path + '/window')
    wait(lambda: r.s.run(f'return {BUTTON}?.getAttribute("aria-label")'), 10)
    label = r.s.run(f'return {BUTTON}.getAttribute("aria-label")')
    r.check('the transport offers the preview window with its key', label == 'Open the preview window (Shift+F)', label)
    off = r.s.run(OFF_CENTRE)
    # Linux's fonts make the right side 2 px wider than its half at 1440 px, which nudges Play that far.
    r.check(f'with the button the transport keeps Play in its middle ({off} px off)', abs(off) <= 2, off)
    saved = r.saved_project().read_bytes()
    alone = wait(lambda: r.s.run(MAIN_FRAME), 10)

    r.s.run("window.__nuzky.store.setState({aiRun: 'Cutting pauses', lockShown: true})")
    locked = r.s.run(f'const b = {BUTTON}; return b.disabled || b.getAttribute("aria-disabled") === "true"')
    r.s.run("window.__nuzky.store.setState({aiRun: null, lockShown: false})")
    r.check('it stays open to the user while an agent edits, as it changes nothing in the project', locked is False, locked)

    r.key('F', shiftKey=True)
    if not r.check('Shift+F opens a second window', wait(lambda: len(handles(r)) == 2, 10), handles(r)):
        return
    preview = next(h for h in handles(r) if h != main)
    r.check('the button shows it is open', wait(lambda: pressed(r) == 'true', 5)
            and r.s.run(f'return {BUTTON}.getAttribute("aria-label")') == 'Close the preview window (Shift+F)')

    def both():
        switch(r, preview)
        theirs = r.s.run(FRAME, retries=3)
        switch(r, main)
        return theirs, r.s.run(MAIN_FRAME, retries=3)

    def same(us=None):
        def check():
            theirs, ours = both()
            return theirs and ours and theirs == ours and (us is None or theirs['us'] == us) and (theirs, ours)
        return wait(check, 10, 0.3) or both()

    first = same()
    r.check('the window shows the frame the editor shows, pixel for pixel', first[0] and first[0] == first[1], first)
    switch(r, preview)
    time.sleep(0.4)
    r.shot('window')
    switch(r, main)
    r.shot('editor-with-window')

    r.seek(3_000_000)
    seeked = same(3_000_000)
    r.check('seeking in the editor moves the window to the same frame', seeked[0] and seeked[0] == seeked[1] and seeked[0]['us'] == 3_000_000, seeked)
    time.sleep(1.5)
    r.check('opening the window and seeking leave the saved project as it was, byte for byte', r.saved_project().read_bytes() == saved)

    added = r.s.call('window.__nuzky.store.getState().edit(arguments[0])', CAPTION)
    edited = wait(lambda: (lambda f: f if f[0] and f[0] == f[1] and f[0]['sum'] != seeked[0]['sum'] else None)(both()), 10) or both()
    r.check('an edit shows in the window at once, the same as in the editor', added['ok'] and edited[0] and edited[0] == edited[1]
            and edited[0]['sum'] != seeked[0]['sum'], [added, edited])
    switch(r, preview)
    time.sleep(0.4)
    r.shot('window-after-edit')

    r.s.run(STUB)
    start = r.s.run(WINDOW_STATE)['timeUs']
    r.key(' ')
    playing = wait(lambda: r.s.run(WINDOW_STATE)['playing'], 5)
    moved = wait(lambda: r.s.run(FRAME)['us'] > start + 700_000, 8)
    r.key(' ')
    r.check('Space in the window plays the video there, the frames moving on', playing and moved, [start, r.s.run(FRAME)])
    stopped = wait(lambda: not r.s.run(WINDOW_STATE)['playing'], 5)
    window_time = r.s.run(WINDOW_STATE)['timeUs']
    switch(r, main)
    editor = r.s.run('const s = window.__nuzky.store.getState(); return {playing: s.playing, timeUs: s.timeUs}')
    r.check('and pauses the editor at the same moment as the window', stopped and not editor['playing']
            and editor['timeUs'] == window_time and window_time > start, [window_time, editor])
    paused = same(window_time)
    r.check('after playing both show the frame where it paused', paused[0] and paused[0] == paused[1], paused)

    switch(r, preview)
    r.key('ArrowRight')
    stepped = wait(lambda: r.s.run(WINDOW_STATE)['timeUs'] > window_time, 5)
    r.check('→ in the window steps the editor a frame on', stepped, [window_time, r.s.run(WINDOW_STATE)['timeUs']])
    r.key('Home')
    r.check('Home in the window goes to the start', wait(lambda: r.s.run(FRAME)['us'] == 0, 5), r.s.run(FRAME))
    r.s.run("document.querySelector('button[aria-label^=\"Play\"]').click()")
    played = wait(lambda: r.s.run(WINDOW_STATE)['playing'], 5)
    r.s.run("document.querySelector('button[aria-label^=\"Pause\"]').click()")
    r.check('its Play and Pause buttons play and pause too', played and wait(lambda: not r.s.run(WINDOW_STATE)['playing'], 5))

    switch(r, main)
    r.s.run("window.__nuzky.keys.setState({changed: {playPause: ['P']}})")
    switch(r, preview)
    r.key('P')
    replayed = wait(lambda: r.s.run(WINDOW_STATE)['playing'], 5)
    r.key('P')
    r.check('a key changed in Settings while the window is open plays there at once',
            replayed and wait(lambda: not r.s.run(WINDOW_STATE)['playing'], 5))
    switch(r, main)
    r.s.run("window.__nuzky.keys.setState({changed: {}})")
    switch(r, preview)

    r.key('f')
    r.check('F asks for full screen, and the button turns into Exit full screen',
            r.s.run('return window.__fullscreen') == [True]
            and wait(lambda: r.s.run("return !!document.querySelector('button[aria-label=\"Exit full screen (F)\"]')"), 5),
            r.s.run('return window.__fullscreen'))
    r.key('Escape')
    r.check('Esc leaves full screen and keeps the window', r.s.run('return window.__fullscreen') == [True, False] and len(handles(r)) == 2,
            r.s.run('return window.__fullscreen'))
    r.s.run("document.querySelector('button[aria-label=\"Full screen (F)\"]').click()")
    r.check('the button asks for full screen too', wait(lambda: r.s.run('return window.__fullscreen') == [True, False, True], 5),
            r.s.run('return window.__fullscreen'))
    r.s.run("window.__nuzkyWindow.state.setState({full: false})")

    small = r.s.run(FRAME)['size']
    webdriver('POST', r.s.path + '/window/rect', {'width': 1400, 'height': 1100})
    larger = wait(lambda: (lambda f: f if f['size'][1] > small[1] * 1.3 else None)(r.s.run(FRAME)), 10) or r.s.run(FRAME)
    switch(r, main)
    ours = wait(lambda: (lambda f: f if f['size'] == larger['size'] else None)(r.s.run(MAIN_FRAME)), 5) or r.s.run(MAIN_FRAME)
    r.check('a larger window gets frames drawn in more pixels, not stretched, and the editor the same frames',
            larger['size'][1] > small[1] * 1.3 and ours['size'] == larger['size'], [small, larger, ours])

    switch(r, main)
    editor = 'const s = window.__nuzky.store.getState(); return {view: s.view, timeUs: s.timeUs, playing: s.playing, tracks: s.snap.project.tracks.length}'
    before = r.s.run(editor)
    switch(r, preview)
    r.key('F', shiftKey=True)
    closed = wait(lambda: len(handles(r)) == 1, 10)
    switch(r, main)
    after = r.s.run(editor)
    r.check('Shift+F in the window closes it; the editor stays where it was, with its playhead and the edit',
            closed and wait(lambda: pressed(r) == 'false', 5) and after == before and after['tracks'] == 2, [before, after])
    back = wait(lambda: (lambda f: f if f['size'] == alone['size'] else None)(r.s.run(MAIN_FRAME)), 5) or r.s.run(MAIN_FRAME)
    r.check('the editor\'s preview draws for its own size again', back['size'] == alone['size'], [alone, back])

    r.s.run(f'{BUTTON}.click()')
    r.check('the button opens it again', wait(lambda: len(handles(r)) == 2, 10) and wait(lambda: pressed(r) == 'true', 5))
    preview = next(h for h in handles(r) if h != main)
    switch(r, preview)
    # The title-bar button: Tauri's close asks the same CloseRequested question as the editor's own. WebKitWebDriver
    # closes the page without asking the window, so on Linux the window manager asks it, as the button would.
    if LINUX:
        close_window(title='Nuzky Preview')
    else:
        webdriver('DELETE', r.s.path + '/window')
    switch(r, main)
    r.check('its title-bar close closes only the window; the editor does not go back to the projects',
            wait(lambda: len(handles(r)) == 1, 10) and wait(lambda: pressed(r) == 'false', 5)
            and r.s.run('return window.__nuzky.store.getState().view') == 'editor', handles(r))

    r.s.run(f'{BUTTON}.click()')
    wait(lambda: len(handles(r)) == 2, 10)
    switch(r, next(h for h in handles(r) if h != main))
    wait(lambda: r.s.run("return !!document.querySelector('main[aria-label=\"Preview window\"] canvas')", retries=3), 10)
    r.key('w', ctrlKey=True)
    switch(r, main)
    r.check('Ctrl+W in the window closes it', wait(lambda: len(handles(r)) == 1, 10) and wait(lambda: pressed(r) == 'false', 5), handles(r))

    asset = r.s.run('return window.__nuzky.store.getState().snap.project.assets[0].id')
    r.s.run("window.__nuzky.source.setState({asset: window.__nuzky.store.getState().snap.project.assets[0]})")
    r.check('a media item shown in the preview keeps the button in its transport', asset and wait(lambda: pressed(r) == 'false', 5))
    r.s.run(f'{BUTTON}.click()')
    wait(lambda: len(handles(r)) == 2, 10)
    r.s.run("window.__nuzky.source.setState({asset: null})")
    r.s.run("window.__nuzky.store.setState({view: 'home'})")
    r.check('leaving the editor for the projects closes the window', wait(lambda: len(handles(r)) == 1, 10), handles(r))
    r.s.run("window.__nuzky.store.setState({view: 'editor'})")
    r.check('no error toast', not r.errors(), r.errors())

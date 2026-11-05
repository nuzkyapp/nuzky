import subprocess, time

from e2e.harness import CLI, flow, wait, webdriver

BUTTON = "document.querySelector(`section[aria-label=Preview] button[aria-label^='${arguments[0]}']`)"
LAYOUT = """const s = document.querySelector('section[aria-label=Preview]').getBoundingClientRect();
const c = document.querySelector('section[aria-label=Preview] canvas');
return {section: [s.left, s.top, s.width, s.height], window: [innerWidth, innerHeight], frame: [c.width, c.height]};"""
# Records what the app asks of its window instead of doing it: full screen would take over the screen of whoever
# runs the flows, and on macOS bring out the window the run keeps off every screen.
STUB = """const win = window.__nuzky.previewWindow;
window.__fullscreen = [];
win.isFullscreen = () => Promise.resolve(false);
win.setFullscreen = (on) => { window.__fullscreen.push(on); return Promise.resolve(); };"""
# A window already in full screen, answering only after a moment, as a real window may.
SLOW_FULL = """window.__fullscreen = [];
window.__nuzky.previewWindow.isFullscreen = () => new Promise((done) => setTimeout(() => done(true), 1000));"""
# Records what the preview tells the engine; the sound card's silence itself cannot be heard from a run.
SPY = """const api = window.__nuzky.api, send = api.setPreviewMuted;
window.__muted = [];
api.setPreviewMuted = (on) => { window.__muted.push(on); return send(on); };"""
FOCUS_IN_PREVIEW = "return document.activeElement?.matches('section[aria-label=Preview]')"
# The transport's three parts, left to right, and whether each fits in the bar beside the next.
TRANSPORT = """const bar = document.querySelector('section[aria-label=Preview] .bar'), b = bar.getBoundingClientRect();
const parts = [...bar.children].map((e) => e.getBoundingClientRect());
return {fits: parts.every((p, i) => p.left >= b.left && p.right <= (parts[i + 1]?.left ?? b.right)),
        parts: parts.map((p) => [Math.round(p.left), Math.round(p.right)]), bar: [b.left, b.right]};"""
SOUND = """const p = window.__nuzky.store.getState().snap.project;
return {muted: p.tracks[0].muted, volume: p.tracks[0].clips[0].content.volume};"""
CAPTION = {'type': 'addCaptions', 'segments': [{'startUs': 0, 'endUs': 6_000_000, 'text': 'Poslední dva', 'words': []}],
           'style': {'fontFamily': None, 'fontSize': 110, 'color': '#ffffff', 'bold': True, 'strokeWidth': 11,
                     'strokeColor': '#000000', 'background': None}}


def setup(r):
    video = r.work / 'tone.mp4'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=s=1080x1920:r=30:d=6', '-f', 'lavfi',
                    '-i', 'sine=f=440:d=6', '-shortest', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-c:a', 'aac',
                    str(video)], check=True)
    project = r.work / 'data/nuzky/projects/preview.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(video)], env=r.env, check=True, capture_output=True)


def layout(r):
    return r.s.run(LAYOUT, retries=3)


def pressed(r, label):
    return r.s.run(f'const b = {BUTTON}; return b && b.getAttribute("aria-pressed")', label)


def time_us(r):
    return r.s.run('return window.__nuzky.store.getState().timeUs')


@flow('preview', 'Mute silences the preview on this computer only: the picture plays on, the project keeps its sound '
      'and the choice outlives a restart. F or the button shows the preview over the whole screen in more pixels, and '
      'Esc brings the editor back', before=setup)
def preview(r):
    added = r.s.call('window.__nuzky.store.getState().edit(arguments[0])', CAPTION)
    r.check('a caption is on the video', added['ok'] and wait(lambda: len(r.state()['tracks']) == 2, 5), added)

    r.s.run(SPY)
    r.key('m')
    r.check('M mutes the preview: its button shows it and the engine is told',
            pressed(r, 'Unmute') == 'true' and r.s.run('return window.__muted') == [True], r.s.run('return window.__muted'))
    start = time_us(r)
    r.key(' ')
    moved = wait(lambda: time_us(r) > start + 1_000_000, 8)
    r.key(' ')
    r.check('muted, the preview still plays: the playhead moves on with the sound card', moved, [start, time_us(r)])
    sound = r.s.run(SOUND)
    r.check('the project keeps its sound: the track is not muted and the clip is at full volume',
            sound == {'muted': False, 'volume': 1}, sound)
    r.s.run('window.__oldPage = true; location.reload()')
    wait(lambda: r.s.run("const c = window.__nuzky; if (window.__oldPage || !c?.store.getState().snap) return false; "
                         "c.store.setState({view: 'editor'}); return true", retries=3), 30)
    r.check('after a restart the preview is still muted', wait(lambda: pressed(r, 'Unmute') == 'true', 10))
    r.s.run(SPY)
    r.s.run(BUTTON + '.click()', 'Unmute')
    r.check('clicking the button turns the sound back on',
            pressed(r, 'Mute') == 'false' and r.s.run('return window.__muted') == [False], r.s.run('return window.__muted'))

    normal = wait(lambda: (lambda l: l if l['frame'][1] > 100 else None)(layout(r)), 10) or layout(r)
    time.sleep(0.3)  # The button's colour fades for 120 ms.
    r.shot('editor')
    allowed = r.s.call("window.__TAURI_INTERNALS__.invoke('plugin:window|set_fullscreen', {label: 'main', value: false})")
    r.check('the app may take its window to full screen and back', allowed['ok'], allowed)
    r.s.run(STUB)
    r.key('f')
    full = wait(lambda: (lambda l: l if l['frame'][1] > normal['frame'][1] else None)(layout(r)), 10) or layout(r)
    r.check('F shows the preview over the whole window', full['section'] == [0, 0, *full['window']], full)
    r.check('the larger preview is drawn in more pixels, not stretched', full['frame'][1] > normal['frame'][1] * 1.4,
            [normal['frame'], full['frame']])
    r.check('the window is asked to go full screen', r.s.run('return window.__fullscreen') == [True])
    r.shot('full-screen')
    r.key('Escape')
    back = wait(lambda: (lambda l: l if l['section'] == normal['section'] else None)(layout(r)), 5) or layout(r)
    r.check('Esc brings the editor back as it was and the window out of full screen',
            back['section'] == normal['section'] and r.s.run('return window.__fullscreen') == [True, False], back)

    r.s.run(BUTTON + '.click()', 'Full screen')
    r.check('the button opens full screen too', wait(lambda: layout(r)['section'] == [0, 0, *normal['window']], 5), layout(r))
    r.s.run(BUTTON + '.click()', 'Exit full screen')
    r.check('and closes it', wait(lambda: layout(r)['section'] == normal['section'], 5), layout(r))

    r.s.run(SLOW_FULL)
    r.key('f')
    r.key('f')
    time.sleep(2.5)
    r.check('a window already in full screen stays in it, even after F pressed twice quickly',
            r.s.run('return window.__fullscreen') == [] and layout(r)['section'] == normal['section'],
            [r.s.run('return window.__fullscreen'), layout(r)])
    r.s.run(STUB)

    clip = r.track()[0]['id']
    r.focus_clip(clip)
    r.key('f')
    r.check('full screen takes the keyboard from the editor hidden behind it', wait(lambda: r.s.run(FOCUS_IN_PREVIEW), 5))
    r.key('Delete')
    r.s.run("window.__nuzky.store.setState({aiRun: 'Cutting pauses'})")
    run_bar = r.s.run("return document.querySelector('section[aria-label=Preview] [role=status]')?.textContent")
    r.check('an AI run shows what it does and its Stop over the full screen video', 'Stop and edit' in (run_bar or ''), run_bar)
    r.shot('full-screen-ai-run')
    r.s.run("window.__nuzky.store.setState({aiRun: null})")
    r.key('Escape')
    r.check('Delete does not cut the clip selected on the hidden timeline, and Esc gives the keyboard back to that clip',
            wait(lambda: layout(r)['section'] == normal['section'], 5) and [c['id'] for c in r.track()][:1] == [clip]
            and r.s.run('return document.activeElement?.dataset.clipId') == clip, r.track())

    r.s.run("window.__nuzky.openSettings()")
    wait(lambda: r.s.run("return !!document.querySelector('[aria-modal=true]')"), 5)
    r.key('m')
    r.key('f')
    time.sleep(0.3)
    r.check('with a dialog open, M and F leave the preview alone',
            pressed(r, 'Mute') == 'false' and layout(r)['section'] == normal['section'], layout(r))
    r.s.run("window.__nuzky.settings.setState({open: false})")

    size = layout(r)['window']
    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    if r.check('the window gets to 1024 x 640', wait(lambda: layout(r)['window'] == [1024, 640], 5)):
        narrow = wait(lambda: (lambda t: t if t['fits'] else None)(r.s.run(TRANSPORT)), 5) or r.s.run(TRANSPORT)
        time_width = narrow['parts'][0][1] - narrow['parts'][0][0]
        r.check('in the smallest window the transport shows the current time and every control it keeps without overlap',
                narrow['fits'] and time_width > 40, narrow)
        time.sleep(0.3)
        r.shot('small-window')
        webdriver('POST', r.s.path + '/window/rect', {'width': size[0], 'height': size[1]})
    r.check('no error toast', not r.errors(), r.errors())

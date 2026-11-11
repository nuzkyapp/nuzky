"""Follow a person in a wide interview on a vertical canvas. The fixture is the NASA interview at 1920x1080 with her face
a third of the way across until 5 s, crossing to two thirds by 6 s. On a 16:9 canvas the picture just fills the frame,
so Follow says why it can't move it. On 9:16 the clip is fitted small; Choose what to follow shows a box over the
picture, a drag draws one around her head at 2 s, and Enter on Follow follows it: the clip fills the frame and its
keyframes keep her face whole inside the frame and inside what Reels leaves free at every quarter second, the preview
showing what the export renderer draws. Esc leaves choosing without a change, the chip's Undo brings the clip back as it
was in one step, and an agent attached to the open app follows the same head over MCP inside its run. No model is
needed."""
import json, time

from e2e.harness import FIXTURES, Bridge, flow, press, wait
from e2e.home import resize
from e2e.reframe import SAFE, faces, unlike_export
from e2e.snap import POINTER

PROJECT = 'return window.__nuzky.store.getState().snap.project'
REVISION = 'return window.__nuzky.store.getState().snap.revision'
BUTTON = '[data-follow]'
CHIP = 'section[aria-label=Preview] [role=status]'
# Her head at 2 s, in fractions of the picture: left, top, width and height.
HEAD = (0.21, 0.03, 0.25, 0.5)
# Where the picker draws, in window pixels, and the clip's picture in canvas pixels at the playhead.
PICKER = """const b = document.querySelector('[data-testid=follow-picker]').getBoundingClientRect();
return {left: b.left, top: b.top, width: b.width};"""
BOUNDS = 'window.__nuzky.api.layerBounds(window.__nuzky.store.getState().timeUs).then((b) => JSON.stringify(b))'


def chip(r):
    return r.s.run(f'return document.querySelector("{CHIP}")?.innerText ?? ""')


def drag_head(r, clip_id):
    """Draws the box around her head in the picker, from its top-left to its bottom-right corner."""
    found = r.s.call(BOUNDS)
    frame = next(b['frame'] for b in json.loads(found['value']) if b['clipId'] == clip_id)
    picker = r.s.run(PICKER)
    k = picker['width'] / r.s.run(PROJECT)['canvas']['width']

    def at(u, v):
        x = frame[0][0] + u * (frame[1][0] - frame[0][0]) + v * (frame[3][0] - frame[0][0])
        y = frame[0][1] + u * (frame[1][1] - frame[0][1]) + v * (frame[3][1] - frame[0][1])
        return picker['left'] + x * k, picker['top'] + y * k

    (x0, y0), (x1, y1) = at(HEAD[0], HEAD[1]), at(HEAD[0] + HEAD[2], HEAD[1] + HEAD[3])
    r.s.run(POINTER, 'pointerdown', x0, y0)
    r.s.run(POINTER, 'pointermove', (x0 + x1) / 2, (y0 + y1) / 2)
    r.s.run(POINTER, 'pointermove', x1, y1)
    r.s.run(POINTER, 'pointerup', x1, y1)


@flow('follow', 'Follow a head boxed in a wide interview on a vertical canvas: the face stays whole in the frame and the '
      'safe area at every quarter second as the export draws it, Esc and Undo change nothing, a picture with no room says '
      'why, and an agent follows it over MCP')
def follow(r):
    r.import_media(FIXTURES / 'wide-head.mp4')
    r.add_clip('wide-head.mp4')
    r.s.call("window.__nuzky.store.getState().edit({type: 'setCanvas', width: 1920, height: 1080})")
    clip = wait(lambda: (p := r.s.run(PROJECT)) and p['canvas']['width'] == 1920 and p['tracks'][0]['clips'][0], 10)
    r.focus_clip(clip['id'])
    r.seek(2_000_000)
    full = wait(lambda: r.s.run(f'return document.querySelector("{BUTTON}")?.getAttribute("aria-disabled")') == 'true'
                and r.s.run(f'return document.querySelector("{BUTTON}").title'), 10)
    r.check('on a frame the picture just fills, Follow says why it cannot move it', full and 'zoom in' in full, full)

    r.s.call("window.__nuzky.store.getState().edit({type: 'setCanvas', width: 1080, height: 1920})")
    wait(lambda: r.s.run(PROJECT)['canvas']['height'] == 1920, 10)
    r.check('on 9:16 the fitted picture has room, so Follow is offered',
            wait(lambda: r.s.run(f'return document.querySelector("{BUTTON}")?.getAttribute("aria-disabled")') is None, 10))
    time.sleep(1)
    r.shot('follow-before')

    # Esc leaves choosing and changes nothing.
    revision = r.s.run(REVISION)
    r.s.run(f'document.querySelector("{BUTTON}").click()')
    shown = wait(lambda: r.s.run('return !!document.querySelector("[data-testid=follow-picker] [data-follow-box]")'), 10)
    r.check('Choose what to follow shows a box over the picture and the chip says what to do', shown
            and 'Put the box around what to follow' in chip(r), chip(r))
    r.check('the keyboard is on Follow', wait(lambda: r.s.run('return document.activeElement?.textContent.trim()') == 'Follow', 5))
    press('Escape')
    gone = wait(lambda: not r.s.run('return !!document.querySelector("[data-testid=follow-picker]")'), 5)
    r.check('Esc leaves choosing, the clip stays selected and nothing changes', gone and r.s.run(REVISION) == revision
            and r.s.run('return window.__nuzky.store.getState().selection') == [clip['id']], r.s.run(REVISION))

    # Choosing while the video plays pauses it first and stays open.
    r.s.run('window.__nuzky.store.getState().togglePlay()')
    wait(lambda: r.s.run('return window.__nuzky.store.getState().playing'), 5)
    r.s.run(f'document.querySelector("{BUTTON}").click()')
    shown = wait(lambda: r.s.run('return !!document.querySelector("[data-testid=follow-picker] [data-follow-box]")'), 10)
    time.sleep(1)
    r.check('choosing while the video plays pauses it and the box stays', shown
            and r.s.run('return !!document.querySelector("[data-testid=follow-picker] [data-follow-box]")')
            and not r.s.run('return window.__nuzky.store.getState().playing')
            and r.s.run(f'return document.querySelector("{BUTTON}").textContent.trim()') == 'Cancel choosing')
    r.seek(2_000_000)
    time.sleep(1)
    drag_head(r, clip['id'])
    time.sleep(0.5)
    r.shot('follow-picking')
    if r.check('the window goes to its smallest size', resize(r, 1024, 640)):
        time.sleep(1)
        whole = r.s.run(f'const c = document.querySelector("{CHIP} span"); return c.scrollWidth <= c.clientWidth')
        r.check('the chip says it whole in the smallest window', whole)
        r.shot('follow-picking-1024')
        resize(r, 1440, 900)
        time.sleep(1)
    r.s.run('document.querySelector("[data-follow-go]").focus()')
    press('Return')
    running = wait(lambda: next((j for j in r.state()['jobs'] if j['kind'] == 'follow'), None), 10)
    r.check('Enter on Follow starts a job and the preview says so', running and wait(lambda: 'Following' in chip(r), 10), running)
    done = wait(lambda: (j := next((j for j in r.state()['jobs'] if j['kind'] == 'follow'), None)) and j['status'] != 'running' and j, 120)
    r.check('the job finishes', done and done['status'] == 'done', done)

    project = r.s.run(PROJECT)
    followed = project['tracks'][0]['clips'][0]
    r.check('the clip fills the vertical frame', abs(followed['content']['transform']['scale'] - 1920 / 607.5) < 0.01,
            followed['content']['transform'])
    r.check('it moves with keyframes', len(followed['keyframes']) >= 2, followed['keyframes'])
    placed = faces(project)
    outside = [f for f in placed if f[1] < 0 or f[2] > 1080 or not SAFE[0] < (f[1] + f[2]) / 2 < SAFE[1]]
    r.check('at every quarter second her face is whole in the frame and centred inside the safe area', not outside, outside or placed)
    text = wait(lambda: 'Followed it' in (t := chip(r)) and t, 10)
    r.check('the preview says it followed her and offers Undo', text and 'Undo' in text, text)
    saved = wait(lambda: json.loads(r.saved_project().read_text())['tracks'][0]['clips'][0]['keyframes'], 10)
    for seconds in (2, 5.5, 8):
        r.seek(int(seconds * 1_000_000))
        time.sleep(1.5)
        unlike = wait(lambda: (d := unlike_export(r, f'follow-{seconds}s', seconds)) < 0.05 and d, 10) if saved else None
        r.check(f'at {seconds} s the preview shows what the export renderer draws', unlike,
                unlike_export(r, f'follow-{seconds}s', seconds) if saved and not unlike else unlike)

    r.s.run(f"""[...document.querySelectorAll('{CHIP} button')].find((b) => b.textContent.trim() === 'Undo').click()""")
    back = wait(lambda: (c := r.s.run(PROJECT)['tracks'][0]['clips'][0]) and not c['keyframes'] and c, 10)
    r.check('Undo brings the clip back as it was in one step', back and back['content']['transform'] == clip['content']['transform'],
            back and back['content']['transform'])

    # An agent attached to the open app follows the same head inside its run.
    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Follow her'})['run_id']
        job = bridge.call('follow_object', {'run_id': run, 'clip_id': clip['id'], 'at_us': 2_000_000, 'box': list(HEAD)})
        done = wait(lambda: (s := bridge.call('job', {'job_id': job['job_id'], 'action': 'get'}))['status'] != 'running' and s, 120, 0.5)
        shown = wait(lambda: r.s.run(PROJECT)['tracks'][0]['clips'][0]['keyframes'], 10)
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
    finally:
        bridge.close()
    r.check('an agent follows her over MCP and the open app shows it', done and done['status'] == 'done' and shown
            and done['result']['lost_us'] == 0, done)
    agent = faces(r.s.run(PROJECT))
    r.check("the agent's result keeps her face in the safe area too",
            all(0 <= f[1] and f[2] <= 1080 and SAFE[0] < (f[1] + f[2]) / 2 < SAFE[1] for f in agent), agent)
    r.check('nothing went wrong on the way', not r.errors(), r.errors())

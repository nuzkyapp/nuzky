"""Layout beside 9:16 in the Ratio menu on the wide talking head cut at 7 s. Fade shows both clips whole across the
middle over a blurred copy, Stack shows each twice with a silent copy above it, and each is one undo step drawn as the
export renderer draws it. Stack on a selected clip leaves the other alone, Fill reframes to the face and takes the
copies away, and an agent stacks the clips over MCP. The face model is linked from tmp-test, so nothing is
downloaded."""
import json
import time

from e2e.harness import FIXTURES, Bridge, flow, wait
from e2e.reframe import PROJECT, RATIO, reframe_setup, unlike_export

CHIP = 'return document.querySelector("section[aria-label=Preview] [role=status]")?.innerText'


def video_tracks(project):
    return [t for t in project['tracks'] if t['kind'] == 'video']


def copies(project):
    """(clip id it copies, its y, its volume) of each clip over the main track."""
    main = project['tracks'][0]['clips']
    out = []
    for track in video_tracks(project)[1:]:
        for c in track['clips']:
            of = next((m['id'] for m in main if (m['startUs'], m['durationUs'], m['content']['sourceInUs'])
                       == (c['startUs'], c['durationUs'], c['content']['sourceInUs'])), None)
            out.append((of, c['content']['transform']['y'], c['content']['volume']))
    return out


def choose(r, style):
    """Opens Ratio, opens Layout beside 9:16 and clicks `style`."""
    r.s.run(f'document.querySelector("{RATIO}").click()')
    wait(lambda: r.s.run('return !!document.querySelector("[role=menu]")'), 5)
    r.s.run("""[...document.querySelectorAll('[role=menu] [role=menuitem]')]
        .find((b) => b.textContent.trim() === 'Layout' && b.closest('div.group').textContent.startsWith('9:16')).click()""")
    tiles = wait(lambda: r.s.run("""return [...document.querySelectorAll('[data-layouts] [role=menuitem]')]
        .map((b) => b.textContent.trim())"""), 5)
    r.s.run("""[...document.querySelectorAll('[data-layouts] [role=menuitem]')]
        .find((b) => b.textContent.trim() === arguments[0]).click()""", style)
    return tiles


def settled(r, check):
    """The project once `check` holds for it and it is saved."""
    project = wait(lambda: (p := r.s.run(PROJECT)) and check(p) and p, 10)
    saved = project and wait(lambda: check(json.loads(r.saved_project().read_text())), 10)
    return project if saved else None


@flow('reel-layout', 'Layout beside 9:16 in the Ratio menu: Fade shows the wide video whole over a blurred copy and Stack '
      'twice, each one undo step drawn as the export draws it; Stack on the selected clip only, Fill reframes and takes '
      'the copies away, and an agent stacks over MCP',
      before=reframe_setup)
def reel_layout(r):
    r.import_media(FIXTURES / 'wide-head.mp4')
    r.add_clip('wide-head.mp4')
    r.s.call("window.__nuzky.store.getState().edit({type: 'setCanvas', width: 1920, height: 1080})")
    clip = r.s.run(PROJECT)['tracks'][0]['clips'][0]
    r.s.call("window.__nuzky.store.getState().edit({type: 'splitClip', clipId: arguments[0], atUs: 7000000})", clip['id'])
    wide = wait(lambda: (p := r.s.run(PROJECT)) and len(p['tracks'][0]['clips']) == 2 and p['canvas']['width'] == 1920 and p, 10)
    r.check('a 16:9 project with the wide talking head cut at 7 s', wide, wide and wide['canvas'])
    r.s.run('window.__nuzky.store.getState().select([])')
    r.seek(2_000_000)
    time.sleep(1)

    tiles = choose(r, 'Fade')
    r.check('Layout beside 9:16 offers Fill, Fade and Stack', tiles == ['Fill', 'Fade', 'Stack'], tiles)
    fade = settled(r, lambda p: p['canvas']['height'] == 1920)
    clips = fade['tracks'][0]['clips'] if fade else []
    r.check('Fade makes the canvas 9:16 with its blur on and fits both clips whole in the middle',
            fade and fade['canvas']['backgroundBlur'] == 0.5 and len(clips) == 2 and len(video_tracks(fade)) == 1
            and all((c['content']['transform']['scale'], c['content']['transform']['x'], c['content']['transform']['y'])
                    == (1, 0, 0) and not c['keyframes'] for c in clips),
            fade and (fade['canvas'], [c['content']['transform'] for c in clips]))
    chip = wait(lambda: (t := r.s.run(CHIP)) and 'Layout: Fade' in t and t, 10)
    r.check('the preview says Layout: Fade with Undo', chip and 'Undo' in chip, chip)
    unlike = wait(lambda: (d := unlike_export(r, 'layout-fade', 2)) < 0.05 and d, 10)
    r.check('the preview shows Fade as the export renderer draws it', unlike, unlike or unlike_export(r, 'layout-fade', 2))

    choose(r, 'Stack')
    stack = settled(r, lambda p: len(video_tracks(p)) == 2)
    made = copies(stack) if stack else []
    main = stack['tracks'][0]['clips'] if stack else []
    r.check('Stack puts each clip over the top half and a silent copy of it over the bottom half',
            stack and len(made) == 2 and {m[0] for m in made} == {c['id'] for c in main}
            and all(abs(y - 0.25) < 1e-4 and volume == 0 for _, y, volume in made)
            and all(abs(c['content']['transform']['y'] + 0.25) < 1e-4 for c in main), made)
    chip = wait(lambda: (t := r.s.run(CHIP)) and 'Layout: Stack' in t and t, 10)
    r.check('the preview says Layout: Stack with Undo', chip and 'Undo' in chip, chip)
    unlike = wait(lambda: (d := unlike_export(r, 'layout-stack', 2)) < 0.05 and d, 10)
    r.check('the preview shows Stack as the export renderer draws it', unlike, unlike or unlike_export(r, 'layout-stack', 2))
    r.s.run("""[...document.querySelectorAll('section[aria-label=Preview] [role=status] button')]
        .find((b) => b.textContent.trim() === 'Undo').click()""")
    back = wait(lambda: (p := r.s.run(PROJECT)) and len(video_tracks(p)) == 1 and p, 10)
    r.check('Undo takes Stack back to Fade in one step', back and back['tracks'] == fade['tracks'] and back['canvas'] == fade['canvas'],
            back and copies(back))

    second = back['tracks'][0]['clips'][1]['id']
    r.s.run('window.__nuzky.store.getState().select([arguments[0]])', second)
    choose(r, 'Stack')
    one = settled(r, lambda p: len(video_tracks(p)) == 2)
    r.check('Stack with the second clip selected copies only that clip', one and [m[0] for m in copies(one)] == [second],
            one and copies(one))
    r.shot('layout-stack-selected')

    r.s.run('window.__nuzky.store.getState().select([])')
    choose(r, 'Fill')
    done = wait(lambda: (j := next((j for j in r.state()['jobs'] if j['kind'] == 'reframe'), None)) and j['status'] != 'running' and j, 120)
    r.check('Fill follows the face as Reframe does', done and done['status'] == 'done', done)
    filled = settled(r, lambda p: len(video_tracks(p)) == 1)
    r.check('Fill takes the copy away and both clips fill the frame', filled and all(
        abs(c['content']['transform']['scale'] - 1920 / 607.5) < 0.01 for c in filled['tracks'][0]['clips']),
        filled and [c['content']['transform'] for c in filled['tracks'][0]['clips']])
    r.shot('layout-fill')

    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Stack for Reels'})['run_id']
        job = bridge.call('reframe', {'run_id': run, 'format': '9:16', 'layout': 'stack'})
        status = bridge.call('job', {'job_id': job['job_id'], 'action': 'wait'})
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
    finally:
        bridge.close()
    agent = wait(lambda: (p := r.s.run(PROJECT)) and len(copies(p)) == 2 and p, 10)
    r.check('an agent stacks both clips over MCP and the open app shows it', status['status'] == 'done' and agent, status)
    time.sleep(1)
    r.shot('layout-agent-stack')
    r.check('nothing went wrong on the way', not r.errors(), r.errors())

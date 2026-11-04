"""B-roll from screen recordings. In a talking head that says "...and add captions," the Transcript tab's B-roll suggests
the silent video named after it for just that sentence, Add puts it over the whole frame with the voice and the
captions as they were, one undo takes it back, and an agent gets the same over MCP with the reason. A recording named
after nothing said, or none at all, gives no suggestion and says why."""
import subprocess, time

from PIL import ImageStat

from e2e.harness import FIXTURES, Bridge, flow, link_models, preview_crop, preview_rect, wait, webdriver

PROJECT = 'return window.__nuzky.store.getState().snap.project'
BAR = "document.querySelector('[data-broll-bar]')"



def small_window(r, name):
    """A screenshot in the smallest window, 1024 × 640, then back to 1440 × 900."""
    for width, height, shot in ((1024, 640, name), (1440, 900, None)):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        if shot:
            time.sleep(0.5)
            r.shot(shot)


def silent(path, color, seconds):
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', f'color=c={color}:s=1920x1080:r=30:d={seconds}',
                    '-c:v', 'libx264', '-preset', 'veryfast', '-pix_fmt', 'yuv420p', '-an', str(path)], check=True)


def setup(r):
    link_models(r)
    silent(r.work / 'Captions app.mov', 'red', 8)
    silent(r.work / 'Spreadsheet.mov', 'blue', 4)


def poll(bridge, job, timeout):
    end = time.time() + timeout
    while True:
        state = bridge.call('job', {'job_id': job['job_id'], 'action': 'get'})
        if state['status'] != 'running' or time.time() > end:
            return state
        time.sleep(1)


def project(r):
    return r.s.run(PROJECT, retries=3)


def placed(r, asset_id):
    return [(i, c) for i, t in enumerate(project(r)['tracks']) if i > 0 for c in t['clips'] if c['content'].get('assetId') == asset_id]


def suggest(r):
    toasts = len(r.state()['toasts'])
    r.s.run("document.querySelector('[data-suggest-broll]').click()")
    return wait(lambda: r.s.run(f'return {BAR}?.textContent ?? null') or (r.state()['toasts'][toasts:] or None), 15)


@flow('broll', 'In the Transcript tab B-roll suggests the screen recording named after what is said for just that sentence, '
      'Add puts it over the whole frame as one undo step with voice and captions kept, an agent finds it over MCP with '
      'the reason, and a recording nobody names, or none, gives no suggestion', before=setup, reuse_transcripts=True)
def broll(r):
    r.import_media(FIXTURES / 'talk.mp4', r.work / 'Spreadsheet.mov')
    r.add_clip('talk.mp4')
    bridge = Bridge(r, r.saved_project())
    try:
        job = poll(bridge, bridge.call('transcribe', {'language': 'en', 'model': 'small'}), 600)
        if not r.check('the speech is recognised', job and job['status'] == 'done', job):
            return
        run = bridge.call('begin_run', {'label': 'Captions'})['run_id']
        bridge.call('build_captions', {'run_id': run})
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
        transcript = bridge.call('get_transcript', {})
        with_broll(r, bridge, transcript)
    finally:
        bridge.close()


def with_broll(r, bridge, transcript):
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === 'Transcript').click()")
    r.check('the Transcript tab offers B-roll', wait(lambda: r.s.run(
        "const b = document.querySelector('[data-suggest-broll]'); return !!b && b.getAttribute('aria-disabled') !== 'true'"), 30))
    said = suggest(r)
    r.check('a recording named after nothing said gives no suggestion and says so', isinstance(said, list)
            and 'No recording here is named after an app or tool you mention' in said[-1]['text'], said)

    r.import_media(r.work / 'Captions app.mov')
    red = next(a['id'] for a in project(r)['assets'] if a['name'] == 'Captions app.mov')
    before = project(r)
    shown = suggest(r)
    items = r.s.run("return [...document.querySelectorAll('[data-broll-item]')].map((i) => i.textContent)")
    r.check('B-roll suggests the recording for the one sentence that names it', isinstance(shown, str) and items and len(items) == 1
            and 'captions' in items[0].lower() and 'Captions app.mov' in items[0], items or shown)
    sentence = next(s for s in transcript['sentences'] if 'caption' in s['text'].lower())
    start, end = sentence['start_us'], sentence['end_us']
    r.check('focus is on its Add', r.s.run("return document.activeElement?.hasAttribute('data-broll-add') ?? false"))
    time.sleep(0.3)
    r.shot('suggested')
    small_window(r, 'suggested-1024')

    r.s.run("document.querySelector('[data-broll-add]').click()")
    found = wait(lambda: placed(r, red) or None, 10)
    clip = found and found[0][1]
    r.check('Add puts it on a track above the main one, from the first word of the sentence to its last',
            clip and found[0][0] >= 1 and (clip['startUs'], clip['durationUs']) == (start, end - start), [clip, start, end])
    after = project(r)
    r.check('the talking head, its voice and the captions stay as they were',
            after['tracks'][0] == before['tracks'][0] and [t for t in after['tracks'] if t['kind'] == 'text'] == [t for t in before['tracks'] if t['kind'] == 'text'],
            [t['name'] for t in after['tracks']])
    r.check('the recording itself is silent, so only the voice is heard', next(a for a in after['assets'] if a['id'] == red)['hasAudio'] is False)
    r.check('the suggestions close once each is added', wait(lambda: not r.s.run(f'return !!{BAR}'), 5))
    toast = r.s.run("return window.__nuzky.store.getState().toasts.filter((t) => t.text.startsWith('Added Captions app.mov')).map((t) => t.action?.label)")
    r.check('a toast says what was added and offers Undo', toast == ['Undo'], r.state()['toasts'])

    r.s.run('window.__nuzky.store.getState().select([])')
    r.seek((start + end) // 2)
    time.sleep(1.5)
    r.shot('broll-added')
    middle = preview_crop(r.work / 'broll-added.png', preview_rect(r))
    w, h = middle.size
    colour = ImageStat.Stat(middle.crop((w // 2 - 10, h // 2 - 10, w // 2 + 10, h // 2 + 10))).mean
    r.check('the preview shows the recording over the video while the sentence is said', colour[0] > 170 and colour[1] < 90 and colour[2] < 90,
            [round(c) for c in colour])

    r.key('z', ctrlKey=True)
    r.check('one undo takes it back and leaves the rest as before', wait(lambda: project(r)['tracks'] == before['tracks'], 5))

    found = bridge.call('find_broll', {})
    suggestion = found['suggestions'][0] if len(found['suggestions']) == 1 else None
    r.check('an agent finds the same recording for the same words, with the reason', suggestion and suggestion['asset_id'] == red
            and (suggestion['start_us'], suggestion['end_us']) == (start, end) and 'captions' in suggestion['reason'].lower()
            and suggestion['edit'] == {'type': 'addOverlay', 'assetId': red, 'startUs': start, 'durationUs': end - start}, found)
    key = found['transcript_key']
    nothing = bridge.call('find_broll', {'words': [0, 3], 'transcript_key': key})
    r.check('words that name no recording get none', nothing['suggestions'] == [] and len(nothing['recordings']) == 2, nothing)
    run = bridge.call('begin_run', {'label': 'B-roll'})['run_id']
    bridge.call('apply_edits', {'run_id': run, 'request_id': 'broll', 'edits': [suggestion['edit']]})
    bridge.call('end_run', {'run_id': run, 'action': 'keep'})
    agent = wait(lambda: placed(r, red) or None, 10)
    r.check('its edit places the recording where Add does', agent and (agent[0][1]['startUs'], agent[0][1]['durationUs']) == (start, end - start), agent)
    again = bridge.call('find_broll', {})
    r.check('a sentence the recording already covers is not suggested again', again['suggestions'] == [], again)
    bridge.call('undo_run', {'run_id': run})
    r.check('undoing the agent\'s run takes it back', wait(lambda: project(r)['tracks'] == before['tracks'], 5))

    # No silent video at all: B-roll says how to get one.
    for asset in [a for a in project(r)['assets'] if not a['hasAudio']]:
        r.s.call("window.__nuzky.store.getState().edit({type: 'removeAsset', assetId: arguments[0]})", asset['id'])
    wait(lambda: all(a['hasAudio'] for a in project(r)['assets']), 5)
    said = suggest(r)
    r.check('without a recording it says to record the screen first', isinstance(said, list)
            and 'Record your screen in Media' in said[-1]['text'], said)
    r.check('no error toast', not r.errors(), r.errors())

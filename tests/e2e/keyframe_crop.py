"""A b-roll over a phone clip that grows and crops itself between two keyframes, as issue #450 reported. The first
keyframe comes from the inspector's diamond and the second from typing a crop and a scale at a later playhead. With the
clip selected, the playhead between the keyframes and on the second one shows the values in between in the inspector,
and the window keeps drawing: the Transform section once re-rendered forever there and the window broke."""
import subprocess, time

from e2e.harness import flow, press, unfold, wait
from e2e.proxy import phone_clip

SECTION = "aside[aria-label=Inspector]"
AT_US = 1_000_000
SECOND_US = 3_000_000
BETWEEN_US = 2_000_000
BROKEN = "return [...document.querySelectorAll('[role=alertdialog]')].some((d) => d.textContent.includes('Something went wrong'))"


def broll(path):
    """A portrait HEVC screen recording at a jittered 42 fps, as file 3 of the report."""
    times = "settb=1/600,setpts='(N*14+eq(mod(N\\,3)\\,1)*5)/600/TB'"
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=s=1170x2532:r=42:d=4', '-vf',
                    f'{times},format=yuvj420p', '-fps_mode', 'vfr', '-enc_time_base', '1/600', '-c:v', 'libx265', '-preset',
                    'ultrafast', '-x265-params', 'log-level=error', '-tag:v', 'hvc1', '-video_track_timescale', '600', str(path)],
                   check=True)


def keyframes(r, clip_id):
    clips = (c for t in r.s.run('return window.__nuzky.store.getState().snap.project', retries=3)['tracks'] for c in t['clips'])
    return next(c for c in clips if c['id'] == clip_id)['keyframes']


def type_into(r, label, text):
    field = f"document.querySelector('{SECTION} input[aria-label=\"{label}\"]')"
    if not wait(lambda: r.s.run(f'const f = {field}; if (!f) return false; f.focus(); f.select(); return true'), 5):
        return False
    time.sleep(0.3)
    for key in text:
        press(key)
    press('Return')
    return True


def scale_shown(r):
    value = r.s.run(f"return document.querySelector('{SECTION} input[aria-label=\"Scale value\"]')?.value")
    return float(value) if value not in (None, '') else None


def ui_errors(r):
    log = r.work / 'data/nuzky/logs/nuzky.log'
    return [line for line in log.read_text().splitlines() if 'ERROR ui' in line] if log.exists() else []


@flow('keyframe-crop', 'A b-roll over a phone clip that scales and crops between two keyframes stays selected while the '
      'playhead moves between them and onto the second, the inspector shows the values in between, and the window keeps drawing')
def keyframe_crop(r):
    main, overlay = r.work / 'IMG_0001.MOV', r.work / 'IMG_0002.MOV'
    phone_clip(main, 6)
    broll(overlay)
    r.import_media(main, overlay)
    r.add_clip(main.name)
    r.seek(AT_US)
    put = f"document.querySelector('[aria-label=\"Put {overlay.name} over the video\"]')"
    if not r.check('the b-roll tile offers Overlay', wait(lambda: r.s.run(f'return !!{put}'), 10)):
        return
    r.s.run(f'{put}.click()')
    clip = wait(lambda: next((c for t in r.s.run('return window.__nuzky.store.getState().snap.project')['tracks'][1:] for c in t['clips']), None), 10)
    if not r.check('the b-roll lands selected on a track above the main one',
                   clip and r.s.run('return window.__nuzky.store.getState().selection') == [clip['id']], clip):
        return

    unfold(r, 'Transform')
    add = f"{SECTION} button[aria-label='Add keyframe at playhead']"
    wait(lambda: r.s.run(f'return !!document.querySelector("{add}")'), 5)
    r.s.run(f'document.querySelector("{add}").click()')
    r.check('the diamond adds a keyframe at the clip\'s start', wait(lambda: [k['tUs'] for k in keyframes(r, clip['id'])] == [0], 5),
            keyframes(r, clip['id']))
    r.seek(SECOND_US)
    unfold(r, 'Crop and shape')
    type_into(r, 'Crop left', '20')
    wait(lambda: len(keyframes(r, clip['id'])) == 2, 5)
    r.s.run(f"""const i = document.querySelector('{SECTION} input[aria-label="Scale value"]'); i.focus();
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(i, '180');
        i.dispatchEvent(new Event('input', {{bubbles: true}})); i.blur();""")
    second = lambda: (ks := keyframes(r, clip['id'])) and len(ks) == 2 and ks[1]['transform']
    typed = wait(lambda: (t := second()) and abs(t['scale'] - 1.8) < 1e-6 and abs((t.get('crop') or {}).get('left', 0) - 0.2) < 1e-6 and t, 5)
    if not r.check('typing a crop and a scale at a later playhead writes a second keyframe with both', typed, second()):
        r.shot('second-keyframe')
        return

    r.seek(BETWEEN_US)
    time.sleep(1)
    between, first = scale_shown(r), round(keyframes(r, clip['id'])[0]['transform']['scale'] * 100)
    r.check('between the keyframes the inspector shows a scale between theirs', between and first + 1 < between < 179, (first, between))
    r.shot('between-keyframes')
    r.check('the window keeps drawing with the playhead between the keyframes', not r.s.run(BROKEN))
    for _ in range(5):
        r.seek(AT_US + 500_000)
        time.sleep(0.3)
        r.seek(SECOND_US)
        time.sleep(0.3)
    r.check('moving the playhead back and onto the second keyframe keeps the window drawing', not r.s.run(BROKEN))
    r.check('on the second keyframe the inspector shows its scale', scale_shown(r) == 180, scale_shown(r))
    r.check('the interface logged no error', not ui_errors(r), ui_errors(r)[:1])

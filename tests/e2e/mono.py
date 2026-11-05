"""Mono on a phone video whose microphone recorded on the left only: once its sound is prepared, the Audio tab says
it plays in one ear, Play it on both sides turns Mono on from the left by keyboard, the project file keeps it and the
export carries the voice in both channels, then From and Ctrl+Z change it back."""
import json, subprocess, time

import numpy as np

from e2e.harness import export, flow, press, wait, webdriver

RATE = 48_000
CONTENT = "return window.__nuzky.store.getState().snap.project.tracks[0].clips[0].content"
INSPECTOR = "document.querySelector('aside[aria-label=Inspector]')"
CHECKBOX = (f"[...{INSPECTOR}.querySelectorAll('label')].find((l) => l.textContent === 'Mono')?.querySelector('input')")
BUTTON = f"[...{INSPECTOR}.querySelectorAll('button')].find((b) => b.textContent.trim() === arguments[0])"
NOTICE = f"return [...{INSPECTOR}.querySelectorAll('[role=status]')].map((e) => e.textContent).find((t) => t.includes('side has sound')) ?? null"
FROM = f"return {INSPECTOR}.querySelector('[role=group][aria-label=\"Mono from\"] [aria-pressed=true]')?.textContent ?? null"


def channels(path):
    """RMS of the left and the right channel of a file's sound, in dBFS."""
    raw = subprocess.run(['ffmpeg', '-v', 'error', '-i', str(path), '-vn', '-ac', '2', '-ar', str(RATE), '-f', 'f32le', '-'],
                         capture_output=True, check=True).stdout
    stereo = np.frombuffer(raw, dtype='<f4').astype(np.float64).reshape(-1, 2)
    return [10 * np.log10(np.mean(stereo[:, ch] ** 2) + 1e-20) for ch in (0, 1)]


@flow('mono', 'Mono plays a voice recorded on the left only in both ears: the inspector finds the side, Play it on both sides fixes it by keyboard, and the export has the voice in both channels')
def mono(r):
    source = r.work / 'lav.mp4'
    voice = '0.2*sin(2*PI*180*t)*(0.6+0.4*sin(2*PI*3*t))'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=s=1080x1920:r=30:d=4',
                    '-f', 'lavfi', '-i', f'aevalsrc={voice}|0:s={RATE}:d=4', '-c:v', 'libx264', '-preset', 'ultrafast',
                    '-pix_fmt', 'yuv420p', '-c:a', 'aac', '-b:a', '128k', str(source)], check=True)
    r.import_media(source)
    r.add_clip('lav.mp4')
    clip = r.track()[0]
    content = lambda: r.s.run(CONTENT)
    off = export(r, 'off.mp4')
    left, right = channels(off)
    r.check('exported as recorded, the voice is on the left only', left > -30 and right < left - 40, f'{left:.1f} / {right:.1f} dBFS')

    r.focus_clip(clip['id'])
    tab = f"{INSPECTOR}.querySelector('[role=tab][aria-selected=true]')"
    if not wait(lambda: r.s.run(f'return !!{tab}'), 10):
        raise RuntimeError('the inspector shows no tabs for the clip')
    r.s.run(f'{tab}.focus()')
    r.key('End')
    r.check('End in the tab bar opens the Audio tab', wait(lambda: r.s.run(f'return {tab}?.textContent') == 'Audio', 5),
            r.s.run(f'return {tab}?.textContent'))
    r.check('Mono is there, off', wait(lambda: r.s.run(f'return {CHECKBOX}?.checked ?? null') is False, 5),
            r.s.run(f'return {CHECKBOX}?.checked ?? null'))
    notice = wait(lambda: r.s.run(NOTICE), 60) or r.s.run(NOTICE)
    r.check('once the sound is prepared, a line says only the left side has sound',
            notice == 'Only the left side has sound, so it plays in one ear.', notice)
    r.check('From stays hidden while Mono is off', r.s.run(FROM) is None, r.s.run(FROM))
    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        time.sleep(1)
        r.shot(f'inspector-one-side{suffix}')

    r.s.run(f'{BUTTON}.focus()', 'Play it on both sides')
    press('Return')
    r.check('Enter on Play it on both sides turns Mono on from the left', wait(lambda: content().get('mono') == 'left', 5), content())
    r.check('Mono is checked and From marks Left', wait(lambda: r.s.run(f'return {CHECKBOX}?.checked') and r.s.run(FROM) == 'Left', 5),
            r.s.run(FROM))
    r.check('the line about one side goes away', wait(lambda: r.s.run(NOTICE) is None, 5), r.s.run(NOTICE))
    r.shot('inspector-mono-left')
    saved = lambda: json.loads(r.saved_project().read_text())['tracks'][0]['clips'][0]['content'].get('mono')
    r.check('the project file on disk plays the left side in both ears', wait(lambda: saved() == 'left', 10), saved())

    both = export(r, 'mono.mp4')
    left, right = channels(both)
    r.check('the export has the voice in both channels at the level it had on the left',
            abs(left - right) < 0.5 and abs(left - channels(off)[0]) < 0.5, f'{left:.1f} / {right:.1f} dBFS')

    r.s.run(f'{BUTTON}.click()', 'Both')
    r.check('From Both mixes the sides into one', wait(lambda: content().get('mono') == 'both', 5), content())
    r.focus_clip(clip['id'])
    r.key('z', ctrlKey=True)
    r.check('Ctrl+Z takes it back to Left', wait(lambda: content().get('mono') == 'left', 5), content())
    r.key('z', ctrlKey=True)
    r.check('a second Ctrl+Z turns Mono off', wait(lambda: content().get('mono', 'off') == 'off', 5), content())
    r.check('and the line about one side comes back', wait(lambda: r.s.run(NOTICE) is not None, 5), r.s.run(NOTICE))
    r.check('no error toast', not r.errors(), r.errors())

"""Play backwards and a speed ramp from the inspector's Speed tab, on 6 s of a white bar moving 80 px a second with a
tone, so where the bar is tells which moment of the file shows. Backwards, the preview at 1.5 s shows the file at 4.5 s
and the export does too; the Slow-mo ramp keeps the 6 s of the file and
plays them in 10.9 s, its points move by keyboard and by pointer, each change undoes on its own, the preview and the
export show the same frame on the ramp, and Steady takes the ramp off at its average speed."""
import json, subprocess, tempfile, time
from pathlib import Path

from e2e.harness import export, ffprobe, flow, press, preview_crop, preview_rect, wait
from e2e.home import resize

CLIP = "return window.__nuzky.store.getState().snap.project.tracks[0].clips[0]"
TAB = "[...document.querySelectorAll('aside[aria-label=Inspector] [role=tab]')].find((t) => t.textContent === 'Speed')"
BACKWARDS = ("[...document.querySelectorAll('aside[aria-label=Inspector] label')]"
             ".find((l) => l.textContent === 'Play backwards')?.querySelector('input')")
SEGMENT = "document.querySelector(`aside[aria-label=Inspector] [role=group][aria-label='Speed mode'] [data-seg='${arguments[0]}']`)"
CURVE = "document.querySelector('[data-ramp-curve]')"
SLOW_MO = [{'at': 0, 'speed': 1}, {'at': 0.4, 'speed': 0.25}, {'at': 0.6, 'speed': 0.25}, {'at': 1, 'speed': 1}]


def bar_video(path):
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i',
                    "nullsrc=s=540x960:r=30:d=6,format=gray,geq=lum='if(between(X,T*80,T*80+24),235,40)'",
                    '-f', 'lavfi', '-i', 'sine=frequency=440:duration=6', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-g', '60',
                    '-c:a', 'aac', '-shortest', str(path)], check=True)


def bar_at(image):
    """The second of the file a picture of the bar video shows: the bar's left edge, 80 px a second at 540 px wide."""
    image = image.convert('L')
    image = image.resize((540, round(540 * image.height / image.width)))
    rows = range(0, image.height, 16)
    lit = [sum(image.getpixel((x, y)) for y in rows) / len(rows) > 150 for x in range(image.width)]
    # The bar is 24 px wide; the preview's selection outline along the picture's edges is a line of a pixel or two.
    for x in range(image.width - 12):
        if all(lit[x:x + 12]):
            return round(x / 80, 3)
    return None


def file_frame(path, seconds):
    """The frame FFmpeg shows at `seconds` of `path`."""
    from PIL import Image
    with tempfile.TemporaryDirectory() as folder:
        out = Path(folder) / 'frame.png'
        subprocess.run(['ffmpeg', '-v', 'error', '-ss', f'{seconds:.3f}', '-i', str(path), '-frames:v', '1', str(out)], check=True)
        return Image.open(out).convert('RGB').copy()


def picture(r, us, name):
    """The preview at timeline time `us`, once the frame there is drawn."""
    r.seek(us)
    time.sleep(1.5)
    r.shot(name)
    return preview_crop(r.work / f'{name}.png', preview_rect(r))


def ramp(r):
    return [{'at': round(p['at'], 3), 'speed': round(p['speed'], 2)} for p in r.s.run(CLIP)['content'].get('ramp', [])]


@flow('retime', 'Play backwards and a speed ramp from the Speed tab: the preview and the export play the file backwards and '
      'on the ramp, the ramp keeps the clip\'s part of the file, its points move by keyboard and pointer and undo one by one')
def retime(r):
    source = r.work / 'bar.mp4'
    bar_video(source)
    r.import_media(source)
    r.add_clip('bar.mp4')
    clip = r.track()[0]
    r.focus_clip(clip['id'])
    if not wait(lambda: r.s.run(f'return !!{TAB}'), 10):
        raise RuntimeError('the inspector has no Speed tab for the clip')
    r.s.run(f'{TAB}.click()')
    forwards = bar_at(picture(r, 1_500_000, 'forwards-1.5'))
    r.check('forwards, the preview at 1.5 s shows the file at 1.5 s', forwards is not None and abs(forwards - 1.5) < 0.05, forwards)

    # Backwards, by keyboard: the clip keeps its length and the preview at 1.5 s shows the file at 4.5 s.
    if not wait(lambda: r.s.run(f'return !!{BACKWARDS}'), 5):
        raise RuntimeError('the Speed tab has no Play backwards')
    r.s.run(f'{BACKWARDS}.focus()')
    press('space')
    r.check('Space plays the clip backwards and it keeps its length',
            wait(lambda: r.s.run(CLIP)['content'].get('reverse') is True, 5) and r.s.run(CLIP)['durationUs'] == clip['durationUs'],
            r.s.run(CLIP))
    r.check('the clip on the timeline says it plays backwards',
            wait(lambda: r.s.run(f"return !!document.querySelector(`[data-clip-id='{clip['id']}'] [aria-label='Plays backwards']`)"), 5))
    # Backwards each moment shows the frame just before it in the file: at 1.5 s the one from 4.467 s.
    backwards = bar_at(picture(r, 1_500_000, 'backwards-1.5'))
    r.check('backwards, the preview at 1.5 s shows the file at 4.5 s', backwards is not None and abs(backwards - 4.5) < 0.05, backwards)
    out = export(r, 'backwards.mp4')
    length, codecs = ffprobe(out)
    exported = bar_at(file_frame(out, 1.5))
    r.check('the export is as long, with sound, and at 1.5 s shows the file at 4.5 s',
            abs(length - 6) < 0.1 and 'aac' in codecs and exported is not None and abs(exported - 4.5) < 0.05,
            {'length': length, 'codecs': sorted(codecs), 'shows': exported})
    # The preview is about 300 px wide on screen, so its bar is read to a couple of pixels of 540: within a frame.
    r.check('the preview and the export show the same frame backwards',
            None not in (backwards, exported) and abs(backwards - exported) < 0.035, {'preview': backwards, 'export': exported})

    # Ramp: Slow-mo keeps the 6 s of the file and plays them at an average of 0.55x.
    r.s.run(f'{SEGMENT}.click()', 'ramp')
    r.check('Ramp starts with Slow-mo and keeps the clip\'s part of the file',
            wait(lambda: ramp(r) == SLOW_MO, 5) and abs(r.s.run(CLIP)['durationUs'] - 6_000_000 / 0.55) < 40_000,
            r.s.run(CLIP))
    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        resize(r, width, height)
        time.sleep(1)
        r.shot(f'inspector-ramp{suffix}')

    # The curve by keyboard: the chosen point (the second) slows by Down, Delete takes back a point Enter added.
    r.s.run(f'{CURVE}.focus()')
    press('Down')
    r.check('Down slows the chosen point of the curve', wait(lambda: ramp(r)[1]['speed'] == 0.24, 5), ramp(r))
    press('Return')
    r.check('Enter adds a point after it', wait(lambda: len(ramp(r)) == 5, 5), ramp(r))
    press('Delete')
    r.check('Delete removes the chosen point', wait(lambda: len(ramp(r)) == 4, 5), ramp(r))

    # By pointer: the third point dragged up speeds it up; a double-click adds a point where it lands.
    r.s.run("""const point = document.querySelector('[data-ramp-point="2"]').getBoundingClientRect();
        const at = {clientX: point.left + point.width / 2, clientY: point.top + point.height / 2, bubbles: true, button: 0, pointerId: 1};
        document.querySelector('[data-ramp-point="2"]').dispatchEvent(new PointerEvent('pointerdown', at));
        window.dispatchEvent(new PointerEvent('pointermove', {...at, clientY: at.clientY - 24}));
        window.dispatchEvent(new PointerEvent('pointerup', {...at, clientY: at.clientY - 24}));""")
    r.check('dragging a point up speeds the clip up there', wait(lambda: ramp(r)[2]['speed'] > 0.4, 5), ramp(r))
    dragged = ramp(r)
    r.s.run("""const plot = document.querySelector('[data-ramp-plot]'); const box = plot.getBoundingClientRect();
        plot.dispatchEvent(new MouseEvent('dblclick', {clientX: box.left + box.width * 0.8, clientY: box.top + box.height / 2, bubbles: true}));""")
    r.check('a double-click on the curve adds a point there',
            wait(lambda: len(ramp(r)) == 5 and abs(ramp(r)[3]['at'] - 0.8) < 0.02, 5), ramp(r))
    r.key('z', ctrlKey=True)
    r.check('Ctrl+Z takes back only the added point', wait(lambda: ramp(r) == dragged, 5), ramp(r))
    r.check('the saved project keeps the ramp and plays backwards',
            wait(lambda: (lambda c: c.get('reverse') is True and len(c.get('ramp', [])) == 4)(
                json.loads(r.saved_project().read_text())['tracks'][0]['clips'][0]['content']), 10))

    # The preview and the export show the same frame on the ramp, and the export is the clip's new length.
    duration = r.s.run(CLIP)['durationUs'] / 1e6
    on_ramp = [bar_at(picture(r, us, f'ramp-{us / 1e6:.1f}')) for us in (3_000_000, 6_000_000)]
    out = export(r, 'ramp.mp4')
    length, codecs = ffprobe(out)
    exported = [bar_at(file_frame(out, s)) for s in (3.0, 6.0)]
    r.check('the ramp\'s export is the clip\'s length, with sound', abs(length - duration) < 0.1 and 'aac' in codecs,
            {'export': length, 'clip': duration, 'codecs': sorted(codecs)})
    # One frame of the bar is 2.7 px, 0.033 s of the file.
    r.check('the preview and the export show the same frames on the ramp',
            None not in on_ramp + exported and all(abs(a - b) < 0.035 for a, b in zip(on_ramp, exported)),
            {'preview': on_ramp, 'export': exported})

    # Steady takes the ramp off; the clip keeps its length at the ramp's average.
    before = r.s.run(CLIP)['durationUs']
    r.s.run(f'{SEGMENT}.click()', 'steady')
    r.check('Steady takes the ramp off and keeps the length',
            wait(lambda: ramp(r) == [] and r.s.run(CLIP)['durationUs'] == before, 5), r.s.run(CLIP))
    time.sleep(1)
    r.shot('inspector-steady-backwards')
    r.check('no error toast', not r.errors(), r.errors())

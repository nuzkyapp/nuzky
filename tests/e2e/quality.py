"""The quality chip before Export in the top bar. It names what the export starts from: a remembered 720p export shows
HD 30 on a new project, and Export still opens at 720p until someone chooses. Choosing FHD and then 60 fps in its
popover changes the canvas in one undo step each, the export dialog then opens at 1080p 60 although the last export was
720p Reels, the preview steps a sixtieth of a second, the size estimate follows the quality and the timeline's length,
and HD exports a 720x1280 file at 60 fps."""
import json
import subprocess
import time

from e2e.harness import FIXTURES, changed_share, flow, preview_crop, preview_rect, wait
from e2e.home import resize

CANVAS = 'return window.__nuzky.store.getState().snap.project.canvas'
CHIP = '[data-quality]'
POPOVER = '[role=dialog][aria-label=Quality]'
ESTIMATE = f"return document.querySelector('{POPOVER} [data-estimate]')?.textContent ?? null"
DIALOG_PRESSED = "return document.querySelector(`[role=dialog][aria-labelledby=export-title] [aria-label='${arguments[0]}'] [aria-pressed=true]`)?.textContent"
DIALOG_SIZE = "return document.querySelector('[role=dialog][aria-labelledby=export-title] [title^=Estimate] span')?.textContent ?? null"


def chip(r):
    return r.s.run(f"return document.querySelector('{CHIP}')?.textContent ?? null")


def open_quality(r):
    if not r.s.run(f"return !!document.querySelector('{POPOVER}')"):
        r.s.run(f"document.querySelector('{CHIP}').click()")
    return wait(lambda: r.s.run(f"return !!document.querySelector('{POPOVER}')"), 5)


def close_quality(r):
    r.key('Escape')
    # The popover fades out for 150 ms.
    wait(lambda: not r.s.run(f"return !!document.querySelector('{POPOVER}')"), 5)
    time.sleep(0.3)


def quality(r, resolution=None, fps=None, shot=None):
    """Chooses a resolution ("FHD", "4K") and a frame rate in the quality popover, as clicks do."""
    open_quality(r)
    time.sleep(0.3)
    for group, label in (('Resolution', resolution), ('Frame rate', fps)):
        if label is not None:
            r.s.run(f"""[...document.querySelectorAll(`{POPOVER} [aria-label='{group}'] button`)]
                .find((b) => b.textContent === arguments[0]).click()""", str(label))
    if shot:
        time.sleep(0.6)
        r.shot(shot)
    close_quality(r)


def canvas_is(r, **want):
    return wait(lambda: (c := r.s.run(CANVAS)) and all(c[k] == v for k, v in want.items()) and c, 10)


def export_dialog(r):
    r.s.run("window.__nuzky.store.setState({exportOpen: true})")
    wait(lambda: r.s.run(DIALOG_PRESSED, 'Resolution'), 5)
    time.sleep(0.4)
    return r.s.run(DIALOG_PRESSED, 'Resolution'), r.s.run(DIALOG_PRESSED, 'Frame rate')


def close_export(r):
    r.s.run("window.__nuzky.store.setState({exportOpen: false})")
    wait(lambda: not r.s.run("return !!document.querySelector('[role=dialog][aria-labelledby=export-title]')"), 5)
    time.sleep(0.3)


def preview(r, name):
    r.s.run('window.__nuzky.store.getState().select([])')
    time.sleep(1.5)
    r.shot(name)
    return preview_crop(r.work / f'{name}.png', preview_rect(r))


def megabytes(text):
    return float(text.split()[-2]) * (1000 if text.endswith('GB') else 1)


@flow('quality', 'The quality chip in the top bar names what the export starts from, changes the canvas and frame rate in '
      'one undo step for the preview and the export, and estimates the file from the quality and the length')
def quality_chip(r):
    # As after a 720p export: the dialog remembers it, and the chip reads it again when the dialog closes.
    r.s.run("localStorage.setItem('nuzky.export', JSON.stringify({resolution: 720, quality: 'recommended', preset: null}))")
    export_dialog(r)
    close_export(r)
    shown = wait(lambda: chip(r) == 'HD 30', 5) and chip(r)
    r.check('a new project after a 720p export shows HD 30, what Export starts from', shown == 'HD 30', chip(r))
    open_quality(r)
    empty = r.s.run(f"return document.querySelector('{POPOVER}').textContent")
    r.check('with an empty timeline the popover says how to get a size', 'Add a clip to the timeline to see the file size' in empty, empty)
    close_quality(r)
    r.check('the export dialog still opens at the remembered 720p', export_dialog(r) == ('720p', '30'), export_dialog(r))
    close_export(r)
    r.check('nothing in the project changed', r.s.run(CANVAS)['qualityChosen'] is False and not r.s.run(
        'return window.__nuzky.store.getState().snap.canUndo'), r.s.run(CANVAS))

    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    quality(r, resolution='FHD')
    chosen = canvas_is(r, width=1080, height=1920, fps=30, qualityChosen=True)
    r.check('choosing FHD keeps the 1080x1920 canvas and marks its quality as chosen', chosen, r.s.run(CANVAS))
    r.check('the chip then says FHD 30', wait(lambda: chip(r) == 'FHD 30', 5), chip(r))

    open_quality(r)
    at_30 = wait(lambda: r.s.run(ESTIMATE), 5)
    r.s.run(f"""[...document.querySelectorAll(`{POPOVER} [aria-label='Frame rate'] button`)].find((b) => b.textContent === '60').click()""")
    r.check('60 fps sets the canvas frame rate', canvas_is(r, fps=60, qualityChosen=True), r.s.run(CANVAS))
    at_60 = wait(lambda: (e := r.s.run(ESTIMATE)) != at_30 and e, 5)
    r.check('the estimate grows with the frame rate', at_30 and at_60 and megabytes(at_60) > megabytes(at_30) * 1.5, [at_30, at_60])
    r.check('the chip says FHD 60', wait(lambda: chip(r) == 'FHD 60', 5), chip(r))
    time.sleep(0.5)
    r.shot('quality-popover')
    # A longer timeline makes a larger file, while the popover stays open.
    r.add_clip('talk.mp4')
    longer = wait(lambda: (e := r.s.run(ESTIMATE)) != at_60 and e, 5)
    r.check('the estimate follows the timeline\'s length', longer and abs(megabytes(longer) - 2 * megabytes(at_60)) <= 2,
            [at_60, longer])
    close_quality(r)

    r.s.run('window.__nuzky.store.getState().undo()')
    r.check('one undo removes the second clip', wait(lambda: len(r.track()) == 1, 5), len(r.track()))
    r.s.run('window.__nuzky.store.getState().undo()')
    back = canvas_is(r, fps=30, qualityChosen=True)
    r.check('the next undo brings back 30 fps in one step', back and wait(lambda: chip(r) == 'FHD 30', 5), [r.s.run(CANVAS), chip(r)])
    r.s.run('window.__nuzky.store.getState().redo()')
    r.check('redo brings 60 fps back', canvas_is(r, fps=60) and wait(lambda: chip(r) == 'FHD 60', 5), chip(r))

    r.seek(0)
    time.sleep(0.3)
    r.s.run('document.activeElement?.blur()')
    r.key('ArrowRight')
    step = wait(lambda: r.s.run('return window.__nuzky.store.getState().timeUs') or None, 5)
    r.check('the preview steps a sixtieth of a second per frame', step == 16667, step)

    r.s.run("localStorage.setItem('nuzky.export', JSON.stringify({resolution: 720, quality: 'small', preset: 'reels'}))")
    picked = export_dialog(r)
    preset = r.s.run("return document.querySelector('[role=dialog][aria-labelledby=export-title] button[aria-label^=Preset]').getAttribute('aria-label')")
    r.check('the export dialog opens at the chosen 1080p 60, not the remembered 720p Reels at 30 fps',
            picked == ('1080p', '60') and preset == 'Preset: Custom', [picked, preset])
    in_dialog = r.s.run(DIALOG_SIZE)
    close_export(r)
    open_quality(r)
    in_chip = r.s.run(ESTIMATE)
    r.check('the chip estimates the same file as the export dialog', in_dialog and in_chip == f'about {in_dialog}', [in_chip, in_dialog])
    close_quality(r)

    r.seek(2_000_000)
    fhd = preview(r, 'quality-preview-fhd')
    quality(r, resolution='HD')
    r.check('HD makes the canvas 720x1280', canvas_is(r, width=720, height=1280, fps=60), r.s.run(CANVAS))
    hd = preview(r, 'quality-preview-hd')
    differ = changed_share(fhd, hd)
    r.check('the preview shows the same picture on HD', differ < 0.01, differ)
    resolution, fps = export_dialog(r)
    close_export(r)
    r.check('Export opens at 720p 60', (resolution, fps) == ('720p', '60'), [resolution, fps])
    target = r.work / 'quality.mp4'
    started = r.s.call('window.__nuzky.api.startExport(arguments[0], arguments[1], arguments[2], true)', str(target),
                       {'resolution': 720, 'fps': 60, 'quality': 'small'}, r.state()['epoch'])
    job = started['ok'] and wait(lambda: (j := r.s.run("return window.__nuzky.store.getState().jobs[arguments[0]] ?? null", started['value']))
                                 and j['status'] != 'running' and j, 300)
    r.check('the export finishes', job and job['status'] == 'done', [started, job])
    probe = json.loads(subprocess.run(['ffprobe', '-v', 'error', '-select_streams', 'v', '-show_entries', 'stream=width,height,r_frame_rate',
                                       '-of', 'json', str(target)], capture_output=True, text=True, check=True).stdout)['streams'][0]
    r.check('the file is 720x1280 at 60 fps', probe == {'width': 720, 'height': 1280, 'r_frame_rate': '60/1'}, probe)

    r.s.run("window.__nuzky.store.setState({aiRun: 'Cutting', lockShown: true})")
    locked = wait(lambda: r.s.run(f"return document.querySelector('{CHIP}').getAttribute('aria-disabled')") == 'true', 5)
    r.s.run(f"document.querySelector('{CHIP}').click()")
    time.sleep(0.3)
    r.check('while the AI edits the chip is locked and opens nothing', locked and not r.s.run(f"return !!document.querySelector('{POPOVER}')"))
    r.s.run("window.__nuzky.store.setState({aiRun: null, lockShown: false})")

    for width, height in ((1440, 900), (1024, 640)):
        resize(r, width, height)
        time.sleep(0.5)
        quality(r, shot=f'quality-{width}')
        box = r.s.run(f"const b = document.querySelector('{CHIP}').getBoundingClientRect(); return [b.height, b.right]")
        export = r.s.run("return document.querySelector('[data-tour=export]').getBoundingClientRect().right")
        r.check(f'the chip stays one line and Export stays in the window at {width}x{height}', box[0] <= 32 and export <= width,
                [box, export])

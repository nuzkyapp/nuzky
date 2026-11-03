"""A 4K canvas from the Ratio menu. A 9:16 project with a grey picture in a red 20 px border and the Classic text
switches to 4K: the canvas becomes 2160x3840, the text, its outline and the border double in canvas pixels, so the
preview looks as it did, and Classic is still the text's style. Title added on 4K comes out twice its 1080p size.
16:9 keeps 4K, Export opens at 4K although the last export was 1080p, and 1080p brings back the sizes exactly."""
import subprocess, time

from e2e.harness import FIXTURES, changed_share, flow, link_models, preview_crop, preview_rect, wait
from e2e.home import resize

PROJECT = 'return window.__nuzky.store.getState().snap.project'
RATIO = 'section[aria-label=Preview] button[aria-haspopup=menu]'
TEXTS = "return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips).filter((c) => c.content.type === 'text').map((c) => c.content.style)"
BORDER = "return window.__nuzky.store.getState().snap.project.tracks[0].clips[0].content.shape.borderWidth"


def picture(r):
    r.grey = r.work / 'grey.png'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'color=0x808080:s=1080x1920', '-frames:v', '1',
                    str(r.grey)], check=True)


def ratio(r, label, shot=None):
    """Picks the Ratio menu row that starts with `label`, as a click does."""
    r.s.run(f'document.querySelector("{RATIO}").click()')
    wait(lambda: r.s.run('return !!document.querySelector("[role=menu][aria-label=\'Canvas ratio\']")'), 5)
    if shot:
        time.sleep(0.3)
        r.shot(shot)
    r.s.run("""[...document.querySelectorAll('[role=menu] [role=menuitemradio]')]
        .find((b) => b.textContent.trim().startsWith(arguments[0])).click()""", label)


def canvas_is(r, width, height):
    return wait(lambda: (c := r.s.run(PROJECT)['canvas']) and (c['width'], c['height']) == (width, height) and c, 10)


def add_text(r, name):
    count = len(r.s.run(TEXTS))
    r.s.run("window.__nuzky.store.setState({panelTab: 'text'})")
    find = "[...document.querySelectorAll('button')].find((b) => b.title === `Add \"${arguments[0]}\" text at the playhead`)"
    wait(lambda: r.s.run(f'return !!{find}', name), 5)
    r.s.run(f'{find}.click()', name)
    return wait(lambda: (s := r.s.run(TEXTS)) and len(s) == count + 1 and s[-1], 10)


def preview(r, name):
    time.sleep(1.5)
    r.shot(name)
    return preview_crop(r.work / f'{name}.png', preview_rect(r))


@flow('resolution', 'Switch a 9:16 project to a 4K canvas from Ratio: text, outline and border keep their size in the '
      'preview, Classic stays recognised, Title comes out at 4K size, 16:9 keeps 4K, Export opens at 4K and 1080p '
      'brings the sizes back', before=picture)
def resolution(r):
    r.import_media(r.grey)
    r.add_clip('grey.png')
    r.s.call("""window.__nuzky.store.getState().edit((p) => ({type: 'updateClip', clipId: p.tracks[0].clips[0].id,
        transform: {...p.tracks[0].clips[0].content.transform, scale: 0.6},
        shape: {radius: 0, borderWidth: 20, borderColor: '#ff0000', shadow: 0}}))""")
    wait(lambda: r.s.run(BORDER) == 20, 10)
    r.seek(0)
    classic = add_text(r, 'Classic')
    r.check('Classic on 1080p is 84 px with a 7 px outline', classic and (classic['fontSize'], classic['strokeWidth']) == (84, 7), classic)
    r.s.run('window.__nuzky.store.getState().select([])')
    hd = preview(r, 'resolution-1080p')

    ratio(r, '4K', 'resolution-menu')
    four_k = canvas_is(r, 2160, 3840)
    r.check('4K in Ratio makes the 9:16 canvas 2160x3840', four_k, four_k or r.s.run(PROJECT)['canvas'])
    style = r.s.run(TEXTS)[0]
    r.check('the text and its outline double in canvas pixels', (style['fontSize'], style['strokeWidth']) == (168, 14), style)
    r.check('the border doubles too', r.s.run(BORDER) == 40, r.s.run(BORDER))
    uhd = preview(r, 'resolution-4k')
    differ = changed_share(hd, uhd)
    r.check('the preview looks as it did on 1080p', differ < 0.01, differ)

    r.s.run("const st = window.__nuzky.store.getState(); st.select([st.snap.project.tracks.flatMap((t) => t.clips).find((c) => c.content.type === 'text').id])")
    pressed = wait(lambda: r.s.run("return document.querySelector('aside[aria-label=Inspector] button[title=\"Apply Classic style\"]')?.getAttribute('aria-pressed')") == 'true', 5)
    r.check('the inspector still marks Classic as the text\'s style', pressed)
    r.shot('resolution-inspector')

    title = add_text(r, 'Title')
    r.check('Title added on 4K is 260 px, twice its 1080p size', title and title['fontSize'] == 260, title)

    ratio(r, '16:9')
    wide = canvas_is(r, 3840, 2160)
    r.check('16:9 keeps 4K: 3840x2160', wide, wide or r.s.run(PROJECT)['canvas'])
    label = r.s.run(f'return document.querySelector("{RATIO}").textContent')
    r.check('the Ratio button names the resolution', label.endswith('16:9 · 4K'), label)

    r.s.run("localStorage.setItem('nuzky.export', JSON.stringify({resolution: 1080, quality: 'recommended', preset: null}))")
    r.s.run("window.__nuzky.store.setState({exportOpen: true})")
    # The Ratio menu fades out for 150 ms after a pick.
    wait(lambda: r.s.run("return !document.querySelector('[role=menu][aria-label=\\'Canvas ratio\\']') && !!document.querySelector('[role=dialog]')"), 5)
    time.sleep(0.4)
    # The dialog shows the options of its last opening for a frame, until it sets this project's.
    shown = lambda: r.s.run("return document.querySelector('[role=group][aria-label=Resolution] [aria-pressed=true]')?.textContent")
    picked = wait(lambda: shown() == '4K', 5) and shown()
    r.check('Export opens at 4K although the last export was 1080p', picked == '4K', picked)
    r.shot('resolution-export')
    # Only quality changes here; the 4K stays this project's and is not remembered for the 1080p ones.
    r.s.run("[...document.querySelectorAll('[role=group][aria-label=Quality] button')].find((b) => b.textContent === 'High').click()")
    r.s.run("window.__nuzky.store.setState({exportOpen: false})")

    ratio(r, '1080p')
    back = canvas_is(r, 1920, 1080)
    styles = r.s.run(TEXTS)
    r.check('1080p brings back 1920x1080 and the exact sizes',
            back and [(s['fontSize'], s['strokeWidth']) for s in styles] == [(84, 7), (130, 0)] and r.s.run(BORDER) == 20,
            [back, styles, r.s.run(BORDER)])
    r.s.run("window.__nuzky.store.setState({exportOpen: true})")
    picked = wait(lambda: shown() == '1080p', 5) and shown()
    remembered = r.s.run("return JSON.parse(localStorage.getItem('nuzky.export'))")
    r.check('Export on 1080p still opens at the remembered 1080p, with the quality picked on 4K', picked == '1080p'
            and remembered['quality'] == 'high', [picked, remembered])


@flow('resolution-captions', 'Captions generated from the Captions tab on a 4K canvas come out at twice the Reel '
      'style\'s 1080p size, wrap inside the 4K safe area, and Reel stays marked as their style', before=link_models)
def resolution_captions(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    ratio(r, '4K')
    r.check('the canvas is 2160x3840', canvas_is(r, 2160, 3840), r.s.run(PROJECT)['canvas'])
    r.s.run("window.__nuzky.speech.setState({model: 'small', language: 'en'})")
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === 'Captions').click()")
    time.sleep(0.5)
    r.s.run("[...document.querySelectorAll('button')].find((b) => b.textContent.includes('Generate captions')).click()")
    job = wait(lambda: next((j for j in r.state()['jobs'] if j['kind'] == 'captions' and j['status'] != 'running'), None), 300, 1)
    r.check('captions finish', job and job['status'] == 'done', job)
    styles = r.s.run(TEXTS)
    sizes = {(s['fontSize'], s['strokeWidth']) for s in styles}
    r.check('every caption is Reel at 4K: 190 px with a 15 px outline', styles and sizes == {(190, 15)}, sizes)
    # The safe area leaves 720 px of a 1080 px wide vertical canvas for a centred caption.
    r.check('they wrap inside the 4K safe area: 1440 px, twice 720', {s['maxWidth'] for s in styles} == {1440},
            {s['maxWidth'] for s in styles})
    reel = wait(lambda: r.s.run("return document.querySelector('button[title=\"Apply Reel to all captions\"]')?.getAttribute('aria-pressed') === 'true'"), 5)
    r.check('the Captions tab marks Reel as their style', reel)
    track = next(t for t in r.s.run(PROJECT)['tracks'] if t['kind'] == 'text')
    caption = max(track['clips'], key=lambda c: c['durationUs'])
    r.s.run('window.__nuzky.store.getState().select([])')
    r.seek(caption['startUs'] + caption['durationUs'] // 2)
    time.sleep(1.5)
    r.shot('resolution-captions')
    # The Ratio button names the resolution on one line, at the smallest window too.
    for width, height in ((1440, 900), (1024, 640)):
        resize(r, width, height)
        time.sleep(0.5)
        lines = r.s.run(f'return Math.round(document.querySelector("{RATIO}").getBoundingClientRect().height)')
        r.check(f'the Ratio button stays one line at {width}x{height}', lines <= 32, lines)
        clipped = r.s.run("const t = document.querySelector('section[aria-label=Preview] span.tabular.truncate'); return t ? t.scrollWidth > t.clientWidth : null")
        r.check(f'the timecode is not cut short at {width}x{height}', clipped is False, clipped)
        r.shot(f'resolution-captions-{width}')

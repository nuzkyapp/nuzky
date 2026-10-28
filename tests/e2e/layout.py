import json, subprocess, time

from e2e.harness import CLI, FIXTURES, flow, preview_crop, wait, webdriver

# Nulls while the editor is still drawing after a reload.
SIZES = """const width = (e) => e ? Math.round(e.getBoundingClientRect().width) : null;
const timeline = document.querySelector('section[aria-label=Timeline]');
return {library: width(document.querySelector('[role=tablist][aria-label=Library]')?.closest('aside')),
        inspector: width(document.querySelector('aside[aria-label=Inspector]')),
        preview: width(document.querySelector('section[aria-label=Preview]')),
        timeline: timeline ? Math.round(timeline.getBoundingClientRect().height) : null,
        window: [window.innerWidth, window.innerHeight]};"""
GAP = "document.querySelector(`[role=separator][aria-label='Resize ${arguments[0]}']`)"
DRAG = GAP + """; const b = s.getBoundingClientRect();
const at = (type, dx, dy) => new PointerEvent(type, {bubbles: true, button: 0, pointerId: 1,
  clientX: b.left + b.width / 2 + dx, clientY: b.top + b.height / 2 + dy});
s.dispatchEvent(at('pointerdown', 0, 0));
window.dispatchEvent(at('pointermove', arguments[1], arguments[2]));
window.dispatchEvent(at('pointerup', arguments[1], arguments[2]));"""
SLIDERS = """const inspector = document.querySelector('aside[aria-label=Inspector]'); const full = inspector.getBoundingClientRect().width;
return [...inspector.querySelectorAll('input[type=range]')].map((range) => {
  const label = range.closest('div.flex-col').firstElementChild;
  const l = label.getBoundingClientRect(), t = range.getBoundingClientRect();
  return {label: label.textContent, above: l.bottom <= t.top && Math.abs(l.left - t.left) < 2,
          whole: label.scrollWidth <= label.clientWidth, share: Math.round(t.width / full * 100) / 100};
});"""


# A 4 px square of the stage beside the frame, and the mark of media the timeline uses.
LOOK = """const box = (r) => ({left: r.left, top: r.top, width: r.width, height: r.height, viewport: window.innerWidth});
const stage = document.querySelector('section[aria-label=Preview]').getBoundingClientRect();
const canvas = document.querySelector('section[aria-label=Preview] canvas').getBoundingClientRect();
const beside = canvas.left - stage.left >= 12 ? {left: (stage.left + canvas.left) / 2 - 2, top: canvas.top + canvas.height / 2}
                                              : {left: canvas.left + canvas.width / 2, top: (stage.top + canvas.top) / 2 - 2};
const name = [...document.querySelectorAll('[role=tablist][aria-label=Library] ~ [role=tabpanel] span')].find((e) => e.textContent === 'talk.mp4');
const tile = name.closest('.group');
const dot = [...document.querySelectorAll('[role=status]')].find((s) => s.textContent.trim() === 'Saved')?.querySelector('span[aria-hidden]');
return {stage: box({...beside, width: 4, height: 4}), saved: dot ? getComputedStyle(dot).backgroundColor : null,
        used: (tile.title || '').includes('In the timeline') && !!tile.querySelector('span[aria-hidden] > svg')};"""
# Chooses the tab named arguments[1] in the tab list at arguments[0]: whether the chosen tab's plate slides there, and once
# it settled, how many px it lies off that tab: at most 1, as its edges snap to the screen's pixels.
SLIDE = """(async () => { const list = document.querySelector(arguments[0]);
const plate = () => list.querySelector(':scope > .seg-thumb');
[...list.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === arguments[1]).click();
await new Promise(requestAnimationFrame);
const sliding = !!plate()?.getAnimations().some((a) => a.transitionProperty === 'transform' && a.playState === 'running');
await new Promise((done) => setTimeout(done, 400));
return JSON.stringify({sliding, off: PLATE_OFF(list)}); })()"""
PLATE_OFF = """((list) => { const plate = list.querySelector(':scope > .seg-thumb'), tab = list.querySelector('[role=tab][aria-selected=true]');
if (!plate || !tab) return null;
const a = plate.getBoundingClientRect(), b = tab.getBoundingClientRect();
return Math.round(Math.max(Math.abs(a.left - b.left), Math.abs(a.top - b.top), Math.abs(a.width - b.width), Math.abs(a.height - b.height))); })"""
SLIDE = SLIDE.replace('PLATE_OFF', PLATE_OFF)
LIBRARY_TABS, INSPECTOR_TABS = '[role=tablist][aria-label=Library]', 'aside[aria-label=Inspector] [role=tablist]'


def slide(r, tabs, name):
    done = r.s.call(SLIDE, tabs, name)
    return json.loads(done['value']) if done['ok'] else done


def slid(moved):
    return moved.get('sliding') is True and moved.get('off') is not None and moved['off'] <= 1


# The inspector's header tile and the group behind the first slider.
HEADER = """const tile = document.querySelector('aside[aria-label=Inspector] > div:first-child > span').getBoundingClientRect();
const group = document.querySelector('aside[aria-label=Inspector] input[type=range]').closest('section > div');
return {tile: {left: tile.left + 2, top: tile.top + 2, width: 4, height: 4, viewport: window.innerWidth},
        group: getComputedStyle(group).backgroundColor, radius: getComputedStyle(group).borderRadius};"""


COLUMNS = """const tile = [...document.querySelectorAll('[role=tablist][aria-label=Library] ~ [role=tabpanel] *')].find((e) => e.textContent === 'talk.mp4');
return getComputedStyle(tile.closest('.grid')).gridTemplateColumns.split(' ').length;"""


def setup(r):
    project = r.work / 'data/nuzky/projects/layout.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(FIXTURES / 'talk.mp4')], env=r.env, check=True, capture_output=True)


def sizes(r):
    return r.s.run(SIZES, retries=3)


def drag(r, what, dx, dy=0):
    r.s.run('const s = ' + DRAG, what, dx, dy)


def press(r, what, key):
    r.s.run(GAP + '.focus()', what)
    r.key(key)


def resize(r, width, height):
    webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
    return wait(lambda: sizes(r)['window'] == [width, height], 5)


@flow('layout', "The library, inspector and timeline start in CapCut's proportions, resize from the gaps between them by "
      'pointer and keyboard, never squeeze the preview under 340 px, are remembered, and sliders carry their label above',
      before=setup)
def layout(r):
    first = sizes(r)
    columns = r.s.run(COLUMNS)
    w, h = first['window']
    r.check("by default the library and the inspector take 28 % of the window's width and the timeline 35 % of its height",
            first['library'] == round(w * 0.28) and first['inspector'] == round(w * 0.28) and first['timeline'] == round(h * 0.35), first)
    r.shot('default')
    look = r.s.run(LOOK)
    stage = preview_crop(r.work / 'default.png', look['stage']).getpixel((1, 1))
    r.check('the preview is black around the frame, so the video is the brightest thing there', max(stage) <= 2, stage)
    r.check('the media tile of a video the timeline uses carries a check and says so in its tooltip', look['used'], look)
    r.check('Saved in the top bar starts with a green dot', look['saved'] == 'rgb(48, 209, 88)', look['saved'])
    fade = r.s.run("""const img = [...document.querySelectorAll('[role=tablist][aria-label=Library] ~ [role=tabpanel] span')]
        .find((e) => e.textContent === 'talk.mp4').closest('.group').querySelector('img'); const st = getComputedStyle(img);
        return {loaded: img.dataset.loaded ?? null, opacity: st.opacity, fades: st.animationName === 'img-in' && st.animationDuration === '0.18s'};""")
    r.check("the media tile's thumbnail, which arrives after the tile, fades in over 180 ms instead of popping in, and then shows fully",
            fade == {'loaded': 'fade', 'opacity': '1', 'fades': True}, fade)
    zoom = r.s.run("""const b = document.querySelector('input[aria-label="Timeline zoom"]').getBoundingClientRect();
        return {left: b.left, top: b.top, width: b.width, height: b.height, viewport: window.innerWidth};""")
    track = preview_crop(r.work / 'default.png', zoom)
    start = track.getpixel((1, track.height // 2))
    r.check('the timeline zoom slider fills with accent from the very left end of its track',
            start[2] > 200 and start[0] < 100, start)

    drag(r, 'library', 100)
    s = sizes(r)
    r.check('dragging the gap right of the library widens it, and only the preview gives way',
            s['library'] == first['library'] + 100 and s['inspector'] == first['inspector'] and s['preview'] == first['preview'] - 100, s)
    r.seek(1_000_000)
    wait(lambda: r.s.run('return window.__nuzky.store.getState().timeUs') == 1_000_000, 5)
    press(r, 'inspector', 'ArrowLeft')
    s = sizes(r)
    r.check('←, with the gap left of the inspector focused, widens the inspector by 24 px', s['inspector'] == first['inspector'] + 24, s)
    r.check('the arrow did not move the playhead', r.s.run('return window.__nuzky.store.getState().timeUs') == 1_000_000)
    drag(r, 'timeline', 0, -60)
    press(r, 'timeline', 'ArrowUp')
    s = sizes(r)
    r.check('dragging the gap above the timeline up and ↑ make the timeline taller', s['timeline'] == first['timeline'] + 84, s)
    drag(r, 'library', 2000)
    wide = sizes(r)
    r.check('the library stops where the preview has 340 px left, and the inspector keeps its width',
            wide['preview'] == 340 and wide['inspector'] == first['inspector'] + 24, wide)
    tabs = r.s.run("return [...document.querySelectorAll('[role=tablist][aria-label=Library] [role=tab]')].map((t) => t.getBoundingClientRect().width)")
    r.check('the library tabs share the extra width', min(tabs) > 50, tabs)
    r.check('the wider library shows more media in a row instead of larger tiles', columns == 3 and r.s.run(COLUMNS) >= 5, [columns, r.s.run(COLUMNS)])
    r.shot('resized')

    r.s.run('window.__oldPage = true; location.reload()')
    wait(lambda: r.s.run("const c = window.__nuzky; if (window.__oldPage || !c?.store.getState().snap) return false; "
                         "c.store.setState({view: 'editor'}); return true", retries=3), 30)
    r.check('after a restart every pane is as the user left it', wait(lambda: sizes(r) == wide, 10), sizes(r))
    r.s.run(GAP + ".dispatchEvent(new MouseEvent('dblclick', {bubbles: true}))", 'library')
    s = sizes(r)
    r.check('double-clicking the gap puts the library back to its share of the window', s['library'] == first['library'], s)

    if r.check('the window gets to 1024 x 640', resize(r, 1024, 640)):
        # The panes follow on the render after the resize event; the library, back on its share, shows it.
        wait(lambda: sizes(r)['library'] == 360, 5)
        small = sizes(r)
        r.check('in the smallest window the inspector gives way first and the preview keeps 340 px',
                small['preview'] == 340 and small['library'] == 360 and small['inspector'] == 300, small)
        r.check('the timeline leaves 300 px above it for the preview', small['timeline'] == 640 - 48 - 300 - 12, small)
        tabs = r.s.run("const list = document.querySelector('[role=tablist][aria-label=Library]').getBoundingClientRect();"
                       "return {list: [list.left, list.right], labels: [...document.querySelectorAll('[role=tablist][aria-label=Library] [role=tab] span')]"
                       ".map((s) => { const r = s.getBoundingClientRect(); return [r.left, r.right, s.scrollWidth]; })}")
        labels = tabs['labels']
        gaps = [b[0] - a[1] for a, b in zip(labels, labels[1:])]
        r.check('in the narrowest library every tab label shows whole and no two touch',
                len(labels) == 8 and min(gaps) >= 2 and labels[0][0] >= tabs['list'][0] and labels[-1][1] <= tabs['list'][1], tabs)
        r.shot('small-window')
        resize(r, w, h)
        r.check('back in the large window the inspector and the timeline are as the user left them',
                wait(lambda: (lambda s: s['inspector'] == wide['inspector'] and s['timeline'] == wide['timeline'])(sizes(r)), 5), sizes(r))
        plate = lambda: r.s.run(f'return {PLATE_OFF}(document.querySelector(arguments[0]))', LIBRARY_TABS)
        r.check("after the library's tabs changed width, the chosen tab's plate still lies exactly under it", wait(lambda: (lambda off: off is not None and off <= 1)(plate()), 5), plate())

    clip = r.track()[0]['id']
    r.s.run('window.__nuzky.store.getState().select([arguments[0]])', clip)
    wait(lambda: r.s.run("return [...document.querySelectorAll('aside[aria-label=Inspector] [role=tab]')].some((t) => t.textContent.trim() === 'Adjust')"), 5)
    groups = r.s.run("""return [...document.querySelectorAll('aside[aria-label=Inspector] [role=group]')].filter((g) => g.querySelector('[aria-pressed=true]'))
        .map((g) => { const a = g.querySelector(':scope > .seg-thumb')?.getBoundingClientRect(), b = g.querySelector('[aria-pressed=true]').getBoundingClientRect();
          return a ? Math.round(Math.abs(a.left - b.left) + Math.abs(a.width - b.width)) : null; })""")
    r.check("every segmented control in the Video tab has its plate under the chosen segment", groups and all(g is not None and g <= 1 for g in groups), groups)
    moved = slide(r, INSPECTOR_TABS, 'Adjust')
    r.check("choosing Adjust slides the inspector tabs' plate from Video to Adjust, where it settles", slid(moved), moved)
    sliders = wait(lambda: (lambda s: s if len(s) == 10 else None)(r.s.run(SLIDERS)), 5) or r.s.run(SLIDERS)
    r.check('every Adjust slider has its name above it, from the same left edge, in full',
            len(sliders) == 10 and all(s['above'] and s['whole'] for s in sliders), sliders)
    r.check('the sliders run across most of the inspector', all(s['share'] >= 0.7 for s in sliders), [s['share'] for s in sliders])
    r.shot('adjust')
    header = r.s.run(HEADER)
    tile = preview_crop(r.work / 'adjust.png', header['tile']).getpixel((1, 1))
    r.check("the inspector's header shows the clip's kind on a tile in the video clips' blue",
            all(abs(a - b) <= 6 for a, b in zip(tile, (0x24, 0x58, 0x91))), tile)
    r.check("the Adjust sliders sit in a section's rounded group", header['group'] not in ('rgba(0, 0, 0, 0)', 'transparent') and header['radius'] == '10px', header)

    moved = slide(r, LIBRARY_TABS, 'Text')
    r.check("choosing Text slides the library tabs' plate there", slid(moved), moved)
    wait(lambda: r.s.run("const b = document.querySelector('button[title$=\"text at the playhead\"]'); if (!b) return false; b.click(); return true"), 5)
    font = wait(lambda: r.s.run("""const l = [...document.querySelectorAll('aside[aria-label=Inspector] span')].find((s) => s.textContent === 'Font');
        if (!l) return null; const a = l.getBoundingClientRect(), b = l.nextElementSibling.getBoundingClientRect();
        return {above: a.bottom <= b.top, left: Math.round(b.left - a.left), height: Math.round(b.height)};"""), 5)
    r.check('the font picker has its label above it too, like the sliders, at its full 32 px height',
            font and font['above'] and font['left'] == 0 and font['height'] == 32, font)
    # The inspector settles on the new clip first, then shows the Style section for the proof.
    time.sleep(1)
    r.s.run("[...document.querySelectorAll('aside[aria-label=Inspector] h3')].find((h) => h.textContent === 'Style').scrollIntoView()")
    time.sleep(0.3)
    r.shot('text')

    # A track of only pictures takes the pictures' colour in its header, as its clips have.
    r.import_media(FIXTURES / 'face-open.png')
    r.s.call("""(() => { const st = window.__nuzky.store.getState();
        const asset = st.snap.project.assets.find((a) => a.name === 'face-open.png');
        return st.edit({type: 'addPictureInPicture', assetId: asset.id, startUs: 0, durationUs: null}); })()""")
    tone = lambda: r.s.run("""const lane = [...document.querySelectorAll('[data-lane]')].find((l) => l.querySelector('[aria-label^="face-open.png"]'));
        const tile = lane?.querySelector('span[aria-hidden]');
        return tile ? getComputedStyle(tile).backgroundColor : null;""")
    r.check("a track holding only a picture shows its tile in the pictures' indigo",
            wait(lambda: tone() == 'rgb(67, 70, 142)', 5), tone())
    r.check('no error toast', not r.errors(), r.errors())

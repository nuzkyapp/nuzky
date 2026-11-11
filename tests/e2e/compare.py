import colorsys, subprocess, time

from e2e.harness import export, flow, preview_crop, preview_rect, wait, webdriver

WIDTH, HEIGHT = 360, 640
ADJUST = "return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips).find((c) => c.id === arguments[0]).content.adjust"
REVISION = "return window.__nuzky.store.getState().snap.revision"
TAB = ("const t = [...document.querySelectorAll('[role=tablist][aria-label=Library] [role=tab]')].find((t) => t.textContent.trim() === arguments[0]);"
       'if (!t) return false; t.click(); return true')
TOGGLE = """const b = [...document.querySelectorAll('section[aria-label=Preview] button')].find((b) => b.querySelector('.lucide-square-split-horizontal'));
if (!b) return null; if (arguments[0]) b.click(); return b.getAttribute('aria-pressed') === 'true'"""
HANDLE = "document.querySelector('[role=slider][aria-label=\"Before and after divider\"]')"
# The line's place as a share of the frame's width, or null without it.
LINE = f"""const h = {HANDLE};
if (!h) return null;
const b = h.getBoundingClientRect(), f = document.querySelector('section[aria-label=Preview] canvas').getBoundingClientRect();
return (b.left + b.width / 2 - f.left) / f.width"""
# Presses the handle, or moves the pressed pointer to the share `arguments[1]` of the frame's width, or lets go.
POINTER = f"""const h = {HANDLE}, b = h.getBoundingClientRect();
const f = document.querySelector('section[aria-label=Preview] canvas').getBoundingClientRect();
const x = arguments[0] === 'pointerdown' ? b.left + b.width / 2 : f.left + f.width * arguments[1];
const event = new PointerEvent(arguments[0], {{bubbles: true, button: 0, pointerId: 1, clientX: x, clientY: b.top + b.height / 2}});
(arguments[0] === 'pointerdown' ? h : window).dispatchEvent(event);"""

BUTTON = "[...document.querySelectorAll('section[aria-label=Preview] button')].find((b) => b.querySelector('.lucide-square-split-horizontal'))"
REASON = f"const b = {BUTTON}; return b && b.getAttribute('aria-disabled') === 'true' ? b.title : null"
BOX = "return !!document.querySelector('[data-testid=layer-overlay]')"
# How far Play's centre is from the transport capsule's, in px.
OFF_CENTRE = """const play = [...document.querySelectorAll('section[aria-label=Preview] button')].find((b) => /^(Play|Pause)/.test(b.getAttribute('aria-label') || ''));
const p = play.getBoundingClientRect(), o = play.closest('.bar').getBoundingClientRect();
return Math.round((p.left + p.width / 2) - (o.left + o.width / 2))"""
# Whether the Compare button lies inside the transport capsule.
FITS = """const b = [...document.querySelectorAll('section[aria-label=Preview] button')].find((b) => b.querySelector('.lucide-square-split-horizontal'));
const bar = b?.closest('.bar');
if (!b || !bar) return false;
const r = b.getBoundingClientRect(), o = bar.getBoundingClientRect();
return r.width > 0 && r.left >= o.left && r.right <= o.right"""


def saturation(crop, start, end):
    """Mean saturation, max minus min channel, of the columns from `start` to `end` of the crop's width, clear of the
    labels at the top and the handle in the middle."""
    box = crop.crop((int(start * crop.width), int(0.12 * crop.height), int(end * crop.width), int(0.4 * crop.height)))
    pixels = list(box.getdata())
    return sum(max(p) - min(p) for p in pixels) / len(pixels)


def preview(r, name, rect, done, timeout=6):
    """The preview's picture once `done(crop)` holds, so it follows the last change."""
    end = time.time() + timeout
    while True:
        r.shot(name)
        crop = preview_crop(r.work / f'{name}.png', rect)
        if done(crop) or time.time() > end:
            return crop
        time.sleep(0.3)


@flow('compare', 'Compare under the preview shows the picture before a filter left of a line and after it on the right; the '
      'line follows the pointer and the arrow keys, changes nothing in the project or its undo, and never reaches the export')
def compare(r):
    from PIL import Image
    # Every column a fully saturated hue, so a filter's change shows anywhere along the width.
    image = Image.new('RGB', (WIDTH, HEIGHT))
    for x in range(WIDTH):
        colour = tuple(round(255 * c) for c in colorsys.hsv_to_rgb(x / WIDTH, 0.8, 0.85))
        image.paste(colour, (x, 0, x + 1, HEIGHT))
    path = r.work / 'hues.png'
    image.save(path)
    r.import_media(path)
    r.add_clip(path.name)
    clip = r.track()[0]
    middle = clip['startUs'] + clip['durationUs'] // 2
    r.seek(middle)
    r.focus_clip(clip['id'])
    reason = wait(lambda: r.s.run(REASON), 5)
    r.check('without a filter or adjustments Compare is disabled and says why', reason and 'Nothing to compare' in reason, reason)
    wait(lambda: r.s.run(TAB, 'Filters'), 5)
    wait(lambda: r.s.run("const b = document.querySelector('button[title=\"Apply Mono\"]'); if (b) b.click(); return !!b"), 5)
    r.check('Mono takes the colour out of the clip', wait(lambda: r.s.run(ADJUST, clip['id'])['saturation'] == -1, 5))
    rect = preview_rect(r)
    grey = preview(r, 'mono', rect, lambda c: saturation(c, 0.05, 0.95) <= 3)
    r.check('the preview shows the clip grey', saturation(grey, 0.05, 0.95) <= 3, saturation(grey, 0.05, 0.95))

    revision = r.s.run(REVISION)
    r.check('Compare turns on', wait(lambda: r.s.run(TOGGLE, True) is not None, 5) and wait(lambda: r.s.run(TOGGLE, False), 5))
    r.check('and draws the line across the middle', abs((wait(lambda: r.s.run(LINE), 5) or 0) - 0.5) < 0.01, r.s.run(LINE))
    r.check('the selected layer\'s box and its handles hide while comparing', wait(lambda: not r.s.run(BOX), 5), r.s.run(BOX))
    split = preview(r, 'compare', rect, lambda c: saturation(c, 0.05, 0.4) >= 40)
    r.check('left of the line the picture is as it was before the filter', saturation(split, 0.05, 0.4) >= 40, saturation(split, 0.05, 0.4))
    r.check('and right of it as the filter makes it', saturation(split, 0.6, 0.95) <= 3, saturation(split, 0.6, 0.95))

    r.s.run(POINTER, 'pointerdown', 0)
    for i, share in enumerate((0.58, 0.66, 0.75)):
        r.s.run(POINTER, 'pointermove', share)
        line = wait(lambda: (x := r.s.run(LINE)) is not None and abs(x - share) < 0.01 and x, 2)
        r.check(f'the line follows the pointer to {share:.0%}', line, r.s.run(LINE))
        moved = preview(r, f'compare-{i + 1}', rect, lambda c: saturation(c, share - 0.08, share - 0.04) >= 40)
        r.check('and the picture before the filter follows it', saturation(moved, share - 0.08, share - 0.04) >= 40
                and saturation(moved, share + 0.04, share + 0.08) <= 3,
                [saturation(moved, share - 0.08, share - 0.04), saturation(moved, share + 0.04, share + 0.08)])
    r.s.run(POINTER, 'pointerup', 0.75)
    r.check('a dropped line stays where it was let go', abs(r.s.run(LINE) - 0.75) < 0.01, r.s.run(LINE))
    r.s.run(f'{HANDLE}.focus()')
    r.key('ArrowLeft')
    r.check('← moves the focused line by 5 %', wait(lambda: abs(r.s.run(LINE) - 0.70) < 0.01, 2), r.s.run(LINE))
    r.check('moving the line changes nothing in the project', r.s.run(REVISION) == revision, [revision, r.s.run(REVISION)])

    rendered = export(r, 'compare.mp4')
    frame = r.work / 'exported.png'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-ss', str(middle / 1e6), '-i', str(rendered), '-frames:v', '1', str(frame)], check=True)
    exported = Image.open(frame).convert('RGB')
    r.check('the export has no line: all of it is after the filter', saturation(exported, 0.05, 0.95) <= 4,
            saturation(exported, 0.05, 0.95))

    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
        wait(lambda: r.s.run('return window.innerWidth') == width, 5)
        time.sleep(1)
        r.shot(f'compare-window{suffix}')
        r.check(f'at {width} x {height} Compare fits in the transport', r.s.run(FITS), r.s.run(FITS))
        # The smallest window has no room for both sides of the transport to match; main leaves Play 10 px left there.
        if width == 1440:
            off = r.s.run(OFF_CENTRE)
            r.check(f'and Play stays in the middle of it ({off} px off)', abs(off) <= 1, off)
    rect = preview_rect(r)

    r.s.run(TOGGLE, True)
    r.check('Compare turns off and takes the line away', wait(lambda: r.s.run(TOGGLE, False) is False and r.s.run(LINE) is None, 5))
    r.check('and the selected layer\'s box comes back', wait(lambda: r.s.run(BOX), 5))
    whole = preview(r, 'compare-off', rect, lambda c: saturation(c, 0.05, 0.95) <= 3)
    r.check('and the whole picture shows the filter again', saturation(whole, 0.05, 0.95) <= 3, saturation(whole, 0.05, 0.95))
    r.key('z', ctrlKey=True)
    r.check('one Undo after moving the line takes back the filter, so the line added no step',
            wait(lambda: r.s.run(ADJUST, clip['id'])['saturation'] == 0, 5), r.s.run(ADJUST, clip['id']))
    r.check('nothing shows an error', not r.errors(), r.errors())

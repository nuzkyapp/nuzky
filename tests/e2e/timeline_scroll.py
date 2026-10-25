import time as clock

from e2e.harness import FIXTURES, flow, wait
from e2e.timeline_select import PRESS, RECT, RELEASE

HEADER_W = 140
# The scroller and its lanes, the track headers' left edges and the ruler labels' left edges on screen.
VIEW = """const ruler = document.querySelector('[data-ruler]'); const el = ruler.closest('.overflow-auto'); const r = el.getBoundingClientRect();
return {left: r.left, right: r.left + el.clientWidth, scrollLeft: el.scrollLeft, scrollWidth: el.scrollWidth, content: ruler.parentElement.parentElement.offsetWidth,
  headers: [...el.querySelectorAll('[role=listbox]')].map((l) => l.previousElementSibling.getBoundingClientRect().left),
  labels: [...ruler.querySelectorAll('span.tabular')].map((s) => s.getBoundingClientRect().left),
  clipEnd: el.querySelector('[data-clip-id]').getBoundingClientRect().right};"""
# Ctrl + one wheel notch out, with the pointer arguments[0] px into the lanes; returns where the scroll and zoom ended up.
ZOOM_OUT = """const el = document.querySelector('[data-ruler]').closest('.overflow-auto'); const r = el.getBoundingClientRect();
el.dispatchEvent(new WheelEvent('wheel', {bubbles: true, cancelable: true, ctrlKey: true, deltaY: 100, deltaMode: 0, clientX: r.left + 140 + arguments[0], clientY: r.top + 60}));"""
SCROLL_ZOOM = "return [document.querySelector('[data-ruler]').closest('.overflow-auto').scrollLeft, window.__nuzky.store.getState().zoom]"
SCROLL_TO = "document.querySelector('[data-ruler]').closest('.overflow-auto').scrollLeft = arguments[0]"
TO_END = "const el = document.querySelector('[data-ruler]').closest('.overflow-auto'); el.scrollLeft = el.scrollWidth"


@flow('timeline_scroll', 'Scrolling the timeline to its end again and again stops where the lanes end: the ruler does not '
      'stretch it, the last clip ends in the middle of the lanes, the track headers stay put, the ruler has its times up to the right edge '
      'and zooming out there keeps the moment under the pointer; dragging two captions does not stretch it either')
def timeline_scroll(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    r.s.run('window.__nuzky.store.getState().setZoom(60)')
    clock.sleep(0.5)
    widths = []
    for _ in range(8):
        r.s.run(TO_END)
        clock.sleep(0.15)
        widths.append(r.s.run(VIEW)['scrollWidth'])
    v = r.s.run(VIEW)
    r.check('the timeline scrolls only as far as its lanes go', set(widths) == {v['content']}, (widths, v['content']))
    r.check('it reached the end', v['scrollLeft'] + v['right'] - v['left'] >= v['scrollWidth'] - 1, v)
    middle = v['left'] + HEADER_W + (v['right'] - v['left'] - HEADER_W) / 2
    r.check('the last clip ends in the middle of the lanes', abs(v['clipEnd'] + 1 - middle) < 2, (v['clipEnd'], middle))
    r.check('the track headers stay at the left edge', all(abs(h - v['left']) < 1 for h in v['headers']), (v['headers'], v['left']))
    # Ticks every 2 s at 60 px per second: one label within the last 120 px before the right edge.
    r.check('the ruler has its times up to the right edge', any(v['right'] - 130 <= x < v['right'] for x in v['labels']), (v['labels'], v['right']))
    r.shot('end')

    left, zoom = r.s.run(SCROLL_ZOOM)
    r.s.run(ZOOM_OUT, 100)
    clock.sleep(0.3)
    after, zoomed = r.s.run(SCROLL_ZOOM)
    r.check('zooming out at the end keeps the moment under the pointer',
            zoomed < zoom and abs((after + 100) / zoomed - (left + 100) / zoom) * zoomed < 1.5, (left, zoom, after, zoomed))

    # Two selected captions dragged 200 px right draw their ghosts as one shifted layer, which must not stretch the lanes.
    segment = lambda start, text: {'startUs': start, 'endUs': start + 600_000, 'text': text, 'words': []}
    r.s.call("window.__nuzky.store.getState().edit({type: 'addCaptions', segments: arguments[0], style: {fontSize: 60, color: '#ffffff'}})",
             [segment(1_000_000, 'One'), segment(2_000_000, 'Two')])
    ids = wait(lambda: [c['id'] for t in r.state()['tracks'] if t['kind'] == 'text' for c in t['clips']] or None, 5)
    r.s.run(SCROLL_TO, 0)
    r.s.run('window.__nuzky.store.getState().select(arguments[0])', ids)
    clock.sleep(0.3)
    b = r.s.run(RECT, ids[0])
    x, y = (b[0] + b[2]) / 2, (b[1] + b[3]) / 2
    r.s.run(PRESS, x, y, {}, [(x + 10, y), (x + 200, y)])
    widths = []
    for _ in range(6):
        r.s.run(TO_END)
        clock.sleep(0.15)
        widths.append(r.s.run(VIEW)['scrollWidth'])
    r.check('dragging two captions while scrolled to the end does not stretch the timeline', len(set(widths)) == 1, widths)
    r.s.run(SCROLL_TO, 0)
    r.s.run(RELEASE, x + 200, y, {})

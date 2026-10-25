import time as clock

from e2e.harness import FIXTURES, flow

HEADER_W = 140
# The scroller and its lanes, the track headers' left edges and the ruler labels' left edges on screen.
VIEW = """const ruler = document.querySelector('[data-ruler]'); const el = ruler.closest('.overflow-auto'); const r = el.getBoundingClientRect();
return {left: r.left, right: r.left + el.clientWidth, scrollLeft: el.scrollLeft, scrollWidth: el.scrollWidth, content: ruler.parentElement.parentElement.offsetWidth,
  headers: [...el.querySelectorAll('[role=listbox]')].map((l) => l.previousElementSibling.getBoundingClientRect().left),
  labels: [...ruler.querySelectorAll('span.tabular')].map((s) => s.getBoundingClientRect().left),
  clipEnd: el.querySelector('[data-clip-id]').getBoundingClientRect().right};"""
TO_END = "const el = document.querySelector('[data-ruler]').closest('.overflow-auto'); el.scrollLeft = el.scrollWidth"


@flow('timeline_scroll', 'Scrolling the timeline to its end again and again stops where the lanes end: the ruler does not '
      'stretch it, the last clip ends in the middle of the lanes, the track headers stay put and the ruler has its times up to the right edge')
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

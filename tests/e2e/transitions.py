import time

from e2e.harness import FIXTURES, flow, wait

# Where the cut between two main-track clips is: its x on the page and the top of the main track's lane.
CUT = """const [a, b] = [arguments[0], arguments[1]].map((id) => document.querySelector(`[data-clip-id='${id}']`).getBoundingClientRect());
const lane = document.querySelector(`[data-clip-id='${arguments[0]}']`).closest('[role=listbox]').getBoundingClientRect();
return {x: (a.right + b.left) / 2, top: lane.top};"""
# What is under a point of the page: a cut marker or a clip.
UNDER = """const el = document.elementFromPoint(arguments[0], arguments[1]);
return {marker: el?.closest('[data-cut]')?.dataset.cut ?? null, clip: el?.closest('[data-clip-id]')?.dataset.clipId ?? null};"""
HOVER = """const el = document.elementFromPoint(arguments[0], arguments[1]);
el.dispatchEvent(new PointerEvent('pointermove', {bubbles: true, pointerId: 1, clientX: arguments[0], clientY: arguments[1]}));"""
LEAVE = "document.querySelector('[data-cut]').closest('[role=listbox]').dispatchEvent(new PointerEvent('pointerleave', {pointerId: 1}))"
MARKER = """const m = document.querySelector(`[data-cut='${arguments[0]}']`);
if (!m) return null;
const r = m.getBoundingClientRect();
const top = document.elementFromPoint(r.left + r.width / 2, r.top + 1)?.closest('[data-cut]') === m;
return {opacity: getComputedStyle(m).opacity, top: r.top, size: r.width, uncovered: top};"""
# A pointer pressed on the "+" moving 1 px, as a click often does on a trackpad.
JITTER = """const m = document.querySelector(`[data-cut='${arguments[0]}']`).getBoundingClientRect();
document.elementFromPoint(m.left + 8, m.top + 8).dispatchEvent(new PointerEvent('pointermove',
  {bubbles: true, pointerId: 1, buttons: 1, clientX: m.left + 9, clientY: m.top + 8}));"""
TRANSITION = """return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips)
  .find((c) => c.id === arguments[0])?.transitionIn ?? null;"""
# Presses at a point, drags `arguments[2]` px along the timeline and lets go, as a trim of a clip edge.
DRAG = """const at = (type, dx) => new PointerEvent(type, {bubbles: true, button: 0, buttons: 1, pointerId: 1,
  clientX: arguments[0] + dx, clientY: arguments[1]});
document.elementFromPoint(arguments[0], arguments[1]).dispatchEvent(at('pointerdown', 0));
window.dispatchEvent(at('pointermove', arguments[2] / 2));
window.dispatchEvent(at('pointermove', arguments[2]));
window.dispatchEvent(new PointerEvent('pointerup', {bubbles: true, button: 0, pointerId: 1}));"""


@flow('transitions', 'An empty cut on the main track shows no marker and its clip edges trim along their whole height; '
      'the pointer near the cut brings up "+" on the top edge, which adds a Dissolve whose marker stays there')
def transitions(r):
    r.import_media(FIXTURES / 'talk.mp4', FIXTURES / 'wide.mp4')
    for name in ('talk.mp4', 'wide.mp4'):
        r.add_clip(name)
    first, second = r.track()
    time.sleep(0.5)
    cut = r.s.run(CUT, first['id'], second['id'])
    # The old marker sat across both trim edges, 15 to 35 px down the lane.
    under = [r.s.run(UNDER, cut['x'] + dx, cut['top'] + dy) for dx, dy in ((-3, 25), (3, 25), (-3, 50), (3, 50))]
    r.check('at rest nothing covers the trim edges of the clips at the cut',
            [u['clip'] for u in under] == [first['id'], second['id']] * 2 and not any(u['marker'] for u in under), under)
    marker = r.s.run(MARKER, second['id'])
    r.check('and the "+" of the empty cut is not shown', marker and marker['opacity'] == '0', marker)
    r.shot('rest')

    r.s.run(DRAG, cut['x'] - 3, cut['top'] + 25, -40)
    trimmed = wait(lambda: (c := r.track()) and c[0]['durationUs'] < first['durationUs'] and c, 5)
    r.check('dragging the first clip\'s edge at the cut trims it', trimmed, r.track())
    first, second = r.track()
    time.sleep(0.5)
    cut = r.s.run(CUT, first['id'], second['id'])

    r.s.run(HOVER, cut['x'] + 4, cut['top'] + 40)
    shown = wait(lambda: (m := r.s.run(MARKER, second['id'])) and m['opacity'] == '1' and m, 3)
    # The clips' top edge is 4 px into the lane; the marker reaches 6 px above it, 2 px above the lane, still under the ruler.
    r.check('the pointer near the cut brings up "+" on the clips\' top edge, not hidden by the ruler',
            shown and abs(shown['top'] - (cut['top'] - 2)) <= 1 and shown['size'] == 16 and shown['uncovered'], r.s.run(MARKER, second['id']))
    r.shot('hover')
    r.s.run(JITTER, second['id'])
    time.sleep(0.3)
    r.check('a press that moves 1 px keeps the "+" there', r.s.run(MARKER, second['id'])['opacity'] == '1', r.s.run(MARKER, second['id']))
    top = r.s.run(UNDER, cut['x'], cut['top'] + 5)
    r.check('the "+" is what the pointer clicks there', top['marker'] == second['id'], top)
    r.s.run("document.elementFromPoint(arguments[0], arguments[1]).closest('[data-cut]').click()", cut['x'], cut['top'] + 5)
    added = wait(lambda: r.s.run(TRANSITION, second['id']), 5)
    r.check('clicking it adds a 0.5 s Dissolve and selects it',
            added == {'kind': 'dissolve', 'durationUs': 500_000} and r.s.run('return window.__nuzky.store.getState().cut') == second['id'],
            added)

    r.s.run(LEAVE)
    time.sleep(0.4)
    marker = r.s.run(MARKER, second['id'])
    r.check('with the pointer gone the set marker stays on the top edge', marker and marker['opacity'] == '1', marker)
    under = [r.s.run(UNDER, cut['x'] + dx, cut['top'] + 25) for dx in (-3, 3)]
    r.check('and the clip edges under it still trim', [u['clip'] for u in under] == [first['id'], second['id']], under)
    r.shot('set')
    r.check('no error toast', not r.errors(), r.errors())

import time
from itertools import groupby

from e2e.agent import agent_project
from e2e.harness import Bridge, flow, wait

# Every frame: how many clips the project has on the main track and how many the timeline draws, and the pointer.
SAMPLE = """window.__pointerLog = [];
const tick = () => {
  if (!window.__pointerLog) return;
  const s = window.__nuzky.store.getState(), c = window.__nuzky.agentCursor.getState();
  const row = document.querySelector('[data-timeline] [data-track-id="main"]');
  const tip = document.querySelector('.agent-pointer').getBoundingClientRect();
  const clips = row ? [...row.querySelectorAll('[data-clip-id]')].map((e) => [e.getBoundingClientRect().left, e.dataset.clipId]).sort((a, b) => a[0] - b[0]) : [];
  window.__pointerLog.push({stored: s.snap.project.tracks.find((t) => t.id === 'main').clips.length,
    drawn: row ? row.querySelectorAll('[data-clip-id]').length : -1, first: clips[0]?.[1] ?? null, tracks: document.querySelectorAll('[data-timeline] [data-track-id]').length,
    t: performance.now(), held: !!c.held, shown: c.shown, glyph: c.glyph, label: c.step ? `${c.name} · ${c.step}` : c.name, band: !!document.querySelector('.agent-band'), x: tip.left, y: tip.top});
  requestAnimationFrame(tick);
};
requestAnimationFrame(tick);"""
LOG = 'const log = window.__pointerLog; window.__pointerLog = null; return log;'
# Where a time on the main track is on screen.
X_AT = """const row = document.querySelector('[data-timeline] [data-track-id="main"]').getBoundingClientRect();
return row.left + arguments[0] / 1e6 * window.__nuzky.store.getState().zoom;"""
LABEL = """const label = document.querySelector('[data-agent-cursor] .agent-label');
return label ? [...label.querySelectorAll('span span')].map((e) => e.textContent.trim()).join(' ') : null;"""


def act(r, bridge, run, request, edits, done):
    """Applies the agent's edits and returns what each frame showed until the timeline draws them (`done`, on a frame's
    entry) with nothing held back, and a moment after, as the pointer lets go."""
    r.s.run(SAMPLE)
    bridge.call('apply_edits', {'run_id': run, 'request_id': request, 'edits': edits})
    wait(lambda: r.s.run(f'const e = window.__pointerLog.at(-1); return !!e && !e.held && ({done})(e)'), 10, 0.1)
    time.sleep(0.5)
    return r.s.run(LOG)


def first(log, test):
    return next((i for i, e in enumerate(log) if test(e)), None)


@flow('agent_pointer', "While an AI agent edits, a pointer with its name acts each change out on the timeline: the timeline "
      "shows the change as the pointer gets there, it snips a split and a cut out, drags a clip to a new track, goes back "
      "to the AI button when the run ends, and Settings › AI turns it off", before=agent_project)
def agent_pointer(r):
    clip = r.track()[0]
    bridge = Bridge(r, r.saved_project(), client='claude-code')
    try:
        run = bridge.call('begin_run', {'label': 'Tighten the intro'})['run_id']
        r.check("the agent's pointer comes out with its name", wait(lambda: r.s.run(LABEL) == 'Claude Code', 5), r.s.run(LABEL))
        r.s.run(SAMPLE)
        time.sleep(4)
        r.shot('looking')
        log = r.s.run(LOG)
        lanes = r.s.run("const b = document.querySelector('[data-timeline]').getBoundingClientRect(); return [b.left, b.top, b.right, b.bottom];")
        clips = r.s.run("return [...document.querySelectorAll('[data-timeline] [data-clip-id]')].map((e) => { const b = e.getBoundingClientRect(); return [b.left, b.top, b.right, b.bottom]; })")
        over = [e for e in log if any(a <= e['x'] <= c and b <= e['y'] <= d for a, b, c, d in clips)]
        stops = [list(run) for moved, run in groupby(zip(log, log[1:]), lambda p: abs(p[0]['x'] - p[1]['x']) + abs(p[0]['y'] - p[1]['y']) > 0.1) if not moved]
        still = max((run[-1][1]['t'] - run[0][0]['t'] for run in stops), default=0)
        r.check('while the agent thinks the pointer looks through the clips', len(over) > len(log) / 2 and max(e['x'] for e in log) - min(e['x'] for e in log) > 60,
                {'over': len(over), 'frames': len(log), 'xs': [round(e['x']) for e in log[::20]]})
        r.check('it never stands still for long', still < 2600, {'longest_stop_ms': still})
        r.check('it stays over the timeline, presses nothing and holds nothing back',
                all(lanes[0] <= e['x'] <= lanes[2] and lanes[1] <= e['y'] <= lanes[3] and e['glyph'] == 'arrow' and not e['held'] and e['label'] == 'Claude Code' for e in log[len(log) // 4:]),
                next((e for e in log[len(log) // 4:] if not (lanes[0] <= e['x'] <= lanes[2] and lanes[1] <= e['y'] <= lanes[3]) or e['glyph'] != 'arrow' or e['held']), None))

        split_x = r.s.run(X_AT, 3_000_000)
        log = act(r, bridge, run, 'split', [{'type': 'splitClip', 'clipId': clip['id'], 'atUs': 3_000_000}], '(e) => e.drawn === 2')
        held = [e for e in log if e['stored'] == 2 and e['drawn'] == 1]
        r.check('the timeline keeps the clip whole while the pointer goes to the cut', len(held) >= 5, len(held))
        shown = first(log, lambda e: e['drawn'] == 2)
        r.check('the split shows under the pointer as it snips', shown is not None and abs(log[shown]['x'] - split_x) < 14 and log[shown]['glyph'] == 'scissors',
                shown is not None and {**log[shown], 'split_x': split_x})
        r.check('the label says what the agent does', any(e['label'] == 'Claude Code · Splitting talk.mp4' for e in log), {e['label'] for e in log})

        joint_x = r.s.run(X_AT, 5_000_000)
        log = act(r, bridge, run, 'cut', [{'type': 'rippleDeleteRanges', 'ranges': [{'startUs': 5_000_000, 'endUs': 6_000_000}]}], '(e) => e.drawn === 3')
        r.check('the piece is marked before it goes', any(e['band'] and e['drawn'] == 2 and e['stored'] == 3 for e in log), [e for e in log if e['band']][:3])
        shown = first(log, lambda e: e['drawn'] == 3)
        r.check('the gap closes as the pointer snips it out', shown is not None and log[shown]['glyph'] == 'scissors', shown is not None and log[shown])
        r.check('the pointer ends on the cut', abs(log[-1]['x'] - joint_x) < 14, {'pointer': log[-1]['x'], 'cut': joint_x})
        r.check('the label names the cut', any(e['label'] == 'Claude Code · Cutting out a piece' for e in log), {e['label'] for e in log})
        r.shot('cut')

        last = r.track()[-1]
        rows = len(r.state()['tracks'])
        log = act(r, bridge, run, 'move', [{'type': 'moveClip', 'clipId': last['id'], 'trackId': 'main', 'startUs': 8_000_000, 'newTrack': 'above'}],
                  f'(e) => e.tracks === {rows + 1}')
        grabbed = first(log, lambda e: e['glyph'] == 'grab')
        r.check('the pointer grabs the clip before it moves', grabbed is not None and log[grabbed]['tracks'] == rows, grabbed is not None and log[grabbed])
        r.check('the clip goes to its new track with the pointer', log[-1]['tracks'] == rows + 1 and any(e['label'] == 'Claude Code · Moving talk.mp4' for e in log), [log[-1], {e['label'] for e in log}])
        r.shot('moved')
        time.sleep(1)
        r.check('once done the label names only the agent', r.s.run(LABEL) == 'Claude Code', r.s.run(LABEL))

        main = r.track()
        moved_x = r.s.run(X_AT, main[-1]['startUs'] + main[-1]['durationUs'] // 2)
        log = act(r, bridge, run, 'reorder', [{'type': 'moveClip', 'clipId': main[-1]['id'], 'trackId': 'main', 'startUs': 0}], f"(e) => e.first === '{main[-1]['id']}'")
        grabbed = first(log, lambda e: e['glyph'] == 'grab')
        r.check('the short clip put before the long one on the main track is the one grabbed and carried there',
                grabbed is not None and log[grabbed]['first'] == main[0]['id'] and abs(log[grabbed]['x'] - moved_x) < 14
                and any(e['label'] == 'Claude Code · Moving talk.mp4' for e in log), grabbed is not None and {**log[grabbed], 'moved_x': moved_x})

        r.s.run('window.__nuzky.store.getState().togglePlay()')
        r.check('the video plays', wait(lambda: r.s.run('return window.__nuzky.store.getState().playing'), 5))
        main = r.track()
        log = act(r, bridge, run, 'playing', [{'type': 'splitClip', 'clipId': main[0]['id'], 'atUs': main[0]['startUs'] + 500_000}], f'(e) => e.drawn === {len(main) + 1}')
        r.s.run('window.__nuzky.store.getState().togglePlay()')
        r.check('while the video plays the timeline never waits for the pointer', not any(e['held'] for e in log), [e for e in log if e['held']][:2])

        # The run ends right after its last edit, while the pointer still acts it out; a press on the timeline then shows
        # the timeline as it is and starts no drag from the clips as drawn.
        wait(lambda: not r.s.run('return window.__nuzky.store.getState().playing'), 5)
        main = r.track()
        bridge.call('apply_edits', {'run_id': run, 'request_id': 'last', 'edits': [{'type': 'splitClip', 'clipId': main[-1]['id'], 'atUs': main[-1]['startUs'] + 400_000}]})
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
        held = wait(lambda: r.s.run('return !!window.__nuzky.agentCursor.getState().held && !window.__nuzky.store.getState().aiRun'), 3, 0.05)
        pressed = r.s.run("""const clip = document.querySelector('[data-timeline] [data-clip-id]');
            const e = new PointerEvent('pointerdown', {bubbles: true, cancelable: true, button: 0, pointerId: 1, clientX: clip.getBoundingClientRect().left + 5, clientY: clip.getBoundingClientRect().top + 5});
            clip.dispatchEvent(e);
            return {stopped: e.defaultPrevented, held: !!window.__nuzky.agentCursor.getState().held};""")
        r.check('a press on the timeline right after the run shows it as it is and starts nothing', held and pressed['stopped'] and not pressed['held'], {'held': held, **pressed})
        r.check('the pointer goes away when the run ends', wait(lambda: r.s.run(LABEL) is None, 5), r.s.run(LABEL))
        r.check('the timeline shows the whole run', len(r.track()) == 4 and not r.s.run('return window.__nuzky.agentCursor.getState().held'), r.track())
        r.s.run('document.activeElement?.blur()')
        r.key('z', ctrlKey=True)
        r.check('one undo still takes back the whole run', wait(lambda: len(r.track()) == 1, 10), r.track())

        r.s.run("window.__nuzky.openSettings('ai')")
        switch = "return document.querySelector('[role=switch][aria-label=\"Show the agent\\'s pointer\"]')"
        r.check('Settings › AI offers to turn the pointer off', wait(lambda: r.s.run(switch), 5))
        r.s.run(switch.replace('return ', '') + ".scrollIntoView({block: 'center'})")
        time.sleep(0.5)
        r.shot('setting')
        r.s.run(switch.replace('return ', '') + '.click()')
        r.check('the switch turns it off and remembers it', r.s.run("return localStorage.getItem('nuzky.agentPointer')") == '0')
        r.s.run('window.__nuzky.settings.setState({open: false})')

        run = bridge.call('begin_run', {'label': 'Again'})['run_id']
        clip = r.track()[0]
        log = act(r, bridge, run, 'off', [{'type': 'splitClip', 'clipId': clip['id'], 'atUs': 3_000_000}], '(e) => e.drawn === 2')
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
        r.check('turned off, no pointer shows and the timeline never waits', not any(e['shown'] or (e['stored'] == 2 and e['drawn'] == 1 and e['held']) for e in log), [e for e in log if e['shown'] or e['held']][:3])
        r.check('turned off, the change still shows', wait(lambda: r.s.run("return document.querySelectorAll('[data-timeline] [data-track-id=\"main\"] [data-clip-id]').length") == 2, 5))
        r.check('no error toast', not r.errors(), r.errors())
    finally:
        bridge.close()

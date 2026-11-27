"""Keep pitch on a Czech talking head sped up to 1.25x in the inspector: the voice in the export is as high as in the
recording, 1.25 times shorter, and every phrase starts where speed puts it, which is where captions and cuts land.
Turned off by keyboard, the voice rises with the speed again, and it stays off when the speed changes."""
import array, json, math, statistics, subprocess, time

from e2e.harness import FIXTURES, export, ffprobe, flow, press, wait
from e2e.split_screen import smallest

SOURCE = FIXTURES / 'reel-1.mp4'
RATE = 16_000
CONTENT = "return window.__nuzky.store.getState().snap.project.tracks[0].clips[0].content"
CHECKBOX = ("[...document.querySelectorAll('aside[aria-label=Inspector] label')]"
            ".find((l) => l.textContent === 'Keep pitch')?.querySelector('input')")
TYPE = """const el = document.querySelector(arguments[0]); el.focus();
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, arguments[1]);
el.dispatchEvent(new Event('input', {bubbles: true}));
el.dispatchEvent(new KeyboardEvent('keydown', {key: 'Enter', bubbles: true})); return true;"""


def decode(path):
    raw = subprocess.run(['ffmpeg', '-v', 'error', '-i', str(path), '-vn', '-ac', '1', '-ar', str(RATE), '-f', 'f32le', '-'],
                         capture_output=True, check=True).stdout
    return array.array('f', raw)


def levels(samples, window=RATE // 100):
    """dBFS of each 10 ms."""
    return [10 * math.log10(sum(x * x for x in samples[i:i + window]) / window + 1e-12)
            for i in range(0, len(samples) - window + 1, window)]


def pitch(samples):
    """Median pitch in Hz of the loudest 40 ms frames of speech, by autocorrelation between 70 and 400 Hz."""
    frame, found = RATE // 25, []
    loud = levels(samples, frame)
    for index in sorted(range(len(loud)), key=lambda i: loud[i])[-40:]:
        x = samples[index * frame:(index + 1) * frame]
        mean = sum(x) / frame
        x = [v - mean for v in x]
        corr = {lag: sum(a * b for a, b in zip(x, x[lag:])) for lag in range(RATE // 400 - 1, RATE // 70 + 2)}
        lag = max(range(RATE // 400, RATE // 70 + 1), key=corr.get)
        energy = sum(v * v for v in x)
        if corr[lag] < 0.4 * energy:
            continue
        # The peak between samples, through a parabola.
        a, b, c = corr[lag - 1], corr[lag], corr[lag + 1]
        found.append(RATE / (lag + 0.5 * (a - c) / (a - 2 * b + c)))
    return statistics.median(found), len(found)


def onsets(samples):
    """Seconds where speech starts after at least 200 ms of quiet."""
    loud = levels(samples)
    top = max(loud)
    found, quiet = [], 20
    for i, level in enumerate(loud):
        if level > top - 30:
            if quiet >= 20:
                found.append(i / 100)
            quiet = 0
        else:
            quiet += 1
    return found


@flow('speed', 'Keep pitch keeps a voice sped up to 1.25x as high as recorded in the export, with phrases where speed puts them; off, it rises again')
def speed(r):
    if not SOURCE.exists():
        raise RuntimeError('tmp-test/reel-1.mp4 is missing: run scripts/fixtures.sh')
    r.import_media(SOURCE)
    r.add_clip('reel-1.mp4')
    clip = r.track()[0]
    r.focus_clip(clip['id'])
    tab = "[...document.querySelectorAll('aside[aria-label=Inspector] [role=tab]')].find((t) => t.textContent === 'Speed')"
    if not wait(lambda: r.s.run(f'return !!{tab}'), 10):
        raise RuntimeError('the inspector has no Speed tab for the clip')
    r.s.run(f'{tab}.click()')
    checked = lambda: r.s.run(f'return {CHECKBOX}?.checked ?? null')
    r.check('Keep pitch is not offered at 1x', wait(lambda: r.s.run('return !!document.querySelector(\'input[aria-label="Speed value"]\')'), 5)
            and checked() is None, checked())
    r.s.run(TYPE, 'input[aria-label="Speed value"]', '1.25')
    content = lambda: r.s.run(CONTENT)
    r.check('typing 1.25 speeds the clip up with Keep pitch on',
            wait(lambda: content()['speed'] == 1.25 and content().get('keepPitch') is True, 5), content())
    r.check('the inspector shows Keep pitch checked', wait(lambda: checked() is True, 5), checked())
    r.shot('inspector-keep-pitch')
    saved = lambda: next((c['content'].get('keepPitch') for t in json.loads(r.saved_project().read_text())['tracks']
                          for c in t['clips']), None)
    r.check('the project file on disk has Keep pitch on', wait(lambda: saved() is True, 10), saved())
    kept = export(r, 'kept.mp4')

    # What a listener gets: the voice's pitch, the length, and where each phrase starts.
    source, sped = decode(SOURCE), decode(kept)
    (recorded, frames), (heard, heard_frames) = pitch(source), pitch(sped)
    r.check('the voice in the export is as high as recorded, within 3 %', abs(heard / recorded - 1) < 0.03,
            f'{recorded:.1f} Hz recorded, {heard:.1f} Hz exported ({frames} and {heard_frames} voiced frames)')
    length, expected = len(sped) / RATE, len(source) / RATE / 1.25
    r.check('the export is 1.25 times shorter', abs(length - expected) < 0.06, f'{length:.3f} s, expected {expected:.3f} s')
    starts, placed = [t / 1.25 for t in onsets(source)], onsets(sped)
    worst = max(min(abs(t - p) for p in placed) for t in starts)
    r.check('every phrase starts where speed puts it, within 25 ms', len(starts) == len(placed) and worst <= 0.025,
            f'{len(starts)} phrases recorded, {len(placed)} exported, furthest {worst * 1000:.0f} ms off')

    r.s.run(f'{CHECKBOX}.focus()')
    press('space')
    r.check('Space turns Keep pitch off', wait(lambda: content().get('keepPitch', False) is False, 5), content())
    preset = ("[...document.querySelectorAll('aside[aria-label=Inspector] button')]"
              ".find((b) => b.textContent === '1.5x')")
    r.s.run(f'{preset}.click()')
    r.check('another speed leaves Keep pitch off', wait(lambda: content()['speed'] == 1.5, 5)
            and content().get('keepPitch', False) is False and checked() is False, content())
    raised = export(r, 'raised.mp4')
    higher, _ = pitch(decode(raised))
    r.check('without Keep pitch the voice rises with the speed', abs(higher / recorded - 1.5) < 0.05,
            f'{recorded:.1f} Hz recorded, {higher:.1f} Hz at 1.5x')

    r.focus_clip(clip['id'])
    r.key('z', ctrlKey=True)
    r.key('z', ctrlKey=True)
    r.check('two Ctrl+Z bring back 1.25x with Keep pitch on',
            wait(lambda: content()['speed'] == 1.25 and content().get('keepPitch') is True, 5), content())
    r.check('no error toast', not r.errors(), r.errors())


GROUP = "aside[aria-label=Inspector] [role=group][aria-label='Speed presets']"


@flow('multi_speed', 'Two selected talking-head chunks share a Speed section: a typed 2x and a preset change both, back to back, '
      'as one undo step, and the export is as long as both at that speed')
def multi_speed(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    r.add_clip('talk.mp4')
    first, second = (c['id'] for c in r.track())
    source_us = r.track()[0]['durationUs']
    r.s.call("window.__nuzky.store.getState().edit({type: 'updateClip', clipId: arguments[0], speed: 1.5})", first)
    main = lambda: r.s.run("return window.__nuzky.store.getState().snap.project.tracks[0].clips.map((c) => ({speed: c.content.speed, startUs: c.startUs, durationUs: c.durationUs}))")
    wait(lambda: main()[0]['speed'] == 1.5, 5)
    r.s.run('window.__nuzky.store.getState().select(arguments[0])', [first, second])

    field = 'aside[aria-label=Inspector] input[aria-label="Speed value"]'
    r.check('two selected video clips show Speed in the inspector', wait(lambda: r.s.run(f"return !!document.querySelector('{field}')"), 5))
    r.check('their different speeds show a dash and no preset',
            r.s.run(f"return document.querySelector('{field}')?.value ?? null") == '—'
            and r.s.run(f'return document.querySelectorAll("{GROUP} [aria-pressed=true]").length') == 0)
    r.s.run(f"document.querySelector('{field}').scrollIntoView({{block: 'center'}})")
    smallest(r, 'multi-speed-mixed')

    speeds = lambda: [c['speed'] for c in main()]
    r.s.run(TYPE, field, '2')
    r.check('typing 2 sets both clips to 2x', wait(lambda: speeds() == [2, 2], 5), speeds())
    clips = main()
    half = source_us / 2
    r.check('each lasts half its source and they stay back to back',
            all(abs(c['durationUs'] - half) < 40_000 for c in clips) and clips[1]['startUs'] == clips[0]['startUs'] + clips[0]['durationUs'],
            [(c['startUs'], c['durationUs']) for c in clips])
    r.check('the 2x preset shows chosen', wait(lambda: r.s.run(f'return document.querySelector("{GROUP} [data-seg=\'2\']").getAttribute("aria-pressed")') == 'true', 5))
    r.s.run(f"document.querySelector('{field}').scrollIntoView({{block: 'center'}})")
    time.sleep(1)
    r.shot('multi-speed-2x')

    r.s.run(f'document.querySelector("{GROUP} [data-seg=\'0.5\']").click()')
    r.check('the 0.5x preset slows both down', wait(lambda: speeds() == [0.5, 0.5], 5), speeds())
    r.s.call('window.__nuzky.store.getState().undo()')
    r.check('one Undo brings both back to 2x', wait(lambda: speeds() == [2, 2], 5), speeds())

    saved = lambda: [c['content']['speed'] for t in json.loads(r.saved_project().read_text())['tracks'] for c in t['clips']]
    r.check('the project file on disk has both at 2x', wait(lambda: saved() == [2, 2], 10), saved())
    length, _ = ffprobe(export(r, 'both-2x.mp4'))
    r.check('the export is as long as both chunks at 2x', abs(length - source_us / 1e6) < 0.1, f'{length:.3f} s, source {source_us / 1e6:.3f} s')

    r.s.call("window.__nuzky.store.getState().edit({type: 'updateClip', clipId: arguments[0], ramp: [{at: 0, speed: 1}, {at: 1, speed: 3}]})", first)
    r.check('with a clip on a speed ramp selected, the shared Speed does not show',
            wait(lambda: r.s.run(f"return !document.querySelector('{field}')"), 5)
            and r.s.run("return document.querySelector('aside[aria-label=Inspector]').textContent.includes('2 clips selected')"))
    r.check('no error toast', not r.errors(), r.errors())

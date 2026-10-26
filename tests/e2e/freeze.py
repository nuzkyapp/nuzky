"""Shift+R holds the frame at the playhead of a talking head for 3 s: the clip splits there, the still goes in between
with the title over the rest moving along, the preview and the export show the very frame the video showed there for
the whole still, the export is silent over it, dragging its right edge stretches it, and one undo takes it back."""
import array, math, subprocess, time

from e2e.harness import FIXTURES, changed_share, export, flow, preview_crop, preview_rect, wait

RATE = 48_000
CONTENT = """return window.__nuzky.store.getState().snap.project.tracks[0].clips.map((c) => ({id: c.id, startUs: c.startUs,
  durationUs: c.durationUs, speed: c.content.speed, sourceInUs: c.content.sourceInUs, volume: c.content.volume}))"""
TITLE = "return window.__nuzky.store.getState().snap.project.tracks.find((t) => t.kind === 'text')?.clips[0].startUs ?? null"
SELECTION = 'return window.__nuzky.store.getState().selection'
TABS = "return [...document.querySelectorAll('aside[aria-label=Inspector] [role=tab]')].map((t) => t.textContent)"
# Presses a clip just inside its right edge, drags that edge `arguments[1]` px along the timeline and lets go.
STRETCH = """const el = document.querySelector(`[data-clip-id='${arguments[0]}']`);
const b = el.getBoundingClientRect();
const at = (type, dx) => new PointerEvent(type, {bubbles: true, button: 0, pointerId: 1,
  clientX: b.right - 3 + dx, clientY: b.top + b.height / 2});
el.dispatchEvent(at('pointerdown', 0));
window.dispatchEvent(at('pointermove', arguments[1]));
window.dispatchEvent(at('pointerup', arguments[1]));"""


def preview_at(r, us, name):
    r.seek(us)
    time.sleep(1.2)
    r.shot(name)
    return preview_crop(r.work / f'{name}.png', preview_rect(r))


def exported_frame(r, path, seconds, name):
    out = r.work / f'{name}.png'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-ss', f'{seconds:.3f}', '-i', str(path), '-frames:v', '1', str(out)], check=True)
    return out


def loudest_db(path, start, end):
    """The loudest 10 ms of the export's sound between `start` and `end` seconds, in dBFS."""
    raw = subprocess.run(['ffmpeg', '-v', 'error', '-ss', f'{start:.3f}', '-t', f'{end - start:.3f}', '-i', str(path), '-vn',
                          '-ac', '1', '-ar', str(RATE), '-f', 'f32le', '-'], capture_output=True, check=True).stdout
    samples, window = array.array('f', raw), RATE // 100
    levels = [sum(x * x for x in samples[i:i + window]) / window for i in range(0, len(samples) - window + 1, window)]
    return 10 * math.log10(max(levels, default=0) + 1e-12)


@flow('freeze', 'Shift+R holds the frame at the playhead for 3 s between the halves of its clip, silent and the same frame in '
      'the preview and the export, stretched by dragging its edge and taken back by one undo')
def freeze(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    clip = r.track()[0]
    r.s.call("window.__nuzky.store.getState().edit({type: 'addText', startUs: 3000000, text: 'Later',"
             " style: {fontSize: 80, color: '#ffffff'}})")
    r.check('a title sits over the clip at 3 s', wait(lambda: r.s.run(TITLE) == 3_000_000, 5), r.s.run(TITLE))
    shown = preview_at(r, 1_500_000, 'before')
    r.focus_clip(clip['id'])
    r.key('R', shiftKey=True)
    clips = wait(lambda: (c := r.s.run(CONTENT)) and len(c) == 3 and c, 10)
    r.check('Shift+R splits the clip at 1.5 s and puts a 3 s still of that frame in between, silent',
            clips and [(c['startUs'], c['durationUs'], c['speed'], c['sourceInUs'], c['volume']) for c in clips[:2]]
            == [(0, 1_500_000, 1, 0, 1), (1_500_000, 3_000_000, 0, 1_500_000, 0)]
            and (clips[2]['startUs'], clips[2]['sourceInUs'], clips[2]['speed']) == (4_500_000, 1_500_000, 1), clips)
    if not clips:
        return
    still = clips[1]['id']
    r.check('the title over the rest of the clip moves on with its picture', r.s.run(TITLE) == 6_000_000, r.s.run(TITLE))
    r.check('the still is selected', r.s.run(SELECTION) == [still], r.s.run(SELECTION))
    tabs = wait(lambda: r.s.run(TABS), 5)
    r.check('its inspector has no Speed or Audio tab', tabs and 'Speed' not in tabs and 'Audio' not in tabs, tabs)
    r.shot('frozen')

    # The selected still draws its handles over the preview; without a selection the preview is the video alone.
    r.s.run('window.__nuzky.store.getState().select([])')
    middle, end, after = (preview_at(r, us, name) for us, name in ((3_000_000, 'still-middle'), (4_450_000, 'still-end'),
                                                                     (5_500_000, 'played-on')))
    r.check('the preview shows the frame from before the freeze for the whole still',
            changed_share(middle, shown) < 0.002 and changed_share(end, shown) < 0.002,
            [changed_share(middle, shown), changed_share(end, shown)])
    r.check('after the still the video moves on', changed_share(after, shown) > 0.003, changed_share(after, shown))

    r.s.run('window.__nuzky.store.getState().setZoom(100)')
    time.sleep(0.5)
    r.s.run(STRETCH, still, 100)
    stretched = wait(lambda: (c := r.s.run(CONTENT)) and c[1]['durationUs'] > 3_900_000 and c, 5)
    r.check("dragging the still's right edge 1 s out stretches it, still the same frame, and moves the rest along",
            stretched and abs(stretched[1]['durationUs'] - 4_000_000) <= 34_000 and stretched[1]['sourceInUs'] == 1_500_000
            and stretched[2]['startUs'] == 1_500_000 + stretched[1]['durationUs'], stretched)
    r.s.run('window.__nuzky.store.getState().setZoom(40)')
    time.sleep(1)
    r.shot('stretched')

    video = export(r, 'freeze.mp4')
    end_s = (1_500_000 + (stretched or clips)[1]['durationUs']) / 1e6
    frames = [exported_frame(r, video, t, f'export-{i}') for i, t in enumerate((1.4, 1.6, 3.0, end_s - 0.1, end_s))]
    still_frames = [changed_share(frames[1], f) for f in frames[2:]]
    r.check('the export holds one frame from the start of the still to the first frame of the rest, which plays on from it', max(still_frames) < 0.002, still_frames)
    r.check('the frame before the still is another one, so the check can tell them apart', changed_share(frames[0], frames[1]) > 0.002,
            changed_share(frames[0], frames[1]))
    around, over = loudest_db(video, 0.0, 1.45), loudest_db(video, 1.55, end_s - 0.05)
    r.check('the export is silent over the still, while the speech around it is heard', over < -70 and around > -30,
            f'{over:.1f} dBFS over the still, {around:.1f} dBFS before it')

    r.focus_clip(still)
    r.key('z', ctrlKey=True)
    r.key('z', ctrlKey=True)
    back = wait(lambda: (c := r.s.run(CONTENT)) and len(c) == 1 and c, 5)
    r.check('two undos take back the stretch and the freeze, the title back at 3 s',
            back and back[0]['durationUs'] == clip['durationUs'] and r.s.run(TITLE) == 3_000_000, [back, r.s.run(TITLE)])
    r.check('no error toast', not r.errors(), r.errors())

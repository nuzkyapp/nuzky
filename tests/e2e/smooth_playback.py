import json, subprocess, time

from e2e.harness import flow, wait

EDIT = 'return window.__nuzky.store.getState().edit(arguments[0]).then((s) => !!s)'
# Every frame the preview paints, with the display frame it was painted on.
RECORD = """const c = document.querySelector('canvas[data-preview-frame]');
window.__paints = {paints: [], ticks: [], on: true};
window.__paintsSeen = new MutationObserver(() => __paints.paints.push([performance.now(), +c.dataset.us]));
__paintsSeen.observe(c, {attributes: true, attributeFilter: ['data-n']});
const tick = (t) => { __paints.ticks.push(t); if (__paints.on) requestAnimationFrame(tick); };
requestAnimationFrame(tick);"""
STOP = "__paints.on = false; __paintsSeen.disconnect(); return JSON.stringify(__paints)"


@flow('smooth_playback', 'A phone video cut into many pieces under an adjustment layer plays in the preview with every frame '
      'painted once and each one on screen equally long: two refreshes of a 60 Hz display for 30 fps, however unevenly '
      'the frames took to draw')
def smooth_playback(r):
    path = r.work / 'talk.mp4'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=s=1080x1920:r=30', '-f', 'lavfi', '-i',
                    'anullsrc=r=48000:cl=stereo', '-t', '40', '-c:v', 'libx264', '-g', '15', '-pix_fmt', 'yuv420p',
                    '-c:a', 'aac', '-shortest', str(path)], check=True)
    r.import_media(path)
    r.add_clip(path.name)
    # Kept 0.8 s, cut 0.4 s, as cutting pauses leaves a talk: a piece starts every 0.8 s.
    ranges = [{'startUs': 800_000 + k * 1_200_000, 'endUs': 1_200_000 + k * 1_200_000} for k in range(32)]
    r.s.run(EDIT, {'type': 'rippleDeleteRanges', 'ranges': ranges, 'keepTrackIds': None})
    wait(lambda: len(r.track()) == 33, 10)
    adjust = {'exposure': 0.1, 'tint': 0, 'highlights': 0.4, 'shadows': -0.15, 'fade': 0, 'brightness': 0, 'contrast': 0.2,
              'saturation': 0.25, 'temperature': -0.1, 'vignette': 0.2}
    r.s.run(EDIT, {'type': 'addAdjustment', 'startUs': 0, 'durationUs': 26_000_000, 'adjust': adjust})
    r.s.run('window.__nuzky.store.getState().select([]); document.activeElement?.blur()')
    r.seek(0)
    time.sleep(1.5)

    # Muted, the sound does not play on this computer's speakers, and its clock still drives the picture.
    r.s.run("""document.querySelector('button[aria-label^="Mute the preview"]')?.click()""")
    time.sleep(0.3)
    r.s.run(RECORD)
    r.s.run('window.__nuzky.store.getState().togglePlay()')
    time.sleep(20)
    r.s.run('window.__nuzky.store.getState().togglePlay()')
    data = json.loads(r.s.run(STOP))
    ticks, paints = data['ticks'], data['paints']
    refresh = sorted(b - a for a, b in zip(ticks, ticks[1:]))[len(ticks) // 2]
    frame = 1e6 / 30
    index = [round(p[1] / frame) for p in paints]
    skipped = sum(b - a - 1 for a, b in zip(index, index[1:]) if b > a + 1)
    repeated = sum(1 for a, b in zip(index, index[1:]) if b == a)
    gaps = [round((b[0] - a[0]) / refresh) for a, b in zip(paints, paints[1:])]
    usual = max(set(gaps), key=gaps.count)
    # The page missed a refresh, busy with something else, such as the timeline jumping to its next page.
    missed = [b for a, b in zip(ticks, ticks[1:]) if b - a > refresh * 1.5]
    near_missed = lambda i: any(paints[i][0] - refresh <= t <= paints[i + 1][0] + refresh * 2.5 for t in missed)
    # The first frames wait for the sound to start, which takes a few refreshes, and on Bluetooth a fifth of a second.
    starting = lambda i: i < 10
    uneven = [(round(paints[i + 1][0] - paints[0][0]), g) for i, g in enumerate(gaps) if g != usual and not near_missed(i) and not starting(i)]
    # The display and the sound card keep their own time, so now and then one frame has to stay a refresh longer, or
    # shorter, for the picture to stay with the sound. A frame shown a refresh early and the next a refresh late is a
    # stutter nothing called for.
    needed = abs(sum(g - usual for _, g in uneven))
    (r.work / 'paints.json').write_text(json.dumps(data))
    r.check('20 s of playing paints every frame of the video once', len(paints) > 550 and skipped <= 1 and repeated == 0,
            {'paints': len(paints), 'skipped': skipped, 'repeated': repeated})
    r.check('frames are on screen 1/30 s each', abs(usual * refresh - frame / 1000) < 4, {'refresh_ms': refresh, 'usual': usual})
    # Before frames were shown when due, about one frame in sixteen stuttered.
    r.check('each frame stays on screen as long as the one before, but to keep up with the sound', len(uneven) - needed == 0,
            {'refreshes per frame': {g: gaps.count(g) for g in sorted(set(gaps))}, 'missed refreshes': len(missed),
             'uneven at ms': uneven, 'needed': needed})
    r.shot('played')

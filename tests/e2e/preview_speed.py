import json, subprocess, time

from e2e.harness import flow, wait

# How fast the playhead moves for 1.5 s of the clock, in timeline seconds per second.
PACE = """(async () => { const st = window.__nuzky.store;
const start = [performance.now(), st.getState().timeUs];
await new Promise((d) => setTimeout(d, 1500));
const end = [performance.now(), st.getState().timeUs];
return JSON.stringify((end[1] - start[1]) / 1e3 / (end[0] - start[0])); })()"""
PILL = "const b = document.querySelector('[data-speed]'); return b && {text: b.textContent.trim(), label: b.getAttribute('aria-label')}"
PLAYING = 'return window.__nuzky.store.getState().playing'
TIME = 'return window.__nuzky.store.getState().timeUs'
PROJECT = """const st = window.__nuzky.store.getState();
return {revision: st.snap.revision, canUndo: st.snap.canUndo,
  speeds: st.snap.project.tracks.flatMap((t) => t.clips).map((c) => c.content.speed ?? null)}"""
MENU = "return !!document.querySelector('[role=menu][aria-label=\"Preview speed\"]')"
OPEN = "document.querySelector('[data-speed]').click()"
PICK = """const item = [...document.querySelectorAll('[role=menu][aria-label="Preview speed"] [role=menuitemradio]')].find((i) => i.textContent.trim() === arguments[0]);
if (!item) return false; item.click(); return true"""
CALM = 'window.__nuzky.store.getState().select([]); document.activeElement?.blur()'


@flow('preview_speed', 'The preview plays faster or backwards on this computer only: L plays and speeds up through 1.5x and '
      '2x, K stops, J plays backwards, the pill under the preview shows the speed and chooses it, and none of it changes '
      'the project, its undo or the clips')
def preview_speed(r):
    path = r.work / 'tone.mp4'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=s=1080x1920:r=30', '-f', 'lavfi', '-i',
                    'sine=frequency=440:sample_rate=48000', '-t', '40', '-c:v', 'libx264', '-g', '30', '-pix_fmt', 'yuv420p',
                    '-c:a', 'aac', '-shortest', str(path)], check=True)
    r.import_media(path)
    r.add_clip(path.name)
    r.s.run(CALM)
    before = r.s.run(PROJECT)
    pace = lambda: json.loads(r.s.call(PACE)['value'])
    pill = lambda: r.s.run(PILL)

    def choose(speed, shot=None):
        r.s.run(OPEN)
        if shot:
            wait(lambda: r.s.run(MENU), 5)
            time.sleep(0.4)
            r.shot(shot)
        return wait(lambda: r.s.run(PICK, speed), 5)

    r.check('the pill shows 1x before anything plays', (pill() or {}).get('text') == '1x', pill())
    r.seek(2_000_000)
    r.key('l')
    r.check('L plays', wait(lambda: r.s.run(PLAYING), 5))
    one = pace()
    r.check('at 1x the playhead moves a second per second', 0.85 < one < 1.15, one)
    r.key('l')
    wait(lambda: (pill() or {}).get('text') == '1.5x', 5)
    one_half = pace()
    r.check('a second L plays at 1.5x', 1.3 < one_half < 1.7 and pill()['text'] == '1.5x', (one_half, pill()))
    r.key('l')
    wait(lambda: (pill() or {}).get('text') == '2x', 5)
    two = pace()
    r.check('a third L plays at 2x and the pill says so', 1.75 < two < 2.25 and pill()['text'] == '2x', (two, pill()))
    r.shot('playing-at-2x')
    # The voiceover and a reel play at 1x with Play: Play while faster goes on at 1x and keeps the speed Play uses.
    r.s.run('window.__nuzky.api.play()')
    wait(lambda: (pill() or {}).get('text') == '1x', 5)
    time.sleep(0.3)
    played = pace()
    r.check('Play while playing faster goes on at 1x', 0.85 < played < 1.15, played)

    r.key('k')
    r.check('K stops', wait(lambda: not r.s.run(PLAYING), 5))
    r.check('stopped, the pill keeps the speed Play uses', pill()['text'] == '2x', pill())

    r.key('j')
    r.check('J plays', wait(lambda: r.s.run(PLAYING), 5))
    back = pace()
    r.check('J plays backwards at 1x', -1.15 < back < -0.85 and 'backwards' in pill()['label'], (back, pill()))
    r.shot('playing-backwards')
    r.key('k')
    wait(lambda: not r.s.run(PLAYING), 5)

    r.seek(0)
    wait(lambda: r.s.run(TIME) == 0, 5)
    r.key('j')
    near_end = wait(lambda: r.s.run(PLAYING) and r.s.run(TIME) > 30_000_000, 5)
    from_end = pace()
    r.check('J at the start plays backwards from the end', near_end and -1.15 < from_end < -0.85, (from_end, r.s.run(TIME)))
    r.key('k')
    wait(lambda: not r.s.run(PLAYING), 5)
    r.seek(2_000_000)

    r.check('the menu chooses 1.5x', choose('1.5x', 'speed-menu'))
    r.s.run(CALM)
    r.key(' ')
    r.check('Space plays', wait(lambda: r.s.run(PLAYING), 5))
    space = pace()
    r.check('Space plays at the speed chosen', 1.3 < space < 1.7, space)
    r.check('the menu chooses 1x while playing', choose('1x'))
    wait(lambda: (pill() or {}).get('text') == '1x', 5)
    normal = pace()
    r.check('and playback goes on at 1x at once', 0.85 < normal < 1.15, normal)
    r.key(' ')
    wait(lambda: not r.s.run(PLAYING), 5)

    after = r.s.run(PROJECT)
    r.check('the speed changed nothing in the project, its undo or the clips', after == before, (before, after))

"""Beat cuts in the music clip's inspector: Cut to beat in Project status finds the song's beat as a job, the clip shows
its beats, bars and drop, three rhythms say what each would cut, and cutting is one undo step that agents' cut_to_beats
plans the same way."""
import json, subprocess, time

import numpy as np
from PIL import Image

from e2e.harness import Bridge, flow, wait
from e2e.layout import resize

SONG = 'nuzky:dancing'
SIZES = ((1440, 900, ''), (1024, 640, '-1024'))
SECTION = "[...document.querySelectorAll('aside[aria-label=Inspector] h3')].find((h) => h.textContent === 'Beats')?.closest('section')"
BEATS = f"""const s = {SECTION};
if (!s) return null;
const rows = [...s.querySelectorAll('[data-rhythm]')].map((b) => ({{rhythm: b.dataset.rhythm, pressed: b.getAttribute('aria-pressed') === 'true',
  disabled: b.getAttribute('aria-disabled') === 'true', text: b.innerText.replace(/\\s+/g, ' ').trim()}}));
const cut = s.querySelector('[data-cut-to-beat]');
return {{text: s.innerText, rows, cut: cut ? {{disabled: cut.getAttribute('aria-disabled') === 'true', text: cut.innerText.trim()}} : null}};"""
TICKS = """const clip = document.querySelector(`[data-clip-id='${arguments[0]}']`);
const svg = clip?.querySelector('[data-beat-ticks]');
const box = clip?.getBoundingClientRect();
return {ticks: svg ? [...svg.querySelectorAll('path')].map((p) => (p.getAttribute('d').match(/M/g) || []).length) : [],
        drops: [...(clip?.querySelectorAll('[data-beat-drop]') ?? [])].map((d) => d.innerText.trim()),
        box: box && [box.left, box.top, box.width, box.height], width: window.innerWidth};"""
PROJECT = 'return window.__nuzky.store.getState().snap.project'
STATUS_BUTTON = """const li = [...document.querySelectorAll('ul[aria-label="Project status"] > li')]
  .find((li) => li.querySelector('span').firstChild.textContent === 'Music');
const b = li?.querySelector('button[aria-label="Cut to beat"]');
if (b) b.click();
return !!b;"""


def make_clips(r, count=6, seconds=8):
    """Short phone-shaped videos with their own sound, each a different colour."""
    for n in range(count):
        subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', f'testsrc2=s=1080x1920:r=30:d={seconds},hue=h={n * 60}',
                        '-f', 'lavfi', '-i', f'sine=frequency={300 + 60 * n}:duration={seconds}', '-shortest',
                        '-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', '-c:a', 'aac', str(r.work / f'IMG_{2041 + n}.mp4')], check=True)


def clips_of(r, count=6):
    return [r.work / f'IMG_{2041 + n}.mp4' for n in range(count)]


def music_clip(project):
    return next(c for t in project['tracks'] if t['kind'] == 'audio' for c in t['clips'])


def main(project):
    return sorted(next(t for t in project['tracks'] if t['id'] == 'main')['clips'], key=lambda c: c['startUs'])


def beats(r):
    return r.s.run(BEATS)


def ticks(r, clip_id):
    return r.s.run(TICKS, clip_id)


def tick_columns(r, name, clip_id, beat_from):
    """Columns with a white tick in the music clip's top 12 px: (in the intro before the beat, after it). The drop's flag
    chip and the fade dots in the clip's corners are left out."""
    r.shot(name)
    t = ticks(r, clip_id)
    image = np.asarray(Image.open(r.work / f'{name}.png').convert('RGB')).astype(int)
    scale = image.shape[1] / t['width']
    left, top, width, _ = (v * scale for v in t['box'])
    band = image[round(top + 2 * scale):round(top + 12 * scale)]
    white = ((band > 140).all(-1) & (band.max(-1) - band.min(-1) < 45)).any(0)
    beat = left + beat_from * width
    corner, chip = 18 * scale, 140 * scale
    intro = white[round(left + corner):round(beat - 4 * scale)]
    rest = white[round(beat + chip):round(min(left + width, image.shape[1]) - corner)]
    return int(intro.sum()), int(rest.sum())


def plans(r, clip_id):
    return json.loads(r.s.call('window.__nuzky.api.beatPlans(arguments[0], window.__nuzky.store.getState().snap.sessionEpoch).then(JSON.stringify)', clip_id)['value'])


def fit(r):
    """The whole video and song on screen."""
    r.s.run("document.querySelector('button[aria-label=\"Fit timeline to view\"]').click()")
    time.sleep(0.5)


def reveal(r):
    """Project status, then its Cut to beat, which selects the song and shows its Beats section."""
    r.s.run('window.__nuzky.store.getState().select([])')
    return wait(lambda: r.s.run(STATUS_BUTTON), 10)


def in_view(r, selector):
    """The element lies whole inside the inspector's scrolled area."""
    return r.s.run(f"""const e = document.querySelector('{selector}'); const p = e?.closest('[role=tabpanel]');
        if (!e || !p) return null; const a = e.getBoundingClientRect(), b = p.getBoundingClientRect();
        return a.top >= b.top && a.bottom <= b.bottom && a.left >= b.left && a.right <= b.right;""")


def shoot(r, name, cut_in_view=False):
    """The state at 1440 x 900 and at the smallest window, 1024 x 640, the Beats section in view."""
    for width, height, suffix in SIZES:
        resize(r, width, height)
        # As Cut to beat shows the section: from its top, then the Cut row into view.
        r.s.run(f'{SECTION}?.scrollIntoView({{block: "start"}}); document.querySelector("[data-cut-to-beat]")?.scrollIntoView({{block: "nearest"}})')
        time.sleep(0.6)
        r.shot(name + suffix)
        if cut_in_view:
            r.check(f'with the section in view at {width} x {height} the Cut button shows whole', in_view(r, '[data-cut-to-beat]'))
    resize(r, 1440, 900)


def setup(r):
    make_clips(r)


@flow('beat-cuts', 'Cut to beat in Project status finds the beat of the song, draws its beats, bars and drop on its clip, '
      'offers Calm, Builds to the drop and Fast as an agent plans them, cuts the main track on those frames as one undo '
      'step with the title in place and the videos muted, replaces that cut with another rhythm and one Undo restores all',
      before=setup)
def beat_cuts(r):
    resize(r, 1440, 900)
    r.import_media(*clips_of(r))
    for path in clips_of(r):
        r.add_clip(path.name)
    r.s.call("window.__nuzky.store.getState().edit({type: 'addText', startUs: 1000000, text: 'Lisbon trip', style: {fontSize: 80, color: '#ffffff'}})")
    r.s.call(f"window.__nuzky.api.soundAdd('{SONG}', 0, window.__nuzky.store.getState().snap.sessionEpoch).then((s) => window.__nuzky.store.getState().setSnap(s))")
    before = wait(lambda: (p := r.s.run(PROJECT)) and any(t['kind'] == 'audio' and t['clips'] for t in p['tracks']) and p, 20)
    song = music_clip(before)
    title = next(c for t in before['tracks'] if t['kind'] == 'text' for c in t['clips'])
    original = [(c['id'], c['startUs'], c['durationUs']) for c in main(before)]
    r.check('six videos, a title and Dancing on a music track', len(original) == 6 and song and title, [len(original), title and title['id']])

    r.s.run('window.__nuzky.store.getState().select([])')
    fit(r)
    r.check('Music in Project status offers Cut to beat', reveal(r))
    r.check('Cut to beat selects the song', wait(lambda: r.s.run('return window.__nuzky.store.getState().selection') == [song['id']], 5),
            r.s.run('return window.__nuzky.store.getState().selection'))
    found = wait(lambda: (b := beats(r)) and len(b['rows']) == 3 and b, 60)
    r.check('the Beats section finds the beat and offers three rhythms', found, beats(r))
    job = next((j for j in r.state()['jobs'] if j['kind'] == 'beats'), None)
    r.check('finding the beat ran as a job', job and job['status'] == 'done', r.state()['jobs'])
    header = found and found['text'].split('\n')[1]
    r.check('the header says the tempo and where the beat comes in', header and ' BPM · beat from 00:' in header, header)
    focused = r.s.run('return document.activeElement?.dataset.rhythm ?? null')
    r.check('the keyboard lands on the chosen rhythm', focused == 'builds', focused)
    r.check('the Cut button is in view without scrolling', in_view(r, '[data-cut-to-beat]'), found)

    planned = plans(r, song['id'])
    by = {p['rhythm']: p for p in planned['rhythms']}
    r.check('every rhythm has a plan', all(not p['error'] and p['clips'] > 1 for p in by.values()), planned['rhythms'])
    r.check('Builds to the drop says why Dancing cuts it the same as Calm',
            by['builds']['sameAs'] == 'calm' and 'drops where its beat starts' in found['text'] and 'same as Calm' in found['text'],
            [by['builds']['sameAs'], found['text']])
    r.check('the rows count the clips of each plan',
            [row['text'].split(' ')[-2] for row in found['rows']] == [str(by[k]['clips']) for k in ('calm', 'builds', 'fast')], found['rows'])

    t = ticks(r, song['id'])
    r.check('the song clip shows beats, bar starts and the drop', len(t['ticks']) >= 2 and t['ticks'][0] > 20 and t['drops'] == ['Drop · beat starts'], t)
    intro, rest = tick_columns(r, 'ticks', song['id'], planned['beatFromUs'] / song['durationUs'])
    r.check('the ticks are on screen from where the beat comes in, none before it', intro == 0 and rest > 40, {'intro': intro, 'after the beat': rest})
    shoot(r, 'rhythms', cut_in_view=True)
    reveal(r)
    wait(lambda: (b := beats(r)) and len(b['rows']) == 3, 10)

    # An agent plans the same cuts: the person and the agent do the same thing.
    bridge = Bridge(r, r.saved_project())
    try:
        agent = {k: bridge.call('cut_to_beats', {'dry_run': True, **args})
                 for k, args in (('calm', {'every': 'bar'}), ('builds', {}), ('fast', {'every': 'beat'}))}
        r.check("each rhythm cuts where an agent's cut_to_beats plans it",
                all(agent[k]['cut_us'] == by[k]['cutUs'] and agent[k]['end_us'] == by[k]['endUs'] for k in by),
                {k: (len(agent[k]['cut_us']), by[k]['clips']) for k in by})
        # Taking the lock shows the reason and keeps the numbers readable.
        run = bridge.call('begin_run', {'label': 'Looking at the beat'})['run_id']
        locked = wait(lambda: (b := beats(r)) and all(row['disabled'] for row in b['rows']) and b['cut']['disabled'] and b, 10)
        r.check('under the AI lock the rhythms and Cut are locked and still read', locked and all(' clips' in row['text'] for row in locked['rows']), beats(r))
        r.shot('locked')
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
    finally:
        bridge.close()
    wait(lambda: (b := beats(r)) and not b['cut']['disabled'], 10)
    # The stand-in agent's run ends with its own toast, which is not part of a person's cut.
    wait(lambda: r.s.run('return window.__nuzky.store.getState().toasts.some((t) => t.text.startsWith("AI edit done"))'), 5)
    r.s.run('const s = window.__nuzky.store.getState(); s.toasts.filter((t) => t.text.startsWith("AI edit done")).forEach((t) => s.dismissToast(t.id))')

    r.s.run('document.querySelector(\'[data-rhythm="fast"]\').click()')
    chosen = wait(lambda: (b := beats(r)) and next(row for row in b['rows'] if row['pressed'])['rhythm'] == 'fast' and b, 5)
    r.check('picking Fast chooses it and says what it does', chosen and 'A cut on every beat after the intro.' in chosen['text']
            and f"{by['fast']['clips']} clips from your 6 videos · their sound muted" in chosen['text'], beats(r))
    r.s.run('document.querySelector("[data-cut-to-beat]").click()')
    toast = f"Cut {by['fast']['clips']} clips to the beat of Dancing"
    r.check('a toast says what was cut and offers Undo', wait(lambda: r.s.run(
        'return window.__nuzky.store.getState().toasts.find((t) => t.text.startsWith(arguments[0]))?.action?.label ?? null', toast) == 'Undo', 10),
        r.state()['toasts'])

    def saved():
        project = json.loads(r.saved_project().read_text())
        return project if [c['startUs'] for c in main(project)] == by['fast']['cutUs'] else None
    cut = wait(saved, 15)
    r.check('the saved main track starts its clips on the planned cut frames', cut, cut and [c['startUs'] for c in main(cut)][:8])
    if cut:
        saved_title = next(c for t in cut['tracks'] if t['kind'] == 'text' for c in t['clips'] if c['id'] == title['id'])
        r.check('the title keeps its time', (saved_title['startUs'], saved_title['durationUs']) == (title['startUs'], title['durationUs']), saved_title)
        r.check("the new clips' own sound is muted", all(c['content']['volume'] == 0 for c in main(cut)), [c['content']['volume'] for c in main(cut)][:5])
        r.check('the video ends with the plan', main(cut)[-1]['startUs'] + main(cut)[-1]['durationUs'] == by['fast']['endUs'])
    after = wait(lambda: (b := beats(r)) and 'Current' in b['text'] and b, 10)
    r.check('the section shows Fast as the current cut and that another rhythm replaces it',
            after and 'Current' in next(row for row in after['rows'] if row['rhythm'] == 'fast')['text']
            and 'Picking another rhythm replaces this cut.' in after['text'], beats(r))
    fit(r)
    focused = r.s.run('return document.activeElement?.dataset.rhythm ?? null')
    r.check('the keyboard moves to the current rhythm', focused == 'fast', focused)
    t = ticks(r, song['id'])
    r.check('the ticks stay on the song after the cut', t['ticks'] and t['ticks'][0] > 20, t)
    label = r.s.call('window.__nuzky.api.peekHistory(window.__nuzky.store.getState().snap.sessionEpoch, false).then((p) => p?.label ?? null)')['value']
    r.check('Undo names the step Cut to beat', label == 'Cut to beat', label)
    shoot(r, 'cut')
    r.check('no error toast', not r.errors(), r.errors())

    # Another rhythm replaces the cut, so one Undo still leads back to the original.
    r.s.run('document.querySelector(\'[data-rhythm="calm"]\').click()')
    recut = wait(lambda: [c['startUs'] for c in main(r.s.run(PROJECT))] == by['calm']['cutUs'], 15)
    r.check('picking Calm recuts the video on its cuts', recut, len(main(r.s.run(PROJECT))))
    r.check('Calm is now the current cut', wait(lambda: (b := beats(r)) and 'Current' in next(row for row in b['rows'] if row['rhythm'] == 'calm')['text'], 10), beats(r))
    r.s.call('window.__nuzky.store.getState().undo()')
    back = wait(lambda: (p := r.s.run(PROJECT)) and [(c['id'], c['startUs'], c['durationUs']) for c in main(p)] == original and p, 10)
    r.check('one Undo restores the original video', back, [(c['startUs'], c['durationUs']) for c in main(r.s.run(PROJECT))])
    if back:
        r.check('the original keeps its sound and the song its length',
                all(c['content']['volume'] > 0 for c in main(back)) and music_clip(back)['durationUs'] == song['durationUs'])
    r.check('no error toast', not r.errors(), r.errors())


LONG_MINUTES = 50


def states_setup(r):
    make_clips(r, 3)
    # A click track long enough that finding its beat takes seconds, and noise without a beat.
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', f"aevalsrc='0.8*sin(2*PI*60*t)*exp(-30*mod(t,0.6))':s=48000:d={LONG_MINUTES * 60}",
                    '-ac', '1', '-c:a', 'libopus', '-b:a', '24k', str(r.work / 'Click track.opus')], check=True)
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'anoisesrc=color=brown:duration=60:amplitude=0.5', '-c:a', 'aac',
                    str(r.work / 'City walk.m4a')], check=True)


def add_song(r, name):
    r.import_media(r.work / name)
    r.s.call("""(() => { const st = window.__nuzky.store.getState();
        const asset = st.snap.project.assets.find((a) => a.name === arguments[0]);
        return st.edit({type: 'addClip', assetId: asset.id, startUs: 0, trackId: null}); })()""", name)
    return wait(lambda: next((c for t in r.s.run(PROJECT)['tracks'] if t['kind'] == 'audio' and t['keepInPlace'] for c in t['clips']
                              if c['content']['assetId'] == next(a['id'] for a in r.state()['assets'] if a['name'] == name)), None), 10)


def finding(r):
    b = beats(r)
    return b and 'Finding the beat in Click track' in b['text'] and b


@flow('beat-cuts-states', 'The Beats section shows finding the beat with its progress and a Stop in the top bar that stops it '
      'within seconds, says where a shorter song ends the video, and for a sound without a steady beat explains it '
      'without rhythms or a Cut button', before=states_setup)
def beat_cuts_states(r):
    resize(r, 1440, 900)
    r.import_media(*clips_of(r, 3))
    for path in clips_of(r, 3):
        r.add_clip(path.name)
    click = add_song(r, 'Click track.opus')
    r.check('the click track lies on a music track', click, r.state()['tracks'])
    r.check('Cut to beat is offered', reveal(r))
    r.check('the section shows finding the beat with a bar', wait(lambda: finding(r), 10), beats(r))
    r.s.run(f'{SECTION}?.scrollIntoView({{block: "start"}})')
    r.shot('finding')
    pill = r.s.run("return [...document.querySelectorAll('[role=status]')].map((s) => s.innerText.replace(/\\s+/g, ' ')).find((t) => t.includes('Finding the beat')) ?? null")
    r.check('the top bar lists the job', pill and 'Finding the beat in Click track' in pill, pill)
    stop = "[...document.querySelectorAll('button')].find((b) => b.textContent === 'Stop' && b.title.startsWith('Stop finding the beat'))"
    r.check('the top bar job has Stop', r.s.run(f'return !!{stop}'))
    asked = time.time()
    r.s.run(f'{stop}.click()')
    stopped = wait(lambda: (b := beats(r)) and 'Stopped before the beat was found.' in b['text'] and b, 10, 0.1)
    r.check('Stop stops finding within seconds and offers to find it again', stopped and time.time() - asked < 5, [time.time() - asked, beats(r)])
    r.s.run("[...document.querySelectorAll('button')].find((b) => b.textContent === 'Find the beat').click()")
    resize(r, 1024, 640)
    r.check('Find the beat starts again', wait(lambda: finding(r), 10), beats(r))
    r.s.run(f'{SECTION}?.scrollIntoView({{block: "start"}})')
    r.shot('finding-1024')
    resize(r, 1440, 900)
    found = wait(lambda: (b := beats(r)) and len(b['rows']) == 3 and b, 120, 0.5)
    r.check('the click track then offers rhythms', found, beats(r))
    r.check('its tempo is 100 BPM', found and '100 BPM' in found['text'], found and found['text'][:60])

    # Trimmed to 20 s, the song is shorter than the 24 s of videos: the section says where the video ends.
    r.s.call("window.__nuzky.store.getState().edit({type: 'trimClip', clipId: arguments[0], startUs: 0, durationUs: 20000000, sourceInUs: null})", click['id'])
    shorter = wait(lambda: (b := beats(r)) and 'Click track is shorter. The video ends with it on the bar at' in b['text'] and b, 10)
    r.check('a shorter song says where the video will end', shorter, beats(r))
    r.check('a beat from the first second names no intro', shorter and 'beat from' not in shorter['text'] and 'after its intro' not in shorter['text'], shorter and shorter['text'][:200])
    r.shot('shorter')

    r.s.call("window.__nuzky.store.getState().edit({type: 'deleteClips', clipIds: [arguments[0]]})", click['id'])
    walk = add_song(r, 'City walk.m4a')
    r.check('City walk lies on a music track', walk)
    r.check('Cut to beat is offered for it', reveal(r))
    plain = wait(lambda: (b := beats(r)) and 'No steady beat in City walk,' in b['text'] and b, 30)
    r.check('a sound without a steady beat says so and offers no rhythm or Cut', plain and not plain['rows'] and not plain['cut']
            and 'Add a song with drums' in plain['text'], beats(r))
    ticks_now = ticks(r, walk['id'])
    r.check('no ticks on a sound without a beat', not ticks_now['ticks'], ticks_now)
    shoot(r, 'no-beat')
    r.check('no error toast', not r.errors(), r.errors())

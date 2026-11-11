"""Record voiceover from the timeline's microphone button, with a stand-in microphone that plays prepared sound
(NUZKY_FAKE_MICROPHONE, debug builds): the script typed in the panel is saved with the project, the meter says when
the voice is too quiet, too loud or missing, Record counts 3, 2, 1 and Esc cancels, the take records while the video
plays and lands on an audio track where it started as one undo step, captions hear it, Discard leaves nothing, an
agent's script shows in the panel, and a take stopped while the AI edits waits with Add."""
import json, os, subprocess, time
from pathlib import Path

import numpy as np

from e2e.harness import FIXTURES, Bridge, Session, ffprobe, flow, link_models, press, wait, webdriver

PANEL = "document.querySelector('[data-voiceover]')"
SCRIPT = 'Three things make a phone video sound better. First, get close to the microphone.'
AGENT_SCRIPT = 'Written by the agent.\nRead it slowly.'
START_US = 1_500_000
RATE = 48_000


def setup(r):
    link_models(r)
    raw = subprocess.run(['ffmpeg', '-v', 'error', '-i', str(FIXTURES / 'speech.wav'), '-f', 'f32le', '-ac', '1', '-ar', str(RATE), '-'],
                         capture_output=True, check=True).stdout
    speech = np.frombuffer(raw, np.float32)
    speech = speech / np.abs(speech).max()
    # What the stand-in microphone can hear: 32-bit float mono samples at 48 kHz, played in a loop.
    for name, samples in {'speech': speech * 0.5, 'quiet': speech * 0.025, 'loud': np.clip(speech * 3, -0.99, 0.99),
                          'silence': np.zeros(RATE, np.float32)}.items():
        (r.work / f'{name}.f32').write_bytes(samples.astype('<f4').tobytes())
    (r.work / 'mic.f32').write_bytes((r.work / 'speech.f32').read_bytes())
    r.env.update(NUZKY_FAKE_MICROPHONE=str(r.work / 'mic.f32'))
    video = r.work / 'silent.mp4'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', f'testsrc2=s=1080x1920:r=30:d={len(speech) / RATE + 4:.3f}',
                    '-c:v', 'libx264', '-preset', 'veryfast', '-pix_fmt', 'yuv420p', str(video)], check=True)


def hear(r, name):
    """Gives the microphone another sound, heard from its start."""
    copy = r.work / f'{name}.next'
    copy.write_bytes((r.work / f'{name}.f32').read_bytes())
    os.replace(copy, r.work / 'mic.f32')


def panel(r):
    return r.s.run(f"""const p = {PANEL}; if (!p) return null;
        const record = p.querySelector('[data-record-voiceover]');
        return {{microphone: p.querySelector('[data-microphone]')?.textContent, warning: p.querySelector('[data-mic-warning]')?.dataset.micWarning ?? null,
                warningText: p.querySelector('[data-mic-warning]')?.textContent ?? null, error: p.querySelector('[data-mic-error]')?.textContent ?? null,
                meter: parseFloat(p.querySelector('[data-meter-fill]')?.style.width ?? '0'), record: record && [record.textContent, record.getAttribute('aria-disabled')],
                script: p.querySelector('[data-voiceover-script]')?.value ?? null, reading: p.querySelector('[data-voiceover-reading]')?.textContent ?? null,
                buttons: [...p.querySelectorAll('button')].map((b) => b.textContent.trim())}}""", retries=3)


def phase(r):
    return r.s.run('return window.__nuzky.voiceover.getState().phase', retries=3)


def click(r, text):
    r.s.run(f"[...{PANEL}.querySelectorAll('button')].find((b) => b.textContent.trim().startsWith(arguments[0])).click()", text)


def project(r):
    return r.s.run('return window.__nuzky.store.getState().snap.project', retries=3)


def script(r):
    return project(r).get('script', '')


def playing(r):
    return r.s.run('return window.__nuzky.store.getState().playing')


def voiceovers(r):
    return [a for a in project(r)['assets'] if a['name'].startswith('Voiceover')]


def audio_clips(r):
    return [c for t in r.state()['tracks'] if t['kind'] == 'audio' for c in t['clips']]


def recordings(r):
    folder = r.work / 'data/nuzky/recordings'
    return sorted(folder.glob('voiceover-*.wav')) if folder.is_dir() else []


def saved(r):
    return json.loads(r.saved_project().read_text())


def recorded_us(r):
    return r.s.run('return window.__nuzky.micLevel.getState().level?.recordedUs ?? 0') or 0


def type_text(text):
    for c in text:
        press({' ': 'space', '\n': 'Return'}.get(c, c))
        time.sleep(0.02)


def record(r, length_s, during=lambda: None):
    """Record and, past the countdown, `length_s` of the take, with the speech from its start; `during` runs meanwhile."""
    click(r, 'Record')
    if not wait(lambda: phase(r) == 'recording', 6, 0.05):
        return False
    hear(r, 'speech')
    during()
    return wait(lambda: recorded_us(r) >= length_s * 1e6, length_s + 10, 0.1)


def open_voiceover(r):
    r.s.run("document.querySelector('[data-voiceover-button]').click()")
    return wait(lambda: (p := panel(r)) and p['microphone'] == 'Test microphone' and p, 10)


def voiceover(r):
    r.import_media(r.work / 'silent.mp4')
    r.add_clip('silent.mp4')
    r.s.run('window.__nuzky.store.getState().select([])')
    r.seek(START_US)
    r.check('a project saved before any script has none in its file',
            wait(lambda: 'script' not in saved(r) and saved(r)['tracks'][0]['clips'], 10), list(saved(r)))

    # The microphone opens only from the button, and its meter shows the voice.
    time.sleep(1)
    r.check('nothing listens to the microphone before the user opens the voiceover', panel(r) is None and phase(r) == 'closed')
    r.check('the timeline\'s microphone button opens the voiceover in the library\'s place, naming the microphone', open_voiceover(r), panel(r))
    level = wait(lambda: (p := panel(r)) and p['meter'] > 50 and p, 5, 0.1)
    r.check('the meter shows the voice at a good level, without a warning', level and level['warning'] is None, panel(r))

    # The script, typed with the keyboard, is the project's and saved with it.
    r.s.run(f"{PANEL}.querySelector('[data-voiceover-script]').focus()")
    type_text(SCRIPT)
    r.check('the typed script is in the project', wait(lambda: script(r) == SCRIPT, 5), script(r))
    r.check('and in its saved file', wait(lambda: saved(r).get('script') == SCRIPT, 10), saved(r).get('script'))
    r.s.run('document.activeElement.blur()')
    time.sleep(0.3)
    r.shot('ready')

    # Warnings in words beside the meter.
    for name, warning, words in (('quiet', 'quiet', 'Too quiet'), ('loud', 'loud', 'Too loud'), ('silence', 'silent', 'hears nothing')):
        hear(r, name)
        seen = wait(lambda: (p := panel(r)) and p['warning'] == warning and p, 6, 0.1)
        r.check(f'a {name} microphone gets the warning "{words}…" with what to do', seen and words in seen['warningText'], panel(r))
        time.sleep(0.2)
        r.shot(f'warning-{warning}')
    hear(r, 'speech')
    r.check('a good voice again clears the warning', wait(lambda: (p := panel(r)) and p['warning'] is None and p['meter'] > 50, 6, 0.1), panel(r))

    # Record counts 3, 2, 1 over the preview; Esc cancels without recording.
    click(r, 'Record')
    counts = []
    deadline = time.time() + 1.6
    while time.time() < deadline:
        n = r.s.run("return document.querySelector('[data-voiceover-countdown] .tabular')?.textContent ?? null")
        if n and n not in counts:
            counts.append(n)
            if n == '2':
                r.shot('countdown')
        time.sleep(0.1)
    r.check('Record counts down over the preview from 3', counts[:2] == ['3', '2'], counts)
    press('Escape')
    r.check('Esc cancels the countdown', wait(lambda: phase(r) == 'ready', 3)
            and not r.s.run("return !!document.querySelector('[data-voiceover-countdown]')"), phase(r))
    time.sleep(3)
    r.check('and the cancelled countdown never starts a take', phase(r) == 'ready' and not recordings(r), [phase(r), recordings(r)])

    # A take: the video plays from the playhead while it records, and the take grows on the timeline.
    seconds, _ = ffprobe(str(FIXTURES / 'speech.wav'))
    seen = {}
    def during():
        time.sleep(2.5)
        seen['playing'] = playing(r)
        # The take and the picture moved the same time since it started.
        seen['sync'] = r.s.run('''const st = window.__nuzky.store.getState(), v = window.__nuzky.voiceover.getState();
            return [window.__nuzky.micLevel.getState().level.recordedUs, st.timeUs - v.startUs]''')
        width = "const c = document.querySelector('[data-recording-clip]'); return c && c.getBoundingClientRect().width"
        seen['widths'] = [r.s.run(width)]
        r.shot('recording')
        time.sleep(0.5)
        seen['widths'].append(r.s.run(width))
        seen['panel'] = panel(r)
        # The editor's keys wait while it records, Command ones too: Cmd+Z would undo the typed script.
        r.key('z', metaKey=True)
        r.key('z', ctrlKey=True)
        time.sleep(0.5)
        seen['undo'] = [script(r), phase(r)]
    took = record(r, seconds + 0.6, during)
    r.check('the take records while the video plays from the playhead', took and seen.get('playing') is True, seen.get('playing'))
    sync = seen.get('sync') or [0, 1e9]
    r.check('the take keeps time with the picture: both moved the same within 0.1 s', abs(sync[0] - sync[1]) < 100_000, sync)
    widths = seen.get('widths') or [0, 0]
    r.check('it grows on the timeline as it records', widths[0] and widths[1] and widths[1] > widths[0], widths)
    r.check('Cmd+Z and Ctrl+Z wait while it records', seen.get('undo') == [SCRIPT, 'recording'], seen.get('undo'))
    p = seen.get('panel')
    r.check('the script shows large to read, and Stop takes Record\'s place', p and p['reading'] == SCRIPT and 'StopSpace' in p['buttons'], p)
    before = len(r.state()['assets'])
    press('space')
    wait(lambda: audio_clips(r) and voiceovers(r), 20)
    take = next(iter(voiceovers(r)), None)
    clip = next(iter(audio_clips(r)), None)
    r.check('Space stops the take and puts it on an audio track where it started, with the first moving frame', clip and take
            and START_US < clip['startUs'] < START_US + 150_000
            and clip['assetId'] == take['id'] and take['kind'] == 'audio' and take['name'] == 'Voiceover', [clip, take])
    r.check('the video stops and the take is as long as it recorded', take and not playing(r)
            and abs(take['durationUs'] / 1e6 - (seconds + 0.6)) < 0.6, take and take['durationUs'])
    files = recordings(r)
    r.check('the take is a WAV in Nuzky\'s own folder that FFmpeg reads at its length', take and len(files) == 1
            and Path(take['path']).resolve() == files[0].resolve() and abs(ffprobe(str(files[0]))[0] - take['durationUs'] / 1e6) < 0.05,
            [str(f) for f in files])
    toast = wait(lambda: next((t for t in r.state()['toasts'] if t['text'].startswith('Voiceover added')), None), 5)
    r.check('a toast says where it went, with Undo', toast and 'at 00:01.' in toast['text'], toast)
    time.sleep(0.6)
    r.shot('added')
    r.key('z', ctrlKey=True)
    r.check('one undo takes the take off the timeline and out of Media',
            wait(lambda: not audio_clips(r) and len(r.state()['assets']) == before, 5), [a['name'] for a in r.state()['assets']])
    r.key('z', ctrlKey=True, shiftKey=True)
    r.check('and redo brings it back', wait(lambda: audio_clips(r) and voiceovers(r), 5))

    # Captions hear the take, as any voice-over.
    r.s.run("document.querySelector('[data-voiceover-button]').click()")
    r.check('the microphone button closes the voiceover and gives the library back', wait(lambda: panel(r) is None and phase(r) == 'closed', 5))
    r.s.run("window.__nuzky.speech.setState({model: 'small', language: 'en'})")
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === 'Captions').click()")
    time.sleep(0.5)
    r.s.run("document.querySelector('[data-generate-captions]').click()")
    job = wait(lambda: next((j for j in r.state()['jobs'] if j['kind'] == 'captions' and j['status'] != 'running'), None), 600, 1)
    texts = ' '.join(c['text'] for t in r.state()['tracks'] if t['name'] == 'Captions' for c in t['clips']).lower()
    found = [w for w in ['channel', 'video', 'laptop', 'clips', 'pauses', 'captions'] if w in texts]
    r.check('captions say what the voiceover says', job and job['status'] == 'done' and len(found) >= 4, [job and job['status'], found, texts[:200]])

    # At the end of the video Record waits with the reason, since sound after the picture is not exported.
    open_voiceover(r)
    r.seek(10**9)
    at_end = wait(lambda: (p := panel(r)) and p['record'] and p['record'][1] == 'true' and p, 5)
    reason = r.s.run(f"return {PANEL}.querySelector('[data-record-voiceover]').title")
    r.check('at the end of the video Record waits, saying why', at_end and 'video ends here' in reason, reason)
    # A take that reaches the end of the video stops there and goes on the timeline.
    takes = len(voiceovers(r))
    end = project(r)['tracks'][0]['clips'][-1]
    r.seek(end['startUs'] + end['durationUs'] - 2_000_000)
    click(r, 'Record')
    ended = wait(lambda: len(voiceovers(r)) == takes + 1 and phase(r) == 'ready', 12)
    toast = next((t['text'] for t in r.state()['toasts'] if t['text'].startswith('The video ended')), None)
    r.check('a take that reaches the end of the video stops there and is kept, saying so', ended and toast, [toast, r.state()['toasts']])
    r.key('z', ctrlKey=True)
    wait(lambda: len(voiceovers(r)) == takes, 5)
    r.seek(START_US)

    # Moving the playhead, even half a second, ends the take where it was: the picture no longer matches the voice.
    record(r, 1.0)
    r.seek(r.s.run('return window.__nuzky.store.getState().timeUs') + 500_000)
    moved = wait(lambda: len(voiceovers(r)) == takes + 1 and phase(r) == 'ready', 10)
    toast = next((t['text'] for t in r.state()['toasts'] if t['text'].startswith('The playhead moved')), None)
    r.check('moving the playhead ends the take and keeps it, saying so', moved and toast, [toast, r.state()['toasts']])
    r.key('z', ctrlKey=True)
    wait(lambda: len(voiceovers(r)) == takes, 5)
    r.seek(START_US)

    # Discard leaves nothing, and the next take starts where the discarded one did.
    count = len(r.state()['assets'])
    files = len(recordings(r))
    record(r, 1.5)
    click(r, 'Discard')
    r.check('Discard ends a take without adding it and deletes what it recorded', wait(lambda: phase(r) == 'ready', 10)
            and len(r.state()['assets']) == count and len(recordings(r)) == files, [len(r.state()['assets']), recordings(r)])
    started = r.s.run('return window.__nuzky.voiceover.getState().startUs')
    r.check('and the playhead goes back to where it started', wait(lambda: r.s.run('return window.__nuzky.store.getState().timeUs') == started, 5),
            [started, r.s.run('return window.__nuzky.store.getState().timeUs')])

    # An agent writes the script over MCP and the panel shows it; a take stopped while the AI edits waits with Add.
    bridge = Bridge(r, r.saved_project())
    try:
        run = bridge.call('begin_run', {'label': 'Script'})['run_id']
        bridge.call('apply_edits', {'run_id': run, 'request_id': 'script', 'edits': [{'type': 'setScript', 'text': AGENT_SCRIPT}]})
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
        r.check('an agent\'s script over MCP shows in the panel', wait(lambda: (p := panel(r)) and p['script'] == AGENT_SCRIPT, 10), panel(r))
        r.check('and get_state gives it to the agent', bridge.call('get_state', {}).get('script') == AGENT_SCRIPT)
        r.seek(START_US)
        record(r, 1.5)
        run = bridge.call('begin_run', {'label': 'Edit'})['run_id']
        wait(lambda: r.state()['aiRun'], 10)
        click(r, 'Stop')
        waiting = wait(lambda: (p := panel(r)) and phase(r) == 'ready' and 'Add to timeline' in p['buttons'] and p, 15)
        r.check('a take stopped while the AI edits is kept, waiting with Add', waiting and len(recordings(r)) == files + 1
                and len(r.state()['assets']) == count, [panel(r), recordings(r)])
        time.sleep(0.3)
        r.shot('not-added')
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
        wait(lambda: not r.state()['aiRun'], 10)
    finally:
        bridge.close()
    click(r, 'Add to timeline')
    r.check('Add then puts it on the timeline', wait(lambda: len(audio_clips(r)) == 2 and len(voiceovers(r)) == 2, 10),
            [a['name'] for a in voiceovers(r)])
    errors = [e for e in r.errors() if 'AI' not in e]
    r.check('no other error toast', not errors, r.errors())

    # The smallest window.
    webdriver('POST', r.s.path + '/window/rect', {'width': 1024, 'height': 640})
    wait(lambda: r.s.run('return window.innerWidth') == 1024, 5)
    time.sleep(0.6)
    r.shot('ready-1024')
    webdriver('POST', r.s.path + '/window/rect', {'width': 1440, 'height': 900})
    wait(lambda: r.s.run('return window.innerWidth') == 1440, 5)

    # The script outlives a restart.
    r.s.close()
    r.s = Session()
    wait(lambda: r.s.run('return !!window.__nuzky?.store.getState().snap', retries=3), 60)
    r.check('the script is back after a restart', wait(lambda: script(r) == AGENT_SCRIPT, 20), script(r))


flow('voiceover', 'Record voiceover with the script on screen, a 3-2-1 countdown and a meter that warns about a quiet, loud or '
     'silent microphone; the take lands on an audio track where it started as one undo step, captions hear it, Discard leaves '
     'nothing, an agent\'s script shows, and a take stopped while the AI edits waits with Add', before=setup)(voiceover)

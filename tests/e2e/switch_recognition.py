import re, subprocess, time

from e2e.harness import CLI, FIXTURES, flow, link_models, wait

TOP_BAR = "return [...document.querySelector('header').querySelectorAll('[role=status]')].map((e) => e.innerText).join(' | ')"
SPEECH_JOBS = ("return Object.values(window.__nuzky.store.getState().jobs).filter((j) => j.kind === 'transcript' || j.kind === 'captions')"
               '.map((j) => [j.id, j.status])')
TAB = "[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === arguments[0]).click()"


def setup(r):
    link_models(r)
    # Two minutes of speech, so its recognition still runs when the other project opens.
    long = r.work / 'long-talk.mp4'
    subprocess.run(['ffmpeg', '-v', 'error', '-i', str(FIXTURES / 'speech.wav'), '-f', 'lavfi', '-i', 'color=s=320x568:r=30',
                    '-af', 'aresample=16000,apad=whole_dur=16,aloop=loop=7:size=256000', '-c:v', 'libx264', '-preset', 'ultrafast',
                    '-c:a', 'aac', '-shortest', str(long)], check=True)
    for name, media in (('long', long), ('short', FIXTURES / 'talk.mp4')):
        subprocess.run([str(CLI), 'new', str(r.work / f'data/nuzky/projects/{name}.nuzky'), str(media)], env=r.env, check=True, capture_output=True)


@flow('switch_recognition', 'Opening another project while a transcript is recognised stops it: the opened project shows '
      'nothing of it and transcribes its own speech at once', before=setup)
def switch_recognition(r):
    projects = r.work / 'data/nuzky/projects'

    def open_project(name):
        path = str(projects / f'{name}.nuzky')
        r.s.run('window.__nuzky.openProject({path: arguments[0], name: arguments[1]})', path, name)
        if not wait(lambda: r.s.run('return window.__nuzky.store.getState().snap?.path') == path, 15):
            raise RuntimeError(f'{name} did not open')
        r.s.run(TAB, 'Transcript')

    button = "document.querySelector('[data-transcribe]')"
    can_transcribe = lambda: r.s.run(f'return !!{button} && !{button}.disabled')

    def speech_shown():
        """What of speech recognition the window shows: the top bar's capsule and the Transcript tab's progress card."""
        text = r.s.run('return document.body.innerText')
        phases = ('Transcript ·', 'Recognising speech', 'Measuring word times', 'Waiting for another transcription', 'Downloading word timing model')
        return [phrase for phrase in phases if phrase in text]

    r.s.run("window.__nuzky.speech.setState({model: 'small', language: 'en'})")
    open_project('long')
    wait(can_transcribe, 15)
    r.s.run(f'{button}.click()')
    recognising = wait(lambda: (found := re.search(r'Recognising speech\s*(\d+)%', r.s.run(TOP_BAR) or '')) and int(found[1]) > 0, 600, 0.2)
    r.check('the long video is being recognised, with its percentage in the top bar', recognising, r.s.run(TOP_BAR))
    r.shot('recognising')

    stopping = [job_id for job_id, status in r.s.run(SPEECH_JOBS) if status == 'running']
    open_project('short')
    ready = wait(can_transcribe, 1, 0.05)
    shown = speech_shown()
    r.shot('opened')
    r.check("the opened project shows nothing of the other project's recognition", not shown, {'shown': shown, 'top bar': r.s.run(TOP_BAR)})
    r.check('Transcribe timeline is there at once', ready)
    # Clicked while the other project's recognition may still be stopping.
    if ready:
        r.s.run(f'{button}.click()')
    errors = set()
    # That recognition keeps sending progress until it stops, so the jobs are watched for a while, not once.
    seen = []
    until = time.monotonic() + 4
    while time.monotonic() < until:
        errors.update(r.errors())
        left = [job for job in r.s.run(SPEECH_JOBS) if job[0] in stopping]
        if left:
            seen.append({'jobs': left, 'top bar': r.s.run(TOP_BAR)})
        time.sleep(0.2)
    r.check("the other project's recognition never comes back while it stops", not seen, seen[:3])

    def transcribed():
        errors.update(r.errors())
        return any(status == 'done' for _, status in r.s.run(SPEECH_JOBS))
    r.check('the opened project transcribes its own speech', wait(transcribed, 600, 0.5), r.s.run(SPEECH_JOBS))
    words = "[role=listbox][aria-label=Transcript] [role=option][data-t]"
    r.check('its words show in the Transcript tab', wait(lambda: r.s.run(f"return document.querySelectorAll('{words}').length") > 5, 15))
    r.shot('transcribed')
    errors.update(r.errors())
    r.check('no error toast along the way, not even about the stopped recognition', not errors, sorted(errors))

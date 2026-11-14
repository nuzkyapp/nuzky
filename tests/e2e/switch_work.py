import re, subprocess, time

from e2e.harness import CLI, FIXTURES, flow, link_models, wait

TOP_BAR = "return [...document.querySelector('header').querySelectorAll('[role=status]')].map((e) => e.innerText).join(' | ')"
PREVIEW = 'return document.querySelector("section[aria-label=Preview]").innerText'
JOBS = 'return Object.values(window.__nuzky.store.getState().jobs).map((j) => [j.id, j.kind, j.status])'
TAB = "[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === arguments[0]).click()"
RATIO = 'section[aria-label=Preview] button[aria-haspopup=menu]'
REFRAME_WIDE = '[role=menu][aria-label="Canvas ratio"] [role=menuitem][title^="Switch to 16:9"]'


def setup(r):
    link_models(r, names=('ggml-small.bin', 'ggml-silero-v5.1.2.bin', 'yunet-2026may.onnx'))
    # Two minutes of speech, so its recognition still runs when the other project opens.
    long = r.work / 'long-talk.mp4'
    subprocess.run(['ffmpeg', '-v', 'error', '-i', str(FIXTURES / 'speech.wav'), '-f', 'lavfi', '-i', 'color=s=320x568:r=30',
                    '-af', 'aresample=16000,apad=whole_dur=16,aloop=loop=7:size=256000', '-c:v', 'libx264', '-preset', 'ultrafast',
                    '-c:a', 'aac', '-shortest', str(long)], check=True)
    for name, media in (('long', long), ('short', FIXTURES / 'talk.mp4')):
        subprocess.run([str(CLI), 'new', str(r.work / f'data/nuzky/projects/{name}.nuzky'), str(media)], env=r.env, check=True, capture_output=True)


@flow('switch_work', 'Opening another project stops the work started for the one before: a transcript, Reframe and '
      'preparing the sound of a file the opened project does not have. The opened project shows none of it and '
      'transcribes its own speech at once', before=setup)
def switch_work(r):
    projects = r.work / 'data/nuzky/projects'
    button = "document.querySelector('[data-transcribe]')"
    can_transcribe = lambda: r.s.run(f'return !!{button} && !{button}.disabled')

    def open_project(name):
        path = str(projects / f'{name}.nuzky')
        r.s.run('window.__nuzky.openProject({path: arguments[0], name: arguments[1]})', path, name)
        if not wait(lambda: r.s.run('return window.__nuzky.store.getState().snap?.path') == path, 15):
            raise RuntimeError(f'{name} did not open')
        r.s.run(TAB, 'Transcript')

    def logged(line):
        return sum(log.read_text().count(line) for log in (r.work / 'data/nuzky/logs').glob('*.log'))

    def shown(phrases):
        """Which of the phrases the window shows: the top bar's capsule, the preview's chips and the panels."""
        text = r.s.run('return document.body.innerText')
        return [phrase for phrase in phrases if phrase in text]

    def watch(seconds, seen):
        """What `seen` finds while the work of the project open before stops, which keeps sending progress until then."""
        found, until = [], time.monotonic() + seconds
        while time.monotonic() < until:
            if (now := seen()):
                found.append(now)
            time.sleep(0.2)
        return found

    def stops_in_other_project(what, kind, phrases, start):
        """Starts work in the long project, opens the short one and checks it shows none of it and the work stops."""
        open_project('long')
        start()
        running = wait(lambda: [job for job in r.s.run(JOBS) if job[1] == kind and job[2] == 'running'], 120, 0.1)
        r.check(f'{what} runs in the long project', running and wait(lambda: shown(phrases), 10, 0.1), {'jobs': r.s.run(JOBS), 'top bar': r.s.run(TOP_BAR)})
        stopped = logged(f'job {kind} cancelled')
        open_project('short')
        r.shot(f'opened-{kind}')
        ids = [job[0] for job in running or []]
        left = watch(3, lambda: {'shown': shown(phrases), 'jobs': [job for job in r.s.run(JOBS) if job[0] in ids]} if shown(phrases) or any(
            job[0] in ids for job in r.s.run(JOBS)) else None)
        r.check(f'the short project shows nothing of {what}', not left, left[:3])
        r.check(f'{what} stops', wait(lambda: logged(f'job {kind} cancelled') > stopped, 10), r.s.run(TOP_BAR))

    r.s.run("window.__nuzky.speech.setState({model: 'small', language: 'en'})")

    # A transcript: the short project transcribes its own speech at once, while the long one's may still be stopping.
    open_project('long')
    wait(can_transcribe, 15)
    r.s.run(f'{button}.click()')
    recognising = wait(lambda: (found := re.search(r'Recognising speech\s*(\d+)%', r.s.run(TOP_BAR) or '')) and int(found[1]) > 0, 600, 0.2)
    r.check('the long video is being recognised, with its percentage in the top bar', recognising, r.s.run(TOP_BAR))
    r.shot('recognising')
    stopping = [job[0] for job in r.s.run(JOBS) if job[1] == 'transcript' and job[2] == 'running']
    open_project('short')
    ready = wait(can_transcribe, 1, 0.05)
    speech = ('Transcript ·', 'Recognising speech', 'Measuring word times', 'Waiting for another transcription', 'Downloading word timing model')
    r.shot('opened')
    r.check("the opened project shows nothing of the other project's recognition", not shown(speech), {'shown': shown(speech), 'top bar': r.s.run(TOP_BAR)})
    r.check('Transcribe timeline is there at once', ready)
    if ready:
        r.s.run(f'{button}.click()')
    errors = set()
    left = watch(4, lambda: errors.update(r.errors()) or [job for job in r.s.run(JOBS) if job[0] in stopping])
    r.check("the other project's recognition never comes back while it stops", not left, left[:3])

    def transcribed():
        errors.update(r.errors())
        return any(job[1] == 'transcript' and job[2] == 'done' for job in r.s.run(JOBS))
    r.check('the opened project transcribes its own speech', wait(transcribed, 600, 0.5), r.s.run(JOBS))
    words = "[role=listbox][aria-label=Transcript] [role=option][data-t]"
    r.check('its words show in the Transcript tab', wait(lambda: r.s.run(f"return document.querySelectorAll('{words}').length") > 5, 15))
    r.shot('transcribed')

    # Reframe from the Ratio menu: its chip in the preview and its capsule in the top bar stay with the long project.
    def reframe():
        r.s.run(f'document.querySelector("{RATIO}").click()')
        wait(lambda: r.s.run(f"return !!document.querySelector('{REFRAME_WIDE}')"), 5)
        r.s.run(f"document.querySelector('{REFRAME_WIDE}').click()")
    stops_in_other_project('Reframe', 'reframe', ('Following the face',), reframe)
    r.s.run(f'document.querySelector("{RATIO}").click()')
    wait(lambda: r.s.run(f"return !!document.querySelector('{REFRAME_WIDE}')"), 5)
    why = r.s.run(f"const b = document.querySelector('{REFRAME_WIDE}'); "
                  "return !b ? 'no Reframe in the Ratio menu' : b.disabled || b.getAttribute('aria-disabled') === 'true' ? b.title : null")
    r.check('the short project can reframe at once', why is None, why)
    r.key('Escape')

    # A file only the long project has: preparing its sound stops, and the short project shows nothing of it.
    stops_in_other_project('preparing the sound of hour.m4a', 'audio', ('Preparing audio for hour.m4a',), lambda: r.import_media(FIXTURES / 'hour.m4a'))

    errors.update(r.errors())
    r.check('no error toast along the way, not even about the stopped work', not errors, sorted(errors))

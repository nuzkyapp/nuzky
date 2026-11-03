import re, subprocess, time

from e2e.ai_panel import FAKE, click, idle, panel, send
from e2e.harness import CLI, FIXTURES, flow, link_models, wait

AGENT_JOBS = "return Object.values(window.__nuzky.store.getState().jobs).filter((j) => j.kind === 'agent').map((j) => [j.label, j.status, j.progress])"
TOP_BAR = "return [...document.querySelector('header').querySelectorAll('[role=status]')].map((e) => e.innerText).join(' | ')"


def setup(r):
    link_models(r)
    # Two minutes of speech with pauses: long enough to watch the percentage move.
    media = r.work / 'speech.mp4'
    subprocess.run(['ffmpeg', '-v', 'error', '-i', str(FIXTURES / 'speech.wav'), '-f', 'lavfi', '-i', 'color=s=320x568:r=30',
                    '-af', 'aresample=16000,apad=whole_dur=16,aloop=loop=7:size=256000', '-c:v', 'libx264', '-preset', 'ultrafast',
                    '-c:a', 'aac', '-shortest', str(media)], check=True)
    project = r.work / 'data/nuzky/projects/jobs.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(media)], env=r.env, check=True, capture_output=True)
    r.env.update(NUZKY_AGENT_CLAUDE=str(FAKE), NUZKY_AGENT_CODEX=str(r.work / 'no-codex'), NUZKY_FAKE_AGENT_LOG=str(r.work / 'fake'))


@flow('agent_jobs', "A transcription the AI agent runs shows in the top bar with a percentage that moves as the speech is "
      'recognised and timed, and Stop there ends only that work within seconds', before=setup)
def agent_jobs(r):
    r.s.run('document.body.focus()')
    r.key('j', ctrlKey=True)
    wait(lambda: panel(r), 5)
    wait(lambda: r.s.run('return window.__nuzky.agent.getState().agents'), 10)
    send(r, 'transcribe please')
    seen = []

    def moving():
        found = re.search(r'Transcribing\s*(\d+)%', r.s.run(TOP_BAR) or '')
        if found and int(found[1]) not in seen:
            seen.append(int(found[1]))
        return len([p for p in seen if 0 < p < 100]) >= 3
    r.check("the agent's transcription shows in the top bar with a percentage that moves", wait(moving, 600, 0.2), seen)
    r.shot('transcribing')
    r.check('the percentage only goes up', seen == sorted(seen), seen)
    r.check('the agent waits on the job until it is done and says so', wait(lambda: 'Přepis skončil: done.' in panel(r)['text'], 600), panel(r)['text'])
    r.check('the capsule leaves the top bar once it is done', wait(lambda: 'Transcribing' not in r.s.run(TOP_BAR), 5), r.s.run(TOP_BAR))
    idle(r)

    send(r, 'transcribe again please')
    r.check('a second transcription shows again', wait(lambda: 'Transcribing' in r.s.run(TOP_BAR), 60), r.s.run(TOP_BAR))
    # Its turn may wait behind another flow's recognition (one at a time on a computer); Stop has to end a running one.
    r.check('the second transcription is recognising speech',
            wait(lambda: (found := re.search(r'Transcribing\s*(\d+)%', r.s.run(TOP_BAR) or '')) and int(found[1]) > 0, 600, 0.2), r.s.run(TOP_BAR))
    started = time.monotonic()
    click(r, 'Stop', 'header')
    stopped = wait(lambda: any(j[1] == 'cancelled' for j in r.s.run(AGENT_JOBS)), 5, 0.05)
    r.check('Stop in the top bar ends the transcription within seconds', stopped, {'seconds': round(time.monotonic() - started, 2), 'jobs': r.s.run(AGENT_JOBS)})
    r.check('the agent sees that it stopped and says so, without being stopped itself',
            wait(lambda: 'Přepis skončil: cancelled.' in panel(r)['text'], 15), panel(r)['text'])
    r.shot('stopped')
    r.check('no error toast along the way', not r.errors(), r.errors())

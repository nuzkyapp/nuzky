"""An agent in the AI panel learns what it cannot see on the timeline: when and where a phone video was recorded, its
own frames, the quiet place for a cut, mains hum, what the video shows and where a dog in it is. The scene model is
missing, so the conversation asks for it with one Download button and the agent goes on by itself. The model comes
from a loopback server; the clip is the sample video built into Nuzky (CC BY) with an iPhone's tags of when and where."""
import json, re, subprocess

from e2e.ai_panel import agent, click, panel, send
from e2e.harness import CLI, MODELS, ROOT, flow, wait
from e2e.speech_model import Models

# The scene model's files with the path the loopback server answers them on: Hugging Face's path without its host.
SCENE = {name: url.removeprefix('huggingface.co') for name, url in re.findall(
    r'file: "(florence-[^"]+)",\s*label: "[^"]+",\s*url: "https://([^"]+)"', (ROOT / 'crates/vision/src/models.rs').read_text())}
CARD = 'aside[aria-label=AI] [aria-labelledby=scene-question-title]'
PROMPT = 'What is in the clips?'
STEPS = ['Reading when and where clips were shot', 'Looking through a clip', 'Finding a quiet spot to cut',
         'Listening for hum and whine', 'Describing a clip', 'Following an object']


def needs():
    missing = [name for name in SCENE if not (MODELS / name).exists()]
    return f'the scene model in {MODELS} (scripts/fixtures.sh)' if missing else None


def before(r):
    take = r.work / 'IMG_0042.MOV'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-i', str(ROOT / 'assets/sample/spotlight-effect.mp4'), '-t', '20',
                    '-c', 'copy', '-movflags', 'use_metadata_tags',
                    '-metadata', 'com.apple.quicktime.creationdate=2024-05-12T14:03:22+0200',
                    '-metadata', 'com.apple.quicktime.location.ISO6709=+50.0755+014.4378+250.000/',
                    '-metadata', 'com.apple.quicktime.make=Apple', '-metadata', 'com.apple.quicktime.model=iPhone 15 Pro',
                    str(take)], check=True)
    project = r.work / 'data/nuzky/projects/dog.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(take)], env=r.env, check=True, capture_output=True)
    (r.work / 'data/nuzky/models').mkdir(parents=True)
    r.models = Models({path: MODELS / name for name, path in SCENE.items()})
    r.env.update(NUZKY_MODEL_URL=f'http://127.0.0.1:{r.models.http.server_port}', NUZKY_AGENT_CLAUDE=str(ROOT / 'tests/e2e/fake_agent'),
                 NUZKY_AGENT_CODEX=str(r.work / 'no-codex'), NUZKY_FAKE_AGENT_LOG=str(r.work / 'fake'))


def card(r):
    return r.s.run("""const c = document.querySelector(arguments[0]);
        return c && {text: c.innerText, buttons: [...c.querySelectorAll('button')].map((b) => b.textContent.trim())};""", CARD)


def last_answer(r):
    return r.s.run("return window.__nuzky.agent.getState().items.filter((i) => i.kind === 'agent').at(-1)?.text ?? null")


@flow('agent_inputs', 'An agent reads when and where a clip was shot, its frames, a quiet cut, hum, a description and a '
      'dog it follows, after the AI panel asks for the scene model once', before=before, needs=needs)
def agent_inputs(r):
    models = r.work / 'data/nuzky/models'
    try:
        r.s.run('document.body.focus()')
        r.key('j', ctrlKey=True)
        wait(lambda: panel(r), 5)
        wait(lambda: r.s.run("return window.__nuzky.agent.getState().agents?.some((a) => a.name === 'Claude Code' && a.path)"), 10)
        r.models.rate = 8 * 1024 * 1024
        send(r, PROMPT)
        c = wait(lambda: card(r), 60)
        if not r.check('the conversation asks for the scene model, 278 MB, with Download and Not now',
                       c and 'Download the scene model?' in c['text'] and c['buttons'] == ['Download 278 MB', 'Not now'], c):
            return
        r.check('nothing was downloaded before the answer', not r.models.requests and not any(models.iterdir()), r.models.requests)
        r.shot('question')
        click(r, 'Download 278 MB', CARD)
        r.check('the top bar shows the download with its own Stop', wait(lambda: r.s.run(
            "return !!document.querySelector('header button[aria-label=\"Stop downloading the scene model\"]')"), 10))
        r.check('the card turns into the download with its percentage and Stop', wait(lambda: re.search(
            r'Downloading the scene model\s*\d+%', panel(r)['text']), 10), panel(r)['text'])
        r.shot('downloading')
        r.models.rate = 0
        r.check('the agent goes on by itself once the model is in', wait(lambda: agent(r)['status'] == 'idle', 240)
                and (last_answer(r) or '').startswith('Natočeno 2024-05-12'), panel(r)['text'])
        fetched = sorted({q['path'] for q in r.models.requests})
        r.check('only the scene model was downloaded, and it is installed', fetched == sorted(SCENE.values())
                and all((models / name).stat().st_size == (MODELS / name).stat().st_size for name in SCENE), fetched)
        click(r, '7 steps')
        text = panel(r)['text']
        r.check('each input is a step with its plain title, the object with its words',
                all(step in text for step in STEPS) and re.search(r'Following an object\W*dog', text), text)
        found = json.loads((r.work / 'fake/inputs.json').read_text())
        meta = found['meta']
        r.check('the date, place and phone come from the file', meta['recorded_at'] == '2024-05-12T14:03:22+02:00'
                and meta['location']['latitude'] == 50.0755 and meta['device'] == 'Apple iPhone 15 Pro', meta)
        description = found['described']['shots'][0]['description'].lower()
        # The 8-bit model's words differ by processor: on x86-64 the dog in her arms reads as "a toy".
        r.check('the description names the woman', 'woman' in description, description)
        r.check('the dog is followed in every look, low in the middle of the picture', found['tracked']['found'] == 1.0
                and all(0.35 < l['box'][0] + l['box'][2] / 2 < 0.65 and l['box'][1] > 0.5 for l in found['tracked']['track']),
                found['tracked'])
        r.check('the quiet place lies near the time asked for', abs(found['quiet']['quiet_us'] - 2_000_000) <= 300_000, found['quiet'])
        r.check('the sample has no mains hum', found['spectrum']['hum'] is None, found['spectrum'])
        r.check('the card is gone', not card(r), text)
        r.shot('answered')
    finally:
        r.models.http.shutdown()
        r.models.http.server_close()
    r.check('no error toast', not r.errors(), r.errors())

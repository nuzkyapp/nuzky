"""Reels from a long video in the app: the Transcript tab of a recording over three minutes opens the mode, Find reels
asks the AI panel's agent (the fake one, which proposes four moments over MCP) and the cards show up live. The user
checks two, trims one word by word, and Make 2 reels writes two 9:16 projects as a job. What it checks is what the user
gets: the made projects' canvas and length, that they sit in the source's collection, that one trim burst is one undo
step, that a reel whose words moved fails with advice, and that Export… opens a made reel on Reels & TikTok."""
import json, subprocess, time
from pathlib import Path

from e2e.harness import AI_EDITING, CLI, FIXTURES, WINDOWS, Bridge, deny, flow, link_models, wait, webdriver
from e2e.reel import MODEL, poll

FAKE = Path(__file__).resolve().parent / 'fake_agent'
CANDIDATES = """const p = window.__nuzky.store.getState().snap.project;
return (p.reelCandidates ?? []).map((c) => ({id: c.id, from: c.from, to: c.to, durationUs: c.durationUs, status: c.status,
  score: c.score, title: c.title, hook: c.hook, projectPath: c.projectPath ?? null, pause: c.shortenPausesUs ?? null}));"""
# Clicks the button whose text or label is `arguments[0]` inside `arguments[1]`; null when there is none.
CLICK = """const scope = arguments[1] ? document.querySelector(arguments[1]) : document;
const b = [...(scope?.querySelectorAll('button') ?? [])].find((b) => b.textContent.trim() === arguments[0] || b.getAttribute('aria-label') === arguments[0]);
if (!b) return null; b.click(); return b.getAttribute('aria-disabled') ?? 'false';"""
LIST = 'section[aria-label="Reels from a long video"]'
CARD = f'{LIST} [data-reel="{{}}"]'
TRIM = 'aside[aria-label=Reel] [role=listbox]'


def setup(r):
    link_models(r, ('ggml-silero-v5.1.2.bin', f'ggml-{MODEL}.bin'))
    # A 16:9 talk just over three minutes: the three Czech takes said four times over a test picture.
    takes = [FIXTURES / f'reel-{n}.mp4' for n in (1, 2, 3)] * 4
    video = r.work / 'long-talk.mp4'
    inputs = [arg for take in takes for arg in ('-i', str(take))]
    sound = ''.join(f'[{n}:a]' for n in range(len(takes))) + f'concat=n={len(takes)}:v=0:a=1[a]'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', *inputs, '-f', 'lavfi', '-i', 'testsrc2=size=1920x1080:rate=30',
                    '-filter_complex', sound, '-map', f'{len(takes)}:v', '-map', '[a]', '-shortest',
                    '-c:v', 'libx264', '-preset', 'veryfast', '-pix_fmt', 'yuv420p', '-c:a', 'aac', str(video)], check=True)
    project = r.work / 'data/nuzky/projects/long-talk.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(video)], env=r.env, check=True, capture_output=True)
    # The fake stands in for Claude Code; Codex is deliberately missing, whatever this machine has.
    r.env.update(NUZKY_AGENT_CLAUDE=str(FAKE), NUZKY_AGENT_CODEX=str(r.work / 'no-codex'), NUZKY_FAKE_AGENT_LOG=str(r.work / 'fake'))


def candidates(r):
    return r.s.run(CANDIDATES, retries=3)


def text(r, selector):
    return r.s.run('return document.querySelector(arguments[0])?.innerText ?? null', selector)


def resize(r, width, height):
    webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
    return wait(lambda: r.s.run('return [window.innerWidth, window.innerHeight]') == [width, height], 5)


def project_length(path):
    """How long a saved project plays, as the engine counts it: its pictures and texts, else its sound."""
    tracks = json.loads(Path(path).read_text())['tracks']
    ends = lambda audio: [c['startUs'] + c['durationUs'] for t in tracks if (t['kind'] == 'audio') == audio for c in t['clips']]
    return max(ends(False) or ends(True) or [0])


@flow('reels', 'Reels from a long video: the Transcript tab opens the mode, Find reels shows the agent\'s moments live, two '
      'are checked and one trimmed by words, Make 2 reels writes two 9:16 projects of the right length into the source\'s collection',
      before=setup)
def reels(r):
    source = r.s.run('return window.__nuzky.store.getState().snap.path')
    # A talk recorded wide, as podcasts are.
    r.s.call("window.__nuzky.store.getState().edit({type: 'setCanvas', width: 1920, height: 1080, background: null, backgroundBlur: null})")
    made = r.s.call('window.__nuzky.api.createCollection("Podcast").then((c) => window.__nuzky.api.setCollection([arguments[0]], c.id).then(() => c.id))', source)
    collection = made['value']
    bridge = Bridge(r, r.saved_project())
    try:
        job = poll(bridge, bridge.call('transcribe', {'language': 'cs', 'model': MODEL}), 600)
        r.check('the talk is transcribed', job['status'] == 'done', job)
        find_and_pick(r, bridge)
    finally:
        bridge.close()
    style_changes(r)
    trim(r)
    make(r, source, collection)
    failure(r)
    moved(r)
    missing(r)
    export(r, source)
    from_home(r)
    r.check('no error toast', not r.errors(), r.errors())


def find_and_pick(r, bridge):
    r.s.run("window.__nuzky.store.setState({panelTab: 'transcript'})")
    r.check('a project over three minutes offers Reels in its Transcript tab', wait(lambda: r.s.run(CLICK, 'Reels…', 'aside') == 'false', 10))
    r.check('the mode opens in place of the timeline, without the inspector',
            wait(lambda: r.s.run(f"return !!document.querySelector('{LIST}') && !document.querySelector('aside[aria-label=Inspector]')"), 5))
    r.check('before any reels it says what Find reels does', wait(lambda: 'No reels yet' in (text(r, LIST) or ''), 10), text(r, LIST))
    r.shot('empty')

    r.s.run(CLICK, 'Find reels', LIST)
    r.check('Find reels asks the AI panel and says the agent is at work', wait(lambda: 'Finding reels' in (text(r, LIST) or ''), 10), text(r, LIST))
    r.check('the agent\'s four moments show up as cards', wait(lambda: len(candidates(r)) == 4 and r.s.run(
        f"return document.querySelectorAll('{LIST} [data-reel]').length") == 4, 30), candidates(r))
    order = r.s.run(f"return [...document.querySelectorAll('{LIST} [data-reel]')].map((c) => c.getAttribute('aria-label').split(',')[0])")
    r.check('the best moment comes first', order == ['Moment 1', 'Moment 2', 'Moment 3', 'Moment 4'], order)
    r.check('finding ends with the agent\'s answer', wait(lambda: not r.s.run('return window.__nuzky.reels.getState().finding'), 20))
    kept = r.s.run("""const box = document.querySelector('section[aria-label=Preview] .border-dashed');
        const frame = document.querySelector('section[aria-label=Preview] canvas');
        return box && frame && box.getBoundingClientRect().width / frame.getBoundingClientRect().width;""")
    r.check('Center crop marks the middle 9:16 of the wide picture', kept and abs(kept - 1080 * 9 / 16 / 1920) < 0.01, kept)

    run = bridge.call('begin_run', {'label': 'Look again'})['run_id']
    try:
        r.check('the UI knows an agent is editing', wait(lambda: r.state()['aiRun'], 10))
        locked = r.s.run(f"""const box = document.querySelector('{LIST} input[type=checkbox]');
            const make = [...document.querySelectorAll('{LIST} button')].find((b) => b.textContent.startsWith('Make'));
            return {{box: box.disabled, make: make.getAttribute('aria-disabled'), reason: make.title}};""")
        r.check('while the AI edits, checking and making are locked with the reason', locked['box'] and locked['make'] == 'true'
                and locked['reason'] == AI_EDITING, locked)
        r.shot('locked')
    finally:
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
    r.check('the lock ends with the run', wait(lambda: not r.state()['aiRun'], 10))

    first, second = candidates(r)[:2]
    for c in (first, second):
        r.s.run(f"document.querySelector('{CARD.format(c['id'])} input[type=checkbox]').click()")
    r.check('two reels are checked', wait(lambda: [c['status'] for c in candidates(r)][:2] == ['picked', 'picked'], 10), candidates(r))
    r.check('Make names how many', r.s.run(f"return [...document.querySelectorAll('{LIST} button')].some((b) => b.textContent === 'Make 2 reels')"))


def style_changes(r):
    """The creator's style changes its pause length after the reels were proposed without one."""
    r.check('each proposed reel keeps the pause it was measured with', all(c['pause'] == 300000 for c in candidates(r)), candidates(r))
    (r.work / 'data/nuzky/EDIT.md').write_text('# Editing style\n\n## Settings\n\n| Setting | Value |\n|---|---|\n'
                                               '| edit_transcript shorten_pauses_us | 1500000 |\n')
    c = candidates(r)[0]
    again = r.s.call('window.__nuzky.api.reelBounds(arguments[0], arguments[1], arguments[2]).then((b) => b.durationUs)',
                     c['id'], c['from'], c['to'])
    r.check('measuring a reel again keeps its pause after the style changed', again.get('value') == c['durationUs'], [again, c])


def trim(r):
    """Trims the second moment, which has words before it to grow into."""
    c = next(x for x in candidates(r) if x['title'] == 'Moment 2')
    now = lambda: next(x for x in candidates(r) if x['id'] == c['id'])
    r.s.run(f"document.querySelector('{CARD.format(c['id'])}').click()")
    r.s.run(f"document.querySelector('{TRIM}').focus()")
    for _ in range(2):
        r.key('ArrowLeft', shiftKey=True)
    shorter = wait(lambda: now() if now()['to'] == c['to'] - 2 else None, 10)
    r.check('Shift+← twice ends the reel two words earlier and it gets shorter', shorter and shorter['durationUs'] < c['durationUs'],
            [c, now()])
    header = text(r, 'aside[aria-label=Reel]') or ''
    r.check('the length updates in the trim pane', shorter and f"{shorter['durationUs'] / 1e6:.1f} s once cut" in header, header[:120])
    r.s.run('document.activeElement?.blur()')
    r.key('z', ctrlKey=True)
    r.check('one undo takes the burst back', wait(lambda: now()['to'] == c['to'], 10), now())
    r.key('z', ctrlKey=True, shiftKey=True)
    r.check('redo brings it again', wait(lambda: now()['to'] == c['to'] - 2, 10), now())

    # Shift-click a word before the reel: its start moves back there.
    word = r.s.run(f"""const w = document.querySelector('{TRIM} [data-w="{c['from'] - 3}"]');
        if (!w) return null; w.dispatchEvent(new PointerEvent('pointerdown', {{bubbles: true, button: 0, shiftKey: true}})); return w.textContent;""")
    longer = wait(lambda: now() if now()['from'] == c['from'] - 3 else None, 10)
    r.check('Shift-click on a word before the reel starts it there', word and longer and shorter and longer['durationUs'] > shorter['durationUs'],
            [word, now()])
    r.check('a trimmed reel stays checked', now()['status'] == 'picked', now())
    # How long measuring a trim takes on this three-minute talk, which the length waits for.
    took = r.s.call(f"""(async () => {{ const t = []; for (let i = 0; i < 5; i++) {{ const s = performance.now();
        await window.__nuzky.api.reelBounds(arguments[0], {c['from']}, {c['to']} - i); t.push(performance.now() - s); }}
        return t.map((x) => x.toFixed(1)).join(' '); }})()""", c['id'])
    print(f"  measuring a trim took {took.get('value')} ms", flush=True)
    r.shot('trimmed')


def make(r, source, collection):
    before = candidates(r)
    gets = r.s.run(f"return [...document.querySelectorAll('{LIST} button')].find((b) => b.textContent.startsWith('Each reel gets'))?.textContent ?? null")
    r.check('each reel gets every finishing step until the user turns one off', gets == 'Each reel gets all', gets)
    r.s.run(CLICK, 'Make 2 reels', LIST)
    done = wait(lambda: [c for c in candidates(r) if c['status'] == 'made'], 30)
    r.check('Make 2 reels marks both made', done and len(done) == 2, candidates(r))
    jobs = [j for j in r.state()['jobs'] if j['kind'] == 'reels']
    r.check('making ran as a job that finished', jobs and jobs[-1]['status'] == 'done', jobs)
    for c in done or []:
        path = Path(c['projectPath'])
        planned = next(x for x in before if x['id'] == c['id'])
        saved = json.loads(path.read_text()) if path.exists() else {}
        r.check(f'{path.name} is a 9:16 project', saved.get('canvas', {}).get('width') == 1080 and saved['canvas']['height'] == 1920,
                saved.get('canvas'))
        r.check(f'{path.name} is as long as its card says once made', path.exists() and project_length(path) == c['durationUs'],
                [project_length(path) if path.exists() else None, c['durationUs']])
        captions = [clip['content']['text'] for t in saved.get('tracks', []) if t.get('name') == 'Captions' for clip in t['clips']]
        r.check(f'{path.name} has captions of its words', len(captions) > 3, captions[:5])
        if c['title'] == 'Moment 1':
            # It opens on two attempts at "Dneska vám ukážu, jak natočit…" before the one said in full.
            said = ' '.join(captions).lower()
            r.check('the retakes it opened on are cut, only the attempt said in full stays', said.count('dneska') == 1
                    and project_length(path) < planned['durationUs'], [said[:120], project_length(path), planned['durationUs']])
            r.check('its card names the sentence it opens with now', 'video za' in c['hook'] and 'video za' not in planned['hook'],
                    [planned['hook'], c['hook']])
        r.check(f'{path.name} sits beside the source', path.parent == Path(source).parent, [str(path), source])
    r.s.call('window.__nuzky.refreshLibrary()')
    members = r.s.run('return (window.__nuzky.library.getState().projects ?? []).filter((p) => p.collection === arguments[0]).map((p) => p.path)', collection)
    r.check("the made reels are in the source's collection", all(c['projectPath'] in members for c in done or []) and source in members, members)
    r.check('a toast says how many were made', any(t['text'] == 'Made 2 reels' for t in r.state()['toasts']), r.state()['toasts'])
    r.shot('made')
    if resize(r, 1024, 640):
        time.sleep(0.5)
        fits = r.s.run(f"""const pane = document.querySelector('{LIST}').getBoundingClientRect();
            return [...document.querySelectorAll('{LIST} button')].filter((b) => b.offsetParent).every((b) => b.getBoundingClientRect().right <= pane.right + 0.5);""")
        r.check('at the smallest window every control of the list stays inside it', fits)
        r.shot('small')
        resize(r, 1440, 900)


def pick(r, title):
    c = next(c for c in candidates(r) if c['title'] == title)
    r.s.run(f"document.querySelector('{CARD.format(c['id'])} input[type=checkbox]').click()")
    wait(lambda: next(x for x in candidates(r) if x['id'] == c['id'])['status'] == 'picked', 10)
    return c


def failure(r):
    third = pick(r, 'Moment 3')
    wait(lambda: r.s.run('return window.__nuzky.store.getState().saveState') == 'saved', 10)
    # The folder the reels go into cannot be written to, as on a full or locked disk.
    folder = Path(r.s.run('return window.__nuzky.store.getState().snap.path')).parent
    before = sorted(folder.iterdir())
    allow = deny(folder, 0o555)
    try:
        r.s.run(CLICK, 'Make 1 reel', LIST)
        said = wait(lambda: text(r, f'{LIST} [role=alert]'), 20)
        r.check('a make that fails says so under the list', said and said.startswith("Couldn't save the reels") and 'Free some space' in said, said)
        r.check('it leaves no file and the reel is not made', sorted(folder.iterdir()) == before
                and next(c for c in candidates(r) if c['id'] == third['id'])['status'] == 'picked', candidates(r))
        time.sleep(0.3)
        r.shot('failed')
    finally:
        allow()


def moved(r):
    fourth = pick(r, 'Moment 4')
    # The user cuts a word out of that moment meanwhile: its words no longer play where they did.
    cut = r.s.call("""window.__nuzky.api.transcriptView(500000).then((v) => window.__nuzky.api.cutWords(v.key, [[arguments[0], arguments[0]]], [],
        window.__nuzky.store.getState().snap.sessionEpoch)).then((t) => { window.__nuzky.store.getState().setSnap(t.snapshot); return 'cut'; })""",
                   fourth['from'] + 1)
    r.check('a word of the fourth moment is cut out', cut.get('value') == 'cut', cut)
    card = CARD.format(fourth['id'])
    r.check('its card says its words moved and what to do', wait(lambda: 'Find again' in (text(r, card) or ''), 10), text(r, card))
    r.check('and it can no longer be checked or made', r.s.run(f"return document.querySelector('{card} input[type=checkbox]').disabled")
            and r.s.run(f"return [...document.querySelectorAll('{LIST} button')].some((b) => b.textContent === 'Make 1 reel')"),
            text(r, LIST))
    time.sleep(0.3)
    r.shot('moved')


def missing(r):
    video, moved = r.work / 'long-talk.mp4', r.work / 'moved.mp4'
    try:
        if WINDOWS:
            if not move_while_away(r, video, moved):
                return
        else:
            video.rename(moved)
        r.s.call('window.__nuzky.refreshLibrary()')
        said = wait(lambda: 'missing' in (text(r, f'{LIST} [role=status]') or '') and text(r, f'{LIST} [role=status]'), 10)
        r.check('with its video moved away, the list says to put it back', said and 'Put it back' in said, said)
        make = r.s.run(f"const b = [...document.querySelectorAll('{LIST} button')].find((b) => b.textContent.startsWith('Make')); return b && [b.getAttribute('aria-disabled'), b.title];")
        r.check('and Make waits with that reason', make and make[0] == 'true' and 'missing' in make[1], make)
        time.sleep(0.3)
        r.shot('missing')
    finally:
        if moved.exists():
            moved.rename(video)
        r.s.call('window.__nuzky.refreshLibrary()')


def move_while_away(r, video, moved):
    """Windows does not let Explorer move a video Nuzky has open, so there the user moves it while another project is
    open and comes back to the reels of the talk."""
    source = r.s.run('return window.__nuzky.store.getState().snap.path')
    r.s.run("window.__nuzky.store.setState({view: 'home'})")
    r.s.run("[...document.querySelectorAll('button')].find((b) => b.textContent.trim().startsWith('New project')).click()")
    wait(lambda: r.s.run("const i = [...document.querySelectorAll('[role=menuitem]')].find((i) => i.textContent.startsWith('Empty 16:9'));"
                         "i?.click(); return !!i"), 5)
    if not r.check('an empty project opens in place of the talk',
                   wait(lambda: r.s.run('return window.__nuzky.store.getState().snap.path') not in (None, source), 20)):
        return False
    # Drawing posters on Home keeps the video open for up to 20 s more.
    if not r.check('then the video can be moved', wait(lambda: video.rename(moved), 30)):
        return False
    r.s.run('window.__nuzky.openProject({path: arguments[0], name: "long-talk"})', source)
    r.check('the talk opens again', wait(lambda: r.s.run('return window.__nuzky.store.getState().snap.path') == source, 20))
    r.s.run("window.__nuzky.store.setState({panelTab: 'transcript'})")
    wait(lambda: r.s.run(CLICK, 'Reels…', 'aside') == 'false', 10)
    r.check('with its reels', wait(lambda: r.s.run('return window.__nuzky.reels.getState().open'), 10))
    return True


def export(r, source):
    reel = next(c for c in candidates(r) if c['title'] == 'Moment 2')
    r.s.run(f"[...document.querySelectorAll('{CARD.format(reel['id'])} button')].find((b) => b.textContent === 'Export…').click()")
    r.check('Export… opens the made reel', wait(lambda: r.s.run('return window.__nuzky.store.getState().snap.path') == reel['projectPath'], 20))
    r.check('the mode closes with the source', wait(lambda: not r.s.run('return window.__nuzky.reels.getState().open'), 5))
    dialog = wait(lambda: r.s.run("""const d = [...document.querySelectorAll('[role=dialog]')].find((d) => d.innerText.includes('Preset'));
        return d ? d.innerText : null;"""), 10)
    r.check('with the Export dialog on Reels & TikTok', dialog and 'Reels & TikTok' in dialog, dialog)
    # Past the dialog's fade and the reel's first frame.
    time.sleep(1)
    r.shot('export')
    r.key('Escape')
    r.check('the export dialog closes', wait(lambda: not r.s.run('return window.__nuzky.store.getState().exportOpen'), 5))
    r.check('a toast offers the way back', r.s.run(CLICK, 'Back to reels', None) == 'false')
    r.check("Back to reels opens the long video's reels again", wait(lambda: r.s.run(
        'return window.__nuzky.store.getState().snap.path === arguments[0] && window.__nuzky.reels.getState().open', source), 20))


def from_home(r):
    video = str(r.work / 'long-talk.mp4')
    r.s.run("window.__nuzky.store.setState({view: 'home'})")
    r.s.run("[...document.querySelectorAll('button')].find((b) => b.textContent.trim().startsWith('New project')).click()")
    menu = wait(lambda: r.s.run("return [...document.querySelectorAll('[role^=menuitem]')].map((m) => m.textContent)"), 5)
    r.check('New project offers Reels from a long video', any(m.startswith('Reels from a long video…') for m in menu or []), menu)
    r.key('Escape')
    # The native file picker cannot be driven; the video it would return starts the same way.
    r.s.run('window.__nuzky.newProjectFromMedia([arguments[0]], window.__nuzky.openReelsToFind)', video)
    r.check('a new project of the video opens straight in the mode', wait(lambda: r.s.run(
        "return window.__nuzky.reels.getState().open && window.__nuzky.store.getState().view === 'editor'"), 20))
    r.check('with the keyboard on Find reels', wait(lambda: r.s.run("return document.activeElement?.hasAttribute('data-find-reels')"), 5),
            r.s.run('return document.activeElement?.outerHTML.slice(0, 120)'))

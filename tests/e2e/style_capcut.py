"""Learn my style from CapCut: the dialog finds the drafts in CapCut's folder of an isolated home, lists them by name
and date, reads a chosen one at once with what it misses (an encrypted JianYing draft says what to do instead), takes a
draft folder added by hand, learns as a job and shows EDIT.md as `nuzky style learn --from <draft>` writes it, with
the captions' look and the zoom read from the draft."""
import json, re, subprocess, time

from pathlib import Path

from e2e.harness import CLI, MACOS, MODELS, WINDOWS, flow, link_models, wait
from e2e.home import CLICK_TEXT, resize
from e2e.style import DIALOG, PAGE, edit_md, record, view

# Where Nuzky looks for CapCut's drafts in the home folder (crates/interchange/src/capcut.rs).
DRAFTS = ('AppData/Local' if WINDOWS else 'Movies') + '/CapCut/User Data/Projects/com.lveditor.draft'
STATE = """const s = window.__nuzky.style.getState(); return {drafts: s.capcutDrafts, chosen: s.capcut};"""
JOB = """const l = window.__nuzky.style.getState().projectsLearning; if (!l) return null;
const j = window.__nuzky.store.getState().jobs[l.jobId];
return j && {tab: l.tab, status: j.status, phase: j.phase, output: j.output, results: l.results, message: j.message};"""
DIALOG_TEXT = f"return document.querySelector('{DIALOG}')?.innerText ?? ''"
PREVIEW = f"return document.querySelector('{DIALOG} section[aria-label=\"EDIT.md as learned\"] pre')?.textContent ?? null"
ROW = f"""return [...document.querySelectorAll('{DIALOG} ul[aria-label="CapCut drafts"] li')]
  .find((li) => li.innerText.startsWith(arguments[0]))?.innerText ?? null"""
CHECK = f"""const box = [...document.querySelectorAll('{DIALOG} ul[aria-label="CapCut drafts"] input[type=checkbox]')]
  .find((c) => c.getAttribute('aria-label') === arguments[0]); box.focus(); return !!box;"""
WORDS = ['Hello there', 'big zoom', 'two words', 'short cut', 'fast pace', 'keep going', 'nice edit']


def draft(folder, name, recording, pieces, modified_s, zoom_first=True):
    """A CapCut draft as CapCut for macOS writes it, cut down to what Nuzky reads: the pieces on the main track, every
    other one zoomed in to 1.25 and the first a slow push in, keyed in the recording's own time, and a two-word
    caption every half second in yellow Montserrat Bold."""
    folder.mkdir(parents=True)
    segments, at = [], 0
    for k, (start, end) in enumerate(pieces):
        source, length = round(start * 1e6), round(end * 1e6) - round(start * 1e6)
        scale = 1.25 if k % 2 else 1.0
        segment = {'material_id': 'talk', 'target_timerange': {'start': at, 'duration': length},
                   'source_timerange': {'start': source, 'duration': length}, 'speed': 1.0, 'volume': 1.0,
                   'clip': {'scale': {'x': scale, 'y': scale}, 'transform': {'x': 0.0, 'y': 0.1}, 'rotation': 0.0}}
        if k == 0 and zoom_first:
            segment['common_keyframes'] = [{'property_type': 'KFTypeScaleX', 'keyframe_list': [
                {'time_offset': source, 'values': [1.0], 'curveType': 'Line'},
                {'time_offset': source + length, 'values': [1.15], 'curveType': 'Line'}]}]
        segments.append(segment)
        at += length
    texts, captions = [], []
    for i in range(at // 500_000):
        words = WORDS[i % len(WORDS)]
        style = {'fill': {'content': {'solid': {'color': [1.0, 0.941, 0.349]}}}, 'size': 12, 'range': [0, len(words)],
                 'font': {'path': '/Users/creator/Library/Containers/com.lemon.lvoverseas/Data/Library/Fonts/Montserrat-Bold.otf'}}
        texts.append({'id': f'text{i}', 'type': 'subtitle', 'content': json.dumps({'text': words, 'styles': [style]})})
        captions.append({'material_id': f'text{i}', 'target_timerange': {'start': i * 500_000, 'duration': 500_000},
                         'clip': {'scale': {'x': 1.5, 'y': 1.5}, 'transform': {'x': 0.0, 'y': -0.3}, 'rotation': 0.0}})
    content = {'fps': 30.0, 'canvas_config': {'width': 540, 'height': 960, 'ratio': 'original'},
               'platform': {'app_source': 'cc', 'app_version': '9.5.0', 'os': 'mac'},
               'tracks': [{'type': 'video', 'attribute': 0, 'segments': segments},
                          {'type': 'text', 'attribute': 0, 'segments': captions}],
               'materials': {'videos': [{'id': 'talk', 'type': 'video', 'path': str(recording), 'material_name': recording.name}],
                             'texts': texts}}
    (folder / 'draft_info.json').write_text(json.dumps(content))
    meta = {'draft_name': name, 'tm_draft_modified': int(modified_s * 1e6), 'tm_duration': at}
    (folder / 'draft_meta_info.json').write_text(json.dumps(meta))


def setup(r):
    if not (MODELS / 'ggml-small.bin').exists():
        raise RuntimeError('run scripts/fixtures.sh first, missing the speech models')
    link_models(r)
    recording, pieces = record(r.work / 'media')
    # macOS gives every flow a home of its own; on Linux this flow takes one too, where CapCut's folder is looked for.
    home = Path(r.env['HOME']) if MACOS else r.work / 'home'
    r.env['HOME'] = str(home)
    root = home / DRAFTS
    now = time.time()
    r.reel = root / '0928'
    draft(r.reel, 'Talking reel', recording, pieces, now - 3600)
    draft(root / '0915', 'Shared reel', r.work / 'gone/their-talk.mp4', pieces[:3], now - 86400 * 3)
    (root / '1009').mkdir(parents=True)
    (root / '1009/draft_content.json').write_text('Hk3vQ0bS8vZ2Yz4uXw9pR1tL7mN5kJ2hG6fD0sA8qW3eR5tY7uI9oP1aS3dF5gH7jK9lZ1xC3vB5nM7')
    (root / '1009/draft_meta_info.json').write_text(json.dumps({'draft_name': 'From JianYing', 'tm_draft_modified': int(now * 1e6), 'tm_duration': 5_000_000}))
    (root / '0825').mkdir()
    (root / '0825/draft_info.json').write_text(json.dumps({'tracks': [], 'materials': {}}))
    (root / '0825/draft_meta_info.json').write_text(json.dumps({'draft_name': 'Empty', 'tm_draft_modified': int(now * 1e6), 'tm_duration': 0}))
    # A draft kept elsewhere, the same recording cut shorter, added by hand.
    r.elsewhere = r.work / 'Backup/Short reel'
    draft(r.elsewhere, 'Short reel', recording, pieces[:4], now - 86400 * 30, zoom_first=False)
    r.cuts = len(pieces)


def job(r):
    return r.s.run(JOB)


def shot(r, name):
    """A screenshot once the dialog has faded in or changed."""
    time.sleep(0.5)
    r.shot(name)


@flow('style-capcut', 'Learn my style from CapCut drafts: found in CapCut\'s folder of an isolated home, listed by name and date, '
                      'chosen with what each misses, an encrypted draft explained, a folder added by hand, the job and EDIT.md '
                      'as `nuzky style learn --from <draft>` writes it', before=setup, home=True)
def style_capcut(r):
    r.s.run("document.querySelector('nav[aria-label=Collections] button[data-row]:nth-of-type(2)').click()")
    r.check('Your style opens from the sidebar', wait(lambda: r.s.run(f"return !!document.querySelector('{PAGE}')"), 10))
    r.s.run(CLICK_TEXT, 'Learn my style', PAGE)
    r.check('Learn my style opens its dialog', wait(lambda: r.s.run(f"return !!document.querySelector('{DIALOG}')"), 5))
    # The keyboard moves to the last tab.
    r.s.run(f"document.querySelector('{DIALOG} [role=tab][aria-selected=true]').focus()")
    r.s.run(f"document.querySelector('{DIALOG} [role=tablist]').dispatchEvent(new KeyboardEvent('keydown', {{key: 'End', bubbles: true}}))")
    tab = f"return document.querySelector('{DIALOG} [role=tab][aria-selected=true]')?.textContent"
    r.check('End opens the CapCut tab', wait(lambda: r.s.run(tab) == 'CapCut', 5), r.s.run(tab))

    found = wait(lambda: (s := r.s.run(STATE)) and s['drafts'] is not None and s['drafts'], 10)
    names = [d['name'] for d in found or []]
    r.check('it finds the drafts in CapCut\'s folder of this home, newest first, without the empty one',
            names == ['From JianYing', 'Talking reel', 'Shared reel'], found)
    row = (r.s.run(ROW, 'Talking reel') or '').split('\n')
    r.check('each draft shows its name, when it was last changed and its length',
            len(row) == 3 and row[0] == 'Talking reel' and re.search(r'\d:\d\d', row[1]) and re.fullmatch(r'0:\d\d', row[2]), row)
    shot(r, 'capcut-drafts')

    # Choosing from the keyboard: the checkbox has the draft's name, Space ticks it.
    r.check('a draft\'s checkbox takes the focus by its name', r.s.run(CHECK, 'Talking reel'))
    r.s.run("document.activeElement.click()")
    reel = wait(lambda: next((p for p in r.s.run(STATE)['chosen'] if p['name'] == 'Talking reel'), None), 10)
    r.check('a chosen draft is read at once: its recording in its pieces', reel and [(x['name'], x['clips'], x['missing']) for x in reel['recordings']] == [('talk.mp4', r.cuts, None)], reel)
    r.check('and the row says so', wait(lambda: f'talk.mp4 · {r.cuts} clips' in (r.s.run(ROW, 'Talking reel') or ''), 5), r.s.run(ROW, 'Talking reel'))
    r.s.run(CHECK, 'Shared reel')
    r.s.run("document.activeElement.click()")
    r.check('a draft whose recording is elsewhere says it is not on this computer',
            wait(lambda: 'their-talk.mp4: not on this computer' in (r.s.run(ROW, 'Shared reel') or ''), 10), r.s.run(ROW, 'Shared reel'))
    r.s.run(CHECK, 'From JianYing')
    r.s.run("document.activeElement.click()")
    r.check('an encrypted draft says why it cannot be read and what to do instead',
            wait(lambda: 'encrypts its drafts' in (r.s.run(ROW, 'From JianYing') or '') and 'Learn from the exported video' in r.s.run(ROW, 'From JianYing'), 10),
            r.s.run(ROW, 'From JianYing'))
    shot(r, 'capcut-chosen')
    r.s.run(CHECK, 'From JianYing')
    r.s.run("document.activeElement.click()")
    r.check('unticking leaves it out', wait(lambda: [p['name'] for p in r.s.run(STATE)['chosen']] == ['Talking reel', 'Shared reel'], 5), r.s.run(STATE)['chosen'])

    # Adding a folder: the native dialog cannot be driven, so the flow hands its path to what it calls.
    r.s.call('window.__nuzky.findDrafts(arguments[0])', str(r.elsewhere))
    r.check('a draft folder added by hand joins the list, chosen',
            wait(lambda: 'Short reel' in [p['name'] for p in r.s.run(STATE)['chosen']], 10), r.s.run(STATE))

    (r.work / 'Nothing here').mkdir()
    r.s.call('window.__nuzky.findDrafts(arguments[0])', str(r.work / 'Nothing here'))
    note = "return window.__nuzky.store.getState().toasts.find((t) => t.text.startsWith('No CapCut drafts in this folder'))?.text ?? null"
    r.check('a folder without drafts says so', wait(lambda: r.s.run(note), 5), r.state()['toasts'])

    before = job(r)
    r.s.run(CLICK_TEXT, 'Learn', f'{DIALOG} div.border-t')
    bar = f"return document.querySelector('{DIALOG} [role=progressbar]')?.getAttribute('aria-label') ?? null"
    r.check('learning runs as a job, a row showing its phase over a progress bar',
            wait(lambda: (j := job(r)) and j != before and j['status'] == 'running' and r.s.run(bar), 30), job(r))
    shot(r, 'capcut-learning')
    # Closed, the dialog comes back from the top bar on the tab that learns.
    r.s.run("document.activeElement.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true, cancelable: true}))")
    wait(lambda: not r.s.run(f"return !!document.querySelector('{DIALOG}')"), 5)
    r.s.run("document.querySelector('button[title=\"Show learning\"]').click()")
    r.check('the top bar opens learning on the CapCut tab', wait(lambda: r.s.run(tab) == 'CapCut' and r.s.run(bar), 5), r.s.run(tab))
    # Choosing on another tab leaves this learning alone.
    r.s.run(f"[...document.querySelectorAll('{DIALOG} [role=tab]')].find((t) => t.textContent === 'Projects (.otio)').click()")
    r.s.call('window.__nuzky.addProjects(arguments[0])', [str(r.work / 'gone.otio')])
    remove = f"""const b = document.querySelector('{DIALOG} button[aria-label="Remove gone"]'); if (!b) return false; b.click(); return true;"""
    r.check('a project can be chosen and removed on the other tab meanwhile', wait(lambda: r.s.run(remove), 5))
    owner = "return window.__nuzky.style.getState().projectsLearning?.tab ?? null"
    r.check('and the CapCut learning stays, Stop and all', r.s.run(owner) == 'capcut', r.s.run(owner))
    r.s.run(f"[...document.querySelectorAll('{DIALOG} [role=tab]')].find((t) => t.textContent === 'CapCut').click()")
    r.check('learning ends', wait(lambda: (j := job(r)) and j['status'] != 'running', 600), job(r))
    result = job(r)
    chosen = [p['path'] for p in r.s.run(STATE)['chosen']]
    outcome = {Path(path).name: x for path, x in zip(chosen, result['results'])}
    r.check('the reel and the draft added by hand teach their recording; the shared one says why not',
            result['status'] == 'done' and outcome['0928']['learned'] == outcome['Short reel']['learned'] == ['talk.mp4']
            and outcome['0915']['learned'] == [] and outcome['0915']['skipped'][0][0] == 'their-talk.mp4', result)
    preview = wait(lambda: r.s.run(PREVIEW), 5)
    r.check('the dialog shows EDIT.md as learned', preview and preview.startswith('# Editing style') and '| Talking reel |' in preview, (preview or '')[:300])
    r.check('the captions\' look comes from the draft', preview and '| build_captions style |' in preview and 'Montserrat bold' in preview and '#fff059' in preview, preview)
    r.check('and the zoom too, not measured in pixels', preview and 'Captions and zoom not measured' not in preview and '## Zoom' in preview, preview)
    cli = r.work / 'cli.md'
    subprocess.run([str(CLI), 'style', 'learn', *sum((['--from', p] for p in chosen), []), '--out', str(cli)], env=r.env, check=True, capture_output=True)
    r.check('it is what nuzky style learn --from writes from the same drafts', preview == cli.read_text(), {'app': len(preview or ''), 'cli': len(cli.read_text())})
    shot(r, 'capcut-learned')
    if r.check('the window goes to its smallest size', resize(r, 1024, 640)):
        shot(r, 'capcut-learned-1024')
        box = r.s.run(f"""const d = document.querySelector('{DIALOG}').getBoundingClientRect();
            const b = [...document.querySelectorAll('{DIALOG} button')].find((x) => x.textContent === 'Use as my style').getBoundingClientRect();
            return [d.top >= 0 && d.bottom <= innerHeight, b.bottom <= d.bottom]""")
        r.check('the dialog and its buttons fit the smallest window', box == [True, True], box)
        r.s.run(CLICK_TEXT, 'Learn from more', DIALOG)
        r.check('Learn from more goes back to the drafts, still chosen', wait(lambda: r.s.run(ROW, 'Talking reel') is not None, 5))
        shot(r, 'capcut-drafts-1024')
        resize(r, 1440, 900)
        r.s.run(CLICK_TEXT, 'Learn', f'{DIALOG} div.border-t')
        wait(lambda: (j := job(r)) and j['status'] == 'done' and r.s.run(PREVIEW), 600)

    r.s.run(CLICK_TEXT, 'Use as my style', DIALOG)
    r.check('Use as my style writes EDIT.md as shown', wait(lambda: edit_md(r) == preview, 10), (edit_md(r) or '')[:200])
    # What was learned is kept just after EDIT.md is written.
    learned = lambda: any(s['kind'] == 'timeline' and s['title'] == 'talk.mp4 in Talking reel' for s in view(r)['sources'])
    r.check('the style says where it was learned', wait(learned, 10), view(r)['sources'])
    r.check('no error toast', not r.errors(), r.errors())

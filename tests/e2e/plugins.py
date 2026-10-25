import json
import shutil
import subprocess
import time
from pathlib import Path

from e2e.harness import CLI, FIXTURES, ROOT, Bridge, export, flow, leave_editor, wait

EXAMPLE = ROOT / 'skills/nuzky-plugin/examples/remove-short-clips'
ROW = "document.querySelector('[data-plugin=\"remove-short-clips\"]')"
MARKS = """return [...document.querySelectorAll('[data-clip-id][data-peek="gone"]')]
    .map((c) => ({id: c.dataset.clipId, text: c.textContent}));"""


def prepare(r):
    plugins = r.work / 'data/nuzky/plugins'
    shutil.copytree(EXAMPLE, plugins / 'remove-short-clips')
    broken = plugins / 'broken'
    broken.mkdir()
    (broken / 'plugin.json').write_text('{invalid json')
    r.env['NUZKY_OPENED_URLS'] = str(r.work / 'opened-folders.txt')


def project(r):
    return r.s.run('return window.__nuzky.store.getState().snap.project')


def saved(r):
    return json.loads(r.saved_project().read_text())


@flow('plugins', 'Plugins list broken entries, preview removed clips without editing, then apply and save one undo '
      'step; plugins saved by an agent appear live and removed plugins leave the list', before=prepare)
def plugins(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    r.s.run('window.__nuzky.store.getState().setZoom(140)')
    for at in (600_000, 1_400_000):
        clip = r.track()[-1]
        r.focus_clip(clip['id'])
        r.seek(at)
        count = len(r.track())
        r.key('s')
        if not wait(lambda: len(r.track()) == count + 1, 5):
            raise RuntimeError('the short clip was not split')
    before = project(r)
    original = r.track()
    short = [c['id'] for c in original if c['durationUs'] < 1_000_000]
    r.check('the main track has two short clips and one longer clip', len(short) == 2 and len(original) == 3, original)
    r.s.run('window.__nuzky.store.getState().select([])')
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((b) => b.textContent === 'Plugins').click()")
    r.check('the Plugins tab loads the installed plugin', wait(lambda: r.s.run(f'return !!{ROW}'), 10))
    broken = r.s.run("""const row = document.querySelector('[data-plugin="broken"]')?.closest('li');
        return row && {text: row.textContent, fix: [...row.querySelectorAll('button')].some((b) => b.textContent.includes('Fix with AI'))};""")
    r.check('the broken plugin says it can\'t load and why, without the internal code, with Fix with AI',
            broken and "Can’t load: plugin.json:" in broken['text'] and 'PLUGIN_' not in broken['text'] and broken['fix'], broken)
    r.shot('list')

    r.s.run(f'{ROW}.click()')
    preview = wait(lambda: r.s.run("return document.body.textContent.includes('Removes 2 clips.')"), 10)
    settings = r.s.run("""return {shorter: !!document.querySelector('input[type=range][aria-label="Shorter than"]'),
        tracks: document.body.textContent.includes('Main track') && document.body.textContent.includes('Every track')};""")
    r.check('the inspector shows the plugin settings', settings['shorter'] and settings['tracks'], settings)
    r.check('the preview says it removes two clips', preview)
    marks = r.s.run(MARKS)
    r.check('only the short clips carry a Removed mark on the timeline',
            sorted(m['id'] for m in marks) == sorted(short) and all('Removed' in m['text'] for m in marks), marks)
    r.check('preview leaves the entire project unchanged', project(r) == before)
    r.check('preview also leaves the saved clips unchanged',
            wait(lambda: saved(r)['tracks'] == before['tracks'], 5))
    r.shot('preview')

    r.s.run("[...document.querySelectorAll('button')].find((b) => b.textContent === 'Remove clips').click()")
    expected = [dict(c, startUs=c['startUs'] - 1_400_000) for c in original if c['id'] not in short]
    changed = wait(lambda: r.track() == expected, 10)
    r.check('running removes the short clips and closes their gaps', changed, r.track())
    after = project(r)
    r.check('running changes no other project content', changed and after == dict(before, tracks=[
        dict(t, clips=[dict(c, startUs=c['startUs'] - 1_400_000) for c in t['clips'] if c['id'] not in short])
        if t['id'] == 'main' else t for t in before['tracks']]))
    r.check('the saved project has exactly the remaining clips',
            changed and wait(lambda: saved(r)['tracks'] == after['tracks'], 5), saved(r)['tracks'])
    r.check('the plugin message reaches the success toast', wait(lambda: any(
        t['kind'] == 'success' and t['text'] == 'Removed 2 clips.' for t in r.state()['toasts']), 5), r.state()['toasts'])
    r.check('the finished inspector says there is nothing else to remove', wait(lambda:
        r.s.run("return document.body.textContent.includes('Nothing left to change.')"), 5))
    r.shot('done')

    r.s.call('window.__nuzky.store.getState().undo()')
    r.check('one undo restores the entire project exactly', wait(lambda: project(r) == before, 5), project(r))
    r.check('undo also restores the saved clips exactly', wait(lambda: saved(r)['tracks'] == before['tracks'], 5))
    r.s.run("document.querySelector('[aria-label=\"Close plugin\"]').click()")
    bridge = Bridge(r, r.saved_project())
    try:
        manifest = json.loads((EXAMPLE / 'plugin.json').read_text())
        manifest['name'] = 'Updated short clips'
        bridge.call('save_plugin', {'manifest': manifest, 'script': (EXAMPLE / 'plugin.js').read_text()})
        r.check('an agent saving the plugin refreshes the open list',
                wait(lambda: r.s.run(f"return {ROW}.textContent.includes('Updated short clips')"), 5))
        manifest['id'] = 'command-check'
        bridge.call('save_plugin', {'manifest': manifest, 'script': '''function run(ctx) {
            if (ctx.inputs.minSeconds === 1) { while (true) {} }
            if (ctx.inputs.minSeconds === 2) throw new Error("broken");
            if (ctx.inputs.minSeconds === 3) return [{type: "moveClip", clipId: "missing", trackId: "main", startUs: 0}];
            return {edits: [], message: "Nothing to do"};
        }'''})
        invalid = r.s.call("window.__nuzky.api.previewPlugin('command-check', {minSeconds: 3}, [], 0)")
        r.check('an invalid preview rejects with the validation error',
                not invalid['ok'] and invalid['error'].startswith('PLUGIN_BAD_EDIT:'), invalid)
        epoch = r.state()['epoch']
        stale = r.s.call("window.__nuzky.api.runPlugin('command-check', {minSeconds: 4}, [], 0, 'old-project')")
        r.check('a run for an old project is refused', not stale['ok'] and stale['error'].startswith('EPOCH_CHANGED:'), stale)
        failed = r.s.call("window.__nuzky.api.runPlugin('command-check', {minSeconds: 2}, [], 0, arguments[0])", epoch)
        r.check('a failed run preserves the script error and its line',
                not failed['ok'] and failed['error'].startswith('PLUGIN_FAILED: plugin.js:') and 'broken' in failed['error'], failed)
        noop = r.s.call("window.__nuzky.api.runPlugin('command-check', {minSeconds: 4}, [], 0, arguments[0]).then(JSON.stringify)", epoch)
        r.check('a run with no edits returns no snapshot and its message',
                noop['ok'] and json.loads(noop['value']) == {'snapshot': None, 'message': 'Nothing to do'}, noop)
        r.s.run("window.pluginPreview = window.__nuzky.api.previewPlugin('command-check', {minSeconds: 1}, [], 0).catch(String)")
        r.s.call("window.__nuzky.api.previewPlugin('command-check', {minSeconds: 4}, [], 0)")
        cancelled = r.s.call('window.pluginPreview')
        r.check('a new preview cancels a stuck older preview',
                cancelled['ok'] and cancelled['value'].startswith('CANCELLED:'), cancelled)
        r.s.run("window.pluginRun = window.__nuzky.api.runPlugin('command-check', {minSeconds: 1}, [], 0, arguments[0]).catch(String)", epoch)
        r.check('a running plugin shows the AI lock', wait(lambda: r.state()['aiRun'], 5))
        r.s.call('window.__nuzky.api.stopRun(arguments[0])', epoch)
        stopped = r.s.call('window.pluginRun')
        r.check('Stop ends a stuck plugin and preserves the project',
                stopped['ok'] and stopped['value'].startswith(('CANCELLED:', 'RUN_STOPPED:')) and project(r) == before, stopped)
    finally:
        bridge.close()
    rejected = r.s.call("window.__nuzky.api.removePlugin('../projects')")
    r.check('removing a folder outside plugins is refused', not rejected['ok'] and rejected['error'].startswith('PLUGIN_NOT_FOUND:'), rejected)
    r.s.run("document.querySelector('[aria-label=\"More for Broken\"]').click()")
    r.s.run("[...document.querySelectorAll('[role=menuitem]')].find((b) => b.textContent.includes('Move to Trash')).click()")
    r.check('Move to Trash removes the broken plugin folder and row', wait(lambda:
        not (r.work / 'data/nuzky/plugins/broken').exists()
        and r.s.run("return !document.querySelector('[data-plugin=broken]')"), 5))
    for plugin in (None, 'remove-short-clips'):
        r.s.call('window.__nuzky.api.showPluginFolder(arguments[0])', plugin)
    folders = (r.work / 'opened-folders.txt').read_text().splitlines()
    base = r.work / 'data/nuzky/plugins'
    r.check('Show folder opens the plugins folder or the chosen plugin folder', folders == [str(base), str(base / 'remove-short-clips')], folders)
    r.check('no error toast', not r.errors(), r.errors())


PAUSES = ROOT / 'skills/nuzky-plugin/examples/remove-pauses'
PAUSES_ROW = "document.querySelector('[data-plugin=\"remove-pauses\"]')"


SLIDE = """const input = document.querySelector(`input[type=range][aria-label="${arguments[0]}"]`);
if (!input) return false;
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, String(arguments[1]));
input.dispatchEvent(new Event('input', {bubbles: true}));
return true;"""


def prepare_pauses(r):
    shutil.copytree(PAUSES, r.work / 'data/nuzky/plugins/remove-pauses')


def cuts(tracks):
    """Where each clip sits and what of its file it plays; the pieces of a cut get ids of their own."""
    return [[(c['startUs'], c['durationUs'], c['content'].get('sourceInUs')) for c in t['clips']] for t in tracks]


@flow('plugin_pauses', 'A plugin that needs the silences of the sound previews and cuts the pauses of a trimmed '
      'talking head where the timeline has them, as one undo step', before=prepare_pauses)
def plugin_pauses(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    # Trimmed, so the timeline and the file disagree about where each pause is.
    clip = r.track()[0]
    trimmed = r.s.call('window.__nuzky.store.getState().edit(arguments[0])',
                       {'type': 'trimClip', 'clipId': clip['id'], 'startUs': 0,
                        'durationUs': clip['durationUs'] - 700_000, 'sourceInUs': 700_000})
    r.check('the take starts 0.7 s into its file',
            trimmed['ok'] and wait(lambda: project(r)['tracks'][0]['clips'][0]['content']['sourceInUs'] == 700_000, 5), trimmed)
    before = project(r)
    length = r.track()[-1]['startUs'] + r.track()[-1]['durationUs']
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((b) => b.textContent === 'Plugins').click()")
    r.check('the Plugins tab lists Remove pauses', wait(lambda: r.s.run(f'return !!{PAUSES_ROW}'), 10))
    r.s.run(f'{PAUSES_ROW}.click()')
    # The synthetic speech pauses for about 0.4 s between sentences.
    for label, value in (('Longer than', 0.3), ('Keep on each side', 0.05)):
        r.check(f'{label} is set to {value} s', wait(lambda: r.s.run(SLIDE, label, value), 5))
    shown = wait(lambda: r.s.run("return document.body.textContent.includes('Video length')"), 30)
    r.check('the inspector says how much shorter the video gets', shown, r.s.run('return document.body.innerText.slice(-600)'))
    r.check('the preview leaves the project unchanged', project(r) == before)
    r.shot('pauses-preview')
    preview = r.s.call("window.__nuzky.api.previewPlugin('remove-pauses', arguments[0], [], 0).then(JSON.stringify)",
                       {'minSeconds': 0.3, 'keepSeconds': 0.05})
    expected = cuts(json.loads(preview['value'])['tracks']) if preview['ok'] else None
    r.check('the preview of these settings cuts pauses out of the main track',
            expected and len(expected[0]) > 1 and expected[0][-1][0] + expected[0][-1][1] < length, preview)

    r.s.run("[...document.querySelectorAll('button')].find((b) => b.textContent === 'Remove pauses').click()")
    cut = wait(lambda: cuts(project(r)['tracks']) == expected, 20)
    r.check('running cuts exactly what the preview showed', cut, cuts(project(r)['tracks']))
    end = r.track()[-1]['startUs'] + r.track()[-1]['durationUs']
    toast = next((t['text'] for t in r.state()['toasts'] if t['kind'] == 'success'), '')
    r.check('the toast says how many pauses went and how long they were',
            toast.startswith('Removed ') and f'{(length - end) / 1e6:.1f} s in all.' in toast, (toast, length - end))
    r.check('the saved project has the cut', cut and wait(lambda: cuts(saved(r)['tracks']) == expected, 5))
    r.shot('pauses-done')
    r.s.call('window.__nuzky.store.getState().undo()')
    r.check('one undo brings the pauses back', wait(lambda: project(r) == before, 5), project(r))
    r.check('no error toast', not r.errors(), r.errors())


LOGO_SCRIPT = '''function run(ctx) {
  const logo = ctx.files["logo.png"];
  const end = ctx.project.duration_us;
  if (end === 0) return {edits: [], message: "Add a video first."};
  return {edits: [{type: "addSticker", assetId: logo.assetId, startUs: 0, durationUs: end,
    transform: {x: 0.35, y: -0.35, scale: 0.2, rotation: 0, opacity: 1}}], message: "Added the logo."};
}'''
LOGO_TRANSFORM = {'x': 0.35, 'y': -0.35, 'scale': 0.2}


def white(image, box):
    """Share of the box in the logo's white; the test pattern's bars are all saturated colours."""
    import numpy as np
    a = np.asarray(image.convert('RGB').crop(tuple(round(v) for v in box)), dtype=int)
    return float(((a > 215).all(axis=-1)).mean())


def logo_box(image, canvas):
    """The middle half of where the logo lies in a frame of `canvas`'s shape: a square logo at scale 0.2 is a fifth of
    the canvas's shorter side."""
    k = image.width / canvas['width']
    side = LOGO_TRANSFORM['scale'] * min(canvas['width'], canvas['height']) * k
    cx = (0.5 + LOGO_TRANSFORM['x']) * image.width
    cy = (0.5 + LOGO_TRANSFORM['y']) * image.height
    return (cx - side / 4, cy - side / 4, cx + side / 4, cy + side / 4)


def exported_frame(r, video, name):
    from PIL import Image
    path = r.work / f'{name}.png'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-ss', '1', '-i', str(video), '-frames:v', '1', '-update', '1', str(path)],
                   check=True)
    r.shots.append(path.name)
    return Image.open(path)


@flow('plugin_files', 'A plugin an agent saved with its own logo previews "Adds logo.png" without changing anything, '
      'puts the logo in the top right corner as one undo step and the export shows it there; after the plugin is '
      'deleted the project opens again and still exports the logo')
def plugin_files(r):
    from PIL import Image
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    canvas = project(r)['canvas']
    source = r.work / 'brand-logo.png'
    Image.new('RGB', (256, 256), (255, 255, 255)).save(source)
    time.sleep(1.5)  # `nuzky frame` reads the saved file, written about a second after an edit.
    plain = r.work / 'plain.png'
    subprocess.run([str(CLI), 'frame', str(r.saved_project()), '1', str(plain), '720'], env=r.launch_env(), check=True,
                   capture_output=True)
    r.check('the video alone has no white where the logo will go',
            white(Image.open(plain), logo_box(Image.open(plain), canvas)) < 0.01)

    manifest = {'nuzkyPlugin': 2, 'id': 'corner-logo', 'name': 'Corner logo', 'version': '1.0.0', 'action': 'Add logo',
                'description': 'Puts your logo in the top right corner for the whole video.', 'files': ['logo.png']}
    bridge = Bridge(r, r.saved_project())
    try:
        saved = bridge.call('save_plugin', {'manifest': manifest, 'script': LOGO_SCRIPT, 'files': {'logo.png': str(source)}})
    finally:
        bridge.close()
    folder = r.work / 'data/nuzky/plugins/corner-logo'
    r.check('save_plugin copies the agent\'s logo into the plugin\'s folder',
            saved['plugin']['status'] == 'ok' and (folder / 'logo.png').read_bytes() == source.read_bytes(), saved)

    before = project(r)
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((b) => b.textContent === 'Plugins').click()")
    row = "document.querySelector('[data-plugin=\"corner-logo\"]')"
    r.check('the Plugins tab lists the plugin', wait(lambda: r.s.run(f'return !!{row}'), 10))
    r.s.run(f'{row}.click()')
    said = wait(lambda: (t := r.s.run("return document.body.textContent")) and 'Puts logo.png in Media' in t and t, 10)
    r.check('the preview says it adds the logo to the video and to Media', said and 'Adds logo.png' in said and 'Marked on the timeline' not in said)
    r.check('the preview changes nothing in the project', project(r) == before)
    r.shot('preview')

    r.s.run("[...document.querySelectorAll('button')].find((b) => b.textContent === 'Add logo').click()")
    logo = wait(lambda: next((a for a in project(r)['assets'] if a['name'] == 'logo.png'), None), 10)
    if not r.check('running adds logo.png to the project', logo, project(r)['assets']):
        return
    kept = Path(logo['path'])
    r.check('the project keeps its own copy of the logo, outside the plugin\'s folder',
            kept.is_file() and kept.parent.name == 'plugin-files' and 'plugins' not in kept.parts, logo['path'])
    placed = [c for t in project(r)['tracks'][1:] for c in t['clips'] if c['content'].get('assetId') == logo['id']]
    r.check('the logo lies over the whole video in the top right corner',
            len(placed) == 1 and placed[0]['startUs'] == 0 and placed[0]['durationUs'] == r.track()[0]['durationUs']
            and all(abs(placed[0]['content']['transform'][k] - v) < 1e-6 for k, v in LOGO_TRANSFORM.items()), placed)
    r.check('the plugin message reaches the success toast', wait(lambda: any(
        t['kind'] == 'success' and t['text'] == 'Added the logo.' for t in r.state()['toasts']), 5), r.state()['toasts'])
    r.s.run('window.__nuzky.store.getState().select([])')
    r.shot('done')
    r.s.call('window.__nuzky.store.getState().undo()')
    r.check('one undo takes away the logo and its media', wait(lambda: project(r) == before, 5))
    r.s.call('window.__nuzky.store.getState().redo()')
    r.check('redo brings both back', wait(lambda: any(a['id'] == logo['id'] for a in project(r)['assets']), 5))

    frame = exported_frame(r, export(r, 'with-logo.mp4'), 'export-with-logo')
    share = white(frame, logo_box(frame, canvas))
    r.check('the export shows the logo in the top right corner', share > 0.9, f'{share:.0%} of the box is the logo')

    r.s.run("document.querySelector('[aria-label=\"Close plugin\"]').click()")
    shutil.rmtree(folder)
    r.check('the deleted plugin leaves the list', wait(lambda: r.s.run(f'return !{row}'), 5))
    leave_editor(r)
    path = str(r.saved_project())
    r.s.run('window.__nuzky.openProject({path: arguments[0], name: "reopened"})', path)
    reopened = wait(lambda: r.s.run("const s = window.__nuzky.store.getState(); return s.view === 'editor' && s.snap?.project",
                                    retries=3), 20)
    r.check('the project opens again without the plugin, with its logo', reopened and any(
        a['id'] == logo['id'] and a['path'] == logo['path'] for a in reopened['assets']), reopened and reopened['assets'])
    frame = exported_frame(r, export(r, 'after-delete.mp4'), 'export-after-delete')
    share = white(frame, logo_box(frame, canvas))
    r.check('it still exports the logo in the corner', share > 0.9, f'{share:.0%} of the box is the logo')
    r.check('no error toast', not r.errors(), r.errors())

import json
import shutil
import subprocess
import time
from pathlib import Path

from e2e.harness import CLI, FIXTURES, ROOT, Bridge, export, ffprobe, flow, leave_editor, wait, webdriver

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
            toast.startswith('Removed ') and f'{(length - end) / 1e6:.1f}\u00a0s in all.' in toast, (toast, length - end))
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


# The inspector's line saying what the open plugin would do, and whether it is dimmed while a newer check runs.
OUTCOME = """const line = document.querySelector('.lucide-eye')?.parentElement;
    return line ? {text: line.innerText, stale: !!line.closest('.opacity-50')} : null;"""


def outcome(r, text, timeout=10):
    """Waits for the inspector to say `text`, from a check of the current project and settings."""
    return wait(lambda: (o := r.s.run(OUTCOME)) and not o['stale'] and o['text'] == text, timeout) or r.s.run(OUTCOME)


def resize(r, width, height):
    webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
    return wait(lambda: r.s.run('return [window.innerWidth, window.innerHeight]') == [width, height], 5)


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


@flow('plugin_files', 'A plugin an agent saved with its own logo previews in one line that it adds the logo to the '
      'video and Media without changing anything, '
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
    said = outcome(r, "Adds the plugin's logo.png to the video and Media.")
    r.check('the preview says in one line that the plugin\'s logo goes into the video and Media', said is True, said)
    r.check('the preview changes nothing in the project', project(r) == before)
    r.shot('preview')
    r.check('the window goes to its smallest size', resize(r, 1024, 640))
    time.sleep(0.5)
    r.shot('preview-1024')
    resize(r, 1440, 900)
    # The inspector shows the first three lines, so the plugin's file leads even when its clips are not all it changes.
    busy = r.s.call("""window.__nuzky.api.previewPlugin('corner-logo', {}, [], 0).then(async (preview) => {
        const {describePeek} = await import('/src/lib/peek.ts');
        const now = window.__nuzky.store.getState().snap.project;
        const then = structuredClone(preview);
        then.tracks[0].clips[0].content.volume = 0.5;
        then.name = 'Branded';
        then.canvas = {...then.canvas, width: then.canvas.height, height: then.canvas.width};
        return JSON.stringify(describePeek(now, then, true).lines);
    })""")
    r.check('with other changes too, the line about the plugin\'s logo comes first',
            busy['ok'] and json.loads(busy['value'])[0].startswith("Adds the plugin's logo.png to "), busy)

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
    again = outcome(r, 'Adds logo.png.')
    r.check('run again, the preview adds the logo the project already has, not a second copy in Media', again is True, again)
    r.shot('done')
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((b) => b.textContent === 'Media').click()")
    tile = """const add = document.querySelector('[aria-label="Add logo.png at playhead"]');
        return add?.closest('.group')?.querySelector('img')?.naturalWidth > 0;"""
    r.check('Media shows the copied logo with its thumbnail', wait(lambda: r.s.run(tile), 10))
    r.shot('media')
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((b) => b.textContent === 'Plugins').click()")
    r.s.call('window.__nuzky.store.getState().undo()')
    r.check('one undo takes away the logo and its media', wait(lambda: project(r) == before, 5))
    r.s.call('window.__nuzky.store.getState().redo()')
    r.check('redo brings both back', wait(lambda: any(a['id'] == logo['id'] for a in project(r)['assets']), 5))

    frame = exported_frame(r, export(r, 'with-logo.mp4'), 'export-with-logo')
    share = white(frame, logo_box(frame, canvas))
    r.check('the export shows the logo in the top right corner', share > 0.9, f'{share:.0%} of the box is the logo')

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


TITLE_JS = """function run(ctx) {
  const has = ctx.project.tracks.some((t) => t.clips.some((c) => c.content.type === "text" && c.content.text === "Title card"));
  if (has) return { edits: [], message: "The title is already there." };
  return { edits: [{ type: "addText", startUs: 0, text: "Title card", style: { fontSize: 120, color: "#FFFFFF", bold: true } }],
           message: "Added a title." };
}"""
# Neutral descriptions, so the Plugins list says when they run only where the switches say so.
AUTO_PLUGINS = {
    'add-logo': ('Add logo', 'Puts your logo in the corner.', 'function run(ctx) { throw new Error("the logo file is missing"); }'),
    'slow-check': ('Slow check', 'Checks every frame.', 'function run(ctx) { while (true) {} }'),
    'title-card': ('Title card', 'Puts a title at the start.', TITLE_JS),
}
UNDO_IN_TOAST = """const toast = [...document.querySelectorAll('[data-toast]')].find((t) => t.textContent.includes(arguments[0]));
const undo = toast && [...toast.querySelectorAll('button')].find((b) => b.textContent === 'Undo');
if (undo) undo.click();
return !!undo;"""
DIALOG_TEXT = "return document.querySelector('[role=dialog]')?.textContent ?? ''"
DIALOG_BUTTONS = "return [...document.querySelectorAll('[role=dialog] button')].map((b) => b.getAttribute('aria-label') || b.textContent)"
CLICK_IN_DIALOG = """const b = [...document.querySelectorAll('[role=dialog] button')].find((b) => b.textContent === arguments[0]);
if (b) b.click();
return !!b;"""
SWITCH = "document.querySelector(`[role=switch][aria-label=\"${arguments[0]}\"]`)"
ROW_TEXT = "return document.querySelector(`[data-plugin=\"${arguments[0]}\"]`)?.innerText ?? ''"
INSPECTOR_BUTTON = "[...document.querySelectorAll('[aria-label=Inspector] button')].find((b) => b.textContent === arguments[0])"
# Whether the element is wholly on screen in the inspector: neither scrolled away, cut off nor covered. Hit testing
# follows rounded corners, so it probes the middle of the top and bottom edges.
SHOWN = """const e = %s;
if (!e) return false;
const box = e.getBoundingClientRect();
const pane = document.querySelector('[aria-label=Inspector]').getBoundingClientRect();
const hits = (x, y) => { const hit = document.elementFromPoint(x, y); return !!hit && (hit === e || e.contains(hit)); };
return box.height > 0 && box.top >= pane.top && box.bottom <= pane.bottom && box.bottom <= window.innerHeight
    && hits(box.left + box.width / 2, box.top + 1) && hits(box.left + box.width / 2, box.bottom - 1);"""


def prepare_autorun(r):
    plugins = r.work / 'data/nuzky/plugins'
    shutil.copytree(PAUSES, plugins / 'remove-pauses')
    for id, (name, description, script) in AUTO_PLUGINS.items():
        (plugins / id).mkdir()
        (plugins / id / 'plugin.json').write_text(json.dumps({'nuzkyPlugin': 2, 'id': id, 'name': name, 'version': '1.0.0',
                                                              'description': description}))
        (plugins / id / 'plugin.js').write_text(script)


def open_plugin(r, id):
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((b) => b.textContent === 'Plugins').click()")
    row = f"document.querySelector('[data-plugin=\"{id}\"]')"
    if not wait(lambda: r.s.run(f'return !!{row}'), 10):
        raise RuntimeError(f'the Plugins tab does not list {id}')
    r.s.run(f'{row}.click()')
    if not wait(lambda: r.s.run(f'return !!{SWITCH} && document.querySelector("[aria-label=Inspector]").textContent.includes(arguments[1])',
                                'Run after import', AUTO_PLUGINS.get(id, ('Remove pauses',))[0]), 5):
        raise RuntimeError(f'{id} did not open in the inspector')


def switched_on(r, label):
    return r.s.run(f"return {SWITCH}.getAttribute('aria-checked')", label) == 'true'


def turn_on(r, id, label):
    open_plugin(r, id)
    r.s.run(f'{SWITCH}.click()', label)
    r.check(f'{id}: {label} turns on', wait(lambda: switched_on(r, label), 5))


def new_project(r, video):
    old = r.state()['epoch']
    r.s.call('window.__nuzky.newProjectFromMedia(arguments[0])', [str(video)])
    if not wait(lambda: (s := r.state())['epoch'] != old and s['tracks'][0]['clips'], 30):
        raise RuntimeError('the new project did not open with the video on the timeline')


def auto_toasts(r):
    """The toasts of automatic runs, and the one a run of the AI gets, which a plugin run must not get as well."""
    return [t for t in r.state()['toasts'] if ' after import' in t['text'] or ' before export' in t['text']
            or t['text'].startswith('AI edit done')]


def main_cuts(p):
    return cuts([t for t in p['tracks'] if t['id'] == 'main'])


def titles(p):
    return [c['content']['text'] for t in p['tracks'] for c in t['clips'] if c['content']['type'] == 'text']


def sizes(r, name):
    """A screenshot at the smallest window, 1024 x 640, then at 1440 x 900, where the flow goes on."""
    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        resize(r, width, height)
        time.sleep(1)
        r.shot(f'{name}{suffix}')


def saved_open(r):
    return json.loads(Path(r.s.run('return window.__nuzky.store.getState().snap.path')).read_text())


def start_export(r, target):
    """Started as Export… does after the save panel, which cannot be driven; resolves to whether the export started."""
    r.s.run("window.autorunExport = window.__nuzky.exportVideo(arguments[0], arguments[1], arguments[2], true, false)"
            ".then(String, String)", str(target), {'resolution': 720, 'fps': 30, 'quality': 'small'}, r.state()['epoch'])


def open_dialog(r):
    r.key('e', ctrlKey=True)
    shown = wait(lambda: r.s.run(DIALOG_TEXT), 5)
    time.sleep(0.5)  # the dialog fades in over 200 ms
    return shown


def dismiss_toasts(r):
    r.s.run('const s = window.__nuzky.store.getState(); s.toasts.forEach((t) => s.dismissToast(t.id))')


def finished_export(r):
    started = wait(lambda: r.s.run('return window.__nuzky.store.getState().exportJobId'), 15)
    return started and wait(lambda: (j := r.s.run('return window.__nuzky.store.getState().jobs[arguments[0]] ?? null', started))
                            and j['status'] != 'running' and j, 180)


@flow('plugin_autorun', 'Plugins the user turned on run by themselves after a new project is made of a video, with the '
      'settings last set; one toast says what the pass did with one Undo for all of it, and a failure its own; the '
      'Plugins list marks them; before an export the dialog says what each did, Cancel export writes no file, Skip '
      'lets the export go on, the switches stay usable while a plugin runs, and while the AI edits one toast says '
      'they did not run', before=prepare_autorun)
def plugin_autorun(r):
    video = FIXTURES / 'talk.mp4'
    open_plugin(r, 'remove-pauses')
    r.check('both switches start off', not switched_on(r, 'Run after import') and not switched_on(r, 'Run before export'))
    for label, value in (('Longer than', 0.3), ('Keep on each side', 0.05)):
        r.check(f'{label} is set to {value} s', wait(lambda: r.s.run(SLIDE, label, value), 5))

    new_project(r, video)
    time.sleep(3)
    plain = project(r)
    r.check('with every plugin off, the new project keeps the whole video as one clip',
            len(plain['tracks'][0]['clips']) == 1 and not auto_toasts(r), (cuts(plain['tracks']), auto_toasts(r)))

    def preview(values):
        answer = r.s.call("window.__nuzky.api.previewPlugin('remove-pauses', arguments[0], [], 0).then(JSON.stringify)", values)
        return answer['ok'] and main_cuts(json.loads(answer['value']))
    expected = wait(lambda: preview({'minSeconds': 0.3, 'keepSeconds': 0.05}), 30)
    r.check('the settings set cut pauses out of this video', expected and len(expected[0]) > 1, expected)
    defaults = preview({'minSeconds': 0.6, 'keepSeconds': 0.1})
    r.check('the default settings would cut it differently, so the run shows which it used',
            defaults and defaults != expected, defaults)

    open_plugin(r, 'remove-pauses')
    shown = r.s.run("return document.querySelector('input[type=range][aria-label=\"Longer than\"]').value")
    r.check('the settings stay as last set in the next project', shown == '0.3', shown)
    r.check('no line about automatic runs while both switches are off',
            'Runs on the whole project' not in r.s.run("return document.querySelector('[aria-label=Inspector]').textContent"))
    r.s.run(f'{SWITCH}.click()', 'Run after import')
    r.check('Run after import turns on', wait(lambda: switched_on(r, 'Run after import'), 5))
    r.check('Run before export stays off', not switched_on(r, 'Run before export'))
    r.check('a line says it runs on the whole project with these settings',
            'Runs on the whole project with these settings.' in r.s.run("return document.querySelector('[aria-label=Inspector]').textContent"))
    r.check('the switches follow the settings',
            r.s.run(f"return {SWITCH}.getBoundingClientRect().top > document.querySelector('input[type=range][aria-label=\"Keep on each side\"]').getBoundingClientRect().bottom",
                    'Run after import'))
    description = r.s.run("""const d = [...document.querySelectorAll('[aria-label=Inspector] div')].find((e) => e.textContent.startsWith('Cuts out the quiet') && !e.children.length);
        const style = d && getComputedStyle(d);
        return d && {lines: Math.round(d.getBoundingClientRect().height / (parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.25)), title: d.title};""")
    for width, height, suffix in ((1024, 640, '-1024'), (1440, 900, '')):
        resize(r, width, height)
        r.check(f'at {width} x {height} Remove pauses shows whole without scrolling, under two sliders and a long description',
                wait(lambda: r.s.run(SHOWN % INSPECTOR_BUTTON, 'Remove pauses'), 5))
        time.sleep(1)
        r.shot(f'autorun-settings{suffix}')
    r.check('the description takes at most two lines and keeps all of it in its tooltip',
            description and description['lines'] <= 2 and description['title'].startswith('Cuts out the quiet')
            and description['title'].endswith('on each side.'), description)
    resize(r, 1024, 640)
    r.s.run(f'{SWITCH}.scrollIntoView({{block: "nearest"}})', 'Run before export')
    r.check('at 1024 x 640 the switches scroll into view above the button, which stays',
            wait(lambda: r.s.run(SHOWN % SWITCH, 'Run after import') and r.s.run(SHOWN % SWITCH, 'Run before export')
                 and r.s.run(SHOWN % INSPECTOR_BUTTON, 'Remove pauses'), 5))
    time.sleep(1)
    r.shot('autorun-switch-1024')
    resize(r, 1440, 900)
    turn_on(r, 'title-card', 'Run after import')
    turn_on(r, 'add-logo', 'Run after import')
    rows = {id: r.s.run(ROW_TEXT, id) for id in ('remove-pauses', 'title-card', 'slow-check')}
    r.check('the Plugins list marks the plugins that run after import, and only them',
            'After import · Cuts out the quiet' in rows['remove-pauses'] and 'After import · Puts a title' in rows['title-card']
            and 'After import' not in rows['slow-check'] and 'Before export' not in rows['remove-pauses'], rows)

    new_project(r, video)
    ran = wait(lambda: main_cuts(project(r)) == expected and titles(project(r)) == ['Title card'], 30)
    r.check('after the import the plugins cut the pauses exactly as the preview with the settings set does, and added the title',
            ran, (main_cuts(project(r)), titles(project(r))))
    passed = wait(lambda: next((t['text'] for t in auto_toasts(r) if t['kind'] == 'success'), None), 5)
    r.check('one toast says both plugins ran after import and what each did',
            passed and passed.startswith('Plugins ran after import. Remove pauses: removed ') and passed.endswith(' s in all. Title card: added a title.'),
            auto_toasts(r))
    failed = [t['text'] for t in auto_toasts(r) if t['kind'] == 'warning']
    r.check('the plugin that threw says it failed after import and why, without the place in its script',
            failed == ['Add logo failed after import: the logo file is missing. Nothing changed.'], auto_toasts(r))
    r.check('the pass gets those two toasts only', len(auto_toasts(r)) == 2, auto_toasts(r))
    r.check('the saved project has the cut and the title', ran and wait(lambda: main_cuts(saved_open(r)) == expected and titles(saved_open(r)) == ['Title card'], 5))
    r.shot('autorun-import')
    r.check('the toast offers one Undo', r.s.run(UNDO_IN_TOAST, 'Plugins ran after import'))
    r.check('Undo takes back the whole pass: the whole video as one clip again, without the title',
            wait(lambda: cuts(project(r)['tracks']) == cuts(plain['tracks']), 5), cuts(project(r)['tracks']))
    time.sleep(1)
    r.shot('autorun-undo')

    for id in AUTO_PLUGINS:
        turn_on(r, id, 'Run before export')
    rows = {id: r.s.run(ROW_TEXT, id) for id in AUTO_PLUGINS}
    r.check('the Plugins list says when each runs',
            'After import · Before export · Puts a title' in rows['title-card'] and 'Before export · Checks every frame' in rows['slow-check'], rows)
    r.shot('autorun-list')
    dismiss_toasts(r)
    before = project(r)
    length = max(c['startUs'] + c['durationUs'] for c in before['tracks'][0]['clips'])

    # Cancel export: the plugin stops, the rest are skipped, and no file is written.
    r.check('Ctrl+E opens the export dialog', open_dialog(r))
    cancelled = r.work / 'cancelled.mp4'
    start_export(r, cancelled)
    waiting = wait(lambda: 'Waiting for Slow check…' in r.s.run(DIALOG_TEXT), 10)
    text = r.s.run(DIALOG_TEXT)
    r.check('the dialog says which plugin the export waits for, which of the three it is, and that the export starts after it',
            waiting and '2 of 3' in text and "The export starts when it's done." in text, text)
    r.check('the dialog says why Add logo failed, with Copy error, and no toast hides behind it',
            'Add logo failed: the logo file is missing. Nothing changed.' in text and 'Copy error' in r.s.run(DIALOG_BUTTONS)
            and not auto_toasts(r), (text, auto_toasts(r)))
    buttons = r.s.run(DIALOG_BUTTONS)
    r.check('the footer offers Cancel export, Skip Slow check and Hide, and × hides',
            {'Cancel export', 'Skip Slow check', 'Hide', 'Hide (export keeps running)'} <= set(buttons), buttons)
    r.check('the export has not started meanwhile', not r.s.run('return window.__nuzky.store.getState().exportJobId'))
    sizes(r, 'autorun-export-waits')
    r.check('Cancel export is there to click', r.s.run(CLICK_IN_DIALOG, 'Cancel export'))
    r.check('Cancel export resolves without starting the export', wait(lambda: r.s.call('window.autorunExport') == {'ok': True, 'value': 'false'}, 10),
            r.s.call('window.autorunExport'))
    time.sleep(2)
    text = r.s.run(DIALOG_TEXT)
    r.check('Cancel export writes no file and starts no export job',
            not cancelled.exists() and not r.s.run('return window.__nuzky.store.getState().exportJobId')
            and not any(j['kind'] == 'export' for j in r.s.run('return Object.values(window.__nuzky.store.getState().jobs)')),
            (cancelled.exists(), r.s.run('return window.__nuzky.store.getState().exportJobId')))
    r.check('the dialog says the export was cancelled and the plugin stopped',
            'Export cancelled. No file was written.' in text and 'Slow check was stopped. Nothing changed.' in text, text)
    r.check('the plugin after it was skipped: the project is as it was', project(r) == before, titles(project(r)))
    r.shot('autorun-export-cancelled')

    # Skip: the export goes on without the stuck plugin; Hide leaves the switches usable while it runs.
    target = r.work / 'after-plugins.mp4'
    start_export(r, target)
    r.check('Export again runs the plugins again', wait(lambda: 'Waiting for Slow check…' in r.s.run(DIALOG_TEXT), 10), r.s.run(DIALOG_TEXT))
    r.check('Hide is there to click', r.s.run(CLICK_IN_DIALOG, 'Hide'))
    r.check('Hide closes the dialog while the plugin keeps running',
            wait(lambda: not r.s.run(DIALOG_TEXT), 5) and r.s.run('return window.__nuzky.plugins.getState().pass') == 'export')
    open_plugin(r, 'slow-check')
    usable = r.s.run(f"const s = {SWITCH}; return s.getAttribute('aria-disabled') !== 'true' && !s.title.includes('AI is editing')", 'Run after import')
    r.s.run(f'{SWITCH}.click()', 'Run after import')
    turned = wait(lambda: switched_on(r, 'Run after import'), 5)
    r.s.run(f'{SWITCH}.click()', 'Run after import')
    r.check('while the plugin runs, its switches still turn on and off',
            usable and turned and wait(lambda: not switched_on(r, 'Run after import'), 5) and switched_on(r, 'Run before export'))
    r.check('while it runs, its button shows whole and says so', r.s.run(SHOWN % INSPECTOR_BUTTON, 'Running…'))
    r.shot('autorun-running')
    r.check('the export dialog opens again on the plugin it waits for',
            open_dialog(r) and 'Waiting for Slow check…' in r.s.run(DIALOG_TEXT), r.s.run(DIALOG_TEXT))
    r.check('Skip Slow check is there to click', r.s.run(CLICK_IN_DIALOG, 'Skip Slow check'))
    job = finished_export(r)
    r.check('the export finishes after the skipped plugin and the one after it', job and job['status'] == 'done', job)
    r.check('exportVideo resolves that the export started', r.s.call('window.autorunExport') == {'ok': True, 'value': 'true'})
    text = r.s.run(DIALOG_TEXT)
    r.check('the dialog lists all three: the failure, the skipped plugin and the one that ran',
            all(line in text for line in ('Add logo failed: the logo file is missing. Nothing changed.',
                                          'Slow check was stopped. Nothing changed.', 'Title card ran: added a title.')), text)
    r.check('no toast while the dialog is open', not auto_toasts(r), auto_toasts(r))
    after = project(r)
    r.check('only Title card changed the project', main_cuts(after) == main_cuts(before) and titles(after) == ['Title card'], titles(after))
    duration = ffprobe(target)[0] if target.exists() else 0
    r.check('the exported file has the length of the project', abs(duration - length / 1e6) < 0.1, (duration, length / 1e6))
    r.shot('autorun-export-done')
    r.check('Done is there to click', r.s.run(CLICK_IN_DIALOG, 'Done'))
    toasts = wait(lambda: len(t := auto_toasts(r)) == 2 and t, 5) or auto_toasts(r)
    texts = {t['kind']: t['text'] for t in toasts}
    r.check('once the dialog closes, one toast says what the pass did and one why Add logo failed, without "the export goes on"',
            texts.get('success') == 'Plugins ran before export. Slow check: stopped, nothing changed. Title card: added a title.'
            and texts.get('warning') == 'Add logo failed before export: the logo file is missing. Nothing changed.', toasts)
    r.shot('autorun-export-toasts')
    r.check('its Undo takes the title back', r.s.run(UNDO_IN_TOAST, 'Plugins ran before export')
            and wait(lambda: project(r)['tracks'] == before['tracks'], 5), titles(project(r)))

    # While the AI edits, the export goes on without the plugins and says so once.
    dismiss_toasts(r)
    bridge = Bridge(r, Path(r.s.run('return window.__nuzky.store.getState().snap.path')))
    try:
        run = bridge.call('begin_run', {'label': 'Tighten the intro'})['run_id']
        r.check('the AI run holds the lock', wait(lambda: r.s.run('return window.__nuzky.store.getState().aiRun') == 'Tighten the intro', 10),
                r.s.run('return window.__nuzky.store.getState().aiRun'))
        r.check('the export dialog opens', open_dialog(r))
        start_export(r, r.work / 'while-ai-edits.mp4')
        job = finished_export(r)
        r.check('the export finishes without the plugins', job and job['status'] == 'done', job)
        text = r.s.run(DIALOG_TEXT)
        r.check('the dialog says once that the plugins did not run', text.count("didn't run") == 1
                and "Plugins didn't run: the AI is editing." in text, text)
        r.check('Done is there to click', r.s.run(CLICK_IN_DIALOG, 'Done'))
        skipped = wait(lambda: [t['text'] for t in auto_toasts(r) if "didn't run" in t['text']], 5)
        r.check('one toast for the pass says the plugins did not run before export',
                skipped == ["Plugins didn't run before export: the AI is editing."], auto_toasts(r))
        r.check('the project is as it was', project(r)['tracks'] == before['tracks'])
        r.shot('autorun-skipped')
        bridge.call('end_run', {'run_id': run, 'action': 'keep'})
    finally:
        bridge.close()

    # An export waits for the plugins still running after an import, and Cancel export stays cancelled when they end.
    dismiss_toasts(r)
    for id in AUTO_PLUGINS:
        open_plugin(r, id)
        if switched_on(r, 'Run before export'):
            r.s.run(f'{SWITCH}.click()', 'Run before export')
    turn_on(r, 'slow-check', 'Run after import')
    r.s.run('window.__nuzky.importPaths(arguments[0])', [str(video)])
    r.check('the plugins after the import reach Slow check',
            wait(lambda: r.s.run("const p = window.__nuzky.plugins.getState(); return p.pass === 'import' && p.running === 'slow-check'"), 30))
    r.check('the export dialog opens', open_dialog(r))
    waited = r.work / 'waited.mp4'
    start_export(r, waited)
    r.check('the export waits for the plugins after the import and has not started',
            wait(lambda: 'Waiting for plugins…' in r.s.run(DIALOG_TEXT), 10) and not r.s.run('return window.__nuzky.store.getState().exportJobId'),
            r.s.run(DIALOG_TEXT))
    r.check('Cancel export is there to click', r.s.run(CLICK_IN_DIALOG, 'Cancel export'))
    r.check('Slow check is still running after Cancel export, as it runs for the import',
            r.s.run('return window.__nuzky.plugins.getState().running') == 'slow-check')
    r.s.call('window.__nuzky.api.stopRun(window.__nuzky.store.getState().snap.sessionEpoch, false)')
    r.check('once Slow check is stopped the pass after the import ends',
            wait(lambda: r.s.run('return window.__nuzky.plugins.getState().pass') is None, 30))
    time.sleep(2)
    r.check('the cancelled export never starts when that pass ends',
            r.s.call('window.autorunExport') == {'ok': True, 'value': 'false'} and not waited.exists()
            and not r.s.run('return window.__nuzky.store.getState().exportJobId'), r.s.call('window.autorunExport'))
    r.check('no error toast', not r.errors(), r.errors())

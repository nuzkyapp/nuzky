import json
import shutil

from e2e.harness import FIXTURES, ROOT, Bridge, flow, wait

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

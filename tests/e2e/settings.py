"""Settings: one window for Nuzky's own settings, opened from the gear, Ctrl+, and the home screen, on the pane shown
last. Search finds a setting or a key and lights it up; Storage clears what Nuzky makes again and keeps what the open
project uses, so an export goes on; Learn from my edits, Revoke on a plugin with full access and the plugins'
automatic runs change what the backend does; Privacy leads to the setting it names."""
import json, shutil, time

from e2e.harness import FIXTURES, ROOT, export, ffprobe, flow, press, wait
from e2e.home import TYPE, resize

EXPORT_XML = ROOT / 'skills/nuzky-plugin/examples/export-xml'
OPEN = "return !!document.querySelector('[role=dialog][data-settings]')"
TITLE = "return document.querySelector('[data-settings] #settings-title')?.textContent ?? null"
SEARCH = '[data-settings] input[type=search]'
RESULTS = "return [...document.querySelectorAll('[data-settings] [data-results] button')].map((b) => b.innerText)"
LIT = "return !!document.querySelector(arguments[0])?.classList.contains('changed')"
TAB = "document.querySelector(`#settings-tab-${arguments[0]}`).click()"
BUTTON = """const scope = document.querySelector(arguments[1]);
return scope && [...scope.querySelectorAll('button')].find((b) => b.textContent.trim() === arguments[0] && b.getAttribute('aria-disabled') !== 'true') || null;"""
CLICK = BUTTON.replace('return scope &&', 'const b = scope &&') + '\nif (!b) return false; b.click(); return true;'
USAGE = "window.__nuzky.api.storageUsage().then((u) => JSON.stringify(Object.fromEntries(u.groups.map((g) => [g.id, g]))))"
ROW = "return document.querySelector(`[data-storage=${arguments[0]}]`)?.innerText ?? null"
TEXT = "return document.querySelector(arguments[0])?.innerText ?? ''"
FITS = """const b = document.querySelector('[data-settings]').getBoundingClientRect();
return {fits: b.top >= 0 && b.left >= 0 && b.bottom <= innerHeight && b.right <= innerWidth, width: b.width, height: b.height};"""
PLUGIN = "return window.__nuzky.plugins.getState().list?.find((p) => p.id === 'export-xml') ?? null"
LEARN = "return document.querySelector('[data-setting=learn] [role=switch]')?.getAttribute('aria-checked') ?? null"
IDLE = "return Object.values(window.__nuzky.store.getState().jobs).every((j) => j.status !== 'running')"


def prepare(r):
    shutil.copytree(EXPORT_XML, r.work / 'data/nuzky/plugins/export-xml')


def usage(r):
    return json.loads(r.s.call(USAGE)['value'])


@flow('settings', "Settings: the gear, Ctrl+, and the home screen open it on the pane shown last; search finds settings and keys; "
      "Storage clears caches without breaking the open project; learning, plugin access and automatic runs reach the backend",
      before=prepare)
def settings(r):
    r.import_media(FIXTURES / 'talk.mp4', FIXTURES / 'wide.mp4')
    r.add_clip('talk.mp4')
    other = next(a for a in r.state()['assets'] if a['name'] == 'wide.mp4')
    r.check("both videos' sound is prepared",
            wait(lambda: (s := usage(r)['sound'])['bytes'] > 0 and s['bytes'] == s['keptBytes'] and r.s.run(IDLE), 60), usage(r)['sound'])
    both = usage(r)['sound']['bytes']
    # wide.mp4 leaves the project, so its prepared sound is no longer the open project's.
    r.s.call("window.__nuzky.store.getState().edit({type: 'removeAsset', assetId: arguments[0]})", other['id'])
    r.check('without wide.mp4 the open project keeps less than all of it', wait(lambda: 0 < usage(r)['sound']['keptBytes'] < both, 5), usage(r)['sound'])

    r.s.run("document.querySelector('header button[aria-label^=\"Settings\"]').click()")
    r.check("the editor's gear opens Settings on General", wait(lambda: r.s.run(TITLE) == 'General', 5), r.s.run(TITLE))
    r.check('the keyboard starts in its search field', r.s.run("return document.activeElement?.matches(arguments[0])", SEARCH))
    time.sleep(0.5)
    r.shot('general')

    # Search finds an action by name with its key, and Enter shows it lit up in Keyboard.
    r.s.run(TYPE, SEARCH, 'split')
    results = wait(lambda: r.s.run(RESULTS), 3) or []
    r.check('typing "split" lists the Split action with its key and its pane',
            any(x.split('\n') == ['Split at the playhead', 'S', 'Keyboard'] for x in results), results)
    time.sleep(0.3)
    r.shot('search')
    press('Return')
    r.check('Enter shows Keyboard with Split lit up', wait(lambda: r.s.run(TITLE) == 'Keyboard' and r.s.run(LIT, '[data-action=split]'), 3),
            (r.s.run(TITLE), r.s.run(LIT, '[data-action=split]')))
    r.s.run(TYPE, SEARCH, 'disk')
    r.check('"disk" finds Storage by another word for it', any(x.startswith('Storage') for x in (wait(lambda: r.s.run(RESULTS), 3) or [])), r.s.run(RESULTS))
    r.s.run(TYPE, SEARCH, 'dark mode')
    none = wait(lambda: 'No settings match' in r.s.run(TEXT, '[data-settings] [data-results]') and r.s.run(TEXT, '[data-settings] [data-results]'), 3)
    r.check('with nothing found it says so and what to try', none and 'Try a word' in none, none)
    r.s.run(TYPE, SEARCH, '')

    # Storage: Clear deletes what the open project no longer uses and keeps the rest.
    r.s.run(TAB, 'storage')
    row = wait(lambda: (t := r.s.run(ROW, 'sound')) and 'Prepared sound' in t and t, 10)
    data = usage(r)['data']
    r.check('the projects, plugins and style count, also when the data folder is a link, as it is here', data['bytes'] > 0 and data['path'], data)
    r.check('Storage lists the prepared sound with its size and what the open project keeps', row and 'kept for the open project' in row, row)
    time.sleep(0.5)
    r.shot('storage')
    r.check('Clear on the prepared sound runs', r.s.run(CLICK, 'Clear', '[data-storage=sound]'))
    cleared = wait(lambda: (t := r.s.run(ROW, 'sound')) and 'Cleared' in t and t, 10)
    r.check('the row says what it cleared and what it kept', cleared and 'kept for the open project' in cleared, cleared)
    after = usage(r)['sound']
    r.check("only the open project's sound is left", after['bytes'] == after['keptBytes'] and 0 < after['bytes'] < both, (after, both))
    time.sleep(0.5)
    r.shot('storage-cleared')
    duration, codecs = ffprobe(export(r, 'after-clear.mp4'))
    r.check('an export after Clear has its picture and sound', duration > 1 and 'aac' in codecs and 'h264' in codecs, (duration, codecs))

    # Learn from my edits reaches the style's settings in the data folder, which exports and agents read.
    r.s.run(TAB, 'ai')
    r.check('Learn from my edits starts on', wait(lambda: r.s.run(LEARN) == 'true', 5), r.s.run(LEARN))
    r.s.run("document.querySelector('[data-setting=learn] [role=switch]').click()")
    saved = r.work / 'data/nuzky/style/settings.json'
    r.check('turned off, it is saved in the data folder',
            wait(lambda: r.s.run(LEARN) == 'false' and saved.exists() and json.loads(saved.read_text()).get('learnFromEdits') is False, 5),
            saved.read_text() if saved.exists() else None)
    r.check('and the row says Your style now learns only when asked', 'learns only when you choose Learn my style' in r.s.run(TEXT, '[data-setting=learn]'))
    time.sleep(0.5)
    r.shot('pane-ai')

    # Plugins: an allowed plugin with full access is revoked here and asks again; its run after import turns on.
    listed = r.s.call("window.__nuzky.api.listPlugins().then((l) => JSON.stringify(l.find((p) => p.id === 'export-xml')))")
    plugin = json.loads(listed['value']) if listed['ok'] else {}
    r.check('Export XML is installed and asks for full access', plugin.get('accessKey') and not plugin.get('allowed'), listed)
    r.s.call("window.__nuzky.api.allowPluginAccess('export-xml', arguments[0])", plugin.get('accessKey'))
    r.s.run(TAB, 'plugins')
    r.check('Plugins lists Export XML as allowed, with Revoke', wait(lambda: r.s.run(BUTTON, 'Revoke', '[data-setting=plugins-access]'), 10))
    time.sleep(0.5)
    r.shot('pane-plugins')
    r.s.run(CLICK, 'Revoke', '[data-setting=plugins-access]')
    r.check('Revoke takes the access back: the plugin asks before its next run',
            wait(lambda: (p := r.s.run(PLUGIN)) and not p['allowed'] and 'Asks you before its next run' in r.s.run(TEXT, '[data-setting=plugins-access]'), 10),
            r.s.run(PLUGIN))
    box = "document.querySelector('[data-plugin=\"export-xml\"] input[type=checkbox]').click()"
    r.s.run(box)
    r.check('After import turns on its run after an import',
            wait(lambda: r.s.run("return window.__nuzky.plugins.getState().auto.import") == ['export-xml'], 3), r.s.run("return window.__nuzky.plugins.getState().auto"))
    r.s.run(box)

    # Privacy names what goes online and leads to the setting a row names.
    r.s.run(TAB, 'privacy')
    time.sleep(0.5)
    r.shot('pane-privacy')
    text = r.s.run(TEXT, '[data-settings] [data-setting=privacy]')
    r.check('Privacy names the update check, ChatGPT and Agent access', all(w in text for w in ('Update check', 'ChatGPT', 'Agent access')), text)
    r.s.run("[...document.querySelectorAll('[data-setting=privacy] button')].find((b) => b.innerText.startsWith('Agent access')).click()")
    r.check('its Agent access row shows AI with Agent access lit up', wait(lambda: r.s.run(TITLE) == 'AI' and r.s.run(LIT, '[data-setting=agent-access]'), 5),
            r.s.run(TITLE))

    for pane in ('models', 'sounds', 'keyboard'):
        r.s.run(TAB, pane)
        time.sleep(1)
        r.shot(f'pane-{pane}')
    r.s.run(TAB, 'storage')
    if r.check('the window goes to its smallest size', resize(r, 1024, 640)):
        time.sleep(0.5)
        fits = r.s.run(FITS)
        r.check('at 1024 x 640 Settings fits the window at 880 x 592', fits['fits'] and fits['width'] == 880 and fits['height'] == 592, fits)
        r.shot('storage-1024')
        resize(r, 1440, 900)

    r.key('Escape')
    r.check('Esc closes it', wait(lambda: not r.s.run(OPEN), 3))
    r.key(',', ctrlKey=True)
    r.check('Ctrl+, opens it again on the pane shown last', wait(lambda: r.s.run(TITLE) == 'Storage', 5), r.s.run(TITLE))
    r.key('Escape')
    wait(lambda: not r.s.run(OPEN), 3)

    r.s.run("window.__nuzky.store.setState({view: 'home'})")
    time.sleep(1)
    r.s.run("[...document.querySelectorAll('nav[aria-label=Collections] button')].find((b) => b.textContent.trim() === 'Settings')?.click()")
    r.check("the home screen's Settings row opens the same window", wait(lambda: r.s.run(TITLE) == 'Storage', 5), r.s.run(TITLE))
    r.s.run(TAB, 'general')
    time.sleep(0.5)
    r.shot('home')
    r.key('Escape')
    r.check('no error toast', not r.errors(), r.errors())

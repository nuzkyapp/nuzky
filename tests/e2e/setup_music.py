"""Get set up's Music from YouTube step downloads Nuzky's own yt-dlp only on a click, from a local stand-in for yt-dlp's
GitHub releases, on a computer whose installed yt-dlp YouTube refuses: a failure with Try again, Stop with Resume
download, one download at a time, done after it ends with the step skipped and when Nuzky already has it. A song that
failed in Music waits for its own Play or +, a failure Music shows under its link is no toast as well, and the song
then adds with Nuzky's copy and no further download."""
import time

from e2e.harness import FIXTURES, flow, press, wait
from e2e.home import focus_card, make, resize, wait_library
from e2e.setup import BOX, CLICK, FOCUS_TEXT, dialog, rail, relaunch, row
from e2e.youtube import PREVIEW, PROJECT, alert, answer, invocations, music, outdated, paste
from e2e.youtube import dialog as rights_dialog

STEP = """const p = document.querySelector('#setup-panel');
return p && {text: p.innerText, status: p.querySelector('[role=status]')?.innerText ?? null,
             alert: p.querySelector('[role=alert]')?.innerText ?? null};"""
# Every change of Get set up's footer, to catch a button shown for a moment.
WATCH = """const box = document.querySelector(arguments[0]);
window.__footers = [];
new MutationObserver(() => window.__footers.push([...box.querySelectorAll('footer button')].map((b) => b.textContent.trim())))
  .observe(box, {childList: true, subtree: true, characterData: true});"""
SOUNDS = 'return window.__nuzky.sounds.getState()'
OLD_SONG = '[data-sound="youtube:ytForbid001"]'
STOPPED = "Stopped. Resume download goes on where it stopped."


def step(r):
    return r.s.run(STEP, retries=3) or {}


def installed(r):
    """An installed yt-dlp YouTube refuses for ytForbid001 unless it runs as Nuzky's own copy, and a project to add to."""
    outdated(r)
    r.releases.correct = False
    make(r, 'song.nuzky', 'Song video', [FIXTURES / 'talk.mp4'], 0)


def view(r, name):
    r.s.run('window.__nuzky.store.setState({view: arguments[0]})', name)
    wait(lambda: r.s.run('return window.__nuzky.store.getState().view') == name, 5)


def open_setup(r):
    r.s.run(CLICK, 'Get set up', 'nav[aria-label=Collections]')
    return wait(lambda: (x := dialog(r)) and x['panel'] == 'Music from YouTube' and x, 5)


def downloads(calls):
    return [args for args in calls if '-o' in args]


@flow('setup-music', "Get set up's Music from YouTube step downloads Nuzky's own yt-dlp on a click, with Try again, Stop and "
      'one download at a time, is done once Nuzky has it, leaves a song asked for in Music to Music, and a YouTube link '
      'the installed yt-dlp cannot get then adds with Nuzky\'s copy', before=installed, home=True)
def setup_music(r):
    try:
        run(r)
    finally:
        r.releases.shutdown()
        r.releases.server_close()
        r.release_worker.join(timeout=5)


def run(r):
    tools = r.work / 'data/nuzky/tools'
    own = tools / 'yt-dlp'
    wait_library(r)
    r.s.run(CLICK, 'Get set up', 'nav[aria-label=Collections]')
    wait(lambda: dialog(r), 5)
    press('Down')
    press('Down')
    d = wait(lambda: (dialog(r) or {}).get('panel') == 'Music from YouTube' and dialog(r), 5)
    r.check('Music from YouTube is the third of four steps, to do',
            d and rail(d) == [['Captions model'], ['Face and person models'], ['Music from YouTube'], ['Connect your AI']] and d['count'] == '0 of 4 done', d)
    r.check('the focus stays on the step in the rail, not on Download yt-dlp', d and d['focus'] == 'tab: Music from YouTube', d)
    r.check('it offers Download yt-dlp before Skip', d and d['buttons'] == ['Download yt-dlp', 'Skip'], d)
    text = step(r).get('text', '')
    r.check('it says what Nuzky can do and why its own copy helps, also with yt-dlp installed',
            "Nuzky can add a song from a YouTube or YouTube Music link in Audio's Music tab." in text
            and "so YouTube doesn't refuse an old one" in text, text)
    r.check('showing the step downloads nothing and asks GitHub nothing', r.releases.requests == [] and not tools.exists(), r.releases.requests)
    time.sleep(0.3)
    r.shot('music-step')
    if resize(r, 1024, 640):
        time.sleep(0.5)
        r.shot('music-step-1024')
        resize(r, 1440, 900)

    r.s.run(FOCUS_TEXT, 'Download yt-dlp', f'{BOX} footer')
    press('Return')
    d = wait(lambda: step(r).get('alert') and (x := dialog(r)) and x['focus'] == 'Try again' and x, 10)
    r.check('a download that does not check out says why and offers Try again, which takes the focus',
            d and 'yt-dlp did not download correctly' in step(r)['alert'] and 'YTDLP_' not in step(r)['alert']
            and d['buttons'] == ['Try again', 'Skip'], {'step': step(r), 'dialog': dialog(r)})
    r.check('and leaves no yt-dlp behind', not own.exists())
    r.check('no toast repeats it over Get set up', not r.errors(), r.errors())
    r.shot('music-failed')

    r.releases.correct, r.releases.silent = True, True
    press('Return')
    d = wait(lambda: (x := dialog(r)) and x['focus'] == 'Stop' and x, 10)
    r.check('Try again downloads: the step shows Downloading yt-dlp and Stop, which takes the focus',
            d and 'Downloading yt-dlp' in (step(r).get('status') or ''), {'step': step(r), 'dialog': dialog(r)})
    r.check('the Music tab sees the same download', r.s.run('return window.__nuzky.sounds.getState().installing'))
    second = r.s.call('window.__nuzky.api.installYtdlp()')
    r.check('a second download meanwhile is refused', not second['ok'] and 'already downloading' in second['error'], second)
    r.shot('music-downloading')
    started = time.monotonic()
    press('Return')
    d = wait(lambda: (x := dialog(r)) and x['buttons'] == ['Resume download', 'Skip'] and x, 3, 0.1)
    r.check('Enter on Stop ends the download within three seconds and says, as Captions model does, that Resume download goes on',
            d and time.monotonic() - started <= 3 and step(r).get('status') == STOPPED, {'step': step(r), 'took': time.monotonic() - started})
    r.check('the focus goes to Resume download', wait(lambda: (dialog(r) or {}).get('focus') == 'Resume download', 3), dialog(r))
    r.check('a stopped download installs nothing', not own.exists())
    r.shot('music-stopped')
    r.s.run(CLICK, 'Close', BOX)

    # In Music, the installed yt-dlp finds the song but YouTube refuses it the download.
    focus_card(r, 'Song video')
    press('Return')
    r.check('the project opens', wait(lambda: r.s.run("return window.__nuzky.store.getState().view") == 'editor', 20))
    music(r)
    paste(r, 'https://youtu.be/ytForbid001')
    r.check('a pasted link first asks about the rights', wait(lambda: rights_dialog(r), 5))
    answer(r, 'I understand')
    r.check('the installed yt-dlp looks the song up', wait(lambda: r.s.run('return !!document.querySelector(arguments[0])', OLD_SONG), 10))
    r.seek(1_000_000)
    wait(lambda: r.s.run('return window.__nuzky.store.getState().timeUs') == 1_000_000, 5)
    before = r.s.run(PROJECT)
    r.s.run('document.querySelector(arguments[0]).click()', OLD_SONG + ' button[aria-label^=Add]')
    offer = wait(lambda: (t := alert(r)) and 'out of date' in t and t, 10) or ''
    r.check('+ on it offers Download yt-dlp in place of the row and adds nothing',
            'Download yt-dlp' in r.s.run('return document.querySelector("[role=alert] button")?.textContent ?? ""')
            and r.s.run(PROJECT) == before, offer)

    # Get set up's download fails while it is closed: Music says it under the link, and no toast says it again.
    view(r, 'home')
    r.releases.correct, r.releases.silent = False, False
    r.releases.requests.clear()
    r.check('Get set up opens on Music from YouTube, still with Resume download', (d := open_setup(r)) and d['buttons'] == ['Resume download', 'Skip'], dialog(r))
    r.s.run(f"""[...document.querySelectorAll('{BOX} footer button')].find((b) => b.textContent === 'Resume download').click();
document.querySelector('{BOX} button[aria-label=Close]').click();""")
    ended = wait(lambda: (s := r.s.run(SOUNDS)) and not s['installing'] and s['installError'], 10) or ''
    time.sleep(0.5)
    r.check('the download fails its check with Get set up closed', ended.startswith('YTDLP_CORRUPT') and not dialog(r), ended)
    r.check('Music shows it under the link, so no toast says it again', not r.errors(), r.errors())
    view(r, 'editor')
    shown = wait(lambda: (t := alert(r)) and 'did not download correctly' in t and t, 5)
    r.check('Music shows it under the link', shown, alert(r))

    # Get set up's download succeeds: the step is done at once, and the song + asked for in Music waits for Music.
    view(r, 'home')
    r.releases.correct = True
    r.releases.requests.clear()
    calls = len(invocations(r))
    d = open_setup(r)
    r.check('Get set up shows that failure with Try again', d and 'did not download correctly' in (step(r).get('alert') or '')
            and d['buttons'] == ['Try again', 'Skip'], {'step': step(r), 'dialog': d})
    r.s.run(WATCH, BOX)
    r.s.run(FOCUS_TEXT, 'Try again', f'{BOX} footer')
    press('Return')
    d = wait(lambda: (x := dialog(r)) and rail(x)[2] == ['Music from YouTube', 'Done'] and x, 15)
    r.check('Try again downloads Nuzky\'s own yt-dlp, the release\'s file, executable, and the step is done',
            d and d['count'] == '1 of 4 done' and own.exists() and own.read_bytes() == r.tool and own.stat().st_mode & 0o777 == 0o755
            and r.releases.requests == ['/latest', '/' + r.release], {'dialog': dialog(r), 'requests': r.releases.requests.copy()})
    r.check('the focus goes to Next', wait(lambda: (dialog(r) or {}).get('focus') == 'Next', 3), dialog(r))
    seen = r.s.run('return window.__footers') or []
    begun = next((i for i, buttons in enumerate(seen) if 'Try again' not in buttons), None)
    flashed = [b for b in seen[begun:] if {'Try again', 'Download yt-dlp', 'Resume download'} & set(b)] if begun is not None else None
    r.check('on its way to done the step never shows its download button again', flashed == [] and seen[-1:] == [['Next']], seen)
    r.check('the step says yt-dlp is downloaded and offers only Next', 'yt-dlp is downloaded' in step(r).get('text', '')
            and d and d['buttons'] == ['Next'], step(r))
    time.sleep(0.3)
    r.shot('music-done')
    looked = wait(lambda: (r.s.run(SOUNDS)['link']['sound'] or {}).get('title') == 'Old Song', 10)
    r.check('the link waiting for yt-dlp is looked up again', looked, r.s.run(SOUNDS)['link'])
    replayed = wait(lambda: r.s.run(PROJECT) != before or r.s.run(PREVIEW) or downloads(invocations(r)[calls:]), 5)
    r.check('the download from Get set up neither adds the song + asked for in Music nor plays it',
            not replayed and r.s.run(PROJECT) == before and not downloads(invocations(r)[calls:]), invocations(r)[calls:])
    r.s.run(CLICK, 'Close', BOX)

    # The installed yt-dlp is refused, so the song adds only if Nuzky's copy goes first.
    view(r, 'editor')
    r.check('Music shows the song, with no offer to download yt-dlp', wait(lambda: r.s.run('return !!document.querySelector(arguments[0])', OLD_SONG), 5)
            and 'Download yt-dlp' not in (alert(r) or ''), alert(r))
    r.seek(2_000_000)
    wait(lambda: r.s.run('return window.__nuzky.store.getState().timeUs') == 2_000_000, 5)
    r.s.run('document.querySelector(arguments[0]).click()', OLD_SONG + ' button[aria-label^=Add]')
    wait(lambda: any(a['name'] == 'Old Song' for a in r.state()['assets']) or 'Download yt-dlp' in (alert(r) or ''), 15)
    project = r.s.run(PROJECT)
    asset = next((a for a in project['assets'] if a['name'] == 'Old Song'), None)
    placed = [c for t in project['tracks'] for c in t['clips'] if asset and c['content'].get('assetId') == asset['id']]
    r.check("+ adds the song the installed yt-dlp could not get, with Nuzky's copy, once, at the playhead, and offers no download",
            asset and len(placed) == 1 and placed[0]['startUs'] == 2_000_000 and 'Download yt-dlp' not in (alert(r) or '')
            and r.releases.requests == ['/latest', '/' + r.release], {'placed': placed, 'alert': alert(r), 'requests': r.releases.requests.copy()})
    r.shot('music-link')
    r.check('no error toast', not r.errors(), r.errors())

    # Download again from Get set up while Music shows a link that does not wait for yt-dlp.
    paste(r, 'https://youtu.be/ytPrivate01')
    r.check('a private video says so under the link', wait(lambda: 'private' in (alert(r) or ''), 10), alert(r))
    calls = len(invocations(r))
    # As on a computer Nuzky's copy was removed from, so Get set up downloads it once more.
    own.unlink()
    (tools / 'yt-dlp.checked').unlink()
    r.s.run("window.__nuzky.setup.setState({states: {}})")
    view(r, 'home')
    r.releases.requests.clear()
    r.check('Get set up offers Download yt-dlp again', (d := open_setup(r)) and d['buttons'] == ['Download yt-dlp', 'Skip'], dialog(r))
    # Skip in the same moment, so the download ends with the step already skipped.
    r.s.run(f"""[...document.querySelectorAll('{BOX} footer button')].find((b) => b.textContent === 'Download yt-dlp').click();
[...document.querySelectorAll('{BOX} footer button')].find((b) => b.textContent === 'Skip').click();""")
    r.check('Skip during the download shows Connect your AI', wait(lambda: (dialog(r) or {}).get('panel') == 'Connect your AI', 5), dialog(r))
    d = wait(lambda: (x := dialog(r)) and rail(x)[2] == ['Music from YouTube', 'Done'] and x, 15)
    r.check('once the download ends, Music from YouTube is done, though it was skipped', d and d['count'] == '1 of 4 done' and own.exists(), dialog(r))
    looked = wait(lambda: len(invocations(r)) > calls, 3)
    r.check('the private video is not looked up again', not looked and (r.s.run(SOUNDS)['link']['error'] or '').startswith('SOUND_UNAVAILABLE'),
            {'calls': invocations(r)[calls:], 'link': r.s.run(SOUNDS)['link']})

    # Forgotten states, as on an install that got yt-dlp before this step existed: the step finds Nuzky's copy itself.
    r.s.run("window.__nuzky.setup.setState({states: {}, current: 'captions'})")
    r.check('forgotten, the step is to do', wait(lambda: rail(dialog(r))[2] == ['Music from YouTube'], 5), dialog(r))
    r.s.run(f"document.querySelector('{BOX} #setup-tab-music').click()")
    r.check('with Nuzky already having yt-dlp, the step is done again on showing', wait(lambda: rail(dialog(r))[2] == ['Music from YouTube', 'Done'], 5), dialog(r))
    r.s.run("window.__nuzky.setup.setState({states: {}})")
    r.s.run(CLICK, 'Close', BOX)
    relaunch(r)
    wait_library(r)
    r.check('after a restart, the sidebar counts three steps left: Nuzky already has yt-dlp',
            wait(lambda: row(r).get('count') == '3 steps left', 10), row(r))

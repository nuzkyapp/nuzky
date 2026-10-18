"""YouTube music stays behind the rights notice, previews and adds without credit, and ducks in the export.
All yt-dlp lookups and downloads are served by the executable fake, without contacting YouTube."""
import hashlib, json, platform, statistics, subprocess, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from e2e.ducking import CHECKBOX, SOURCE, TONE, WINDOW, RATE, decode, speech_levels, tone_levels
from e2e.harness import Bridge, flow, export, press, wait
from e2e.layout import resize

SONG = 'yt_Song0001'
ROW = f'[data-sound="youtube:{SONG}"]'
FIELD = 'input[aria-label="Paste a YouTube link"]'
DIALOG = 'dialog[aria-labelledby="youtube-rights-title"][open]'
CLICK = 'document.querySelector(arguments[0]).click()'
PROJECT = 'return window.__nuzky.store.getState().snap.project'
PREVIEW = 'return window.__nuzky.sounds.getState().preview'


def prepare(r):
    audio = r.work / 'tone.m4a'
    subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i',
                    f'sine=frequency={TONE}:sample_rate={RATE}:duration=20', '-c:a', 'aac', str(audio)], check=True)
    r.log = r.work / 'ytdlp.jsonl'
    r.log.touch()
    r.env.update(NUZKY_YTDLP=str(Path(__file__).with_name('fake_ytdlp')),
                 NUZKY_FAKE_YTDLP_AUDIO=str(audio), NUZKY_FAKE_YTDLP_LOG=str(r.log))


def missing(r):
    prepare(r)
    r.env['NUZKY_YTDLP'] = str(r.work / 'not-installed')
    r.release = 'yt-dlp_macos' if platform.system() == 'Darwin' else 'yt-dlp_linux'
    r.tool = Path(__file__).with_name('fake_ytdlp').read_bytes()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            r.releases.requests.append(self.path)
            if self.path == '/latest':
                sha = hashlib.sha256(r.tool).hexdigest() if r.releases.correct else '0' * 64
                body = json.dumps({'tag_name': '2099.01.01', 'assets': [
                    {'name': r.release, 'size': len(r.tool), 'digest': f'sha256:{sha}',
                     'browser_download_url': f'http://127.0.0.1:{r.releases.server_port}/{r.release}'}]}).encode()
            elif self.path == '/' + r.release:
                if r.releases.silent:
                    time.sleep(20)
                    return
                body = r.tool
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    r.releases = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    r.releases.requests, r.releases.correct, r.releases.silent = [], False, False
    r.release_worker = threading.Thread(target=r.releases.serve_forever, daemon=True)
    r.release_worker.start()
    r.env['NUZKY_YTDLP_RELEASES'] = f'http://127.0.0.1:{r.releases.server_port}/latest'


def music(r):
    r.s.run("window.__nuzky.store.setState({panelTab: 'audio'})")
    wait(lambda: r.s.run("return !!document.querySelector('#audio-tab-music')"), 5)
    r.s.run(CLICK, '#audio-tab-music')
    r.check('Music shows Paste a YouTube link', wait(lambda: r.s.run('return !!document.querySelector(arguments[0])', FIELD), 5))


def paste(r, url):
    if r.s.run('return !!document.querySelector("button[aria-label=\\"Clear link\\"]")'):
        r.s.run(CLICK, 'button[aria-label="Clear link"]')
    r.s.run("""const input = document.querySelector(arguments[0]); input.focus();
const event = new Event('paste', {bubbles: true, cancelable: true});
Object.defineProperty(event, 'clipboardData', {value: {getData: () => arguments[1]}});
input.dispatchEvent(event);""", FIELD, url)


def dialog(r):
    return r.s.run('return !!document.querySelector(arguments[0])', DIALOG)


def answer(r, text):
    r.s.run("[...document.querySelectorAll(arguments[0] + ' button')].find((b) => b.textContent === arguments[1]).click()", DIALOG, text)


def invocations(r):
    return [json.loads(line) for line in r.log.read_text().splitlines()]


def alert(r):
    return r.s.run('return document.querySelector("[role=alert]")?.innerText ?? ""')


@flow('youtube', __doc__.split('.')[0], before=prepare)
def youtube(r):
    r.import_media(SOURCE)
    r.add_clip(SOURCE.name)
    music(r)
    # The notice changes no project, so it answers while an agent's run locks the editor.
    bridge = Bridge(r, r.saved_project())
    run = bridge.call('begin_run', {'label': 'Rough cut'})
    try:
        r.check('an agent run locks the editor', wait(lambda: r.state()['aiRun'], 10))
        paste(r, f'https://music.youtube.com/watch?v={SONG}&si=test')
        r.check('pasting first opens the rights notice with Cancel and I understand',
                wait(lambda: dialog(r), 5) and r.s.run("return document.querySelector(arguments[0]).innerText.includes('Music from YouTube is yours to clear') && [...document.querySelectorAll(arguments[0] + ' button')].map((b) => b.textContent).join('|') === 'Cancel|I understand'", DIALOG))
        r.check('yt-dlp has not run before the notice is answered', invocations(r) == [], invocations(r))
        r.check('the notice buttons are not locked by the agent run',
                r.s.run("return [...document.querySelectorAll(arguments[0] + ' button')].every((b) => !b.hasAttribute('aria-disabled'))", DIALOG))
        time.sleep(0.4)
        r.shot('rights-dialog')
        answer(r, 'Cancel')
        r.check('Cancel closes the notice without looking up or adding anything',
                wait(lambda: not dialog(r), 5) and invocations(r) == [] and len(r.state()['assets']) == 1)
        paste(r, f'https://youtu.be/{SONG}?si=test')
        r.check('pasting after Cancel asks again and still runs nothing', wait(lambda: dialog(r), 5) and invocations(r) == [])
        answer(r, 'I understand')
        row = wait(lambda: r.s.run('return document.querySelector(arguments[0])?.innerText ?? ""', ROW), 10) or ''
        r.check('the song row has its title, artist, YouTube and duration',
                all(text in row for text in ('Steady Song', 'Test Artist · YouTube', '20.0 s')), row)
        bridge.call('end_run', {'run_id': run['run_id'], 'action': 'keep'})
    finally:
        bridge.close()
    r.check('the lock ends with the run', wait(lambda: not r.state()['aiRun'], 10))
    r.check('YouTube has no licence chip', r.s.run('return !!document.querySelector(arguments[0]) && !document.querySelector(arguments[0] + " button[aria-label^=CC]")', ROW))
    settings = r.work / 'data/nuzky/sound-library.json'
    r.check('accepting saves youtubeRights on disk', settings.exists() and json.loads(settings.read_text()).get('youtubeRights') is True)
    r.check('looking up runs only metadata and adds nothing',
            len(invocations(r)) == 1 and '--skip-download' in invocations(r)[0] and len(r.state()['assets']) == 1, invocations(r))
    r.shot('looked-up-row')

    r.s.run(CLICK, ROW + ' button[aria-label^=Play]')
    playing = wait(lambda: (p := r.s.run(PREVIEW)) and p['id'] == f'youtube:{SONG}' and not p['loading'], 15)
    r.check('Play downloads and previews the song with a Stop button',
            playing and r.s.run('return !!document.querySelector(arguments[0])', ROW + ' button[aria-label="Stop Steady Song"]')
            and any('-o' in args for args in invocations(r)), invocations(r))
    r.check('previewing adds nothing', len(r.state()['assets']) == 1)
    r.s.run(CLICK, ROW + ' button[aria-label="Stop Steady Song"]')
    r.check('Stop ends the preview', wait(lambda: r.s.run(PREVIEW) is None, 5))
    r.seek(1_000_000)
    wait(lambda: r.s.run('return window.__nuzky.store.getState().timeUs') == 1_000_000, 5)
    r.s.run(CLICK, ROW + ' button[aria-label^=Add]')
    wait(lambda: len(r.state()['assets']) == 2, 15)
    project = r.s.run(PROJECT)
    asset = next((a for a in project['assets'] if a['name'] == 'Steady Song'), None)
    r.check('+ adds the song as an audio asset without credit', asset and asset['kind'] == 'audio' and asset.get('credit') is None, asset)
    if not asset:
        raise RuntimeError('the YouTube song was not added')
    placed = [(t, c) for t in project['tracks'] for c in t['clips'] if c['content'].get('assetId') == asset['id']]
    r.check('the music clip starts at the playhead on an audio track',
            len(placed) == 1 and placed[0][0]['kind'] == 'audio' and placed[0][0].get('keepInPlace') is True
            and placed[0][1]['startUs'] == 1_000_000, placed)
    file = r.work / f'data/nuzky/sounds/youtube-{SONG}.m4a'
    r.check('the added file is kept in the library with the downloaded audio',
            Path(asset['path']).resolve() == file.resolve() and file.read_bytes() == (r.work / 'tone.m4a').read_bytes())
    r.check('adding reuses the preview download', sum('-o' in args for args in invocations(r)) == 1, invocations(r))
    r.shot('clip-on-timeline')
    off = export(r, 'without-ducking.mp4')
    r.focus_clip(placed[0][1]['id'])
    r.check('the inspector offers Lower under speech, initially off',
            wait(lambda: r.s.run(f'return {CHECKBOX}?.checked === false'), 5))
    r.s.run(f'{CHECKBOX}.click()')
    duck = 'return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips).find((c) => c.id === arguments[0])?.content.duckDb'
    r.check('Lower under speech turns on for the YouTube clip', wait(lambda: r.s.run(duck, placed[0][1]['id']) == 12, 5))
    on = export(r, 'lower-under-speech.mp4')
    filters = f'highpass=f={TONE - 500},highpass=f={TONE - 500}'
    gain = [b - a for a, b in zip(tone_levels(decode(off, filters)), tone_levels(decode(on, filters)))]
    voice = speech_levels(decode(SOURCE, 'anull'))
    loudest = max(voice)
    speech = [i for i, v in enumerate(voice) if v > loudest - 15 and i >= RATE // WINDOW]
    runs, start = [], None
    for i, quiet in enumerate([v < loudest - 35 for v in voice] + [False]):
        if quiet and start is None:
            start = i
        elif not quiet and start is not None:
            runs.append((start, i))
            start = None
    first, last = max(runs, key=lambda run: run[1] - run[0])
    pause = list(range(max(first + 14, RATE // WINDOW), last - 4))
    under, back = statistics.median(gain[i] for i in speech), statistics.median(gain[i] for i in pause)
    r.check('the exported music is 12 dB lower under speech', abs(under + 12) < 1.5, f'{under:+.1f} dB')
    r.check('the exported music returns to full level in the long pause', abs(back) < 1, f'{back:+.1f} dB')

    before = r.s.run(PROJECT)
    calls = len(invocations(r))
    paste(r, 'https://youtube.com/playlist?list=PLtest')
    error = wait(lambda: alert(r), 5) or ''
    r.check('a playlist shows a plain error under the field', 'playlist' in error and 'one song' in error and 'PLAYLIST:' not in error, error)
    r.check('a playlist adds nothing and never runs yt-dlp', r.s.run(PROJECT) == before and len(invocations(r)) == calls)
    r.shot('playlist-error')
    paste(r, 'https://youtu.be/ytPrivate01')
    error = wait(lambda: alert(r), 10) or ''
    r.check('a private video shows a plain error and adds nothing',
            'private' in error and 'SOUND_UNAVAILABLE:' not in error and r.s.run(PROJECT) == before, error)
    paste(r, f'https://youtu.be/{SONG}')
    r.check('another paste shows the row without asking about rights again',
            wait(lambda: r.s.run('return !!document.querySelector(arguments[0])', ROW), 10) and not dialog(r))
    r.check('the row of a song the project has shows the check',
            r.s.run('return !!document.querySelector(arguments[0])', ROW + ' [title="In this project"]'))
    r.s.run(CLICK, 'button[aria-label="Clear link"]')
    r.s.run("""const input = document.querySelector(arguments[0]); input.focus();
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, arguments[1]);
input.dispatchEvent(new Event('input', {bubbles: true}));""", FIELD, f'https://youtu.be/{SONG}')
    press('Return')
    r.check('Enter also looks up a typed link without another rights notice',
            wait(lambda: r.s.run('return !!document.querySelector(arguments[0])', ROW), 10) and not dialog(r))

    slow = '[data-sound="youtube:ytSlow00001"]'
    paste(r, 'https://youtu.be/ytSlow00001')
    wait(lambda: r.s.run('return !!document.querySelector(arguments[0])', slow), 10)
    assets = len(r.state()['assets'])
    r.s.run(CLICK, slow + ' button[aria-label^=Add]')
    wait(lambda: any('-o' in args and args[-1].endswith('ytSlow00001') for args in invocations(r)), 10)
    started = time.time()
    # The button stops only on a single click, as a double click on + must not stop the add it started.
    r.s.run("document.querySelector(arguments[0]).dispatchEvent(new MouseEvent('click', {bubbles: true, detail: 1}))",
            slow + ' button[aria-label="Stop adding Slow Song"]')
    stopped = wait(lambda: r.s.run('return window.__nuzky.sounds.getState().adding.length === 0'), 5)
    took = time.time() - started
    time.sleep(0.5)
    left = [p.name for p in r.work.rglob('*') if p.name.startswith('.nuzky-part-') or 'ytSlow' in p.name]
    running = subprocess.run(['pgrep', '-f', 'nuzky-fake-ytdlp-child'], capture_output=True, text=True).stdout.split()
    r.check('Stop ends a YouTube download and what yt-dlp started within seconds, adds nothing and leaves no part behind',
            stopped and took < 3 and len(r.state()['assets']) == assets and left == [] and running == [],
            (round(took, 1), left, running))
    r.check('every invocation gets only a canonical watch URL',
            bool(invocations(r)) and all(args[args.index('--') + 1:] in ([f'https://www.youtube.com/watch?v={v}'] for v in (SONG, 'ytPrivate01', 'ytSlow00001'))
                                         for args in invocations(r)), invocations(r))
    if r.check('the window goes to its smallest size', resize(r, 1024, 640)):
        time.sleep(0.5)
        r.shot('music-1024')
    r.check('no error toasts', not r.errors(), r.errors())


@flow('youtube-missing', 'A missing yt-dlp downloads in Music, rejects corrupt bytes and retries the link', before=missing)
def youtube_missing(r):
    try:
        music(r)
        paste(r, f'https://youtu.be/{SONG}')
        r.check('the notice still precedes lookup when yt-dlp is missing', wait(lambda: dialog(r), 5))
        answer(r, 'I understand')
        offer = wait(lambda: alert(r), 10) or ''
        button = '[role=alert] button'
        label = 'return document.querySelector(arguments[0])?.textContent ?? ""'
        r.check('missing yt-dlp offers Download yt-dlp without a terminal command or error code',
                'Music from YouTube comes through yt-dlp, a free tool. Nuzky can download it for you.' in offer
                and r.s.run(label, button) == 'Download yt-dlp'
                and not any(text in offer for text in ('brew', 'winget', 'pipx', 'YTDLP_MISSING:')), offer)
        tools = r.work / 'data/nuzky/tools'
        own = tools / 'yt-dlp'
        project = r.s.run(PROJECT)
        r.check('the offer downloads nothing and adds no asset or clip',
                list(tools.glob('*')) == [] and r.releases.requests == [] and invocations(r) == []
                and project['assets'] == [] and all(not t['clips'] for t in project['tracks']))
        r.shot('download-ytdlp-offer')

        r.s.run(CLICK, button)
        error = wait(lambda: (text := alert(r)) and 'did not download correctly' in text and text, 10) or ''
        r.check('a wrong checksum shows a plain error with Try again',
                'yt-dlp did not download correctly. Try again.' in error and 'YTDLP_CORRUPT:' not in error
                and r.s.run(label, button) == 'Try again', error)
        r.check('a wrong checksum leaves no tool, stamp or partial download and changes no project',
                not own.exists() and not (tools / 'yt-dlp.checked').exists()
                and list(tools.glob('*.part')) == [] and r.s.run(PROJECT) == project and invocations(r) == [])
        r.check('the corrupt download requested the checksum and platform release only',
                r.releases.requests == ['/latest', '/' + r.release], r.releases.requests.copy())
        r.shot('corrupt-ytdlp-error')

        r.releases.correct, r.releases.silent = True, True
        r.s.run(CLICK, button)
        r.check('Try again brings back Download yt-dlp without another rights notice',
                wait(lambda: r.s.run(label, button) == 'Download yt-dlp', 10) and not dialog(r))
        r.s.run(CLICK, button)
        wait(lambda: r.s.run(label, button) == 'Stop', 5)
        stopped_at = time.time()
        r.s.run(CLICK, button)
        back = wait(lambda: r.s.run(label, button) == 'Download yt-dlp', 5)
        r.check('Stop ends a download from a silent server within seconds and installs nothing',
                back and time.time() - stopped_at < 3 and not own.exists(), round(time.time() - stopped_at, 1))
        r.releases.silent = False
        r.releases.requests.clear()
        r.s.run(CLICK, button)
        row = wait(lambda: r.s.run('return document.querySelector(arguments[0])?.innerText ?? ""', ROW), 10) or ''
        r.check('after downloading the pasted song appears by itself with title, artist and duration',
                all(text in row for text in ('Steady Song', 'Test Artist · YouTube', '20.0 s')), row)
        r.check('the download fetched the release and its file from the local release server',
                r.releases.requests == ['/latest', '/' + r.release], r.releases.requests.copy())
        r.check('the installed tool is executable, matches the release bytes and has a checked stamp',
                own.exists() and own.read_bytes() == r.tool and own.stat().st_mode & 0o777 == 0o755
                and (tools / 'yt-dlp.checked').is_file())
        r.check('automatic lookup runs the downloaded tool despite the missing override and adds nothing',
                len(invocations(r)) == 1 and '--skip-download' in invocations(r)[0] and r.s.run(PROJECT) == project,
                invocations(r))
        r.shot('downloaded-song-row')

        r.import_media(SOURCE)
        r.add_clip(SOURCE.name)
        r.seek(1_000_000)
        wait(lambda: r.s.run('return window.__nuzky.store.getState().timeUs') == 1_000_000, 5)
        r.s.run(CLICK, ROW + ' button[aria-label^=Add]')
        wait(lambda: len(r.state()['assets']) == 2, 15)
        project = r.s.run(PROJECT)
        asset = next((a for a in project['assets'] if a['name'] == 'Steady Song'), None)
        placed = [(t, c) for t in project['tracks'] for c in t['clips']
                  if asset and c['content'].get('assetId') == asset['id']]
        r.check('+ adds the downloaded song on an audio track at the playhead',
                asset and asset['kind'] == 'audio' and len(placed) == 1 and placed[0][0]['kind'] == 'audio'
                and placed[0][1]['startUs'] == 1_000_000, placed)
        r.check('the added song uses the fake audio and never contacts YouTube',
                asset and Path(asset['path']).read_bytes() == (r.work / 'tone.m4a').read_bytes()
                and len(invocations(r)) == 2 and '-o' in invocations(r)[1], invocations(r))
    finally:
        r.releases.shutdown()
        r.releases.server_close()
        r.release_worker.join(timeout=5)


def outdated(r):
    missing(r)
    r.env['NUZKY_YTDLP'] = str(Path(__file__).with_name('fake_ytdlp'))
    r.releases.correct = True


@flow('youtube-outdated', 'An installed yt-dlp YouTube refuses offers Nuzky\'s own and then adds the song', before=outdated)
def youtube_outdated(r):
    try:
        forbidden = '[data-sound="youtube:ytForbid001"]'
        r.import_media(SOURCE)
        r.add_clip(SOURCE.name)
        music(r)
        paste(r, 'https://youtu.be/ytForbid001')
        wait(lambda: dialog(r), 5)
        answer(r, 'I understand')
        r.check('the installed yt-dlp looks the song up',
                wait(lambda: r.s.run('return !!document.querySelector(arguments[0])', forbidden), 10))
        before = r.s.run(PROJECT)
        r.s.run(CLICK, forbidden + ' button[aria-label^=Add]')
        offer = wait(lambda: (text := alert(r)) and 'out of date' in text and text, 10) or ''
        r.check('a download the installed yt-dlp fails offers Download yt-dlp in place of the row, without a code',
                "couldn't get this song, probably because it's out of date" in offer
                and r.s.run('return document.querySelector("[role=alert] button")?.textContent') == 'Download yt-dlp'
                and not r.s.run('return !!document.querySelector(arguments[0])', forbidden)
                and 'YTDLP_OUTDATED' not in offer and '403' not in offer, offer)
        r.check('the failed download adds nothing, downloads no yt-dlp and shows no error toast',
                r.s.run(PROJECT) == before and r.releases.requests == [] and not r.errors(), r.errors())
        r.shot('outdated-ytdlp-offer')

        r.s.run(CLICK, '[role=alert] button')
        added = wait(lambda: any(a['name'] == 'Old Song' for a in r.state()['assets']), 15)
        project = r.s.run(PROJECT)
        asset = next((a for a in project['assets'] if a['name'] == 'Old Song'), None)
        placed = [c for t in project['tracks'] for c in t['clips'] if asset and c['content'].get('assetId') == asset['id']]
        r.check('after Download yt-dlp the song is added by itself, once, without another click',
                added and len(placed) == 1 and asset['kind'] == 'audio'
                and Path(asset['path']).read_bytes() == (r.work / 'tone.m4a').read_bytes(), placed)
        own = r.work / 'data/nuzky/tools/yt-dlp'
        downloads = [args for args in invocations(r) if '-o' in args]
        r.check("Nuzky's own copy came from the release server and did the second download",
                r.releases.requests == ['/latest', '/' + r.release] and own.read_bytes() == r.tool and len(downloads) == 2,
                (r.releases.requests, len(downloads)))
        r.shot('song-added-after-download')
        r.check('no error toasts', not r.errors(), r.errors())
    finally:
        r.releases.shutdown()
        r.releases.server_close()
        r.release_worker.join(timeout=5)

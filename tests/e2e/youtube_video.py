"""Video from YouTube downloads a video, plays it in its own preview, and imports the chosen part or all of it.
All yt-dlp lookups and downloads are served by the executable fake, without contacting YouTube."""
import json, subprocess, time
from pathlib import Path

from e2e.harness import flow, wait
from e2e.layout import resize

LONG = 'ytVideo0001'
SHORT = 'ytShort0001'
DIALOG = '[role=dialog][aria-labelledby="youtube-title"]'
FIELD = 'input[aria-label="Link to a YouTube video"]'
CLICK = 'document.querySelector(arguments[0]).click()'


def prepare(r):
    videos = r.work / 'videos'
    videos.mkdir()
    for video, seconds in ((LONG, 90), (SHORT, 8)):
        # testsrc2 prints the time in every frame, so a frame shows where in the video it is.
        subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', f'testsrc2=size=640x360:rate=30:duration={seconds}',
                        '-c:v', 'libx264', '-g', '60', '-pix_fmt', 'yuv420p', str(videos / f'{video}.mp4')], check=True)
        subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', f'sine=frequency=440:duration={seconds}',
                        '-c:a', 'aac', str(videos / f'{video}.m4a')], check=True)
    r.log = r.work / 'ytdlp.jsonl'
    r.log.touch()
    r.env.update(NUZKY_YTDLP=str(Path(__file__).with_name('fake_ytdlp')), NUZKY_FAKE_YTDLP_VIDEOS=str(videos),
                 NUZKY_FAKE_YTDLP_AUDIO=str(videos / f'{SHORT}.m4a'), NUZKY_FAKE_YTDLP_LOG=str(r.log))


def is_open(r):
    return r.s.run('return !!document.querySelector(arguments[0])', DIALOG)


def text(r):
    return r.s.run('return document.querySelector(arguments[0])?.innerText ?? ""', DIALOG)


def button(r, label):
    r.s.run("[...document.querySelectorAll(arguments[0] + ' button')].find((b) => b.textContent.trim().startsWith(arguments[1])).click()", DIALOG, label)


def paste(r, url):
    r.s.run("""const input = document.querySelector(arguments[0]); input.focus();
const event = new Event('paste', {bubbles: true, cancelable: true});
Object.defineProperty(event, 'clipboardData', {value: {getData: () => arguments[1]}});
input.dispatchEvent(event);""", FIELD, url)


def ready(r):
    return r.s.run('return !!document.querySelector(arguments[0] + " canvas")', DIALOG)


def strip(r):
    return r.s.run('return !!document.querySelector(arguments[0] + " [data-strip=ready]")', DIALOG)


def type_time(r, label, value):
    r.s.run("""const input = document.querySelector(`input[aria-label="${arguments[0]}"]`); input.focus();
const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
set.call(input, arguments[1]); input.dispatchEvent(new Event('input', {bubbles: true})); input.blur();""", label, value)


def picture(r):
    """How many pixels of the dialog's preview are not black, so a drawn frame is told from an empty canvas."""
    return r.s.run("""const c = document.querySelector(arguments[0] + ' canvas');
const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let lit = 0;
for (let i = 0; i < d.length; i += 4) if (d[i] + d[i + 1] + d[i + 2] > 60) lit++; return lit;""", DIALOG)


def frame_time(r):
    return int(r.s.run('return Number(document.querySelector(arguments[0] + " canvas").dataset.us ?? -1)', DIALOG))


def key(r, k):
    r.s.run("""document.querySelector(arguments[0] + ' canvas').closest('[class]').dispatchEvent(
new KeyboardEvent('keydown', {key: arguments[1], bubbles: true, cancelable: true}))""", DIALOG, k)


def videos(r):
    return r.s.run("return window.__nuzky.store.getState().snap.project.assets.filter((a) => a.name.startsWith('Test video'))")


def clip_of(r, asset_id):
    return r.s.run("""return window.__nuzky.store.getState().snap.project.tracks.flatMap((t) => t.clips)
.find((c) => c.content.assetId === arguments[0])""", asset_id)


@flow('youtube_video', __doc__.split('.')[0], before=prepare)
def youtube_video(r):
    r.s.run("window.__nuzky.store.setState({panelTab: 'media'})")
    r.check('Media offers From YouTube', wait(lambda: r.s.run('return !!document.querySelector("button[aria-label=\\"Video from YouTube\\"]")'), 5))
    r.shot('media-empty')
    r.s.run(CLICK, 'button[aria-label="Video from YouTube"]')
    r.check('the dialog opens with the link field focused',
            wait(lambda: is_open(r), 5) and r.s.run('return document.activeElement === document.querySelector(arguments[0])', FIELD))
    r.check('Import waits for a video', r.s.run("return [...document.querySelectorAll(arguments[0] + ' button')].find((b) => b.textContent === 'Import').getAttribute('aria-disabled') === 'true'", DIALOG))
    r.shot('empty')

    paste(r, f'https://www.youtube.com/watch?v={LONG}&t=12s')
    r.check('the first link asks about the rights before yt-dlp runs',
            wait(lambda: 'yours to clear' in text(r), 5) and r.log.read_text() == '')
    time.sleep(0.4)
    r.shot('rights')
    button(r, 'I understand')
    r.check('the video downloads and its preview draws a frame', wait(lambda: ready(r) and picture(r) > 1000, 30), text(r))
    calls = [json.loads(line) for line in r.log.read_text().splitlines()]
    r.check('yt-dlp only ever gets the watch address built from the id',
            all(c[c.index('--') + 1:] == [f'https://www.youtube.com/watch?v={LONG}'] for c in calls) and len(calls) == 3, calls)
    r.check('title, channel and length show, and the whole video is the default',
            all(t in text(r) for t in (f'Test video {LONG}', 'Test Channel · 1:30', 'Import whole video')), text(r))
    r.check('a long video gets the overview and the zoom', r.s.run('return !!document.querySelector("button[aria-label=\\"Zoom in\\"]")'))
    r.check('the strip shows frames of the video', wait(lambda: strip(r), 10))
    r.check('Play has focus, so Space, I, O and Enter work at once',
            r.s.run('return document.activeElement?.getAttribute("aria-label") === "Play (Space)"'))
    r.shot('downloaded')

    type_time(r, 'Start of the part', '0:30')
    type_time(r, 'End of the part', '0:34')
    r.check('typed times choose a 4 s part and the button says so', wait(lambda: 'Import 4.0 s' in text(r), 3), text(r))
    r.check('the preview shows the frame at the typed end', wait(lambda: abs(frame_time(r) - 34_000_000) < 50_000, 5), frame_time(r))
    r.s.run("window.__nuzky.api.youtubePreviewSeek(31000000)")
    wait(lambda: abs(frame_time(r) - 31_000_000) < 50_000, 5)
    key(r, 'i')
    r.check('I sets the start at the playhead', wait(lambda: 'Import 3.0 s' in text(r), 3), text(r))
    key(r, ' ')
    stopped = wait(lambda: not r.s.run('return document.querySelector("button[aria-label=\\"Pause (Space)\\"]") !== null') and frame_time(r) >= 33_900_000, 10)
    r.check('Space plays the part and stops at its end', stopped and abs(frame_time(r) - 34_000_000) < 100_000, frame_time(r))
    time.sleep(0.3)
    r.shot('part')
    resize(r, 1024, 640)
    time.sleep(0.5)
    r.check('at the smallest window Import is in view without scrolling',
            r.s.run("""const b = [...document.querySelectorAll(arguments[0] + ' button')].find((b) => b.textContent.startsWith('Import'));
return b.getBoundingClientRect().bottom <= window.innerHeight && document.querySelector(arguments[0]).scrollHeight <= document.querySelector(arguments[0]).clientHeight""", DIALOG))
    r.shot('part-small')
    resize(r, 1440, 900)

    key(r, 'Enter')
    r.check('Enter imports: the dialog closes at once', wait(lambda: not is_open(r), 3))
    added = wait(lambda: videos(r), 30)
    r.check('the part becomes one video asset', added and len(added) == 1, r.state()['assets'])
    asset = added[0]
    part = asset.get('part') or {}
    length = part.get('endUs', 0) - part.get('startUs', 0)
    r.check('the asset keeps the 3 s part and footage around it, starting on a keyframe',
            abs(length - 3_000_000) < 40_000 and part['startUs'] >= 3_000_000 and asset['durationUs'] >= length + 5_900_000, asset)
    path = Path(asset['path'])
    r.check('the file is in Nuzky\'s own folder', path.parent == (r.work / 'data/nuzky/videos').resolve() and path.exists(), asset['path'])
    r.check('the tile shows the part\'s length', wait(lambda: r.s.run('return document.body.innerText.includes("3.0 s")'), 5))
    time.sleep(1)
    r.shot('imported')

    r.add_clip(asset['name'])
    clip = clip_of(r, asset['id'])
    r.check('a new clip takes only the part',
            clip['content']['sourceInUs'] == part['startUs'] and abs(clip['durationUs'] - length) < 40_000, clip)
    trimmed = r.s.call("""window.__nuzky.store.getState().edit({type: 'trimClip', clipId: arguments[0], sourceInUs: 0,
        startUs: arguments[1], durationUs: arguments[2]})""", clip['id'], clip['startUs'], clip['durationUs'] + part['startUs'])
    r.check('the clip\'s edge drags out over the hidden footage', trimmed['ok'] and clip_of(r, asset['id'])['content']['sourceInUs'] == 0, trimmed)

    r.s.run(CLICK, 'button[aria-label="Video from YouTube"]')
    wait(lambda: is_open(r), 5)
    paste(r, f'https://youtu.be/{SHORT}')
    r.check('a second link needs no notice and a short video shows whole in the strip',
            wait(lambda: ready(r), 30) and not r.s.run('return !!document.querySelector("button[aria-label=\\"Zoom in\\"]")'), text(r))
    type_time(r, 'Start of the part', '0:00.5')
    type_time(r, 'End of the part', '0:07.5')
    r.check('a part 0.5 s from each end imports without a word about it', wait(lambda: 'Import 7.0 s' in text(r), 3), text(r))
    wait(lambda: strip(r), 10)
    time.sleep(0.3)
    r.shot('short-part')
    button(r, 'Import')
    edge = wait(lambda: len(videos(r)) == 2 and videos(r)[1], 30)
    r.check('near the ends the hidden footage is just what there is',
            edge and edge['part'] and edge['part']['startUs'] == 500_000 and abs(edge['durationUs'] - 8_000_000) < 50_000, edge)

    r.s.run(CLICK, 'button[aria-label="Video from YouTube"]')
    wait(lambda: is_open(r), 5)
    paste(r, f'https://youtu.be/{SHORT}')
    wait(lambda: ready(r), 30)
    button(r, 'Import whole video')
    whole = wait(lambda: len(videos(r)) == 3 and videos(r)[2], 30)
    r.check('the whole video has no part', whole and not whole.get('part'), whole)

    r.s.run(CLICK, 'button[aria-label="Video from YouTube"]')
    wait(lambda: is_open(r), 5)
    paste(r, 'https://www.youtube.com/watch?v=ytPrivate01')
    r.check('a private video says so in plain words', wait(lambda: 'private' in text(r), 10), text(r))
    r.shot('error')
    r.s.run("document.querySelector(arguments[0]).dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))", FIELD)
    r.check('Esc closes it and nothing else was added', wait(lambda: not is_open(r), 3) and len(videos(r)) == 3)
    r.check('no error toasts', not r.errors(), r.errors())

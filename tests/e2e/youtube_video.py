"""Video from YouTube downloads a video, plays it in its own preview, and imports the chosen part or all of it.
All yt-dlp lookups and downloads are served by the executable fake, without contacting YouTube."""
import json, subprocess, time
from pathlib import Path

from e2e.corrections import Hands
from e2e.harness import flow, press, wait
from e2e.layout import resize

LONG = 'ytVideo0001'
SHORT = 'ytShort0001'
DIALOG = '[role=dialog][aria-labelledby="youtube-title"]'
FIELD = 'input[aria-label="Link to a YouTube video"]'
CLICK = 'document.querySelector(arguments[0]).click()'
START = '[role=slider][aria-label="Start of the part"]'
END = '[role=slider][aria-label="End of the part"]'
FRAME = 33_333


def prepare(r):
    videos = r.work / 'videos'
    videos.mkdir()
    # The short one stands, as a Short does.
    for video, seconds, size in ((LONG, 90, '640x360'), (SHORT, 8, '360x640')):
        # testsrc2 prints the time in every frame, so a frame shows where in the video it is.
        subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', f'testsrc2=size={size}:rate=30:duration={seconds}',
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


def paste(r, url, over=False):
    """Pastes into the link field, over all its text when `over`."""
    r.s.run("""const input = document.querySelector(arguments[0]); input.focus(); if (arguments[2]) input.select();
const event = new Event('paste', {bubbles: true, cancelable: true});
Object.defineProperty(event, 'clipboardData', {value: {getData: () => arguments[1]}});
input.dispatchEvent(event);""", FIELD, url, over)


def ready(r):
    return r.s.run('return !!document.querySelector(arguments[0] + " canvas")', DIALOG)


def strip(r):
    return r.s.run('return !!document.querySelector(arguments[0] + " [data-strip=ready]")', DIALOG)


def type_time(r, label, value, blur=True):
    r.s.run("""const input = document.querySelector(`input[aria-label="${arguments[0]}"]`); input.focus();
const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
set.call(input, arguments[1]); input.dispatchEvent(new Event('input', {bubbles: true})); if (arguments[2]) input.blur();""", label, value, blur)


def value(r, label):
    return r.s.run('return document.querySelector(`input[aria-label="${arguments[0]}"]`).value', label)


def in_chooser(r):
    """Focus rests in the chooser itself, where Space, I, O and Enter act, rather than on the page, the dialog, a field or a button."""
    return r.s.run("""const a = document.activeElement;
return document.querySelector(arguments[0]).contains(a) && a.tagName === 'DIV' && a.tabIndex === -1 && a.getAttribute('role') !== 'dialog'""", DIALOG)


def rect(r, selector):
    return r.s.run('const b = document.querySelector(arguments[0])?.getBoundingClientRect(); return b ? [b.left, b.right] : [-1e9, -1e9]', f'{DIALOG} {selector}')


def pointer(r, kind, selector, x):
    """A pointer event at page x, level with the element: pressing it, or moving or letting go anywhere."""
    r.s.run("""const el = document.querySelector(arguments[0] + ' ' + arguments[1]), at = el.getBoundingClientRect();
(arguments[2] === 'pointerdown' ? el : window).dispatchEvent(new PointerEvent(arguments[2],
    {bubbles: true, cancelable: true, button: 0, pointerId: 1, clientX: arguments[3], clientY: at.top + at.height / 2}));""", DIALOG, selector, kind, x)


def on_strip(fraction):
    return lambda r: (lambda film: film[0] + (film[1] - film[0]) * fraction)(rect(r, '[data-strip]'))


def drag(r, selector, at, fraction):
    """Presses the element at page x `at` and lets go over `fraction` of the strip's width, as a pointer does."""
    to = on_strip(fraction)(r)
    pointer(r, 'pointerdown', selector, at)
    pointer(r, 'pointermove', selector, to)
    pointer(r, 'pointerup', selector, to)


def end_us(r):
    return int(r.s.run('return document.querySelector(arguments[0])?.getAttribute("aria-valuenow") ?? -1', f'{DIALOG} {END}'))


def playing(r):
    return r.s.run('return !!document.querySelector(arguments[0] + " button[aria-label=\\"Pause (Space)\\"]")', DIALOG)


def zoomed(r):
    return r.s.run('return !!document.querySelector(arguments[0] + " [data-zoomed]")', DIALOG)


def picture(r):
    """How many pixels of the dialog's preview are not black, so a drawn frame is told from an empty canvas."""
    return r.s.run("""const c = document.querySelector(arguments[0] + ' canvas');
const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let lit = 0;
for (let i = 0; i < d.length; i += 4) if (d[i] + d[i + 1] + d[i + 2] > 60) lit++; return lit;""", DIALOG)


def frame_time(r):
    return int(r.s.run('return Number(document.querySelector(arguments[0] + " canvas").dataset.us ?? -1)', DIALOG))


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
    r.check('the strip shows frames of the video', wait(lambda: strip(r), 10))
    film, start, end = rect(r, '[data-strip]'), rect(r, START), rect(r, END)
    r.check('a long video shows whole in the strip, with a handle at each end',
            abs(start[1] - film[0]) < 2 and abs(end[0] - film[1]) < 2, (film, start, end))
    r.check('the chooser has focus rather than Play or the link, so Space, I, O and Enter work at once', in_chooser(r))
    r.check("the link's t=12s opens the video there", wait(lambda: abs(frame_time(r) - 12_000_000) < 50_000, 5) and '0:12.0 /' in text(r),
            (frame_time(r), text(r)))
    r.shot('downloaded')
    r.s.run("document.querySelector('[data-scrim]').dispatchEvent(new PointerEvent('pointerdown', {bubbles: true}))")
    r.check('once a video is in, a press beside the dialog does not throw it away', is_open(r))

    # Caught where the part ends, its edge follows the pointer exactly.
    drag(r, END, rect(r, END)[0], 50 / 90)
    r.check('the end handle drags to any point of the video', wait(lambda: value(r, 'End of the part').startswith('0:50'), 3), value(r, 'End of the part'))
    r.check('the dragged handle has the arrow keys, and the hints say so',
            r.s.run('return document.activeElement?.getAttribute("aria-label")') == 'End of the part' and 'Move end' in text(r), text(r))
    dragged = end_us(r)
    press('Right')
    r.check('an arrow then moves it by a frame', wait(lambda: end_us(r) - dragged == FRAME, 3), (dragged, end_us(r)))
    dragged = end_us(r)
    left, right = rect(r, END)
    pointer(r, 'pointerdown', END, right + 6)
    pointer(r, 'pointerup', END, right + 6)
    r.check('pressing a handle beside its middle leaves it where it was', end_us(r) == dragged, (dragged, end_us(r)))
    pointer(r, 'pointerdown', END, left)
    held = wait(lambda: zoomed(r), 2)
    time.sleep(0.6)
    r.shot('zoomed')
    pointer(r, 'pointermove', END, left + 30)
    time.sleep(0.3)
    moved = end_us(r) - dragged
    pointer(r, 'pointerup', END, left + 30)
    r.check('holding a handle still zooms the strip in, so the pointer moves it a frame a pixel',
            held and abs(moved - 30 * FRAME) <= 2 * FRAME, moved / FRAME)
    r.check('letting go shows the whole video again', wait(lambda: not zoomed(r), 2))
    press('space')
    # Until the preview reports it plays, its seek to the part's start reports it paused, so a second Space then plays again.
    r.check('Space plays and hands the arrows back to the playhead',
            wait(lambda: playing(r) and frame_time(r) > 300_000, 3) and in_chooser(r) and 'Move end' not in text(r), text(r))
    press('space')
    r.check('Space again pauses', wait(lambda: not playing(r), 2) and (time.sleep(0.5) or not playing(r)))

    pointer(r, 'pointerdown', '[data-strip]', on_strip(60 / 90)(r))
    pointer(r, 'pointerup', '[data-strip]', on_strip(60 / 90)(r))
    press('i')
    r.check("I past the end starts a new part there that runs to the video's end",
            wait(lambda: value(r, 'Start of the part') == '1:00.0' and value(r, 'End of the part') == '1:30.0', 3),
            (value(r, 'Start of the part'), value(r, 'End of the part')))

    type_time(r, 'Start of the part', '0:30')
    type_time(r, 'End of the part', '0:34', blur=False)
    press('Return')
    r.check('typed times choose a 4 s part and the button says so', wait(lambda: 'Import 4.0 s' in text(r), 3), text(r))
    r.check('the preview shows the frame at the typed end', wait(lambda: abs(frame_time(r) - 34_000_000) < 50_000, 5), frame_time(r))
    r.check('Enter in a time field hands the keys back to the chooser', wait(lambda: in_chooser(r), 2))
    r.s.run("window.__nuzky.api.youtubePreviewSeek(31000000)")
    wait(lambda: abs(frame_time(r) - 31_000_000) < 50_000, 5)
    press('i')
    r.check('I sets the start at the playhead', wait(lambda: 'Import 3.0 s' in text(r), 3), text(r))
    press('space')
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

    type_time(r, 'Start of the part', '0:30', blur=False)
    r.check('a click on Play takes a time still being typed and leaves the keys with the chooser',
            Hands(r).click(f'{DIALOG} button[aria-label="Play (Space)"]') and wait(lambda: in_chooser(r) and 'Import 4.0 s' in text(r), 2), text(r))
    press('Return')
    r.check('Enter imports: the dialog closes at once', wait(lambda: not is_open(r), 3))
    added = wait(lambda: videos(r), 30)
    r.check('the part becomes one video asset', added and len(added) == 1, r.state()['assets'])
    asset = added[0]
    part = asset.get('part') or {}
    length = part.get('endUs', 0) - part.get('startUs', 0)
    r.check('the asset keeps the 4 s part and footage around it, starting on a keyframe',
            abs(length - 4_000_000) < 40_000 and part['startUs'] >= 3_000_000 and asset['durationUs'] >= length + 5_900_000, asset)
    path = Path(asset['path'])
    r.check('the file is in Nuzky\'s own folder', path.parent == (r.work / 'data/nuzky/videos').resolve() and path.exists(), asset['path'])
    r.check('the tile shows the part\'s length', wait(lambda: r.s.run('return document.body.innerText.includes("4.0 s")'), 5))
    time.sleep(1)
    r.shot('imported')

    r.add_clip(asset['name'])
    clip = clip_of(r, asset['id'])
    r.check('a new clip takes only the part',
            clip['content']['sourceInUs'] == part['startUs'] and abs(clip['durationUs'] - length) < 40_000, clip)
    r.s.run('window.__nuzky.store.getState().select([arguments[0]])', clip['id'])
    detail = wait(lambda: r.s.run('return [...document.querySelectorAll("div")].filter((e) => e.children.length === 0).map((e) => e.textContent).find((t) => /· from .* of /.test(t ?? "")) ?? ""'), 5)
    r.check('the inspector counts within the part, so nothing shows there is more', detail and detail.endswith('from 0.0 s of 4.0 s'), detail)
    trimmed = r.s.call("""window.__nuzky.store.getState().edit({type: 'trimClip', clipId: arguments[0], sourceInUs: 0,
        startUs: arguments[1], durationUs: arguments[2]})""", clip['id'], clip['startUs'], clip['durationUs'] + part['startUs'])
    r.check('the clip\'s edge drags out over the hidden footage', trimmed['ok'] and clip_of(r, asset['id'])['content']['sourceInUs'] == 0, trimmed)

    r.s.run(CLICK, 'button[aria-label="Video from YouTube"]')
    wait(lambda: is_open(r), 5)
    paste(r, f'https://youtu.be/{SHORT}')
    r.check('a second link needs no notice', wait(lambda: ready(r), 30), text(r))
    wait(lambda: strip(r), 10)
    frames = r.s.run("""return [...document.querySelector(arguments[0] + ' [data-strip]').children]
.filter((d) => d.style.backgroundImage).map((d) => [d.getBoundingClientRect().width, parseFloat(d.style.backgroundPosition)])""", DIALOG)
    r.check('a standing Short fills the strip with its own narrow frames, one each half second, in order',
            len(frames) == 23 and all(25 < w < 34 for w, _ in frames) and len({x for _, x in frames}) == 16
            and all(a[1] >= b[1] for a, b in zip(frames, frames[1:])), frames)
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
    paste(r, f'https://youtu.be/{SHORT}', over=True)
    r.check('a link pasted over the whole field is looked up at once', wait(lambda: ready(r), 30), text(r))
    r.s.run("document.querySelector(arguments[0]).dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))", FIELD)
    r.check('Esc closes it and nothing else was added', wait(lambda: not is_open(r), 3) and len(videos(r)) == 3)
    r.check('no error toasts', not r.errors(), r.errors())

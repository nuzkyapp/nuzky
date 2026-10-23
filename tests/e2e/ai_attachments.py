"""Pictures and videos the user attaches to an AI panel message, with + or by pasting, as each agent gets them."""
import base64, json, wave
from pathlib import Path


from e2e.ai_panel import CHIPS, agent, click, idle, panel, runs, send, setup, setup_chatgpt, setup_codex
from e2e.harness import FIXTURES, flow, wait
from e2e.setup_ai import ANSWER_FILE_DIALOG

# A picture of `w` × `h` pasted into the field as a screenshot is: a file on the clipboard, without a path.
PASTE = """const [w, h, name] = arguments;
const c = document.createElement('canvas'); c.width = w; c.height = h;
const g = c.getContext('2d'); g.fillStyle = '#d02020'; g.fillRect(0, 0, w / 2, h); g.fillStyle = '#2040d0'; g.fillRect(w / 2, 0, w / 2, h);
const blob = await new Promise((done) => c.toBlob(done, 'image/png'));
const data = new DataTransfer(); data.items.add(new File([blob], name, {type: 'image/png'}));
const t = document.querySelector('aside[aria-label=AI] textarea'); t.focus();
return t.dispatchEvent(new ClipboardEvent('paste', {clipboardData: data, bubbles: true, cancelable: true}));"""
SENT = "return [...document.querySelectorAll('aside[aria-label=AI] [role=img]')].map((e) => e.getAttribute('aria-label'))"
PREVIEWS = "return document.querySelectorAll('aside[aria-label=AI] span > img[src^=\"data:image/png\"]').length"
TOASTS = "return window.__nuzky.store.getState().toasts.map((t) => t.text)"


def paste(r, w, h, name):
    return r.s.call(f'(async () => {{ {PASTE} }})()', w, h, name)


def open_panel(r):
    r.s.run('document.body.focus()')
    r.key('j', ctrlKey=True)
    wait(lambda: panel(r), 5)


@flow('ai_attachments', 'Pictures and videos go with an AI panel message: + picks files and Cmd+V pastes a screenshot, '
      'chips show them with ×, Claude gets each picture turned the way Nuzky shows it and no larger than models take, '
      'a video as its path to import, also in a message sent while it works, and the chat keeps them', before=setup)
def ai_attachments(r):
    from PIL import Image

    # Landscape pixels that EXIF turns upright, as a phone saves a portrait photo.
    photo = r.work / 'portrait-photo.jpg'
    exif = Image.Exif()
    exif[0x0112] = 6
    Image.new('RGB', (400, 200), (200, 40, 40)).save(photo, exif=exif.tobytes())
    sound = r.work / 'beep.wav'
    with wave.open(str(sound), 'wb') as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(48000)
        w.writeframes(b'\0\0' * 4800)
    video = FIXTURES / 'talk.mp4'
    open_panel(r)

    r.s.run(ANSWER_FILE_DIALOG, [str(photo), str(video), str(sound)])
    r.check('+ attaches the picture and the video picked, each as a chip with its own preview',
            click(r, 'Attach pictures or videos. You can also paste them into the field.') == 'false'
            and wait(lambda: r.s.run(CHIPS)[:2] == ["Don't send portrait-photo.jpg", "Don't send talk.mp4"], 15)
            and wait(lambda: r.s.run(PREVIEWS) == 2, 5), [r.s.run(CHIPS), r.s.run(PREVIEWS)])
    r.check('a file with only sound is not attached and says why',
            wait(lambda: any('beep.wav has no picture' in t for t in r.s.run(TOASTS)), 5), r.s.run(TOASTS))

    pasted = paste(r, 3000, 1500, 'Screenshot.png')
    r.check('a pasted screenshot becomes a chip too, without pasting any text',
            pasted.get('ok') and wait(lambda: r.s.run(CHIPS)[2:3] == ["Don't send Screenshot.png"], 15)
            and agent(r)['draft'] == '', [pasted, r.s.run(CHIPS), agent(r)])
    kept = sorted((r.work / 'data/nuzky/attachments').glob('*.png'))
    r.check("Nuzky keeps the pasted picture in its own data, as the clipboard has no file", len(kept) == 1, kept)
    paste(r, 64, 64, 'Extra.png')
    wait(lambda: "Don't send Extra.png" in r.s.run(CHIPS), 15)
    r.check('× takes an attachment off the message',
            click(r, "Don't send Extra.png") == 'false'
            and wait(lambda: r.s.run(CHIPS) == ["Don't send portrait-photo.jpg", "Don't send talk.mp4", "Don't send Screenshot.png",
                                                "Don't send the playhead"], 5), r.s.run(CHIPS))
    r.shot('attached')

    send(r, 'What is in what I attached?')
    idle(r)
    run = runs(r)[-1]
    r.check('Claude gets both pictures as JPEGs, the photo upright and the screenshot no wider than 1568 pixels',
            run['pictures'] == [[200, 400], [1568, 784]], run['pictures'])
    r.check('the message names each attachment, the video by its path to import with import_media',
            f'[Nuzky] The user attached the video {video}. To see or use it, add it to the project with import_media.' in run['prompt']
            and f'[Nuzky] The user attached the picture {photo}; you see it with this message. To put it in the video or on a cover, add it to the project with import_media.' in run['prompt']
            and f'/nuzky/attachments/{kept[0].name}; you see it' in run['prompt'], run['prompt'])
    text = panel(r)['text']
    r.check("the agent's answer comes back", 'I see 2 pictures (200×400, 1568×784) and 1 videos.' in text, text)
    r.check('the sent message shows what went with it, and the field is empty of attachments again',
            r.s.run(SENT) == ['Picture portrait-photo.jpg', 'Video talk.mp4', 'Picture Screenshot.png']
            and r.s.run(CHIPS) == ["Don't send the playhead"], [r.s.run(SENT), r.s.run(CHIPS)])
    r.shot('sent')
    saved = wait(lambda: [json.loads(f.read_text()) for f in (r.work / 'data/nuzky/chats').glob('*/*.json')], 5)
    attached = [it.get('attachments') for it in saved[0]['items'] if it['kind'] == 'user'] if saved else None
    r.check('the saved chat keeps the attachments with their previews, so reopening it shows them',
            attached and [a['name'] for a in attached[0]] == ['portrait-photo.jpg', 'talk.mp4', 'Screenshot.png']
            and all(a['preview'].startswith('data:image/png;base64,') for a in attached[0]), attached)

    # A picture sent while Claude works reaches it in the same turn.
    send(r, 'steer me')
    wait(lambda: 'Pracuju' in panel(r)['text'] or 'Splitting' in panel(r)['text'] or len(r.track()) > 1, 15)
    paste(r, 800, 600, 'Steer.png')
    wait(lambda: "Don't send Steer.png" in r.s.run(CHIPS), 15)
    send(r, 'split here too')
    steered = r.work / 'fake/steered.jsonl'
    got = wait(lambda: steered.exists() and [json.loads(line) for line in steered.read_text().splitlines()], 15) or []
    r.check('a picture sent while Claude works goes with that message', [m['pictures'] for m in got] == [[[800, 600]]], got)
    idle(r)

    # A draft's attachment stays with the chat it was added in, as its text does.
    click(r, 'New chat')
    send(r, 'Hello there')
    idle(r)
    paste(r, 64, 64, 'ForThisChat.png')
    wait(lambda: "Don't send ForThisChat.png" in r.s.run(CHIPS), 15)
    past = 'ul[aria-label="Past chats"]'
    click(r, 'Past chats in this project')
    wait(lambda: r.s.run(f"return !!document.querySelector('{past}')"), 5)
    click(r, 'What is in what I attached?', past)
    r.check('opening another chat leaves the attachment with the draft it was added to',
            wait(lambda: 'I see 2 pictures' in panel(r)['text'], 10) and "Don't send ForThisChat.png" not in r.s.run(CHIPS), r.s.run(CHIPS))
    click(r, 'Past chats in this project')
    wait(lambda: r.s.run(f"return !!document.querySelector('{past}')"), 5)
    click(r, 'Hello there', past)
    r.check('and going back brings it back', wait(lambda: "Don't send ForThisChat.png" in r.s.run(CHIPS), 10), r.s.run(CHIPS))


@flow('ai_attachments_codex', 'Codex gets a pasted picture as an --image file Nuzky prepared', before=setup_codex)
def ai_attachments_codex(r):
    open_panel(r)
    paste(r, 2000, 1000, 'Screenshot.png')
    wait(lambda: "Don't send Screenshot.png" in r.s.run(CHIPS), 15)
    send(r, 'Look at this')
    idle(r)
    run = runs(r)[-1]
    images = [a.removeprefix('--image=') for a in run['argv'] if a.startswith('--image=')]
    r.check('Codex gets one --image, a JPEG no wider than 1568 pixels in the cache',
            run['pictures'] == [[1568, 784]] and len(images) == 1
            and Path(images[0]).resolve().parent == (r.work / 'cache/nuzky/attachments').resolve(), run)
    r.check("Codex's answer comes back", 'I see 1 pictures (1568×784) and 0 videos.' in panel(r)['text'], panel(r)['text'])


@flow('ai_attachments_chatgpt', 'ChatGPT gets a pasted picture as an input_image beside the message', before=setup_chatgpt)
def ai_attachments_chatgpt(r):
    open_panel(r)
    wait(lambda: 'GPT E2E Sol' in panel(r)['text'], 15)
    paste(r, 640, 360, 'Screenshot.png')
    wait(lambda: "Don't send Screenshot.png" in r.s.run(CHIPS), 15)
    send(r, 'Split it where this frame is')
    idle(r)
    sent = r.chatgpt.requests[0]['body']['input'] if r.chatgpt.requests else []
    user = next((item for item in sent if item.get('role') == 'user'), {})
    images = [part['image_url'] for part in user.get('content', []) if part['type'] == 'input_image']
    r.check('the first request carries the picture as a JPEG data URL after the text',
            user.get('content', [{}])[0].get('type') == 'input_text' and len(images) == 1
            and images[0].startswith('data:image/jpeg;base64,')
            and base64.b64decode(images[0].split(',', 1)[1])[:2] == b'\xff\xd8', [r.chatgpt.errors, user])
    r.check('ChatGPT answers and splits the clip', wait(lambda: len(r.track()) == 2, 10), r.track())

from e2e.harness import FIXTURES, flow, wait

TIME = 'return window.__nuzky.store.getState().timeUs'
FPS = 'return window.__nuzky.store.getState().snap.project.canvas.fps'
ADD_CAPTION = """window.__nuzky.store.getState().edit({type: 'addText', startUs: arguments[0], text: 'Caption',
  style: {fontSize: 64, color: '#ffffff', bold: false, strokeWidth: 0, strokeColor: '#000000'}})"""
CALM = 'window.__nuzky.store.getState().select([]); document.activeElement?.blur()'
# Clicks the ruler `arguments[1]` px right of where the clip `arguments[0]` starts on screen.
SCRUB = """const ruler = document.querySelector('[data-ruler]').getBoundingClientRect();
const clip = document.querySelector(`[data-clip-id='${arguments[0]}']`).getBoundingClientRect();
const at = (type) => new PointerEvent(type, {bubbles: true, button: 0, pointerId: 1,
  clientX: clip.left - 1 + arguments[1], clientY: ruler.top + ruler.height / 2});
document.querySelector('[data-ruler]').dispatchEvent(at('pointerdown'));
window.dispatchEvent(at('pointerup'));"""


@flow('playhead', 'The playhead sits on frames: arrows step onto the next frame, scrubbing lands on a frame or snaps to a '
      'caption edge, ↑/↓ jump to clip and caption edges, and Q there cuts the video exactly where the caption starts')
def playhead(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    fps = r.s.run(FPS)
    frames = [round(i * 1_000_000 / fps) for i in range(10 * fps)]
    # A caption starts where its word was heard, between two frames.
    caption_at = 1_015_000
    r.check('the caption start is not on a frame', caption_at not in frames, caption_at)
    added = r.s.call(ADD_CAPTION, caption_at)
    caption = wait(lambda: next((t['clips'][0] for t in r.state()['tracks'] if t['kind'] == 'text' and t['clips']), None))
    r.check('a caption is added between two frames', added['ok'] and caption and caption['startUs'] == caption_at, caption)
    r.s.run(CALM)
    time = lambda: r.s.run(TIME)
    # The engine echoes every seek back a moment later, so a check waits for the time to settle on what it expects.
    settles = lambda ok: wait(lambda: ok(time()), 5)

    between = 512_345
    r.seek(between)
    r.key('ArrowRight')
    after = min(f for f in frames if f > between)
    r.check('→ from between two frames lands on the next frame', settles(lambda t: t == after), (time(), after))
    r.key('ArrowLeft')
    r.check('← steps back one frame', settles(lambda t: t == frames[frames.index(after) - 1]), time())

    r.s.run(SCRUB, caption['id'], 3)
    r.check('clicking the ruler 3 px from where the caption starts snaps to it', settles(lambda t: t == caption_at), time())
    r.s.run(SCRUB, caption['id'], 40)
    r.check('clicking the ruler away from any edge lands on a frame', settles(lambda t: t in frames and t > caption_at), time())

    r.key('ArrowUp')
    r.check('↑ jumps back to where the caption starts', settles(lambda t: t == caption_at), time())
    r.key('ArrowUp')
    r.check('↑ again jumps to the start of the video', settles(lambda t: t == 0), time())
    r.key('ArrowDown')
    r.check('↓ jumps to where the caption starts', settles(lambda t: t == caption_at), time())

    video = r.track()[0]
    r.key('q')
    cut = wait(lambda: (c := r.track()[0]) and c['durationUs'] != video['durationUs'] and c)
    moved = next(t['clips'][0] for t in r.state()['tracks'] if t['kind'] == 'text')
    r.check('Q deletes the video exactly up to where the caption starts',
            cut and cut['durationUs'] == video['durationUs'] - caption_at and moved['startUs'] == cut['startUs'] == 0, (cut, moved))
    r.check('after Q the playhead sits on the cut', settles(lambda t: t == 0), time())
    r.shot('cut-at-caption')

    r.seek(10_000)
    r.focus_clip(cut['id'])
    r.key('q')
    said = wait(lambda: [t['text'] for t in r.state()['toasts'] if 'edge' in t['text']], 5)
    r.check('Q less than a frame from the edge says to move a frame away',
            said == ["Move the playhead at least a frame away from the clip's edge to delete one side of it."], r.state()['toasts'])
    r.check('and changes nothing', r.track()[0] == cut, r.track())

"""Media dragged from the panel onto a track beside the main one snaps to clip edges like a moved clip: dropped a few
pixels after a picture in picture, the new clip starts exactly where the picture ends, and the drop line shows it
there before the release. With snapping off it lands where the pointer is."""
import time as clock

from PIL import Image

from e2e.harness import FIXTURES, flow, wait

ZOOM = 60
PIP_US = 3_000_000
# Presses the middle of a media tile and drags it to `arguments[1]`, `arguments[2]`, without letting go.
DRAG_TILE = """const tile = document.querySelector(`[aria-label='Add ${arguments[0]} at playhead']`).closest('.cursor-grab');
const b = tile.getBoundingClientRect();
const at = (type, x, y) => new PointerEvent(type, {bubbles: true, button: 0, pointerId: 1, clientX: x, clientY: y});
const [x, y] = [b.left + b.width / 2, b.top + b.height / 3];
document.elementFromPoint(x, y).dispatchEvent(at('pointerdown', x, y));
window.dispatchEvent(at('pointermove', x + 20, y));
window.dispatchEvent(at('pointermove', arguments[1], arguments[2]));"""
RELEASE = """window.dispatchEvent(new PointerEvent('pointerup', {bubbles: true, button: 0, pointerId: 1,
  clientX: arguments[0], clientY: arguments[1]}))"""
RECT = "const b = document.querySelector(`[data-clip-id='${arguments[0]}']`).getBoundingClientRect(); return [b.left, b.top, b.right, b.bottom];"
LINE = "return document.querySelector('[data-testid=drop-line]')?.getBoundingClientRect().left ?? null"
PIP = """(() => { const st = window.__nuzky.store.getState();
  const asset = st.snap.project.assets.find((a) => a.name === arguments[0]);
  return st.edit({type: 'addPictureInPicture', assetId: asset.id, startUs: 0, durationUs: arguments[1]}); })()"""


@flow('drop', 'Media dropped a few pixels after a picture in picture snaps to its end, shown by the drop line, and '
      'lands under the pointer with snapping off')
def drop_flow(r):
    Image.new('RGB', (1280, 720), (230, 30, 30)).save(r.work / 'red.png')
    r.import_media(FIXTURES / 'talk.mp4', r.work / 'red.png')
    r.add_clip('talk.mp4')
    r.s.call(PIP, 'red.png', PIP_US)
    overlay = lambda: next((t for t in r.state()['tracks'][1:] if t['kind'] == 'video'), {'clips': []})
    r.check('the picture in picture sits on a track over the main one', wait(lambda: len(overlay()['clips']) == 1, 10))
    pip = overlay()['clips'][0]['id']
    r.s.run('const st = window.__nuzky.store.getState(); st.select([]); st.seek(0); st.setZoom(arguments[0])', ZOOM)
    clock.sleep(0.5)

    def drop(name):
        """Drags red.png to 5 px after the picture's end on its track; where the line showed, that end, and where the clip went."""
        _, top, right, bottom = r.s.run(RECT, pip)
        x, y = right + 5, (top + bottom) / 2
        before = {c['id'] for c in overlay()['clips']}
        r.s.run(DRAG_TILE, 'red.png', x, y)
        line = wait(lambda: r.s.run(LINE), 5)
        clock.sleep(0.5)
        r.shot(name)
        r.s.run(RELEASE, x, y)
        wait(lambda: len(overlay()['clips']) > len(before), 10)
        added = [c for c in overlay()['clips'] if c['id'] not in before]
        return line, right, added[0]['startUs'] if added else None

    line, end, start = drop('snapped')
    r.check('while dragged, the drop line sits on the picture\'s end', line is not None and abs(line - end) < 2, [line, end])
    r.check('dropped, the new clip starts exactly where the picture ends', start == PIP_US, start)

    r.s.run("document.querySelector('[aria-label=\"Snapping on\"]').click()")
    r.check('snapping turns off', wait(lambda: r.s.run("return !!document.querySelector('[aria-label=\"Snapping off\"]')"), 5))
    r.s.call('window.__nuzky.store.getState().undo()')
    wait(lambda: len(overlay()['clips']) == 1, 10)
    line, end, start = drop('free')
    r.check('with snapping off the line follows the pointer', line is not None and abs(line - end - 5) < 2, [line, end])
    r.check('and the clip lands under the pointer', start is not None and abs(start - (PIP_US + 5 / ZOOM * 1e6)) < 20_000, start)
    r.check('no error toast', not r.errors(), r.errors())


@flow('insert', 'Media dropped on the cut between two clips of the main track goes in there and pushes the captions and '
      'titles after it along, so they stay over their picture; the captions before it stay, one undo takes it back, and a '
      'text duplicated with the clip it ends on stays right after itself')
def insert_flow(r):
    Image.new('RGB', (1280, 720), (230, 30, 30)).save(r.work / 'red.png')
    r.import_media(FIXTURES / 'talk.mp4', FIXTURES / 'wide.mp4', r.work / 'red.png')
    r.add_clip('talk.mp4')
    r.add_clip('wide.mp4')
    talk, wide = r.track()
    segment = lambda clip, text: {'startUs': clip['startUs'] + 200_000, 'endUs': clip['startUs'] + 900_000, 'text': text, 'words': []}
    r.s.call("window.__nuzky.store.getState().edit({type: 'addCaptions', segments: arguments[0],"
             " style: {fontSize: 60, color: '#ffffff'}})", [segment(talk, 'Talk'), segment(wide, 'Wide')])
    r.s.call("window.__nuzky.store.getState().edit({type: 'addText', startUs: arguments[0], text: 'Title',"
             " style: {fontSize: 80, color: '#ffffff'}})", wide['startUs'] + 500_000)
    texts = lambda: {c['text']: c['startUs'] for t in r.state()['tracks'] if t['kind'] == 'text' for c in t['clips']}
    r.check('a caption over each clip and a title over the second',
            wait(lambda: texts() == {'Talk': talk['startUs'] + 200_000, 'Wide': wide['startUs'] + 200_000,
                                     'Title': wide['startUs'] + 500_000}, 5), texts())
    before = texts()
    r.s.run('const st = window.__nuzky.store.getState(); st.select([]); st.seek(0); st.setZoom(arguments[0])', ZOOM)
    clock.sleep(0.5)

    _, top, right, bottom = r.s.run(RECT, talk['id'])
    r.s.run(DRAG_TILE, 'red.png', right, (top + bottom) / 2)
    clock.sleep(0.5)
    r.shot('over-cut')
    r.s.run(RELEASE, right, (top + bottom) / 2)
    wait(lambda: len(r.track()) == 3, 10)
    main = r.track()
    names = {a['id']: a['name'] for a in r.state()['assets']}
    r.check('the picture goes in between the two clips', [names[c['assetId']] for c in main] == ['talk.mp4', 'red.png', 'wide.mp4'],
            main)
    pushed = main[1]['durationUs']
    r.check('the caption and the title over the second clip move with it, the caption over the first stays',
            texts() == {'Talk': before['Talk'], 'Wide': main[2]['startUs'] + 200_000, 'Title': main[2]['startUs'] + 500_000}
            and main[2]['startUs'] == wide['startUs'] + pushed, (texts(), main))
    r.shot('inserted')
    r.s.call('window.__nuzky.store.getState().undo()')
    r.check('one undo takes the picture out and puts the texts back', wait(lambda: len(r.track()) == 2 and texts() == before, 10), texts())

    # Duplicating a text that ends where the first clip ends, selected before that clip: the copy of the text goes right
    # after it, over the copy of the clip, and the clip's copy does not push it on past itself.
    r.s.call("window.__nuzky.store.getState().edit({type: 'addText', startUs: arguments[0], text: 'Outro',"
             " style: {fontSize: 80, color: '#ffffff'}})", talk['durationUs'] - 3_000_000)
    outro = wait(lambda: next((c['id'] for t in r.state()['tracks'] for c in t['clips'] if c['text'] == 'Outro'), None), 5)
    r.s.run('window.__nuzky.store.getState().select(arguments[0])', [outro, talk['id']])
    r.key('d', ctrlKey=True)
    wait(lambda: len(r.track()) == 3, 10)
    outros = lambda: sorted(c['startUs'] for t in r.state()['tracks'] for c in t['clips'] if c['text'] == 'Outro')
    r.check('a text duplicated with the clip it ends on stays right after itself, over the copy of the clip',
            wait(lambda: outros() == [talk['durationUs'] - 3_000_000, talk['durationUs']], 5)
            and r.track()[1]['startUs'] == talk['durationUs'], (outros(), r.track()))
    r.check('no error toast', not r.errors(), r.errors())


CAPTION_END_US = 4_500_000
# Presses the middle of a clip and drags it `arguments[1]` px to the right, without letting go.
DRAG_RIGHT = """const el = document.querySelector(`[data-clip-id='${arguments[0]}']`);
const b = el.getBoundingClientRect();
const at = (type, dx) => new PointerEvent(type, {bubbles: true, button: 0, pointerId: 1,
  clientX: b.left + b.width / 2 + dx, clientY: b.top + b.height / 2});
el.dispatchEvent(at('pointerdown', 0));
window.dispatchEvent(at('pointermove', 10));
window.dispatchEvent(at('pointermove', arguments[1]));
return [b.left + b.width / 2 + arguments[1], b.top + b.height / 2];"""


@flow('snap-captions', 'Media dropped and a clip moved a few pixels after the end of a caption stay under the pointer instead '
      'of snapping to the caption')
def snap_captions_flow(r):
    Image.new('RGB', (1280, 720), (230, 30, 30)).save(r.work / 'red.png')
    r.import_media(FIXTURES / 'talk.mp4', r.work / 'red.png')
    r.add_clip('talk.mp4')
    r.s.call(PIP, 'red.png', PIP_US)
    r.s.call("window.__nuzky.store.getState().edit({type: 'addCaptions', segments: [{startUs: 4000000, endUs: arguments[0],"
             " text: 'Hello', words: []}], style: {fontSize: 60, color: '#ffffff'}})", CAPTION_END_US)
    overlay = lambda: next((t for t in r.state()['tracks'][1:] if t['kind'] == 'video'), {'clips': []})
    captions = lambda: [c for t in r.state()['tracks'] if t['kind'] == 'text' for c in t['clips']]
    r.check('a picture in picture and a caption sit over the video',
            wait(lambda: len(overlay()['clips']) == 1 and len(captions()) == 1, 10), r.state()['tracks'])
    pip = overlay()['clips'][0]['id']
    r.s.run('const st = window.__nuzky.store.getState(); st.select([]); st.seek(0); st.setZoom(arguments[0])', ZOOM)
    clock.sleep(0.5)
    free_us = CAPTION_END_US + 5 / ZOOM * 1e6

    left, top, _, bottom = r.s.run(RECT, pip)
    x = left + free_us / 1e6 * ZOOM
    r.s.run(DRAG_TILE, 'red.png', x, (top + bottom) / 2)
    line = wait(lambda: r.s.run(LINE), 5)
    clock.sleep(0.5)
    r.shot('drop-after-caption')
    r.s.run(RELEASE, x, (top + bottom) / 2)
    r.check('while dragged 5 px after the caption\'s end, the drop line stays under the pointer', line is not None and abs(line - x) < 2, [line, x])
    wait(lambda: len(overlay()['clips']) == 2, 10)
    added = [c['startUs'] for c in overlay()['clips'] if c['id'] != pip]
    r.check('dropped, the new clip starts under the pointer, not on the caption\'s end', added and abs(added[0] - free_us) < 20_000, added)
    r.s.call('window.__nuzky.store.getState().undo()')
    wait(lambda: len(overlay()['clips']) == 1, 10)

    x, y = r.s.run(DRAG_RIGHT, pip, free_us / 1e6 * ZOOM)
    clock.sleep(0.5)
    r.shot('move-after-caption')
    r.s.run(RELEASE, x, y)
    moved = wait(lambda: (s := overlay()['clips'][0]['startUs']) > 0 and s, 10)
    r.check('a clip moved to 5 px after the caption\'s end starts there, not on the caption\'s end', moved and abs(moved - free_us) < 20_000, moved)
    r.check('no error toast', not r.errors(), r.errors())

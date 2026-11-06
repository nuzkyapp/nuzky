import time

from e2e.harness import FIXTURES, changed_share, flow, link_models, preview_crop, preview_rect, preview_redraw, wait


@flow('captions', 'Whisper on this computer captions the spoken words of a video', before=link_models)
def captions(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    r.s.run("window.__nuzky.speech.setState({model: 'small', language: 'en'})")
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === 'Captions').click()")
    time.sleep(0.5)
    r.s.run("[...document.querySelectorAll('button')].find((b) => b.textContent.includes('Generate captions')).click()")
    job = wait(lambda: next((j for j in r.state()['jobs'] if j['kind'] == 'captions' and j['status'] != 'running'), None), 600, 1)
    r.check('captions finish', job and job['status'] == 'done', job)
    track = next(t for t in r.state()['tracks'] if t['kind'] == 'text')
    texts = [c['text'] for c in track['clips']]
    heard = ' '.join(texts).lower()
    spoken = ['channel', 'video', 'laptop', 'clips', 'pauses', 'captions']
    found = [w for w in spoken if w in heard]
    r.check('captions contain the spoken words', len(found) >= 4, {'found': found, 'captions': texts[:8]})
    # Hiding and showing the captions track redraws the same moment, so the two pictures differ only by the caption.
    # Each is taken once the preview has redrawn, so a picture left from before the seek cannot pass for either.
    caption = max(track['clips'], key=lambda c: c['durationUs'])
    r.seek(caption['startUs'] + caption['durationUs'] // 2)
    time.sleep(1.5)
    r.shot('captions-seek')
    before = preview_crop(r.work / 'captions-seek.png', preview_rect(r))
    hide = "window.__nuzky.store.getState().edit({type: 'updateTrack', trackId: arguments[0], hidden: arguments[1]})"
    r.s.call(hide, track['id'], True)
    hidden, redrawn_hidden = preview_redraw(r, 'captions-hidden', before)
    r.s.call(hide, track['id'], False)
    shown, redrawn_shown = preview_redraw(r, 'captions', hidden)
    r.check('the preview redraws after hiding and after showing the captions', redrawn_hidden and redrawn_shown,
            {'after hiding': redrawn_hidden, 'after showing': redrawn_shown})
    changed = changed_share(hidden, shown)
    r.check(f'the caption "{caption["text"]}" shows in the preview', changed > 0.005, f'{changed:.2%} of the preview changes')
    r.check('no error toast', not r.errors(), r.errors())


@flow('captions_some_clips', 'Generate makes captions only over the selected clips and keeps the others', before=link_models)
def captions_some_clips(r):
    r.import_media(FIXTURES / 'talk.mp4')
    r.add_clip('talk.mp4')
    first = r.track()[0]
    half = first['durationUs'] // 2
    r.s.call("window.__nuzky.store.getState().edit({type: 'splitClip', clipId: arguments[0], atUs: arguments[1]})", first['id'], half)
    wait(lambda: len(r.track()) == 2)
    second = r.track()[1]
    r.s.run("window.__nuzky.speech.setState({model: 'small', language: 'en'})")
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === 'Captions').click()")
    time.sleep(0.5)
    generate = "document.querySelector('[data-generate-captions]')"

    def finished(count):
        done = lambda: [j for j in r.state()['jobs'] if j['kind'] == 'captions' and j['status'] != 'running']
        wait(lambda: len(done()) >= count, 600, 1)
        return done()[-1] if done() else None

    def captions():
        return next(t for t in r.state()['tracks'] if t['name'] == 'Captions')['clips']

    # Splitting selects the second half; with nothing selected Generate captions the whole video.
    r.s.run('window.__nuzky.store.getState().select([])')
    time.sleep(0.5)
    r.check('with nothing selected, Generate is for the whole video', r.s.run(f'return {generate}.textContent').strip() == 'Generate captions')
    r.s.run(f'{generate}.click()')
    job = finished(1)
    r.check('captions for the whole video finish', job and job['status'] == 'done', job)
    before = [(c['id'], c['text']) for c in captions() if c['startUs'] < half]
    over = [c for c in captions() if c['startUs'] >= half]
    r.check('the whole video has captions over both clips', before and over, {'first': len(before), 'second': len(over)})

    r.s.call('window.__nuzky.store.getState().select([arguments[0]])', second['id'])
    time.sleep(0.5)
    label = r.s.run(f'return {generate}.textContent')
    scope = r.s.run("return document.querySelector('[role=group][aria-label=\"Make captions for\"] [aria-pressed=true]')?.textContent")
    r.check('with a clip selected, Generate is for that clip', scope == '1 clip selected' and label.endswith('for 1 clip'), {'scope': scope, 'button': label})
    r.shot('captions-some-clips')
    r.s.run(f'{generate}.click()')
    job = finished(2)
    r.check('captions for the selected clip finish', job and job['status'] == 'done', job)
    after = captions()
    r.check('the other clip keeps its captions as they were', [(c['id'], c['text']) for c in after if c['startUs'] < half] == before,
            {'before': before, 'after': [(c['id'], c['text']) for c in after if c['startUs'] < half]})
    remade = [c for c in after if c['startUs'] >= half]
    r.check('the selected clip has new captions in its place', remade and not {c['id'] for c in remade} & {c['id'] for c in over},
            {'old': [c['id'] for c in over], 'new': [c['id'] for c in remade]})
    toasts = [t['text'] for t in r.state()['toasts']]
    r.check('the toast names the clip', any(t.startswith('Added') and t.endswith('to 1 clip') for t in toasts), toasts)
    r.check('no error toast', not r.errors(), r.errors())

import subprocess, time

from e2e.corrections import Hands
from e2e.harness import FIXTURES, changed_share, ffprobe, flow, link_models, preview_crop, preview_rect, preview_redraw, wait


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
    # Whole video holds while the selection stays, also over a visit to another tab, and only until it changes.
    tab = "[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === arguments[0]).click()"
    r.s.run("document.querySelector('[data-seg=whole]').click()")
    r.s.run(tab, 'Text')
    r.s.run(tab, 'Captions')
    r.s.call("window.__nuzky.store.getState().edit({type: 'updateClip', clipId: arguments[0], volume: 0.9})", second['id'])
    time.sleep(0.5)
    r.check('Whole video stays picked after another tab and an edit', r.s.run(f'return {generate}.textContent').strip() == 'Regenerate captions')
    r.s.call('window.__nuzky.store.getState().select([arguments[0]])', first['id'])
    r.s.call('window.__nuzky.store.getState().select([arguments[0]])', second['id'])
    time.sleep(0.5)
    r.check('a new selection makes captions for the selected clip again', r.s.run(f'return {generate}.textContent').endswith('for 1 clip'))
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


@flow('captions_voice_over', 'A voice-over recorded apart from a silent video gets captions, and deleting one of its words in the '
      'transcript cuts it out of the voice-over and the video alike', before=link_models)
def captions_voice_over(r):
    speech = FIXTURES / 'speech.wav'
    if not speech.exists():
        raise RuntimeError('tmp-test/speech.wav is missing: run scripts/fixtures.sh')
    seconds, _ = ffprobe(str(speech))
    video, voice = r.work / 'silent.mp4', r.work / 'voice-over.m4a'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', f'testsrc2=s=1080x1920:r=30:d={seconds:.3f}', '-c:v', 'libx264',
                    '-preset', 'veryfast', '-pix_fmt', 'yuv420p', str(video)], check=True)
    # As a phone's voice memo: AAC in .m4a, nothing but sound.
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-i', str(speech), '-c:a', 'aac', '-b:a', '128k', str(voice)], check=True)
    r.import_media(video, voice)
    r.add_clip('silent.mp4')
    asset = next(a['id'] for a in r.state()['assets'] if a['name'] == 'voice-over.m4a')
    r.s.call("window.__nuzky.store.getState().edit({type: 'addClip', assetId: arguments[0], startUs: 0, trackId: null})", asset)
    voice_track = lambda: next((t for t in r.state()['tracks'] if t['kind'] == 'audio' and t['clips']), None)
    track = wait(voice_track, 10)
    if not track:
        raise RuntimeError('the voice-over was not placed')
    kept = r.s.run("return window.__nuzky.store.getState().snap.project.tracks.find((t) => t.id === arguments[0]).keepInPlace", track['id'])
    r.check('the voice-over lies on an audio track kept in place, as sound files start', kept is True, kept)
    r.s.run("window.__nuzky.speech.setState({model: 'small', language: 'en'})")
    tab = "[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === arguments[0]).click()"
    r.s.run(tab, 'Captions')
    time.sleep(0.5)
    generate = "document.querySelector('[data-generate-captions]')"
    r.s.call('window.__nuzky.store.getState().select([arguments[0]])', track['clips'][0]['id'])
    time.sleep(0.5)
    label = r.s.run(f'return {generate}.textContent')
    r.check('with the voice-over selected, Generate is for that clip', label.endswith('for 1 clip'), label)
    r.s.run('window.__nuzky.store.getState().select([])')
    time.sleep(0.5)
    button = r.s.run(f"const b = {generate}; return {{disabled: b.getAttribute('aria-disabled') === 'true', text: b.textContent, why: b.title}}")
    r.shot('captions-voice-over-generate')
    if not r.check('Generate captions is open to a silent video with a voice-over', not button['disabled'], button):
        return
    r.s.run(f'{generate}.click()')
    job = wait(lambda: next((j for j in r.state()['jobs'] if j['kind'] == 'captions' and j['status'] != 'running'), None), 600, 1)
    r.check('captions finish', job and job['status'] == 'done', job)
    captions = next((t['clips'] for t in r.state()['tracks'] if t['name'] == 'Captions'), [])
    texts = [c['text'] for c in captions]
    found = [w for w in ['channel', 'video', 'laptop', 'clips', 'pauses', 'captions'] if w in ' '.join(texts).lower()]
    r.check('the captions say what the voice-over says', len(found) >= 4, {'found': found, 'captions': texts[:8]})
    caption = max(captions, key=lambda c: c['durationUs'], default=None)
    if caption:
        r.seek(caption['startUs'] + caption['durationUs'] // 2)
        time.sleep(1.5)
        r.shot('captions-voice-over')

    # Deleting a word of the voice-over cuts it from the voice-over, though its track keeps its place while cutting,
    # and the video loses the same time, so the voice-over stays with the picture.
    r.s.run(tab, 'Transcript')
    words = "[role=listbox][aria-label=Transcript] [role=option][data-t]"
    shown = wait(lambda: (n := r.s.run(f"return document.querySelectorAll('{words}').length")) and n > 5 and n, 30)
    r.check('the transcript shows the voice-over\'s words', shown, shown)
    def lengths():
        """How long the video and the voice-over play, read from one state so a cut never lands between them."""
        tracks = r.state()['tracks']
        length = lambda kind: sum(c['durationUs'] for t in tracks if t['kind'] == kind for c in t['clips'])
        return {'video': length('video'), 'voice': length('audio')}
    caption_text = lambda: ' '.join(c['text'] for c in next((t['clips'] for t in r.state()['tracks'] if t['name'] == 'Captions'), [])).lower()
    said = caption_text()
    before = lengths()
    word = r.s.run(f"const w = document.querySelectorAll('{words}')[4]; w.scrollIntoView({{block: 'center'}}); "
                   "return {id: w.id, text: w.textContent}")
    r.check(f'a click on the word "{word and word["text"]}" selects it', word and Hands(r).click(f'#{word["id"]}')
            and wait(lambda: r.s.run("return document.getElementById(arguments[0])?.getAttribute('aria-selected')", word['id']) == 'true', 3))
    r.key('Delete')
    after = wait(lambda: (a := lengths()) and a['voice'] < before['voice'] and a, 10)
    frame = 1_000_000 // 30 + 1
    r.check('Delete cuts the word out of the voice-over', after, {'before': before, 'after': after})
    r.check('the video loses the same time, so the voice-over stays with the picture',
            after and abs((before['video'] - after['video']) - (before['voice'] - after['voice'])) <= frame, {'before': before, 'after': after})
    gone = word and word['text'].strip().lower()
    r.check(f'the captions show "{gone}" once less', gone and caption_text().count(gone) == said.count(gone) - 1,
            {'before': said, 'after': caption_text()})
    r.shot('captions-voice-over-cut')
    r.check('no error toast', not r.errors(), r.errors())

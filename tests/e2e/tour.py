"""The first launch and the tour on the sample video: Get set up opens by itself once, and clicked through to its end it
goes on to the sample, with the AI request in the field; eight spotlight steps lead through a scripted AI edit with one
Undo and Redo, playback, cutting a word in the transcript, a built-in song, Lower under speech and Export. No speech
model, no agent and no internet: the sample carries its transcript, the AI turn is Nuzky's own edit, and the song is
built in."""
import time

from e2e.corrections import Hands
from e2e.harness import MACOS, flow, leave_editor, pointer, press, wait
from e2e.home import resize
from e2e.setup import relaunch

REQUEST = 'Cut the long pauses, then add captions.'
SONG = 'nuzky:dancing'
CARD = '[aria-labelledby=tour-title]'
DEMO = 'Tour demo · no AI ran'
TOUR = """const t = window.__nuzky.tour.getState(), c = document.querySelector('[aria-labelledby=tour-title]'), f = document.activeElement;
const svg = document.querySelector('svg path[fill-rule=evenodd]')?.ownerSVGElement;
const video = document.querySelector('section[aria-label=Preview] canvas')?.getBoundingClientRect(), b = c?.getBoundingClientRect();
return {step: t.step, stopped: t.stopped, undone: t.undone, card: c ? c.innerText : null, dim: !!svg,
        aside: !!c && getComputedStyle(c).visibility === 'hidden' && getComputedStyle(svg).visibility === 'hidden',
        overVideo: !!(b && video && b.right > video.left && b.left < video.right && b.bottom > video.top && b.top < video.bottom),
        buttons: c ? [...c.querySelectorAll('button')].map((b) => b.textContent.trim()) : [],
        focus: f?.getAttribute('aria-label') || f?.textContent?.trim().slice(0, 40) || f?.tagName};"""
PROJECT = """const st = window.__nuzky.store.getState(), p = st.snap.project;
const end = (audio) => p.tracks.filter((t) => (t.kind === 'audio') === audio).reduce((m, t) => t.clips.reduce((n, c) => Math.max(n, c.startUs + c.durationUs), m), 0);
const song = (c) => { const a = p.assets.find((a) => a.id === c.content.assetId); return !!a?.credit && `${a.credit.source}:${a.credit.id}` === arguments[0]; };
const captions = p.tracks.filter((t) => t.kind === 'text' && t.name === 'Captions').flatMap((t) => t.clips).sort((a, b) => a.startUs - b.startUs);
return {name: p.name, durationUs: end(false) || end(true), epoch: st.snap.sessionEpoch, view: st.view, panelTab: st.panelTab, aiRun: st.aiRun,
        captions: captions.length, firstCaption: captions[0] ? [captions[0].startUs, captions[0].startUs + captions[0].durationUs] : null,
        timeUs: st.timeUs, playing: st.playing, canRedo: st.snap.canRedo,
        songs: p.tracks.filter((t) => t.kind === 'audio').flatMap((t) => t.clips).filter(song).map((c) => ({id: c.id, volume: c.content.volume, duckDb: c.content.duckDb ?? 0})),
        selection: st.selection, canUndo: st.snap.canUndo};"""
AGENT = """const a = window.__nuzky.agent.getState(), panel = document.querySelector('aside[aria-label=AI]');
return {demo: a.demo, status: a.status, draft: a.draft, chat: a.chat,
        items: a.items.map((i) => ({kind: i.kind, text: i.text ?? i.label ?? null, stopped: i.stopped ?? null, undone: i.undone ?? null,
                                    steps: i.steps?.map((s) => [s.title, s.status]) ?? null})),
        header: panel?.querySelector('header')?.innerText ?? null, agentMenu: !!panel?.querySelector('button[title="Choose the agent"]'),
        modelMenu: !!panel?.querySelector('button[title="Choose the model and effort"]'), attach: !!panel?.querySelector('button[aria-label^="Attach"]'),
        separators: panel ? [...panel.querySelectorAll('[role=separator]')].map((s) => s.textContent.trim()) : [],
        undo: (() => { const b = panel?.querySelector('[data-run-card] button[title="Undo everything this run did"]'); return b ? b.getAttribute('aria-disabled') !== 'true' : null; })(),
        readOnly: panel?.querySelector('textarea')?.readOnly ?? null};"""
SETUP_OPEN = "return !!document.querySelector('[aria-labelledby=setup-title]')"
FULL = "return document.querySelector('section[aria-label=Preview]').classList.contains('fixed')"
FIRST_LAUNCH = 'return window.__nuzky.tour.getState().firstLaunch'
FOOTER = """return [...document.querySelectorAll('#setup-panel footer button')].map((b) => ({text: b.textContent.trim(),
    prominent: b.classList.contains('btn-prominent'), ghost: b.hasAttribute('data-sample'), focused: b === document.activeElement}));"""
TAKE_THE_TOUR = "[...document.querySelectorAll('nav[aria-label=Collections] [data-row]')].find((b) => b.textContent.startsWith('Take the tour')).click()"


def tour(r):
    return r.s.run(TOUR, retries=3)


def project(r):
    return r.s.run(PROJECT, SONG, retries=3)


def agent(r):
    return r.s.run(AGENT, retries=3)


def at_step(r, step, timeout=20):
    return wait(lambda: tour(r)['step'] == step, timeout)


def card_button(r, text):
    """A selector for the tour card's button with this text."""
    r.s.run(f"[...document.querySelectorAll('{CARD} button')].find((b) => b.textContent.trim() === arguments[0]).dataset.e2e = arguments[0]", text)
    return f'{CARD} [data-e2e="{text}"]'


def click_at(hands, css):
    """A real click in the middle of the element, whatever lies over it there. Returns whether the element was the one on
    top, as a person's click would reach it."""
    on_top = hands.over(css)
    if MACOS:
        pointer('down', *hands.at, 1)
        time.sleep(0.03)
        pointer('up', *hands.at, 1)
    else:
        from Xlib import X
        hands.fake(X.ButtonPress, 1)
        time.sleep(0.03)
        hands.fake(X.ButtonRelease, 1)
    time.sleep(0.4)
    return on_top


def offline(r):
    # Every request Nuzky would make goes to a closed port, as without internet.
    dead = 'http://127.0.0.1:9'
    r.env.update(HTTP_PROXY=dead, HTTPS_PROXY=dead, ALL_PROXY=dead, http_proxy=dead, https_proxy=dead, all_proxy=dead,
                 NO_PROXY='localhost', no_proxy='localhost')


def storage(r, script):
    """Changes the profile's remembered settings, then gives WebKit, which writes them half a second later, time to."""
    r.s.run(script)
    time.sleep(1)


@flow('tour', 'A fresh profile opens Get set up by itself once, and its end goes on to the sample, never for a profile that kept anything; '
      'the tour leads through a scripted AI edit with one Undo and Redo, playback from the first caption, cutting a word in the '
      'transcript, a built-in song, Lower under speech and Export, offline, without speech models or agents; clicks outside the '
      'spotlight do nothing, and the launcher and the full screen preview put it aside', before=offline, home=True, first_launch=True)
def flow_tour(r):
    r.check('the window is 1440 × 900', resize(r, 1440, 900))
    r.check('a fresh profile opens Get set up by itself', wait(lambda: r.s.run(SETUP_OPEN), 15))
    r.check('it says so in the tour state', r.s.run(FIRST_LAUNCH) is True)
    # After the dialog's fade-in.
    time.sleep(0.3)
    r.shot('0-setup-1440')
    press('Escape')
    r.check('Esc closes it', wait(lambda: not r.s.run(SETUP_OPEN), 5))
    r.check('and leaves the home screen with its own Try the sample video', r.s.run(
        "return [...document.querySelectorAll('button')].some((b) => b.textContent.trim() === 'Try the sample video')"))

    # Still only the empty project Nuzky made, but Get set up was seen: it never opens by itself again.
    relaunch(r)
    r.check('after a relaunch Get set up does not open by itself again',
            wait(lambda: r.s.run(FIRST_LAUNCH) is False, 15) and not r.s.run(SETUP_OPEN), r.s.run('return window.__nuzky.tour.getState()'))

    # Someone who used Nuzky before, with no projects left and nothing of Get set up or the tour remembered, but other choices.
    storage(r, "localStorage.removeItem('nuzky.setup'); localStorage.removeItem('nuzky.tour'); localStorage.setItem('nuzky.previewMuted', '0');")
    relaunch(r)
    kept = r.s.run("return Object.keys(localStorage).filter((k) => k.startsWith('nuzky.'))")
    r.check('a profile that kept any choice of Nuzky\'s is no first launch, so Get set up does not open by itself',
            wait(lambda: r.s.run(FIRST_LAUNCH) is False, 15) and not r.s.run(SETUP_OPEN), {'tour': r.s.run('return window.__nuzky.tour.getState()'), 'kept': kept})

    # A fresh profile once more: Get set up, clicked through to its end, goes on to the sample.
    storage(r, 'localStorage.clear()')
    relaunch(r)
    r.check('a profile with nothing remembered opens Get set up by itself again', wait(lambda: r.s.run(FIRST_LAUNCH) is True and r.s.run(SETUP_OPEN), 15))
    r.check('the window is 1440 × 900 again', resize(r, 1440, 900))
    hands = Hands(r)
    # Through to the end from the keyboard, as a person reaches the footer.
    for _ in range(4):
        r.s.run("[...document.querySelectorAll('#setup-panel footer button')].find((b) => ['Skip', 'Next'].includes(b.textContent.trim()))?.focus()")
        press('Return')
        time.sleep(0.3)
    footer = wait(lambda: (f := r.s.run(FOOTER)) and any(b['text'] == 'Try the sample video' and not b['ghost'] for b in f) and f, 5) or r.s.run(FOOTER)
    sample = [b for b in footer if b['text'] == 'Try the sample video']
    r.check('on the first launch the last step ends on Try the sample video, prominent and focused, in place of Done',
            len(sample) == 1 and sample[0]['prominent'] and not sample[0]['ghost'] and sample[0]['focused'] and 'Done' not in [b['text'] for b in footer]
            and not any(b['prominent'] for b in footer if b['text'] != 'Try the sample video'), footer)
    r.shot('0-setup-end-1440')
    press('Return')
    r.check('it closes Get set up and opens the sample in the editor',
            wait(lambda: not r.s.run(SETUP_OPEN) and project(r)['name'] == 'Sample video' and project(r)['view'] == 'editor', 20), project(r))
    r.check('the tour starts on Ask the AI to edit', at_step(r, 'ask'), tour(r))
    a = wait(lambda: (x := agent(r))['header'] and x, 10) or agent(r)
    r.check('the request is in the AI field, which takes no typing', a['draft'] == REQUEST and a['readOnly'] is True, a)
    r.check('the panel is a demo: no agent, model or attachments offered', a['demo'] and 'AI demo' in a['header'] and not a['agentMenu']
            and not a['modelMenu'] and not a['attach'], a)
    r.check('the keyboard is in the field', wait(lambda: tour(r)['focus'] == 'Request to the AI demo', 5), tour(r))
    r.check('the card says 1 of 8 and to try the request written in plain words', '1 of 8' in (tour(r)['card'] or '')
            and 'Try this one: press Enter.' in tour(r)['card'] and 'Close the tour' not in r.s.run(f"return document.querySelector('{CARD}').innerHTML"), tour(r))
    time.sleep(0.5)
    r.shot('1-ask-1440')

    # A real click outside the hole lands on the dim, not on the Text tab under it.
    before = project(r)
    reached = click_at(hands, '[data-tab=text]')
    r.check('a real click on the Text tab outside the spotlight is blocked', not reached and project(r)['panelTab'] == before['panelTab'],
            {'reached': reached, 'panelTab': project(r)['panelTab']})
    r.check('the click did not end the tour', tour(r)['step'] == 'ask', tour(r))

    # Enter sends the request: the edit runs as an AI run, live.
    hands.over('[data-tour=ai-field] textarea')
    r.s.run("document.querySelector('[data-tour=ai-field] textarea').focus()")
    press('Return')
    r.check('Enter starts the turn and Watch it edit', at_step(r, 'watch', 10), tour(r))
    r.check('the editor locks for the run', wait(lambda: project(r)['aiRun'] is not None, 10), project(r))
    r.check('the card says to watch the timeline and that Stop and edit takes over',
            'Watch the timeline change. Stop and edit, at the top, lets you take over.' in (tour(r)['card'] or ''), tour(r))
    r.shot('2-watch-1440')
    r.check('the panel labels the scripted turn as a demo', wait(lambda: DEMO in agent(r)['separators'], 5), agent(r)['separators'])
    r.check('the turn ends', wait(lambda: agent(r)['status'] == 'idle', 30), agent(r))
    ended = time.time()
    r.check('Watch it edit stays a moment after the edit ends', tour(r)['step'] == 'watch', tour(r))
    r.check('then One Undo takes it all back follows', at_step(r, 'undo', 10) and time.time() - ended > 1, [tour(r), time.time() - ended])
    after = project(r)
    a = agent(r)
    kinds = [i['kind'] for i in a['items']]
    reply = next((i['text'] for i in a['items'] if i['kind'] == 'agent'), '')
    r.check('the timeline is shorter', after['durationUs'] < before['durationUs'], [before['durationUs'], after['durationUs']])
    r.check('captions were added', after['captions'] > 0, after['captions'])
    r.check('the conversation is the request, the steps, the reply and the run card', kinds == ['user', 'steps', 'agent', 'run'], kinds)
    steps = next(i['steps'] for i in a['items'] if i['kind'] == 'steps')
    r.check('the three steps are done', steps == [['Read the transcript', 'done'], ['Cutting long pauses', 'done'], ['Adding captions', 'done']], steps)
    seconds = f"{(before['durationUs'] - after['durationUs']) / 1e6:.1f} s shorter"
    r.check('the reply counts what the edit did, and no retakes, as the sample has none',
            reply.startswith('Done. I cut') and seconds in reply and f"added {after['captions']} caption" in reply and 'retake' not in reply,
            {'reply': reply, 'expected': seconds})
    r.check('the demo keeps no chat with the project', a['chat'] is None, a)
    r.check('the ring around the spotlight is a white hairline, not blue', r.s.run(
        "const ring = document.querySelector('svg path[fill-rule=evenodd]').ownerSVGElement.querySelector('rect'); return [ring.getAttribute('stroke'), ring.getAttribute('stroke-width')]") == ['rgb(255 255 255 / 0.24)', '1'])
    time.sleep(0.5)
    r.shot('3-undo-1440')

    # Undo on the run card, the button the step points at: the card says the video is back and offers Redo.
    r.check('a real click reaches Undo on the run card', click_at(hands, '[data-run-card] button[title="Undo everything this run did"]'))
    t = wait(lambda: (x := tour(r))['undone'] and x, 10) or tour(r)
    p = project(r)
    r.check('the edit is undone, and the card says so and offers Redo with its shortcut',
            p['durationUs'] == before['durationUs'] and p['captions'] == 0 and 'Undone. The video is back as it was.' in (t['card'] or '')
            and 'Ctrl+Shift+Z' in t['card'] and t['buttons'][-2:] == ['Redo', 'Next'], [p, t])
    r.check('the run card says Undone', any(i['kind'] == 'run' and i['undone'] for i in agent(r)['items']), agent(r)['items'])
    time.sleep(0.3)
    r.shot('3-undone-1440')
    r.check('a real click reaches Redo', click_at(hands, card_button(r, 'Redo')))
    t = wait(lambda: not (x := tour(r))['undone'] and x, 10) or tour(r)
    p = project(r)
    a = agent(r)
    r.check('Redo brings the edit back and the card its first words', p['durationUs'] == after['durationUs'] and p['captions'] == after['captions']
            and 'Undo here takes back the whole edit' in (t['card'] or '') and 'Redo' not in t['buttons'], [p, t])
    r.check('the run card offers Undo again', not any(i['kind'] == 'run' and i['undone'] for i in a['items']) and a['undo'] is True, a)

    r.check('Next leads to Play the result', click_at(hands, card_button(r, 'Next')) and at_step(r, 'play', 5), tour(r))
    p = project(r)
    r.check('the playhead is on the first caption, so the still frame shows the edit', p['firstCaption'] is not None
            and p['firstCaption'][0] <= p['timeUs'] < p['firstCaption'][1], p)
    r.check('the card names no licence', 'CC BY' not in (tour(r)['card'] or ''), tour(r))
    r.check('the keyboard is on Next, out of the text field', wait(lambda: tour(r)['focus'] == 'Next', 5), tour(r))
    time.sleep(0.5)
    r.shot('4-play-1440')
    press('space')
    r.check('Space plays', wait(lambda: project(r)['playing'], 5), project(r))
    time.sleep(1.5)
    r.check('playing does not move on: the card stays and keeps off the video', tour(r)['step'] == 'play' and not tour(r)['overVideo'], tour(r))
    r.shot('4-playing-1440')
    press('f')
    r.check('F shows the preview full screen, with the card over it', wait(lambda: r.s.run(FULL), 5) and not tour(r)['aside'], tour(r))
    time.sleep(0.5)
    r.shot('4-full-1440')
    r.check('a real click reaches Next over the full screen preview', click_at(hands, card_button(r, 'Next')))
    r.check('Next leads to Fix anything by hand, stops playback and leaves full screen', at_step(r, 'transcript', 5) and wait(
        lambda: not project(r)['playing'] and not r.s.run(FULL), 5) and not tour(r)['aside'], [tour(r), project(r), r.s.run(FULL)])

    # The transcript's first word is selected: Delete cuts it.
    r.check('the Transcript tab shows the words', wait(lambda: project(r)['panelTab'] == 'transcript' and len(
        r.s.run("return document.querySelector('#library-panel-transcript')?.innerText ?? ''")) > 100, 10), project(r))
    picked = lambda: r.s.run("return [...document.querySelectorAll('#library-panel-transcript [aria-selected=true]')].map((w) => w.textContent.trim())")
    r.check('the first word, But, is selected', wait(lambda: picked() == ['But'], 5), picked())
    t = wait(lambda: (x := tour(r))['focus'] == 'Transcript' and x, 5) or tour(r)
    r.check('the card names it and says Delete cuts it, and the keyboard is in the transcript',
            'The video starts on “But”. Press Delete to cut it.' in (t['card'] or '') and t['focus'] == 'Transcript', t)
    r.check('the card keeps off the video', not t['overVideo'], t)
    time.sleep(0.5)
    r.shot('5-transcript-1440')
    cut = project(r)
    press('Delete')
    r.check('Delete cuts the word and moves on to Add a song in Audio › Music', at_step(r, 'song', 10) and project(r)['panelTab'] == 'audio'
            and project(r)['durationUs'] < cut['durationUs'], [tour(r), project(r)['durationUs'], cut['durationUs']])

    row = f'[data-sound="{SONG}"]'
    r.check('the built-in song is listed, its + showing without the pointer and focused',
            wait(lambda: r.s.run(f"""const b = document.querySelector('{row} button[aria-label^="Add"]');
                return !!b && getComputedStyle(b).opacity === '1' && document.activeElement === b;"""), 10), tour(r))
    r.check('Sound effects does not list the song', SONG not in r.s.run(
        "return window.__nuzky.sounds.getState().builtIn.filter((s) => s.kind === 'effect').map((s) => s.id)"))
    r.check('the card speaks of her voice and keeps off the video', 'under her voice' in (tour(r)['card'] or '') and not tour(r)['overVideo'], tour(r))
    time.sleep(0.5)
    r.shot('6-song-1440')
    press('Return')
    r.check('Enter on + adds the song offline and leads to Fine-tune', at_step(r, 'duck', 20), [tour(r), r.errors()])
    p = project(r)
    r.check('the song clip is on the timeline and selected', len(p['songs']) == 1 and p['selection'] == [p['songs'][0]['id']], p)
    r.check('the card says to tick Lower under speech, and the keyboard is on it',
            'Tick Lower under speech' in (tour(r)['card'] or '') and wait(lambda: r.s.run("return !!document.activeElement?.closest('[data-tour=duck]')"), 5), tour(r))
    r.check('the AI panel, a column that fits beside the editor, stays open', r.s.run('return window.__nuzky.dock.getState().open'))
    time.sleep(0.5)
    r.shot('7-duck-1440')
    # A narrower window puts the AI panel in the inspector's place: the step keeps it out of the way meanwhile.
    resize(r, 1200, 900)
    r.check('shrunk under 1330 px, the AI panel closes and Lower under speech stays in the hole', wait(
        lambda: not r.s.run('return window.__nuzky.dock.getState().open') and r.s.run(
            "const b = document.querySelector('[data-tour=duck]')?.getBoundingClientRect(); return !!b && b.width > 0 && !!document.elementFromPoint(b.left + 8, b.top + b.height / 2)?.closest('[data-tour=duck]')"), 5),
        r.s.run('return window.__nuzky.dock.getState()'))
    r.key('j', ctrlKey=True)
    time.sleep(0.3)
    r.check('opened again with Ctrl+J meanwhile, it stays out of the way', not r.s.run('return window.__nuzky.dock.getState().open'))
    resize(r, 1440, 900)
    r.s.run("document.querySelector('[data-tour=duck] input').focus()")
    press('space')
    r.check('Space ticks Lower under speech, which moves on to Export', at_step(r, 'export', 10) and project(r)['songs'][0]['duckDb'] > 0, [tour(r), project(r)['songs']])
    r.check('the AI panel is back', wait(lambda: r.s.run('return window.__nuzky.dock.getState().open'), 5))

    t = tour(r)
    r.check('without a real AI the last step says the tour used a demo and offers Connect your AI in place of Done',
            'The AI in this tour was a demo.' in (t['card'] or '') and t['buttons'] == ['Connect your AI', 'Open my own video…'], t)
    r.shot('8-export-1440')
    # Ctrl+K: the launcher opens above the editor, and the tour steps aside until it closes.
    r.key('k', ctrlKey=True)
    r.check('Ctrl+K opens the launcher, and the tour steps aside without ending', wait(
        lambda: r.s.run("return !!document.querySelector('[role=dialog][aria-label=Projects]')") and tour(r)['aside'], 5) and tour(r)['step'] == 'export', tour(r))
    r.check("a real click reaches the launcher's search field",
            click_at(hands, '[aria-label="Search projects and what was said in them"]') and r.s.run("return document.activeElement?.getAttribute('role')") == 'combobox')
    r.shot('8-launcher-1440')
    press('Escape')
    r.check('Esc closes the launcher and the tour comes back on the same step', wait(
        lambda: not r.s.run("return !!document.querySelector('[role=dialog][aria-label=Projects]')") and not tour(r)['aside'], 5)
        and tour(r)['step'] == 'export', tour(r))
    r.check('a real click reaches Connect your AI', click_at(hands, card_button(r, 'Connect your AI')))
    t = wait(lambda: (x := tour(r))['step'] is None and x, 5) or tour(r)
    r.check('Connect your AI ends the tour and opens Get set up on Connect your AI', t['step'] is None and not t['dim'] and t['card'] is None
            and wait(lambda: r.s.run(SETUP_OPEN), 5) and r.s.run('return window.__nuzky.setup.getState().current') == 'ai', t)
    r.check('Get set up ends with Done again, the first launch is over', 'Done' in [b['text'] for b in r.s.run(FOOTER)], r.s.run(FOOTER))
    press('Escape')
    r.check('Esc closes Get set up', wait(lambda: not r.s.run(SETUP_OPEN), 5))
    a = wait(lambda: (x := agent(r))['agentMenu'] and x, 5) or agent(r)
    r.check('after the tour the AI panel is the usual one again', not a['demo'] and a['agentMenu'] and a['readOnly'] is False and a['draft'] == '', a)
    r.check('and the scripted turn, still shown under it, says no AI ran', DEMO in a['separators'] and 'AI demo' not in (a['header'] or ''), a)
    r.shot('9-done-1440')

    # The smallest window: the AI panel takes the inspector's place, which Lower under speech needs.
    r.check('the window is 1024 × 640', resize(r, 1024, 640))
    leave_editor(r)
    r.s.run(TAKE_THE_TOUR)
    r.check('Take the tour opens a fresh sample', wait(lambda: (p := project(r))['epoch'] != after['epoch'] and p['name'] == 'Sample video' and p['captions'] == 0, 20)
            and at_step(r, 'ask'), project(r))
    wait(lambda: tour(r)['focus'] == 'Request to the AI demo', 5)
    time.sleep(0.5)
    r.shot('1-ask-1024')
    press('Return')
    r.check('at 1024 × 640 the edit runs too', wait(lambda: agent(r)['status'] == 'idle' and tour(r)['step'] == 'watch', 30), [agent(r), tour(r)])
    # Ctrl+Z while Watch it edit still shows: One Undo then says the edit is undone and offers Redo, which brings it back.
    r.s.run('document.activeElement?.blur()')
    r.key('z', ctrlKey=True)
    r.check('Undo while Watch it edit still shows is followed by One Undo saying so',
            at_step(r, 'undo', 10) and wait(lambda: (t := tour(r))['undone'] and 'Redo' in t['buttons'], 5), tour(r))
    click_at(hands, card_button(r, 'Redo'))
    r.check('its Redo brings the edit back', wait(lambda: not tour(r)['undone'] and project(r)['captions'] > 0, 10), [tour(r), project(r)])
    time.sleep(0.5)
    r.shot('3-undo-1024')
    cut = project(r)

    # Undo on the run card, through the hole, takes back the cut and the captions in one step; Next goes on without Redo.
    r.check('a real click reaches Undo on the run card', click_at(hands, '[data-run-card] button[title="Undo everything this run did"]'))
    undone = wait(lambda: (p := project(r))['durationUs'] == before['durationUs'] and p['captions'] == 0 and p, 10) or project(r)
    r.check('one Undo restores the original length and removes the captions', cut['durationUs'] < before['durationUs'] and cut['captions'] > 0
            and undone['durationUs'] == before['durationUs'] and undone['captions'] == 0, [before['durationUs'], cut, undone])
    r.check('the run card says Undone, and the tour card offers Redo', any(i['kind'] == 'run' and i['undone'] for i in agent(r)['items'])
            and wait(lambda: 'Redo' in tour(r)['buttons'], 5), [agent(r)['items'], tour(r)])
    r.shot('3-undone-1024')
    click_at(hands, card_button(r, 'Next'))
    r.check('Next goes on to Play the result on the uncut video', at_step(r, 'play', 5) and project(r)['durationUs'] == before['durationUs'], tour(r))
    r.shot('4-play-1024')
    click_at(hands, card_button(r, 'Next'))
    r.check('Fix anything by hand', at_step(r, 'transcript', 5), tour(r))
    wait(lambda: 'starts on' in (tour(r)['card'] or ''), 5)
    r.check('the card keeps off the video at 1024 × 640 too', not tour(r)['overVideo'], tour(r))
    r.shot('5-transcript-1024')
    words = project(r)['durationUs']
    click_at(hands, card_button(r, 'Next'))
    r.check('Next skips cutting the word', at_step(r, 'song', 5) and project(r)['durationUs'] == words, tour(r))
    wait(lambda: r.s.run(f"return document.activeElement?.closest('{row}') !== null"), 10)
    time.sleep(0.5)
    r.shot('6-song-1024')
    press('Return')
    r.check('Fine-tune', at_step(r, 'duck', 20), tour(r))
    r.check('the AI panel in the inspector place closes for it, so Lower under speech shows',
            not r.s.run('return window.__nuzky.dock.getState().open') and r.s.run("return !!document.querySelector('[data-tour=duck]')"))
    r.check('the card keeps off the video', not tour(r)['overVideo'], tour(r))
    time.sleep(0.5)
    r.shot('7-duck-1024')
    click_at(hands, card_button(r, 'Next'))
    r.check('Export, with the AI panel back', at_step(r, 'export', 5) and r.s.run('return window.__nuzky.dock.getState().open'), tour(r))
    r.check('the keyboard is on Open my own video…, not in the field of the panel that came back',
            wait(lambda: tour(r)['focus'] == 'Open my own video…', 5), tour(r))
    time.sleep(0.5)
    r.shot('8-export-1024')
    press('Escape')
    r.check('Esc ends the tour', wait(lambda: tour(r)['step'] is None, 5), tour(r))

    # Get set up's own Try the sample video, from the keyboard; Stop and edit in the top bar, inside the second hole, ends
    # the turn as stopped after the cut, before the captions; One Undo then undoes and redoes that cut; the home screen
    # ends the tour.
    leave_editor(r)
    r.s.run("[...document.querySelectorAll('nav[aria-label=Collections] [data-row]')].find((b) => b.textContent.startsWith('Get set up')).click()")
    r.check('the sidebar opens Get set up', wait(lambda: r.s.run(SETUP_OPEN), 5))
    r.check('Get set up offers Try the sample video', r.s.run("""const b = [...document.querySelectorAll('#setup-panel footer button')]
        .find((b) => b.textContent.trim() === 'Try the sample video'); if (!b) return false; b.focus(); return document.activeElement === b;"""))
    press('Return')
    r.check('it closes Get set up and the tour starts again', wait(lambda: not r.s.run(SETUP_OPEN), 5) and at_step(r, 'ask', 20), tour(r))
    wait(lambda: tour(r)['focus'] == 'Request to the AI demo', 5)
    uncut = project(r)['durationUs']
    press('Return')
    r.check('the run bar shows Stop and edit', wait(lambda: r.s.run("return !!document.querySelector('[data-tour=run-bar] button')"), 10))
    r.check('the cut shows before the captions', wait(lambda: (x := project(r))['durationUs'] < uncut and x['captions'] == 0, 10), project(r))
    r.check('a real click reaches Stop and edit', click_at(hands, '[data-tour=run-bar] button'))
    t = wait(lambda: (x := tour(r))['stopped'] and x, 15) or tour(r)
    r.check('the card says what it changed stays and offers Next', t['step'] == 'watch' and 'What it changed so far stays' in (t['card'] or '')
            and 'Next' in t['card'], t)
    a = agent(r)
    r.check('the turn ends stopped, without a reply', a['status'] == 'idle' and not any(i['kind'] == 'agent' for i in a['items'])
            and any(i['kind'] == 'run' and i['stopped'] for i in a['items']), a)
    stopped = project(r)
    r.check('the cut stays, without captions', stopped['durationUs'] < uncut and stopped['captions'] == 0, stopped)
    r.shot('2-stopped-1024')
    r.check('Next leads to One Undo', click_at(hands, card_button(r, 'Next')) and at_step(r, 'undo', 5), tour(r))
    r.check('a real click reaches Undo on the stopped run card', click_at(hands, '[data-run-card] button[title="Undo everything this run did"]'))
    t = wait(lambda: (x := tour(r))['undone'] and x, 10) or tour(r)
    r.check('the cut is undone and the card offers Redo', project(r)['durationUs'] == uncut and t['buttons'][-2:] == ['Redo', 'Next'], [project(r), t])
    r.check('a real click reaches Redo', click_at(hands, card_button(r, 'Redo')))
    t = wait(lambda: not (x := tour(r))['undone'] and x, 10) or tour(r)
    a = agent(r)
    r.check('Redo brings the cut back, and the run card offers Undo again', project(r)['durationUs'] == stopped['durationUs']
            and 'Redo' not in t['buttons'] and not any(i['kind'] == 'run' and i['undone'] for i in a['items']) and a['undo'] is True, [t, a])
    leave_editor(r)
    r.check('going home ends the tour', wait(lambda: tour(r)['step'] is None, 5), tour(r))
    # The AI beside Your style shows the same conversation, so it says there too that no AI ran.
    r.s.run("[...document.querySelectorAll('nav[aria-label=Collections] [data-row]')].find((b) => b.textContent.startsWith('Your style')).click()")
    r.check('Your style opens', wait(lambda: r.s.run("return !!document.querySelector('main[aria-label=\"Your style\"]')"), 10))
    ai = 'button[title^="Talk about your style"]'
    r.s.run(f"document.querySelector('{ai}').click()")
    chat = """const panel = document.querySelector('main[aria-label="Your style"] ~ aside[aria-label=AI]');
        return panel && {items: panel.querySelectorAll('[data-run-card]').length, separators: [...panel.querySelectorAll('[role=separator]')].map((s) => s.textContent.trim())};"""
    a = wait(lambda: r.s.run(chat), 5) or r.s.run(chat)
    r.check("Your style's AI says the tour's turn was a demo", a and a['items'] and DEMO in a['separators'], a)
    r.s.run("document.querySelector('button[title^=\"Close the AI\"]').click()")

    # Skipped while the AI edits, the panel stays a demo until its turn ends, and its own Stop ends that turn.
    r.s.run(TAKE_THE_TOUR)
    r.check('the tour starts once more', at_step(r, 'ask', 20), tour(r))
    wait(lambda: tour(r)['focus'] == 'Request to the AI demo', 5)
    press('Return')
    press('Escape')
    r.check('Esc ends the tour while the AI works', wait(lambda: tour(r)['step'] is None, 5), tour(r))
    r.check('the panel stays a demo while its turn runs', agent(r)['demo'] and agent(r)['status'] == 'working', agent(r))
    r.s.run("[...document.querySelectorAll('aside[aria-label=AI] button')].find((b) => b.textContent.trim() === 'Stop').dataset.e2e = 'stop'")
    r.check("a real click reaches the panel's Stop", click_at(hands, '[data-e2e=stop]'))
    a = wait(lambda: (x := agent(r))['status'] == 'idle' and x, 15) or agent(r)
    r.check("the panel's Stop ends the turn stopped, without a reply", not any(i['kind'] == 'agent' for i in a['items'])
            and any(i['kind'] == 'stopped' or (i['kind'] == 'run' and i['stopped']) for i in a['items']), a)
    r.check('then the panel is the usual one again', wait(lambda: (x := agent(r))['agentMenu'] and not x['demo'], 5), agent(r))

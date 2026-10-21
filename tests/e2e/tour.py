"""The first launch and the tour on the sample video: Get set up opens by itself once, Try the sample video opens the sample
with the AI request in the field, and eight spotlight steps lead through a scripted AI edit with one Undo, playback, the
transcript, a built-in song, its volume and Export. No speech model, no agent and no internet: the sample carries its
transcript, the AI turn is Nuzky's own edit, and the song is built in."""
import time

from e2e.corrections import Hands
from e2e.harness import MACOS, flow, leave_editor, pointer, press, wait
from e2e.home import resize
from e2e.setup import relaunch

REQUEST = 'Cut the long pauses, then add captions.'
SONG = 'nuzky:dancing'
CARD = '[aria-labelledby=tour-title]'
TOUR = """const t = window.__nuzky.tour.getState(), c = document.querySelector('[aria-labelledby=tour-title]'), f = document.activeElement;
return {step: t.step, stopped: t.stopped, card: c ? c.innerText : null, dim: !!document.querySelector('svg path[fill-rule=evenodd]'),
        focus: f?.getAttribute('aria-label') || f?.textContent?.trim().slice(0, 40) || f?.tagName};"""
PROJECT = """const st = window.__nuzky.store.getState(), p = st.snap.project;
const end = (audio) => p.tracks.filter((t) => (t.kind === 'audio') === audio).reduce((m, t) => t.clips.reduce((n, c) => Math.max(n, c.startUs + c.durationUs), m), 0);
const song = (c) => { const a = p.assets.find((a) => a.id === c.content.assetId); return !!a?.credit && `${a.credit.source}:${a.credit.id}` === arguments[0]; };
return {name: p.name, durationUs: end(false) || end(true), epoch: st.snap.sessionEpoch, view: st.view, panelTab: st.panelTab, aiRun: st.aiRun,
        captions: p.tracks.filter((t) => t.kind === 'text' && t.name === 'Captions').reduce((n, t) => n + t.clips.length, 0),
        songs: p.tracks.filter((t) => t.kind === 'audio').flatMap((t) => t.clips).filter(song).map((c) => ({id: c.id, volume: c.content.volume})),
        selection: st.selection, canUndo: st.snap.canUndo};"""
AGENT = """const a = window.__nuzky.agent.getState(), panel = document.querySelector('aside[aria-label=AI]');
return {demo: a.demo, status: a.status, draft: a.draft, chat: a.chat,
        items: a.items.map((i) => ({kind: i.kind, text: i.text ?? i.label ?? null, stopped: i.stopped ?? null, undone: i.undone ?? null,
                                    steps: i.steps?.map((s) => [s.title, s.status]) ?? null})),
        header: panel?.querySelector('header')?.innerText ?? null, agentMenu: !!panel?.querySelector('button[title="Choose the agent"]'),
        modelMenu: !!panel?.querySelector('button[title="Choose the model and effort"]'), attach: !!panel?.querySelector('button[aria-label^="Attach"]'),
        readOnly: panel?.querySelector('textarea')?.readOnly ?? null};"""
SETUP_OPEN = "return !!document.querySelector('[aria-labelledby=setup-title]')"


def tour(r):
    return r.s.run(TOUR, retries=3)


def project(r):
    return r.s.run(PROJECT, SONG, retries=3)


def agent(r):
    return r.s.run(AGENT, retries=3)


def at_step(r, step, timeout=20):
    return wait(lambda: tour(r)['step'] == step, timeout)


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


@flow('tour', 'A fresh profile opens Get set up by itself once; Try the sample video opens the sample with the AI request, and the '
      'tour leads through a scripted AI edit that one Undo takes back, playback, the transcript, a built-in song and Export, '
      'offline, without speech models or agents; clicks outside the spotlight do nothing', before=offline, home=True, first_launch=True)
def flow_tour(r):
    r.check('the window is 1440 × 900', resize(r, 1440, 900))
    r.check('a fresh profile opens Get set up by itself', wait(lambda: r.s.run(SETUP_OPEN), 15))
    r.check('it says so in the tour state', r.s.run('return window.__nuzky.tour.getState().firstLaunch') is True)
    r.shot('0-setup-1440')
    press('Escape')
    r.check('Esc closes it', wait(lambda: not r.s.run(SETUP_OPEN), 5))

    # Still only the empty project Nuzky made, but Get set up was seen: it never opens by itself again.
    relaunch(r)
    r.check('after a relaunch Get set up does not open by itself again',
            wait(lambda: r.s.run('return window.__nuzky.tour.getState().firstLaunch') is False, 15) and not r.s.run(SETUP_OPEN),
            r.s.run('return window.__nuzky.tour.getState()'))
    r.check('the window is 1440 × 900 again', resize(r, 1440, 900))
    hands = Hands(r)

    # Try the sample video, from the keyboard as a person would reach it.
    r.s.run("[...document.querySelectorAll('nav[aria-label=Collections] [data-row]')].find((b) => b.textContent.startsWith('Get set up')).click()")
    r.check('the sidebar opens Get set up', wait(lambda: r.s.run(SETUP_OPEN), 5))
    r.check('Get set up offers Try the sample video', r.s.run("""const b = [...document.querySelectorAll('#setup-panel footer button')]
        .find((b) => b.textContent.trim() === 'Try the sample video'); if (!b) return false; b.focus(); return document.activeElement === b;"""))
    press('Return')
    r.check('it closes Get set up and opens the sample in the editor',
            wait(lambda: not r.s.run(SETUP_OPEN) and project(r)['name'] == 'Sample video' and project(r)['view'] == 'editor', 20), project(r))
    r.check('the tour starts on Ask the AI to edit', at_step(r, 'ask'), tour(r))
    a = wait(lambda: (x := agent(r))['header'] and x, 10) or agent(r)
    r.check('the request is in the AI field, which takes no typing', a['draft'] == REQUEST and a['readOnly'] is True, a)
    r.check('the panel is a demo: no agent, model or attachments offered', a['demo'] and 'AI demo' in a['header'] and not a['agentMenu']
            and not a['modelMenu'] and not a['attach'], a)
    r.check('the keyboard is in the field', wait(lambda: tour(r)['focus'] == 'Request to the AI demo', 5), tour(r))
    r.check('the card says 1 of 8 and what to do', '1 of 8' in (tour(r)['card'] or '') and 'Press Enter to send it.' in tour(r)['card'], tour(r))
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
    r.shot('2-watch-1440')
    r.check('the run ends and One Undo takes it all back follows', at_step(r, 'undo', 30), tour(r))
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
    time.sleep(0.5)
    r.shot('3-undo-1440')

    r.check('Next leads to Play the result', click_at(hands, f'{CARD} [data-primary]') and at_step(r, 'play', 5), tour(r))
    r.check('the card credits the video', 'Video: Dianne Glavaš · CC BY 3.0' in (tour(r)['card'] or ''), tour(r))
    r.check('the keyboard is on Next, out of the text field', wait(lambda: tour(r)['focus'] == 'Next', 5), tour(r))
    r.shot('4-play-1440')
    press('space')
    r.check('Space plays, which moves on to Fix anything by hand', at_step(r, 'transcript', 5), tour(r))
    r.s.call('window.__nuzky.api.pause()')
    r.check('the Transcript tab shows the words', wait(lambda: project(r)['panelTab'] == 'transcript' and len(
        r.s.run("return document.querySelector('#library-panel-transcript')?.innerText ?? ''")) > 100, 10), project(r))
    time.sleep(0.5)
    r.shot('5-transcript-1440')

    r.check('the keyboard is on Next', wait(lambda: tour(r)['focus'] == 'Next', 5), tour(r))
    press('Return')
    r.check('Next leads to Add a song in Audio › Music', at_step(r, 'song', 5) and project(r)['panelTab'] == 'audio', [tour(r), project(r)])
    row = f'[data-sound="{SONG}"]'
    r.check('the built-in song is listed, its + showing without the pointer and focused',
            wait(lambda: r.s.run(f"""const b = document.querySelector('{row} button[aria-label^="Add"]');
                return !!b && getComputedStyle(b).opacity === '1' && document.activeElement === b;"""), 10), tour(r))
    r.check('Sound effects does not list the song', SONG not in r.s.run(
        "return window.__nuzky.sounds.getState().builtIn.filter((s) => s.kind === 'effect').map((s) => s.id)"))
    time.sleep(0.5)
    r.shot('6-song-1440')
    press('Return')
    r.check('Enter on + adds the song offline and leads to Fine-tune', at_step(r, 'volume', 20), [tour(r), r.errors()])
    p = project(r)
    r.check('the song clip is on the timeline and selected', len(p['songs']) == 1 and p['selection'] == [p['songs'][0]['id']], p)
    r.check('the keyboard is on Volume', wait(lambda: tour(r)['focus'] == 'Volume', 5), tour(r))
    # One press: through the macOS test bridge only the first of several arrow presses moved the slider, which says
    # nothing about the app; one is enough to show the keyboard path.
    press('Down')
    r.check('an arrow key turns the volume down', wait(lambda: project(r)['songs'][0]['volume'] < 1, 5), project(r)['songs'])
    time.sleep(0.5)
    r.shot('7-volume-1440')

    r.check('Next leads to Export when it is ready', click_at(hands, f'{CARD} [data-primary]') and at_step(r, 'export', 5), tour(r))
    card = tour(r)['card'] or ''
    r.check('without a real AI the last step says the tour used a demo', 'The AI in this tour was a demo.' in card, card)
    r.check('it offers Open my own video… and Done', 'Open my own video…' in card and 'Done' in card and 'Skip tour' not in card, card)
    r.shot('8-export-1440')
    r.s.run(f"[...document.querySelectorAll('{CARD} button')].find((b) => b.textContent.trim() === 'Done').dataset.e2e = 'done'")
    r.check('a real click reaches Done', click_at(hands, '[data-e2e=done]'))
    t = wait(lambda: (x := tour(r))['step'] is None and x, 5) or tour(r)
    r.check('Done ends the tour and lifts the dim', t['step'] is None and not t['dim'] and t['card'] is None, t)
    a = wait(lambda: (x := agent(r))['agentMenu'] and x, 5) or agent(r)
    r.check('after the tour the AI panel is the usual one again', not a['demo'] and a['agentMenu'] and a['readOnly'] is False and a['draft'] == '', a)
    r.shot('9-done-1440')

    # The smallest window: the AI panel takes the inspector's place, which the volume step needs.
    r.check('the window is 1024 × 640', resize(r, 1024, 640))
    leave_editor(r)
    r.s.run("[...document.querySelectorAll('nav[aria-label=Collections] [data-row]')].find((b) => b.textContent.startsWith('Take the tour')).click()")
    r.check('Take the tour opens a fresh sample', wait(lambda: (p := project(r))['epoch'] != after['epoch'] and p['name'] == 'Sample video' and p['captions'] == 0, 20)
            and at_step(r, 'ask'), project(r))
    wait(lambda: tour(r)['focus'] == 'Request to the AI demo', 5)
    time.sleep(0.5)
    r.shot('1-ask-1024')
    press('Return')
    r.check('at 1024 × 640 the edit runs too', at_step(r, 'undo', 30), tour(r))
    time.sleep(0.5)
    r.shot('3-undo-1024')
    cut = project(r)

    # Undo on the run card, through the hole, takes back the cut and the captions in one step.
    r.check('a real click reaches Undo on the run card', click_at(hands, '[data-run-card] button[title="Undo everything this run did"]'))
    undone = wait(lambda: (p := project(r))['durationUs'] == before['durationUs'] and p['captions'] == 0 and p, 10) or project(r)
    r.check('one Undo restores the original length and removes the captions', cut['durationUs'] < before['durationUs'] and cut['captions'] > 0
            and undone['durationUs'] == before['durationUs'] and undone['captions'] == 0, [before['durationUs'], cut, undone])
    r.check('the run card says Undone', any(i['kind'] == 'run' and i['undone'] for i in agent(r)['items']), agent(r)['items'])
    r.shot('3-undone-1024')
    click_at(hands, f'{CARD} [data-primary]')
    r.check('Play the result', at_step(r, 'play', 5), tour(r))
    r.shot('4-play-1024')
    click_at(hands, f'{CARD} [data-primary]')
    click_at(hands, f'{CARD} [data-primary]')
    r.check('Add a song', at_step(r, 'song', 5), tour(r))
    wait(lambda: r.s.run(f"return document.activeElement?.closest('{row}') !== null"), 10)
    time.sleep(0.5)
    r.shot('6-song-1024')
    press('Return')
    r.check('Fine-tune', at_step(r, 'volume', 20), tour(r))
    r.check('the AI panel in the inspector place closes for it, so Volume shows',
            not r.s.run('return window.__nuzky.dock.getState().open') and r.s.run("return !!document.querySelector('[data-tour=volume]')"))
    time.sleep(0.5)
    r.shot('7-volume-1024')
    click_at(hands, f'{CARD} [data-primary]')
    r.check('Export, with the AI panel back', at_step(r, 'export', 5) and r.s.run('return window.__nuzky.dock.getState().open'), tour(r))
    r.check('the keyboard is on Open my own video…, not in the field of the panel that came back',
            wait(lambda: tour(r)['focus'] == 'Open my own video…', 5), tour(r))
    time.sleep(0.5)
    r.shot('8-export-1024')
    press('Escape')
    r.check('Esc ends the tour', wait(lambda: tour(r)['step'] is None, 5), tour(r))

    # Stop and edit in the top bar, inside the second hole, ends the turn as stopped; the home screen ends the tour.
    leave_editor(r)
    r.s.run("[...document.querySelectorAll('nav[aria-label=Collections] [data-row]')].find((b) => b.textContent.startsWith('Take the tour')).click()")
    r.check('the tour starts again', at_step(r, 'ask', 20), tour(r))
    wait(lambda: tour(r)['focus'] == 'Request to the AI demo', 5)
    press('Return')
    r.check('the run bar shows Stop and edit', wait(lambda: r.s.run("return !!document.querySelector('[data-tour=run-bar] button')"), 10))
    r.check('a real click reaches Stop and edit', click_at(hands, '[data-tour=run-bar] button'))
    t = wait(lambda: (x := tour(r))['stopped'] and x, 15) or tour(r)
    r.check('the card says it was stopped and offers Next', t['step'] == 'watch' and 'You stopped it' in (t['card'] or '') and 'Next' in t['card'], t)
    a = agent(r)
    r.check('the turn ends stopped, without a reply', a['status'] == 'idle' and not any(i['kind'] == 'agent' for i in a['items'])
            and any(i['kind'] == 'stopped' or (i['kind'] == 'run' and i['stopped']) for i in a['items']), a)
    r.shot('2-stopped-1024')
    leave_editor(r)
    r.check('going home ends the tour', wait(lambda: tour(r)['step'] is None, 5), tour(r))

    # Skipped while the AI edits, the panel stays a demo until its turn ends, and its own Stop ends that turn.
    r.s.run("[...document.querySelectorAll('nav[aria-label=Collections] [data-row]')].find((b) => b.textContent.startsWith('Take the tour')).click()")
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

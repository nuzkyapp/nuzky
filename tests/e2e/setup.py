"""Get set up: one dialog with the setup steps side by side and whether each is done or skipped, only the steps that
are built. The first start's Get set up and the home sidebar's row open it, the keyboard moves through the steps,
Skip moves on, and closing, also quitting Nuzky, loses neither the steps' states nor the step shown."""
import time

from e2e.corrections import Hands
from e2e.harness import FIXTURES, Session, flow, press, wait
from e2e.home import FIRST_RUN_TEXT, OPEN_VIDEO, make, resize

TITLES = ['Captions model', 'Face and person models', 'Connect your AI']
BOX = '[aria-labelledby=setup-title]'
DIALOG = """const d = document.querySelector('[aria-labelledby=setup-title]');
if (!d) return null;
const lines = (el) => el.innerText.split('\\n').map((l) => l.trim()).filter((l) => l && !/^\\d$/.test(l));
const focus = document.activeElement;
return {
  title: d.querySelector('#setup-title').textContent, count: d.querySelector('#setup-count').textContent,
  steps: [...d.querySelectorAll('[role=tablist] [role=tab]')].map((t) => ({lines: lines(t), selected: t.getAttribute('aria-selected') === 'true'})),
  panel: d.querySelector('[role=tabpanel] h3')?.textContent ?? null,
  buttons: [...d.querySelectorAll('footer button')].map((b) => b.textContent.trim()),
  focus: focus?.getAttribute('role') === 'tab' ? 'tab: ' + lines(focus)[0] : (focus?.textContent.trim() || focus?.getAttribute('aria-label') || focus?.tagName),
};"""
FOCUS = "const f = document.activeElement; return f?.textContent.trim() || f?.getAttribute('aria-label') || f?.tagName"
ROW = """const row = [...document.querySelectorAll('nav[aria-label=Collections] [data-row]')].find((b) => b.textContent.startsWith('Get set up'));
return row && {count: row.querySelector('[aria-label]')?.getAttribute('aria-label') ?? null, current: row.getAttribute('aria-current')};"""
FOCUS_TEXT = """const el = [...document.querySelectorAll(arguments[1] + ' button')].find((b) => b.textContent.trim().startsWith(arguments[0]));
if (!el) return false; el.focus(); return document.activeElement === el;"""
CLICK = """const el = [...document.querySelectorAll(arguments[1] + ' button')].find((b) => b.textContent.trim().startsWith(arguments[0]) || b.getAttribute('aria-label') === arguments[0]);
if (!el) return false; el.click(); return true;"""
FITS = """const d = document.querySelector('[aria-labelledby=setup-title]'), b = d.getBoundingClientRect();
const out = [...d.querySelectorAll('h2, h3, p, button, [role=tab]')].filter((el) => {
  const e = el.getBoundingClientRect(); return e.width && (e.left < b.left || e.right > b.right || e.top < b.top || e.bottom > b.bottom); })
  .map((el) => el.textContent.trim());
if (b.top < 0 || b.bottom > innerHeight || b.left < 0 || b.right > innerWidth) out.push(`the dialog leaves the window: ${JSON.stringify(b)}`);
return out;"""
CLOSED = "return !document.querySelector('[aria-labelledby=setup-title], #setup-panel')"
CONNECT_OPEN = "return !!document.querySelector('[role=dialog][aria-labelledby=connect-title]')"
CONNECT_ON_TOP = """const c = document.querySelector('[role=dialog][aria-labelledby=connect-title]');
if (!c) return false;
const b = c.getBoundingClientRect();
return !!document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2)?.closest('[role=dialog][aria-labelledby=connect-title]');"""
TALL = """const box = document.querySelector('[aria-labelledby=setup-title]'), title = box.querySelector('#setup-title');
const region = box.querySelector('[role=tabpanel] > .overflow-y-auto'), top = title.getBoundingClientRect().top;
const tall = document.createElement('div');
tall.style.cssText = 'height: 1000px; flex-shrink: 0';
region.append(tall);
region.scrollTop = region.scrollHeight;
const b = box.getBoundingClientRect(), f = box.querySelector('footer button').getBoundingClientRect();
const out = {footer: f.top >= b.top && f.bottom <= b.bottom, header: title.getBoundingClientRect().top === top,
             box: box.scrollHeight > box.clientHeight, step: region.scrollTop > 0};
tall.remove();
return out;"""
AFTER_KEY = """return {search: document.querySelector('main input[type=search]')?.value ?? null,
        inside: !!document.activeElement?.closest('[aria-labelledby=setup-title]')};"""


def dialog(r):
    return r.s.run(DIALOG, retries=3)


def rail(d):
    return [s['lines'] for s in d['steps']] if d else None


def row(r):
    return r.s.run(ROW, retries=3) or {}


def relaunch(r):
    r.s.close()
    r.s = Session()
    if not wait(lambda: r.s.run('return !!window.__nuzky?.store.getState().snap', retries=3), 60):
        raise RuntimeError('the app did not start again')
    time.sleep(1)


@flow('setup', 'Get set up shows the built setup steps with their states; the first start and the sidebar open it, the keyboard '
      'moves through it, Skip moves on, and the states and the step shown stay after closing and after a restart', home=True)
def setup(r):
    r.check('the first start shows', wait(lambda: OPEN_VIDEO in (r.s.run(FIRST_RUN_TEXT) or ''), 10))
    r.check('while closed, nothing of Get set up is in the page', r.s.run(CLOSED))
    r.check('Get set up on the first start takes the focus', r.s.run(FOCUS_TEXT, 'Get set up', 'main'))
    press('Return')
    d = wait(lambda: dialog(r), 5)
    r.check('Enter on it opens the Get set up dialog', d and d['title'] == 'Get set up', d)
    r.check('it stays on the home screen', r.s.run("return window.__nuzky.store.getState().view") == 'home')
    r.check('the rail lists the built steps in order, none done, and no step that is not ready', rail(d) == [[t] for t in TITLES], rail(d))
    r.check('the header says 0 of 3 done', d and d['count'] == '0 of 3 done', d)
    r.check('Captions model is shown and selected', d and d['panel'] == 'Captions model' and d['steps'][0]['selected'], d)
    r.check("the focus is on the current step's row, not on Skip", d and d['focus'] == 'tab: Captions model', d)
    r.check('a step still to do offers Skip last', d and d['buttons'][-1:] == ['Skip'], d)

    for key, title in (('Down', 'Face and person models'), ('Down', 'Connect your AI'), ('Up', 'Face and person models'),
                       ('Up', 'Captions model'), ('End', 'Connect your AI'), ('Home', 'Captions model')):
        press(key)
        d = wait(lambda: (dialog(r) or {}).get('panel') == title and dialog(r), 3)
        r.check(f'{key} shows {title} and the focus follows', d and d['focus'] == f'tab: {title}', dialog(r))

    r.check('Skip takes the focus', r.s.run(FOCUS_TEXT, 'Skip', BOX + ' footer') and r.s.run(FOCUS) == 'Skip', r.s.run(FOCUS))
    press('Return')
    d = wait(lambda: (dialog(r) or {}).get('panel') == 'Face and person models' and dialog(r), 3)
    r.check('Enter on Skip marks Captions model Skipped and shows Face and person models',
            d and rail(d)[0] == ['Captions model', 'Skipped'] and d['steps'][1]['selected'], d)
    r.check('a skipped step does not count as done', d and d['count'] == '0 of 3 done', d)

    press('Escape')
    r.check('Esc closes it', wait(lambda: not dialog(r), 3))
    r.check('the focus is back on Get set up', wait(lambda: r.s.run(FOCUS) == 'Get set up', 3), r.s.run(FOCUS))
    r.check('closed, nothing of it stays in the page', r.s.run(CLOSED))

    r.check('the sidebar row counts 2 steps left, as news', row(r).get('count') == '2 steps left', row(r))
    r.s.run(CLICK, 'Get set up', 'nav[aria-label=Collections]')
    d = wait(lambda: dialog(r), 5)
    r.check('the sidebar row opens it again on Face and person models',
            d and d['panel'] == 'Face and person models' and d['focus'] == 'tab: Face and person models', d)
    r.check('with Captions model still Skipped', d and rail(d)[0] == ['Captions model', 'Skipped'], d)
    r.check('the row is not a page', row(r).get('current') is None, row(r))
    press('End')
    r.check('End reaches Connect your AI', wait(lambda: (dialog(r) or {}).get('panel') == 'Connect your AI', 3), dialog(r))

    r.s.run("window.__nuzky.store.setState({connectOpen: true})")
    r.check('a dialog a step opens, Connect, shows above Get set up', wait(lambda: r.s.run(CONNECT_ON_TOP), 5))
    time.sleep(0.5)
    press('Escape')
    if r.check('Esc closes only the Connect dialog; Get set up stays open', wait(lambda: not r.s.run(CONNECT_OPEN), 3) and dialog(r), dialog(r)):
        press('Escape')
        r.check('Esc again closes Get set up', wait(lambda: not dialog(r), 3), dialog(r))
    r.s.run(CLICK, 'Get set up', 'nav[aria-label=Collections]')
    d = wait(lambda: dialog(r), 5)
    r.check('the row opens it again on Connect your AI', d and d['panel'] == 'Connect your AI', d)
    # Connect your AI looks for the agents; with none, it offers Check again.
    r.check('Connect your AI, still to do, offers Check again and Skip', wait(lambda: (dialog(r) or {}).get('buttons') == ['Check again', 'Skip'], 10), dialog(r))
    time.sleep(0.3)
    r.shot('setup')
    if r.check('the window goes to its smallest size', resize(r, 1024, 640)):
        time.sleep(0.5)
        clipped = r.s.run(FITS)
        r.check('nothing of the dialog is cut off at the smallest window', clipped == [], clipped)
        r.shot('setup-1024')
        resize(r, 1440, 900)
    time.sleep(0.5)
    tall = r.s.run(TALL)
    r.check('a step taller than the dialog scrolls inside it: Skip stays in the dialog and the header in place, '
            'and the dialog itself does not scroll', tall == {'footer': True, 'header': True, 'box': False, 'step': True}, tall)

    make(r, 'talk.nuzky', 'Talk to camera', [FIXTURES / 'talk.mp4'], 0)
    r.s.run(CLICK, 'Close', BOX)
    r.check('× closes it', wait(lambda: not dialog(r), 3))

    relaunch(r)
    r.check('after a restart the row still counts 2 steps left', wait(lambda: row(r).get('count') == '2 steps left', 10), row(r))
    r.check('the sidebar row takes the focus', r.s.run(FOCUS_TEXT, 'Get set up', 'nav[aria-label=Collections]'))
    press('Return')
    d = wait(lambda: dialog(r), 5)
    r.check('after a restart it opens on the step shown before, with the same states',
            d and d['panel'] == 'Connect your AI' and rail(d) == [['Captions model', 'Skipped'], ['Face and person models'], ['Connect your AI']] and d['count'] == '0 of 3 done', d)

    hands = Hands(r)
    r.check('a click on the step text', hands.click(f'{BOX} [role=tabpanel] p'))
    press('a')
    time.sleep(0.3)
    after = r.s.run(AFTER_KEY)
    r.check("then a letter key leaves the home screen's search empty and the focus in Get set up", after == {'search': '', 'inside': True}, after)
    r.s.run("window.__nuzky.store.getState().toast({kind: 'info', text: 'Exported Talk to camera'})")
    press('F8')
    r.check('F8 still takes the focus to the newest toast', wait(lambda: r.s.run("return !!document.activeElement?.closest('[data-toast]')"), 3), r.s.run(FOCUS))
    press('Escape')
    r.check('Esc there dismisses the toast and gives the focus back to Get set up, which stays open',
            wait(lambda: (r.s.run(AFTER_KEY) or {}).get('inside'), 3) and dialog(r), r.s.run(FOCUS))
    r.check('a click beside the box', hands.click('body', 0.04, 0.5))
    r.check('a click on the scrim beside it closes Get set up', wait(lambda: not dialog(r), 3), dialog(r))
    r.check('and the focus is back on the sidebar row that opened it', wait(lambda: (r.s.run(FOCUS) or '').startswith('Get set up'), 3), r.s.run(FOCUS))
    if dialog(r):
        r.s.run(CLICK, 'Close', BOX)
    r.s.run(CLICK, 'Get set up', 'nav[aria-label=Collections]')
    d = wait(lambda: dialog(r), 5)
    r.check('it opens again where it was, with the same states',
            d and d['panel'] == 'Connect your AI' and rail(d) == [['Captions model', 'Skipped'], ['Face and person models'], ['Connect your AI']], d)
    r.s.run('document.activeElement?.blur()')
    press('Tab')
    r.check('with the focus lost, Tab goes into Get set up, never to the home screen under it',
            wait(lambda: r.s.run(f"return !!document.activeElement?.closest('{BOX}')"), 3), r.s.run(FOCUS))
    r.s.run(f"document.querySelector('{BOX} [role=tab][aria-selected=true]').focus()")

    r.s.run("window.__nuzky.setup.getState().finishStep('ai')")
    d = wait(lambda: (dialog(r) or {}).get('count') == '1 of 3 done' and dialog(r), 3)
    r.check('a finished step shows Done and the header 1 of 3 done', d and rail(d)[2] == ['Connect your AI', 'Done'], d)
    r.check('done, the last step offers Done last', d and d['buttons'][-1:] == ['Done'], d)
    r.check('with Face and person models still to do, the row counts 1 step left',
            wait(lambda: row(r).get('count') == '1 step left', 3), row(r))
    press('Home')
    d = wait(lambda: (dialog(r) or {}).get('panel') == 'Captions model' and dialog(r), 3)
    r.check('a skipped step offers Next last', d and d['buttons'][-1:] == ['Next'], d)
    r.s.run(FOCUS_TEXT, 'Next', BOX + ' footer')
    press('Return')
    r.check('Enter on Next shows the next step', wait(lambda: (dialog(r) or {}).get('panel') == 'Face and person models', 3), dialog(r))
    r.s.run(FOCUS_TEXT, 'Skip', BOX + ' footer')
    press('Return')
    d = wait(lambda: (dialog(r) or {}).get('panel') == 'Connect your AI' and dialog(r), 3)
    r.check('Enter on Skip marks Face and person models Skipped and shows Connect your AI',
            d and rail(d)[1] == ['Face and person models', 'Skipped'] and d['steps'][2]['selected'], d)
    r.check('with no step left, the row shows no count', wait(lambda: row(r).get('count') is None, 3), row(r))
    r.s.run(FOCUS_TEXT, 'Done', BOX + ' footer')
    press('Return')
    r.check('Done closes it', wait(lambda: not dialog(r), 3), dialog(r))
    r.check('no error toast along the way', not r.errors(), r.errors())

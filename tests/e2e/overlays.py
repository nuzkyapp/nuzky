import json, subprocess, time

from e2e.harness import CLI, FIXTURES, flow, wait

# Opens the clip menu by a right-click 40 px into the clip.
RIGHT_CLICK = """const c = document.querySelector(`[data-clip-id="${arguments[3]}"]`); const b = c.getBoundingClientRect();
const at = {x: b.left + 40, y: b.top + 20};
c.dispatchEvent(new MouseEvent('contextmenu', {bubbles: true, cancelable: true, clientX: at.x, clientY: at.y}));
return at;"""
# Runs arguments[0] (JS) and, two frames later, says how the layer at arguments[1] came in: whether its entry animation
# arguments[2] runs, and the point in the window it grows from.
CAME_IN = """(async () => { const done = new Function(arguments[0]).apply(null, arguments);
await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame);
const el = [...document.querySelectorAll(arguments[1])].pop(); if (!el) return JSON.stringify(null); const b = el.getBoundingClientRect();
const [x, y] = getComputedStyle(el).transformOrigin.split(' ').map(parseFloat);
return JSON.stringify({entering: el.getAnimations().some((a) => a.animationName === arguments[2] && a.playState === 'running'),
                       origin: [Math.round(b.left + x), Math.round(b.top + y)], done: done ?? null}); })()"""
# What the window shows of a layer that just went: the fading copies, and whether anything still finds the layer.
GONE = """const copies = [...document.querySelectorAll('.exit-ghost')].map((g) => { const b = g.getBoundingClientRect();
  const hit = document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2);
  return {fading: g.getAnimations().some((a) => a.animationName === 'exit-out'), inert: g.inert, text: g.textContent.trim(),
          chosen: g.querySelector('[data-chosen]')?.textContent.trim() ?? null, takesPointer: !!hit && g.contains(hit),
          top: Math.round(b.top), display: getComputedStyle(g.querySelector('.overlay') ?? g).display}; });
return {copies, live: document.querySelectorAll('[role=menu], [role=dialog], dialog[open], [aria-modal=true]').length};"""
PICK = """const item = [...document.querySelectorAll('[role=menu] [role^=menuitem]')].find((i) => i.textContent.includes(arguments[0]));
item.click();"""
TOAST = "window.__nuzky.store.getState().toast({kind: 'info', text: arguments[0]})"
TOP = """const t = [...document.querySelectorAll('[data-toast]')].find((t) => t.textContent.includes(arguments[0]));
return t ? Math.round(t.getBoundingClientRect().top) : null;"""
DISMISS = """[...document.querySelectorAll('[data-toast]')].find((t) => t.textContent.includes(arguments[0]))
  .querySelector('button[aria-label=Dismiss]').click();"""
SLIDING = """const t = [...document.querySelectorAll('[data-toast]')].find((t) => t.textContent.includes(arguments[0]));
return t.getAnimations().some((a) => !(a instanceof CSSAnimation) && a.playState === 'running');"""


def setup(r):
    project = r.work / 'data/nuzky/projects/overlays.nuzky'
    subprocess.run([str(CLI), 'new', str(project), str(FIXTURES / 'talk.mp4')], env=r.env, check=True, capture_output=True)


def came_in(r, trigger, layer, animation, *args):
    done = r.s.call(CAME_IN, trigger, layer, animation, *args)
    return json.loads(done['value']) if done['ok'] else done


def gone(r, then=None, *args):
    """Runs `then` (JS) and reads at once what is left of the layer it closed."""
    return r.s.run((then or '') + ';' + GONE, *args)


def faded(r):
    return wait(lambda: not r.s.run("return document.querySelectorAll('.exit-ghost').length"), 2)


@flow('overlays', 'Menus grow out of where they were opened, and a chosen item blinks while the menu fades out and its '
      'action runs at once; toasts rise in and the others slide to their places when one goes; dialogs fade out; '
      'nothing that fades out takes a click or counts as open', before=setup)
def overlays(r):
    clip = r.track()[0]['id']
    menu = came_in(r, RIGHT_CLICK, '[role=menu][aria-label="Clip actions"]', 'menu-in', clip)
    at = (menu or {}).get('done') or {}
    r.check('the clip menu grows out of the point that was right-clicked',
            menu and menu.get('entering') and abs(menu['origin'][0] - at['x']) <= 1 and abs(menu['origin'][1] - at['y']) <= 1, menu)
    time.sleep(0.3)
    inside = r.s.run("const b = document.querySelector('[role=menu][aria-label=\"Clip actions\"]').getBoundingClientRect(); return [Math.round(b.bottom), innerHeight]")
    r.check('once grown, the clip menu lies wholly inside the window', inside[0] <= inside[1] - 8, inside)
    r.shot('clip-menu')
    left = gone(r, PICK, 'Duplicate')
    copy = left['copies'][0] if left['copies'] else {}
    r.check('choosing Duplicate closes the menu at once; its copy blinks Duplicate and fades, inert and letting clicks through',
            left['live'] == 0 and len(left['copies']) == 1 and copy['chosen'].startswith('Duplicate') and copy['fading']
            and copy['inert'] and not copy['takesPointer'], left)
    r.check('the duplicate lands while the menu fades', wait(lambda: len(r.track()) == 2, 5), r.track())
    r.check('then the copy is gone', faded(r))

    menu = came_in(r, "document.querySelector('header button[aria-label^=\"Versions\"]').click()", '[role=menu][aria-label=Versions]', 'menu-in')
    r.check('the versions menu grows in too', menu and menu.get('entering'), menu)
    left = gone(r, "document.querySelector('[role=menu][aria-label=Versions]').dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))")
    r.check('Esc closes it at once and its copy fades without a blink',
            left['live'] == 0 and len(left['copies']) == 1 and left['copies'][0]['chosen'] is None and left['copies'][0]['fading'], left)
    r.check('and goes', faded(r))

    r.s.run(TOAST, 'First news')
    time.sleep(0.4)
    toast = came_in(r, TOAST.replace('arguments[0]', 'arguments[3]'), '[data-toast]', 'toast-in', 'Second news')
    r.check('a new toast rises into the stack', toast and toast.get('entering'), toast)
    time.sleep(0.5)
    first, second = r.s.run(TOP, 'First news'), r.s.run(TOP, 'Second news')
    r.shot('two-toasts')
    left = gone(r, DISMISS, 'Second news')
    copy = left['copies'][0] if left['copies'] else {}
    r.check('dismissing the newest toast leaves its copy fading where it was',
            len(left['copies']) == 1 and 'Second news' in copy['text'] and copy['fading'] and copy['top'] == second, [left, second])
    r.check('the toast above slides down into its place instead of jumping', r.s.run(SLIDING, 'First news'))
    r.check('and settles where the newest one was', wait(lambda: r.s.run(TOP, 'First news') == second and not r.s.run(SLIDING, 'First news'), 2),
            [first, r.s.run(TOP, 'First news'), second])
    r.check('the copy goes', faded(r))

    r.key('e', ctrlKey=True)
    r.check('Ctrl+E opens the export dialog', wait(lambda: r.s.run("return !!document.querySelector('[role=dialog]')"), 5))
    time.sleep(0.5)
    left = gone(r, "document.querySelector('[role=dialog]').dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))")
    r.check('Esc closes the export dialog at once; its copy fades with the scrim and nothing counts it as open',
            left['live'] == 0 and len(left['copies']) == 1 and left['copies'][0]['fading'] and not left['copies'][0]['takesPointer'], left)
    r.shot('dialog-fading')
    r.check('and goes', faded(r))

    r.s.run("const b = document.querySelector('header [data-feedback]'); b.focus(); b.click();")
    wait(lambda: r.s.run("return !!document.querySelector('[role=menu][aria-label=\"Send feedback\"]')"), 5)
    r.s.run(PICK, 'Report a bug')
    r.check('Report a bug opens its dialog', wait(lambda: r.s.run("return !!document.querySelector('dialog[open]')"), 5))
    faded(r)
    time.sleep(0.4)
    left = gone(r, "[...document.querySelectorAll('dialog[open] button')].find((b) => b.textContent.trim() === 'Cancel').click()")
    copy = left['copies'][0] if left['copies'] else {}
    r.check('Cancel closes the modal Report a bug dialog; its copy shows on a scrim of its own and fades, and no dialog counts as open',
            left['live'] == 0 and len(left['copies']) == 1 and copy['fading'] and copy['display'] == 'block' and not copy['takesPointer'], left)
    r.check('and goes', faded(r))
    # A dialog that stays mounted and only closes, as the model question does when the agent stops waiting, fades out too,
    # still showing what it asked.
    for time_ in ('first', 'second'):
        r.s.run("window.__nuzky.store.setState({speechModelQuestion: {id: 'test', sizeMb: 574}})")
        r.check(f'the model question opens the {time_} time', wait(lambda: r.s.run("return !!document.querySelector('dialog[open]')"), 5))
        time.sleep(0.4)
        left = gone(r, "window.__nuzky.store.setState({speechModelQuestion: null})")
        if not left['copies']:
            time.sleep(0.05)
            left = r.s.run(GONE)
        copy = left['copies'][0] if left['copies'] else {}
        r.check(f'closing it in place the {time_} time leaves a fading copy that still says what it asked, and no dialog counts as open',
                left['live'] == 0 and len(left['copies']) == 1 and copy['fading'] and '574 MB' in copy['text'], left)
        r.check('and goes', faded(r))
    r.check('no error toast', not r.errors(), r.errors())

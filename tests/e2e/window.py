"""The window's close button. In the editor it goes back to the projects with the project still open and Nuzky
running; on the home screen it quits."""
import time

from e2e.diagnostics import app_pid
from e2e.harness import FIXTURES, MACOS, close_window, flow, wait
from e2e.home import make, view

OPEN_CARD = """return [...document.querySelectorAll('[role=listbox][aria-label=Projects] [role=option][data-path]')]
  .filter((c) => c.querySelector('[data-meta]')?.textContent.includes('Open now')).map((c) => c.dataset.path)"""


OPEN_DIALOGS = "return [...document.querySelectorAll('dialog[open], [aria-modal=true]')].map((d) => d.getAttribute('aria-label') || d.textContent.slice(0, 40))"


BUTTONS = "[...document.querySelectorAll('[aria-modal=true] button')].map((b) => b.textContent.trim())"
DONE_SHOWN = f"return {BUTTONS}.includes('Done')"
EXPORT_SHOWN = f"return {BUTTONS}.some((t) => t.startsWith('Export'))"


def seed(r):
    make(r, 'talk.nuzky', 'Talk to camera', [FIXTURES / 'talk.mp4'], 0)
    make(r, 'reel.nuzky', 'Morning reel', [FIXTURES / 'portrait.mp4'], 1)


def shown(r):
    """What view(r) says, or None once the window is gone."""
    try:
        return view(r)
    except RuntimeError:
        return None


def running(r):
    return (r.app.poll() is None) if MACOS else app_pid(r) is not None


@flow('close-window', "In the editor the window's close button goes back to the projects and Nuzky keeps running, "
      'the project still open; on the home screen it quits', before=seed)
def close_window_flow(r):
    start = view(r)
    r.check('the flow starts in the editor of the newest project', start['view'] == 'editor' and start['name'] == 'Talk to camera', start)
    r.s.run('document.activeElement?.blur()')
    r.key('k', ctrlKey=True)
    r.check('Ctrl+K opens the launcher over the editor', wait(lambda: view(r)['launcher'], 5))

    close_window()
    r.check('closing the window in the editor shows the projects and closes the launcher',
            wait(lambda: (v := shown(r)) and v['view'] == 'home' and not v['launcher'], 10), shown(r))
    time.sleep(1)
    r.check('and Nuzky keeps running', running(r))
    r.check('the project stays open: its card says Open now',
            wait(lambda: r.s.run(OPEN_CARD) == [start['path']], 10), r.s.run(OPEN_CARD))
    r.shot('home-after-close')

    r.s.run("[...document.querySelectorAll('button')].find((b) => b.title === 'Back to the editor (Esc)').click()")
    r.check('Back returns to the same project', wait(lambda: view(r)['view'] == 'editor' and view(r)['path'] == start['path'], 10), view(r))
    time.sleep(0.5)
    close_window()
    r.check('closing again from the editor goes back to the projects again', wait(lambda: view(r)['view'] == 'home', 10), view(r))

    # The editor's dialogs go with it, so they do not cover the projects. A finished export, as the dialog
    # starts it, shows Done once; closing the window counts as closing the dialog.
    r.s.run("window.__nuzky.store.setState({view: 'editor'})")
    started = r.s.call("window.__nuzky.api.startExport(arguments[0], {resolution: 720, fps: 30, quality: 'small'}, "
                       "window.__nuzky.store.getState().snap.sessionEpoch, false)"
                       ".then((job) => (window.__nuzky.store.setState({exportJobId: job, exportOpen: true}), job))",
                       str(r.work / 'closed.mp4'))
    r.check('an export from the dialog finishes and the dialog says Done',
            started['ok'] and wait(lambda: r.s.run(DONE_SHOWN), 120), started)
    close_window()
    r.check('closing the window with Export open shows the projects without the dialog',
            wait(lambda: view(r)['view'] == 'home' and not r.s.run(OPEN_DIALOGS), 10), [view(r), r.s.run(OPEN_DIALOGS)])
    r.s.run("[...document.querySelectorAll('button')].find((b) => b.title === 'Back to the editor (Esc)').click()")
    r.s.run("window.__nuzky.store.setState({exportOpen: true})")
    r.check('Export opened again starts a new export instead of showing the finished one',
            wait(lambda: r.s.run(OPEN_DIALOGS), 5) and wait(lambda: r.s.run(EXPORT_SHOWN), 5) and not r.s.run(DONE_SHOWN), r.s.run(OPEN_DIALOGS))
    r.s.run("window.__nuzky.store.setState({exportOpen: false})")

    # A reloaded page starts on the projects, so the next close quits even though the editor was shown before.
    r.s.run("window.__nuzky.store.setState({view: 'editor'})")
    time.sleep(0.5)
    r.s.run('window.__oldPage = true; location.reload()')
    r.check('after a reload the page starts on the projects',
            wait(lambda: r.s.run("return !window.__oldPage && window.__nuzky?.store.getState().snap ? window.__nuzky.store.getState().view : null", retries=3) == 'home', 30))
    time.sleep(0.5)

    close_window()
    r.check('closing the window on the home screen quits Nuzky', wait(lambda: not running(r), 15))

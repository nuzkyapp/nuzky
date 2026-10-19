"""The window's close button. In the editor it goes back to the projects with the project still open and Nuzky
running; on the home screen it quits."""
import time

from e2e.diagnostics import app_pid
from e2e.harness import FIXTURES, MACOS, close_window, flow, wait
from e2e.home import make, view

OPEN_CARD = """return [...document.querySelectorAll('[role=listbox][aria-label=Projects] [role=option][data-path]')]
  .filter((c) => c.querySelector('[data-meta]')?.textContent.includes('Open now')).map((c) => c.dataset.path)"""


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
    time.sleep(0.5)

    close_window()
    r.check('closing the window on the home screen quits Nuzky', wait(lambda: not running(r), 15))

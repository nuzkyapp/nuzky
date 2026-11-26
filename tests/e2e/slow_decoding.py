import os
import signal
import subprocess
import time

from e2e.harness import APP, CLI, WINDOWS, changed_share, flow, preview_crop, preview_rect, wait

# The frame just before the second keyframe, so the preview decodes almost ten seconds of video to show it.
TARGET_US = 9_900_000


def footage(path):
    """Full HD with a keyframe every ten seconds, which no preview copy replaces; grain makes each frame real work."""
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=s=1920x1080:r=60:d=12,noise=alls=4:allf=t',
                    '-c:v', 'libx264', '-preset', 'veryfast', '-g', '600', '-keyint_min', '600', '-sc_threshold', '0',
                    '-pix_fmt', 'yuv420p', str(path)], check=True)


def app_pid(r):
    """The app's process: started by the harness on macOS and Windows, by the WebDriver on Linux."""
    if r.app:
        return r.app.pid
    found = subprocess.run(['pgrep', '-n', '-f', str(APP)], capture_output=True, text=True).stdout.split()
    if not found:
        raise RuntimeError('the app process is gone')
    return int(found[0])


def suspended(pid, seconds):
    """Stops the app as a sleeping laptop or a starved process does: its clock goes on while none of it runs."""
    if WINDOWS:
        import ctypes
        kernel, ntdll = ctypes.windll.kernel32, ctypes.windll.ntdll
        handle = kernel.OpenProcess(0x0800, False, pid)
        ntdll.NtSuspendProcess(handle)
        time.sleep(seconds)
        ntdll.NtResumeProcess(handle)
        kernel.CloseHandle(handle)
        return
    os.kill(pid, signal.SIGSTOP)
    try:
        time.sleep(seconds)
    finally:
        os.kill(pid, signal.SIGCONT)


def shown(r, name, size):
    r.shot(name)
    return preview_crop(r.work / f'{name}.png', preview_rect(r)).resize(size)


@flow('slow_decoding', 'A paused frame that takes longer than 8 s to decode, here because the app is suspended in the '
      'middle of it as on a sleeping laptop, still shows once decoded, without an error')
def slow_decoding(r):
    video = r.work / 'long-gop.mp4'
    footage(video)
    r.import_media(video)
    r.add_clip(video.name)
    time.sleep(1.5)
    reference = r.work / 'reference.png'
    subprocess.run([str(CLI), 'frame', str(r.saved_project()), str(TARGET_US / 1e6), str(reference), '360'], env=r.env,
                   check=True, capture_output=True)
    first = shown(r, 'first-frame', (360, 640))
    r.check('the preview shows the clip before the jump', changed_share(first, reference) > 0.005,
            f'{changed_share(first, reference):.2%} of pixels differ from the frame at 9.9 s')

    # The preview starts decoding as soon as it reports the new playhead; the app stops in the middle of it.
    r.seek(TARGET_US)
    reported = wait(lambda: r.s.run('return window.__nuzky.store.getState().timeUs') == TARGET_US, 5, 0.01)
    r.check('the preview took the jump to 9.9 s', reported)
    suspended(app_pid(r), 10)

    drawn = wait(lambda: changed_share(shown(r, 'after-pause', (360, 640)), reference) < 0.02, 20, 0.5)
    changed = changed_share(shown(r, 'after-pause', (360, 640)), reference)
    r.check('once the app runs again the preview shows the frame at 9.9 s, as export draws it', drawn,
            f'{changed:.2%} of pixels differ')
    r.check('the preview shows no error', r.s.run('return window.__nuzky.store.getState().engineError') is None,
            r.s.run('return window.__nuzky.store.getState().engineError'))
    # The app's own log: app.log where the harness starts it, driver.log where the WebDriver does.
    log = ''.join(p.read_text(errors='replace') for p in (r.work / 'app.log', r.work / 'driver.log') if p.exists())
    r.check('the preview did not give up on the frame', 'Preview render failed' not in log,
            [line for line in log.splitlines() if 'Preview render failed' in line][:3])
    r.check('nothing shows an error', not r.errors(), r.errors())

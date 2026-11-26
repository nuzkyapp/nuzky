import os
import signal
import subprocess
import time
from pathlib import Path

from e2e.harness import APP, CLI, MACOS, WINDOWS, changed_share, flow, preview_crop, preview_rect, wait

# The frame just before the second keyframe, so the preview decodes almost ten seconds of video to show it.
TARGET_US = 9_900_000


def footage(path):
    """Full HD with a keyframe every ten seconds, which no preview copy replaces; grain makes each frame real work, and
    the picture brightens through the clip, so the frame at 9.9 s looks nothing like the first."""
    picture = "testsrc2=s=1920x1080:r=60:d=12,noise=alls=4:allf=t,eq=brightness='0.05*t-0.3':eval=frame"
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', picture, '-c:v', 'libx264', '-preset', 'veryfast',
                    '-g', '600', '-keyint_min', '600', '-sc_threshold', '0', '-pix_fmt', 'yuv420p', str(path)], check=True)


def app_pid(r):
    """The app's process: started by the harness on macOS and Windows, on Linux by this run's own WebDriver."""
    if r.app:
        return r.app.pid
    found, queue = None, [r.driver.pid]
    while queue and not found:
        pid = queue.pop()
        children = subprocess.run(['pgrep', '-P', str(pid)], capture_output=True, text=True).stdout.split()
        for child in map(int, children):
            if Path(f'/proc/{child}/exe').resolve() == APP.resolve():
                found = child
            queue.append(child)
    if not found:
        raise RuntimeError('the app process of this run is gone')
    return found


def cpu_seconds(pid):
    """CPU time the process has used so far."""
    if WINDOWS:
        import ctypes
        from ctypes import wintypes
        kernel = ctypes.windll.kernel32
        handle = kernel.OpenProcess(0x1000, False, pid)
        times = [wintypes.FILETIME() for _ in range(4)]
        ok = handle and kernel.GetProcessTimes(handle, *[ctypes.byref(t) for t in times])
        kernel.CloseHandle(handle)
        if not ok:
            raise RuntimeError(f'cannot read the CPU time of {pid}')
        return sum((t.dwHighDateTime << 32 | t.dwLowDateTime) / 1e7 for t in times[2:])
    if MACOS:
        minutes, seconds = subprocess.run(['ps', '-o', 'cputime=', '-p', str(pid)], capture_output=True,
                                          text=True).stdout.strip().split(':')[-2:]
        return int(minutes) * 60 + float(seconds)
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return (int(fields[11]) + int(fields[12])) / os.sysconf('SC_CLK_TCK')


def suspended(pid, seconds):
    """Stops the app as a sleeping laptop or a starved process does: its clock goes on while none of it runs."""
    if WINDOWS:
        import ctypes
        kernel, ntdll = ctypes.windll.kernel32, ctypes.windll.ntdll
        handle = kernel.OpenProcess(0x0800, False, pid)
        if not handle or ntdll.NtSuspendProcess(handle) != 0:
            raise RuntimeError(f'cannot suspend {pid}')
        try:
            time.sleep(seconds)
        finally:
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


def clip_part(picture):
    """The middle of the wide clip in the 360×640 portrait frame, without the black bars above and below it."""
    from PIL import Image
    picture = Image.open(picture) if isinstance(picture, Path) else picture
    return picture.crop((0, 240, 360, 400))


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
    first = clip_part(shown(r, 'first-frame', (360, 640)))
    reference = clip_part(reference)
    r.check('the preview shows the clip before the jump, nothing like the frame at 9.9 s',
            changed_share(first, reference) > 0.5, f'{changed_share(first, reference):.2%} of pixels differ')

    # Once the app works on the jump, which is what spends its CPU while the preview is paused, it stops in the
    # middle of decoding.
    pid = app_pid(r)
    idle = cpu_seconds(pid)
    r.seek(TARGET_US)
    decoding = wait(lambda: cpu_seconds(pid) - idle >= 0.1, 5, 0.01)
    r.check('the app started decoding the jump to 9.9 s', decoding, f'{cpu_seconds(pid) - idle:.2f} s of CPU')
    suspended(pid, 10)

    drawn = wait(lambda: changed_share(clip_part(shown(r, 'after-pause', (360, 640))), reference) < 0.02, 20, 0.5)
    changed = changed_share(clip_part(shown(r, 'after-pause', (360, 640))), reference)
    r.check('once the app runs again the preview shows the frame at 9.9 s, as export draws it', drawn,
            f'{changed:.2%} of pixels differ')
    r.check('the preview shows no error', r.s.run('return window.__nuzky.store.getState().engineError') is None,
            r.s.run('return window.__nuzky.store.getState().engineError'))
    # The app's own log: app.log where the harness starts it, driver.log where the WebDriver does.
    log = ''.join(p.read_text(errors='replace') for p in (r.work / 'app.log', r.work / 'driver.log') if p.exists())
    r.check('the preview did not give up on the frame', 'Preview render failed' not in log,
            [line for line in log.splitlines() if 'Preview render failed' in line][:3])
    r.check('nothing shows an error', not r.errors(), r.errors())

"""An iPhone from the 16 on records Apple spatial audio, which FFmpeg cannot decode, beside stereo AAC, and can mark
the spatial track as the default. Such a video imports with its stereo sound prepared, without an error, draws its
waveform, and exports that sound where FFmpeg's own decoding of it has it. A video whose only sound is spatial is
added without sound, and a toast says why. The spatial track is a 4-channel PCM track whose sample entry is renamed
to `apac`, so FFmpeg reads it as a sound it has no decoder for."""
import subprocess

import numpy as np

from e2e.harness import export, flow, wait

RATE = 48_000
TONE = 'if(lt(t\\,1)\\,0\\,0.25*sin(2*PI*440*t))'
TALLEST = """const clip = document.querySelector(`[data-clip-id='${arguments[0]}']`);
let tallest = 0;
for (const canvas of clip?.querySelectorAll('canvas') ?? []) {
  if (!canvas.width || !canvas.height) continue;
  const pixels = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data;
  for (let x = 0; x < canvas.width; x++) {
    let drawn = 0;
    for (let y = 0; y < canvas.height; y++) drawn += pixels[(y * canvas.width + x) * 4 + 3] > 0;
    tallest = Math.max(tallest, drawn / canvas.height);
  }
}
return tallest;"""
# Waveform.tsx draws a bar RANGE_DB under full scale as empty; the tone at -12 dBFS rises to 1 - 12 / 42 of it.
TONE_BAR = 1 - 12 / 42
ASSET = "return window.__nuzky.store.getState().snap.project.assets.find((a) => a.name === arguments[0]) ?? null"
AUDIO_JOB = "return window.__nuzky.store.getState().jobs[`audio:${arguments[0]}`] ?? null"
SHOWN = """return [...document.querySelectorAll('[data-toast]')].some((e) => e.textContent.includes(arguments[0])
  && e.getBoundingClientRect().height > 0 && e.getAnimations().every((a) => a.playState === 'finished'))"""


def iphone_video(path, stereo):
    """A portrait video with a 4-channel spatial track marked as the default, after a stereo AAC track that is not."""
    made = path.with_suffix('.pcm.mov')
    args = ['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=s=360x640:r=30:d=4',
            '-f', 'lavfi', '-i', f'aevalsrc=0.25*sin(2*PI*880*t)|0.25*sin(2*PI*880*t)|0|0:s={RATE}:d=4']
    if stereo:
        args += ['-f', 'lavfi', '-i', f'aevalsrc={TONE}|{TONE}:s={RATE}:d=4', '-map', '0:v', '-map', '2:a', '-map', '1:a',
                 '-c:a:0', 'aac', '-c:a:1', 'pcm_s16le', '-disposition:a:0', '0', '-disposition:a:1', 'default']
    else:
        args += ['-map', '0:v', '-map', '1:a', '-c:a', 'pcm_s16le']
    subprocess.run(args + ['-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', str(made)], check=True)
    data = bytearray(made.read_bytes())
    if data.count(b'lpcm') != 1:
        raise RuntimeError(f'{made.name} should have one lpcm sample entry, has {data.count(b"lpcm")}')
    at = data.find(b'lpcm')
    data[at:at + 4] = b'apac'
    path.write_bytes(data)
    made.unlink()
    return path


def sound(path, stream):
    """The left channel of a file's sound stream, decoded by FFmpeg at 48 kHz."""
    raw = subprocess.run(['ffmpeg', '-v', 'error', '-i', str(path), '-map', stream, '-ac', '2', '-ar', str(RATE),
                          '-f', 'f32le', '-'], capture_output=True, check=True).stdout
    return np.frombuffer(raw, dtype='<f4').reshape(-1, 2)[:, 0]


def onset(samples):
    return int(np.argmax(np.abs(samples) > 0.02))


def level_db(samples, hz, at, seconds):
    """Level in dBFS of the `hz` tone from `at` for `seconds`."""
    part = samples[int(at * RATE):int((at + seconds) * RATE)]
    phase = 2 * np.pi * hz * np.arange(len(part)) / RATE
    return 20 * np.log10(2 * abs(np.sum(part * np.exp(-1j * phase))) / len(part) + 1e-12)


@flow('spatial_audio', 'An iPhone video with Apple spatial audio beside its stereo track plays and exports the stereo sound in step with the picture; one whose only sound is spatial is added without sound and says why')
def spatial_audio(r):
    source = iphone_video(r.work / 'IMG_0420.MOV', stereo=True)
    r.import_media(source)
    r.add_clip(source.name)
    asset = r.s.run(ASSET, source.name)
    r.check('the video has sound', asset['hasAudio'], asset)
    job = wait(lambda: (j := r.s.run(AUDIO_JOB, asset['id'])) and j['status'] != 'running' and j, 60) or r.s.run(AUDIO_JOB, asset['id'])
    r.check('its sound is prepared', job and job['status'] == 'done', job)
    clip = r.track()[0]
    tallest = wait(lambda: (t := r.s.run(TALLEST, clip['id'])) > TONE_BAR - 0.1 and t, 15) or r.s.run(TALLEST, clip['id'])
    r.check('the clip draws the waveform of its stereo sound', abs(tallest - TONE_BAR) < 0.1,
            f'the tallest bar fills {tallest:.2f} of its strip, the tone should fill {TONE_BAR:.2f}')
    r.check('no error toast', not r.errors(), r.errors())
    r.shot('stereo-and-spatial')

    out = export(r, 'iphone.mp4')
    if out.exists():
        reference, exported = sound(source, '0:a:0'), sound(out, '0:a')
        heard, spatial = level_db(exported, 440, 2, 1), level_db(exported, 880, 2, 1)
        r.check('the export plays the stereo track and not the spatial one', abs(heard + 12) < 0.5 and spatial < -60,
                f'440 Hz {heard:.1f} dBFS, 880 Hz {spatial:.1f} dBFS')
        r.check('the exported sound starts where FFmpeg decodes it in the source, within 1 ms',
                abs(onset(exported) - onset(reference)) <= RATE // 1000, f'{onset(exported)} vs {onset(reference)} samples')

    only = iphone_video(r.work / 'IMG_0421.MOV', stereo=False)
    r.import_media(only)
    asset = r.s.run(ASSET, only.name)
    r.check('a video whose only sound is spatial is added without sound', asset and not asset['hasAudio'], asset)
    warning = wait(lambda: next((t['text'] for t in r.state()['toasts'] if t['kind'] == 'warning'), None), 10)
    r.check('a toast says it is added without sound and why',
            warning == 'IMG_0421.MOV is added without sound. Nuzky cannot play Apple spatial audio, the only sound in '
                       'this file. Export it with stereo sound from the app it came from.', warning)
    r.check('and no error toast', not r.errors(), r.errors())
    r.check('the toast is on screen', wait(lambda: r.s.run(SHOWN, 'added without sound'), 5))
    r.shot('spatial-only')

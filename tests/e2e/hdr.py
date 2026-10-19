import math
import subprocess
import time

from e2e.harness import CLI, flow, preview_crop, preview_rect

# HLG signal values of a phone shot in a 2 x 4 grid: greys at 50 %, at HDR reference white (75 %) and in the
# highlights above it, skin, a lit wall, and a face lit by a window in an iPhone clip, where a tenth of it is
# brighter than the cheek and the forehead is the brightest. Phones put faces and walls around and above
# reference white.
PATCHES = {
    'grey 50 %': (0.5, 0.5, 0.5), 'grey 75 %': (0.75, 0.75, 0.75),
    'grey 85 %': (0.85, 0.85, 0.85), 'grey 95 %': (0.95, 0.95, 0.95),
    'skin': (0.62, 0.55, 0.50), 'wall': (0.85, 0.83, 0.76),
    'lit cheek': (0.90, 0.79, 0.75), 'lit forehead': (0.94, 0.89, 0.82),
}
ROWS = len(PATCHES) // 2
WIDTH, HEIGHT = 360, 640


def centre(crop, index):
    """Mean colour of the middle of grid cell `index` (row by row) in a crop of the picture."""
    column, row = index % 2, index // 2
    w, h = crop.width / 2, crop.height / ROWS
    box = crop.crop((int((column + 0.3) * w), int((row + 0.3) * h), int((column + 0.7) * w), int((row + 0.7) * h)))
    pixels = list(box.getdata())
    return [sum(p[c] for p in pixels) / len(pixels) for c in range(3)]


def hlg_to_pq(e):
    """The same light as PQ, as an Android phone records HDR10: HLG on its nominal 1000 nit display, then the PQ curve."""
    scene = [v * v / 3 if v <= 0.5 else (math.exp((v - 0.55991073) / 0.17883277) + 0.28466892) / 12 for v in e]
    luma = 0.2627 * scene[0] + 0.678 * scene[1] + 0.0593 * scene[2]
    pq = []
    for s in scene:
        y = (luma ** 0.2 * s / 10) ** 0.1593017578125
        pq.append(((0.8359375 + 18.8515625 * y) / (1 + 18.6875 * y)) ** 78.84375)
    return pq


def phone_clip(r, name, transfer, values):
    """A 2 s HEVC clip of the patches with the given HDR transfer, as a phone records it."""
    pixel = lambda patch: b''.join(round(v * 65535).to_bytes(2, 'big') for v in patch)
    rows = [(pixel(values[2 * band]) * (WIDTH // 2) + pixel(values[2 * band + 1]) * (WIDTH // 2)) * (HEIGHT // ROWS + 1)
            for band in range(ROWS)]
    picture = b''.join(rows)[:WIDTH * HEIGHT * 6]
    (r.work / f'{name}.ppm').write_bytes(f'P6\n{WIDTH} {HEIGHT}\n65535\n'.encode() + picture)
    clip = r.work / f'{name}.mov'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-loop', '1', '-i', str(r.work / f'{name}.ppm'), '-t', '2', '-r', '30', '-vf',
                    'scale=in_range=pc:out_color_matrix=bt2020nc:out_range=tv,format=yuv420p10le', '-c:v', 'libx265',
                    '-x265-params', f'log-level=error:crf=4:colorprim=bt2020:transfer={transfer}:colormatrix=bt2020nc',
                    '-tag:v', 'hvc1', str(clip)], check=True)
    return clip


def shown_patches(r, shot, at_us, names):
    r.seek(at_us)
    time.sleep(1.5)
    r.shot(shot)
    crop = preview_crop(r.work / f'{shot}.png', preview_rect(r))
    return {name: centre(crop, i) for i, name in enumerate(names)}


@flow('hdr', "Phone HDR, HLG and PQ, shows in the preview as Apple's SDR conversion shows HLG, the same as the renderer, "
             'with walls and highlights not blown out')
def hdr(r):
    hlg = phone_clip(r, 'hlg-phone', 'arib-std-b67', list(PATCHES.values()))
    # The PQ clip holds the patches in reverse order, so a preview still showing the HLG clip cannot pass for it.
    reversed_names = list(reversed(PATCHES))
    pq = phone_clip(r, 'pq-phone', 'smpte2084', [hlg_to_pq(PATCHES[name]) for name in reversed_names])
    r.import_media(hlg, pq)
    r.add_clip(hlg.name)
    r.add_clip(pq.name)
    preview = shown_patches(r, 'hdr-preview', 1_000_000, list(PATCHES))
    rendered_png = r.work / 'hdr-rendered.png'
    subprocess.run([str(CLI), 'frame', str(r.saved_project()), '1', str(rendered_png), str(WIDTH)], env=r.env, check=True,
                   capture_output=True)
    from PIL import Image
    rendered = Image.open(rendered_png).convert('RGB')
    off = max(abs(a - b) for i, name in enumerate(PATCHES) for a, b in zip(preview[name], centre(rendered, i)))
    r.check('the preview shows the same colours as the renderer', off <= 8, f'{off:.0f} levels apart')
    # Apple's SDR conversion (avconvert -p PresetHighestQuality, macOS 27.0.1) shows the 75 % grey at 155.
    grey = preview['grey 75 %'][1]
    r.check("HDR reference white shows as Apple's light grey (155)", abs(grey - 155) <= 8, round(grey))
    r.check('the lit wall keeps its colour instead of turning white', max(preview['wall']) < 235,
            [round(v) for v in preview['wall']])
    step = preview['grey 95 %'][1] - preview['grey 85 %'][1]
    r.check('highlights above reference white stay distinct', step > 20, f'{step:.0f} levels between 85 % and 95 %')
    # The red of lit skin used to reach white: the cheek showed at 254 and the forehead at 255, flat orange.
    cheek, forehead = preview['lit cheek'], preview['lit forehead']
    r.check('lit skin keeps its shades instead of clipping', max(forehead) < 248 and forehead[0] - cheek[0] >= 4,
            f'cheek {[round(v) for v in cheek]}, forehead {[round(v) for v in forehead]}')
    # The PQ clip comes after the HLG one. The preview decodes PQ with a little noise and shows the lit cheek's red
    # a few levels lower than the renderer draws it, as it did before PQ showed like HLG; the renderer's own frames
    # of the two clips are within 4 levels.
    pq_preview = shown_patches(r, 'hdr-pq-preview', 3_000_000, reversed_names)
    off, worst = max((max(abs(a - b) for a, b in zip(pq_preview[name], preview[name])), name) for name in PATCHES)
    r.check('the same scene recorded as PQ shows as the HLG clip does', off <= 12,
            f'{off:.0f} levels apart at {worst}: PQ {[round(v) for v in pq_preview[worst]]}, '
            f'HLG {[round(v) for v in preview[worst]]}')
    grey = pq_preview['grey 75 %'][1]
    r.check("PQ's HDR reference white also shows as Apple's light grey (155)", abs(grey - 155) <= 8, round(grey))
    r.check('nothing shows an error', not r.errors(), r.errors())

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


@flow('hdr', 'A phone HLG clip shows in the preview tone mapped like the renderer, with walls and highlights not blown out')
def hdr(r):
    values = list(PATCHES.values())
    pixel = lambda patch: b''.join(round(v * 65535).to_bytes(2, 'big') for v in patch)
    rows = [(pixel(values[2 * band]) * (WIDTH // 2) + pixel(values[2 * band + 1]) * (WIDTH // 2)) * (HEIGHT // ROWS + 1)
            for band in range(ROWS)]
    picture = b''.join(rows)[:WIDTH * HEIGHT * 6]
    (r.work / 'hlg.ppm').write_bytes(f'P6\n{WIDTH} {HEIGHT}\n65535\n'.encode() + picture)
    clip = r.work / 'hlg-phone.mov'
    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-loop', '1', '-i', str(r.work / 'hlg.ppm'), '-t', '2', '-r', '30', '-vf',
                    'scale=in_range=pc:out_color_matrix=bt2020nc:out_range=tv,format=yuv420p10le', '-c:v', 'libx265',
                    '-x265-params', 'log-level=error:crf=4:colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc',
                    '-tag:v', 'hvc1', str(clip)], check=True)
    r.import_media(clip)
    r.add_clip(clip.name)
    r.seek(1_000_000)
    time.sleep(1.5)
    r.shot('hdr-preview')
    shown = preview_crop(r.work / 'hdr-preview.png', preview_rect(r))
    preview = {name: centre(shown, i) for i, name in enumerate(PATCHES)}
    rendered_png = r.work / 'hdr-rendered.png'
    subprocess.run([str(CLI), 'frame', str(r.saved_project()), '1', str(rendered_png), str(WIDTH)], env=r.env, check=True,
                   capture_output=True)
    from PIL import Image
    rendered = Image.open(rendered_png).convert('RGB')
    off = max(abs(a - b) for i, name in enumerate(PATCHES) for a, b in zip(preview[name], centre(rendered, i)))
    r.check('the preview shows the same colours as the renderer', off <= 8, f'{off:.0f} levels apart')
    # The old curve showed the 75 % grey at 244 and the wall and every highlight at 255.
    grey = preview['grey 75 %'][1]
    r.check('HDR reference white shows as a light grey, not white', 150 <= grey <= 200, round(grey))
    r.check('the lit wall keeps its colour instead of turning white', max(preview['wall']) < 235,
            [round(v) for v in preview['wall']])
    step = preview['grey 95 %'][1] - preview['grey 85 %'][1]
    r.check('highlights above reference white stay distinct', step > 20, f'{step:.0f} levels between 85 % and 95 %')
    # The red of lit skin used to reach white: the cheek showed at 254 and the forehead at 255, flat orange.
    cheek, forehead = preview['lit cheek'], preview['lit forehead']
    r.check('lit skin keeps its shades instead of clipping', max(forehead) < 248 and forehead[0] - cheek[0] >= 4,
            f'cheek {[round(v) for v in cheek]}, forehead {[round(v) for v in forehead]}')
    r.check('nothing shows an error', not r.errors(), r.errors())

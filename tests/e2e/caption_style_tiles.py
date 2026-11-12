"""Caption style tiles show how each style behaves, not just its name: a sample of two captions drawn in the style,
still at rest with the word being said lit and the key words coloured, and playing while pointed at: each caption
enters with the style's animation and karaoke lights the words one after another as they are said."""
import json, time

from e2e.harness import flow, wait, webdriver

TILE = "[...document.querySelectorAll('[data-style-tile]')].find((b) => b.textContent.trim() === arguments[0])"
SAMPLE = f"""const tile = {TILE};
const s = tile?.parentElement.querySelector('[data-caption-sample]');
return s ? {{text: s.textContent, lit: [...s.querySelectorAll('span')].map((w) => [w.textContent, getComputedStyle(w).color]),
  animation: s.style.animationName || null}} : null;"""
OVER = TILE + ".dispatchEvent(new PointerEvent('pointerover', {bubbles: true, pointerType: 'mouse'}))"
OUT = TILE + ".dispatchEvent(new PointerEvent('pointerout', {bubbles: true, pointerType: 'mouse', relatedTarget: document.body}))"
TILES = """return [...document.querySelectorAll('[data-style-tile]')].map((b) =>
  [b.textContent.trim(), b.parentElement.querySelector('[data-caption-sample]')?.textContent ?? null]);"""
ADD_CAPTIONS = """window.__nuzky.store.getState().edit({type: 'addCaptions', style: arguments[0],
  segments: ['no a pak', 'tohle je fakt'].map((text, i) => ({startUs: i * 1000000, endUs: (i + 1) * 1000000, text}))})"""
REEL = {'fontSize': 95, 'color': '#ffffff', 'bold': False, 'strokeWidth': 7.5, 'strokeColor': '#000000', 'background': None}
CLIPPED = """return [...document.querySelectorAll(arguments[0] + ' [data-caption-sample]')]
  .filter((s) => s.scrollWidth > s.clientWidth + 1 || s.getBoundingClientRect().width > s.parentElement.clientWidth + 1)
  .map((s) => s.parentElement.parentElement.querySelector('[data-style-tile]').textContent.trim());"""
YELLOW = 'rgb(255, 225, 77)'
GOLD = 'rgb(255, 212, 0)'


def window(r, width, height):
    webdriver('POST', r.s.path + '/window/rect', {'width': width, 'height': height})
    wait(lambda: r.s.run('return window.innerWidth') == width, 5)
    time.sleep(1)


def no_sample_cut_off(r, place, where):
    """Checks every sample in `place` fits its tile at the usual window and the smallest, and ends at the usual one."""
    for width, height in ((1024, 640), (1440, 900)):
        window(r, width, height)
        clipped = r.s.run(CLIPPED, place)
        r.check(f'at {width} × {height} no sample is cut off in the {where}', clipped == [], clipped)
        if width == 1024:
            r.shot(f'{where.replace(" ", "-").lower()}-1024')


def sample(r, name):
    return r.s.run(SAMPLE, name)


@flow('caption-style-tiles', 'Caption style tiles play a sample of captions in the style: still with the spoken word lit and '
      'key words coloured at rest, entering with the style\'s animation and lighting each word as it is said while pointed at')
def caption_style_tiles(r):
    # A saved style with a second-long zoom in and slide up out, longer than a caption of the sample holds.
    saved = r.s.call("window.__nuzky.api.saveCaptionStyle(arguments[0])", {
        'name': 'Slow slide', 'style': REEL, 'animIn': {'kind': 'zoomIn', 'durationUs': 1_000_000},
        'animOut': {'kind': 'slideUp', 'durationUs': 1_000_000}})
    r.check('a style with long animations is saved', saved.get('ok'), saved)
    r.s.run("[...document.querySelectorAll('[role=tab]')].find((t) => t.textContent.trim() === 'Captions')?.click()")
    tiles = wait(lambda: (t := r.s.run(TILES)) and len(t) >= 29 and t, 10)
    if not r.check('the Captions tab lists the 28 built-in styles and the saved one', tiles, tiles):
        return
    r.check('every tile is a button named by its style with a sample under it',
            all(name and text == 'I made $500' for name, text in tiles), [t for t in tiles if t[1] != 'I made $500'])
    # The bundled faces load once something is drawn in them; until then WebKit draws their text invisible.
    wait(lambda: r.s.run('return document.fonts.status') == 'loaded', 10)
    r.shot('captions-tab')
    no_sample_cut_off(r, 'aside[data-tour=library]', 'Captions tab')

    # At rest: the first caption fully in, its second word being said.
    rest = sample(r, 'Karaoke')
    r.check('at rest Karaoke lights the word being said in its yellow', rest and rest['lit'] == [['made', YELLOW]], rest)
    hormozi = sample(r, 'Hormozi')
    r.check('Hormozi colours the number, its key word, and lights nothing', hormozi and hormozi['lit'] == [['$500', GOLD]], hormozi)
    headline = sample(r, 'Headline')
    r.check('Headline colours the longest word', headline and [w for w, _ in headline['lit']] == ['made'], headline)
    reel = sample(r, 'Reel')
    r.check('Reel, without karaoke or key words, colours nothing', reel and reel['lit'] == [], reel)
    r.check('a tile at rest does not move', all(s and s['animation'] is None for s in (rest, hormozi, reel)))

    # Pointed at, Karaoke lights each word in turn, the first caption's then the second's.
    r.s.run(TILE + ".scrollIntoView({block: 'center'})", 'Karaoke')
    r.shot('styles-at-rest')
    r.s.run(OVER, 'Karaoke')
    seen, end = [], time.time() + 4.5
    while time.time() < end:
        s = sample(r, 'Karaoke')
        word = next((w for w, c in (s or {}).get('lit', []) if c == YELLOW), None)
        if word and (not seen or seen[-1] != word):
            seen.append(word)
        time.sleep(0.05)
    r.check('pointed at, Karaoke lights I, made, $500, then in, one, day', seen[:6] == ['I', 'made', '$500', 'in', 'one', 'day'], seen)
    r.shot('karaoke-playing')
    r.s.run(OUT, 'Karaoke')
    r.check('leaving it stops it at rest again', wait(lambda: sample(r, 'Karaoke') == rest, 3), sample(r, 'Karaoke'))

    # Hormozi pops each caption in.
    r.s.run(OVER, 'Hormozi')
    playing = wait(lambda: (s := sample(r, 'Hormozi')) and s['animation'] and s, 2)
    r.check('pointed at, Hormozi pops its caption in', playing and playing['animation'] == 'a-pop', playing)
    second = wait(lambda: (s := sample(r, 'Hormozi')) and s['text'] == 'in one day' and s, 4)
    r.check('and goes on to the second caption, popping it in too', second and second['animation'] == 'a-pop', second)
    r.s.run(OUT, 'Hormozi')

    # The saved style's caption zooms in whole, then slides on up and out, as the engine draws its exit.
    r.s.run(TILE + ".scrollIntoView({block: 'center'})", 'Slow slide')
    r.s.run(OVER, 'Slow slide')
    phases, start = [], time.time()
    while time.time() - start < 3.5:
        s = sample(r, 'Slow slide')
        name = s and s['animation']
        if name and (not phases or phases[-1][0] != name):
            phases.append((name, round(time.time() - start, 2)))
        time.sleep(0.05)
    r.s.run(OUT, 'Slow slide')
    names = [n for n, _ in phases]
    r.check('a saved style\'s caption zooms in, slides up and out, and the next one zooms in',
            names[:3] == ['a-zoomIn', 'a-slideUp-out', 'a-zoomIn'], phases)
    r.check('its exit waits until its second-long entry and its words are over',
            len(phases) > 1 and phases[1][1] >= 1.9, phases)

    # Frames of a tile playing, for a video of it.
    for name in ('Karaoke', 'Hormozi'):
        r.s.run(TILE + ".scrollIntoView({block: 'center'})", name)
        r.s.run(OVER, name)
        start, times = time.time(), []
        while time.time() - start < 4.5:
            times.append(time.time() - start)
            r.s.screenshot(r.work / f'play-{name.lower()}-{len(times):03d}.png')
        (r.work / f'play-{name.lower()}.json').write_text(json.dumps(times))
        r.s.run(OUT, name)

    # A caption's inspector offers the same tiles.
    r.s.run(ADD_CAPTIONS, REEL)
    first = wait(lambda: r.s.run("const t = window.__nuzky.store.getState().snap.project.tracks.find((t) => t.name === 'Captions'); return t?.clips[0]?.id"), 5)
    r.s.run("window.__nuzky.store.getState().select([arguments[0]])", first)
    inspector = "aside[aria-label=Inspector]"
    shown = wait(lambda: r.s.run(f"return document.querySelectorAll('{inspector} [data-caption-sample]').length"), 5)
    r.check("a caption's inspector shows the styles with their samples", shown and shown >= 28, shown)
    r.shot('inspector')
    no_sample_cut_off(r, 'aside[aria-label=Inspector]', 'inspector')

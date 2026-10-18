#!/usr/bin/env python3
"""Build assets/stickers from pinned Noto SVGs and English emoji metadata.

    python3 -I scripts/build-stickers.py

Downloads stay in ~/.cache/nuzky/deps/stickers (or $NUZKY_DEPS/stickers).
The standalone, locked Rust packer resolves SVGs with usvg 0.48.1 and compresses
whole SVGs into independent zstd-19 frames no larger than 4 MiB uncompressed.
"""
import hashlib, json, os, re, shutil, subprocess, sys, tarfile, tempfile, time, urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'assets' / 'stickers'
CACHE = Path(os.environ.get('NUZKY_DEPS', Path.home() / '.cache/nuzky/deps')) / 'stickers'
TARGET = ROOT / 'target' / 'sticker-pack'
NOTO = 'e20cbc2bbec1926686be9f9bee7d1d2cfa1fea0e'
METADATA = '173b9b26e8fcbcbe64e1ecbf073a21a96b95c6b1'
CLDR = '91c267402229a59e3ef2774544f001bf959e8809'
RAW = 'https://raw.githubusercontent.com/'
UNICODE = 'https://www.unicode.org/Public/18.0.0/emoji/emoji-test.txt'
INPUTS = {
    'noto.tar.gz': (f'https://codeload.github.com/googlefonts/noto-emoji/tar.gz/{NOTO}',
                    'faf0e6d0ec962501132657a007a6910a4869367e1a717dca206013a5ef55454f'),
    'ordering.json': (f'{RAW}googlefonts/emoji-metadata/{METADATA}/emoji_18_0_ordering.json',
                      '90b5212a08e14c6e1584e7f4b84ee8a0a071a089672ad2d62daf30635ccc3c40'),
    'annotations.json': (f'{RAW}unicode-org/cldr-json/{CLDR}/cldr-json/cldr-annotations-full/annotations/en/annotations.json',
                         'f22083cb86dffb63a643d5bacb5d9899f82d2fa5d388ad4f3aed72184acef505'),
    'derived.json': (f'{RAW}unicode-org/cldr-json/{CLDR}/cldr-json/cldr-annotations-derived-full/annotationsDerived/en/annotations.json',
                     '15c9457afa703a32dcc78d54626418a91e820fe0ddd67b7d491c4c4d9f5846ab'),
    'unicode-license.txt': (f'{RAW}unicode-org/cldr-json/{CLDR}/LICENSE',
                            '220ba0e1c43b99530d2d5bdb892a99dca0989414f51ab695ecd90163eaa1ec3b'),
    'apache-license.txt': (f'{RAW}googlefonts/emoji-metadata/{METADATA}/LICENSE',
                           'cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30'),
    'emoji-test.txt': (UNICODE, '8f3735cda1f92a779d78af67cf86066bb1f07143dc22f2ac29394d9bc57ab21a'),
}
SVG = '{http://www.w3.org/2000/svg}'
HREF = '{http://www.w3.org/1999/xlink}href'
ET.register_namespace('', SVG[1:-1])
ET.register_namespace('xlink', HREF[1:].split('}')[0])


def sha256(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def fetch(name):
    url, pinned = INPUTS[name]
    path = CACHE / name
    if not path.is_file():
        CACHE.mkdir(parents=True, exist_ok=True)
        request = urllib.request.Request(url, headers={'User-Agent': 'Nuzky-build/1 (https://nuzky.app)'})
        started = time.monotonic()
        with tempfile.NamedTemporaryFile(dir=CACHE, delete=False) as output:
            temporary = Path(output.name)
            try:
                print(f'Downloading {name}', flush=True)
                with urllib.request.urlopen(request, timeout=30) as response:
                    while chunk := response.read(1024 * 1024):
                        output.write(chunk)
                        if time.monotonic() - started > 120:
                            raise TimeoutError(f'{url}: download exceeded 120 s')
                output.close()
                if sha256(temporary) != pinned:
                    sys.exit(f'{url}: SHA-256 mismatch')
                temporary.replace(path)
            finally:
                temporary.unlink(missing_ok=True)
    if sha256(path) != pinned:
        sys.exit(f'{path}: SHA-256 mismatch; remove this cached download and retry')
    return path


def key(sequence):
    return '_'.join(f'{c:04x}' for c in sequence if c not in (0xfe0e, 0xfe0f))


def gradients(data, fix=False):
    root = ET.fromstring(data)
    nodes = {g.get('id'): g for g in root.iter() if g.tag in (SVG + 'linearGradient', SVG + 'radialGradient')}
    changed = 0
    for gradient in nodes.values():
        if gradient.get('gradientTransform') is not None:
            continue
        parent = gradient
        visited = set()
        while (href := parent.get(HREF, parent.get('href', ''))) and href.startswith('#') and href not in visited:
            visited.add(href)
            parent = nodes.get(href[1:])
            if parent is None:
                break
            if (transform := parent.get('gradientTransform')) is not None:
                changed += 1
                if fix:
                    gradient.set('gradientTransform', transform)
                break
    return (ET.tostring(root, encoding='utf-8') if changed and fix else data), changed


def catalog(inputs):
    qualified = {}
    group = None
    for line in inputs['emoji-test.txt'].read_text().splitlines():
        if line.startswith('# group:'):
            group = line.split(':', 1)[1].strip()
        match = re.match(r'^([0-9A-F ]+)\s*;\s*fully-qualified\s*#\s*\S+\s+E\S+\s+(.*)', line)
        if match:
            sequence = [int(c, 16) for c in match[1].split()]
            qualified[key(sequence)] = (sequence, match[2], group)
    annotations = {}
    for name in ('annotations.json', 'derived.json'):
        data = next(iter(json.loads(inputs[name].read_text()).values()))['annotations']
        annotations.update({key(map(ord, emoji)): fields for emoji, fields in data.items()})

    def entry(sequence):
        k = key(sequence)
        canonical, fallback, _ = qualified[k]
        annotation = annotations.get(k, {})
        return {'key': k, 'emoji': ''.join(map(chr, canonical)), 'name': annotation.get('tts', [fallback])[0]}

    own = json.loads((OUT / 'own' / 'manifest.json').read_text())
    groups = [{'name': 'Stickers', 'items': [
        {'key': s['id'], 'name': s['name'], 'keywords': s['keywords'], 'source': 'nuzky'} for s in own]}]
    seen = {}
    duplicates = 0
    for group in json.loads(inputs['ordering.json'].read_text()):
        items = []
        for record in group['emoji']:
            k = key(record['base'])
            shortcodes = [s.strip(':') for s in record['shortcodes']]
            if k in seen:
                duplicates += 1
                seen[k]['shortcodes'] = list(dict.fromkeys(seen[k]['shortcodes'] + shortcodes))
                continue
            item = entry(record['base'])
            item.update(keywords=annotations.get(k, {}).get('default', []), shortcodes=shortcodes, source='noto')
            seen[k] = item
            families = {k: item}
            for sequence in record['alternates']:
                alternate_key = key(sequence)
                if alternate_key in seen:
                    continue
                stem = key(c for c in sequence if not 0x1f3fb <= c <= 0x1f3ff)
                if stem not in families and stem in qualified:
                    family = entry(qualified[stem][0])
                    family.update(keywords=annotations.get(stem, {}).get('default', []), shortcodes=[], source='noto')
                    families[stem] = family
                    seen[stem] = family
                if alternate_key == stem:
                    continue
                alternate = entry(sequence)
                seen[alternate_key] = alternate
                families.get(stem, item).setdefault('alternates', []).append(alternate)
            for stem, family in families.items():
                tones = {}
                for alternate in family.get('alternates', []):
                    sequence = qualified[alternate['key']][0]
                    modifiers = {c for c in sequence if 0x1f3fb <= c <= 0x1f3ff}
                    if len(modifiers) == 1 and key(c for c in sequence if not 0x1f3fb <= c <= 0x1f3ff) == stem:
                        tones[next(iter(modifiers))] = alternate['key']
                if len(tones) == 5:
                    family['tones'] = [tones[t] for t in range(0x1f3fb, 0x1f400)]
            items.extend(families.values())
        groups.append({'name': group['group'], 'items': items})
    added = 0
    for k, (sequence, _, unicode_group) in qualified.items():
        if k in seen:
            continue
        item = entry(sequence)
        item.update(keywords=annotations.get(k, {}).get('default', []), shortcodes=[], source='noto')
        name = {'People & Body': 'People'}.get(unicode_group, unicode_group)
        next(g for g in groups if g['name'] == name)['items'].append(item)
        seen[k] = item
        added += 1
    assert len(qualified) == 3963 and set(seen) == set(qualified)
    assert sum(1 + len(i.get('alternates', [])) for g in groups[1:] for i in g['items']) == len(qualified)
    print(f'{len(qualified)} fully qualified sequences; {len(own)} own stickers; '
          f'{added} families added; {duplicates} repeated bases merged', flush=True)
    return groups, added


def art_sources(archive):
    files, links = {}, {}
    with tarfile.open(archive, 'r|gz') as tar:
        for member in tar:
            path = member.name.split('/', 1)[-1]
            if not (path.startswith(('svg/', '2D/svg/', 'third_party/region-flags/waved-svg/'))
                    or path == 'third_party/region-flags/LICENSE'):
                continue
            if member.isfile():
                files[path] = tar.extractfile(member).read()
            elif member.issym():
                links[path] = str(Path(path).parent / member.linkname)
    for path, destination in links.items():
        while destination in links:
            destination = links[destination]
        files[path] = files[destination]
    return files


def sources(own_revision, added):
    def source(id, name, url, revision, license, license_file, changes):
        return dict(id=id, name=name, url=url, revision=revision, license=license, licenseFile=license_file, changes=changes)
    packed = 'Minified with usvg 0.48.1 (coordinates 2, transforms 4 decimals); packed into zstd level 19 frames.'
    return [
        source('noto', 'Noto Emoji', 'https://github.com/googlefonts/noto-emoji', NOTO, 'Apache-2.0',
               'LICENSE-Apache-2.0.txt', '2D SVG artwork; inherited gradientTransform copied onto referencing gradients. ' + packed),
        source('region-flags', 'Noto region flags', 'https://github.com/googlefonts/noto-emoji/tree/' + NOTO + '/third_party/region-flags',
               NOTO, 'LicenseRef-public-domain', 'LICENSE-region-flags.txt', 'Waved SVG flags. ' + packed),
        source('emoji-metadata', 'Google Emoji Metadata', 'https://github.com/googlefonts/emoji-metadata', METADATA,
               'Apache-2.0', 'LICENSE-Apache-2.0.txt', 'Order, groups, alternates and shortcodes joined; repeated bases merged into their first occurrence.'),
        source('cldr', 'CLDR English annotations', 'https://github.com/unicode-org/cldr-json', CLDR,
               'Unicode-3.0', 'LICENSE-Unicode-3.0.txt', 'English names and keywords joined by sequence without presentation selectors.'),
        source('unicode', 'Unicode Emoji 18.0', UNICODE, '18.0.0', 'Unicode-3.0', 'LICENSE-Unicode-3.0.txt',
               f'Fully qualified sequences; {added} missing families added to People; names used when CLDR has no annotation.'),
        source('nuzky', 'Nuzky stickers', 'https://github.com/nuzkyapp/nuzky', own_revision,
               'CC0-1.0', 'CC0-1.0.txt', packed),
    ]


def main():
    inputs = {name: fetch(name) for name in INPUTS}
    groups, added = catalog(inputs)
    files = art_sources(inputs['noto.tar.gz'])
    TARGET.mkdir(parents=True, exist_ok=True)
    subprocess.run(['cargo', 'build', '--release', '--locked', '--manifest-path', str(ROOT / 'scripts/sticker-pack/Cargo.toml'),
                    '--target-dir', str(TARGET)], check=True, cwd=ROOT)
    with tempfile.TemporaryDirectory(prefix='build-', dir=TARGET) as directory:
        scratch = Path(directory)
        entries, paths = [], []
        fixed = 0
        own_hash = hashlib.sha256((OUT / 'own' / 'manifest.json').read_bytes())
        for group in groups:
            for item in group['items']:
                for entry in [item] + item.get('alternates', []):
                    k = entry['key']
                    if group['name'] == 'Stickers':
                        data = (OUT / 'own' / f'{k}.svg').read_bytes()
                        own_hash.update(data)
                    else:
                        filename = f'emoji_u{k}.svg'
                        flag = 'third_party/region-flags/waved-svg/' + filename
                        path = flag if flag in files else ('svg/' if 'svg/' + filename in files else '2D/svg/') + filename
                        entry['source'] = 'region-flags' if path == flag else 'noto'
                        data = files[path]
                    data, count = gradients(data, fix=True)
                    fixed += bool(count)
                    path = scratch / f'{k}.svg'
                    path.write_bytes(data)
                    entries.append(entry)
                    paths.append(str(path))
        plan = scratch / 'plan.json'
        plan.write_text(json.dumps(paths))
        pack, index = scratch / 'art.pack', scratch / 'index.json'
        subprocess.run([str(TARGET / 'release/sticker-pack'), str(plan), str(pack), str(index)], check=True, cwd=ROOT)
        index = json.loads(index.read_text())
        for entry, art in zip(entries, index['art'], strict=True):
            entry['art'] = art
        inherited = sum(gradients(p.read_bytes())[1] for p in (scratch / 'minified').glob('*.svg'))
        assert inherited == 0
        print(f'{fixed} SVGs fixed; packed SVGs with inherited gradientTransform: {inherited}', flush=True)
        result = dict(packSha256=sha256(pack), frames=index['frames'], sources=sources('sha256:' + own_hash.hexdigest(), added), groups=groups)
        catalog_path = scratch / 'catalog.json'
        catalog_path.write_text(json.dumps(result, ensure_ascii=False, separators=(',', ':')) + '\n', encoding='utf-8')
        print(f'art.pack: {pack.stat().st_size} bytes; catalog.json: {catalog_path.stat().st_size} bytes', flush=True)
        print(f'art.pack SHA-256: {sha256(pack)}\ncatalog.json SHA-256: {sha256(catalog_path)}', flush=True)
        shutil.copyfile(pack, OUT / 'art.pack')
        shutil.copyfile(catalog_path, OUT / 'catalog.json')
        shutil.copyfile(inputs['apache-license.txt'], OUT / 'LICENSE-Apache-2.0.txt')
        shutil.copyfile(inputs['unicode-license.txt'], OUT / 'LICENSE-Unicode-3.0.txt')
        (OUT / 'LICENSE-region-flags.txt').write_bytes(files['third_party/region-flags/LICENSE'])
        shutil.copyfile(ROOT / 'assets/sounds/CC0-1.0.txt', OUT / 'CC0-1.0.txt')


if __name__ == '__main__':
    main()

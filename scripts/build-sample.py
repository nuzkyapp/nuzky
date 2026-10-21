#!/usr/bin/env python3
"""Builds the sample video built into Nuzky, assets/sample, from its CC BY source on YouTube.

The tour for new users opens it as a project. yt-dlp downloads the source once: video format 137 (1080p H.264)
and audio format 140 (AAC), merged into one MP4, which is checked against its pinned SHA-256 and kept in
~/.cache/nuzky/deps/sample. FFmpeg then cuts out the minute the tour uses and encodes it smaller. manifest.json
records the source, author, licence, the attribution line and what was changed, and ATTRIBUTION.txt holds that
line for the installers. The output is committed; run this only after changing the source or the cut.

transcript.json is what Nuzky's own transcribe tool recognised in exactly these bytes (large-v3-turbo, word times
measured with wav2vec2), so the tour cuts and captions without a speech model. It needs the models to be made, so
this script cannot make it: run the transcribe tool of `nuzky mcp` on the new video in a project with its own
NUZKY_DATA_DIR, then copy the record it stored in <data>/transcripts here as transcript.json. A Rust test fails
while the transcript's fingerprint and duration do not match the video.

    python3 -I scripts/build-sample.py
"""
import hashlib, json, shutil, subprocess, sys, tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'assets' / 'sample'
CACHE = Path.home() / '.cache' / 'nuzky' / 'deps' / 'sample'
FILE = 'spotlight-effect.mp4'

VIDEO_ID = 'SfmKRW_bSHI'
PAGE = f'https://www.youtube.com/watch?v={VIDEO_ID}'
TITLE = 'Stop Performing and Unlock Confidence, Creativity & Calm (Solo Table Theory)'
AUTHOR = 'Dianne Glavaš'
LICENSE_URL = 'https://creativecommons.org/licenses/by/3.0/'
FORMATS = '137+140'
SOURCE_SHA256 = '427c2f1a98ef072419f834579d2ce89e10757134e0d49e1fc33326c855639cfa'

START, END = 175.40, 236.75
ENCODE = ['-vf', 'fps=30,scale=1920:1080', '-c:v', 'libx264', '-preset', 'slower', '-crf', '26', '-profile:v', 'high',
          '-pix_fmt', 'yuv420p', '-c:a', 'aac', '-b:a', '128k', '-movflags', '+faststart']


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def clock(seconds):
    return f'{int(seconds // 60)}:{seconds % 60:05.2f}'


def source():
    cached = CACHE / f'{SOURCE_SHA256}.mp4'
    if cached.is_file():
        return cached
    if not shutil.which('yt-dlp'):
        sys.exit('yt-dlp downloads the source: install it and run this again')
    CACHE.mkdir(parents=True, exist_ok=True)
    # Beside the cache, not in /tmp, which may live in memory.
    with tempfile.TemporaryDirectory(dir=CACHE) as scratch:
        subprocess.run(['yt-dlp', '--ignore-config', '--no-playlist', '-f', FORMATS, '--merge-output-format', 'mp4',
                        '-o', f'{scratch}/source.%(ext)s', PAGE], check=True)
        merged = Path(scratch) / 'source.mp4'
        got = sha256(merged)
        if got != SOURCE_SHA256:
            sys.exit(f'{PAGE} formats {FORMATS}: SHA-256 {got}, expected {SOURCE_SHA256}. The source changed: check '
                     'its licence and content again before pinning the new one.')
        merged.rename(cached)
    return cached


def main():
    src = source()
    out = OUT / FILE
    manifest_path = OUT / 'manifest.json'
    before = json.loads(manifest_path.read_text())['sha256'] if manifest_path.is_file() else None
    part = out.with_name(f'.part-{FILE}')
    subprocess.run(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-ss', f'{START:.2f}', '-to', f'{END:.2f}',
                    '-i', str(src), *ENCODE, str(part)], check=True)
    part.replace(out)
    duration = subprocess.run(['ffprobe', '-v', 'error', '-show_entries', 'format=duration', '-of',
                               'default=nw=1:nk=1', str(out)], capture_output=True, text=True, check=True).stdout
    changes = (f'Cut to {clock(START)}-{clock(END)} of the video, scaled to 1920x1080 at 30 fps and encoded as '
               'H.264 (CRF 26) with AAC at 128 kb/s')
    manifest = {
        'file': FILE,
        'credit': {
            'source': 'youtube', 'id': VIDEO_ID, 'title': TITLE, 'author': AUTHOR, 'license': 'by',
            'licenseVersion': '3.0', 'licenseUrl': LICENSE_URL, 'url': PAGE,
        },
        'attribution': f'"{TITLE}" by {AUTHOR} ({PAGE}), licensed under CC BY 3.0 ({LICENSE_URL}). {changes}.',
        'download': f'yt-dlp -f {FORMATS} --merge-output-format mp4 {PAGE}',
        'downloadSha256': SOURCE_SHA256,
        'changes': changes,
        'durationUs': round(float(duration) * 1e6),
        'bytes': out.stat().st_size,
        'sha256': sha256(out),
        'transcript': 'transcript.json',
        'transcriptMadeBy': "Nuzky's transcribe tool: large-v3-turbo-q5_0, word times measured with wav2vec2-base-960h-f16",
    }
    manifest_path.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + '\n')
    # The installers carry it with the licence (scripts/build-bundles.mjs), as CC BY asks of every copy.
    (OUT / 'ATTRIBUTION.txt').write_text(
        f'The sample video built into Nuzky, which its tour for new users opens:\n\n{manifest["attribution"]}\n\n'
        'The licence\'s legal code is CC-BY-3.0.txt beside this file.\n')
    print(f'{out.relative_to(ROOT)}: {manifest["bytes"] / 1e6:.2f} MB, SHA-256 {manifest["sha256"]}')
    if before and before != manifest['sha256']:
        sys.exit('The video changed, so transcript.json no longer belongs to it. Make it again with the transcribe '
                 'tool as the top of this script says.')


if __name__ == '__main__':
    main()

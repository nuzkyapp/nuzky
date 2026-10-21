#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { COMMIT as ORT_COMMIT, VERSION as ORT_VERSION, fetchOnnxRuntime } from './fetch-onnxruntime.mjs';
import { createDmg } from './macos-signing.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
process.chdir(root);
const target = path.resolve(process.env.CARGO_TARGET_DIR || 'target');
const bundle = path.join(target, 'release', 'bundle');
mkdirSync(bundle, { recursive: true });
const run = (cmd, args) => execFileSync(cmd, args, { stdio: 'inherit' });
const output = (cmd, args) => execFileSync(cmd, args, { encoding: 'utf8' }).trim();
const { version } = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8'));
const config = { bundle: {} };
let bundles;
let ffmpegSource;
// ONNX Runtime ships beside the app and is opened only when covers run (crates/vision/src/runtime.rs).
const ort = await fetchOnnxRuntime();
// The emoji and stickers (assets/stickers) and the CC BY sample video (assets/sample) are built into the binary;
// their licences, and the sample's attribution, go with it.
const notices = Object.fromEntries([
  ...ort.notices.map((n) => [n, `licenses/onnxruntime/${path.basename(n)}`]),
  ...['stickers', 'sample'].flatMap((dir) => readdirSync(`assets/${dir}`).filter((n) => n.endsWith('.txt'))
    .map((n) => [path.join(root, 'assets', dir, n), `licenses/${dir}/${n}`])),
]);

switch (process.platform) {
  case 'linux': {
    bundles = 'appimage,deb';
    const dependencies = ['libwebkit2gtk-4.1-0', 'libgtk-3-0t64', 'libasound2t64'];
    for (const lib of ['libavcodec', 'libavformat', 'libavutil', 'libswscale', 'libswresample']) {
      dependencies.push(lib + output('pkg-config', ['--modversion', lib]).split('.')[0]);
    }
    config.bundle.linux = { deb: { depends: dependencies, section: 'video' } };
    const avcodec = dependencies.find((name) => name.startsWith('libavcodec'));
    const [source, sourceVersion] = output('dpkg-query', ['-W', '-f=${source:Package} ${source:Version}', avcodec]).split(' ');
    ffmpegSource = `FFmpeg in the AppImage: Ubuntu's ${source} ${sourceVersion}, source https://launchpad.net/ubuntu/+source/${source}/${sourceVersion} (apt-get source ${source}=${sourceVersion}). The .deb uses the system's.`;
    // Resources land in usr/lib/Nuzky in both the AppImage and the deb, where the app looks.
    config.bundle.resources = { [ort.library]: path.basename(ort.library), ...notices };
    // Allow the bundler's own AppImages to run on hosts without mounted FUSE.
    process.env.APPIMAGE_EXTRACT_AND_RUN = '1';
    break;
  }
  case 'darwin':
    if (process.arch !== 'arm64') throw new Error('Use an Apple Silicon host.');
    bundles = 'app';
    // The library itself goes to Contents/Frameworks through bundle-macos-libs.py below.
    config.bundle.resources = notices;
    break;
  case 'win32': {
    bundles = 'nsis';
    const ffmpeg = process.env.FFMPEG_DIR;
    const tag = process.env.FFMPEG_BTBN_TAG;
    if (!ffmpeg || !tag) throw new Error('Run scripts/setup-windows-deps.ps1 in this terminal first.');
    const [, commit] = path.basename(ffmpeg).match(/-g([0-9a-f]+)-/);
    ffmpegSource = `FFmpeg ${path.basename(ffmpeg)} from https://github.com/BtbN/FFmpeg-Builds/releases/tag/${tag}: FFmpeg commit https://github.com/FFmpeg/FFmpeg/commit/${commit}, built by https://github.com/BtbN/FFmpeg-Builds/tree/${tag}, whose scripts pin the source of every library in it.`;
    const dlls = readdirSync(path.join(ffmpeg, 'bin')).filter(name => name.endsWith('.dll'));
    if (!dlls.some(name => name.startsWith('avcodec-'))) throw new Error('FFmpeg DLLs missing.');
    config.bundle.resources = Object.fromEntries(dlls.map(name => [path.join(ffmpeg, 'bin', name), name]));
    Object.assign(config.bundle.resources, { [ort.library]: path.basename(ort.library) }, notices);
    for (const name of ['LICENSE.txt', 'LICENSE', 'README.txt']) {
      if (existsSync(path.join(ffmpeg, name))) config.bundle.resources[path.join(ffmpeg, name)] = `licenses/ffmpeg/${name}`;
    }
    // Tauri drops the drive letter from an absolute resource path, and GitHub's runner keeps these files on C:
    // and the checkout on D:. Copied beside the build, they are named relative to src-tauri instead.
    const stage = path.join(target, 'bundle-resources');
    rmSync(stage, { recursive: true, force: true });
    config.bundle.resources = Object.fromEntries(Object.entries(config.bundle.resources).map(([from, to]) => {
      const copy = path.join(stage, to);
      mkdirSync(path.dirname(copy), { recursive: true });
      copyFileSync(from, copy);
      return [path.relative('src-tauri', copy), to];
    }));
    break;
  }
  default: throw new Error(`Unsupported platform: ${process.platform}`);
}
const configPath = path.join(target, 'bundle-config.json');
writeFileSync(configPath, JSON.stringify(config, null, 2));
// Call the installed CLI with Node to avoid Windows .cmd quoting/shell differences.
run(process.execPath, ['node_modules/@tauri-apps/cli/tauri.js', 'build', '--bundles', bundles, '--config', configPath, '--', '--locked']);

if (process.platform === 'darwin') {
  const app = path.join(bundle, 'macos', 'Nuzky.app');
  run('python3', ['scripts/bundle-macos-libs.py', app, ort.library]);
  execFileSync('tar', ['--no-mac-metadata', '-czf', path.join(bundle, 'macos', `Nuzky_${version}_aarch64.app.tar.gz`), '-C', path.dirname(app), 'Nuzky.app'], { stdio: 'inherit', env: { ...process.env, COPYFILE_DISABLE: '1' } });
  // It lists where each copied library came from, so it names the exact Homebrew FFmpeg.
  const libs = readFileSync(path.join(app, 'Contents/Resources/bundled-libraries.txt'), 'utf8');
  const [, formula, cellar] = libs.match(/\/Cellar\/([^/]+)\/([^/]+)\/lib\/libavcodec\./);
  ffmpegSource = `FFmpeg from Homebrew's ${formula} ${cellar}, source https://ffmpeg.org/releases/ffmpeg-${cellar.replace(/_\d+$/, '')}.tar.xz. Bundled libraries:\n${libs.trim()}`;
  createDmg(app, path.join(bundle, 'dmg', `Nuzky_${version}_aarch64.dmg`), target);
}
const info = [
  `Nuzky ${version}, commit ${output('git', ['rev-parse', 'HEAD'])}; its source is the Source code archive of the v${version} release`,
  `Platform: ${process.platform} ${process.arch}`,
  output('rustc', ['--version']),
  output('ffmpeg', ['-version']),
  ffmpegSource,
  `ONNX Runtime ${ORT_VERSION} from ${ort.url} (SHA-256 ${ort.sha256}), source https://github.com/microsoft/onnxruntime/tree/${ORT_COMMIT} with the dependencies its cmake/deps.txt pins.`,
  'The rest of the corresponding source is collected by hand: docs/BUILDING.md, Redistribution.',
];
if (process.platform === 'darwin') {
  // sign-macos.mjs rewrites this line when it signs with the Developer ID.
  info.push('macOS signature: ad-hoc, not notarized');
  // Formulae only: the bundled libraries come from them, and a cask from an untrusted tap makes plain `brew list` fail.
  info.push(output('brew', ['list', '--formula', '--versions']));
}
if (process.platform === 'linux') {
  // linuxdeploy copied these libraries from Ubuntu's packages; docs/BUILDING.md fetches their source by this list.
  const appdir = path.join(bundle, 'appimage', 'Nuzky.AppDir');
  const libs = output('find', [appdir, '-type', 'f', '-name', '*.so*', '-not', '-path', '*/usr/lib/Nuzky/*', '-printf', '%f\n']);
  const owners = output('dpkg', ['-S', ...new Set(libs.split('\n').map((name) => `*/${name}`))]).split('\n')
    .flatMap((line) => line.slice(0, line.lastIndexOf(': ')).split(', '));
  const sources = output('dpkg-query', ['-W', '-f=${source:Package} ${source:Version}\n', ...new Set(owners)]);
  info.push(`Ubuntu source packages of the libraries in the AppImage:\n${[...new Set(sources.split('\n'))].sort().join('\n')}`);
  info.push(output('dpkg-query', ['-W']));
}
writeFileSync(path.join(bundle, `build-info-${process.platform}.txt`), info.join('\n') + '\n');

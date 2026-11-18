#!/usr/bin/env node
// Fetches Microsoft's official ONNX Runtime build, which crates/vision loads at run time instead of
// linking it, into the build dependencies directory: Microsoft's NuGet package, which carries the
// library for every system, is checked against its pinned SHA-256 and only this system's library and
// the licence notices are kept. Prints the library's path. Debug builds find it there;
// scripts/build-bundles.mjs puts it in the installers.
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { inflateRawSync } from 'node:zlib';

// Keep in step with RUNTIME_VERSION in crates/vision/src/runtime.rs.
export const VERSION = '1.28.0';
// The commit of github.com/microsoft/onnxruntime release v1.28.0, MIT, as the package's nuspec names it.
export const COMMIT = 'da9b5e364c465de65c49d91e696cd6485270757f';
const PACKAGE = `microsoft.ml.onnxruntime.${VERSION}.nupkg`;
const URL = `https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/${VERSION}/${PACKAGE}`;
const SHA256 = '769d1d3ea8ab6cd69f737c9dd4d4462aa4ad0ccfa106eaf506efc40d7bead5db';
const LIBRARIES = {
  'linux-x64': ['runtimes/linux-x64/native/libonnxruntime.so', 'libonnxruntime.so.1'],
  'darwin-arm64': ['runtimes/osx-arm64/native/libonnxruntime.dylib', 'libonnxruntime.1.dylib'],
  'win32-x64': ['runtimes/win-x64/native/onnxruntime.dll', 'onnxruntime.dll'],
};
const NOTICES = ['LICENSE', 'ThirdPartyNotices.txt'];

/** The same directory the setup scripts keep build dependencies in. */
export function depsDir() {
  if (process.env.NUZKY_DEPS) return process.env.NUZKY_DEPS;
  if (process.platform === 'win32') return path.join(process.env.LOCALAPPDATA, 'Nuzky', 'build-deps');
  return path.join(os.homedir(), '.cache', 'nuzky', 'deps');
}

const sha256 = (data) => createHash('sha256').update(data).digest('hex');

/** Reads the named members of a zip archive (a .nupkg is one) into a map of name to contents. */
function unzip(zip, names) {
  const end = zip.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  if (end < 0) throw new Error('not a zip archive');
  const found = new Map();
  let at = zip.readUInt32LE(end + 16);
  for (let i = zip.readUInt16LE(end + 10); i > 0; i--) {
    if (zip.readUInt32LE(at) !== 0x02014b50) throw new Error('broken zip directory');
    const method = zip.readUInt16LE(at + 10);
    const packed = zip.readUInt32LE(at + 20);
    const size = zip.readUInt32LE(at + 24);
    const nameLength = zip.readUInt16LE(at + 28);
    const local = zip.readUInt32LE(at + 42);
    const name = zip.toString('utf8', at + 46, at + 46 + nameLength);
    at += 46 + nameLength + zip.readUInt16LE(at + 30) + zip.readUInt16LE(at + 32);
    if (!names.includes(name)) continue;
    const start = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28);
    const data = zip.subarray(start, start + packed);
    const out = method === 8 ? inflateRawSync(data) : method === 0 ? data : null;
    if (!out || out.length !== size) throw new Error(`cannot read ${name} from the archive`);
    found.set(name, out);
  }
  const missing = names.filter((n) => !found.has(n));
  if (missing.length) throw new Error(`the archive lacks ${missing.join(', ')}`);
  return found;
}

/** Returns `{dir, url, sha256, library, notices}`; downloads only when the checked copy is missing. */
export async function fetchOnnxRuntime() {
  const library = LIBRARIES[`${process.platform}-${process.arch}`];
  if (!library) throw new Error(`No ONNX Runtime build is pinned for ${process.platform}-${process.arch}.`);
  const deps = depsDir();
  const dir = path.join(deps, `onnxruntime-${VERSION}`);
  const result = {
    dir, url: URL, sha256: SHA256, library: path.join(dir, library[1]), notices: NOTICES.map((n) => path.join(dir, n)),
  };
  if (existsSync(path.join(dir, '.ready'))) return result;
  mkdirSync(deps, { recursive: true });
  const archive = path.join(deps, PACKAGE);
  let zip = existsSync(archive) ? readFileSync(archive) : null;
  if (!zip || sha256(zip) !== SHA256) {
    console.error(`> ${URL}`);
    const response = await fetch(URL);
    if (!response.ok) throw new Error(`Downloading ${URL}: HTTP ${response.status}`);
    zip = Buffer.from(await response.arrayBuffer());
    if (sha256(zip) !== SHA256) throw new Error(`${URL} does not match its SHA-256`);
    writeFileSync(`${archive}.part`, zip);
    renameSync(`${archive}.part`, archive);
  }
  const members = unzip(zip, [library[0], ...NOTICES]);
  const stage = mkdtempSync(path.join(deps, 'onnxruntime-stage-'));
  try {
    const out = path.join(stage, 'out');
    mkdirSync(out);
    writeFileSync(path.join(out, library[1]), members.get(library[0]), { mode: 0o755 });
    for (const notice of NOTICES) writeFileSync(path.join(out, notice), members.get(notice));
    writeFileSync(path.join(out, '.ready'), `${PACKAGE} ${SHA256}\n`);
    rmSync(dir, { recursive: true, force: true });
    renameSync(out, dir);
  } finally {
    rmSync(stage, { recursive: true, force: true });
  }
  return result;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  console.log((await fetchOnnxRuntime()).library);
}

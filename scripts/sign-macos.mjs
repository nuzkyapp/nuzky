#!/usr/bin/env node
// Signs the app in the dmg that build-bundles.mjs made with the Developer ID, makes the dmg again and notarizes it.
// CI runs it in a job of its own, on a machine where no dependency's build code ran, so none can reach the secrets.
import { execFileSync, spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createDmg, hdiutil, macosSigning } from './macos-signing.mjs';

const signing = macosSigning(process.env);
const warn = (message) => console.error(`${process.env.GITHUB_ACTIONS ? '::warning::' : ''}macOS: ${message} Gatekeeper blocks it until the user clicks Open Anyway (docs/BUILDING.md, Signing and notarization).`);
if (signing.identity === '-') {
  warn('the dmg stays ad-hoc signed and is NOT notarized, because APPLE_SIGNING_IDENTITY is not set.');
  process.exit(0);
}
process.chdir(path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..'));
const target = path.resolve(process.env.CARGO_TARGET_DIR || 'target');
const bundle = path.join(target, 'release', 'bundle');
const { version } = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8'));
const dmg = path.join(bundle, 'dmg', `Nuzky_${version}_aarch64.dmg`);
const run = (cmd, args) => execFileSync(cmd, args, { stdio: 'inherit' });
// For commands with a password among the arguments, which a failed command's error would repeat.
const secret = (cmd, args) => {
  try {
    execFileSync(cmd, args, { stdio: ['ignore', 'ignore', 'inherit'] });
  } catch {
    throw new Error(`${cmd} ${args[0]} failed`);
  }
};

const work = mkdtempSync(path.join(target, 'sign-macos-'));
const app = path.join(work, 'Nuzky.app');
const keychainDir = signing.certificate && mkdtempSync(path.join(os.tmpdir(), 'nuzky-signing-'));
const keychain = keychainDir && path.join(keychainDir, 'signing.keychain-db');
let searchList;
try {
  const mount = path.join(work, 'dmg');
  const device = hdiutil(['attach', '-nobrowse', '-readonly', '-mountpoint', mount, dmg]).match(/^\/dev\/disk\d+/m)[0];
  try {
    run('ditto', [path.join(mount, 'Nuzky.app'), app]);
  } finally {
    hdiutil(['detach', device]);
  }
  if (keychain) {
    const password = randomBytes(32).toString('hex');
    const p12 = path.join(keychainDir, 'certificate.p12');
    secret('security', ['create-keychain', '-p', password, keychain]);
    run('security', ['set-keychain-settings', '-lut', '900', keychain]);
    secret('security', ['unlock-keychain', '-p', password, keychain]);
    writeFileSync(p12, Buffer.from(signing.certificate.base64, 'base64'), { mode: 0o600 });
    secret('security', ['import', p12, '-k', keychain, '-P', signing.certificate.password, '-T', '/usr/bin/codesign']);
    rmSync(p12);
    // Lets codesign use the key without asking, which nobody could answer on a runner.
    secret('security', ['set-key-partition-list', '-S', 'apple-tool:,apple:,codesign:', '-s', '-k', password, keychain]);
    // codesign on macOS 14 finds no identity in a keychain outside the search list, even with --keychain.
    searchList = execFileSync('security', ['list-keychains', '-d', 'user'], { encoding: 'utf8' })
      .split('\n').map((line) => line.trim().replace(/^"|"$/g, '')).filter(Boolean);
    run('security', ['list-keychains', '-d', 'user', '-s', keychain, ...searchList]);
  }
  const sign = ['--force', '--timestamp', '--sign', signing.identity, ...(keychain ? ['--keychain', keychain] : [])];
  const frameworks = path.join(app, 'Contents/Frameworks');
  for (const name of readdirSync(frameworks).filter((name) => name.endsWith('.dylib'))) {
    run('codesign', [...sign, '--options', 'runtime', path.join(frameworks, name)]);
  }
  run('codesign', [...sign, '--options', 'runtime', app]);
  run('codesign', ['--verify', '--deep', '--strict', app]);
  createDmg(app, dmg, target);
  run('codesign', [...sign, dmg]);
} finally {
  if (searchList) spawnSync('security', ['list-keychains', '-d', 'user', '-s', ...searchList], { stdio: 'inherit' });
  if (keychain) spawnSync('security', ['delete-keychain', keychain], { stdio: 'inherit' });
  if (keychainDir) rmSync(keychainDir, { recursive: true, force: true });
  rmSync(work, { recursive: true, force: true });
}

let signature = 'Developer ID, not notarized';
if (signing.notary) {
  const credentials = ['--apple-id', signing.notary.appleId, '--team-id', signing.notary.teamId, '--password', signing.notary.password];
  const notarytool = (...args) =>
    spawnSync('xcrun', ['notarytool', ...args, ...credentials], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] }).stdout;
  // notarytool can print its JSON answer and still exit with an error, so the answer decides.
  const answer = notarytool('submit', dmg, '--wait', '--timeout', '30m', '--output-format', 'json');
  console.log(answer);
  const { id, status } = JSON.parse(answer || '{}');
  if (status !== 'Accepted') {
    if (id) console.log(notarytool('log', id));
    throw new Error(`Apple's notary service did not accept the dmg (status: ${status ?? 'none'}).`);
  }
  run('xcrun', ['stapler', 'staple', dmg]);
  run('xcrun', ['stapler', 'validate', dmg]);
  run('spctl', ['--assess', '--type', 'open', '--context', 'context:primary-signature', '-vv', dmg]);
  signature = 'Developer ID, notarized and stapled';
} else {
  warn('the dmg is Developer ID signed and NOT notarized, because APPLE_ID, APPLE_TEAM_ID and APPLE_APP_PASSWORD are not set.');
}
const info = path.join(bundle, 'build-info-darwin.txt');
writeFileSync(info, readFileSync(info, 'utf8').replace(/^macOS signature: .*$/m, `macOS signature: ${signature}`));

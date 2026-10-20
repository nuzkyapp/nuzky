import test from 'node:test';
import assert from 'node:assert/strict';
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { hdiutil, macosSigning } from './macos-signing.mjs';

const identity = 'Developer ID Application: Example (TEAM123456)';
const certificate = { APPLE_CERTIFICATE: 'cDEy', APPLE_CERTIFICATE_PASSWORD: 'certificate-password' };
const notary = { APPLE_ID: 'apple@example.com', APPLE_TEAM_ID: 'TEAM123456', APPLE_APP_PASSWORD: 'app-password' };

test('without variables, or with the empty values of unset GitHub secrets, the build signs ad-hoc', () => {
  const adHoc = { identity: '-', certificate: null, notary: null };
  assert.deepEqual(macosSigning({}), adHoc);
  assert.deepEqual(macosSigning({
    APPLE_SIGNING_IDENTITY: '', APPLE_CERTIFICATE: '', APPLE_CERTIFICATE_PASSWORD: '',
    APPLE_ID: '', APPLE_TEAM_ID: '', APPLE_APP_PASSWORD: '',
  }), adHoc);
});

test('all variables sign with the Developer ID and notarize', () => {
  assert.deepEqual(macosSigning({ APPLE_SIGNING_IDENTITY: identity, ...certificate, ...notary }), {
    identity,
    certificate: { base64: 'cDEy', password: 'certificate-password' },
    notary: { appleId: 'apple@example.com', teamId: 'TEAM123456', password: 'app-password' },
  });
});

test('an identity without notary credentials signs and does not notarize', () => {
  assert.deepEqual(macosSigning({ APPLE_SIGNING_IDENTITY: identity, ...certificate }), {
    identity, certificate: { base64: 'cDEy', password: 'certificate-password' }, notary: null,
  });
  assert.deepEqual(macosSigning({ APPLE_SIGNING_IDENTITY: identity }), { identity, certificate: null, notary: null });
});

test('an incomplete group stops the build and names only the missing variables', () => {
  assert.throws(() => macosSigning({ APPLE_SIGNING_IDENTITY: identity, APPLE_ID: 'apple@example.com' }),
    { message: 'Missing macOS signing variables: APPLE_TEAM_ID, APPLE_APP_PASSWORD' });
  assert.throws(() => macosSigning({ APPLE_SIGNING_IDENTITY: identity, APPLE_CERTIFICATE_PASSWORD: 'certificate-password' }),
    { message: 'Missing macOS signing variables: APPLE_CERTIFICATE' });
  assert.throws(() => macosSigning({ ...certificate, ...notary }),
    { message: 'Missing macOS signing variables: APPLE_SIGNING_IDENTITY' });
});

test('the release notes have the one macOS line that release.yml drops for a notarized dmg', () => {
  const notes = readFileSync(new URL('release-notes.md', import.meta.url), 'utf8');
  const lines = notes.split('\n').filter((line) => line.startsWith('- **macOS**:'));
  assert.equal(lines.length, 1);
  assert.match(lines[0], /System Settings → Privacy & Security, click Open Anyway/);
});

test('hdiutil tries again only while the disk is busy', t => {
  const bin = mkdtempSync(path.join(tmpdir(), 'nuzky-hdiutil-'));
  const calls = path.join(bin, 'calls');
  const path0 = process.env.PATH;
  process.env.PATH = `${bin}${path.delimiter}${path0}`;
  t.after(() => {
    process.env.PATH = path0;
    rmSync(bin, { recursive: true, force: true });
  });
  // Fails with each listed message in turn, then succeeds.
  const fake = (...errors) => {
    writeFileSync(calls, '');
    writeFileSync(path.join(bin, 'hdiutil'), `#!/bin/sh
echo "$*" >> "${calls}"
n=$(grep -c "" "${calls}")
case $n in
${errors.map((error, i) => `  ${i + 1}) echo "${error}" >&2; exit 1;;`).join('\n')}
esac
`);
    chmodSync(path.join(bin, 'hdiutil'), 0o755);
  };
  const called = () => readFileSync(calls, 'utf8').trim().split('\n');

  fake('hdiutil: create failed - Resource busy', 'hdiutil: create failed - Resource busy');
  assert.equal(hdiutil(['create', '-ov', 'rw.dmg'], [0, 0]), '');
  assert.deepEqual(called(), Array(3).fill('create -ov rw.dmg'));

  fake('hdiutil: couldn\'t eject "disk4" - Resource busy');
  hdiutil(['detach', '/dev/disk4'], [0, 0]);
  assert.deepEqual(called(), Array(2).fill('detach /dev/disk4'));

  fake(...Array(3).fill('hdiutil: create failed - Resource busy'));
  assert.throws(() => hdiutil(['create', 'rw.dmg'], [0, 0]), { message: 'hdiutil create failed with exit code 1' });
  assert.equal(called().length, 3);

  fake('hdiutil: create failed - No space left on device');
  assert.throws(() => hdiutil(['create', 'rw.dmg'], [0, 0]), { message: 'hdiutil create failed with exit code 1' });
  assert.equal(called().length, 1);
});

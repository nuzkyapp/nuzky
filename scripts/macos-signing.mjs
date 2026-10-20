import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, rmSync, symlinkSync } from 'node:fs';
import path from 'node:path';

export function macosSigning(env) {
  const certificateKeys = ['APPLE_CERTIFICATE', 'APPLE_CERTIFICATE_PASSWORD'];
  const notaryKeys = ['APPLE_ID', 'APPLE_TEAM_ID', 'APPLE_APP_PASSWORD'];
  const certificate = certificateKeys.some(key => env[key]);
  const notary = notaryKeys.some(key => env[key]);
  const missing = [
    ...(certificate ? certificateKeys.filter(key => !env[key]) : []),
    ...(notary ? notaryKeys.filter(key => !env[key]) : []),
    ...((certificate || notary) && !env.APPLE_SIGNING_IDENTITY ? ['APPLE_SIGNING_IDENTITY'] : []),
  ];
  if (missing.length) throw new Error(`Missing macOS signing variables: ${missing.join(', ')}`);
  return {
    identity: env.APPLE_SIGNING_IDENTITY || '-',
    certificate: certificate ? { base64: env.APPLE_CERTIFICATE, password: env.APPLE_CERTIFICATE_PASSWORD } : null,
    notary: notary ? { appleId: env.APPLE_ID, teamId: env.APPLE_TEAM_ID, password: env.APPLE_APP_PASSWORD } : null,
  };
}

// Finder keeps the window's size, background and icon positions in the volume's .DS_Store, and only Finder writes it.
const layout = mount => `
tell application "Finder"
  set d to POSIX file "${mount}" as alias
  open d
  set w to container window of d
  set current view of w to icon view
  set toolbar visible of w to false
  set statusbar visible of w to false
  set pathbar visible of w to false
  set bounds of w to {200, 120, 840, 552}
  set o to icon view options of w
  set arrangement of o to not arranged
  set icon size of o to 128
  set text size of o to 13
  set background picture of o to POSIX file "${mount}/.background/background.tiff" as alias
  set position of item "Nuzky.app" of d to {180, 190}
  set position of item "Applications" of d to {460, 190}
  update d without registering applications
  delay 1
  close w
end tell`;

// GitHub's macOS runners sometimes hold a disk image for a moment, and hdiutil gives up with "Resource busy" while
// creating or ejecting it (actions/runner-images#7522). A few seconds later it succeeds.
export function hdiutil(args, waits = [2, 4, 8, 16]) {
  for (const wait of [...waits, null]) {
    const { status, stdout, stderr } = spawnSync('hdiutil', args,
      { stdio: ['ignore', 'pipe', 'pipe'], encoding: 'utf8' });
    process.stdout.write(stdout);
    process.stderr.write(stderr);
    if (status === 0) return stdout;
    if (wait === null || !stderr.includes('Resource busy')) {
      throw new Error(`hdiutil ${args[0]} failed with exit code ${status}`);
    }
    execFileSync('sleep', [String(wait)]);
  }
}

export function createDmg(app, dmg, target) {
  // Finder finds a disk by its name, so it would lay out the other one.
  if (existsSync('/Volumes/Nuzky')) throw new Error('Eject the Nuzky disk image that is open in Finder, then build again.');
  mkdirSync(path.dirname(dmg), { recursive: true });
  const stage = path.resolve(mkdtempSync(path.join(target, 'dmg-stage-')));
  const content = path.join(stage, 'content');
  const image = path.join(stage, 'rw.dmg');
  const mount = path.join(stage, 'mount');
  const run = (command, args) => execFileSync(command, args, { stdio: 'inherit' });
  try {
    run('ditto', [app, path.join(content, 'Nuzky.app')]);
    symlinkSync('/Applications', path.join(content, 'Applications'));
    mkdirSync(path.join(content, '.background'));
    run('tiffutil', ['-cathidpicheck', 'src-tauri/dmg/background.png', 'src-tauri/dmg/background@2x.png',
      '-out', path.join(content, '.background', 'background.tiff')]);
    hdiutil(['create', '-ov', '-format', 'UDRW', '-volname', 'Nuzky', '-srcfolder', content, image]);
    const attached = hdiutil(['attach', '-readwrite', '-noverify', '-noautoopen', '-mountpoint', mount, image]);
    // Detaching by the mount point fails once a busy eject has already unmounted the volume; the device stays.
    const device = attached.match(/^\/dev\/disk\d+/m)[0];
    try {
      run('osascript', ['-e', layout(mount)]);
      if (!existsSync(path.join(mount, '.DS_Store'))) throw new Error('Finder did not save the layout of the dmg window.');
    } finally {
      hdiutil(['detach', device]);
    }
    hdiutil(['convert', image, '-ov', '-format', 'UDZO', '-o', dmg]);
  } finally {
    rmSync(stage, { recursive: true, force: true });
  }
}

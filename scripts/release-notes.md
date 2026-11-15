The eleventh alpha of Nuzky, a free, open-source video editor for your computer. It's early, so expect rough edges. Nuzky never changes or deletes your original videos.

## What's new

- **Help improve Nuzky.** Nuzky asks once whether it may send me, its author, what goes wrong and how it's used, so I can fix what breaks on your computer and not only on mine. You choose two things separately, and both stay off unless you tick them: problems and usage (crashes, errors, which features you use and how fast Nuzky runs), and your conversations in the AI panel. Never your videos, sound, captions or file names, and never tied to your name or email. Settings › Privacy turns either on or off, shows what was sent and deletes it.
- **Send crash report.** After Nuzky closes unexpectedly, the message offers to send a crash report in one click, even with both of the above off.

## Fixed

- Opening another project stops the work started for the one before, such as recognising speech, Reframe or preparing the sound of a long recording. Before, that work kept running and showed in the project you opened, which then couldn't transcribe or reframe its own video until it ended.

## Not yet

- macOS 14, Intel Macs, Linux on ARM and Windows on ARM aren't supported.
- Video exports as MP4 only. There's no template library and no phone app.

## Systems

- **macOS 15 or newer on Apple Silicon**: the `.dmg`. Open it and drag Nuzky to Applications.
- **Windows 10 or 11, x64**: the `-setup.exe`. The installer downloads Microsoft WebView2 if it's missing.
- **Linux, x86_64**: the `.deb` for Ubuntu 24.04 or newer (`sudo apt install ./Nuzky_*.deb`), or the `.AppImage` for other distributions with glibc 2.39 or newer. Make the AppImage executable with `chmod +x` and run it.

## Opening an unsigned build

- **macOS**: Apple hasn't checked this build, so macOS won't open it the first time. Close the message, open System Settings → Privacy & Security, click Open Anyway next to Nuzky and confirm.
- **Windows**: Nuzky has no Microsoft signing certificate yet, so SmartScreen says "Windows protected your PC" the first time. Click More info, then Run anyway.

## Updates

Once a day Nuzky reads the version number of the latest release on this page, and nothing else. When a newer one is out, the version at the bottom of the home screen's sidebar says so. Click it and choose Update: Nuzky downloads the new version from this page and keeps it only if its signature matches the key built into Nuzky. Then it saves your project, installs the new version and restarts. If you keep working while it downloads, it waits until you click Restart to update, and while an export or another job runs, the restart waits for it. Nothing downloads or installs before you click. To turn the check off, choose Update settings… in the same menu and untick Check automatically.

## Found a bug?

Open an issue at https://github.com/nuzkyapp/nuzky/issues, or use Send feedback → Report a bug… in the app, which fills in your version and system. Say what you did and what happened, and please don't attach private videos. No GitHub account? Reply to a post or send a DM to [@mach_builds](https://x.com/mach_builds) on X.

## Source

Nuzky is GPL-3.0-or-later, and its source is the Source code archive below. The installers include FFmpeg, ONNX Runtime and other libraries. Their source, and what each installer was built from, is in [Third-party source for Nuzky v0.1.11-alpha](https://github.com/nuzkyapp/nuzky/releases/tag/third-party-source-0.1.11-alpha).

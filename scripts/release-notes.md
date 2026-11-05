The sixth alpha of Nuzky, a free, open-source video editor for your computer. It's early, so expect rough edges. Nuzky never changes or deletes your original videos.

## What's new

- **Record your screen.** Record screen in Media records the whole screen or one window on a Mac and adds it to Media when you click Stop in the top bar. On Linux and Windows the button stays off for now.
- **B-roll where you name an app.** The Transcript tab suggests where to show a silent video from Media: when you say a word of its name, for example "now I open Claude", Add puts the recording over the picture for that sentence. Your voice and captions stay as they were.
- **Text that covers a face.** Find text over faces, under Generate in the Captions tab, finds every caption, text and sticker over the speaker's face and suggests a clear place inside the Reels and TikTok safe area. Nothing moves until you click Move all, which you can undo.
- **Mono for one-sided sound.** A voice recorded on one side, for example by a lavalier in one channel, plays in one ear. Mono in the Audio tab plays it in both, and Nuzky tells you when a file has sound on one side only.
- **One file per hook.** Ask the AI to export each opening of a reel as its own file, with the same body and captions in every one, to try them as Instagram Trial Reels.

## Not yet

- Intel Macs, Linux on ARM and Windows on ARM aren't supported.
- Export makes MP4 only. There's no template library and no phone app.

## Systems

- **macOS 14 or newer on Apple Silicon**: the `.dmg`. Open it and drag Nuzky to Applications.
- **Windows 10 or 11, x64**: the `-setup.exe`. The installer downloads Microsoft WebView2 if it's missing.
- **Linux, x86_64**: the `.deb` for Ubuntu 24.04 or newer (`sudo apt install ./Nuzky_*.deb`), or the `.AppImage` for other distributions with glibc 2.39 or newer. Make the AppImage executable with `chmod +x` and run it.

## Opening an unsigned build

- **macOS**: Apple hasn't checked this build, so macOS won't open it the first time. Close the message, open System Settings → Privacy & Security, click Open Anyway next to Nuzky and confirm.
- **Windows**: Nuzky has no Microsoft signing certificate yet, so SmartScreen says "Windows protected your PC" the first time. Click More info, then Run anyway.

## Updates

Once a day Nuzky reads the version number of the latest release on this page, and nothing else. When a newer one is out, the version at the bottom of the home screen's sidebar says so. Click it and choose Update: Nuzky downloads the new version from this page and keeps it only if its signature matches the key built into Nuzky. Then it saves your project, installs the new version and restarts. If you keep working while it downloads, it waits until you click Restart to update, and while an export or another job runs, the restart waits for it. Nothing downloads or installs before you click. To turn the check off, untick Check automatically in the same menu.

## Found a bug?

Open an issue at https://github.com/nuzkyapp/nuzky/issues, or use Send feedback → Report a bug… in the app, which fills in your version and system. Say what you did and what happened, and please don't attach private videos. No GitHub account? Reply to a post or send a DM to [@mach_builds](https://x.com/mach_builds) on X.

## Source

Nuzky is GPL-3.0-or-later, and its source is the Source code archive below. The installers include FFmpeg, ONNX Runtime and other libraries. Their source, and what each installer was built from, is in [Third-party source for Nuzky v0.1.6-alpha](https://github.com/nuzkyapp/nuzky/releases/tag/third-party-source-0.1.6-alpha).

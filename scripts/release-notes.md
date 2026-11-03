The fifth alpha of Nuzky, a free, open-source video editor for your computer. It's early, so expect rough edges. Nuzky never changes or deletes your original videos.

## What's new

- **Edit from your phone.** The QR code button in the top bar shows a code and a link. Open it on a phone or another computer on the same Wi-Fi to add videos to the timeline, ask the AI to edit, watch the preview, export and download the result. Nuzky on your computer does all the work. Sharing stays off until you turn it on and ends when you click Stop sharing or quit.
- **Reels that come out finished.** Make reels in Reels now also cuts retakes, fillers and hesitations, adds captions in your style and zooms on the sentences you stress. The Each reel gets menu turns any of them off.
- **Your own keyboard shortcuts.** Keyboard shortcuts (Ctrl+, or Cmd+, on a Mac, or the gear in the editor's top bar) lets you change the key of every editor action, switch to the keys of Premiere Pro, Final Cut Pro or DaVinci Resolve, or import a keyboard file exported from them.
- **Font weights.** A Weight menu beside the font picker offers every weight the font has, from Thin to Black, for texts, captions and covers.
- **4K projects.** Ratio under the preview offers 1080p or 4K. Texts, outlines and borders keep their size on the picture when you switch.
- **Preview a file before you use it.** Click a video in Media and it plays in the preview without being added to the timeline. Esc brings the timeline back, and a double-click still adds the file.
- **Plugins with full access.** A plugin can ask to read and write files, use the internet and run other programs, for example to export the timeline for Premiere or Resolve. Nuzky asks you before its first run and again after any change to the plugin, and the AI can never run it.
- **Updates in one click.** When you click Update and leave Nuzky alone while it downloads, it restarts into the new version by itself.
- **Send feedback is labelled** in the editor's top bar, so Report a bug… is easy to find.

## Fixed

- The AI panel works with Codex installed by its own installer. Every message used to fail with "Operation not permitted".

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

Nuzky is GPL-3.0-or-later, and its source is the Source code archive below. The installers include FFmpeg, ONNX Runtime and other libraries. Their source, and what each installer was built from, is in [Third-party source for Nuzky v0.1.5-alpha](https://github.com/nuzkyapp/nuzky/releases/tag/third-party-source-0.1.5-alpha).

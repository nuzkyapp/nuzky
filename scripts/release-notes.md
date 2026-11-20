The twelfth alpha of Nuzky, a free, open-source video editor for your computer. It's early, so expect rough edges. Nuzky never changes or deletes your original videos.

## What's new

- **Chapters for YouTube.** In a video of 30 seconds or longer, Suggest chapters in the Transcript tab asks the AI panel's agent to split what's said where the topic changes and to title each part. Nuzky times every chapter on the finished video itself, so they move with your cuts and speed changes. The chapters show under the timeline's ruler the way YouTube's player shows them: click one to jump there, double-click it to rename it. Copy puts the list on the clipboard for the video's description, and Export can save it beside the video as a `.chapters.txt` file. The list always keeps YouTube's rules (the first chapter at 00:00, at least three, each 10 seconds or longer), and when an edit breaks them, Nuzky says which chapter it left out and why.
- **Layout for a wide video on a vertical canvas.** Beside 9:16 and 4:5, the Ratio menu has Layout with three styles. Fill is Reframe, which keeps the face in the frame. Fade shows the whole picture across the middle over a blurred copy of it. Stack shows the clip in the top half and a silent copy of it in the bottom half. Each style is one undo step, and an agent in the AI panel can pick them too.

## Fixed

- In a narrow window, the AI run in the top bar no longer covers Saved.
- Text style tiles in the Text tab fit their whole word, such as BREAKING, instead of ending it with "…".

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

Nuzky is GPL-3.0-or-later, and its source is the Source code archive below. The installers include FFmpeg, ONNX Runtime and other libraries. Their source, and what each installer was built from, is in [Third-party source for Nuzky v0.1.12-alpha](https://github.com/nuzkyapp/nuzky/releases/tag/third-party-source-0.1.12-alpha).

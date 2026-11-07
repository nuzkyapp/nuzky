The seventh alpha of Nuzky, a free, open-source video editor for your computer. It's early, so expect rough edges. Nuzky never changes or deletes your original videos.

## What's new

- **Edit from your phone, like CapCut.** The page you open from the QR code is now a small editor: a preview with Undo, Redo and Play, every track under a playhead in the middle, and Split, Duplicate, Detach audio and Delete when you tap a clip. Pick the AI agent, its model and past chats there, start a new project or open one from your list. The QR button is now on the home screen too.
- **One Settings window.** `Cmd+,` or `Ctrl+,`, the gear in the editor and Settings in the home screen's sidebar open every setting in one place, with search. New there: how much disk Nuzky takes, with Clear for its caches; Learn from my edits, to stop Your style learning from every export; Revoke for a plugin's full access; Remove for a speech model you don't use; and Privacy, one list of everything that goes online and whether it's on.
- **Captions for some clips only.** Select video clips on the timeline and Generate in the Captions tab makes captions only over them. Captions elsewhere stay as they were, hand edits included.
- **More room for the video.** The side panels and the timeline start smaller, and the AI panel opens in the inspector's place by default, so the video keeps its size while an agent works.

## Fixed

- Take the tour no longer leaves a "Sample video" project among your projects, and the next start doesn't open it.
- On the phone, clips no longer blink and the page opens and follows the preview without the delay it had.

## Not yet

- macOS 14, Intel Macs, Linux on ARM and Windows on ARM aren't supported.
- Export makes MP4 only. There's no template library and no phone app.

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

Nuzky is GPL-3.0-or-later, and its source is the Source code archive below. The installers include FFmpeg, ONNX Runtime and other libraries. Their source, and what each installer was built from, is in [Third-party source for Nuzky v0.1.7-alpha](https://github.com/nuzkyapp/nuzky/releases/tag/third-party-source-0.1.7-alpha).

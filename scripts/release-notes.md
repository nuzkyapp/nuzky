The third alpha of Nuzky, a free, open-source video editor for your computer. It's early, so expect rough edges. Nuzky never changes or deletes your original videos.

## What's new

- **Plugins.** Ask the AI for a feature or a look Nuzky lacks, such as "remove clips shorter than a second" or a VHS look, and it writes a plugin that stays in Nuzky's plugins folder through updates. The Plugins tab shows its settings with Nuzky's own controls and says what it would do before you press its button. A run is one step you can undo. A plugin can add its own logo or sound, captions, zooms and motion, and looks you put on clips in Adjust → Effects. Turn on Run after import or Run before export to have it run by itself. A plugin only reads the project and returns edits. It can't reach the internet, your files or other apps.
- **A plugin gallery.** Gallery in the Plugins tab lists plugins other people share, which Nuzky's maintainer checks first. It tells you what each one may do before you click Install, and installing runs nothing.
- **Cut to the beat.** Pick a song and Cut to beat in Project status finds its beats, shows them on the song and recuts the video on them: Calm, Builds to the drop or Fast. The AI can do the same.

## Fixed

- Emoji in texts and captions keep their own colours, and look the same on every computer. A 💀 in a caption with an outline used to come out as a white silhouette.
- On a Mac, a pasted YouTube link in Music shows its title in seconds.
- A clip's thumbnail shows right after a phone video is added, without waiting for its filmstrip.
- The timeline stops scrolling where the clips end, and the track headers stay in place.
- Your style learns Spelling only from words the speech model heard wrong, and a suggested change shows only the words it adds or drops.
- Update available in the sidebar shows the whole version.

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

Once a day Nuzky reads the version number of the latest release on this page, and nothing else. When a newer one is out, the version at the bottom of the home screen's sidebar says so. Click it and choose Update: Nuzky downloads the new version from this page and keeps it only if its signature matches the key built into Nuzky. When you click Restart to update, it saves your project, installs the new version and restarts. It never restarts by itself, and while an export or another job runs, Restart to update waits for it. Nothing downloads or installs before you click. To turn the check off, untick Check automatically in the same menu.

## Found a bug?

Open an issue at https://github.com/nuzkyapp/nuzky/issues, or use Send feedback → Report a bug… in the app, which fills in your version and system. Say what you did and what happened, and please don't attach private videos. No GitHub account? Reply to a post or send a DM to [@mach_builds](https://x.com/mach_builds) on X.

## Source

Nuzky is GPL-3.0-or-later, and its source is the Source code archive below. The installers include FFmpeg, ONNX Runtime and other libraries. Their source, and what each installer was built from, is in [Third-party source for Nuzky v0.1.2-alpha](https://github.com/nuzkyapp/nuzky/releases/tag/third-party-source-0.1.2-alpha).

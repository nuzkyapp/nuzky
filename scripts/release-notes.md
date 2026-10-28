The fourth alpha of Nuzky, a free, open-source video editor for your computer. It's early, so expect rough edges. Nuzky never changes or deletes your original videos.

## What's new

- **Smoother editing.** When you delete a piece of the timeline, paste a clip or undo, the clips slide into their new places instead of jumping, so you can see which clip went where. While the video plays, the playhead moves smoothly on every frame of your display.
- **Calmer menus, toasts and dialogs.** They come in and fade out with short motion instead of blinking. The chosen tab or option slides to a new choice, and thumbnails fade in once they load.
- **Tooltips that show shortcuts.** Hover a button and Nuzky's own tooltip says what it does, with its key, such as Split at playhead (S). They look the same on macOS, Windows and Linux.
- **The window buttons in the top bar on a Mac.** The grey title bar over the app is gone, so the video gets more room. The window is dark from the moment it opens, and the window switcher shows the project's name.

## Fixed

- You can move the project that just opened to the Trash. Nuzky opens another one behind the home screen first, and Undo brings it back.
- A project you moved to the Trash no longer comes back on the home screen.
- The hint for putting a video or picture over the video shows only while you drag one, not on every project.

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

Nuzky is GPL-3.0-or-later, and its source is the Source code archive below. The installers include FFmpeg, ONNX Runtime and other libraries. Their source, and what each installer was built from, is in [Third-party source for Nuzky v0.1.3-alpha](https://github.com/nuzkyapp/nuzky/releases/tag/third-party-source-0.1.3-alpha).

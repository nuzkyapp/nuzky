The fourteenth alpha of Nuzky, a free, open-source video editor for your computer. It's early, so expect rough edges. Nuzky never changes or deletes your original videos.

## What's new

- **Nuzky account.** Sign in with an optional Nuzky account to keep Your style and your caption styles on every computer. Nuzky syncs only those, never your videos, projects or transcripts. You don't need an account for anything else, and without one Nuzky doesn't contact the account server at all. Find it at the top of Settings.
- **See the agent at work.** While an AI agent edits your project, a pointer with its name acts out each change on the timeline: it snips at a split, drags the edge of a trim and carries a clip to its new place. Turn it off in Settings › AI.
- **A clearer inspector.** The sections of the Video tab fold and say what's set in them. Adjust groups its sliders under Light and Colour, and while you drag one, the preview shows the frame before the change beside the frame after it.
- **Smoother playback.** The preview no longer stutters now and then while a video plays.
- **Project names.** When the AI panel's agent makes captions, it names an untitled project after what the video is about.
- **Clearer ChatGPT errors.** When ChatGPT refuses a message, the AI panel says why. When a chat gets too long, it offers to continue in a new one.

## Fixes

- Nuzky no longer closes a few seconds after starting on Windows PCs with older Intel graphics.
- Nuzky no longer closes when it sets up captions on processors without AVX2, on Windows and Linux.
- Videos from an iPhone 16 or newer that record Apple spatial audio play their stereo sound. A video with only spatial audio is added without sound, and Nuzky says why.
- A slow video, a busy computer or a laptop that went to sleep no longer makes an export or the preview fail with "Timed out waiting for frame".
- Install guides and Manage usage open in your browser. Before, the buttons did nothing.
- A link the agent writes in the AI panel shows as its text, and a linked file shows in its folder when you click it.
- On macOS, the top bar leaves room after the window buttons.
- After an unexpected close, the next bug report says where Nuzky crashed, so we can fix it.

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

Nuzky is GPL-3.0-or-later, and its source is the Source code archive below. The installers include FFmpeg, ONNX Runtime and other libraries. Their source, and what each installer was built from, is in [Third-party source for Nuzky v0.1.14-alpha](https://github.com/nuzkyapp/nuzky/releases/tag/third-party-source-0.1.14-alpha).

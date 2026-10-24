The second alpha of Nuzky, a free, open-source video editor for your computer. It's early, so expect rough edges. Nuzky never changes or deletes your original videos.

## What's new

- **AI editing on Windows.** The AI panel, Connect and `nuzky mcp` now work on Windows too, and captions recognise speech on the graphics card there, as they do on a Mac.
- **OpenCode in the AI panel**, next to ChatGPT, Claude Code and Codex. Connect copies its setup for you.
- **AI that looks things up.** Ask for the logo and colours of a brand you mention, and the AI searches the web, puts the logo into your video and colours the captions to match. It can also bring in a picture from the web by its address.
- **AI that checks its own export.** After an export the AI looks for black frames, a frozen picture and silence, and at the frames around every cut, before it tells you it's done.
- **A tour on a sample video.** The first time you open Nuzky, Get set up offers a one-minute sample video and a short tour of where things are, with a scripted AI edit you can undo. Take the tour in the sidebar replays it.
- **Captions styled all at once.** A caption's style now changes all captions or just this one. Font, size, colour and case come first, and More style adds alignment, box opacity and corners.
- **Snapping and moving.** Moved, scaled and cropped layers snap to the frame's edges and middle. Drag several selected clips together on the timeline.
- **Covers with a logo.** Put images such as a logo on a cover, and see each project's cover on its card on the home screen.
- **Cleaner reels.** Reels start and end on the long video's own cuts, so none opens or closes with a flash of another shot.
- **Easier YouTube parts.** Choosing a part of a YouTube video shows the whole video with both handles, and holding a handle zooms in for frame-accurate trimming.

## Fixed

- Video in the preview, thumbnails and export is as bright as the original. Phone video, HDR included, used to come out slightly darker.
- Inserting a clip between clips on the main track moves the captions and overlays above along with them.
- Remove retakes and pauses keeps the last syllable of a word the speech model heard wrong.
- In Get set up, the captions model follows the spoken language you pick.
- The cover editor shows the cover without the video's overlays.

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

Nuzky is GPL-3.0-or-later, and its source is the Source code archive below. The installers include FFmpeg, ONNX Runtime and other libraries. Their source, and what each installer was built from, is in [Third-party source for Nuzky v0.1.1-alpha](https://github.com/nuzkyapp/nuzky/releases/tag/third-party-source-0.1.1-alpha).

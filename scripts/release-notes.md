The first alpha of Nuzky, a free, open-source video editor for your computer. It's early, so expect rough edges. Nuzky never changes or deletes your original videos.

## What it does

- **Captions in Czech and English.** Speech is recognised on your computer, with no limit. Pick a style and make the spoken word light up. The first run downloads the speech model.
- **Retakes and pauses.** Transcribe a talking video and delete words straight from the text, or press Remove retakes and pauses to cut restarted sentences, filler words and silences in one step. One Undo takes it back.
- **Export.** MP4 from 720p to 4K, with presets for Reels & TikTok, Shorts and YouTube and no watermark. The file looks exactly like the preview.
- **Phone videos.** iPhone and Android videos open the right way up, selfies included, and every cut lands on the frame you picked.
- **AI editing with Claude Code or Codex.** Connect agent sets up Claude Code or Codex in one click, and the AI panel chats with your own Claude Code or Codex. You watch it edit, Stop works at any time, and one Undo takes back the whole run. It runs on the Claude or ChatGPT plan you already have.
- Nuzky saves after every edit and recovers after a crash.

## Not yet

- AI editing doesn't work on Windows. The AI panel is off there.
- On Windows, speech recognition runs on the CPU, so captions take longer than on Linux and macOS, which use the GPU when they can.
- Nuzky doesn't update itself. It tells you when a new version is out, and you download it here.
- Intel Macs, Linux on ARM and Windows on ARM aren't supported.
- Export makes MP4 only. There's no template library and no phone app.

## Systems

macOS works and is where Nuzky is tested. Linux is only partly tested. Windows isn't tested at all: the installer builds, but nobody has tried it on a Windows PC yet. If something breaks on Linux or Windows, please report it.

- **macOS 14 or newer on Apple Silicon**: the `.dmg`. Open it and drag Nuzky to Applications.
- **Windows 10 or 11, x64**: the `-setup.exe`. The installer downloads Microsoft WebView2 if it's missing.
- **Linux, x86_64**: the `.deb` for Ubuntu 24.04 or newer (`sudo apt install ./Nuzky_*.deb`), or the `.AppImage` for other distributions with glibc 2.39 or newer. Make the AppImage executable with `chmod +x` and run it.

`SHA256SUMS` lists the checksums of the installers and the source archive.

## Opening an unsigned build

- **macOS**: Apple hasn't checked this build, so macOS won't open it the first time. Close the message, open System Settings → Privacy & Security, click Open Anyway next to Nuzky and confirm.
- **Windows**: Nuzky has no Microsoft signing certificate yet, so SmartScreen says "Windows protected your PC" the first time. Click More info, then Run anyway.

## Update check

Once a day Nuzky reads the version number of the latest release on this page, and nothing else. It downloads and installs nothing by itself. To turn the check off, click the version at the bottom of the home screen's sidebar and untick Check automatically.

## Found a bug?

Open an issue at https://github.com/nuzkyapp/nuzky/issues, or use Send feedback → Report a bug… in the app, which fills in your version and system. Say what you did and what happened, and please don't attach private videos. No GitHub account? Reply to a post or send a DM to [@mach_builds](https://x.com/mach_builds) on X.

## Source

Nuzky is GPL-3.0-or-later. `Nuzky_*_source.tar.gz` is the source code of this release. The installers include FFmpeg, ONNX Runtime and other libraries. Each `build-info-*.txt` lists their versions and where their source comes from, and their source code is attached to this release.

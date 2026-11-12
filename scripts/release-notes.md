The tenth alpha of Nuzky, a free, open-source video editor for your computer. It's early, so expect rough edges. Nuzky never changes or deletes your original videos.

## What's new

### Editing

- **Speed ramps and reverse.** A clip can play backwards and ramp into slow motion and out again. The Speed tab offers six ready shapes, and you can drag the points of the curve yourself.
- **Masks.** Show a video or picture only inside a circle or a rectangle, or on one side of a soft line. Keyframes move, resize and soften the mask over time.
- **Follow.** Draw a box around a person or a thing, and Nuzky keeps it in the frame through the whole clip, without downloading a model.
- **Split screen.** Select 2 to 4 clips and lay them out side by side, stacked, in columns, rows or a grid.
- **Adjustment layers.** A colour grade, LUT or effect on an adjustment layer changes everything below it for as long as it lasts.
- **More transitions, effects and keyframe curves.** Seven new transitions, seven effects of Nuzky's own (Blur, Zoom blur, Shake, Glitch, RGB split, Pixelate and Film grain), and keyframes that ease in or out.
- **Voiceover.** Record your voice over the video with your script on screen, a 3-2-1 countdown and a level meter that warns when you're too quiet or too loud.
- **Preview in its own window.** Shift+F opens the preview in a window you can move to another screen and make full screen there.

### Colour

- **Log footage and LUTs.** One click turns log footage into normal colour, for 17 profiles including Apple Log, Apple Log 2, Sony S-Log3, DJI D-Log and Fujifilm F-Log. You can also apply your own `.cube` LUTs.
- **Compare.** Every filter tile shows the original and the filtered picture side by side, and Compare puts a line over the preview with the original on one side.

### Text and captions

- **30 title styles and 9 more caption styles.** Point at a style and the preview shows what it would do before you click. Caption style tiles play a sample with karaoke and key words.
- **Key words by hand.** Click any caption word, or press K in the transcript, to show it in the key word colour.
- **English captions.** Add English captions under the Czech, translated by the AI you connected in the AI panel. Nuzky sends the text only to that AI.
- **One place for the look of captions.** A caption's inspector now sets the whole look of all captions. The Captions tab only makes them.

### Export

- **HDR.** Video shot in HDR can stay HDR. Choose HDR in the Export dialog to get 10-bit HEVC in HLG, the format iPhones record HDR in.
- **Captions as .srt files.** Save captions as `.srt` files beside the video, or alone with Export .srt only…, one file per language. Every save dialog of an export starts in the folder the project last exported to, so a video's files land together.
- **Resolution and frame rate in the top bar.** A chip before Export shows the quality the video exports at, such as FHD 30, and lets you change it with the file size it would make.

### Getting around

- **Find any action with Ctrl+K** (Cmd+K on a Mac). Type what you want to do, in English or Czech, and press Enter. Typos are fine.
- **CapCut's keys.** Nuzky asks new users once which editor's keys they know: Nuzky, CapCut, Final Cut Pro, DaVinci Resolve or Premiere Pro.
- **Updates are easier to see.** When a newer version is out, the editor says so in the top bar and once in a message.

### For AI agents

- Agents can use the new editing features too, apart from recording a voiceover and importing a LUT. They can also read when and where a clip was shot, look at a file's own frames, find a quiet place for a cut, hear mains hum, and ask what a clip shows and where a thing in it goes. That last one uses a scene model that runs on your computer and downloads only after you allow it (278 MB).

## Fixed

- On Linux with WebKitGTK 2.54, correcting a word in the transcript replaces it again instead of adding the new word to the old one.
- A text style's tile shows its whole name, smaller when it is wide, instead of cutting it short.
- The project's name in the editor's top bar sits in the middle of the window.

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

Nuzky is GPL-3.0-or-later, and its source is the Source code archive below. The installers include FFmpeg, ONNX Runtime and other libraries. Their source, and what each installer was built from, is in [Third-party source for Nuzky v0.1.10-alpha](https://github.com/nuzkyapp/nuzky/releases/tag/third-party-source-0.1.10-alpha).

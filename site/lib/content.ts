// Copy that the pages, the structured data for search engines and the Markdown for AI agents all share,
// so a fact changes in one place. It is written for creators who edit their videos, not for developers.

export type Faq = { q: string; a: string; link?: { label: string; href: string } };

export const maker = { name: "Tomáš Mach", photo: "/media/tomas.jpg", role: "Makes videos, builds Nuzky" };

// Why Nuzky exists, in Tomáš's words. Every line is his own experience, so keep it first person and true.
export const makerNote = [
  "I make talking-head videos for Instagram and TikTok. Talking is the fun part. Then come two or three hours of writing captions and cutting out every um, every pause and every retake.",
  "CapCut's cheapest paid plan costs 700 CZK a month here in Czechia, about $30. And its Czech captions were so bad that I retyped most of them anyway.",
  "So I'm building the editor I wanted. AI on your computer writes the captions, with the model that knows your language best and the names you actually say. The ChatGPT or Claude you already pay for makes the rough cut. It doesn't make videos for you. It does the boring part.",
  "Nuzky is free, and it stays free. There's no paid plan coming. The code is public, so nobody can take the free version away. Not even me.",
];

// What comes over from a CapCut draft (docs/INTERACTION.md, CapCut drafts). Keep the list to what the import reads.
export const capcutDraftParts = "Clips, cuts, speed, volume, keyframes, texts, captions and transitions";

export const faq: Faq[] = [
  {
    q: "Which computers does it run on?",
    a: "Macs with Apple silicon and macOS 14 or newer, where I test it. 64-bit Intel or AMD PCs with Linux and glibc 2.39 or newer, like Ubuntu 24.04, partly tested. Windows 10 or 11 on a 64-bit Intel or AMD PC, not tested yet, and AI editing is off there. Intel Macs, and Windows or Linux on ARM, aren't supported.",
    link: { label: "See the downloads", href: "#download" },
  },
  {
    q: "Can I open my CapCut projects?",
    a: `Yes, drafts from CapCut on a Mac or a Windows PC. Find them on Nuzky's home screen, or choose New project, then From a CapCut draft. ${capcutDraftParts} come over, a list names anything that didn't, and the draft in CapCut stays as it was.`,
  },
  {
    q: "Is Nuzky really free?",
    a: "Yes. No subscription, no watermark on your videos, no sign-up and no paid plan coming. The code is public, so it stays free. If you let AI do the cutting, it runs on the ChatGPT or Claude plan you already have.",
  },
  {
    q: "How is Nuzky different from CapCut?",
    a: "Nothing in Nuzky is locked behind a paid plan, so export never asks you to upgrade. It's made for editing on a computer, and it opens your CapCut drafts. CapCut has more templates and effects, and a phone app.",
    link: { label: "Compare Nuzky and CapCut", href: "/capcut-alternative" },
  },
  {
    q: "Are the auto captions free?",
    a: "Yes, with no limit. AI on your computer writes them from what you say. Pick the model that's best for your language, give it the names and words you use, and fix anything it still gets wrong. Then pick one of 19 styles or your own, and make the spoken word light up.",
  },
  {
    q: "Can AI edit the video for me?",
    a: "Yes, if you want it to. Type what you want, like “cut the ums and pauses and add captions”, and watch it edit in the app. Stop it any time, and one undo takes it all back. Sign in with ChatGPT inside Nuzky to use the plan you already pay for, with no terminal, or connect Claude Code or Codex. Editing, captions and export work without any AI. AI editing doesn't work on Windows yet.",
  },
  {
    q: "How do updates work?",
    a: "Nuzky tells you when a new version is out. One click downloads it inside the app, and it installs when you restart Nuzky, never by itself.",
  },
  {
    q: "Are my videos private?",
    a: "Yes. Your videos stay on your computer, and nothing gets uploaded to edit, caption or export them. If you ask an AI to edit for you, it sees what's said in the video and the frames it checks.",
  },
  {
    q: "Will videos from my phone work?",
    a: "Yes. iPhone and Android videos open the right way up, selfies included, and every cut lands on the exact frame you picked.",
  },
  {
    q: "Is it ready for TikTok, Reels, Shorts and YouTube?",
    a: "Yes. Pick the Reels & TikTok export and you get a vertical 1080 × 1920 video with the volume levelled for social apps. For YouTube, export widescreen at any size up to 4K.",
  },
  {
    q: "Who makes Nuzky?",
    a: "Tomáš Mach, a creator from Czechia who got tired of spending hours on captions and rough cuts. The name comes from nůžky, Czech for scissors.",
  },
];

// For search engines and AI agents, which want the full picture.
export const features = [
  "Opens CapCut drafts from your computer, with their clips, cuts, speed, volume, keyframes, texts, captions and transitions",
  "Vertical 9:16 canvas, plus 16:9, 1:1 and 4:5, and Reframe, which keeps the speaker's face in frame when the ratio changes",
  "Opens phone and camera footage, including HEVC, H.264, variable frame rate, rotated and front camera video",
  "Magnetic main track with overlay video, audio and text tracks",
  "Trim, split, move, copy and paste with snapping, undo and redo, and Versions to bring back an earlier state",
  "Edit by text: delete words in the transcript and the video cuts with them",
  "Remove retakes and pauses, including filler words inside sentences and hesitations the transcript doesn't show",
  "Keyframes, speed, entry and exit animations, transitions and filter presets",
  "Text with outline, background box, colour, position, rotation and opacity, plus built-in stickers and emoji that work offline",
  "Auto captions on your computer with no limit, a choice of models for your language, a list of the names and words you use, 19 caption styles plus your own, key word colours and a highlight on the spoken word",
  "Clean voice, Reduce echo, Even out volume and Match loudness",
  "Background: blur, a colour or an image behind the person in a video",
  "Reels from a long video: AI finds the moments, you trim them by words, and each becomes its own 9:16 project",
  "Covers and thumbnails: a 9:16 cover and a 16:9 YouTube thumbnail with text behind the person, Pick for me and Make it with AI",
  "A sound library of CC0 and CC BY music and effects from Openverse and Freesound, with credits to copy, and built-in effects that work offline",
  "Music and video from a YouTube or YouTube Music link you have the rights to, video up to 1080p",
  "Real-time preview with sound",
  "MP4 export from 720p to 4K with no watermark, drawn by the same renderer as the preview",
  "Reels and TikTok preset: 1080 × 1920 at 30 fps, sound levelled to −14 LUFS",
  "Saves after every edit and recovers after a crash",
  "AI editing in the app: sign in with ChatGPT, or use Claude Code, Codex or any MCP client, with one undo per run (not on Windows yet)",
  "Learns how a creator edits from their finished cuts or CapCut drafts into an EDIT.md that agents follow",
  "New versions download inside the app in one click, and install when you restart",
];

// What the comparison page says about CapCut was checked against capcut.com/pricing, CapCut's help centre
// and its online editor page. Recheck it when this date gets old.
export const capcutChecked = "October 2026";
export const capcutSources = [
  { label: "CapCut pricing", href: "https://www.capcut.com/pricing" },
  { label: "CapCut help on Pro features", href: "https://www.capcut.com/help/capcut-pro-prompt-standard-user" },
  { label: "CapCut online editor", href: "https://www.capcut.com/tools/online-video-editor" },
];

export const comparison: { label: string; nuzky: string; capcut: string }[] = [
  { label: "Price", nuzky: "Free and open source. No subscription.", capcut: "Free plan, then Pro or Ultra for premium features and AI credits" },
  { label: "Auto captions", nuzky: "Free, with no limit", capcut: "Some caption features need Pro" },
  { label: "Export", nuzky: "Up to 4K, never a watermark", capcut: "Videos that use Pro features need Pro to export" },
  {
    label: "Your videos",
    nuzky: "Stay on your computer. If you use AI editing, the AI reads what's said and looks at a few frames.",
    capcut: "The apps edit on your device. The web editor uploads them.",
  },
  {
    label: "AI editing",
    nuzky: "Type what to cut and watch it work. Sign in with ChatGPT inside the app, or use Claude Code or Codex, on the plan you already have. Not on Windows yet.",
    capcut: "Built-in AI tools, paid with credits",
  },
  {
    label: "Templates and effects",
    nuzky: "Filters, transitions, animations, stickers and 19 caption styles. No template library.",
    capcut: "A large library of templates, effects and stickers",
  },
  {
    label: "Where it runs",
    nuzky: "Mac with Apple silicon (tested), Linux (partly tested) and Windows (not tested yet)",
    capcut: "Phones, Mac, Windows and the web",
  },
];

export const pickNuzky = [
  "You're done paying a subscription to edit",
  "You want auto captions without paying for them",
  "You want your videos to stay on your computer",
  "You'd like AI to do the boring first cut",
];

export const pickCapcut = ["You edit on your phone", "You build videos from templates and effects", "You need your projects on every device"];

export const fromCapcut = [
  { title: "The same layout", text: "Media on the left, the preview in the middle, settings on the right and the timeline at the bottom." },
  { title: "Q and W", text: "Cut away the part before or after the playhead with one key, just like in CapCut." },
  { title: "Edit by text", text: "Delete words in the transcript and the video cuts with them, captions included." },
];

export const capcutFaq: Faq[] = [
  {
    q: "Can I open my CapCut projects in Nuzky?",
    a: `Yes, drafts from CapCut on a Mac or a Windows PC. ${capcutDraftParts} come over. A list names anything that didn't, like effects and colour adjustments, and the draft in CapCut stays as it was.`,
  },
  {
    q: "Are CapCut's auto captions free?",
    a: "Not entirely. CapCut's help centre says auto captions include features that need Pro. In Nuzky, captions are free.",
  },
  {
    q: "Who owns the videos I make in Nuzky?",
    a: "You do. Nuzky doesn't upload your videos and claims no rights to anything you make.",
  },
];

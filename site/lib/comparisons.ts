// Pages that compare Nuzky with another editor. The HTML, the structured data and the Markdown for AI agents
// are all built from this copy. What they say about Nuzky holds for the published release, never only for main;
// what they say about the other editor comes from the sources listed with it, so recheck it when `checked` gets old.

import { type Faq, capcutDraftParts } from "./content";

export type Comparison = {
  path: string;
  /** The other editor. */
  rival: string;
  /** Links and breadcrumbs. */
  name: string;
  /** The page's own title in shares and the Markdown. */
  title: string;
  /** What search results show. */
  metaTitle: string;
  description: string;
  heading: [string, string];
  intro: string;
  /** The Markdown's summary for AI agents, with what the other editor does better. */
  summary: string;
  /** What llms.txt says the page covers. */
  covers: string;
  rows: { label: string; nuzky: string; rival: string }[];
  pickNuzky: string[];
  pickRival: string[];
  stayWith: string;
  moving: { heading: [string, string]; mdHeading: string; items: { title: string; text: string }[] };
  faqHeading: string;
  faq: Faq[];
  sources: { label: string; href: string }[];
  checked: string;
  /** After the sources: that the other editor changes, the trademark and that Nuzky isn't affiliated. */
  fineprint: string;
};

export const comparisons: Comparison[] = [
  {
    path: "/capcut-alternative",
    rival: "CapCut",
    name: "Nuzky vs CapCut",
    title: "Nuzky vs CapCut: the open source CapCut alternative",
    metaTitle: "Free, open source CapCut alternative with no watermark | Nuzky",
    description: "Free auto captions, no watermark and no subscription. See how Nuzky, a free and open source video editor, compares with CapCut.",
    heading: ["The free", "CapCut alternative."],
    intro:
      "Nuzky is an open source video editor for TikTok, Reels, Shorts and YouTube, and it opens your CapCut drafts. Captions, 4K export and every effect cost nothing, so export never asks you to upgrade.",
    summary:
      "Nuzky is a free, open source video editor for TikTok, Reels, Shorts and YouTube. It opens your CapCut drafts, auto captions are free with no limit, exports go up to 4K with no watermark, and there is no subscription or account. It does not have CapCut's template library or a phone app.",
    covers: "Price, platforms, privacy and AI editing compared, with when each one fits better",
    // Checked against capcut.com/pricing, CapCut's help centre and its online editor page.
    rows: [
      { label: "Price", nuzky: "Free and open source. No subscription.", rival: "Free plan, then Pro or Ultra for premium features and AI credits" },
      { label: "Auto captions", nuzky: "Free, with no limit", rival: "Some caption features need Pro" },
      { label: "Export", nuzky: "Up to 4K, never a watermark", rival: "Videos that use Pro features need Pro to export" },
      {
        label: "Your videos",
        nuzky: "Stay on your computer. If you use AI editing, the AI reads what's said and looks at a few frames.",
        rival: "The apps edit on your device. The web editor uploads them.",
      },
      {
        label: "AI editing",
        nuzky: "Type what to cut and watch it work. Sign in with ChatGPT inside the app, or use Claude Code or Codex, on the plan you already have.",
        rival: "Built-in AI tools, paid with credits",
      },
      {
        label: "Templates and effects",
        nuzky: "Filters, transitions, animations, stickers and 19 caption styles. No template library.",
        rival: "A large library of templates, effects and stickers",
      },
      { label: "Where it runs", nuzky: "Mac with Apple silicon, Linux and Windows", rival: "Phones, Mac, Windows and the web" },
    ],
    pickNuzky: [
      "You're done paying a subscription to edit",
      "You want auto captions without paying for them",
      "You want your videos to stay on your computer",
      "You'd like AI to do the boring first cut",
    ],
    pickRival: ["You edit on your phone", "You build videos from templates and effects", "You need your projects on every device"],
    stayWith: "Stay with CapCut if",
    moving: {
      heading: ["Coming from CapCut?", "You already know your way around."],
      mdHeading: "Coming from CapCut",
      items: [
        { title: "The same layout", text: "Media on the left, the preview in the middle, settings on the right and the timeline at the bottom." },
        { title: "Q and W", text: "Cut away the part before or after the playhead with one key, just like in CapCut." },
        { title: "Edit by text", text: "Delete words in the transcript and the video cuts with them, captions included." },
      ],
    },
    faqHeading: "Questions from CapCut users",
    faq: [
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
    ],
    sources: [
      { label: "CapCut pricing", href: "https://www.capcut.com/pricing" },
      { label: "CapCut help on Pro features", href: "https://www.capcut.com/help/capcut-pro-prompt-standard-user" },
      { label: "CapCut online editor", href: "https://www.capcut.com/tools/online-video-editor" },
    ],
    checked: "October 2026",
    fineprint:
      "CapCut's plans change, so check them for the current details. CapCut is a trademark of ByteDance. Nuzky is an independent project and isn't affiliated with ByteDance or CapCut.",
  },
  {
    path: "/edits-alternative",
    rival: "Edits",
    name: "Nuzky vs Edits",
    title: "Nuzky vs Edits: a free Edits alternative for Mac, Windows and Linux",
    metaTitle: "Free Edits alternative for Windows, Mac and Linux | Nuzky",
    description:
      "Looking for Instagram's Edits on Windows or Linux? Nuzky is a free, open source video editor for Reels, TikTok and Shorts on Mac, Windows and Linux, with captions written on your computer and no account.",
    heading: ["The open source", "Edits alternative."],
    intro:
      "Edits runs on phones and Macs with Apple silicon, with no Windows version yet. Nuzky is a free, open source video editor for Reels, TikTok, Shorts and YouTube on Windows, Linux and Mac. It needs no account and opens your CapCut drafts.",
    summary:
      "Nuzky is a free, open source video editor for Reels, TikTok, Shorts and YouTube on Mac, Windows and Linux. It needs no account, writes captions on your computer, opens your CapCut drafts and lets ChatGPT, Claude Code, Codex or any MCP client edit the project you have open. It does not post straight to Instagram or have a phone app.",
    covers: "Platforms, account, captions, AI editing and publishing compared, with when each one fits better",
    // Checked against the launch coverage of Edits for Mac on 5 November 2026. Instagram's own pages sit behind a
    // cookie wall, so the sources are the reports that quote them.
    rows: [
      { label: "Price", nuzky: "Free and open source. No subscription.", rival: "Free" },
      { label: "Account", nuzky: "None. Download it and edit.", rival: "Sign in with an Instagram account" },
      {
        label: "Where it runs",
        nuzky: "Mac with Apple silicon and macOS 15 or newer, Windows 10 or 11, and Linux",
        rival: "Phones, and Macs with Apple silicon. No Windows version yet.",
      },
      { label: "Auto captions", nuzky: "Free with no limit, written on your computer in your language", rival: "Free with no limit, in several languages" },
      { label: "Export", nuzky: "Up to 4K, never a watermark", rival: "Up to 4K without a watermark" },
      {
        label: "AI editing",
        nuzky: "ChatGPT, Claude Code, Codex or any MCP client edits the project you have open while you watch. Stop it any time, and one undo takes the whole run back.",
        rival: "Connect an AI agent over MCP, such as Meta's Muse",
      },
      { label: "Publishing", nuzky: "Exports a video file you post anywhere", rival: "Posts straight to Instagram with tags and location, or exports a file" },
    ],
    pickNuzky: [
      "You edit on Windows or Linux",
      "You'd rather not sign in to edit",
      "You want captions written on your computer",
      "You have drafts in CapCut to finish",
    ],
    pickRival: ["You post to Instagram straight from the editor", "You edit in Edits on your phone and want your saved audio and text styles on the Mac"],
    stayWith: "Stay with Edits if",
    moving: {
      heading: ["Moving to Nuzky?", "Bring what you already have."],
      mdHeading: "Moving to Nuzky",
      items: [
        { title: "Your CapCut drafts", text: "On a Mac or a Windows PC, Nuzky lists them on its home screen and opens them. The draft in CapCut stays as it was." },
        { title: "Your way of editing", text: "Give Nuzky raw recordings and the videos you cut from them. It writes down how you edit, and the AI editor follows it." },
        { title: "No account", text: "Download it and start editing. AI editing runs on the ChatGPT or Claude plan you already have." },
      ],
    },
    faqHeading: "Questions from Edits users",
    faq: [
      {
        q: "Is there Edits for Windows or Linux?",
        a: "No. When Edits came to the Mac in November 2026, it ran only on Macs with Apple silicon, and there was no Windows version yet. Nuzky runs on Windows 10 and 11, on Linux and on Macs with Apple silicon.",
      },
      {
        q: "Can I open my Edits projects in Nuzky?",
        a: "No. Export the video from Edits, or bring your original recordings and edit them in Nuzky.",
      },
      {
        q: "Will Nuzky run on my Mac?",
        a: "Yes, on a Mac with Apple silicon (M1 or newer) and macOS 15 or newer. Intel Macs aren't supported.",
      },
      {
        q: "Can AI edit my video in Nuzky?",
        a: "Yes. ChatGPT, Claude Code or Codex edits the project open in the app while you watch. Your controls lock with a Stop button, and one undo takes the whole run back. It runs on the plan you already have.",
      },
      {
        q: "Can I post to Instagram from Nuzky?",
        a: "Not directly. Nuzky exports a video file, vertical 1080 × 1920 for Reels, and you post it from Instagram as usual.",
      },
    ],
    sources: [
      { label: "MacRumors", href: "https://www.macrumors.com/2026/11/05/instagram-edits-mac/" },
      { label: "9to5Mac", href: "https://9to5mac.com/2026/11/05/instagram-launches-edits-for-mac-with-multi-track-video-editing-and-4k-exports/" },
      { label: "The Next Web", href: "https://thenextweb.com/news/instagram-launches-free-edits-video-editor-for-mac-with-4k-exports" },
      { label: "CineD", href: "https://www.cined.com/instagram-edits-for-mac-released-multi-track-timeline-lut-import-and-watermark-free-4k-export/" },
    ],
    checked: "November 2026",
    fineprint:
      "Instagram plans to update Edits for Mac every week, so check it for the current details. Edits and Instagram are trademarks of Meta. Nuzky is an independent project and isn't affiliated with Meta or Instagram.",
  },
];

export const comparison = (path: string) => comparisons.find((c) => c.path === path)!;

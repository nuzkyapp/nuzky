// Pages that each answer one thing people search for. The HTML, the structured data and the Markdown for AI
// agents are all built from this copy. Every claim comes from docs/INTERACTION.md, so check it there first.

import type { EditorFocus } from "@/components/EditorMock";
import { type Faq, capcutDraftParts } from "./content";

export type Guide = {
  path: string;
  /** Links and breadcrumbs. */
  name: string;
  /** What search results show. It names what people search for. */
  title: string;
  description: string;
  heading: [string, string];
  /** Over the steps, in the page's own words. */
  stepsHeading: [string, string];
  intro: string;
  focus: EditorFocus;
  steps: { title: string; text: string }[];
  details: string[];
  faq: Faq[];
};

export const captionLanguages = "Czech, Slovak, English, German, Polish, Spanish, French, Italian and Ukrainian";

export const guides: Guide[] = [
  {
    path: "/auto-captions",
    name: "Auto captions",
    title: "Free auto captions for TikTok, Reels and Shorts, no watermark",
    description:
      "Nuzky writes captions from what you say, on your computer, with no limit and no watermark. Pick a style, fix any word and export a vertical video for TikTok, Reels or Shorts.",
    heading: ["Free auto captions.", "Written on your computer."],
    stepsHeading: ["You talk.", "Nuzky types."],
    intro:
      "Nuzky is a free video editor that writes your captions from what you say. AI runs on your computer, so your video is never uploaded and there is no limit on how many videos you caption.",
    focus: "captions",
    steps: [
      {
        title: "Open your video",
        text: "Drop in a video from your phone or camera. iPhone and Android videos open the right way up, selfies included.",
      },
      {
        title: "Pick the language",
        text: `Nuzky captions ${captionLanguages}. Pick yours or let Nuzky detect it, then pick Fast, Balanced or Most accurate, and add the names and words you use.`,
      },
      {
        title: "Generate the captions",
        text: "Choose 1, 2 or 3 words per caption, or whole phrases. Captions appear on their own track, timed to every word.",
      },
      {
        title: "Fix and style them",
        text: "Double-click a misheard word and type it. Pick one of 19 styles or your own, colour key words and light up the spoken word.",
      },
      {
        title: "Export for Reels, TikTok or Shorts",
        text: "Export a vertical 1080 × 1920 video with no watermark, or widescreen up to 4K for YouTube.",
      },
    ],
    details: [
      "Free, with no limit on videos or minutes",
      "Speech is recognised on your computer, so nothing is uploaded",
      "The model downloads once, then captions work offline",
      "On vertical videos, captions stay clear of the buttons and text TikTok and Instagram put over the video",
      "Cut words out later and the captions follow the cut",
      "A word you corrected stays corrected when you generate the captions again",
    ],
    faq: [
      {
        q: "Are the auto captions really free?",
        a: "Yes. There's no subscription, no credits and no watermark, and no limit on how many videos you caption. Nuzky is free and open source.",
      },
      {
        q: "Which languages can it caption?",
        a: `${captionLanguages}. Or choose Detect automatically and Nuzky works out the language itself.`,
      },
      {
        q: "Is my video uploaded to make the captions?",
        a: "No. AI on your computer listens to the video and writes the captions. Only the model is downloaded, once.",
      },
      {
        q: "What if it gets a word wrong?",
        a: "Double-click the word in the transcript, or select it and press F2, and type what you said. The captions change with it. Add names and words you use often, so it gets them right next time.",
      },
      {
        q: "Will the captions hide behind TikTok's buttons?",
        a: "No. On a vertical video, captions keep to the area Instagram Reels and TikTok leave free, and longer captions wrap instead of running under the buttons.",
      },
    ],
  },
  {
    path: "/remove-filler-words",
    name: "Remove filler words",
    title: "Remove ums, pauses and retakes from a video automatically",
    description:
      "Cut filler words, long pauses and retakes out of a talking-head video in one click, or delete words from the transcript. Free and open source, with one undo for the whole cut.",
    heading: ["Cut the ums and retakes.", "In one click."],
    stepsHeading: ["Say it as many times as you need.", "Only the last good take stays."],
    intro:
      "Nuzky turns your recording into text. Then one button cuts the retakes, the filler words and the long pauses, and you edit the rest like a document. It's free, and one undo takes the whole cut back.",
    focus: "edit",
    steps: [
      {
        title: "Transcribe the video",
        text: "Nuzky writes down every word you say, on your computer. Pauses show as chips with their length.",
      },
      {
        title: "Remove retakes and pauses",
        text: "One button keeps the last complete take of every sentence you started again, and cuts filler words, hesitations and pauses longer than half a second.",
      },
      {
        title: "Delete what else should go",
        text: "Select words in the transcript and press Delete. The video cuts with them, and so do the captions.",
      },
      {
        title: "Check it and export",
        text: "Watch the result. A note says what went and how much time it saved, with Undo. Then export for Reels, TikTok, Shorts or YouTube.",
      },
    ],
    details: [
      "Cuts the earlier attempts of a sentence you said again, and keeps the last complete one",
      "Finds filler words in English and Czech, and hesitations like “eee” that the transcript didn't write, in any language",
      "Long pauses shrink to the length you set, 0.5 s by default",
      "Two sentences that differ in a number or a “not” both stay",
      "Every cut keeps a little silence around the words, so no syllable gets clipped",
      "Captions and overlays stay in sync with every cut",
    ],
    faq: [
      {
        q: "Which languages does it work in?",
        a: "Retakes, hesitations and pauses are cut in every language Nuzky transcribes. Filler words are looked for in English and Czech, where a word like “um” can't mean something else.",
      },
      {
        q: "Can I undo it?",
        a: "Yes. The whole cleanup is one step, so one undo brings everything back.",
      },
      {
        q: "Can AI do the rough cut for me?",
        a: "Yes. Ask ChatGPT or Claude in Nuzky to cut the ums and pauses and keep the best takes, and watch it edit. It runs on the plan you already pay for.",
        link: { label: "AI video editing in Nuzky", href: "/ai-video-editor" },
      },
      {
        q: "Is it free?",
        a: "Yes. Nuzky is free and open source, with no subscription and no watermark.",
      },
    ],
  },
  {
    path: "/ai-video-editor",
    name: "AI video editor",
    title: "AI video editor that works with ChatGPT and Claude",
    description:
      "Ask ChatGPT, Claude Code or Codex to cut your video in plain words and watch it edit in Nuzky, a free, open source video editor. It runs on the plan you already pay for.",
    heading: ["Your ChatGPT or Claude.", "Editing your video."],
    stepsHeading: ["Tell it what to cut.", "Watch it cut."],
    intro:
      "Nuzky is a free, open source video editor that AI can edit for you. Type what you want, like “cut the ums and pauses, add captions and music”, and watch it work in the open project. Stop it any time, and one undo takes the whole run back.",
    focus: "edit",
    steps: [
      {
        title: "Pick your AI",
        text: "Sign in with ChatGPT inside Nuzky, with no terminal. Or use Claude Code, Codex or OpenCode, or connect Claude Desktop, Cursor or any MCP client.",
      },
      {
        title: "Say what you want",
        text: "Cut the ums and long pauses, keep the best takes, add captions, put music under the video, export for Reels. Attach a screenshot or a video if it helps.",
      },
      {
        title: "Watch it edit",
        text: "The AI edits the project open in Nuzky while you watch. Your controls lock while it works, with a Stop button.",
      },
      {
        title: "Keep it or undo it",
        text: "Every change goes through the same undo as yours, and one undo takes the whole run back.",
      },
    ],
    details: [
      "Runs on the ChatGPT or Claude plan you already have, with no credits to buy",
      "Pick the model and how hard it thinks, and come back to past chats",
      "Learns how you edit from your finished videos or CapCut drafts, and writes it into an EDIT.md that every AI follows",
      "Your videos stay on your computer. The AI sees what's said in them and the frames it checks",
      "Everything works without AI too: editing, captions and export",
    ],
    faq: [
      {
        q: "Do I need to pay for AI credits?",
        a: "No. Nuzky uses the ChatGPT or Claude plan you already pay for. Nuzky itself is free.",
      },
      {
        q: "Does it work without a terminal?",
        a: "Yes. Sign in with ChatGPT inside Nuzky and type in its AI panel. Claude Code, Codex and OpenCode need to be installed on your computer.",
      },
      {
        q: "Can AI ruin my project?",
        a: "No. You see every change as it happens, Stop ends the run within seconds, and one undo takes the whole run back. Your original videos are never changed.",
      },
      {
        q: "Can it open my CapCut drafts?",
        a: `Yes, drafts from CapCut on a Mac or a Windows PC. ${capcutDraftParts} come over. Then press Finish with AI to hand the rest to ChatGPT or Claude.`,
        link: { label: "Nuzky vs CapCut", href: "/capcut-alternative" },
      },
    ],
  },
];

export const guide = (path: string) => guides.find((g) => g.path === path)!;

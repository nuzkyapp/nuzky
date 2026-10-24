"use client";

import Image from "next/image";
import { useEffect, useRef, useState } from "react";
import { ChevronRight } from "lucide-react";
import { Fit } from "./Fit";
import { HeroDownload } from "./GetNuzky";
import { LaunchVideo } from "./LaunchVideo";
import { fitSizes } from "@/lib/fit";
import { usePlaying } from "@/lib/playing";
import type { Release } from "@/lib/site";

// One sunrise on one ridge runs through every step, so what goes in is what comes out: tile-2 is the summit
// clip the reel shows, its last take is the one kept, and its words are the captions. The two photos are crops
// of the wide timeline shots of the same morning, so they need a source four times as wide as the tile.
const clips = [
  { src: "/media/tile-1.jpg", length: "2:05" },
  { src: "/media/timeline-1.jpg", wide: true },
  { src: "/media/tile-2.jpg", length: "0:48", inReel: true },
  { src: "/media/timeline-2.jpg", wide: true },
];

const takes = ["So we, um, made it", "We made it to the… wait", "We made it to the top."];

const captions = [
  { at: "00:01", words: ["Up", "at", "five,"] },
  { at: "00:03", words: ["on", "the", "ridge", "at", "six,"] },
  { at: "00:05", words: ["we", "made", "it", "to", "the", "top."] },
];

// Where the voice talks, in % of the length. The music's waveform is quieter there, as ducking makes it.
const voice = [
  [0, 22],
  [30, 60],
  [70, 96],
];
const BARS = 34;
const wave = Array.from({ length: BARS }, (_, i) => {
  const p = (i / BARS) * 100;
  const talking = voice.some(([from, to]) => p >= from && p < to);
  return talking ? 6 + ((i * 37) % 11) * 0.5 : 18 + ((i * 37) % 11) * 1.8;
});

const titles = ["Your videos and photos", "Cuts the ums and retakes", "Captions in your language", "Music you're allowed to use", "Ready for Reels, TikTok and Shorts"];

const spokenWords = captions.flatMap((c, row) => c.words.map((_, word) => ({ row, word })));
// The last word is lit whenever the walk is not running: that is what the server renders and what reduced motion keeps.
const LAST = spokenWords.length - 1;

// Walks like the spoken word in the app's preview, with a breath at the end.
function useSpokenWord(on: boolean) {
  const [spoken, setSpoken] = useState(LAST);
  useEffect(() => {
    if (!on) return;
    let id = 0;
    const next = (i: number) => {
      setSpoken(i);
      id = window.setTimeout(() => next((i + 1) % spokenWords.length), i === LAST ? 1300 : 380);
    };
    id = window.setTimeout(() => next(0), 1300);
    return () => {
      window.clearTimeout(id);
      setSpoken(LAST);
    };
  }, [on]);
  return spokenWords[spoken];
}

type Spoken = (typeof spokenWords)[number];

const shadow = "drop-shadow(0 2px 8px rgb(0 0 0 / 0.7))";
const bare = (w: string) => w.replace(/[,.]$/, "");

function Clip({ clip, size, sizes }: { clip: (typeof clips)[number]; size: number; sizes: string }) {
  return (
    <div
      className={`relative overflow-hidden rounded-[8px] ${clip.inReel ? "ring-2 ring-accent" : ""}`}
      style={{ width: size, height: size }}
    >
      <Image src={clip.src} alt="" fill sizes={sizes} className={`object-cover ${clip.wide ? "object-left" : ""}`} />
    </div>
  );
}

function Reel({ w, spoken, sizes }: { w: number; spoken: Spoken; sizes: string }) {
  return (
    <div className="relative shrink-0 overflow-hidden rounded-[12px] ring-1 ring-white/15" style={{ width: w, height: Math.round((w * 16) / 9) }}>
      <Image src="/media/reel-hike.jpg" alt="The finished reel: two hikers on a summit at sunrise, captioned as they speak" fill sizes={sizes} className="object-cover" />
      <div
        aria-hidden
        className="absolute inset-x-2 top-[60%] flex flex-wrap justify-center gap-x-[0.26em] font-black uppercase leading-[1.12] tracking-[-0.015em]"
        style={{ filter: shadow, fontSize: Math.round(w / 9) }}
      >
        {captions[spoken.row].words.map((word, i) => (
          <span key={i} className={`transition-colors duration-150 ${i === spoken.word ? "text-caption-yellow" : ""}`}>
            {bare(word)}
          </span>
        ))}
      </div>
    </div>
  );
}

// Screen readers skip the visible number: the order of the steps already says it.
function Num({ n, size = 24, last = false }: { n: number; size?: number; last?: boolean }) {
  return (
    <span
      aria-hidden
      className={`flex shrink-0 items-center justify-center rounded-full font-semibold ${last ? "bg-accent-strong" : "ring-1 ring-inset ring-white/20"}`}
      style={{ width: size, height: size, fontSize: size / 2 }}
    >
      {n}
    </span>
  );
}

const WIDE = 1180;

function Pipeline({ spoken }: { spoken: Spoken }) {
  const visuals = [
    <div key="clips" aria-hidden className="grid grid-cols-2 gap-2">
      {clips.map((c) => (
        <div key={c.src} className="relative">
          <Clip clip={c} size={64} sizes={c.wide ? fitSizes(256, WIDE) : fitSizes(64, WIDE)} />
          {c.length && (
            <span className="absolute bottom-1 right-1 rounded-[4px] bg-black/65 px-1 text-[10px] font-medium leading-[14px] tabular-nums">{c.length}</span>
          )}
        </div>
      ))}
    </div>,
    <div key="takes" className="flex w-full flex-col gap-1.5 text-[11.5px] leading-[1.3]">
      {takes.map((t, i) => {
        const kept = i === takes.length - 1;
        return (
          <div key={t} className={`flex flex-col gap-0.5 rounded-[9px] px-2.5 py-2 ${kept ? "bg-accent/[0.16] ring-1 ring-inset ring-accent/50" : "bg-white/[0.03] text-muted"}`}>
            <span className="text-[10.5px] text-muted">
              Take {i + 1}
              {kept && " · kept"}
            </span>
            {kept ? <span>“{t}”</span> : <del>“{t}”</del>}
          </div>
        );
      })}
    </div>,
    <div key="captions" className="flex w-full flex-col gap-1.5">
      {captions.map((c, row) => (
        <div
          key={c.at}
          className={`flex gap-2 rounded-[9px] px-2.5 py-2 text-[12px] leading-[1.3] transition-colors duration-150 ${
            row === spoken.row ? "bg-accent/[0.18] ring-1 ring-inset ring-accent/50" : "bg-white/[0.03]"
          }`}
        >
          <span className="pt-px text-[10.5px] tabular-nums text-muted">{c.at}</span>
          {c.words.join(" ")}
        </div>
      ))}
    </div>,
    <div key="music" className="flex w-full flex-col gap-1.5">
      <div aria-hidden className="flex flex-col gap-1.5">
        <span className="text-[11px] text-muted">Your voice</span>
        <div className="relative h-3.5">
          {voice.map(([from, to]) => (
            <span key={from} className="absolute inset-y-0 rounded-[3px] bg-clip-video" style={{ left: `${from}%`, width: `${to - from}%` }} />
          ))}
        </div>
        <div className="mt-1 flex justify-between text-[11px]">
          <span className="font-semibold">Sunrise</span>
          <span className="text-muted">CC0</span>
        </div>
        <div className="flex h-11 items-center justify-between rounded-[6px] bg-clip-audio px-1.5">
          {wave.map((h, i) => (
            <span key={i} className="w-[2px] rounded-full bg-white/60" style={{ height: h }} />
          ))}
        </div>
      </div>
      <span className="text-[11px] text-muted">Quieter while you talk</span>
    </div>,
    <Reel key="reel" w={120} spoken={spoken} sizes={fitSizes(120, WIDE)} />,
  ];

  return (
    <ol className="flex h-full gap-10 text-left">
      {visuals.map((visual, i) => {
        const last = i === visuals.length - 1;
        return (
          <li
            key={titles[i]}
            className={`surface relative flex h-full flex-col rounded-[20px] ${last ? "w-[236px] border-[1.5px] border-accent" : "w-[196px]"}`}
          >
            <div className="flex h-[66px] items-start gap-2.5 px-4 pt-4">
              <Num n={i + 1} last={last} />
              <p className="pt-0.5 text-[15px] font-semibold leading-[1.25] tracking-[-0.01em]">{titles[i]}</p>
            </div>
            <div className="flex flex-1 items-center justify-center px-4 pb-4">{visual}</div>
            {!last && (
              <span aria-hidden className="absolute -right-8 top-[184px] flex size-6 items-center justify-center rounded-full bg-white/[0.06] text-muted">
                <ChevronRight className="size-3.5" />
              </span>
            )}
          </li>
        );
      })}
    </ol>
  );
}

const NARROW = 380;

function PipelineNarrow({ spoken }: { spoken: Spoken }) {
  const minis = [
    <div key="clips" className="grid grid-cols-2 gap-0.5">
      {clips.map((c) => (
        <Clip key={c.src} clip={c} size={19} sizes={c.wide ? "76px" : "19px"} />
      ))}
    </div>,
    <div key="takes" className="flex size-full items-center justify-center rounded-[8px] bg-white/[0.05] text-[13px] font-semibold text-muted">
      <del>um</del>
    </div>,
    <div key="captions" className="relative size-full overflow-hidden rounded-[8px]">
      <Image src="/media/tile-2.jpg" alt="" fill sizes="40px" className="object-cover" />
      <span className="absolute inset-x-0 bottom-1 text-center text-[9px] font-black uppercase text-caption-yellow" style={{ filter: shadow }}>
        {bare(captions[spoken.row].words[spoken.word])}
      </span>
    </div>,
    <div key="music" className="flex size-full items-center justify-between rounded-[8px] bg-clip-audio px-1.5">
      {wave
        .filter((_, i) => i % 4 === 0)
        .map((h, i) => (
          <span key={i} className="w-[2px] rounded-full bg-white/60" style={{ height: h * 0.75 }} />
        ))}
    </div>,
  ];

  return (
    <div className="flex h-full gap-4 text-left">
      <ol className="flex min-w-0 flex-1 flex-col gap-2">
        {minis.map((mini, i) => (
          <li key={titles[i]} className="surface flex flex-1 items-center gap-2.5 rounded-[14px] px-2.5">
            <Num n={i + 1} size={20} />
            <div aria-hidden className="size-10 shrink-0">
              {mini}
            </div>
            <p className="text-[14px] font-semibold leading-[1.25] tracking-[-0.01em]">{titles[i]}</p>
          </li>
        ))}
      </ol>
      <div className="flex w-[140px] shrink-0 flex-col gap-2">
        <Reel w={140} spoken={spoken} sizes="140px" />
        <div className="flex items-start gap-1.5">
          <Num n={5} size={20} last />
          <p className="text-[13px] font-semibold leading-[1.3]">{titles[4]}</p>
        </div>
      </div>
    </div>
  );
}

export function Hero({ release }: { release: Release }) {
  const section = useRef<HTMLElement>(null);
  const spoken = useSpokenWord(usePlaying(section));

  return (
    <section ref={section} className="relative">
      <div className="relative z-10 flex flex-col items-center px-5 pt-12 text-center sm:pt-20">
        <h1 className="heading-xl">
          Recording is the fun part.
          <br />
          <span className="text-subtle">Nuzky does the boring one.</span>
        </h1>
        <p className="mt-7 max-w-[600px] text-[17px] text-muted sm:text-[20px]">
          A free desktop video editor that opens your CapCut drafts. Your own ChatGPT or Claude cuts while you watch, and one undo takes it all back.
        </p>

        <LaunchVideo className="mt-10 sm:mt-12" />
        <span data-waves="swell" aria-hidden className="block w-full" />

        <div className="mt-10 w-full sm:mt-12">
          <HeroDownload release={release} />
        </div>

        <div className="mt-14 flex w-full justify-center sm:mt-20">
          <Fit width={WIDE} height={330} className="hidden lg:block">
            <Pipeline spoken={spoken} />
          </Fit>
          <Fit width={NARROW} height={308} className="lg:hidden">
            <PipelineNarrow spoken={spoken} />
          </Fit>
        </div>
      </div>
    </section>
  );
}

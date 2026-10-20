import Image from "next/image";
import { ChevronRight, CircleMinus, FolderOpen, LayoutGrid, ListChecks, Plus, Scissors, WandSparkles } from "lucide-react";
import { SectionHeading } from "./SectionHeading";
import { capcutDraftParts } from "@/lib/content";

// The home screen's CapCut drafts row and the dialog the app shows after opening one (docs/INTERACTION.md,
// CapCut drafts). Both are pictures of the app, so they are hidden from screen readers; the steps under them say it.
const drafts = [
  { src: "/media/ugc.jpg", name: "Talking reel", when: "Yesterday", length: "0:41" },
  { src: "/media/selfie.jpg", name: "Selfie Q&A", when: "Saturday", length: "0:28" },
  { src: "/media/reel-hike.jpg", name: "Ridge sunrise", when: "Oct 12", length: "0:30" },
];

const notCarried = ["Color adjustments", "Effects", "Transition: Woosh"];

function Step({ n, text, children }: { n: number; text: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col items-center justify-center lg:h-[340px]">{children}</div>
      <div className="flex items-start gap-3">
        <span aria-hidden className="flex size-6 shrink-0 items-center justify-center rounded-full text-[12px] font-semibold ring-1 ring-inset ring-white/20">
          {n}
        </span>
        <p className="text-[15px] leading-[1.45] text-fg/85">{text}</p>
      </div>
    </div>
  );
}

function Home() {
  return (
    <div aria-hidden className="flex w-full max-w-[470px] overflow-hidden rounded-[16px] bg-bg text-left ring-1 ring-white/10">
      <div className="hidden w-[128px] shrink-0 flex-col gap-0.5 border-r border-white/[0.06] p-2 text-[11px] sm:flex">
        <span className="flex h-7 items-center gap-2 rounded-[7px] bg-white/[0.07] px-2">
          <LayoutGrid className="size-3 text-accent" />
          All projects
        </span>
        <span className="flex h-7 items-center gap-2 px-2 text-fg/80">
          <Scissors className="size-3 text-muted" />
          Your style
        </span>
        <span className="flex h-7 items-center gap-2 px-2 text-fg/80">
          <ListChecks className="size-3 text-muted" />
          Get set up
        </span>
      </div>
      <div className="flex min-w-0 flex-1 flex-col p-3.5">
        <div className="flex items-center justify-between gap-2">
          <span className="text-[13px] font-semibold">All projects</span>
          <span className="flex h-6 items-center gap-1 rounded-[7px] bg-accent-strong px-2 text-[10.5px] font-medium text-white">
            <Plus className="size-3" />
            New project
          </span>
        </div>
        <div className="mt-4 flex items-baseline justify-between">
          <span className="text-[11.5px] font-semibold">
            CapCut drafts <span className="font-normal text-muted">3</span>
          </span>
          <FolderOpen className="size-3 text-muted" />
        </div>
        <div className="mt-2 grid grid-cols-3 gap-2.5">
          {drafts.map((d, i) => (
            <div key={d.name} className="flex flex-col gap-1.5">
              <div className={`relative h-[148px] overflow-hidden rounded-[10px] bg-raised ${i === 0 ? "ring-2 ring-accent" : "ring-1 ring-inset ring-white/[0.06]"}`}>
                <Image src={d.src} alt="" fill sizes="120px" className="object-cover" />
                <span className="absolute bottom-1.5 right-1.5 rounded-[5px] bg-black/65 px-[5px] py-px text-[9.5px] font-medium tabular-nums">{d.length}</span>
              </div>
              <span className="flex flex-col text-[10.5px] leading-[1.35]">
                <span className="truncate">{d.name}</span>
                <span className="text-muted">{d.when}</span>
              </span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function Opened() {
  return (
    <div aria-hidden className="w-full max-w-[470px] rounded-[18px] bg-[#232327] p-5 text-left shadow-[0_24px_60px_rgb(0_0_0/0.55)] ring-1 ring-white/10 sm:p-6">
      <p className="text-[16px] font-semibold tracking-[-0.01em] sm:text-[17px]">Opened “Talking reel” from CapCut</p>
      <p className="mt-1.5 text-[13px] leading-[1.5] text-fg/75">7 clips, 1 sound, 3 captions, 1 title and 1 zoom came over. These did not:</p>
      <ul className="mt-3 rounded-[12px] bg-white/[0.035] px-3 ring-1 ring-inset ring-white/[0.07]">
        {notCarried.map((t, i) => (
          <li key={t} className={`flex h-9 items-center gap-2.5 text-[13px] ${i > 0 ? "border-t border-white/[0.06]" : ""}`}>
            <CircleMinus className="size-[15px] text-muted" />
            {t}
          </li>
        ))}
      </ul>
      <div className="mt-5 flex flex-wrap items-center gap-x-2 gap-y-3">
        <span className="mr-auto text-[12px] text-muted">The draft in CapCut stays as it was.</span>
        <span className="capsule flex h-8 items-center gap-1.5 rounded-full px-3 text-[12.5px] font-medium">
          <WandSparkles className="size-3.5 text-accent" />
          Finish with AI
        </span>
        <span className="flex h-8 items-center rounded-full bg-accent-strong px-4 text-[12.5px] font-medium text-white">Done</span>
      </div>
    </div>
  );
}

const facts = [
  ["What comes over", `${capcutDraftParts}.`],
  ["Your draft stays as it was", "Nuzky makes its own project from the draft. Nothing changes in CapCut."],
] as const;

export function CapcutDrafts() {
  return (
    <section id="capcut" className="mx-auto w-full max-w-[1240px] scroll-mt-10 px-5 pt-32 sm:pt-[180px]">
      <SectionHeading center first="Your CapCut drafts open here." second="With the cuts, captions and keyframes." />
      <p className="mx-auto mt-6 max-w-[640px] text-center text-[17px] leading-[1.55] text-muted">
        Nuzky&apos;s home screen lists the CapCut drafts on your computer. Open one and keep editing where you left off, in an editor that never asks you to upgrade.
      </p>

      <div className="surface mt-12 grid gap-10 rounded-3xl p-5 sm:p-9 lg:grid-cols-[1fr_auto_1.08fr] lg:gap-7">
        <Step n={1} text="Pick a draft on the home screen, or choose New project, then From a CapCut draft">
          <Home />
        </Step>
        <span aria-hidden className="hidden size-8 items-center justify-center self-start rounded-full bg-white/[0.06] text-muted lg:mt-[154px] lg:flex">
          <ChevronRight className="size-4" />
        </span>
        <Step n={2} text="It opens as a Nuzky project, and a list names anything that didn't come over">
          <Opened />
        </Step>
      </div>

      <dl className="mt-10 grid gap-x-10 gap-y-6 md:grid-cols-3">
        {facts.map(([t, d]) => (
          <div key={t} className="flex flex-col gap-1.5 border-t border-white/[0.08] pt-5">
            <dt className="text-[15px] font-semibold">{t}</dt>
            <dd className="text-[14px] leading-[1.55] text-muted">{d}</dd>
          </div>
        ))}
        <div className="flex flex-col gap-1.5 border-t border-white/[0.08] pt-5">
          <dt className="text-[15px] font-semibold">Then finish with AI</dt>
          <dd className="text-[14px] leading-[1.55] text-muted">
            Press Finish with AI and hand the rest to ChatGPT or Claude.{" "}
            <a href="#agents" className="text-fg underline decoration-white/30 underline-offset-4 transition-colors hover:decoration-fg">
              How AI editing works
            </a>
          </dd>
        </div>
      </dl>
    </section>
  );
}

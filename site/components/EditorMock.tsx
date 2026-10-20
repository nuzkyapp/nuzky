import Image from "next/image";
import {
  ArrowLeft,
  BetweenHorizontalEnd,
  BetweenHorizontalStart,
  Blend,
  Captions,
  Check,
  ChevronLeft,
  ChevronRight,
  Copy,
  Diamond,
  Download,
  Eye,
  FileText,
  Film,
  FolderOpen,
  History,
  Layers,
  Link2,
  Magnet,
  Maximize,
  MessageSquare,
  Music2,
  Play,
  Plus,
  RectangleHorizontal,
  Redo2,
  RotateCcw,
  Scissors,
  SkipBack,
  SkipForward,
  Smartphone,
  Sparkles,
  SunMedium,
  Trash2,
  Type,
  Undo2,
  Volume2,
  WandSparkles,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { fitSizes, viewSizes } from "@/lib/fit";

// A still of the Nuzky editor at 1200 × 740, drawn from the app's tokens in DESIGN.md and its screens today.
// `focus` is the tour tab: Edit and Captions dim what the tab doesn't talk about, while Reels and Cover show those
// modes of the editor and Export its dialog. `framed` says a phone shows one part of the editor enlarged (Fit's
// `view`), so its images load for that size.

export type EditorFocus = "edit" | "captions" | "reels" | "cover" | "export";

type Sizes = (w: number) => string;

const media = [
  ["tile-1", "2:05", "IMG_2041.MOV"],
  ["tile-2", "0:48", "IMG_2044.MOV"],
  ["tile-3", "1:08", "IMG_2047.MOV"],
  ["tile-4", "0:21", "IMG_2052.MOV"],
  ["tile-6", "0:45", "IMG_2058.MOV"],
  ["tile-8", "Image", "coffee.jpg"],
  ["tile-5", "", ""],
  ["tile-7", "", ""],
  ["tile-9", "", ""],
];

// The timeline draws 110 px per 3 s. The preview shows the frame under the playhead: the selected clip, the caption
// line it sits in and the time it reads, so change them together.
const PLAYHEAD = 398;
const captionLine = [
  [0, 138, "first light"],
  [146, 150, "on the ridge"],
  [304, 128, "we made it"],
  [440, 190, "to the top"],
  [640, 150, "worth it"],
  [800, 132, "see you up there"],
] as const;
const clips = [
  [0, 248, "IMG_2041.MOV", "6.8 s", 1],
  [250, 318, "IMG_2044.MOV", "8.7 s", 2],
  [570, 228, "IMG_2047.MOV", "6.2 s", 3],
  [800, 252, "IMG_2052.MOV", "6.9 s", 4],
] as const;

const wave = (n: number, seed: number, peak: number) =>
  Array.from({ length: n }, (_, k) => 2 + Math.round(peak * Math.abs(Math.sin((k + seed * 13) * 0.37) * Math.cos((k + seed * 7) * 0.11))));

function Pane({ className = "", children }: { className?: string; children: React.ReactNode }) {
  return <div className={`overflow-hidden rounded-xl bg-panel ring-1 ring-inset ring-white/[0.06] ${className}`}>{children}</div>;
}

function Capsule({ className = "", children }: { className?: string; children: React.ReactNode }) {
  return <span className={`capsule flex h-[30px] items-center gap-1.5 rounded-full px-3 text-[12px] font-medium ${className}`}>{children}</span>;
}

function Segmented({ items, on, className = "" }: { items: string[]; on: number; className?: string }) {
  return (
    <div className={`flex rounded-[9px] bg-white/[0.045] p-0.5 font-medium ${className}`}>
      {items.map((t, i) => (
        <span key={t} className={`flex-1 rounded-[7px] px-1.5 py-[5px] text-center ${i === on ? "bg-white/15" : "text-muted"}`}>
          {t}
        </span>
      ))}
    </div>
  );
}

function Switch({ on }: { on: boolean }) {
  return (
    <span className={`relative h-[18px] w-[30px] shrink-0 rounded-full ${on ? "bg-accent-strong" : "bg-white/[0.14]"}`}>
      <span className={`absolute top-0.5 size-3.5 rounded-full bg-white shadow-[0_1px_2px_rgb(0_0_0/0.4)] ${on ? "left-[14px]" : "left-0.5"}`} />
    </span>
  );
}

function TopBar({ focus }: { focus: EditorFocus }) {
  const cover = focus === "cover";
  return (
    <div className="relative flex h-11 shrink-0 items-center justify-between">
      <div className="flex items-center gap-2">
        <Capsule>
          {cover ? <ArrowLeft className="size-[15px] text-muted" /> : <FolderOpen className="size-[15px] text-muted" />}
          {cover ? "Back to video" : "Projects"}
        </Capsule>
        <span className="capsule flex size-[30px] items-center justify-center rounded-full">
          <MessageSquare className="size-[14px] text-muted" />
        </span>
      </div>
      <div className="pointer-events-none absolute inset-x-0 flex items-center justify-center gap-2.5 text-[12px]">
        <span className="font-semibold">
          {cover && <span className="font-normal text-muted">Cover · </span>}
          {focus === "reels" ? "Summit vlog" : "Morning run"}
        </span>
        <span className="flex items-center gap-1 text-muted">
          <Check className="size-3" />
          Saved
        </span>
      </div>
      <div className="flex items-center gap-2">
        <Capsule>
          <Sparkles className="size-[15px]" />
          AI
        </Capsule>
        <span className="capsule flex size-[30px] items-center justify-center rounded-full">
          <History className="size-[14px] text-muted" />
        </span>
        <span
          className={`flex h-[30px] items-center gap-1.5 rounded-full bg-accent-strong pl-3 pr-3.5 text-[12px] font-medium text-white shadow-[inset_0_1px_0_rgb(255_255_255/0.15)] ${focus === "reels" ? "opacity-40" : ""}`}
        >
          <Download className="size-3.5" />
          {cover ? "Export cover" : "Export"}
        </span>
      </div>
    </div>
  );
}

const tabs = [
  [Film, "Media"],
  [Music2, "Audio"],
  [Type, "Text"],
  [Captions, "Captions"],
  [FileText, "Transcript"],
  [Blend, "Transitions"],
  [SunMedium, "Filters"],
] as const;

function LibraryTabs({ on, resting = false }: { on: number; resting?: boolean }) {
  return (
    <div className={`flex gap-0.5 rounded-[10px] bg-white/[0.045] p-0.5 ${resting ? "opacity-40" : ""}`}>
      {tabs.map(([I, t], i) => (
        <span key={t} className={`flex flex-auto flex-col items-center gap-[3px] rounded-[8px] px-1 py-1.5 ${i === on ? "bg-white/15" : "text-muted"}`}>
          <I className={`size-3.5 ${i === on ? "text-accent" : ""}`} />
          <span className="text-[9.5px]">{t}</span>
        </span>
      ))}
    </div>
  );
}

function MediaTab({ sizes }: { sizes: Sizes }) {
  return (
    <>
      <LibraryTabs on={0} />
      <div className="flex gap-2">
        <span className="flex h-10 flex-1 items-center gap-2 rounded-[10px] px-2.5 text-[12px] font-semibold ring-1 ring-inset ring-white/10">
          <span className="flex size-6 items-center justify-center rounded-full bg-accent-strong">
            <Plus className="size-3.5" />
          </span>
          Import media
          <span className="ml-auto rounded-[4px] bg-white/[0.07] px-1 text-[10px] font-normal text-muted">⌘ I</span>
        </span>
        <span className="flex h-10 flex-1 items-center justify-center gap-2 rounded-[10px] px-2.5 text-[12px] font-semibold ring-1 ring-inset ring-white/10">
          <Link2 className="size-3.5 text-muted" />
          From YouTube
        </span>
      </div>
      <div className="grid grid-cols-3 gap-x-2 gap-y-2.5">
        {media.map(([t, d, name]) => (
          <div key={t} className="flex flex-col gap-1">
            <div className={`relative h-[78px] overflow-hidden rounded-[9px] bg-raised ${t === "tile-2" ? "ring-2 ring-accent" : ""}`}>
              <Image src={`/media/${t}.jpg`} alt="" fill sizes={sizes(106)} className="object-cover" />
              {d && <span className="absolute bottom-1 right-1 rounded-[4px] bg-black/60 px-1 text-[9.5px] font-medium tabular-nums">{d}</span>}
            </div>
            {name && <span className="truncate text-[10px] text-muted">{name}</span>}
          </div>
        ))}
      </div>
    </>
  );
}

// Nine of the 19 caption styles, drawn roughly in their own colours (assets/presets/captions.json).
const styles = [
  ["Reel", "text-white"],
  ["Yellow", "text-[#ffe14d]"],
  ["Hormozi", "text-white", "text-[#ffd400]"],
  ["Spotlight", "text-white", "text-[#3ce35a]"],
  ["Impact", "text-white font-black tracking-tight"],
  ["Neon", "text-[#ff8cf0]"],
  ["Yellow box", "rounded-[3px] bg-[#ffd400] px-1 text-[#111]"],
  ["White box", "rounded-[3px] bg-white px-1 text-[#111]"],
  ["Typewriter", "font-mono font-medium text-white"],
] as const;

function CaptionsTab() {
  return (
    <>
      <LibraryTabs on={3} />
      <div className="grid grid-cols-3 gap-2">
        {styles.map(([name, cls, key]) => (
          <span
            key={name}
            className={`flex h-[44px] items-center justify-center rounded-[10px] bg-raised text-[11.5px] font-extrabold ${name === "Spotlight" ? "ring-2 ring-accent" : "ring-1 ring-inset ring-white/[0.06]"}`}
          >
            {key ? (
              <span className={cls}>
                {name.slice(0, 4)}
                <span className={key}>{name.slice(4)}</span>
              </span>
            ) : (
              <span className={cls}>{name}</span>
            )}
          </span>
        ))}
      </div>
      <div className="flex items-center justify-between text-[11.5px]">
        <span className="text-muted">My styles</span>
        <Plus className="size-3.5 text-muted" />
      </div>
      <div className="flex flex-col gap-1.5">
        <span className="text-[11.5px] text-muted">Key words</span>
        <Segmented items={["Off", "Emphasis", "First", "Last", "Longest"]} on={1} className="text-[10.5px]" />
      </div>
      <span className="mt-auto flex h-8 items-center justify-center gap-2 rounded-[8px] bg-accent-strong text-[12px] font-medium text-white">
        <Captions className="size-3.5" />
        Regenerate captions
      </span>
    </>
  );
}

// The words of the reel picked, marked in what is said around them (Reels from a long video, Trimming).
const words = [
  ["so we left at five, the car was freezing and", 0],
  ["honestly I almost turned back.", 0],
  ["Nobody tells you the last hour is the best part.", 1],
  ["You stop thinking about your legs and you just", 1],
  ["look up, and the whole ridge is lit.", 1],
  ["Anyway, the coffee at the top was", 0],
] as const;

function ReelWords() {
  return (
    <>
      <LibraryTabs on={4} resting />
      <div className="flex items-baseline justify-between">
        <span className="text-[13px] font-semibold">The last hour</span>
        <span className="text-[11px] tabular-nums text-muted">0:38 once cut</span>
      </div>
      <p className="text-[13px] leading-[1.75]">
        {words.map(([t, inReel], i) => (
          <span key={i} className={inReel ? "rounded-[3px] bg-accent/25 box-decoration-clone px-0.5" : "text-muted"}>
            {t}{" "}
          </span>
        ))}
      </p>
      <span className="mt-auto text-[11px] leading-[1.45] text-muted">Click a word to play from it. Shift-click moves the start or the end of the reel to it.</span>
    </>
  );
}

const frames = [
  ["timeline-1", "00:04.2", "Sharp, well framed"],
  ["timeline-2", "00:11.8", ""],
  ["timeline-3", "00:17.0", ""],
  ["timeline-4", "00:24.6", ""],
  ["tile-2", "00:08.1", ""],
  ["tile-6", "00:27.3", ""],
] as const;

function CoverFrames({ sizes }: { sizes: Sizes }) {
  return (
    <>
      <LibraryTabs on={0} resting />
      <div className="flex items-center justify-between">
        <span className="text-[13px] font-semibold">Frames</span>
        <span className="flex items-center gap-1.5 text-[11.5px] font-medium text-accent">
          <WandSparkles className="size-3.5" />
          Make it with AI
        </span>
      </div>
      <span className="capsule flex h-8 items-center justify-center gap-1.5 rounded-[8px] text-[12px] font-medium">
        <Sparkles className="size-3.5 text-accent" />
        Pick again
      </span>
      <div className="grid grid-cols-3 gap-x-2 gap-y-2.5">
        {frames.map(([t, time, why], i) => (
          <div key={t} className="flex flex-col gap-1">
            <div className={`relative h-[92px] overflow-hidden rounded-[9px] bg-raised ${i === 0 ? "ring-2 ring-accent" : ""}`}>
              <Image src={`/media/${t}.jpg`} alt="" fill sizes={sizes(106)} className="object-cover" />
              <span className="absolute bottom-1 right-1 rounded-[4px] bg-black/60 px-1 text-[9.5px] font-medium tabular-nums">{time}</span>
            </div>
            <span className="truncate text-[10px] text-muted">{why || " "}</span>
          </div>
        ))}
      </div>
    </>
  );
}

function Handles() {
  return (
    <>
      <span className="absolute inset-0 ring-1 ring-white/80" />
      {["-left-[3px] -top-[3px]", "-right-[3px] -top-[3px]", "-bottom-[3px] -left-[3px]", "-bottom-[3px] -right-[3px]"].map((c) => (
        <span key={c} className={`absolute size-1.5 bg-white ${c}`} />
      ))}
      <span className="absolute -top-[3px] left-1/2 h-1.5 w-3.5 -translate-x-1/2 rounded-full bg-white" />
    </>
  );
}

const shadow = "drop-shadow(0 2px 6px rgb(0 0 0 / 0.75))";

function Preview({ focus, sizes }: { focus: EditorFocus; sizes: Sizes }) {
  const cover = focus === "cover";
  const reels = focus === "reels";
  return (
    <div className="flex min-w-0 flex-1 flex-col items-center justify-center gap-3.5 rounded-xl bg-stage p-3.5 ring-1 ring-inset ring-white/[0.06]">
      <div className="relative h-[313px] w-[176px]">
        <Image src="/media/reel-hike.jpg" alt="" fill sizes={sizes(176)} className="object-cover" />
        {cover ? (
          <div className="absolute inset-x-0 top-[46px] flex flex-col items-center text-[30px] font-black uppercase leading-[0.95] tracking-[-0.02em] text-white" style={{ filter: shadow, WebkitTextStroke: "1px #000" }}>
            <span>Morning</span>
            <span>run</span>
          </div>
        ) : reels ? (
          <div className="absolute inset-x-3 bottom-[70px] flex flex-wrap justify-center gap-x-[0.26em] text-[15px] font-black uppercase leading-[1.1] tracking-[-0.015em]" style={{ filter: shadow }}>
            <span>the</span>
            <span>last</span>
            <span className="text-caption-yellow">hour</span>
          </div>
        ) : (
          <div className="absolute inset-x-0 bottom-[72px] flex justify-center gap-[0.26em] text-[17px] font-black uppercase tracking-[-0.015em]" style={{ filter: shadow }}>
            <span>we</span>
            <span>made</span>
            <span className="text-caption-yellow">it</span>
          </div>
        )}
        {focus === "edit" && <Handles />}
        {cover && <span className="absolute inset-x-[18px] top-[38px] h-[76px] ring-1 ring-white/80" />}
      </div>
      <div className="capsule flex h-10 w-full items-center justify-between gap-3 rounded-full px-3.5">
        {cover ? (
          <>
            <span className="text-[12px] font-medium tabular-nums">
              Frame <span className="text-muted">00:04.20</span>
            </span>
            <span className="flex h-7 items-center rounded-full bg-white/[0.06] p-0.5 text-[11px] font-medium">
              <span className="rounded-full bg-white/15 px-2.5 py-1">9:16 Cover</span>
              <span className="px-2.5 py-1 text-muted">16:9 YouTube</span>
            </span>
            <span className="flex items-center gap-1.5 text-[11px] text-muted">
              <Smartphone className="size-3.5" />
              What the apps cover
            </span>
          </>
        ) : (
          <>
            <span className="w-[118px] text-[12px] font-medium tabular-nums">
              {reels ? "06:12.40" : "00:10.85"} <span className="font-normal text-muted">/ {reels ? "12:40.00" : "00:30.00"}</span>
            </span>
            <span className="flex items-center gap-3.5">
              <SkipBack className="size-3.5" />
              <span className="flex size-7 items-center justify-center rounded-full bg-fg text-bg">
                <Play className="size-[13px] fill-current" />
              </span>
              <SkipForward className="size-3.5" />
            </span>
            {reels ? (
              <span className="flex items-center gap-2.5 text-[11px]">
                <span className="text-muted">Reel start</span>
                <span className="text-muted">Hear the ending</span>
                <span className="flex h-6 items-center rounded-full bg-white/[0.12] px-2.5 font-medium">Done</span>
              </span>
            ) : (
              <span className="flex items-center gap-2.5 text-muted">
                <Volume2 className="size-3.5" />
                <Smartphone className="size-3.5" />
                <span className="flex h-6 items-center gap-1 rounded-full bg-white/[0.07] px-2 text-[11px] text-fg">
                  <RectangleHorizontal className="size-3 text-muted" />
                  Ratio <span className="text-muted">9:16</span>
                </span>
                <Maximize className="size-3.5" />
              </span>
            )}
          </>
        )}
      </div>
    </div>
  );
}

const sliders = [
  ["Scale", "100", "%", 0.32],
  ["Position X", "0.0", "%", 0.5],
  ["Position Y", "0.0", "%", 0.5],
  ["Rotation", "0", "°", 0.5],
  ["Opacity", "100", "%", 0.94],
] as const;

function VideoInspector() {
  return (
    <>
      <div className="flex flex-col gap-0.5">
        <span className="flex items-center gap-2 text-[14px] font-semibold">
          <Film className="size-3.5 text-muted" />
          IMG_2044.MOV
        </span>
        <span className="text-[11px] text-muted">8.7 s · from 2.0 s of 48.0 s</span>
      </div>
      <Segmented items={["Video", "Adjust", "Speed", "Animation", "Audio"]} on={0} className="text-[11px]" />
      <div className="flex items-center justify-between pt-1">
        <span className="text-[12.5px] font-semibold">Transform</span>
        <span className="flex items-center gap-2.5 text-muted">
          <ChevronLeft className="size-3.5" />
          <Diamond className="size-3 text-fg" />
          <ChevronRight className="size-3.5" />
          <span className="h-3 w-px bg-white/15" />
          <RotateCcw className="size-3.5" />
        </span>
      </div>
      <Segmented items={["Fit", "Fill"]} on={0} className="text-[11.5px]" />
      <div className="flex flex-col gap-1.5">
        {sliders.map(([label, value, unit, p]) => (
          <div key={label} className="flex flex-col gap-0.5">
            <span className="text-[11px] text-muted">{label}</span>
            <div className="flex items-center gap-2">
              <div className="relative h-3.5 flex-1">
                <div className="absolute inset-x-0 top-[5px] h-1 rounded-full bg-white/[0.13]" />
                {p !== 0.5 && <div className="absolute left-0 top-[5px] h-1 rounded-full bg-accent" style={{ width: `${p * 100}%` }} />}
                <div className="absolute top-0 h-3.5 w-[22px] -translate-x-1/2 rounded-full bg-white shadow-[0_1px_3px_rgb(0_0_0/0.4)]" style={{ left: `${p * 100}%` }} />
              </div>
              <span className="flex h-[22px] w-[50px] items-center justify-end rounded-md bg-white/[0.055] px-[7px] text-[11.5px] tabular-nums ring-1 ring-inset ring-white/[0.08]">{value}</span>
              <span className="w-2.5 text-[11px] text-muted">{unit}</span>
            </div>
          </div>
        ))}
      </div>
      <div className="mt-1 flex items-center justify-between border-t border-white/[0.07] pt-3">
        <span className="text-[12.5px] font-semibold">Crop and shape</span>
        <RotateCcw className="size-3.5 text-muted" />
      </div>
      <Segmented items={["Rectangle", "Rounded", "Circle"]} on={0} className="text-[11px]" />
    </>
  );
}

function CoverInspector() {
  return (
    <>
      <div className="flex items-center justify-between">
        <span className="text-[14px] font-semibold">Text</span>
        <span className="flex items-center gap-1 text-[11.5px] font-medium text-accent">
          <Plus className="size-3.5" />
          Add text
        </span>
      </div>
      <span className="flex h-9 items-center rounded-[8px] bg-white/[0.055] px-2.5 text-[12px] font-semibold ring-1 ring-inset ring-accent/60">MORNING RUN</span>
      <div className="grid grid-cols-3 gap-1.5">
        {[
          ["Big", "text-white"],
          ["Yellow box", "rounded-[3px] bg-[#ffd400] px-1 text-[#111]"],
          ["Outlined", "text-white [-webkit-text-stroke:0.6px_#000]"],
        ].map(([name, cls]) => (
          <span key={name} className={`flex h-10 items-center justify-center rounded-[9px] bg-raised text-[11px] font-black ${name === "Outlined" ? "ring-2 ring-accent" : "ring-1 ring-inset ring-white/[0.06]"}`}>
            <span className={cls}>{name}</span>
          </span>
        ))}
      </div>
      {[
        ["Behind person", true],
        ["Line around person", false],
      ].map(([label, on]) => (
        <div key={label as string} className="flex items-center justify-between text-[12px]">
          {label as string}
          <Switch on={on as boolean} />
        </div>
      ))}
      <div className="mt-1 flex flex-col gap-2 border-t border-white/[0.07] pt-3">
        <span className="text-[12.5px] font-semibold">Background</span>
        <Segmented items={["As filmed", "Blurred", "Color"]} on={0} className="text-[11px]" />
      </div>
      <div className="mt-1 flex flex-col gap-2 border-t border-white/[0.07] pt-3">
        <span className="text-[12.5px] font-semibold">Export</span>
        <Segmented items={["PNG", "JPEG"]} on={1} className="text-[11px]" />
      </div>
    </>
  );
}

function TimelineToolbar() {
  const tools = [[Undo2, Redo2], [Scissors, BetweenHorizontalStart, BetweenHorizontalEnd, Trash2], [Copy, Layers]] as const;
  return (
    <div className="flex h-11 shrink-0 items-center justify-between px-2.5">
      <div className="capsule flex h-[30px] items-center gap-3 rounded-full px-3 text-fg/90">
        {tools.map((group, g) => (
          <span key={g} className="flex items-center gap-3">
            {group.map((I, i) => (
              <I key={i} className={`size-3.5 ${I === Redo2 ? "text-subtle" : ""}`} />
            ))}
            <span className="h-3.5 w-px bg-white/15" />
          </span>
        ))}
        <span className="-mr-1.5 flex size-6 items-center justify-center rounded-full bg-accent/25">
          <Magnet className="size-3.5 text-accent" />
        </span>
      </div>
      <div className="flex items-center gap-2.5 text-muted">
        <ZoomOut className="size-3.5" />
        <div className="relative h-3 w-[90px]">
          <div className="absolute inset-x-0 top-1 h-1 rounded-full bg-white/[0.13]" />
          <div className="absolute left-0 top-1 h-1 w-10 rounded-full bg-accent" />
          <div className="absolute left-[30px] top-0 h-3 w-5 rounded-full bg-white" />
        </div>
        <ZoomIn className="size-3.5" />
        <Maximize className="ml-1 size-3.5" />
      </div>
    </div>
  );
}

function Timeline({ focus, sizes }: { focus: EditorFocus; sizes: Sizes }) {
  const dim = (on: boolean) => `transition-opacity duration-300 ${on ? "opacity-100" : "opacity-35"}`;
  const cover = focus === "cover";
  return (
    <Pane className="flex h-[244px] shrink-0 flex-col">
      <div className={cover ? "opacity-35" : ""}>
        <TimelineToolbar />
      </div>
      <div className="flex min-h-0 flex-1">
        <div className="relative w-[136px] shrink-0 text-muted">
          <span className={`absolute left-3 right-3 top-[26px] flex h-[22px] items-center gap-1.5 text-[11px] text-fg/85 ${dim(focus !== "cover")}`}>
            <Captions className="size-3 text-muted" />
            Captions
            <Eye className="ml-auto size-3 text-muted" />
          </span>
          {/* The Cover tile starts the main track; a made cover shows its picture. */}
          <span className={`absolute left-2.5 top-[54px] flex h-[60px] w-[44px] flex-col overflow-hidden rounded-[7px] bg-raised ${cover ? "ring-2 ring-accent" : "ring-1 ring-white/20"}`}>
            <span className="relative h-[44px] w-full">
              <Image src="/media/reel-hike.jpg" alt="" fill sizes={sizes(44)} className="object-cover" />
              <span className="absolute inset-x-0 top-[5px] text-center text-[6.5px] font-black leading-[1.05] text-white">
                MORNING
                <br />
                RUN
              </span>
            </span>
            <span className="text-center text-[9px] leading-4 text-fg">Cover</span>
          </span>
          <span className={`absolute right-3 top-[76px] flex items-center gap-2 ${dim(!cover)}`}>
            <Eye className="size-3" />
            <Volume2 className="size-3" />
          </span>
          <span className={`absolute left-3 right-3 top-[122px] flex h-8 items-center gap-1.5 text-[11px] text-fg/85 ${dim(focus === "edit")}`}>
            <Music2 className="size-3 text-muted" />
            Music
            <Volume2 className="ml-auto size-3 text-muted" />
          </span>
        </div>
        <div className="relative min-w-0 flex-1 overflow-hidden">
          {Array.from({ length: 10 }, (_, s) => (
            <div key={s} className="absolute top-0.5" style={{ left: s * 110 }}>
              <span className="block pl-1 text-[10px] tabular-nums text-muted">00:{String(s * 3).padStart(2, "0")}</span>
              <span className="mt-0.5 block h-1.5 w-px bg-white/20" />
            </div>
          ))}
          <div className={dim(focus === "edit" || focus === "captions")}>
            {captionLine.map(([x, w, t]) => (
              <span
                key={t}
                className={`absolute top-[26px] flex h-[22px] items-center truncate rounded-[6px] bg-clip-captions px-2 text-[10.5px] ${focus === "captions" && t === "we made it" ? "ring-2 ring-accent" : "ring-1 ring-inset ring-white/10"}`}
                style={{ left: x, width: w - 2 }}
              >
                {t}
              </span>
            ))}
          </div>
          <div className={dim(focus === "edit")}>
            {clips.map(([x, w, name, length, n]) => {
              const selected = n === 2 && focus === "edit";
              return (
                <div
                  key={name}
                  className={`absolute top-[54px] h-[60px] overflow-hidden rounded-[7px] bg-clip-video ${selected ? "ring-2 ring-accent" : "ring-1 ring-inset ring-white/10"}`}
                  style={{ left: x, width: w - 2 }}
                >
                  <div className="absolute inset-x-0 top-0 flex h-[44px]">
                    {Array.from({ length: Math.ceil(w / 176) }, (_, k) => (
                      <Image key={k} src={`/media/timeline-${n}.jpg`} alt="" width={176} height={44} sizes={sizes(176)} className="h-[44px] w-[176px] max-w-none shrink-0 object-cover" />
                    ))}
                  </div>
                  <div className="absolute inset-x-0 bottom-0 flex h-4 items-end gap-px bg-black/40 px-1 pb-0.5">
                    {wave(Math.floor((w - 8) / 3), n, 9).map((h, k) => (
                      <span key={k} className="w-[2px] shrink-0 rounded-[1px] bg-ok/70" style={{ height: h }} />
                    ))}
                  </div>
                  <span className="absolute left-1.5 top-1 flex items-center gap-1 rounded bg-black/60 px-[5px] py-px text-[10px] font-medium">
                    {name}
                    <span className="font-normal text-white/60">{length}</span>
                  </span>
                  {selected && (
                    <>
                      <span className="absolute inset-y-0 left-0 w-1.5 rounded-l-[6px] bg-accent" />
                      <span className="absolute inset-y-0 right-0 w-1.5 rounded-r-[6px] bg-accent" />
                    </>
                  )}
                </div>
              );
            })}
          </div>
          <div className={dim(focus === "edit")}>
            <div className="absolute left-0 top-[122px] h-8 w-[1040px] overflow-hidden rounded-[7px] bg-clip-audio ring-1 ring-inset ring-white/10">
              <div className="absolute inset-x-1 bottom-1 flex h-3 items-end gap-[3px]">
                {wave(200, 0, 10).map((h, k) => (
                  <span key={k} className="w-[2px] shrink-0 rounded-[1px] bg-ok/60" style={{ height: h }} />
                ))}
              </div>
              <span className="absolute left-1.5 top-1 rounded bg-black/60 px-[5px] py-px text-[10px] font-medium">Sunrise.m4a</span>
            </div>
          </div>
          <span className="absolute top-4 h-[184px] w-[1.5px] bg-white" style={{ left: cover ? 152 : PLAYHEAD }} />
          <span className="absolute top-3 size-[11px] rounded-full bg-white" style={{ left: (cover ? 152 : PLAYHEAD) - 5 }} />
        </div>
      </div>
    </Pane>
  );
}

// The reels it found take the timeline's place (Reels from a long video, A card).
const reels = [
  { title: "The last hour", hook: "“Nobody tells you the last hour is the best part.”", length: "0:38", why: "A clear hook and a payoff", score: 92, checked: true, image: "reel-hike" },
  { title: "Turning back", hook: "“Honestly, I almost turned back.”", length: "0:24", why: "Honest and short", score: 84, checked: true, image: "tile-2" },
  { title: "Coffee at the top", hook: "“The coffee at the top was terrible.”", length: "0:31", why: "Funny, ends on a laugh", score: 71, checked: false, image: "tile-8" },
  { title: "Gear I carry", hook: "“People ask what's in the bag.”", length: "1:12", why: "Useful, but long", score: 58, checked: false, image: "tile-6", long: true },
];

function ReelCards({ sizes }: { sizes: Sizes }) {
  return (
    <Pane className="flex h-[244px] shrink-0 flex-col gap-3 p-3">
      <div className="flex items-center justify-between">
        <span className="text-[13px] font-semibold">
          Reels <span className="font-normal text-muted">· 4 found, best first</span>
        </span>
        <div className="flex items-center gap-2">
          <Segmented items={["Center crop", "Blurred background"]} on={0} className="w-[250px] text-[11px]" />
          <span className="flex h-[30px] items-center rounded-full bg-accent-strong px-3.5 text-[12px] font-medium text-white">Make 2 reels</span>
        </div>
      </div>
      <div className="grid min-h-0 flex-1 grid-cols-4 gap-2.5">
        {reels.map((r, i) => (
          <div key={r.title} className={`flex gap-2.5 rounded-[12px] bg-white/[0.035] p-2.5 ${i === 0 ? "ring-2 ring-accent" : "ring-1 ring-inset ring-white/[0.07]"}`}>
            <div className="relative h-[150px] w-[84px] shrink-0 overflow-hidden rounded-[8px]">
              <Image src={`/media/${r.image}.jpg`} alt="" fill sizes={sizes(84)} className="object-cover" />
            </div>
            <div className="flex min-w-0 flex-1 flex-col gap-1.5 text-[11px]">
              <span className="flex items-center gap-1.5 text-[12px] font-semibold">
                <span className={`flex size-3.5 shrink-0 items-center justify-center rounded-[4px] ${r.checked ? "bg-accent-strong" : "ring-1 ring-inset ring-white/30"}`}>
                  {r.checked && <Check className="size-2.5" strokeWidth={3} />}
                </span>
                <span className="truncate">{r.title}</span>
              </span>
              <span className="leading-[1.35] text-fg/85">{r.hook}</span>
              <span className="mt-auto leading-[1.35] text-muted">{r.why}</span>
              <span className="flex justify-between tabular-nums text-muted">
                <span className={r.long ? "text-warn" : ""}>{r.length}</span>
                <span>{r.score}</span>
              </span>
            </div>
          </div>
        ))}
      </div>
    </Pane>
  );
}

export function EditorMock({ focus = "edit", framed = false }: { focus?: EditorFocus; framed?: boolean }) {
  const sizes: Sizes = (w) => (framed ? viewSizes : fitSizes)(w, 1200);
  const dim = (on: boolean) => `transition-opacity duration-300 ${on ? "opacity-100" : "opacity-35"}`;
  const reels = focus === "reels";

  return (
    <div className="flex h-[740px] w-[1200px] flex-col gap-1.5 overflow-hidden rounded-[14px] bg-bg px-1.5 pb-1.5 text-left text-fg ring-1 ring-white/10">
      {/* The macOS window controls. */}
      <div className="-mx-1.5 flex h-7 shrink-0 items-center gap-2 border-b border-white/[0.06] bg-white/[0.025] px-3.5">
        {["#ff5f57", "#febc2e", "#28c840"].map((c) => (
          <span key={c} className="size-3 rounded-full ring-[0.5px] ring-black/20" style={{ background: c }} />
        ))}
      </div>

      <TopBar focus={focus} />

      <div className="flex min-h-0 flex-1 gap-1.5">
        <Pane className={`flex w-[350px] shrink-0 flex-col gap-2.5 p-2.5 ${dim(focus !== "export")}`}>
          {focus === "captions" ? <CaptionsTab /> : reels ? <ReelWords /> : focus === "cover" ? <CoverFrames sizes={sizes} /> : <MediaTab sizes={sizes} />}
        </Pane>
        <Preview focus={focus} sizes={sizes} />
        {!reels && (
          <Pane className={`flex w-[290px] shrink-0 flex-col gap-2.5 p-3 ${dim(focus === "edit" || focus === "cover")}`}>
            {focus === "cover" ? <CoverInspector /> : <VideoInspector />}
          </Pane>
        )}
      </div>

      {reels ? <ReelCards sizes={sizes} /> : <Timeline focus={focus} sizes={sizes} />}
    </div>
  );
}

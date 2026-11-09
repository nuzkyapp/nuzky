import { Fragment } from "react";
import Image from "next/image";
import {
  AudioLines,
  Ban,
  Blend,
  Captions,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Copy,
  Diamond,
  Download,
  Eye,
  Film,
  FolderOpen,
  History,
  Link2,
  Magnet,
  Maximize,
  Maximize2,
  MessageSquare,
  Minimize,
  MonitorDot,
  Music,
  Palette,
  PanelLeftClose,
  PanelRightClose,
  Play,
  Plus,
  Puzzle,
  QrCode,
  RectangleHorizontal,
  Redo2,
  RotateCcw,
  RotateCwSquare,
  ScanFace,
  Scissors,
  ScrollText,
  Settings,
  SkipBack,
  SkipForward,
  Smartphone,
  Snowflake,
  Sparkles,
  Trash2,
  Type,
  Undo2,
  Unlink,
  Volume2,
  X,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { fitSizes, viewSizes } from "@/lib/fit";

// The Nuzky editor as it looks in a 1440 × 900 window on macOS, drawn with the app's own sizes, tokens and materials
// (DESIGN.md, src/components) and shown at 5/6, so it takes MOCK_W × MOCK_H on the page. Every position the tour
// gives (Showcase.tsx) is in that page space. `focus` is the tour tab: Edit and Captions show the editor, Reels and
// Cover those modes of it, and Export the editor under its dialog (Showcase.tsx). `framed` says a phone shows one part
// of the editor enlarged (Fit's `view`), so its images load for that size.

export type EditorFocus = "edit" | "captions" | "reels" | "cover" | "export";

export const MOCK_W = 1200;
export const MOCK_H = 750;
const SCALE = MOCK_W / 1440;

type Sizes = (w: number) => string;

// The window: a 48 px top bar, then the panes 6 px apart, with the timeline as tall as someone dragged it to show
// every track.
const TIMELINE_H = 262;
const LIBRARY_W = 302;
const INSPECTOR_W = 318;

// The timeline draws 40 px per second (a 2 s ruler step) in lanes that start after the 152 px track headers.
const PX = 40;
const HEADER_W = 152;
const PLAYHEAD = 10.85;

const clips = [
  { name: "IMG_2041.MOV", start: 0, length: 6.8, image: "tile-1" },
  { name: "IMG_2044.MOV", start: 6.8, length: 8.7, image: "reel-hike" },
  { name: "IMG_2047.MOV", start: 15.5, length: 6.2, image: "tile-3" },
  { name: "IMG_2052.MOV", start: 21.7, length: 6.9, image: "tile-4" },
];
const LENGTH = 28.6;

// The caption line under the playhead is the one the preview shows.
const captionLine = [
  [0.2, 3.4, "first light"],
  [3.6, 6.6, "on the ridge"],
  [9.4, 12.6, "we made it"],
  [12.8, 16.2, "to the top"],
  [17, 19.8, "worth it"],
  [21.4, 25.4, "see you up there"],
] as const;

const media = [
  ["tile-1", "2:05", "IMG_2041.MOV"],
  ["tile-2", "48.0 s", "IMG_2044.MOV"],
  ["tile-3", "1:08", "IMG_2047.MOV"],
  ["tile-4", "21.0 s", "IMG_2052.MOV"],
  ["tile-6", "45.0 s", "IMG_2058.MOV"],
  ["tile-8", "Image", "coffee.jpg"],
] as const;

const x = (s: number) => Math.round(s * PX);

// Bars every 3 px from the clip's start, as Waveform.tsx draws them; quiet sound is a 1 px floor.
const wave = (n: number, seed: number, peak: number) =>
  Array.from({ length: n }, (_, k) => Math.max(1, Math.round(peak * Math.abs(Math.sin((k + seed * 13) * 0.37) * Math.cos((k + seed * 7) * 0.11)))));

// A pane's scrolling content keeps its 6 px thumb in view (scroll-shown in the app), in a 10 px scrollbar.
function Scrolled({ thumb, className = "", children }: { thumb: [number, number]; className?: string; children: React.ReactNode }) {
  return (
    <div className={`relative min-h-0 flex-1 overflow-hidden pr-2.5 ${className}`}>
      {children}
      <span className="absolute right-0.5 w-1.5 rounded-full bg-white/[0.14]" style={{ top: thumb[0], height: thumb[1] }} />
    </div>
  );
}

function Icon({ i: I, size = 16, className = "" }: { i: typeof Film; size?: number; className?: string }) {
  return <I size={size} strokeWidth={2} className={`shrink-0 ${className}`} />;
}

function IconBtn({ i, size = 16, className = "", off = false }: { i: typeof Film; size?: number; className?: string; off?: boolean }) {
  return (
    <span className={`flex size-8 shrink-0 items-center justify-center rounded-full ${off ? "opacity-40" : ""} ${className}`}>
      <Icon i={i} size={size} />
    </span>
  );
}

function Seg({ items, on, className = "", icons }: { items: string[]; on: number; className?: string; icons?: (typeof Film | null)[] }) {
  return (
    <div className={`app-seg flex gap-0.5 rounded-[9px] p-0.5 ${className}`}>
      {items.map((t, i) => (
        <span key={t} className={`flex h-7 flex-auto items-center justify-center gap-1.5 whitespace-nowrap rounded-[7px] px-2 text-[12px] font-medium ${i === on ? "app-seg-on text-fg" : "text-muted"}`}>
          {icons?.[i] && <Icon i={icons[i]!} size={13} />}
          {t}
        </span>
      ))}
    </div>
  );
}

// ---- Top bar ------------------------------------------------------------------------------------------------

function TopBar({ focus }: { focus: EditorFocus }) {
  const cover = focus === "cover";
  const reels = focus === "reels";
  return (
    <div className="relative flex h-12 shrink-0 items-center gap-2 pl-[78px] pr-1.5">
      {/* macOS puts its window buttons in the bar, 18 px in. */}
      <div className="absolute left-[18px] top-1/2 flex -translate-y-1/2 gap-2">
        {["#ff5f57", "#febc2e", "#28c840"].map((c) => (
          <span key={c} className="size-3 rounded-full shadow-[inset_0_0_0_0.5px_rgb(0_0_0/0.25)]" style={{ background: c }} />
        ))}
      </div>
      {cover ? (
        <span className="app-bar flex h-8 items-center gap-1.5 rounded-full px-3.5 text-[13px] font-medium">
          <Icon i={ChevronLeft} size={15} />
          Back to video
        </span>
      ) : (
        <>
          <span className="app-bar flex h-8 items-center gap-1.5 rounded-full px-3.5 text-[13px] font-medium">
            <Icon i={FolderOpen} size={15} />
            Projects
          </span>
          <span className="app-bar flex items-center rounded-full">
            <IconBtn i={MessageSquare} />
            <IconBtn i={QrCode} />
          </span>
        </>
      )}
      <div className="flex min-w-0 flex-1 justify-center pr-[72px]">
        <div className="relative flex items-center">
          {cover ? (
            <span className="text-[13px]">
              <span className="font-semibold">Cover</span> <span className="text-muted">· Morning run</span>
            </span>
          ) : (
            <span className="px-1.5 text-[13px] font-semibold">{reels ? "Summit vlog" : "Morning run"}</span>
          )}
          <span className="absolute left-full ml-2 flex items-center gap-1.5 whitespace-nowrap text-[12px] text-muted">
            <span className="size-1.5 rounded-full bg-ok" />
            Saved
          </span>
        </div>
      </div>
      <span className="app-bar flex h-8 items-center gap-1.5 rounded-full px-3.5 text-[13px] font-medium">
        <Icon i={Sparkles} size={15} />
        AI
      </span>
      <span className="app-bar flex items-center rounded-full">
        {(cover || reels) && (
          <>
            <IconBtn i={Undo2} />
            <IconBtn i={Redo2} off />
          </>
        )}
        <IconBtn i={History} />
        <IconBtn i={Settings} />
      </span>
      {!reels && (
        <span className="app-prominent flex h-8 items-center gap-1.5 rounded-full px-3.5 text-[13px] font-medium">
          <Icon i={Download} size={15} />
          {cover ? "Export cover" : "Export"}
        </span>
      )}
    </div>
  );
}

// ---- Library --------------------------------------------------------------------------------------------------

const tabs = [
  [Film, "Media"],
  [Music, "Audio"],
  [Type, "Text"],
  [Captions, "Captions"],
  [ScrollText, "Transcript"],
  [Blend, "Transitions"],
  [Palette, "Filters"],
  [Puzzle, "Plugins"],
] as const;

// Under 420 px the eight tabs take two rows of four.
function LibraryTabs({ on }: { on: number }) {
  return (
    <div className="mx-1.5 mb-0.5 mt-1.5 grid shrink-0 grid-cols-4 gap-1 rounded-[13px] p-0.5">
      {tabs.map(([I, t], i) => (
        <span key={t} className={`flex h-10 flex-col items-center justify-center gap-[3px] rounded-[11px] text-[11px] font-medium leading-[13px] ${i === on ? "bg-white/[0.11] text-fg" : "text-fg/70"}`}>
          <Icon i={I} className={i === on ? "text-accent" : ""} />
          {t}
        </span>
      ))}
    </div>
  );
}

const SECONDARY = "bg-white/[0.09] shadow-[inset_0_0_0_1px_rgb(255_255_255/0.06)]";

const TILE = "flex h-9 shrink-0 items-center justify-center gap-2 rounded-[10px] bg-white/[0.055] text-[13px] font-semibold shadow-[inset_0_0_0_1px_rgb(255_255_255/0.07)]";

function MediaTab({ sizes }: { sizes: Sizes }) {
  const used = new Set(clips.map((c) => c.name));
  return (
    <>
      <LibraryTabs on={0} />
      <div className="flex min-h-0 flex-1 flex-col gap-3.5 pt-3.5">
        <div className="mx-3.5 flex gap-2">
          <span className={`${TILE} flex-1`}>
            <Icon i={Plus} className="text-accent" />
            Import media
          </span>
          <span className={`${TILE} w-9`}>
            <Icon i={Link2} className="text-muted" />
          </span>
          <span className={`${TILE} w-9`}>
            <Icon i={MonitorDot} className="text-muted" />
          </span>
        </div>
        <Scrolled thumb={[2, 300]}>
        <div className="grid grid-cols-2 gap-x-2 gap-y-3 px-3.5">
          {media.map(([image, length, name]) => (
            <div key={name} className="flex flex-col gap-1.5">
              <div className="relative aspect-square overflow-hidden rounded-[10px] bg-window outline-1 -outline-offset-1 outline-white/[0.08]">
                <Image src={`/media/${image}.jpg`} alt="" fill sizes={sizes(133)} className="object-cover" />
                <span className="absolute bottom-1 right-1 rounded-[4px] bg-black/60 px-1 text-[11px] font-semibold leading-[15px] tabular-nums">{length}</span>
                {used.has(name) && (
                  <span className="absolute left-1.5 top-1.5 flex size-4 items-center justify-center rounded-full bg-black/60 shadow-[inset_0_0_0_1px_rgb(255_255_255/0.2)]"><Check size={10} strokeWidth={3} />
                  </span>
                )}
              </div>
              <span className="text-[12px] leading-[15px] text-fg/85">{name}</span>
            </div>
          ))}
        </div>
        </Scrolled>
      </div>
    </>
  );
}

// The first caption styles (assets/presets/captions.json), each name drawn in its style as TextSwatch draws it: a
// karaoke style lights the end of its name, key words its start.
const captionStyles: { name: string; color: string; bold?: boolean; outline?: number; box?: boolean; lit?: string; key?: string; font?: string }[] = [
  { name: "Reel", color: "#ffffff", outline: 2 },
  { name: "Outline", color: "#ffffff", bold: true, outline: 1.75 },
  { name: "Yellow", color: "#ffe14d", bold: true, outline: 1.75 },
  { name: "Box", color: "#ffffff", bold: true, box: true },
  { name: "Clean", color: "#ffffff", outline: 0.75 },
  { name: "Karaoke", color: "#ffffff", outline: 2, lit: "#ffe14d" },
  { name: "Green box", color: "#ffffff", bold: true, box: true, lit: "#4ade80" },
  { name: "Hormozi", color: "#ffffff", bold: true, outline: 2, key: "#ffd400" },
];

function StyleName({ s }: { s: (typeof captionStyles)[number] }) {
  const split = s.name.lastIndexOf(" ") + 1 || Math.ceil(s.name.length / 2);
  const key = s.key ? Math.floor(s.name.length / 2) : 0;
  return (
    <span
      className={`app-outline truncate rounded px-1 text-[13px] leading-6 ${s.bold ? "font-bold" : ""} ${s.box ? "bg-black/70" : ""}`}
      style={{ color: s.color, ["--outline" as string]: `${(s.outline ?? 0) * 2}px` }}
    >
      {key > 0 && <span style={{ color: s.key }}>{s.name.slice(0, key)}</span>}
      {s.lit ? (
        <>
          {s.name.slice(key, split)}
          <span style={{ color: s.lit }}>{s.name.slice(split)}</span>
        </>
      ) : (
        s.name.slice(key)
      )}
    </span>
  );
}

function CaptionsTab() {
  return (
    <>
      <LibraryTabs on={3} />
      <div className="flex min-h-0 flex-1 flex-col">
        <Scrolled thumb={[2, 190]} className="flex flex-col gap-3.5 p-3.5">
          <Labelled label="Spoken language">
            <Select value="English" />
          </Labelled>
          <Labelled label="Accuracy">
            <Select value="Most accurate (547 MB)" />
          </Labelled>
          <Labelled label="Words per caption">
            <Seg items={["1", "2", "3", "Phrases"]} on={1} />
          </Labelled>
          <div className="flex flex-col gap-1.5">
            <h3 className="text-[13px] font-semibold">Style</h3>
            <div className="grid grid-cols-2 gap-2">
              {captionStyles.map((s) => {
                const on = s.name === "Karaoke";
                return (
                  <span
                    key={s.name}
                    className={`relative flex h-12 min-w-0 items-center justify-center rounded-[10px] border bg-white/[0.06] px-1 ${on ? "border-accent shadow-[0_0_0_1px_var(--color-accent)]" : "border-white/[0.08]"}`}
                  >
                    <StyleName s={s} />
                    {on && (
                      <span className="absolute right-1 top-1 flex size-3.5 items-center justify-center rounded-full bg-accent-strong shadow-[0_1px_2px_rgb(0_0_0/0.5)]"><Check size={10} strokeWidth={3} />
                      </span>
                    )}
                  </span>
                );
              })}
            </div>
          </div>
        </Scrolled>
        <div className="flex shrink-0 flex-col gap-2 border-t border-white/[0.1] bg-panel p-3.5">
          <span className={`flex h-8 items-center justify-center gap-1.5 rounded-lg text-[13px] font-medium ${SECONDARY}`}>
            <Icon i={Captions} size={15} />
            Regenerate captions
          </span>
          <span className="flex h-8 items-center justify-center gap-1.5 rounded-lg bg-white/[0.06] text-[12px] font-medium">
            <Icon i={ScanFace} size={14} />
            Find text over faces
          </span>
        </div>
      </div>
    </>
  );
}

// ---- Preview --------------------------------------------------------------------------------------------------

// The frame: 9:16 as tall as the preview box (the pane less 12 px above and the 64 px transport row).
const PANES_H = 900 - 48 - TIMELINE_H - 12;
const FRAME_H = PANES_H - 12 - 64;
const FRAME_W = Math.round((FRAME_H * 9) / 16);
// What the canvas's 1080 px are on screen.
const PT = FRAME_W / 1080;

// The preview's selection box: corner handles, the rotate handle (inside the box while it fills the frame) and, on a
// picture, a bar along each edge for cropping.
function SelectionBox({ text = false }: { text?: boolean }) {
  const corner = "absolute size-[9px] bg-fg shadow-[0_0_0_1px_rgb(0_0_0/0.45)]";
  const bar = "absolute bg-fg shadow-[0_0_0_1px_rgb(0_0_0/0.45)]";
  return (
    <>
      <span className="pointer-events-none absolute inset-0 outline-[1.5px] outline-fg [outline-style:solid]" />
      <span className={`${corner} -left-[4.5px] -top-[4.5px]`} />
      <span className={`${corner} -right-[4.5px] -top-[4.5px]`} />
      <span className={`${corner} -bottom-[4.5px] -left-[4.5px]`} />
      <span className={`${corner} -bottom-[4.5px] -right-[4.5px]`} />
      {!text && (
        <>
          <span className={`${bar} left-1/2 top-[-2.5px] h-[5px] w-[18px] -translate-x-1/2`} />
          <span className={`${bar} bottom-[-2.5px] left-1/2 h-[5px] w-[18px] -translate-x-1/2`} />
          <span className={`${bar} left-[-2.5px] top-1/2 h-[18px] w-[5px] -translate-y-1/2`} />
          <span className={`${bar} right-[-2.5px] top-1/2 h-[18px] w-[5px] -translate-y-1/2`} />
        </>
      )}
      {text && <span className="absolute -top-[18px] left-1/2 h-[18px] w-[1.5px] -translate-x-1/2 bg-fg" />}
      <span className={`absolute left-1/2 size-[11px] -translate-x-1/2 rounded-full border-[1.5px] border-fg ${text ? "-top-[29px] bg-fg" : "top-5 bg-black/30"}`} />
    </>
  );
}

// A caption as the engine draws it on the 1080 × 1920 canvas: Reel, the style new captions get, is 95 px Inter in
// white with a 7.5 px black outline; Karaoke is the same with the word being said in yellow.
const CAPTION_AT = 0.62;

function Caption({ words, said }: { words: string[]; said?: number }) {
  return (
    <div
      className="app-outline absolute inset-x-3 flex flex-wrap justify-center gap-x-[0.26em] leading-[1.2]"
      style={{ top: Math.round(FRAME_H * CAPTION_AT), fontSize: 95 * PT, ["--outline" as string]: `${7.5 * PT * 2}px` }}
    >
      {words.map((w, i) => (
        <span key={i} className={i === said ? "text-[#ffe14d]" : "text-white"}>
          {w}
        </span>
      ))}
    </div>
  );
}

// The preview pane: black around the 9:16 frame, and the bar under it. `over` sits over the frame's edges.
function Stage({ children, over, bar }: { children: React.ReactNode; over?: React.ReactNode; bar: React.ReactNode }) {
  return (
    <div className="flex min-w-0 flex-1 flex-col rounded-xl bg-stage shadow-[inset_0_0_0_1px_rgb(255_255_255/0.05)]">
      <div className="mx-4 mt-3 flex min-h-0 flex-1 items-center justify-center">
        <div className="relative shrink-0" style={{ width: FRAME_W, height: FRAME_H }}>
          <div className="absolute inset-0 overflow-hidden bg-black shadow-[0_0_0_1px_rgb(255_255_255/0.07)]">{children}</div>
          {over}
        </div>
      </div>
      <div className="flex shrink-0 justify-center px-5 py-2.5">
        <div className="app-bar grid h-11 w-full max-w-[620px] grid-cols-[1fr_auto_1fr] items-center gap-2 rounded-full pl-[18px] pr-1.5">{bar}</div>
      </div>
    </div>
  );
}

function PlayControls({ size = 38 }: { size?: number }) {
  return (
    <span className="flex items-center gap-2.5">
      <IconBtn i={SkipBack} />
      <span className="flex items-center justify-center rounded-full bg-fg text-black shadow-[0_1px_3px_rgb(0_0_0/0.4)]" style={{ width: size, height: size }}>
        <Play size={16} fill="currentColor" className="translate-x-px" />
      </span>
      <IconBtn i={SkipForward} />
    </span>
  );
}

function Preview({ focus, sizes }: { focus: EditorFocus; sizes: Sizes }) {
  return (
    <Stage
      over={
        focus === "captions" ? (
          <div className="absolute inset-x-[52px]" style={{ top: Math.round(FRAME_H * CAPTION_AT) - 4, height: Math.round(95 * PT * 1.2) + 8 }}>
            <SelectionBox text />
          </div>
        ) : (
          <SelectionBox />
        )
      }
      bar={
        <>
          <span className="truncate text-[13px] tabular-nums">
            <span className="text-[15px] font-semibold">
              00:10<span className="font-medium text-muted">.85</span>
            </span>
            <span className="text-muted"> / 00:28.60</span>
          </span>
          <PlayControls />
          <span className="flex items-center justify-end gap-1">
            <IconBtn i={Volume2} />
            <IconBtn i={Smartphone} />
            <span className="flex h-8 items-center gap-1.5 rounded-full px-2.5 text-[12px] font-medium">
              <Icon i={RectangleHorizontal} size={14} className="text-muted" />
              Ratio <span className="text-muted">9:16</span>
              <Icon i={ChevronDown} size={13} className="text-muted" />
            </span>
            <IconBtn i={Maximize} size={15} />
          </span>
        </>
      }
    >
      <Image src="/media/reel-hike.jpg" alt="" fill sizes={sizes(FRAME_W)} className="object-cover" />
      <Caption words={["we", "made", "it"]} said={focus === "captions" ? 1 : undefined} />
    </Stage>
  );
}

// ---- Inspector ------------------------------------------------------------------------------------------------

function InspectorHeader({ tone, i, title, detail }: { tone: string; i: typeof Film; title: string; detail: string }) {
  return (
    <div className="flex shrink-0 items-center gap-3 px-4 pb-3 pt-3.5">
      <span className={`flex size-8 shrink-0 items-center justify-center rounded-lg shadow-[inset_0_0_0_1px_rgb(255_255_255/0.12)] ${tone}`}>
        <Icon i={i} />
      </span>
      <div className="min-w-0">
        <div className="truncate text-[15px] font-semibold leading-5 tracking-[-0.015em]">{title}</div>
        <div className="mt-px truncate text-[12px] tabular-nums text-muted">{detail}</div>
      </div>
    </div>
  );
}

function Section({ title, actions, children, first = false }: { title?: string; actions?: React.ReactNode; children: React.ReactNode; first?: boolean }) {
  return (
    <section className={`flex flex-col gap-2 px-4 pb-3.5 pt-3 ${first ? "" : "border-t border-white/[0.06]"}`}>
      {title && (
        <div className="flex min-h-6 items-center justify-between gap-2">
          <h3 className="text-[13px] font-semibold">{title}</h3>
          {actions && <div className="-mr-1 flex items-center gap-0.5 text-muted">{actions}</div>}
        </div>
      )}
      <div className="flex flex-col gap-2">{children}</div>
    </section>
  );
}

function Quiet({ i, className = "" }: { i: typeof Film; className?: string }) {
  return (
    <span className={`flex size-6 items-center justify-center rounded-md ${className}`}>
      <Icon i={i} size={14} />
    </span>
  );
}

// One property row: the label in an 80 px column, the slider, and the value with its unit in a 64 px field.
// `origin` is where the fill starts, 0.5 for a range around zero, which marks it with a tick.
function Slider({ label, value, unit, at, origin = 0 }: { label: string; value: string; unit?: string; at: number; origin?: number }) {
  const [from, to] = at < origin ? [at, origin] : [origin, at];
  return (
    <div className="flex min-h-7 items-center gap-2">
      <span className="w-20 shrink-0 truncate text-[12px] leading-4 text-muted">{label}</span>
      <div className="relative h-4 min-w-0 flex-1">
        <div className="absolute inset-x-0 top-1.5 h-1 rounded-full bg-white/[0.13]" />
        {origin > 0 && <div className="absolute top-[3px] h-2.5 w-px bg-white/30" style={{ left: `${origin * 100}%` }} />}
        <div className="absolute top-1.5 h-1 bg-accent" style={{ left: `${from * 100}%`, width: `${(to - from) * 100}%` }} />
        <div
          className="absolute top-px h-3.5 w-[22px] rounded-[7px] bg-[#f7f7f9] shadow-[0_0_0_0.5px_rgb(0_0_0/0.35),0_1px_3px_rgb(0_0_0/0.5)]"
          style={{ left: `calc(${at * 100}% - ${at * 22}px)` }}
        />
      </div>
      <span className={`relative flex h-6 w-16 shrink-0 items-center justify-end rounded-md border border-white/[0.08] bg-white/[0.055] pl-1.5 text-[12px] tabular-nums ${unit ? "pr-[18px]" : "pr-1.5"}`}>
        {value}
        {unit && <span className="absolute right-1.5 text-[11px] text-muted">{unit}</span>}
      </span>
    </div>
  );
}

function VideoInspector() {
  return (
    <>
      <InspectorHeader tone="bg-clip-video" i={Film} title="IMG_2044.MOV" detail="8.7 s · from 2.0 s of 48.0 s" />
      <Seg items={["Video", "Adjust", "Speed", "Animation", "Audio"]} on={0} className="mx-3 mb-1" />
      <Scrolled thumb={[2, 300]}>
      <Section
        first
        title="Transform"
        actions={
          <>
            <Quiet i={ChevronLeft} className="opacity-40" />
            <Quiet i={Diamond} />
            <Quiet i={ChevronRight} className="opacity-40" />
            <Quiet i={RotateCcw} />
          </>
        }
      >
        <Seg items={["Fit", "Fill"]} on={0} icons={[Minimize, Maximize]} />
        <Slider label="Scale" value="100" unit="%" at={0.23} />
        <Slider label="Position X" value="0.0" unit="%" at={0.5} origin={0.5} />
        <Slider label="Position Y" value="0.0" unit="%" at={0.5} origin={0.5} />
        <Slider label="Rotation" value="0" unit="°" at={0.5} origin={0.5} />
        <Slider label="Opacity" value="100" unit="%" at={1} />
      </Section>
      <Section title="Crop and shape" actions={<Quiet i={RotateCcw} />}>
        <Seg items={["Rectangle", "Rounded", "Circle"]} on={0} />
        <Slider label="Crop left" value="0" unit="%" at={0} />
        <Slider label="Crop right" value="0" unit="%" at={0} />
        <Slider label="Crop top" value="0" unit="%" at={0} />
        <Slider label="Crop bottom" value="0" unit="%" at={0} />
      </Section>
      </Scrolled>
    </>
  );
}

// A label over its control, as a row whose label is long or whose control is wide has it.
function Labelled({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-[12px] text-muted">{label}</span>
      {children}
    </div>
  );
}

function Select({ value, className = "", off = false, font = "" }: { value: string; className?: string; off?: boolean; font?: string }) {
  return (
    <span className={`flex h-8 min-w-0 items-center justify-between gap-2 rounded-lg px-2.5 text-[13px] ${SECONDARY} ${off ? "opacity-40" : ""} ${className}`}>
      <span className={`truncate ${font}`}>{value}</span>
      <Icon i={ChevronDown} size={14} className="text-muted" />
    </span>
  );
}

// A label in the 80 px column of a property row, and its control.
function FieldRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex min-h-7 items-center gap-2">
      <span className="w-20 shrink-0 truncate text-[12px] text-muted">{label}</span>
      <div className="flex min-w-0 flex-1 items-center gap-2">{children}</div>
    </div>
  );
}

// The selected caption: its words, where its style goes, and the style itself (inspector/TextSection.tsx).
function CaptionInspector() {
  return (
    <>
      <InspectorHeader tone="bg-clip-captions" i={Captions} title="Caption" detail="3.2 s" />
      <Seg items={["Text", "Animation", "Transform"]} on={0} className="mx-3 mb-1" />
      <Scrolled thumb={[2, 240]}>
      <Section first title="Text">
        <span className="h-[52px] rounded-lg border border-white/[0.08] bg-white/[0.055] px-2.5 py-2 text-[13px] leading-[18px]">we made it</span>
        <Labelled label="Apply style to">
          <Seg items={["All captions", "This caption"]} on={0} />
        </Labelled>
      </Section>
      <Section title="Style">
        <Labelled label="Font">
          <div className="flex gap-2">
            <Select value="Inter" className="flex-1" />
            <Select value="Regular" className="w-[118px]" />
          </div>
        </Labelled>
        <Slider label="Size" value="95" at={0.29} />
        <FieldRow label="Color">
          <span className="ml-auto text-[12px] tabular-nums text-muted">#FFFFFF</span>
          <span className="h-5 w-8 rounded-[5px] bg-white shadow-[inset_0_0_0_1px_rgb(255_255_255/0.18)]" />
        </FieldRow>
        <Labelled label="Case">
          <Seg items={["", "AB", "ab", "Ab"]} icons={[Ban, null, null, null]} on={0} />
        </Labelled>
        <div className="grid grid-cols-3 gap-2">
          {captionStyles.slice(0, 6).map((s) => (
            <span
              key={s.name}
              className={`flex h-12 min-w-0 items-center justify-center rounded-[10px] border bg-white/[0.06] px-1 ${s.name === "Karaoke" ? "border-accent shadow-[0_0_0_1px_var(--color-accent)]" : "border-white/[0.08]"}`}
            >
              <StyleName s={s} />
            </span>
          ))}
        </div>
      </Section>
      </Scrolled>
    </>
  );
}

// ---- Timeline -------------------------------------------------------------------------------------------------

function Divider() {
  return <span className="mx-1 h-4 w-px bg-white/[0.12]" />;
}

function TimelineTools({ cover = false }: { cover?: boolean }) {
  return (
    <div className="flex h-11 shrink-0 items-center gap-1.5 px-2.5">
      {cover ? (
        <p className="truncate pl-1.5 text-[12px] text-muted">Click or drag on the timeline to choose the cover&apos;s frame.</p>
      ) : (
      <div className="app-bar flex items-center rounded-full">
        <IconBtn i={Undo2} />
        <IconBtn i={Redo2} off />
        <Divider />
        <IconBtn i={Scissors} />
        <IconBtn i={Snowflake} />
        <IconBtn i={PanelLeftClose} />
        <IconBtn i={PanelRightClose} />
        <IconBtn i={Trash2} />
        <Divider />
        <IconBtn i={Copy} size={15} />
        <IconBtn i={Unlink} size={15} />
        <IconBtn i={RotateCwSquare} size={15} />
        <Divider />
        <IconBtn i={Magnet} className="bg-accent/20 text-accent" />
      </div>
      )}
      <div className="flex-1" />
      <IconBtn i={ZoomOut} />
      <div className="relative h-4 w-24">
        <div className="absolute inset-x-0 top-1.5 h-1 rounded-full bg-white/[0.13]" />
        <div className="absolute left-0 top-1.5 h-1 w-[45%] rounded-l-full bg-accent" />
        <div className="absolute left-[calc(45%-10px)] top-px h-3.5 w-[22px] rounded-[7px] bg-[#f7f7f9] shadow-[0_1px_3px_rgb(0_0_0/0.5)]" />
      </div>
      <IconBtn i={ZoomIn} />
      <IconBtn i={Maximize2} size={15} />
    </div>
  );
}

function TrackTile({ tone, i }: { tone: string; i: typeof Film }) {
  return (
    <span className={`mr-1.5 flex size-6 shrink-0 items-center justify-center rounded-[7px] shadow-[inset_0_1px_0_rgb(255_255_255/0.18)] ${tone}`}>
      <Icon i={i} size={13} />
    </span>
  );
}

function Toggle({ i }: { i?: typeof Film }) {
  return <span className="flex size-[22px] shrink-0 items-center justify-center text-muted">{i && <Icon i={i} size={14} />}</span>;
}

// A track's row: its header pinned left and its lane, 4 px apart from the next row.
function Row({ h, header, children, main = false }: { h: number; header: React.ReactNode; children?: React.ReactNode; main?: boolean }) {
  return (
    <div className="relative flex shrink-0" style={{ height: h + 4 }}>
      <div className="flex shrink-0 items-center gap-0.5 border-r border-white/[0.07] bg-panel pl-2 pr-1" style={{ width: HEADER_W }}>
        {header}
      </div>
      <div className={`relative flex-1 ${main ? "bg-white/[0.035]" : ""}`} style={{ marginBlock: 2 }}>
        {children}
      </div>
    </div>
  );
}

const EDGE = "shadow-[inset_0_1px_0_rgb(255_255_255/0.2),inset_0_-1px_0_rgb(0_0_0/0.35),inset_0_0_0_1px_rgb(255_255_255/0.1)]";

function ClipBox({ left, width, className = "", selected = false, children }: { left: number; width: number; className?: string; selected?: boolean; children?: React.ReactNode }) {
  return (
    <div className={`absolute bottom-1 top-1 overflow-hidden rounded-[4px] ${className}`} style={{ left: left + 1, width: width - 2 }}>
      {children}
      <span className={`pointer-events-none absolute inset-0 rounded-[4px] ${selected ? "shadow-[inset_0_0_0_2px_var(--color-accent)]" : EDGE}`} />
    </div>
  );
}

// A picture clip's frames, 9:16 at the clip's height, cut from its picture.
function Frames({ image, n, sizes }: { image: string; n: number; sizes: Sizes }) {
  return (
    <div className="absolute inset-0 flex">
      {Array.from({ length: n }, (_, k) => (
        <span key={k} className="relative h-full w-[31.5px] shrink-0">
          <Image src={`/media/${image}.jpg`} alt="" fill sizes={sizes(32)} className="object-cover" />
        </span>
      ))}
    </div>
  );
}

function Bars({ n, seed, peak, color }: { n: number; seed: number; peak: number; color: string }) {
  return (
    <div className="absolute inset-x-0 bottom-0 flex h-full items-end gap-px">
      {wave(n, seed, peak).map((h, k) => (
        <span key={k} className="w-[2px] shrink-0" style={{ height: h, background: color }} />
      ))}
    </div>
  );
}

function Lanes({ focus, sizes }: { focus: EditorFocus; sizes: Sizes }) {
  const captionsOn = focus === "captions";
  const editOn = focus === "edit" || focus === "export";
  return (
    <>
      <Row
        h={34}
        header={
          <>
            <TrackTile tone="bg-clip-captions" i={Captions} />
            <span className="min-w-0 flex-1 truncate text-[12px] font-medium">Captions</span>
            <Toggle i={Eye} />
            <Toggle />
          </>
        }
      >
        {captionLine.map(([from, to, text]) => {
          const selected = captionsOn && text === "we made it";
          return (
            <ClipBox key={text} left={x(from)} width={x(to) - x(from)} selected={selected} className={selected ? "bg-clip-captions" : "bg-clip-captions/55"}>
              <span className="absolute inset-0 flex items-center gap-1.5 px-1 text-[11px]">
                <span className="whitespace-nowrap text-fg/90">{text}</span>
                {x(to) - x(from) > 110 && <span className="shrink-0 tabular-nums text-fg/70">{(to - from).toFixed(1)} s</span>}
              </span>
            </ClipBox>
          );
        })}
      </Row>
      {/* The blank lane over the video, where a picture dragged in goes. */}
      <Row h={20} header={null} />
      <Row
        h={64}
        main
        header={
          <>
            <span className="flex min-w-0 flex-1 items-center gap-2">
              <CoverTile on={focus === "cover"} sizes={sizes} />
              <span className="truncate text-[12px] font-medium">Main</span>
            </span>
            <Toggle i={Eye} />
            <Toggle i={Volume2} />
          </>
        }
      >
        {clips.map((c, n) => {
          const selected = editOn && n === 1;
          const w = x(c.length);
          return (
            <ClipBox key={c.name} left={x(c.start)} width={w} selected={selected} className="bg-clip-video">
              <Frames image={c.image} n={Math.ceil(w / 31.5)} sizes={sizes} />
              <div className="absolute inset-x-0 bottom-0 h-3.5 overflow-hidden bg-[#141416]">
                <Bars n={Math.floor(w / 3)} seed={n + 1} peak={11} color="#a1a1a9" />
              </div>
              <span className="absolute left-2.5 top-1 flex h-[17px] items-center gap-1 rounded-[3px] bg-black/60 pl-[5px] pr-1.5 text-[11px] font-medium">
                <Icon i={Film} size={11} />
                {c.name}
                <span className="pl-0.5 font-normal tabular-nums text-fg/80">{c.length.toFixed(1)} s</span>
              </span>
              {selected && (
                <>
                  <span className="absolute inset-y-0 left-0 flex w-2 items-center justify-center bg-accent">
                    <span className="h-3.5 w-0.5 rounded-full bg-black/55" />
                  </span>
                  <span className="absolute inset-y-0 right-0 flex w-2 items-center justify-center bg-accent">
                    <span className="h-3.5 w-0.5 rounded-full bg-black/55" />
                  </span>
                </>
              )}
            </ClipBox>
          );
        })}
      </Row>
      <Row
        h={58}
        header={
          <>
            <TrackTile tone="bg-clip-audio" i={AudioLines} />
            <span className="min-w-0 flex-1 truncate text-[12px] font-medium">Music</span>
            <Toggle />
            <Toggle i={Volume2} />
          </>
        }
      >
        <ClipBox left={0} width={x(LENGTH)} className="bg-clip-audio">
          <span className="absolute left-2.5 top-0.5 flex h-3.5 items-center gap-1 text-[11px] font-medium">
            <Icon i={AudioLines} size={11} />
            Sunrise.m4a
            <span className="font-normal tabular-nums text-fg/80">28.6 s</span>
          </span>
          <div className="absolute inset-x-0 bottom-[3px] top-[17px]">
            <Bars n={Math.floor(x(LENGTH) / 3)} seed={9} peak={28} color="#5fd497" />
          </div>
        </ClipBox>
      </Row>
    </>
  );
}

function CoverTile({ on, sizes }: { on: boolean; sizes: Sizes }) {
  return (
    <span className={`relative h-14 w-11 shrink-0 overflow-hidden rounded-[7px] ${on ? "shadow-[0_0_0_2px_var(--color-accent)]" : "shadow-[inset_0_0_0_1px_rgb(255_255_255/0.1)]"}`}>
      <Image src="/media/reel-hike.jpg" alt="" fill sizes={sizes(44)} className="object-cover" />
      <span className="app-outline absolute inset-x-0 top-[34px] text-center font-anton text-[7px] leading-none text-white" style={{ ["--outline" as string]: "1px" }}>
        MORNING RUN
      </span>
      <span className="absolute inset-x-0.5 bottom-0.5 rounded-[4px] bg-black/60 text-center text-[11px] leading-[14px]">Cover</span>
    </span>
  );
}

function Ruler() {
  const marks = Array.from({ length: 16 }, (_, k) => k * 2);
  return (
    <div className="relative flex h-7 shrink-0">
      <div className="shrink-0 border-b border-r border-white/[0.07] bg-panel" style={{ width: HEADER_W }} />
      <div className="relative flex-1 overflow-hidden border-b border-white/[0.07]">
        {marks.map((t) => (
          <div key={t} className="absolute top-0 h-full" style={{ left: x(t) }}>
            <span className="absolute left-1 top-[5px] text-[11px] leading-[13px] tabular-nums text-muted">00:{String(t).padStart(2, "0")}</span>
            <span className="absolute bottom-0 h-2 w-px bg-subtle" />
            {[1, 2, 3, 4].map((i) => (
              <span key={i} className="absolute bottom-0 h-1 w-px bg-subtle/60" style={{ left: (i * 2 * PX) / 5 }} />
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}

function Timeline({ focus, sizes }: { focus: EditorFocus; sizes: Sizes }) {
  const cover = focus === "cover";
  return (
    <div className="app-pane flex shrink-0 flex-col overflow-hidden bg-well" style={{ height: TIMELINE_H }}>
      <TimelineTools cover={cover} />
      <div className="relative mx-px mb-px flex min-h-0 flex-1 flex-col overflow-hidden">
        <Ruler />
        <Lanes focus={focus} sizes={sizes} />
        <div className="flex flex-1">
          <div className="shrink-0 border-r border-white/[0.07] bg-panel" style={{ width: HEADER_W }} />
        </div>
        {/* The playhead: its head in the ruler's tick band, under the time labels. */}
        <div className="pointer-events-none absolute bottom-0 top-0" style={{ left: HEADER_W + x(cover ? 8.2 : PLAYHEAD) }}>
          <div className="absolute -left-[0.75px] bottom-0 top-5 w-[1.5px] bg-white shadow-[0_0_0_0.5px_rgb(0_0_0/0.4)]" />
          <div className="absolute -left-[5px] top-[17px] h-[11px] w-[10px] rounded-t-[2px] bg-white [clip-path:polygon(0_0,100%_0,100%_55%,50%_100%,0_55%)]" />
        </div>
      </div>
    </div>
  );
}

// ---- Reels from a long video --------------------------------------------------------------------------------

// What is said around the reel; the reel's words are lit (Reels from a long video, Trimming).
const said = [
  ["06:01", "So we left at five, the car was freezing and honestly I almost turned back.", 0],
  ["06:07", "Nobody tells you the last hour is the best part.", 1],
  ["06:12", "You stop thinking about your legs and you just |look| up, and the whole ridge is lit.", 1],
  ["06:21", "That's when it clicks, why you get up at four for this.", 1],
  ["06:29", "Anyway, the coffee at the top was terrible.", 0],
  ["06:34", "Next time I'm bringing a better stove.", 0],
] as const;

function ReelTrim() {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-col gap-0.5 border-b border-white/[0.07] px-3.5 py-3">
        <h2 className="text-[13px] font-semibold">The last hour</h2>
        <p className="text-[12px] tabular-nums text-muted">38.4 s once cut · 06:07–06:29</p>
      </div>
      <div className="min-h-0 flex-1 overflow-hidden px-3.5 pb-1 pt-3">
        {said.map(([at, text, inReel]) => (
          <p key={at} className="mb-3 grid grid-cols-[40px_1fr] text-[13px] leading-[22px]">
            <span className="text-[11px] tabular-nums text-muted">{at}</span>
            <span>
              {inReel ? (
                <span className="rounded-sm bg-accent/30 box-decoration-clone">
                  {text.split("|").map((part, i) => (i === 1 ? <span key={i} className="bg-accent text-black">{part}</span> : part))}
                </span>
              ) : (
                <span className="text-muted">{text}</span>
              )}
            </span>
          </p>
        ))}
      </div>
      <p className="shrink-0 border-t border-white/[0.07] px-3.5 py-2 text-[12px] text-muted">Shift-click a word to start or end the reel there.</p>
    </div>
  );
}

// The long vlog, playing the reel shown.
function ReelPreview({ sizes }: { sizes: Sizes }) {
  return (
    <Stage
      bar={
        <>
          <span className="text-[13px] font-medium tabular-nums">06:12.40</span>
          <PlayControls size={36} />
          <span className="flex justify-end">
            <span className={`flex h-8 items-center rounded-full px-3.5 text-[13px] font-medium ${SECONDARY}`}>Done</span>
          </span>
        </>
      }
    >
      <Image src="/media/reel-hike.jpg" alt="" fill sizes={sizes(FRAME_W)} className="object-cover" />
      <Caption words={["you", "just", "look", "up"]} />
    </Stage>
  );
}

const reels = [
  { title: "The last hour", length: "38.4 s", hook: "Nobody tells you the last hour is the best part.", why: "Opens on a strong line and ends on the payoff.", score: 92, checked: true },
  { title: "Turning back", length: "24.1 s", hook: "Honestly, I almost turned back.", why: "Honest and short, it works without the rest.", score: 84, checked: true },
  { title: "Coffee at the top", length: "31.0 s", hook: "Anyway, the coffee at the top was terrible.", why: "Funny, and it ends on a laugh.", score: 71, checked: false },
  { title: "What's in the bag", length: "1:12", hook: "People always ask what I carry up there.", why: "Useful, but over a minute once cut.", score: 58, checked: false, long: true },
];

function Checkbox({ on }: { on: boolean }) {
  return (
    <span className={`flex size-4 shrink-0 items-center justify-center rounded-[4px] ${on ? "bg-accent-strong" : "border border-white/25 bg-white/[0.07]"}`}>
      {on && (<Check size={10} strokeWidth={3} />
      )}
    </span>
  );
}

function ScoreBars({ lit }: { lit: number }) {
  return (
    <span className="flex h-3 shrink-0 items-end gap-[2px]">
      {[5, 8, 11].map((h, i) => (
        <span key={h} className={`w-[3px] rounded-[1px] ${i < lit ? "bg-fg/85" : "bg-white/20"}`} style={{ height: h }} />
      ))}
    </span>
  );
}

function ReelList() {
  return (
    <div className="app-pane flex flex-col overflow-hidden" style={{ height: TIMELINE_H }}>
      <div className="flex min-h-11 shrink-0 items-center gap-x-2 px-3 py-2">
        <h2 className="text-[13px] font-semibold">Reels</h2>
        <span className="text-[13px] tabular-nums text-muted">4 · 2 checked</span>
        <span className="flex h-7 items-center gap-1.5 px-2 text-[12px] font-medium text-accent">
          <Icon i={Sparkles} size={14} />
          Find again
        </span>
        <div className="ml-auto flex items-center gap-2">
          <span className={`flex h-7 items-center gap-1.5 rounded-full px-3 text-[12px] font-medium ${SECONDARY}`}>
            Each reel gets <span className="text-muted">all</span>
            <Icon i={ChevronDown} size={14} className="text-muted" />
          </span>
          <span className="text-[12px] text-muted">9:16</span>
          <Seg items={["Center crop", "Blurred background"]} on={0} className="w-[248px]" />
          <span className="app-prominent flex h-8 items-center rounded-full px-3.5 text-[13px] font-medium">Make 2 reels</span>
        </div>
      </div>
      <div className="grid min-h-0 flex-1 auto-rows-min grid-cols-4 gap-2 p-2 pt-0">
        {reels.map((r, i) => (
          <div
            key={r.title}
            className={`flex gap-3 rounded-xl p-2.5 ${i === 0 ? "bg-raised shadow-[inset_0_0_0_2px_var(--color-accent)]" : "bg-white/[0.055] shadow-[inset_0_0_0_1px_rgb(255_255_255/0.06)]"}`}
          >
            <span className="flex h-[100px] w-14 shrink-0 items-center justify-center rounded-[7px] border border-dashed border-white/15 text-muted">
              <Icon i={Film} size={14} />
            </span>
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <div className="flex items-center gap-2">
                <Checkbox on={r.checked} />
                <span className="min-w-0 flex-1 truncate text-[13px] font-semibold">{r.title}</span>
                <span className={`flex shrink-0 items-center gap-1 text-[12px] tabular-nums ${r.long ? "text-warn" : "text-muted"}`}>{r.length}</span>
              </div>
              <p className="line-clamp-2 text-[13px] leading-[18px] text-fg/90">“{r.hook}”</p>
              <p className="line-clamp-2 text-[12px] leading-4 text-muted">{r.why}</p>
              <div className="mt-auto flex items-center gap-2 pt-1">
                <span className="flex flex-1 items-center gap-1.5 text-[12px] tabular-nums text-muted">
                  <ScoreBars lit={r.score >= 75 ? 3 : r.score >= 50 ? 2 : 1} />
                  {r.score}
                </span>
                <span className="flex h-6 items-center px-2 text-[12px] font-medium">Reject</span>
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ---- Cover ----------------------------------------------------------------------------------------------------

const frames = [
  ["reel-hike", "00:08.2", "Sharp, well lit, well framed"],
  ["tile-1", "00:01.6", "Sharp, well lit"],
  ["tile-3", "00:17.0", "Well lit, well framed"],
  ["tile-4", "00:24.6", "Sharp"],
] as const;

function CoverFrames({ sizes }: { sizes: Sizes }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-12 shrink-0 items-center justify-between gap-2 border-b border-white/[0.07] pl-4 pr-2">
        <h2 className="text-[13px] font-semibold">Frames</h2>
        <span className="flex h-7 items-center gap-1 px-2 text-[12px] font-medium text-accent">
          <Icon i={Sparkles} size={13} />
          Make it with AI
        </span>
      </div>
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-hidden px-4 pb-4 pt-3">
        <span className={`flex h-8 items-center justify-center gap-1.5 rounded-lg text-[13px] font-medium ${SECONDARY}`}>
          <Icon i={ScanFace} size={15} />
          Pick again
        </span>
        <div className="grid grid-cols-2 gap-x-2 gap-y-3">
          {frames.map(([image, at, why], i) => (
            <div key={at} className="flex min-w-0 flex-col gap-1">
              <div className={`relative aspect-[9/16] overflow-hidden rounded-[10px] border ${i === 0 ? "border-accent shadow-[0_0_0_1px_var(--color-accent)]" : "border-white/[0.08]"}`}>
                <Image src={`/media/${image}.jpg`} alt="" fill sizes={sizes(131)} className="object-cover" />
                <span className="absolute bottom-1 right-1 rounded bg-black/60 px-1 text-[11px] leading-4 tabular-nums">{at}</span>
                {i === 0 && (
                  <span className="absolute right-1 top-1 flex size-4 items-center justify-center rounded-full bg-accent-strong shadow-[0_1px_2px_rgb(0_0_0/0.5)]"><Check size={10} strokeWidth={3} />
                  </span>
                )}
              </div>
              <span className={`truncate text-[11px] leading-[14px] ${i === 0 ? "text-fg" : "text-muted"}`}>{why}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

// The cover as it exports: the hook in Outlined, 170 px Anton with a 14 px black outline, behind the hikers.
const COVER_TOP = Math.round(FRAME_H * 0.575);

function CoverPreview({ sizes }: { sizes: Sizes }) {
  return (
    <Stage
      over={
        <div className="absolute inset-x-[26px]" style={{ top: COVER_TOP - 8, height: Math.round(170 * PT) + 14 }}>
          <SelectionBox text />
        </div>
      }
      bar={
        <>
          <span className="text-[13px] tabular-nums">
            <span className="text-muted">Frame </span>
            <span className="font-medium">00:08.20</span>
          </span>
          <Seg items={["9:16 Cover", "16:9 YouTube"]} on={0} className="w-[200px]" />
          <span />
        </>
      }
    >
      <Image src="/media/reel-hike.jpg" alt="" fill sizes={sizes(FRAME_W)} className="object-cover" />
      <div className="app-outline absolute inset-x-0 text-center font-anton leading-none text-white" style={{ top: COVER_TOP, fontSize: 170 * PT, ["--outline" as string]: `${14 * PT * 2}px` }}>
        MORNING RUN
      </div>
    </Stage>
  );
}

function Switch({ on }: { on: boolean }) {
  return (
    <span className={`relative h-[22px] w-[38px] shrink-0 rounded-full ${on ? "bg-accent-strong" : "bg-white/[0.16]"}`}>
      <span className={`absolute top-0.5 size-[18px] rounded-full bg-white shadow-[0_1px_3px_rgb(0_0_0/0.4)] ${on ? "left-[18px]" : "left-0.5"}`} />
    </span>
  );
}

// The cover's hook, selected: its words, the three cover looks, Behind person and its style (cover/CoverInspector.tsx).
const coverLooks = [
  ["Big", "text-white"],
  ["Yellow box", "rounded-[3px] bg-[#ffd400] px-1 text-[#111]"],
  ["Outlined", "app-outline text-white"],
] as const;

function CoverInspector() {
  return (
    <>
      <div className="flex items-start justify-between pr-2">
        <InspectorHeader tone="bg-raised" i={Type} title="Text" detail="On the 9:16 cover" />
        <div className="flex items-center gap-0.5 pt-3 text-muted">
          <IconBtn i={Copy} size={15} />
          <IconBtn i={Trash2} size={15} />
        </div>
      </div>
      <Section first>
        <span className="h-[52px] rounded-lg border border-white/[0.08] bg-white/[0.055] px-2.5 py-2 text-[13px] leading-[18px]">MORNING RUN</span>
        <div className="grid grid-cols-3 gap-2">
          {coverLooks.map(([name, look]) => {
            const on = name === "Outlined";
            return (
              <div key={name} className="flex flex-col gap-1">
                <span className={`relative flex h-16 items-center justify-center overflow-hidden rounded-[10px] bg-[#5c5c63] ${on ? "shadow-[inset_0_0_0_2px_var(--color-accent)]" : ""}`}>
                  <span className={`font-anton text-[22px] leading-none ${look}`} style={{ ["--outline" as string]: "3px" }}>
                    Aa
                  </span>
                  {on && (
                    <span className="absolute right-1 top-1 flex size-3.5 items-center justify-center rounded-full bg-accent-strong shadow-[0_1px_2px_rgb(0_0_0/0.5)]"><Check size={10} strokeWidth={3} />
                    </span>
                  )}
                </span>
                <span className={`text-[11px] ${on ? "font-medium text-fg" : "text-muted"}`}>{name}</span>
              </div>
            );
          })}
        </div>
        <div className="flex min-h-6 items-center justify-between gap-2 text-[13px]">
          Behind person
          <Switch on />
        </div>
      </Section>
      <Section>
        <Labelled label="Font">
          <div className="flex gap-2">
            <Select value="Anton" font="font-anton text-[15px]" className="flex-1" />
            <Select value="Regular" off className="w-[118px]" />
          </div>
        </Labelled>
        <Slider label="Size" value="170" at={0.55} />
        <FieldRow label="Color">
          <span className="ml-auto text-[12px] tabular-nums text-muted">#FFFFFF</span>
          <span className="h-5 w-8 rounded-[5px] bg-white shadow-[inset_0_0_0_1px_rgb(255_255_255/0.18)]" />
        </FieldRow>
        <Labelled label="Case">
          <Seg items={["", "AB", "ab", "Ab"]} icons={[Ban, null, null, null]} on={0} />
        </Labelled>
      </Section>
      {[
        ["More style", "Outline, box, shadow, spacing, alignment"],
        ["Position", "Scale, rotation, opacity"],
      ].map(([title, inside]) => (
        <div key={title} className="flex h-[54px] items-center gap-2 border-t border-white/[0.06] px-4 text-[12px]">
          <Icon i={ChevronRight} size={14} className="text-muted" />
          <span className="shrink-0 text-[13px] font-semibold">{title}</span>
          <span className="truncate text-muted">{inside}</span>
        </div>
      ))}
    </>
  );
}

// ---- Export ---------------------------------------------------------------------------------------------------

// The Export dialog on the Reels & TikTok preset. About 16 MB is the app's estimate for 28.6 s at 1080 × 1920, 30 fps,
// Recommended.
function ExportDialog() {
  const rows: [string, React.ReactNode][] = [
    ["Resolution", <Seg key="r" items={["720p", "1080p", "1440p", "4K"]} on={1} />],
    ["Frame rate", <Seg key="f" items={["24", "25", "30", "50", "60"]} on={2} />],
    ["Quality", <Seg key="q" items={["High", "Recommended", "Smaller file"]} on={1} />],
  ];
  return (
    <div className="absolute inset-0 flex items-center justify-center bg-black/55">
      <div className="app-dialog w-[500px] rounded-[20px] p-6">
        <div className="mb-5 flex items-start justify-between gap-3">
          <div className="flex flex-col gap-1">
            <span className="text-[17px] font-semibold leading-[22px] tracking-[-0.025em]">Export video</span>
            <span className="text-[12px] tabular-nums text-muted">MP4 · H.264 + AAC · 9:16 · 1080×1920 · 00:28.60</span>
          </div>
          <IconBtn i={X} className="-mr-2 -mt-1 text-fg" />
        </div>
        <Seg items={["Video", "Cover", "Recap"]} on={0} className="mb-5" />
        <div className="grid grid-cols-[104px_minmax(0,1fr)] items-center gap-x-3.5 gap-y-3">
          <span className="text-right text-[13px] text-muted">Preset</span>
          <span className="flex h-8 items-center gap-2 rounded-lg bg-white/[0.09] px-2.5 text-[13px] shadow-[inset_0_0_0_1px_rgb(255_255_255/0.06),inset_0_1px_0_rgb(255_255_255/0.06)]">
            <span className="flex-1">Reels &amp; TikTok</span>
            <Icon i={ChevronDown} size={14} className="text-muted" />
          </span>
          <span className="col-start-2 -mt-1 flex items-center gap-1.5 text-[12px] text-muted">
            <Icon i={AudioLines} size={14} />
            Loudness −14 LUFS, applied to the file
          </span>
          {rows.map(([label, control]) => (
            <Fragment key={label}>
              <span className="text-right text-[13px] text-muted">{label}</span>
              {control}
            </Fragment>
          ))}
        </div>
        <div className="mt-6 flex items-center justify-between gap-3 border-t border-white/[0.08] pt-4">
          <span className="text-[13px] tabular-nums text-muted">
            about <span className="font-semibold text-fg">16 MB</span>
          </span>
          <div className="flex gap-2.5">
            <span className={`flex h-8 items-center rounded-full px-3.5 text-[13px] font-medium ${SECONDARY}`}>Close</span>
            <span className="app-prominent flex h-8 items-center rounded-full px-3.5 text-[13px] font-medium">Export…</span>
          </div>
        </div>
      </div>
    </div>
  );
}

// ---- The window -----------------------------------------------------------------------------------------------

export function EditorMock({ focus = "edit", framed = false }: { focus?: EditorFocus; framed?: boolean }) {
  // Image sizes in the page's space, where the window is drawn at SCALE.
  const sizes: Sizes = (w) => (framed ? viewSizes : fitSizes)(Math.ceil(w * SCALE), MOCK_W);
  const reels = focus === "reels";
  const cover = focus === "cover";

  return (
    <div className="relative overflow-hidden rounded-[14px] text-left text-fg ring-1 ring-white/10" style={{ width: MOCK_W, height: MOCK_H }}>
      <div className="relative flex origin-top-left flex-col bg-window text-[13px] leading-[1.25] [font-feature-settings:normal]" style={{ width: 1440, height: 900, transform: `scale(${SCALE})` }}>
        <TopBar focus={focus} />
        <div className="flex min-h-0 flex-1 gap-1.5 px-1.5">
          <div className="app-pane flex shrink-0 flex-col overflow-hidden" style={{ width: LIBRARY_W }}>
            {reels ? <ReelTrim /> : cover ? <CoverFrames sizes={sizes} /> : focus === "captions" ? <CaptionsTab /> : <MediaTab sizes={sizes} />}
          </div>
          {reels ? <ReelPreview sizes={sizes} /> : cover ? <CoverPreview sizes={sizes} /> : <Preview focus={focus} sizes={sizes} />}
          {!reels && (
            <div className="app-pane flex shrink-0 flex-col overflow-hidden" style={{ width: INSPECTOR_W }}>
              {cover ? <CoverInspector /> : focus === "captions" ? <CaptionInspector /> : <VideoInspector />}
            </div>
          )}
        </div>
        <div className="shrink-0 px-1.5 pb-1.5 pt-1.5">{reels ? <ReelList /> : <Timeline focus={focus} sizes={sizes} />}</div>
        {focus === "export" && <ExportDialog />}
      </div>
    </div>
  );
}

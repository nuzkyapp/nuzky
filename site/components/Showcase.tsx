"use client";

import { useEffect, useRef, useState } from "react";
import { Captions, ChevronDown, Film, ImagePlus, Plus, Scissors, Upload, X } from "lucide-react";
import { EditorMock, type EditorFocus } from "./EditorMock";
import { Fit, type FitView } from "./Fit";
import { usePlaying } from "@/lib/playing";

// Positions are in the editor's own 1200 × 740 space. A hotspot sits on a gap beside what it describes, never on a
// control or a label, and `tip` is its tooltip's top left corner, placed where it covers neither that nor the
// picture. A phone has no room for hotspots and shows `view`, the part of the editor the spot talks about, with the
// text in a card below. Its 16:15 window fits the editor's 740 px height only up to `w` 789.
type Spot = { x: number; y: number; tip: [number, number]; title: string; text: string; view: FitView };
type Tab = { id: EditorFocus; label: string; icon: typeof Scissors; summary: string; spots: Spot[] };

const tabs: Tab[] = [
  {
    id: "edit",
    label: "Edit",
    icon: Scissors,
    summary: "A timeline, a live preview and an inspector, laid out the way you know from CapCut.",
    spots: [
      { x: 330, y: 512, tip: [16, 404], title: "Magnetic timeline", text: "Clips snap together and every cut lands on the exact frame, even in phone video.", view: { x: 640, y: 610, w: 360 } },
      { x: 360, y: 440, tip: [16, 404], title: "Your media, and YouTube", text: "Drop in video, sound and images, or paste a YouTube link to something that's yours to use.", view: { x: 181, y: 230, w: 350 } },
      { x: 960, y: 512, tip: [900, 546], title: "Inspector", text: "Video, Adjust, Speed, Animation and Audio, with a value you can type beside every slider.", view: { x: 1049, y: 284, w: 300 } },
    ],
  },
  {
    id: "captions",
    label: "Captions",
    icon: Captions,
    summary: "AI on your computer writes the captions and puts each line on the timeline as a clip.",
    spots: [
      { x: 180, y: 418, tip: [16, 488], title: "19 styles, or your own", text: "Pick a style, save your own look, and colour the key words.", view: { x: 181, y: 220, w: 350 } },
      { x: 1130, y: 570, tip: [880, 600], title: "A clip per line", text: "Fix a word or move a line right on the timeline.", view: { x: 480, y: 580, w: 360 } },
      { x: 745, y: 329, tip: [775, 305], title: "The spoken word lights up", text: "Each word lights up as you say it, in the preview and in the export.", view: { x: 630, y: 260, w: 330 } },
    ],
  },
  {
    id: "reels",
    label: "Reels",
    icon: Film,
    summary: "One long video becomes several reels. AI finds the moments, and you trim them by words.",
    spots: [
      { x: 359, y: 488, tip: [60, 340], title: "AI finds the moments", text: "Each reel opens on a hook and says why it works. Check the ones you want.", view: { x: 300, y: 620, w: 430 } },
      { x: 181, y: 310, tip: [211, 286], title: "Trim by words", text: "Click a word to play from it, and move the start or the end a word at a time.", view: { x: 181, y: 250, w: 350 } },
      { x: 1130, y: 484, tip: [880, 322], title: "Each one its own project", text: "Make 2 reels writes two 9:16 projects. The long video stays as it was.", view: { x: 960, y: 560, w: 470 } },
    ],
  },
  {
    id: "cover",
    label: "Cover",
    icon: ImagePlus,
    summary: "A 9:16 cover and a 16:9 YouTube thumbnail, made from a frame of your video.",
    spots: [
      { x: 78, y: 625, tip: [108, 601], title: "Cover first", text: "The cover sits before your first clip, as in CapCut. Click it to make one.", view: { x: 200, y: 610, w: 380 } },
      { x: 180, y: 462, tip: [100, 490], title: "Pick for me", text: "Nuzky looks through the video for sharp, well framed frames of you, best first. Or let AI make the whole cover.", view: { x: 181, y: 260, w: 350 } },
      { x: 1050, y: 462, tip: [880, 490], title: "Text behind you", text: "Put the title behind the person, draw a line around them, or blur what's behind. Export it as PNG or JPEG.", view: { x: 1049, y: 230, w: 300 } },
    ],
  },
  {
    id: "export",
    label: "Export",
    icon: Upload,
    summary: "Export for Reels, TikTok or YouTube in one click. The file looks exactly like the preview.",
    spots: [
      { x: 866, y: 378, tip: [896, 354], title: "Made for Reels and TikTok", text: "1080 × 1920 at 30 fps, with the sound as loud as the rest of the feed.", view: { x: 600, y: 370, w: 500 } },
      { x: 334, y: 532, tip: [12, 508], title: "Same renderer", text: "The same renderer draws the preview and the export.", view: { x: 600, y: 370, w: 500 } },
      { x: 866, y: 271, tip: [896, 247], title: "Cover and recap too", text: "Export the cover as a picture, or a short video of everything that was cut.", view: { x: 600, y: 370, w: 500 } },
    ],
  },
];

// Each hotspot opens in turn for this long; after the last one the tour moves to the next tab.
const SPOT_MS = 3200;

function Choice({ items, on }: { items: string[]; on: number }) {
  return (
    <div className="flex flex-1 rounded-[9px] bg-white/[0.045] p-0.5">
      {items.map((t, i) => (
        <span key={t} className={`flex-1 rounded-[7px] py-1.5 text-center text-[12px] font-medium ${i === on ? "bg-white/15" : "text-muted"}`}>
          {t}
        </span>
      ))}
    </div>
  );
}

// The app's Export dialog on the Reels & TikTok preset.
function ExportDialog() {
  const rows: [string, React.ReactNode][] = [
    ["Export", <Choice key="e" items={["Video", "Cover", "Recap"]} on={0} />],
    [
      "Preset",
      <span key="p" className="flex h-[30px] flex-1 items-center justify-between rounded-[8px] bg-white/[0.07] px-3 text-[12px] ring-1 ring-inset ring-white/[0.08]">
        Reels &amp; TikTok
        <ChevronDown className="size-3.5 text-muted" />
      </span>,
    ],
    ["Resolution", <Choice key="r" items={["720p", "1080p", "1440p", "4K"]} on={1} />],
    ["Frame rate", <Choice key="f" items={["24", "25", "30", "50", "60"]} on={2} />],
    ["Quality", <Choice key="q" items={["High", "Recommended", "Smaller file"]} on={1} />],
  ];
  return (
    <div className="absolute inset-0 flex items-center justify-center rounded-[14px] bg-black/45 animate-[fade-up_200ms_ease-out]">
      <div className="flex w-[480px] flex-col gap-4 rounded-[20px] bg-[#232327] p-6 text-left shadow-[0_24px_60px_rgb(0_0_0/0.6)] ring-1 ring-white/10">
        <div className="flex items-start justify-between">
          <div className="flex flex-col gap-1">
            <span className="text-[17px] font-semibold">Export video</span>
            <span className="text-[12px] tabular-nums text-muted">MP4 · H.264 + AAC · 9:16 · 00:30.00</span>
          </div>
          <X className="size-4 text-muted" />
        </div>
        <div className="flex flex-col gap-2.5">
          {rows.map(([label, control], i) => (
            <div key={label} className="flex flex-col gap-1.5">
              <div className="flex items-center gap-3">
                <span className="w-[76px] shrink-0 text-right text-[12px] text-muted">{label}</span>
                {control}
              </div>
              {i === 1 && <span className="pl-[88px] text-[11.5px] text-muted">Loudness −14 LUFS, applied to the file</span>}
            </div>
          ))}
        </div>
        <div className="flex items-center justify-between border-t border-white/[0.07] pt-4">
          <span className="text-[12px] text-muted">
            about <span className="font-semibold text-fg">12 MB</span>
          </span>
          <div className="flex gap-2.5">
            <span className="flex h-8 items-center rounded-full bg-white/[0.09] px-4 text-[13px] font-medium ring-1 ring-inset ring-white/[0.07]">Close</span>
            <span className="flex h-8 items-center rounded-full bg-accent-strong px-4 text-[13px] font-medium text-white">Export…</span>
          </div>
        </div>
      </div>
    </div>
  );
}

export function Showcase() {
  const [active, setActive] = useState(0);
  const [spot, setSpot] = useState(0);
  // `auto` moves between tabs and ends when the visitor picks one.
  const [auto, setAuto] = useState(true);
  // The pointer or keyboard focus is on a hotspot, so the one the visitor is reading stays open.
  const [held, setHeld] = useState(false);
  // The visitor took the cards on a phone in hand, so the tour stops and the cards lead.
  const [manual, setManual] = useState(false);
  const section = useRef<HTMLElement>(null);
  const cards = useRef<HTMLDivElement>(null);
  const playing = usePlaying(section, 0.35);
  const tab = tabs[active];

  useEffect(() => {
    // `?tour=captions` opens the tour on a tab, for links that point at one feature.
    const linked = tabs.findIndex((t) => t.id === new URLSearchParams(window.location.search).get("tour"));
    if (linked >= 0) {
      setActive(linked);
      setAuto(false);
    }
  }, []);

  // Walks through the hotspots of the open tab, then on to the next tab. A tab the visitor
  // picked keeps cycling its own hotspots.
  const running = !held && !manual && playing;
  useEffect(() => {
    if (!running) return;
    const id = window.setTimeout(() => {
      if (spot + 1 < tab.spots.length) setSpot(spot + 1);
      else {
        if (auto) setActive((a) => (a + 1) % tabs.length);
        setSpot(0);
      }
    }, SPOT_MS);
    return () => window.clearTimeout(id);
  }, [running, active, spot, auto, tab.spots.length]);

  const choose = (i: number) => {
    setAuto(false);
    setActive(i);
    setSpot(0);
    cards.current?.scrollTo({ left: 0 });
  };

  // The tab bar is one Tab stop; the arrow keys, Home and End pick a tab.
  const arrows = (e: React.KeyboardEvent) => {
    const to = { ArrowLeft: active - 1, ArrowRight: active + 1, Home: 0, End: tabs.length - 1 }[e.key];
    if (to === undefined) return;
    e.preventDefault();
    const i = (to + tabs.length) % tabs.length;
    choose(i);
    document.getElementById(`tab-${tabs[i].id}`)?.focus();
  };

  // The cards exist only on a phone, so a window that grows past them gets the desktop tour back.
  useEffect(() => {
    const desktop = window.matchMedia("(min-width: 48rem)");
    const reset = () => desktop.matches && setManual(false);
    desktop.addEventListener("change", reset);
    return () => desktop.removeEventListener("change", reset);
  }, []);

  // While the tour runs, the cards follow it; once the visitor swipes them, the open spot follows the cards.
  useEffect(() => {
    const row = cards.current;
    const card = row?.children[spot] as HTMLElement | undefined;
    if (!row || !card || manual) return;
    row.scrollTo({ left: card.offsetLeft - (row.children[0] as HTMLElement).offsetLeft });
  }, [spot, manual]);

  const follow = (row: HTMLDivElement) => {
    if (!manual) return;
    const all = [...row.children] as HTMLElement[];
    const off = (c: HTMLElement) => Math.abs(c.offsetLeft - all[0].offsetLeft - row.scrollLeft);
    setSpot(all.reduce((best, c, i) => (off(c) < off(all[best]) ? i : best), 0));
  };

  // A touch that moves up or down only scrolls the page, so only a sideways one takes the tour over.
  const touch = useRef({ x: 0, y: 0 });
  const swipe = (t: React.Touch) => {
    const dx = Math.abs(t.clientX - touch.current.x);
    if (dx > 10 && dx > Math.abs(t.clientY - touch.current.y)) setManual(true);
  };

  const current = tab.spots[spot];

  return (
    <section id="editor" ref={section} className="relative pb-10 pt-32 sm:pt-[180px]">
      <div className="relative z-10 mx-auto flex max-w-[1240px] flex-col items-center gap-7 px-5 text-center">
        <h2 className="heading-lg">
          Everything where you expect it.
          <br />
          <span className="text-subtle">From the first cut to the cover.</span>
        </h2>

        <div role="tablist" aria-label="Editor tour" onKeyDown={arrows} className="glass flex h-12 max-w-full items-center gap-0.5 overflow-x-auto rounded-full p-[5px] no-scrollbar">
          {tabs.map((t, i) => (
            <button
              key={t.id}
              role="tab"
              id={`tab-${t.id}`}
              aria-selected={i === active}
              aria-controls="editor-stage"
              tabIndex={i === active ? 0 : -1}
              onClick={() => choose(i)}
              className={`flex h-[38px] shrink-0 items-center gap-2 rounded-full px-3.5 text-[14px] font-medium transition-colors sm:px-[18px] ${
                i === active ? "bg-white/[0.16] text-fg" : "text-muted hover:text-fg"
              }`}
            >
              <t.icon className="hidden size-[15px] sm:block" />
              {t.label}
            </button>
          ))}
        </div>

        <div
          id="editor-stage"
          role="tabpanel"
          aria-labelledby={`tab-${tab.id}`}
          className="w-full"
        >
          <Fit
            width={1200}
            height={740}
            view={current.view}
            className="mx-auto max-md:overflow-hidden max-md:rounded-[18px] max-md:ring-1 max-md:ring-white/10"
          >
            <div className="relative size-full rounded-[14px] shadow-[0_50px_140px_rgb(0_0_0/0.9),0_0_0_1px_rgb(255_255_255/0.08)]">
              <EditorMock focus={tab.id} framed />
              {tab.id === "export" && <ExportDialog />}

              {tab.spots.map((s, i) => (
                <button
                  key={`${tab.id}-${i}`}
                  aria-label={s.title}
                  aria-describedby={`tip-${tab.id}-${i}`}
                  onPointerEnter={() => {
                    setSpot(i);
                    setHeld(true);
                  }}
                  onPointerLeave={() => setHeld(false)}
                  onFocus={() => {
                    setSpot(i);
                    setHeld(true);
                  }}
                  onBlur={() => setHeld(false)}
                  onClick={() => setSpot(i)}
                  className="group absolute hidden size-12 -translate-x-1/2 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full md:flex"
                  style={{ left: s.x, top: s.y }}
                >
                  {/* Every hotspot breathes so it reads as something to point at; the open one turns dark like its tooltip and stays still. */}
                  {i !== spot && (
                    <span
                      className="absolute inset-1 rounded-full bg-white/45 animate-[pulse-ring_2s_ease-out_infinite]"
                      style={{ animationDelay: `${i * 0.5}s` }}
                    />
                  )}
                  <span
                    className={`relative flex size-8 items-center justify-center rounded-full shadow-[0_0_0_4px_rgb(0_0_0/0.25),0_6px_18px_rgb(0_0_0/0.55)] transition-[background-color,transform] duration-150 group-hover:scale-110 ${
                      i === spot ? "bg-[#1c1c1f] text-fg ring-1 ring-inset ring-white/25" : "bg-fg text-bg"
                    }`}
                  >
                    <Plus className={`size-4 transition-transform duration-200 ${i === spot ? "rotate-45" : ""}`} strokeWidth={2.5} />
                  </span>
                </button>
              ))}

              {/* Each hotspot's tooltip is there for screen readers; only the open one shows. Solid, so the editor never shows through its text. */}
              {tab.spots.map((s, i) => (
                <div
                  key={`${tab.id}-${i}`}
                  id={`tip-${tab.id}-${i}`}
                  role="tooltip"
                  className={`pointer-events-none absolute w-[292px] flex-col gap-1.5 rounded-[20px] bg-[#232327] px-[18px] py-4 text-left shadow-[0_18px_50px_rgb(0_0_0/0.55)] ring-1 ring-white/10 animate-[fade-up_180ms_ease-out] ${i === spot ? "hidden md:flex" : "hidden"}`}
                  style={{ left: s.tip[0], top: s.tip[1] }}
                >
                  <span className="text-[15px] font-semibold">{s.title}</span>
                  <span className="text-[13px] leading-[1.45] text-fg/75">{s.text}</span>
                </div>
              ))}
            </div>
          </Fit>
        </div>

        <div className="flex w-full flex-col items-center gap-4 md:pt-3">
          <div
            key={`cards-${tab.id}`}
            ref={cards}
            tabIndex={0}
            role="region"
            aria-label={`${tab.label} cards`}
            onScroll={(e) => follow(e.currentTarget)}
            onTouchStart={(e) => (touch.current = { x: e.touches[0].clientX, y: e.touches[0].clientY })}
            onTouchMove={(e) => swipe(e.touches[0])}
            onWheel={(e) => Math.abs(e.deltaX) > Math.abs(e.deltaY) && setManual(true)}
            onKeyDown={(e) => (e.key === "ArrowLeft" || e.key === "ArrowRight") && setManual(true)}
            className="no-scrollbar relative -mx-5 flex snap-x snap-mandatory scroll-px-5 scroll-smooth gap-2.5 self-stretch overflow-x-auto px-5 after:w-[calc(14%-0.625rem)] after:shrink-0 after:content-[''] md:hidden"
          >
            {tab.spots.map((s, i) => (
              <div key={s.title} className="surface flex w-[86%] shrink-0 snap-start flex-col gap-1.5 rounded-[20px] px-[18px] py-4 text-left">
                <span className="flex items-baseline justify-between gap-3 text-[15px] font-semibold">
                  {s.title}
                  <span className="text-[12px] font-medium tabular-nums text-muted">
                    {i + 1} / {tab.spots.length}
                  </span>
                </span>
                <span className="min-h-[4.35em] text-[14px] leading-[1.45] text-fg/75">{s.text}</span>
              </div>
            ))}
          </div>
          <p key={tab.id} className="min-h-[1.5em] text-[17px] text-muted animate-[fade-up_240ms_ease-out] max-md:hidden">
            {tab.summary}
          </p>
          {/* The open tab's bar fills one step per hotspot. */}
          <div className="flex items-center gap-1.5" aria-hidden>
            {tabs.map((t, i) => (
              <span key={t.id} className={`relative h-1.5 overflow-hidden rounded-full bg-white/25 transition-[width] duration-300 ${i === active ? "w-7" : "w-1.5"}`}>
                {i === active && (
                  <span
                    key={`${active}-${spot}-${running}`}
                    className="absolute inset-0 origin-left rounded-full bg-fg"
                    style={
                      {
                        "--from": spot / tab.spots.length,
                        "--to": (spot + 1) / tab.spots.length,
                        transform: `scaleX(${(spot + 1) / tab.spots.length})`,
                        animation: running ? `progress ${SPOT_MS}ms linear forwards` : undefined,
                      } as React.CSSProperties
                    }
                  />
                )}
              </span>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}

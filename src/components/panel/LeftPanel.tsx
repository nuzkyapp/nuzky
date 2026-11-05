import { Blend, Captions, Film, Music, Palette, Puzzle, ScrollText, Type } from "lucide-react";
import { useCover } from "../../lib/cover";
import { useReels } from "../../lib/reels";
import { useEditor, type PanelTab } from "../../lib/store";
import { CoverFrames } from "../cover/CoverFrames";
import { ReelTrim } from "../reels/ReelTrim";
import { AiLock, SegThumb, TabPanel, tabIds, tabListKeys } from "../ui";
import { AudioTab } from "./AudioTab";
import { CaptionsTab } from "./CaptionsTab";
import { FiltersTab } from "./FiltersTab";
import { MediaTab } from "./MediaTab";
import { PluginsTab } from "./PluginsTab";
import { TextTab } from "./TextTab";
import { TranscriptTab } from "./TranscriptTab";
import { TransitionsTab } from "./TransitionsTab";

// CapCut's order.
const TABS: { id: PanelTab; label: string; icon: typeof Film }[] = [
  { id: "media", label: "Media", icon: Film },
  { id: "audio", label: "Audio", icon: Music },
  { id: "text", label: "Text", icon: Type },
  { id: "captions", label: "Captions", icon: Captions },
  { id: "transcript", label: "Transcript", icon: ScrollText },
  { id: "transitions", label: "Transitions", icon: Blend },
  { id: "filters", label: "Filters", icon: Palette },
  { id: "plugins", label: "Plugins", icon: Puzzle },
];

const TAB_IDS = TABS.map((t) => t.id);
/** Under this width eight labels in one row would read as one line, so they take two rows. */
const COMPACT_BELOW = 420;

export function LeftPanel({ width }: { width: number }) {
  const tab = useEditor((s) => s.panelTab);
  const setTab = (id: PanelTab) => useEditor.setState({ panelTab: id });
  // In the cover editor the library's place holds the frames for the cover; the tabs act on the video.
  const cover = useCover((s) => s.open);
  // Choosing reels, it holds the words of the reel shown, to trim.
  const reels = useReels((s) => s.open);
  // Narrower than eight labels need in one row, the tabs sit in two rows of four, each still with its name.
  const compact = width < COMPACT_BELOW;
  if (reels)
    return (
      <aside className="pane flex shrink-0 flex-col overflow-hidden" style={{ width }} aria-label="Reel">
        <AiLock>
          <ReelTrim />
        </AiLock>
      </aside>
    );
  if (cover)
    return (
      <aside className="pane flex shrink-0 flex-col overflow-hidden" style={{ width }} aria-label="Cover frames">
        <CoverFrames format={cover} />
      </aside>
    );
  return (
    <aside data-tour="library" className="pane flex shrink-0 flex-col overflow-hidden" style={{ width }}>
      {/* In one row the tabs size to their labels and share the leftover width; in a narrower library, two rows of four. */}
      <div className={`relative mx-1.5 mt-1.5 shrink-0 rounded-[13px] p-0.5 ${compact ? "mb-0.5 grid grid-cols-4 gap-x-1 gap-y-1" : "mb-0.5 flex"}`} role="tablist" aria-label="Library" onKeyDown={tabListKeys(TAB_IDS, tab, setTab)}>
        <SegThumb value={tab} className="rounded-[11px] bg-white/[.11]" />
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            id={tabIds("library", t.id).tab}
            data-tab={t.id}
            data-seg={t.id}
            aria-selected={tab === t.id}
            aria-controls={tab === t.id ? tabIds("library", t.id).panel : undefined}
            tabIndex={tab === t.id ? 0 : -1}
            onClick={() => setTab(t.id)}
            className={`relative flex flex-auto flex-col items-center rounded-[11px] px-px text-[11px] leading-[13px] font-medium ${compact ? "h-10 justify-center gap-[3px]" : "gap-[3px] pt-1.5 pb-[5px] tracking-[-0.02em]"} whitespace-nowrap transition-colors duration-[120ms] ease-out ${
              tab === t.id ? "text-fg" : "text-fg/70 hover:text-fg"
            }`}
          >
            <t.icon size={compact ? 16 : 17} className={`transition-colors duration-[120ms] ease-out ${tab === t.id ? "text-accent" : ""}`} />
            <span>{t.label}</span>
          </button>
        ))}
      </div>
      <TabPanel group="library" id={tab} className="flex min-h-0 flex-1 flex-col">
        <AiLock>
          {tab === "media" && <MediaTab />}
          {tab === "audio" && <AudioTab />}
          {tab === "text" && <TextTab />}
          {tab === "captions" && <CaptionsTab />}
          {tab === "transcript" && <TranscriptTab />}
          {tab === "transitions" && <TransitionsTab />}
          {tab === "filters" && <FiltersTab />}
          {tab === "plugins" && <PluginsTab />}
        </AiLock>
      </TabPanel>
    </aside>
  );
}

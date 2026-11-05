import { useEffect } from "react";
import { TEXT_PRESETS, canvasUnit, scaleStyle } from "../../lib/presets";
import { addSticker, loadCatalog, loadThumbs, thumbKey, useStickers, type TextTab as Tab } from "../../lib/stickers";
import { useEditor } from "../../lib/store";
import type { StickerItem } from "../../lib/types";
import { TabBar, TabPanel, TextSwatch, gridKeys, lockedProps, useLockReason } from "../ui";
import { EmojiPicker } from "./EmojiPicker";

const TABS: { id: Tab; label: string }[] = [
  { id: "text", label: "Text" },
  { id: "stickers", label: "Stickers" },
  { id: "emoji", label: "Emoji" },
];

/** Sticker tiles are about 72 px; twice that keeps them sharp on a HiDPI screen. */
const STICKER_PX = 144;

function TextStyles() {
  const edit = useEditor((s) => s.edit);
  const lock = useLockReason();
  return (
    <div className="scroll-shown grid grid-cols-[repeat(auto-fill,minmax(120px,1fr))] content-start gap-x-2 gap-y-3 overflow-y-auto px-3.5 pt-1 pb-6">
      {TEXT_PRESETS.map((p) => (
        <button
          key={p.name}
          type="button"
          title={`Add "${p.name}" text at the playhead`}
          onClick={() => edit((project) => ({ type: "addText", startUs: useEditor.getState().timeUs, text: p.text, style: scaleStyle(p.style, canvasUnit(project.canvas)) }))}
          {...lockedProps(lock)}
          className="group flex min-w-0 flex-col gap-1.5 text-left aria-disabled:cursor-not-allowed aria-disabled:opacity-40"
        >
          {/* The text each preset adds, drawn large in its style, with the preset's name under the tile. */}
          <span className="flex h-20 w-full items-center justify-center rounded-[10px] border border-white/[.08] bg-white/[.06] px-2 transition-colors duration-[120ms] ease-out group-hover:border-white/30 group-aria-disabled:group-hover:border-white/[.08]">
            <TextSwatch style={p.style} label={p.text} size={19} />
          </span>
          <span className="truncate text-[11px] text-muted">{p.name}</span>
        </button>
      ))}
    </div>
  );
}

function StickerTile({ item, first }: { item: StickerItem; first: boolean }) {
  const url = useStickers((s) => s.thumbs[thumbKey(item.key, STICKER_PX)]);
  const adding = useStickers((s) => s.adding.includes(item.key));
  const lock = useLockReason();
  return (
    <button
      type="button"
      data-sticker={item.key}
      tabIndex={first ? 0 : -1}
      aria-label={`Add ${item.name}`}
      title={`Add ${item.name} at the playhead`}
      onClick={() => addSticker(item.key)}
      {...lockedProps(lock)}
      aria-busy={adding}
      className="flex aspect-square min-w-0 items-center justify-center rounded-[10px] border border-white/[.08] bg-white/[.06] transition-colors duration-[120ms] ease-out hover:border-white/30 aria-busy:cursor-progress aria-busy:opacity-60 aria-disabled:cursor-not-allowed aria-disabled:opacity-40 aria-disabled:hover:border-white/[.08]"
    >
      {url && <img src={url} alt="" draggable={false} className="max-h-[62%] max-w-[72%]" />}
    </button>
  );
}

function Stickers() {
  const group = useStickers((s) => s.catalog?.groups.find((g) => g.items.every((i) => i.emoji === null)));
  useEffect(() => {
    loadCatalog();
  }, []);
  useEffect(() => {
    if (group) loadThumbs(group.items.map((i) => i.key), STICKER_PX);
  }, [group]);
  return (
    <div onKeyDown={gridKeys("[data-sticker]")} className="grid grid-cols-[repeat(auto-fill,minmax(72px,1fr))] content-start gap-2 overflow-y-auto px-3.5 pt-1 pb-3.5">
      {group?.items.map((item, i) => (
        <StickerTile key={item.key} item={item} first={i === 0} />
      ))}
    </div>
  );
}

export function TextTab() {
  const tab = useStickers((s) => s.tab);
  return (
    <div className="flex min-h-0 flex-1 flex-col pt-3">
      <TabBar group="text" label="Text" tabs={TABS} value={tab} onChange={(id) => useStickers.setState({ tab: id })} className="mx-3.5 mb-1" />
      <TabPanel group="text" id={tab} className="flex min-h-0 flex-1 flex-col pt-1.5">
        {tab === "text" && <TextStyles />}
        {tab === "stickers" && <Stickers />}
        {tab === "emoji" && <EmojiPicker />}
      </TabPanel>
    </div>
  );
}

import { useEffect } from "react";
import { TEXT_GROUPS, canvasUnit, scaleStyle } from "../../lib/presets";
import { addSticker, loadCatalog, loadThumbs, thumbKey, useStickers, type TextTab as Tab } from "../../lib/stickers";
import { api } from "../../lib/api";
import { aiLocked, endPreview, enqueue, previewHandlers, showPastEntry, useEditor } from "../../lib/store";
import type { EditCmd, Project, Snapshot, StickerItem, TextPreset } from "../../lib/types";
import { AnimatedSwatch, TabBar, TabPanel, TextSwatch, gridKeys, lockedProps, useLockReason } from "../ui";
import { EmojiPicker } from "./EmojiPicker";

const TABS: { id: Tab; label: string }[] = [
  { id: "text", label: "Text" },
  { id: "stickers", label: "Stickers" },
  { id: "emoji", label: "Emoji" },
];

/** Sticker tiles are about 72 px; twice that keeps them sharp on a HiDPI screen. */
const STICKER_PX = 144;

/** The edits a text style's tile makes: its text at the playhead, centred, with the style's animations. */
function addCmds(p: TextPreset, project: Project): EditCmd {
  return { type: "addText", startUs: useEditor.getState().timeUs, text: p.text, style: scaleStyle(p.style, canvasUnit(project.canvas)) };
}

let adds = 0;

/**
 * Adds the style's text and gives it the style's animations as one undo step, in one place in the edit queue, so a
 * second quick click never lands between the two. Each click has its own key, so two adds stay two steps.
 */
async function addStyled(p: TextPreset) {
  if (aiLocked()) return;
  const key = `text-style:${++adds}`;
  let added: Snapshot | null = null;
  const snap = await enqueue(async (epoch) => {
    const project = useEditor.getState().snap?.project;
    if (!project) return null;
    added = await api.applyEdits([addCmds(p, project)], key, epoch);
    const clipId = added.select[0];
    if (!clipId || (!p.animIn && !p.animOut)) return added;
    // Selects the new text now, as the animations' snapshot selects nothing.
    useEditor.getState().setSnap(added);
    return api.applyEdits(
      [
        { type: "setAnimation", clipId, slot: "in", animation: p.animIn ?? null },
        { type: "setAnimation", clipId, slot: "out", animation: p.animOut ?? null },
      ],
      key,
      epoch,
    );
  });
  const id = (added as Snapshot | null)?.select[0];
  const clip = snap?.project.tracks.flatMap((t) => t.clips).find((c) => c.id === id);
  if (clip) showPastEntry(clip.startUs, p.animIn);
}

function TextStyles() {
  const lock = useLockReason();
  useEffect(() => endPreview, []);
  return (
    <div onKeyDown={gridKeys("[data-text-preset]")} className="scroll-shown flex flex-col gap-4 overflow-y-auto px-3.5 pt-1 pb-6">
      {TEXT_GROUPS.map((g) => (
        <section key={g.name} aria-label={g.name} className="flex flex-col gap-2">
          <h3 className="text-[13px] font-semibold text-fg">{g.name}</h3>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(100px,1fr))] gap-x-2 gap-y-3">
            {g.presets.map((p) => (
              <button
                key={p.name}
                type="button"
                title={lock ?? `Add "${p.name}" text at the playhead`}
                data-text-preset={p.name}
                onClick={() => void addStyled(p)}
                {...previewHandlers(lock ? null : (project: Project) => addCmds(p, project))}
                {...lockedProps(lock)}
                className="group flex min-w-0 flex-col gap-1.5 text-left aria-disabled:cursor-not-allowed aria-disabled:opacity-40"
              >
                {/* The text each style adds, drawn large in it and playing its entry animation on hover, with the name under the tile. */}
                <span className="flex h-20 w-full items-center justify-center overflow-hidden rounded-[10px] border border-white/[.08] bg-white/[.06] px-2 transition-colors duration-[120ms] ease-out group-hover:border-white/30 group-aria-disabled:group-hover:border-white/[.08]">
                  <AnimatedSwatch animation={p.animIn?.kind}>
                    <TextSwatch style={p.style} label={p.text} size={19} />
                  </AnimatedSwatch>
                </span>
                <span className="truncate text-[11px] text-muted">{p.name}</span>
              </button>
            ))}
          </div>
        </section>
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

import { useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, Copy, Download, Image as ImageIcon, ImagePlus, Plus, RotateCcw, Trash2, Type } from "lucide-react";
import {
  COVER_PRESETS,
  MAX_OVERLAYS,
  addOverlay,
  addText,
  coverFormat,
  deleteOverlay,
  deleteText,
  downloadModels,
  editCover,
  editOverlay,
  editText,
  overlayPlace,
  presetText,
  selectOverlay,
  selectText,
  selectedOverlay,
  thumbnailOf,
  useCover,
} from "../../lib/cover";
import { DEFAULT_TRANSFORM, sameStyle } from "../../lib/presets";
import { aiLocked, patched, undoAction, useEditor, type StylePatch } from "../../lib/store";
import type { Thumbnail, ThumbnailFormat } from "../../lib/types";
import { InspectorHeader } from "../inspector/Header";
import { MainStyle, MoreStyleFold } from "../inspector/TextSection";
import { IMAGE_EXTENSIONS, importPaths } from "../panel/assets";
import { AiLock, Button, ColorInput, Disclosure, IconButton, Menu, PresetTile, Section, Segmented, Slider, Switch, TextSwatch, useLockReason } from "../ui";
import { COVERED_TOO_MUCH, useShownCover } from "./CoverStage";

const CANNOT_CUT_OUT = "Cutting out the person doesn't work on this computer";

/**
 * A setting that needs the person cut out of the frame. Without the cover models it offers their download, which
 * also turns it on: the cover shows it once the person is found.
 */
function PersonSwitch({ label, checked, onChange }: { label: string; checked: boolean; onChange: (on: boolean) => void }) {
  const models = useCover((s) => s.models);
  const download = !checked && !!models && !models.downloaded && !models.unavailable;
  return (
    <div className="flex min-h-6 items-center justify-between gap-2">
      <span className="text-[13px] text-fg">{label}</span>
      {download ? (
        <Button
          pill
          className="h-6 shrink-0 gap-1 px-2.5 text-[12px]"
          title={`${label} needs the cover models, downloaded once`}
          onClick={() => {
            onChange(true);
            void downloadModels();
          }}
        >
          <Download size={13} /> Download {models?.sizeMb} MB
        </Button>
      ) : (
        <Switch label={label} checked={checked} onChange={onChange} disabledReason={!checked && models?.unavailable ? CANNOT_CUT_OUT : undefined} />
      )}
    </div>
  );
}

/** The inspector in the cover editor: the selected text or image, or the cover's own settings. */
export function CoverInspector({ format }: { format: ThumbnailFormat }) {
  const selected = useCover((s) => s.selected);
  const shown = useShownCover(format);
  if (!shown) return null;
  const text = typeof selected === "number" ? shown.cover.texts[selected] : undefined;
  const overlay = selectedOverlay(selected);
  return (
    <AiLock>
      {text && !shown.draft && typeof selected === "number" ? (
        <TextInspector format={format} index={selected} />
      ) : overlay !== null && shown.cover.overlays?.[overlay] && !shown.draft ? (
        <OverlayInspector format={format} index={overlay} />
      ) : (
        <CoverSettings format={format} cover={shown.cover} draft={shown.draft} />
      )}
    </AiLock>
  );
}

async function importOverlay(format: ThumbnailFormat) {
  if (aiLocked()) return;
  const picked = await open({ multiple: false, filters: [{ name: "Images", extensions: IMAGE_EXTENSIONS }] });
  if (typeof picked !== "string") return;
  const [assetId] = await importPaths([picked]);
  if (assetId) await addOverlay(format, assetId);
}

/** Add image: Import image… and the project's images in a menu, or straight to the file dialog without any. */
function AddImage({ format, count }: { format: ThumbnailFormat; count: number }) {
  const assets = useEditor((s) => s.snap?.project.assets);
  const images = assets?.filter((a) => a.kind === "image") ?? [];
  const [menu, setMenu] = useState<{ x: number; y: number; keyboard: boolean } | null>(null);
  const button = useRef<HTMLButtonElement | null>(null);
  return (
    <>
      <Button
        className="w-full"
        disabled={count >= MAX_OVERLAYS}
        disabledReason={`A cover takes up to ${MAX_OVERLAYS} images`}
        aria-haspopup={images.length > 0 ? "menu" : undefined}
        aria-expanded={images.length > 0 ? !!menu : undefined}
        onClick={(e) => {
          if (images.length === 0) return void importOverlay(format);
          button.current = e.currentTarget;
          const r = e.currentTarget.getBoundingClientRect();
          setMenu({ x: r.right, y: r.bottom + 4, keyboard: e.detail === 0 });
        }}
      >
        <ImagePlus size={15} /> Add image
      </Button>
      {menu && (
        <Menu
          label="Add image"
          at={{ x: menu.x, y: menu.y, align: "end" }}
          keyboard={menu.keyboard}
          onClose={() => {
            setMenu(null);
            requestAnimationFrame(() => button.current?.focus());
          }}
          // Import image… first, so it stays in reach however many images the project has.
          items={[{ label: "Import image…", run: () => void importOverlay(format) }, "separator", ...images.map((a) => ({ label: a.name, run: () => void addOverlay(format, a.id) }))]}
        />
      )}
    </>
  );
}

/** What is behind the person: the frame as filmed or blurred, or a colour. Blurred and Color need the person cut out. */
type Look = "filmed" | "blurred" | "color";

function CoverSettings({ format, cover, draft }: { format: ThumbnailFormat; cover: Thumbnail; draft: boolean }) {
  const f = coverFormat(format);
  const hidden = useCoverHidden();
  const models = useCover((s) => s.models);
  // Clicking the picture on the cover selects it, which opens its zoom and position.
  const frameSelected = useCover((s) => s.selected === "frame");
  const [zoom, setZoom] = useState(false);
  const title = format === "cover_9x16" ? "Cover" : "YouTube thumbnail";
  // A change of the draft makes the cover from it.
  const set = (patch: Partial<Thumbnail>, key: string) => editCover(format, (c) => ({ ...(c ?? cover), ...patch }), key);
  const frame = (patch: Partial<Thumbnail["frame"]>, key: string) => editCover(format, (c) => ({ ...(c ?? cover), frame: { ...(c ?? cover).frame, ...patch } }), key);
  const background = (patch: Partial<Thumbnail["background"]>, key: string) =>
    editCover(format, (c) => ({ ...(c ?? cover), background: { ...(c ?? cover).background, ...patch } }), key);
  const removeCover = async () => {
    const snap = await useEditor.getState().edit({ type: "removeThumbnail", format });
    if (snap) useEditor.getState().toast({ kind: "info", text: `${title} removed`, action: undoAction(snap) });
  };
  const b = cover.background;
  const look: Look = !b.picture ? "color" : b.blur > 0 || b.dim > 0 ? "blurred" : "filmed";
  const missing = !!models && !models.downloaded && !models.unavailable;
  const chooseLook = (id: Look) => {
    if (id === look) return;
    void background(id === "filmed" ? { picture: true, blur: 0, dim: 0 } : id === "blurred" ? { picture: true, blur: 1, dim: 0.2 } : { picture: false }, "look");
    if (id !== "filmed" && missing) void downloadModels();
  };
  const zoomOpen = zoom || frameSelected;

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-testid="cover-inspector">
      <InspectorHeader icon={ImageIcon} title={title} detail={`${f.width}×${f.height}`} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <Section>
          <div className="grid grid-cols-2 gap-2">
            <Button className="w-full" onClick={() => void addText(format)}>
              <Plus size={15} /> Add text
            </Button>
            <AddImage format={format} count={cover.overlays?.length ?? 0} />
          </div>
          {cover.texts.length > 0 && (
            <ul className="-mx-2 flex flex-col" aria-label="Texts on the cover">
              {cover.texts.map((t, i) => (
                <li key={i}>
                  <button
                    type="button"
                    onClick={() => void selectText(format, i)}
                    className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] text-fg hover:bg-white/[.08]"
                    title="Select this text"
                  >
                    <Type size={14} className="shrink-0 text-muted" />
                    <span className="min-w-0 flex-1 truncate">{t.text.replace(/\s+/g, " ") || "Empty text"}</span>
                    {t.behind && <span className="shrink-0 text-[11px] text-muted">Behind person</span>}
                    {(hidden[i] ?? 0) > COVERED_TOO_MUCH && <AlertTriangle size={14} className="shrink-0 text-warn" aria-label="The person covers too much of it" />}
                  </button>
                </li>
              ))}
            </ul>
          )}
          {!!cover.overlays?.length && <OverlayList format={format} cover={cover} />}
        </Section>
        <Section title="Background">
          <Segmented
            label="Background"
            value={look}
            disabled={look === "filmed" && !!models?.unavailable}
            disabledReason={CANNOT_CUT_OUT}
            onChange={chooseLook}
            options={[
              { id: "filmed", label: "As filmed" },
              { id: "blurred", label: "Blurred" },
              { id: "color", label: "Color" },
            ]}
          />
          {look === "filmed" && missing && <p className="text-[12px] text-muted">Blurred and Color download the cover models first ({models?.sizeMb} MB, once).</p>}
          {look === "blurred" && (
            <>
              {/* Never 0, so dragging it down does not turn the background back to as filmed under the pointer. */}
              <Slider label="Blur" value={b.blur * 100} min={1} max={100} step={1} unit="%" format={(v) => String(Math.round(v))} onChange={(v) => void background({ blur: v / 100 }, "blur")} />
              <Slider label="Dim" value={b.dim * 100} min={0} max={100} step={1} unit="%" format={(v) => String(Math.round(v))} onChange={(v) => void background({ dim: v / 100 }, "dim")} />
            </>
          )}
          {look === "color" && <ColorInput label="Color" value={b.color} onChange={(color) => void background({ color }, "color")} />}
          <PersonSwitch label="Line around person" checked={!!cover.outline} onChange={(on) => void set({ outline: on ? { color: "#ffffff", width: format === "youtube_16x9" ? 10 : 14 } : null }, "outline")} />
          {cover.outline && (
            <>
              <ColorInput label="Line color" value={cover.outline.color} onChange={(color) => void set({ outline: { ...cover.outline!, color } }, "outlineColor")} />
              <Slider label="Line width" value={cover.outline.width} min={1} max={40} step={1} unit="px" format={(v) => String(Math.round(v))} onChange={(width) => void set({ outline: { ...cover.outline!, width } }, "outlineWidth")} />
            </>
          )}
        </Section>
        <Disclosure
          title="Picture"
          summary="Zoom, position, rotation"
          open={zoomOpen}
          onToggle={() => {
            setZoom(!zoomOpen);
            if (zoomOpen && frameSelected) useCover.setState({ selected: null });
          }}
        >
          <Slider label="Zoom" value={cover.frame.scale * 100} min={10} max={400} step={1} unit="%" format={(v) => String(Math.round(v))} onChange={(v) => void frame({ scale: v / 100 }, "scale")} />
          <Slider label="Position X" value={cover.frame.x * 100} min={-100} max={100} step={0.5} unit="%" format={(v) => v.toFixed(1)} onChange={(v) => void frame({ x: v / 100 }, "x")} />
          <Slider label="Position Y" value={cover.frame.y * 100} min={-100} max={100} step={0.5} unit="%" format={(v) => v.toFixed(1)} onChange={(v) => void frame({ y: v / 100 }, "y")} />
          <Slider label="Rotation" value={cover.frame.rotation} min={-180} max={180} step={1} unit="°" format={(v) => String(Math.round(v))} onChange={(v) => void frame({ rotation: v }, "rot")} />
          <Button variant="ghost" className="h-7 self-start px-2 text-[12px]" onClick={() => void frame(DEFAULT_TRANSFORM, "reset")}>
            <RotateCcw size={13} /> Reset
          </Button>
        </Disclosure>
        {!draft && (
          <div className="px-4 py-3">
            <Button variant="ghost" className="h-7 px-2 text-[12px]" onClick={() => void removeCover()}>
              <Trash2 size={13} /> Remove {format === "cover_9x16" ? "cover" : "thumbnail"}
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

function OverlayList({ format, cover }: { format: ThumbnailFormat; cover: Thumbnail }) {
  const assets = useEditor((s) => s.snap?.project.assets);
  return (
    <ul className="-mx-2 flex flex-col" aria-label="Images on the cover">
      {cover.overlays!.map((o, i) => {
        const asset = assets?.find((a) => a.id === o.assetId);
        return (
          <li key={i}>
            <button
              type="button"
              onClick={() => void selectOverlay(format, i)}
              className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] text-fg hover:bg-white/[.08]"
              title="Select this image"
            >
              <ImageIcon size={14} className="shrink-0 text-muted" />
              <span className="min-w-0 flex-1 truncate">{asset?.name ?? "Missing image"}</span>
            </button>
          </li>
        );
      })}
    </ul>
  );
}

/** A selected image on the cover: its size, place, rotation and opacity, and Reset to its corner. */
function OverlayInspector({ format, index }: { format: ThumbnailFormat; index: number }) {
  const overlay = useEditor((s) => thumbnailOf(s.snap?.project, format)?.overlays?.[index]);
  const asset = useEditor((s) => s.snap?.project.assets.find((a) => a.id === overlay?.assetId));
  if (!overlay) return null;
  const set = (patch: Partial<Thumbnail["frame"]>, key: string) => void editOverlay(format, index, (o) => ({ ...o, transform: { ...o.transform, ...patch } }), `transform:${key}`);
  const t = overlay.transform;
  return (
    <div className="flex min-h-0 flex-1 flex-col" data-testid="cover-image-inspector">
      <div className="flex items-start justify-between pr-2">
        <InspectorHeader icon={ImageIcon} title="Image" detail={asset?.name ?? "Missing image"} />
        <div className="flex items-center gap-0.5 pt-3">
          <IconButton label="Delete image (Delete)" onClick={() => void deleteOverlay(format, index)}>
            <Trash2 size={15} />
          </IconButton>
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <Section>
          {/* 100 % is as large as fits on the cover; dragging a corner goes no smaller than 10 % either. */}
          <Slider label="Size" value={t.scale * 100} min={10} max={200} step={1} unit="%" format={(v) => String(Math.round(v))} onChange={(v) => set({ scale: v / 100 }, "scale")} />
          <Slider label="Position X" value={t.x * 100} min={-100} max={100} step={0.5} unit="%" format={(v) => v.toFixed(1)} onChange={(v) => set({ x: v / 100 }, "x")} />
          <Slider label="Position Y" value={t.y * 100} min={-100} max={100} step={0.5} unit="%" format={(v) => v.toFixed(1)} onChange={(v) => set({ y: v / 100 }, "y")} />
          <Slider label="Rotation" value={t.rotation} min={-180} max={180} step={1} unit="°" format={(v) => String(Math.round(v))} onChange={(v) => set({ rotation: v }, "rot")} />
          <Slider label="Opacity" value={t.opacity * 100} min={0} max={100} step={1} unit="%" format={(v) => String(Math.round(v))} onChange={(v) => set({ opacity: v / 100 }, "opacity")} />
          {asset && (
            <Button variant="ghost" className="h-7 self-start px-2 text-[12px]" onClick={() => set(overlayPlace(format, asset), "reset")}>
              <RotateCcw size={13} /> Reset
            </Button>
          )}
        </Section>
      </div>
    </div>
  );
}

/** The share of each text of the open cover the person covers, from the cover on screen. */
function useCoverHidden(): number[] {
  return useCover((s) => s.hidden);
}

function TextInspector({ format, index }: { format: ThumbnailFormat; index: number }) {
  const [position, setPosition] = useState(false);
  const text = useEditor((s) => thumbnailOf(s.snap?.project, format)?.texts[index]);
  const hidden = useCoverHidden()[index] ?? 0;
  if (!text) return null;
  const setStyle = (patch: StylePatch, key: string) => void editText(format, index, (t) => ({ ...t, style: patched(t.style, patch) }), `style:${key}`);
  const setTransform = (patch: Partial<Thumbnail["frame"]>, key: string) => void editText(format, index, (t) => ({ ...t, transform: { ...t.transform, ...patch } }), `transform:${key}`);
  const remove = () => deleteText(format, index);
  const duplicate = async () => {
    const snap = await editCover(format, (c) => (c ? { ...c, texts: [...c.texts, { ...c.texts[index], transform: { ...c.texts[index].transform, y: c.texts[index].transform.y + 0.06 } }] } : null));
    const count = thumbnailOf(snap?.project, format)?.texts.length ?? 0;
    if (snap) useCover.setState({ selected: count - 1 });
  };
  const t = text.transform;
  return (
    <div className="flex min-h-0 flex-1 flex-col" data-testid="cover-text-inspector">
      <div className="flex items-start justify-between pr-2">
        <InspectorHeader icon={Type} title="Text" detail={`On the ${format === "cover_9x16" ? "9:16 cover" : "YouTube thumbnail"}`} />
        <div className="flex items-center gap-0.5 pt-3">
          <IconButton label="Duplicate text" onClick={() => void duplicate()}>
            <Copy size={15} />
          </IconButton>
          <IconButton label="Delete text (Delete)" onClick={() => void remove()}>
            <Trash2 size={15} />
          </IconButton>
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <Section>
          <CoverTextField format={format} index={index} text={text.text} />
          <div className="grid grid-cols-3 gap-2">
            {COVER_PRESETS.map((p) => {
              const look = presetText(format, p).style;
              return (
                <PresetTile
                  key={p.name}
                  label={p.name}
                  selected={sameStyle(look, text.style)}
                  title={`Apply the ${p.name} look`}
                  // A look keeps the text's font, case and where it wraps, as text presets do.
                  onClick={() => void editText(format, index, (x) => ({ ...x, style: { ...look, fontFamily: x.style.fontFamily, letterCase: look.letterCase ?? x.style.letterCase, maxWidth: x.style.maxWidth } }))}
                >
                  <PresetLook preset={p} />
                </PresetTile>
              );
            })}
          </div>
          <PersonSwitch label="Behind person" checked={text.behind} onChange={(behind) => void editText(format, index, (x) => ({ ...x, behind }), "behind")} />
          {text.behind && hidden > COVERED_TOO_MUCH && (
            <p className="flex items-start gap-1.5 text-[12px] text-fg" role="status">
              <AlertTriangle size={14} className="mt-px shrink-0 text-warn" />
              The person covers {Math.round(hidden * 100)} % of this text. Move it up or make it smaller.
            </p>
          )}
        </Section>
        <Section>
          <MainStyle style={text.style} setStyle={setStyle} maxSize={format === "cover_9x16" ? 400 : 300} />
        </Section>
        <MoreStyleFold style={text.style} setStyle={setStyle} />
        <Disclosure title="Position" summary="Scale, rotation, opacity" open={position} onToggle={() => setPosition(!position)}>
          <Slider label="Scale" value={t.scale * 100} min={10} max={400} step={1} unit="%" format={(v) => String(Math.round(v))} onChange={(v) => setTransform({ scale: v / 100 }, "scale")} />
          <Slider label="Position X" value={t.x * 100} min={-100} max={100} step={0.5} unit="%" format={(v) => v.toFixed(1)} onChange={(v) => setTransform({ x: v / 100 }, "x")} />
          <Slider label="Position Y" value={t.y * 100} min={-100} max={100} step={0.5} unit="%" format={(v) => v.toFixed(1)} onChange={(v) => setTransform({ y: v / 100 }, "y")} />
          <Slider label="Rotation" value={t.rotation} min={-180} max={180} step={1} unit="°" format={(v) => String(Math.round(v))} onChange={(v) => setTransform({ rotation: v }, "rot")} />
          <Slider label="Opacity" value={t.opacity * 100} min={0} max={100} step={1} unit="%" format={(v) => String(Math.round(v))} onChange={(v) => setTransform({ opacity: v / 100 }, "opacity")} />
          <Button variant="ghost" className="h-7 self-start px-2 text-[12px]" onClick={() => setTransform(DEFAULT_TRANSFORM, "reset")}>
            <RotateCcw size={13} /> Reset
          </Button>
        </Disclosure>
      </div>
    </div>
  );
}

/**
 * The text of a cover text. While it has focus the field shows what was typed, not the confirmed project,
 * which arrives a moment later and would move the caret.
 */
function CoverTextField({ format, index, text }: { format: ThumbnailFormat; index: number; text: string }) {
  const [draft, setDraft] = useState<string | null>(null);
  const lock = useLockReason();
  return (
    <textarea
      aria-label="Text"
      data-cover-text
      disabled={!!lock}
      title={lock ?? undefined}
      value={draft ?? text}
      rows={2}
      onChange={(e) => {
        setDraft(e.target.value);
        void editText(format, index, (t) => ({ ...t, text: e.target.value }), "text");
      }}
      onBlur={() => setDraft(null)}
      className="resize-y rounded-lg border border-white/[.08] bg-white/[.055] px-2.5 py-2 text-[13px] leading-[18px] text-fg outline-offset-0 focus:border-accent disabled:opacity-40"
    />
  );
}

/** A cover look's tile: its "Aa" in the look, as large as the look is, over a grey its outline shows on. */
function PresetLook({ preset }: { preset: (typeof COVER_PRESETS)[number] }) {
  return (
    <span className="absolute inset-0 flex items-center justify-center overflow-hidden bg-[#5c5c63]">
      <TextSwatch style={preset.style} label="Aa" size={Math.round(12 + preset.style.fontSize / 22)} />
    </span>
  );
}

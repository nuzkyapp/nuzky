import { useState } from "react";
import { create } from "zustand";
import { AlignCenter, AlignLeft, AlignRight, Ban, RotateCcw } from "lucide-react";
import { LIMITS } from "../../lib/limits";
import { BOX_RADIUS, CAPTION_STYLES, KEYWORD_COLOR, KEYWORD_PICKS, TEXT_PRESETS, isLook, sameStyle } from "../../lib/presets";
import { applyCaptionLook, editClip, isCaptionTrack, patched, placeCaptions, presetOver, restyleCaptions, useEditor, type StylePatch } from "../../lib/store";
import type { Animation, CaptionPreset, Clip, EditCmd, LetterCase, TextAlign, TextShadow, TextStyle, Transform } from "../../lib/types";
import { FontPicker } from "../FontPicker";
import { Button, Checkbox, ColorInput, Disclosure, LockReason, PresetTile, Section, Segmented, Slider, TextSwatch, parseNumber, useLockReason } from "../ui";

/** Which text folds are open and whether a caption's inspector styles every caption, for this session, wherever they show. */
export const useStyleUi = create<{ more: boolean; position: boolean; allCaptions: boolean }>(() => ({ more: false, position: false, allCaptions: true }));

/** A label above a full-width control, like the sliders' in the inspector. */
export function FieldRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="truncate text-[12px] leading-4 text-muted">{label}</span>
      {/* A row, so a control that fills it with flex-1 keeps its own height. */}
      <div className="flex">{children}</div>
    </div>
  );
}

/**
 * The clip's text. While it has focus the field shows what was typed, not the confirmed project,
 * which arrives a moment later and would move the caret or drop keys typed in between.
 */
function TextField({ clipId, text }: { clipId: string; text: string }) {
  const edit = useEditor((s) => s.edit);
  const [draft, setDraft] = useState<string | null>(null);
  const lock = useLockReason();
  return (
    <textarea
      aria-label="Text"
      disabled={!!lock}
      title={lock ?? undefined}
      value={draft ?? text}
      rows={3}
      onChange={(e) => {
        setDraft(e.target.value);
        edit({ type: "updateClip", clipId, text: e.target.value }, `${clipId}:text`);
      }}
      onBlur={() => setDraft(null)}
      className="resize-y rounded-lg border border-white/[.08] bg-white/[.055] px-2.5 py-2 text-[13px] leading-[18px] text-fg outline-offset-0 focus:border-accent disabled:opacity-40"
    />
  );
}

/**
 * Why no word of a karaoke caption lights up, or null when they do: the engine highlights words only
 * while the text is exactly them joined by single spaces.
 */
function unlitReason(clip: Clip, text: string) {
  const words = clip.content.type === "text" ? (clip.content.words ?? []) : [];
  if (words.length === 0) return "This caption has no word timing. Regenerate captions to add it.";
  if (words.map((w) => w.text).join(" ") !== text) return "Words were added or removed, so none lights up. Regenerate captions to bring it back.";
  return null;
}

const SCOPES = [
  { id: "all", label: "All captions", title: "Style changes go to every caption" },
  { id: "this", label: "This caption", title: "Style changes go to this caption only" },
] as const;

/** Where a caption's style changes go. It only changes what the controls below edit, so it works while the AI edits. */
function ApplyTo() {
  const all = useStyleUi((s) => s.allCaptions);
  return (
    <FieldRow label="Apply to">
      <div className="min-w-0 flex-1">
        <LockReason.Provider value={null}>
          <Segmented label="Apply to" value={all ? "all" : "this"} onChange={(id) => useStyleUi.setState({ allCaptions: id === "all" })} options={[...SCOPES]} />
        </LockReason.Provider>
      </div>
    </FieldRow>
  );
}

const textStyles = (clips: Clip[]) => clips.flatMap((c) => (c.content.type === "text" ? [c.content.style] : []));

/**
 * Style tiles use the presets and names of the Text tab, or of the Captions tab for a caption. A caption's
 * style goes to every caption unless Apply to says This caption; its text is always its own.
 */
export function TextSection({ clip, text, style, caption }: { clip: Clip; text: string; style: TextStyle; caption: boolean }) {
  const edit = useEditor((s) => s.edit);
  const all = useStyleUi((s) => s.allCaptions) && caption;
  const captions = useEditor((s) => (all ? s.snap?.project.tracks.find(isCaptionTrack)?.clips : undefined));
  const everyCaption = all && captions ? captions : null;
  // Built from the latest confirmed style, so two quick changes to different properties both stick.
  const setStyle: SetStyle = everyCaption
    ? (patch, key) => restyleCaptions(patch, `captions:style:${key}`)
    : (patch, key) =>
        editClip(clip.id, (c) => (c.content.type === "text" ? { type: "updateClip", clipId: c.id, style: patched(c.content.style, patch) } : null), `${clip.id}:style:${key}`);
  const styles = everyCaption ? textStyles(everyCaption) : undefined;
  const presets = caption ? CAPTION_STYLES : TEXT_PRESETS;
  const selected = (p: (typeof presets)[number]) => {
    if (!caption) return sameStyle(p.style, style);
    return (everyCaption ?? [clip]).every((c) => c.content.type === "text" && isLook(p as CaptionPreset, c.content.style, c.animIn, c.animOut));
  };
  const apply = (p: (typeof presets)[number]) => {
    const look = p as CaptionPreset;
    if (everyCaption) return void applyCaptionLook({ style: look.style, animIn: look.animIn ?? null, animOut: look.animOut ?? null });
    const restyle: EditCmd = { type: "updateClip", clipId: clip.id, style: presetOver(p.style, style) };
    // A caption style brings its animations too.
    const animate = (slot: "in" | "out", animation: Animation | null | undefined): EditCmd => ({ type: "setAnimation", clipId: clip.id, slot, animation: animation ?? null });
    edit(caption ? [restyle, animate("in", look.animIn), animate("out", look.animOut)] : restyle);
  };
  return (
    <>
      <Section title="Text">
        <TextField clipId={clip.id} text={text} />
        {caption && <ApplyTo />}
        <div className="grid grid-cols-3 gap-2">
          {presets.map((p) => (
            <PresetTile key={p.name} label={p.name} selected={selected(p)} title={everyCaption ? `Apply ${p.name} to all captions` : `Apply ${p.name} style`} onClick={() => apply(p)}>
              <span className="absolute inset-0 flex items-center justify-center bg-line">
                <TextSwatch style={{ ...p.style, fontFamily: p.style.fontFamily ?? style.fontFamily }} label="Aa" />
              </span>
            </PresetTile>
          ))}
        </div>
      </Section>
      <Section title="Style">
        <MainStyle style={style} styles={styles} setStyle={setStyle} />
      </Section>
      {caption && (
        <Section title="Karaoke">
          <HighlightRows style={style} styles={styles} setStyle={setStyle} />
          {style.highlight && unlitReason(clip, text) && <p className="text-[12px] text-muted">{unlitReason(clip, text)}</p>}
        </Section>
      )}
      {caption && (
        <Section title="Key words">
          <KeywordRows style={style} styles={styles} setStyle={setStyle} />
          {style.keywords && unlitReason(clip, text) && <p className="text-[12px] text-muted">{unlitReason(clip, text)}</p>}
        </Section>
      )}
      <MoreStyleFold style={style} styles={styles} setStyle={setStyle} />
      {everyCaption && <CaptionPosition clips={everyCaption} />}
    </>
  );
}

export type SetStyle = (patch: StylePatch, key: string) => void;

/** `styles`: every text the controls change, `style` being the one they show; a dash shows where they differ. */
type StyleProps = { style: TextStyle; styles?: TextStyle[]; setStyle: SetStyle };

function across(style: TextStyle, styles: TextStyle[] = [style]) {
  const json = (v: unknown) => JSON.stringify(v ?? null);
  return {
    differ: (get: (s: TextStyle) => unknown) => styles.some((s) => json(get(s)) !== json(get(style))),
    all: (has: (s: TextStyle) => boolean) => styles.every(has),
    on: (has: (s: TextStyle) => boolean) => ({ checked: styles.every(has), mixed: styles.some(has) && !styles.every(has) }),
  };
}

/** What a text needs most, then the rest folded under More style. */
export function StyleSection({ style, styles, setStyle, maxSize = 300 }: StyleProps & { maxSize?: number }) {
  return (
    <>
      <Section title="Style">
        <MainStyle style={style} styles={styles} setStyle={setStyle} maxSize={maxSize} />
      </Section>
      <MoreStyleFold style={style} styles={styles} setStyle={setStyle} />
    </>
  );
}

const CASES: { id: LetterCase; label: React.ReactNode; title: string }[] = [
  {
    id: "none",
    label: (
      <span className="flex items-center justify-center">
        <Ban size={13} aria-hidden />
        <span className="sr-only">As typed</span>
      </span>
    ),
    title: "As typed",
  },
  { id: "upper", label: "AB", title: "All caps" },
  { id: "lower", label: "ab", title: "All small letters" },
  { id: "title", label: "Ab", title: "Each word capitalised" },
];

/** Font, size, colour and case: what a text needs most. */
export function MainStyle({ style, styles, setStyle, maxSize = 300 }: StyleProps & { maxSize?: number }) {
  const { differ } = across(style, styles);
  return (
    <>
      <FieldRow label="Font">
        <FontPicker value={style.fontFamily} mixed={differ((s) => s.fontFamily)} onChange={(fontFamily) => setStyle({ fontFamily }, "font")} />
      </FieldRow>
      <Slider label="Size" mixed={differ((s) => s.fontSize)} value={style.fontSize} min={12} max={maxSize} step={1} format={(v) => String(Math.round(v))} onChange={(v) => setStyle({ fontSize: v }, "size")} />
      <ColorInput label="Color" mixed={differ((s) => s.color)} value={style.color} onChange={(v) => setStyle({ color: v }, "color")} />
      <FieldRow label="Case">
        <div className="min-w-0 flex-1">
          <Segmented label="Case" value={differ((s) => s.letterCase ?? "none") ? null : (style.letterCase ?? "none")} onChange={(letterCase) => setStyle({ letterCase }, "case")} options={CASES} />
        </div>
      </FieldRow>
    </>
  );
}

/** A caption's karaoke: the word being said in its own colour. */
export function HighlightRows({ style, styles, setStyle }: StyleProps) {
  const { differ, all, on } = across(style, styles);
  return (
    <>
      <Checkbox label="Highlight spoken word" {...on((s) => !!s.highlight)} onChange={(v) => setStyle((s) => ({ highlight: v ? (s.highlight ?? "#ffe14d") : null }), "highlight")} />
      {all((s) => !!s.highlight) && <ColorInput label="Highlight color" mixed={differ((s) => s.highlight)} value={style.highlight ?? "#ffe14d"} onChange={(highlight) => setStyle({ highlight }, "highlightColor")} />}
    </>
  );
}

/** Which words of a caption are key, and their colour. */
export function KeywordRows({ style, styles, setStyle }: StyleProps) {
  const { differ, all } = across(style, styles);
  const pick = differ((s) => s.keywords?.pick ?? "off") ? null : (style.keywords?.pick ?? "off");
  return (
    <>
      <Segmented
        label="Key words"
        value={pick}
        onChange={(id) => setStyle((s) => ({ keywords: id === "off" ? null : { color: s.keywords?.color ?? KEYWORD_COLOR, pick: id } }), "keywords")}
        options={KEYWORD_PICKS}
      />
      {all((s) => !!s.keywords) && (
        <ColorInput label="Key word color" mixed={differ((s) => s.keywords?.color)} value={style.keywords?.color ?? KEYWORD_COLOR} onChange={(color) => setStyle((s) => (s.keywords ? { keywords: { ...s.keywords, color } } : {}), "keywordColor")} />
      )}
    </>
  );
}

/** More style, folded until someone opens it, and open wherever it shows once they have. */
export function MoreStyleFold({ flush, ...props }: StyleProps & { flush?: boolean }) {
  const open = useStyleUi((s) => s.more);
  return (
    <Disclosure title="More style" summary="Bold, spacing, alignment, outline, box, shadow" open={open} onToggle={() => useStyleUi.setState({ more: !open })} flush={flush}>
      <MoreStyle {...props} />
    </Disclosure>
  );
}

const ALIGNS: { id: TextAlign; label: React.ReactNode; title: string }[] = [
  { id: "left", label: <AlignIcon icon={AlignLeft} name="Left" />, title: "Align left" },
  { id: "center", label: <AlignIcon icon={AlignCenter} name="Center" />, title: "Center" },
  { id: "right", label: <AlignIcon icon={AlignRight} name="Right" />, title: "Align right" },
];

function AlignIcon({ icon: Icon, name }: { icon: typeof AlignLeft; name: string }) {
  return (
    <span className="flex items-center justify-center">
      <Icon size={14} aria-hidden />
      <span className="sr-only">{name}</span>
    </span>
  );
}

/** CapCut's own shadow, as it starts when turned on. */
const SHADOW: TextShadow = { color: "#000000", opacity: 0.9, blur: 15, distance: 5, angle: -45 };
/** A box as it starts when turned on: black at 70 %. */
const BOX = "#000000b3";

const percent = (v: number) => String(Math.round(v));
const degrees = (s: string) => ((((parseNumber(s) + 180) % 360) + 360) % 360) - 180;
/** The opacity of a `#rrggbbaa` box, 1 for `#rrggbb`. */
const boxOpacity = (box: string) => (box.length >= 9 ? parseInt(box.slice(7, 9), 16) / 255 : 1);
const withOpacity = (box: string, opacity: number) => box.slice(0, 7) + Math.round(opacity * 255).toString(16).padStart(2, "0");

export function MoreStyle({ style, styles, setStyle }: StyleProps) {
  const { differ, all, on } = across(style, styles);
  const shadow = all((s) => !!s.shadow) ? style.shadow : null;
  // Merged into each shadow as it is when the edit goes out, so quick changes to two of its settings both stick.
  const setShadow = (patch: Partial<TextShadow>, key: string) => setStyle((s) => (s.shadow ? { shadow: { ...s.shadow, ...patch } } : {}), key);
  const shadowDiffers = (k: keyof TextShadow) => differ((s) => s.shadow?.[k]);
  const box = all((s) => s.background !== null) ? (style.background ?? BOX) : null;
  return (
    <>
      <Checkbox label="Bold" {...on((s) => s.bold)} onChange={(v) => setStyle({ bold: v }, "bold")} />
      <Slider label="Letter spacing" mixed={differ((s) => s.letterSpacing ?? 0)} value={(style.letterSpacing ?? 0) * 100} min={-50} max={200} step={1} unit="%" format={percent} onChange={(v) => setStyle({ letterSpacing: v / 100 }, "letterSpacing")} />
      <Slider label="Line spacing" mixed={differ((s) => s.lineSpacing ?? 0)} value={(style.lineSpacing ?? 0) * 100} min={-60} max={300} step={1} unit="%" format={percent} onChange={(v) => setStyle({ lineSpacing: v / 100 }, "lineSpacing")} />
      <FieldRow label="Alignment">
        <div className="min-w-0 flex-1">
          <Segmented label="Alignment" value={differ((s) => s.align ?? "center") ? null : (style.align ?? "center")} onChange={(align) => setStyle({ align }, "align")} options={ALIGNS} />
        </div>
      </FieldRow>
      <Slider label="Outline" mixed={differ((s) => s.strokeWidth)} value={style.strokeWidth} min={0} max={20} step={0.5} format={(v) => v.toFixed(1)} onChange={(v) => setStyle({ strokeWidth: v }, "stroke")} />
      {all((s) => s.strokeWidth > 0) && <ColorInput label="Outline color" mixed={differ((s) => s.strokeColor)} value={style.strokeColor} onChange={(v) => setStyle({ strokeColor: v }, "strokeColor")} />}
      <Checkbox label="Background box" {...on((s) => s.background !== null)} onChange={(v) => setStyle((s) => ({ background: v ? (s.background ?? BOX) : null }), "bg")} />
      {box && (
        <>
          <ColorInput label="Box color" mixed={differ((s) => s.background?.slice(0, 7))} value={box} onChange={(v) => setStyle((s) => ({ background: v.slice(0, 7) + (s.background ?? "").slice(7) }), "bgColor")} />
          <div className="grid grid-cols-2 gap-2">
            <div className="min-w-0">
              <Slider label="Opacity" name="Box opacity" narrow mixed={differ((s) => s.background && Math.round(boxOpacity(s.background) * 100))} value={boxOpacity(box) * 100} min={0} max={100} step={1} unit="%" format={percent} onChange={(v) => setStyle((s) => (s.background ? { background: withOpacity(s.background, v / 100) } : {}), "bgOpacity")} />
            </div>
            <div className="min-w-0">
              <Slider label="Corners" name="Box corners" narrow mixed={differ((s) => s.boxRadius ?? BOX_RADIUS)} value={(style.boxRadius ?? BOX_RADIUS) * 100} min={0} max={100} step={1} unit="%" format={percent} onChange={(v) => setStyle({ boxRadius: v / 100 }, "boxRadius")} />
            </div>
          </div>
        </>
      )}
      <Checkbox label="Shadow" {...on((s) => !!s.shadow)} onChange={(v) => setStyle((s) => ({ shadow: v ? (s.shadow ?? SHADOW) : null }), "shadow")} />
      {shadow && (
        <>
          <ColorInput label="Color" name="Shadow color" mixed={shadowDiffers("color")} value={shadow.color} onChange={(color) => setShadow({ color }, "shadowColor")} />
          {/* A cell each, so stacked sliders' pull towards the one above leaves these rows level. */}
          <div className="grid grid-cols-2 gap-2">
            <div className="min-w-0">
              <Slider label="Opacity" name="Shadow opacity" narrow mixed={shadowDiffers("opacity")} value={shadow.opacity * 100} min={0} max={100} step={1} unit="%" format={percent} onChange={(v) => setShadow({ opacity: v / 100 }, "shadowOpacity")} />
            </div>
            <div className="min-w-0">
              <Slider label="Blur" name="Shadow blur" narrow mixed={shadowDiffers("blur")} value={shadow.blur} min={0} max={100} step={1} unit="%" format={percent} onChange={(blur) => setShadow({ blur }, "shadowBlur")} />
            </div>
            <div className="min-w-0">
              <Slider label="Distance" name="Shadow distance" narrow mixed={shadowDiffers("distance")} value={shadow.distance} min={0} max={100} step={1} format={percent} onChange={(distance) => setShadow({ distance }, "shadowDistance")} />
            </div>
            <div className="min-w-0">
              <Slider label="Angle" name="Shadow angle" narrow mixed={shadowDiffers("angle")} value={shadow.angle} min={-180} max={180} step={1} unit="°" format={percent} parse={degrees} onChange={(angle) => setShadow({ angle }, "shadowAngle")} />
            </div>
          </div>
        </>
      )}
    </>
  );
}

const PLACE = ["x", "y", "scale", "rotation", "opacity"] as const;

/**
 * Where every caption sits, folded until someone opens it. A caption with keyframes keeps its own motion, as
 * the engine leaves it; the values shown are those of the captions without.
 */
export function CaptionPosition({ clips, flush }: { clips: Clip[]; flush?: boolean }) {
  const open = useStyleUi((s) => s.position);
  const free = clips.filter((c) => c.keyframes.length === 0);
  const keyed = clips.length - free.length;
  const field = (k: (typeof PLACE)[number]) => {
    const values = free.map((c) => c.content.transform[k]);
    return { value: values[0] ?? 0, mixed: values.some((v) => Math.abs(v - values[0]) > 1e-4) };
  };
  const set = (patch: Partial<Omit<Transform, "crop">>, key: string) => placeCaptions(patch, `captions:place:${key}`);
  const none = free.length === 0;
  const why = none ? "Every caption has keyframes, which decide where it is" : undefined;
  const [x, y, scale, rotation, opacity] = PLACE.map(field);
  return (
    <Disclosure
      title="Position"
      summary="Scale, position, rotation, opacity"
      open={open}
      onToggle={() => useStyleUi.setState({ position: !open })}
      flush={flush}
    >
      {keyed > 0 && !none && <p className="-mt-1 text-[12px] text-muted">{`${keyed} ${keyed === 1 ? "caption" : "captions"} with keyframes keep their own position.`}</p>}
      <Slider label="Scale" disabled={none} title={why} mixed={scale.mixed} value={scale.value * 100} min={10} max={400} step={1} unit="%" format={percent} onChange={(v) => set({ scale: v / 100 }, "scale")} />
      <Slider label="Position X" disabled={none} title={why} mixed={x.mixed} value={x.value * 100} min={-100} max={100} step={0.5} unit="%" format={(v) => v.toFixed(1)} onChange={(v) => set({ x: v / 100 }, "x")} />
      <Slider label="Position Y" disabled={none} title={why} mixed={y.mixed} value={y.value * 100} min={-100} max={100} step={0.5} unit="%" format={(v) => v.toFixed(1)} onChange={(v) => set({ y: v / 100 }, "y")} />
      <Slider label="Rotation" disabled={none} title={why} mixed={rotation.mixed} value={rotation.value} min={-180} max={180} step={1} unit="°" format={percent} onChange={(v) => set({ rotation: v }, "rotation")} />
      <Slider label="Opacity" disabled={none} title={why} mixed={opacity.mixed} value={opacity.value * 100} min={0} max={100} step={1} unit="%" format={percent} onChange={(v) => set({ opacity: v / 100 }, "opacity")} />
      <Button variant="ghost" className="h-7 self-start px-2 text-[12px]" disabled={none} disabledReason={why} onClick={() => set({ x: 0, y: LIMITS.captionY, scale: 1, rotation: 0, opacity: 1 }, "reset")}>
        <RotateCcw size={13} /> Reset
      </Button>
    </Disclosure>
  );
}

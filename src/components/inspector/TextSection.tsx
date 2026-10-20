import { useState } from "react";
import { Ban } from "lucide-react";
import { CAPTION_STYLES, KEYWORD_COLOR, KEYWORD_PICKS, TEXT_PRESETS, isLook, sameStyle } from "../../lib/presets";
import { editClip, useEditor } from "../../lib/store";
import type { Animation, CaptionPreset, Clip, EditCmd, LetterCase, TextShadow, TextStyle } from "../../lib/types";
import { FontPicker } from "../FontPicker";
import { Checkbox, ColorInput, PresetTile, Section, Segmented, Slider, TextSwatch, parseNumber, useLockReason } from "../ui";

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

/** Style tiles use the presets and names of the Text tab, or of the Captions tab for a caption. */
export function TextSection({ clip, text, style, caption }: { clip: Clip; text: string; style: TextStyle; caption: boolean }) {
  const edit = useEditor((s) => s.edit);
  // Built from the latest confirmed style, so two quick changes to different properties both stick.
  const setStyle: SetStyle = (patch, key) =>
    editClip(clip.id, (c) => (c.content.type === "text" ? { type: "updateClip", clipId: c.id, style: patched(c.content.style, patch) } : null), `${clip.id}:style:${key}`);
  const presets = caption ? CAPTION_STYLES : TEXT_PRESETS;
  return (
    <>
      <Section title="Text">
        <TextField clipId={clip.id} text={text} />
        <div className="grid grid-cols-3 gap-2">
          {presets.map((p) => (
            <PresetTile
              key={p.name}
              label={p.name}
              selected={caption ? isLook(p, style, clip.animIn, clip.animOut) : sameStyle(p.style, style)}
              title={`Apply ${p.name} style`}
              onClick={() => {
                const restyle: EditCmd = { type: "updateClip", clipId: clip.id, style: { ...p.style, fontFamily: p.style.fontFamily ?? style.fontFamily, maxWidth: style.maxWidth } };
                // A caption style brings its animations too.
                const animate = (slot: "in" | "out", animation: Animation | null | undefined): EditCmd => ({ type: "setAnimation", clipId: clip.id, slot, animation: animation ?? null });
                const look = p as CaptionPreset;
                edit(caption ? [restyle, animate("in", look.animIn), animate("out", look.animOut)] : restyle);
              }}
            >
              <span className="absolute inset-0 flex items-center justify-center bg-line">
                <TextSwatch style={{ ...p.style, fontFamily: p.style.fontFamily ?? style.fontFamily }} label="Aa" />
              </span>
            </PresetTile>
          ))}
        </div>
      </Section>
      <StyleSection style={style} setStyle={setStyle} />
      {caption && (
        <Section title="Karaoke">
          <Checkbox label="Highlight spoken word" checked={!!style.highlight} onChange={(v) => setStyle({ highlight: v ? "#ffe14d" : null }, "highlight")} />
          {style.highlight && (
            <>
              <ColorInput label="Highlight color" value={style.highlight} onChange={(v) => setStyle({ highlight: v }, "highlightColor")} />
              {unlitReason(clip, text) && <p className="text-[12px] text-muted">{unlitReason(clip, text)}</p>}
            </>
          )}
        </Section>
      )}
      {caption && (
        <Section title="Key words">
          <Segmented
            label="Key words"
            value={style.keywords?.pick ?? "off"}
            onChange={(id) => setStyle({ keywords: id === "off" ? null : { color: style.keywords?.color ?? KEYWORD_COLOR, pick: id } }, "keywords")}
            options={KEYWORD_PICKS}
          />
          {style.keywords && <ColorInput label="Key word color" value={style.keywords.color} onChange={(color) => setStyle({ keywords: { ...style.keywords!, color } }, "keywordColor")} />}
          {style.keywords && unlitReason(clip, text) && <p className="text-[12px] text-muted">{unlitReason(clip, text)}</p>}
        </Section>
      )}
    </>
  );
}

/** A change to a style, or one worked out from the style as it is when the edit goes out. */
export type StylePatch = Partial<TextStyle> | ((style: TextStyle) => Partial<TextStyle>);
type SetStyle = (patch: StylePatch, key: string) => void;

export function patched(style: TextStyle, patch: StylePatch): TextStyle {
  return { ...style, ...(typeof patch === "function" ? patch(style) : patch) };
}

/** `styles`: every selected text's, `style` being the first; a dash shows where they differ. */
type StyleProps = { style: TextStyle; styles?: TextStyle[]; setStyle: SetStyle };

function across(style: TextStyle, styles: TextStyle[] = [style]) {
  const json = (v: unknown) => JSON.stringify(v ?? null);
  return {
    differ: (get: (s: TextStyle) => unknown) => styles.some((s) => json(get(s)) !== json(get(style))),
    all: (has: (s: TextStyle) => boolean) => styles.every(has),
    on: (has: (s: TextStyle) => boolean) => ({ checked: styles.every(has), mixed: styles.some(has) && !styles.every(has) }),
  };
}

export function StyleSection({ style, styles, setStyle, maxSize = 300 }: StyleProps & { maxSize?: number }) {
  return (
    <Section title="Style">
      <MainStyle style={style} styles={styles} setStyle={setStyle} maxSize={maxSize} />
      <MoreStyle style={style} styles={styles} setStyle={setStyle} />
    </Section>
  );
}

/** Font, size and colour: what a text needs most. */
export function MainStyle({ style, styles, setStyle, maxSize = 300 }: StyleProps & { maxSize?: number }) {
  const { differ } = across(style, styles);
  return (
    <>
      <FieldRow label="Font">
        <FontPicker value={style.fontFamily} mixed={differ((s) => s.fontFamily)} onChange={(fontFamily) => setStyle({ fontFamily }, "font")} />
      </FieldRow>
      <Slider label="Size" mixed={differ((s) => s.fontSize)} value={style.fontSize} min={12} max={maxSize} step={1} format={(v) => String(Math.round(v))} onChange={(v) => setStyle({ fontSize: v }, "size")} />
      <ColorInput label="Color" mixed={differ((s) => s.color)} value={style.color} onChange={(v) => setStyle({ color: v }, "color")} />
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
  { id: "upper", label: "AB", title: "All capitals" },
  { id: "lower", label: "ab", title: "All small letters" },
  { id: "title", label: "Ab", title: "Each word capitalised" },
];

/** CapCut's own shadow, as it starts when turned on. */
const SHADOW: TextShadow = { color: "#000000", opacity: 0.9, blur: 15, distance: 5, angle: -45 };

const percent = (v: number) => String(Math.round(v));
const degrees = (s: string) => ((((parseNumber(s) + 180) % 360) + 360) % 360) - 180;

export function MoreStyle({ style, styles, setStyle }: StyleProps) {
  const { differ, all, on } = across(style, styles);
  const shadow = all((s) => !!s.shadow) ? style.shadow : null;
  // Merged into each shadow as it is when the edit goes out, so quick changes to two of its settings both stick.
  const setShadow = (patch: Partial<TextShadow>, key: string) => setStyle((s) => (s.shadow ? { shadow: { ...s.shadow, ...patch } } : {}), key);
  const shadowDiffers = (k: keyof TextShadow) => differ((s) => s.shadow?.[k]);
  return (
    <>
      <Checkbox label="Bold" {...on((s) => s.bold)} onChange={(v) => setStyle({ bold: v }, "bold")} />
      <FieldRow label="Case">
        <div className="min-w-0 flex-1">
          <Segmented label="Case" value={differ((s) => s.letterCase ?? "none") ? null : (style.letterCase ?? "none")} onChange={(letterCase) => setStyle({ letterCase }, "case")} options={CASES} />
        </div>
      </FieldRow>
      <Slider label="Letter spacing" mixed={differ((s) => s.letterSpacing ?? 0)} value={(style.letterSpacing ?? 0) * 100} min={-50} max={200} step={1} unit="%" format={percent} onChange={(v) => setStyle({ letterSpacing: v / 100 }, "letterSpacing")} />
      <Slider label="Line spacing" mixed={differ((s) => s.lineSpacing ?? 0)} value={(style.lineSpacing ?? 0) * 100} min={-60} max={300} step={1} unit="%" format={percent} onChange={(v) => setStyle({ lineSpacing: v / 100 }, "lineSpacing")} />
      <Slider label="Outline" mixed={differ((s) => s.strokeWidth)} value={style.strokeWidth} min={0} max={20} step={0.5} format={(v) => v.toFixed(1)} onChange={(v) => setStyle({ strokeWidth: v }, "stroke")} />
      {all((s) => s.strokeWidth > 0) && <ColorInput label="Outline color" mixed={differ((s) => s.strokeColor)} value={style.strokeColor} onChange={(v) => setStyle({ strokeColor: v }, "strokeColor")} />}
      <Checkbox label="Background box" {...on((s) => s.background !== null)} onChange={(v) => setStyle((s) => ({ background: v ? (s.background ?? "#000000b3") : null }), "bg")} />
      {all((s) => s.background !== null) && <ColorInput label="Box color" mixed={differ((s) => s.background?.slice(0, 7))} value={style.background ?? "#000000b3"} onChange={(v) => setStyle((s) => ({ background: v.slice(0, 7) + (s.background ?? "").slice(7) }), "bgColor")} />}
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

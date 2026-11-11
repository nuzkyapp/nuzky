import { useShallow } from "zustand/react/shallow";
import { Copy, Layers, Repeat2, RotateCcw, Trash2 } from "lucide-react";
import { cropAndMask, ownTransform } from "../../lib/keyframes";
import { LIMITS } from "../../lib/limits";
import { ADJUST_ROWS, DEFAULT_TRANSFORM, NO_ADJUST, SPLIT_LAYOUTS, sameAdjust } from "../../lib/presets";
import { deleteSelection, duplicateSelection, editClips, findClip, isCaptionTrack, patched, readoutTime, transformAtPlayhead, useCanvasUnit, useEditor, type StylePatch } from "../../lib/store";
import type { Adjust, Clip, EditCmd, Project, Track, Transform } from "../../lib/types";
import { Button, IconButton, PresetTile, Section, Slider } from "../ui";
import { InspectorHeader, QUIET } from "./Header";
import { BackgroundSection } from "./BackgroundSection";
import { DuckingRow, MonoRow, VoiceSection } from "./MediaSections";
import { StyleSection } from "./TextSection";
import { useKeyLabels } from "../../lib/keys";

type Found = { clip: Clip; track: Track };

/** What a clip offers to a shared edit. */
function facts(project: Project, { clip, track }: Found) {
  const c = clip.content;
  const asset = c.type === "media" ? project.assets.find((a) => a.id === c.assetId) : undefined;
  const onAudioTrack = track.kind === "audio";
  return {
    transform: !onAudioTrack && c.type !== "adjustment",
    adjust: c.type === "media" && !onAudioTrack && asset?.kind !== "audio",
    sound: c.type === "media" && c.speed !== 0 && (onAudioTrack || !!asset?.hasAudio),
    text: c.type === "text",
    kind: c.type === "text" ? (isCaptionTrack(track) ? "caption" : "text") : c.type === "adjustment" ? "adjustment" : onAudioTrack ? "audio" : (asset?.kind ?? "video"),
  };
}

/** The shared value, or mixed when the clips differ. */
function shared(values: number[]) {
  return { value: values[0] ?? 0, mixed: values.some((v) => Math.abs(v - values[0]) > 1e-4) };
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** Nouns for the selection summary: "2 videos · 3 captions · 1 audio clip". */
const KIND_NOUNS: Record<string, string> = { video: "video", image: "image", caption: "caption", text: "text clip", audio: "audio clip", adjustment: "adjustment layer" };

const KEYS = ["x", "y", "scale", "rotation", "opacity"] as const;

function TransformRows({ found, coalesce }: { found: Found[]; coalesce: string }) {
  const ids = found.map((f) => f.clip.id);
  // Plain numbers, so playback re-renders the rows only when a keyframed value moves.
  const flat = useEditor(
    useShallow((s) =>
      found.flatMap((f) => {
        const t = transformAtPlayhead(f.clip, readoutTime(s));
        return KEYS.map((k) => t[k]);
      }),
    ),
  );
  const field = (k: (typeof KEYS)[number]) => shared(found.map((_, i) => flat[i * KEYS.length + KEYS.indexOf(k)]));
  const keyed = found.filter((f) => f.clip.keyframes.length > 0).length;
  // One absolute value for every clip, keyframes included, so "110 % everywhere" really is everywhere.
  const set = (patch: Partial<Transform>, key: string) =>
    editClips(
      ids,
      (clip): EditCmd[] => {
        const cmds: EditCmd[] = [{ type: "updateClip", clipId: clip.id, transform: { ...ownTransform(clip), ...patch } }];
        if (clip.keyframes.length > 0) cmds.push({ type: "setKeyframes", clipId: clip.id, keyframes: clip.keyframes.map((k) => ({ ...k, transform: { ...k.transform, ...patch } })) });
        return cmds;
      },
      `${coalesce}:${key}`,
    );
  const reset = () =>
    editClips(ids, (clip, track) => [
      {
        type: "updateClip",
        clipId: clip.id,
        transform: { ...DEFAULT_TRANSFORM, y: isCaptionTrack(track) ? LIMITS.captionY : 0, ...cropAndMask(transformAtPlayhead(clip, useEditor.getState().timeUs)) },
      },
      { type: "setKeyframes", clipId: clip.id, keyframes: [] },
    ]);
  const scale = field("scale");
  const x = field("x");
  const y = field("y");
  const rotation = field("rotation");
  const opacity = field("opacity");
  return (
    <Section
      title="Transform"
      actions={
        <IconButton label="Reset transform of all selected clips" className={QUIET} onClick={reset}>
          <RotateCcw size={14} />
        </IconButton>
      }
    >
      {keyed > 0 && <p className="-mt-1 text-[12px] text-muted">{plural(keyed, "clip")} with keyframes: changes apply to every keyframe.</p>}
      <Slider label="Scale" mixed={scale.mixed} value={scale.value * 100} min={10} max={400} step={1} unit="%" format={(v) => String(Math.round(v))} onChange={(v) => set({ scale: v / 100 }, "scale")} />
      <Slider label="Position X" mixed={x.mixed} value={x.value * 100} min={-100} max={100} step={0.5} unit="%" format={(v) => v.toFixed(1)} onChange={(v) => set({ x: v / 100 }, "x")} />
      <Slider label="Position Y" mixed={y.mixed} value={y.value * 100} min={-100} max={100} step={0.5} unit="%" format={(v) => v.toFixed(1)} onChange={(v) => set({ y: v / 100 }, "y")} />
      <Slider label="Rotation" mixed={rotation.mixed} value={rotation.value} min={-180} max={180} step={1} unit="°" format={(v) => String(Math.round(v))} onChange={(v) => set({ rotation: v }, "rot")} />
      <Slider label="Opacity" mixed={opacity.mixed} value={opacity.value * 100} min={0} max={100} step={1} unit="%" format={(v) => String(Math.round(v))} onChange={(v) => set({ opacity: v / 100 }, "opacity")} />
    </Section>
  );
}

type Split = (typeof SPLIT_LAYOUTS)[number];
const near = (a: number, b: number) => Math.abs(a - b) < 1e-3;

/** The cell of `split` the clip's centre is in the middle of, or -1. */
function cellOf(split: Split, clip: Clip) {
  const t = ownTransform(clip);
  return split.cells.findIndex(([l, top, r, b]) => near(t.x, (l + r) / 2 - 0.5) && near(t.y, (top + b) / 2 - 0.5));
}

/** A layout's cells drawn in the frame's shape, numbered in the order clips fill them. */
function SplitDiagram({ split, aspect }: { split: Split; aspect: number }) {
  // Inside the tile's 4:3 with room around it: a vertical frame fits its height, a wide one its width.
  const [w, h] = aspect >= 4 / 3 ? [80, 80 / aspect] : [44 * aspect, 44];
  return (
    <svg aria-hidden className="absolute inset-0 m-auto" width={w} height={h} viewBox={`0 0 ${w} ${h}`}>
      {split.cells.map(([l, t, r, b], i) => (
        <g key={i}>
          <rect x={l * w + 1} y={t * h + 1} width={(r - l) * w - 2} height={(b - t) * h - 2} rx={2} className="fill-white/[.14]" />
          <text x={((l + r) / 2) * w} y={((t + b) / 2) * h} dominantBaseline="central" textAnchor="middle" className="fill-fg/80 text-[10px] font-semibold">
            {i + 1}
          </text>
        </g>
      ))}
    </svg>
  );
}

/**
 * 2 to 4 videos or images side by side: each layout puts them in its cells, the clip on the highest track in the first,
 * as the timeline lists them top down; the header's button moves each to the next cell.
 */
function SplitScreenRows({ found }: { found: Found[] }) {
  const project = useEditor((s) => s.snap!.project);
  const edit = useEditor((s) => s.edit);
  const track = (f: Found) => project.tracks.findIndex((t) => t.id === f.track.id);
  const ordered = [...found].sort((a, b) => track(b) - track(a) || a.clip.startUs - b.clip.startUs).map((f) => f.clip);
  const splits = SPLIT_LAYOUTS.filter((s) => s.cells.length === ordered.length);
  // The layout every clip sits in a cell of, one clip a cell, and the clips in the order of its cells.
  const placed = (split: Split) => {
    const byCell: (Clip | null)[] = split.cells.map(() => null);
    for (const clip of ordered) {
      const cell = cellOf(split, clip);
      if (cell < 0 || byCell[cell]) return null;
      byCell[cell] = clip;
    }
    return byCell as Clip[];
  };
  const current = splits.find((s) => placed(s));
  const apply = (split: Split, clips: Clip[]) => edit({ type: "splitScreen", clipIds: clips.map((c) => c.id), layout: split.layout });
  const aspect = project.canvas.width / project.canvas.height;
  return (
    <Section
      title="Split screen"
      actions={
        <IconButton
          label={current ? "Move each clip to the next part" : "Move each clip to the next part: choose a layout first"}
          className={QUIET}
          disabled={!current}
          onClick={() => {
            const byCell = current && placed(current);
            if (current && byCell) apply(current, [byCell[byCell.length - 1], ...byCell.slice(0, -1)]);
          }}
        >
          <Repeat2 size={14} />
        </IconButton>
      }
    >
      <div className="grid grid-cols-3 gap-2" data-split-screen>
        {splits.map((split) => (
          <PresetTile key={split.layout} label={split.label} selected={split === current} onClick={() => apply(split, (split === current && placed(split)) || ordered)}>
            <SplitDiagram split={split} aspect={aspect} />
          </PresetTile>
        ))}
      </div>
    </Section>
  );
}

const adjustOf = (clip: Clip) => (clip.content.type === "media" ? clip.content.adjust : NO_ADJUST);

function AdjustRows({ found, coalesce }: { found: Found[]; coalesce: string }) {
  const ids = found.map((f) => f.clip.id);
  const set = (key: keyof Adjust, v: number) =>
    editClips(ids, (c) => (c.content.type === "media" ? { type: "updateClip", clipId: c.id, adjust: { ...c.content.adjust, [key]: v / 100 } } : null), `${coalesce}:adjust:${key}`);
  const untouched = found.every((f) => sameAdjust(adjustOf(f.clip), NO_ADJUST));
  return (
    <Section
      title="Adjust"
      actions={
        <IconButton label="Reset adjustments of all selected clips" className={QUIET} disabled={untouched} onClick={() => editClips(ids, (c) => ({ type: "updateClip", clipId: c.id, adjust: NO_ADJUST }))}>
          <RotateCcw size={14} />
        </IconButton>
      }
    >
      {ADJUST_ROWS.map((r) => {
        const s = shared(found.map((f) => adjustOf(f.clip)[r.key]));
        return <Slider key={r.key} label={r.label} mixed={s.mixed} value={s.value * 100} min={r.min} max={100} step={1} format={(v) => String(Math.round(v))} onChange={(v) => set(r.key, v)} />;
      })}
    </Section>
  );
}

function VolumeRow({ found, coalesce }: { found: Found[]; coalesce: string }) {
  const ids = found.map((f) => f.clip.id);
  const s = shared(found.map((f) => (f.clip.content.type === "media" ? f.clip.content.volume : 0)));
  return (
    <Section title="Audio">
      <Slider
        label="Volume"
        mixed={s.mixed}
        value={s.value * 100}
        min={0}
        max={200}
        step={1}
        unit="%"
        format={(v) => String(Math.round(v))}
        onChange={(v) => editClips(ids, (c) => ({ type: "updateClip", clipId: c.id, volume: v / 100 }), `${coalesce}:volume`)}
      />
      <DuckingRow clips={found.map((f) => f.clip)} coalesce={coalesce} />
      <MonoRow clips={found.map((f) => f.clip)} />
    </Section>
  );
}

function StyleRows({ found, coalesce }: { found: Found[]; coalesce: string }) {
  const unit = useCanvasUnit();
  const ids = found.map((f) => f.clip.id);
  const styles = found.flatMap((f) => (f.clip.content.type === "text" ? [f.clip.content.style] : []));
  const setStyle = (patch: StylePatch, key: string) =>
    editClips(ids, (c) => (c.content.type === "text" ? { type: "updateClip", clipId: c.id, style: patched(c.content.style, patch) } : null), `${coalesce}:style:${key}`);
  return <StyleSection style={styles[0]} styles={styles} setStyle={setStyle} unit={unit} />;
}

/** Several clips: the controls they all share, each change applied to all of them as one undo step. */
export function MultiInspector({ ids }: { ids: string[] }) {
  const { withKey } = useKeyLabels();
  const project = useEditor((s) => s.snap!.project);
  const found = ids.map((id) => findClip(project, id)).filter((f): f is Found => !!f);
  const info = found.map((f) => facts(project, f));
  const all = (k: "transform" | "adjust" | "sound" | "text") => found.length > 0 && info.every((i) => i[k]);
  const counts = new Map<string, number>();
  for (const i of info) counts.set(i.kind, (counts.get(i.kind) ?? 0) + 1);
  const detail = [...counts].map(([kind, n]) => plural(n, KIND_NOUNS[kind] ?? kind)).join(" · ");
  // Scoped to this selection, so a drag is one undo step and never merges with another selection's.
  const coalesce = `multi:${ids.join(",")}`;
  return (
    <>
      <InspectorHeader icon={Layers} title={`${found.length} clips selected`} detail={detail} />
      <div className="scroll-shown min-h-0 flex-1 overflow-y-auto">
        <div className="grid grid-cols-2 gap-2 border-b border-white/[.07] px-4 pb-4 pt-1">
          <Button onClick={duplicateSelection} title={withKey("Duplicate", "duplicate")}>
            <Copy size={14} /> Duplicate
          </Button>
          <Button variant="danger" onClick={deleteSelection} title={withKey("Delete", "delete")}>
            <Trash2 size={14} /> Delete
          </Button>
        </div>
        {found.length >= 2 && found.length <= 4 && all("adjust") && found.every((f) => f.track.kind === "video") && <SplitScreenRows found={found} />}
        {all("transform") && <TransformRows found={found} coalesce={coalesce} />}
        {all("text") && <StyleRows found={found} coalesce={coalesce} />}
        {all("adjust") && <BackgroundSection clips={found.map((f) => f.clip)} />}
        {all("adjust") && <AdjustRows found={found} coalesce={coalesce} />}
        {all("sound") && <VolumeRow found={found} coalesce={coalesce} />}
        {all("sound") && <VoiceSection clips={found.map((f) => f.clip)} coalesce={coalesce} />}
      </div>
    </>
  );
}

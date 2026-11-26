import { useRef, useState, type ReactNode } from "react";
import { useShallow } from "zustand/react/shallow";
import { ChevronDown, ChevronLeft, ChevronRight, Diamond, Maximize, Minimize, RotateCcw } from "lucide-react";
import { clipOffset, cropAndMask, curveIndex, keyframeIndexAt, keyframeTolerance, ownTransform, transformAt, upsertKeyframe } from "../../lib/keyframes";
import { LIMITS } from "../../lib/limits";
import { DEFAULT_TRANSFORM } from "../../lib/presets";
import { editClip, isCaptionTrack, readoutTime, setClipTransform, transformAtPlayhead, useEditor } from "../../lib/store";
import type { Asset, Clip, Ease, EditCmd, Transform } from "../../lib/types";
import { Button, IconButton, Menu, Section, Segmented, Slider } from "../ui";
import { FollowButton } from "./FollowButton";
import { summaryOf, useFold } from "./folds";
import { QUIET } from "./Header";

function KeyframeControls({ clip, at }: { clip: Clip; at: AtPlayhead }) {
  const { seek } = useEditor.getState();
  const fps = useEditor((s) => s.snap!.project.canvas.fps);
  const tol = keyframeTolerance(fps);
  const { prevUs, nextUs, inside, atIndex } = at;
  const has = atIndex >= 0;

  // Built from the latest confirmed keyframes, so a keyframe added a moment ago is kept.
  const toggle = () =>
    editClip(clip.id, (c): EditCmd | EditCmd[] => {
      const offset = clipOffset(c, useEditor.getState().timeUs);
      const i = keyframeIndexAt(c, offset, tol);
      if (i < 0) return { type: "setKeyframes", clipId: c.id, keyframes: upsertKeyframe(c, offset, transformAt(c, ownTransform(c), offset), tol) };
      const rest = c.keyframes.filter((_, j) => j !== i);
      const remove: EditCmd = { type: "setKeyframes", clipId: c.id, keyframes: rest };
      // Removing the last keyframe keeps its values as the clip's fixed transform.
      return rest.length > 0 ? remove : [{ type: "updateClip", clipId: c.id, transform: c.keyframes[i].transform }, remove];
    });

  return (
    <>
      <IconButton label="Previous keyframe" className={QUIET} disabled={prevUs === null} onClick={() => prevUs !== null && seek(clip.startUs + prevUs)}>
        <ChevronLeft size={14} />
      </IconButton>
      <IconButton
        label={!inside ? "Move the playhead over this clip to add a keyframe" : has ? "Remove keyframe at playhead" : "Add keyframe at playhead"}
        className={QUIET}
        active={has}
        disabled={!inside}
        onClick={toggle}
      >
        <Diamond size={13} fill={has ? "currentColor" : "none"} />
      </IconButton>
      <IconButton label="Next keyframe" className={QUIET} disabled={nextUs === null} onClick={() => nextUs !== null && seek(clip.startUs + nextUs)}>
        <ChevronRight size={14} />
      </IconButton>
    </>
  );
}

const CURVES: { id: Ease; label: string }[] = [
  { id: "linear", label: "Linear" },
  { id: "easeIn", label: "Ease in" },
  { id: "easeOut", label: "Ease out" },
  { id: "smooth", label: "Smooth" },
];

/**
 * How the values move from the keyframe at or before the playhead on to the next one: a menu that names the two
 * keyframes, so it is clear which part of the clip it changes.
 */
function CurveMenu({ clip, index, last }: { clip: Clip; index: number; /** The playhead is on the last keyframe. */ last: boolean }) {
  const fps = useEditor((s) => s.snap!.project.canvas.fps);
  const button = useRef<HTMLButtonElement>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; keyboard: boolean } | null>(null);
  // Built from the latest confirmed keyframes and the playhead at the click, as the diamond is.
  const set = (ease: Ease) =>
    editClip(clip.id, (c) => {
      const i = curveIndex(c, clipOffset(c, useEditor.getState().timeUs), keyframeTolerance(fps));
      return i < 0 ? null : { type: "setKeyframes", clipId: c.id, keyframes: c.keyframes.map((k, j) => (j === i ? { ...k, ease } : k)) };
    });
  const ease = index >= 0 ? CURVES.find((c) => c.id === clip.keyframes[index].ease) : undefined;
  const why = last ? "No keyframe after this one to curve to" : "Move the playhead to a keyframe or between two";
  return (
    <>
      <Button
        variant="ghost"
        aria-haspopup="menu"
        aria-expanded={!!menu}
        aria-label={ease ? `Curve from keyframe ${index + 1} to ${index + 2}: ${ease.label}` : "Curve to next keyframe"}
        title={ease ? `How the picture moves from keyframe ${index + 1} to ${index + 2}` : undefined}
        disabled={index < 0}
        disabledReason={why}
        className="-mr-1.5 h-6 gap-1 px-1.5 text-[12px]"
        onClick={(e) => {
          button.current = e.currentTarget;
          const r = e.currentTarget.getBoundingClientRect();
          setMenu({ x: r.right, y: r.bottom + 4, keyboard: e.detail === 0 });
        }}
      >
        {ease && <span className="tabular text-muted">{`${index + 1} → ${index + 2}`}</span>}
        <span>{ease?.label ?? "Curve"}</span>
        <ChevronDown size={13} className="text-muted" />
      </Button>
      {menu && (
        <Menu
          label="Curve to next keyframe"
          at={{ x: menu.x, y: menu.y, align: "end" }}
          keyboard={menu.keyboard}
          minWidth={160}
          onClose={() => {
            setMenu(null);
            requestAnimationFrame(() => button.current?.focus());
          }}
          items={CURVES.map((c) => ({ label: c.label, checked: c.id === ease?.id, run: () => set(c.id) }))}
        />
      )}
    </>
  );
}

/** What the playhead decides in the Transform section, as plain values. */
interface AtPlayhead extends Transform {
  inside: boolean;
  atIndex: number;
  /** The keyframe whose curve the playhead is on (`curveIndex`), or -1. */
  curve: number;
  prevUs: number | null;
  nextUs: number | null;
}

function atPlayhead(clip: Clip, timeUs: number, fps: number): AtPlayhead {
  const offset = clipOffset(clip, timeUs);
  const tol = keyframeTolerance(fps);
  const inside = timeUs >= clip.startUs && timeUs <= clip.startUs + clip.durationUs;
  return {
    ...transformAt(clip, ownTransform(clip), offset),
    inside,
    atIndex: inside ? keyframeIndexAt(clip, offset, tol) : -1,
    curve: inside ? curveIndex(clip, offset, tol) : -1,
    prevUs: [...clip.keyframes].reverse().find((k) => k.tUs < offset - tol)?.tUs ?? null,
    nextUs: clip.keyframes.find((k) => k.tUs > offset + tol)?.tUs ?? null,
  };
}

/** What the folded Transform says is set: "Scale 116 % · moved · 2 keyframes". */
function summary(t: Transform, keyframes: number) {
  const parts = [
    Math.abs(t.scale - 1) > 1e-4 && `scale ${Math.round(t.scale * 100)} %`,
    (Math.abs(t.x) > 1e-4 || Math.abs(t.y) > 1e-4) && "moved",
    Math.abs(t.rotation) > 1e-4 && `rotated ${Math.round(t.rotation)}°`,
    t.opacity < 1 - 1e-4 && `opacity ${Math.round(t.opacity * 100)} %`,
    keyframes > 0 && `${keyframes} keyframe${keyframes === 1 ? "" : "s"}`,
  ].filter((p): p is string => !!p);
  return summaryOf(parts).replace(/, /g, " · ");
}

/**
 * `caption`: says that this moves the one caption, since its style can go to every caption. `fold`: the section
 * folds, as in a video's or image's tab with other sections under it. `follow`: offers Follow under Fit and Fill.
 */
export function TransformSection({ clip, asset, caption = false, fold = false, follow = false }: { clip: Clip; asset?: Asset; caption?: boolean; fold?: boolean; follow?: boolean }) {
  const canvas = useEditor((s) => s.snap!.project.canvas);
  // Plain values, so playback re-renders the section only when a keyframed value moves or a keyframe is passed.
  const at = useEditor(useShallow((s) => atPlayhead(clip, readoutTime(s), canvas.fps)));
  const { atIndex } = at;
  const transform = at;
  const set = (patch: Partial<Transform>, key: string) => setClipTransform(clip.id, patch, `${clip.id}:${key}`);
  // Scale that makes the media cover the whole canvas instead of fitting inside it.
  const fill =
    asset && asset.width > 0
      ? Math.max(canvas.width / asset.width, canvas.height / asset.height) / Math.min(canvas.width / asset.width, canvas.height / asset.height)
      : 1;
  const keyed = clip.keyframes.length > 0;
  const fillScale = Math.round(fill * 1000) / 1000;
  // Which of the two the clip shows now, so the segmented control marks it; any other framing marks neither.
  const centred = Math.abs(transform.x) < 1e-4 && Math.abs(transform.y) < 1e-4;
  const framing = !centred ? null : Math.abs(transform.scale - 1) < 1e-4 ? "fit" : Math.abs(transform.scale - fillScale) < 1e-4 ? "fill" : null;
  const said = summary(transform, clip.keyframes.length);
  const folding = useFold("transform", said, said !== "None");

  // New text is centred; captions sit low in the frame, where the engine puts them.
  // The crop at the playhead stays: it has its own section and reset.
  const reset = () =>
    editClip(clip.id, (c, track) => [
      {
        type: "updateClip",
        clipId: c.id,
        transform: { ...DEFAULT_TRANSFORM, y: isCaptionTrack(track) ? LIMITS.captionY : 0, ...cropAndMask(transformAtPlayhead(c, useEditor.getState().timeUs)) },
      },
      { type: "setKeyframes", clipId: c.id, keyframes: [] },
    ]);

  return (
    <Section
      title="Transform"
      fold={fold ? folding : undefined}
      actions={
        <>
          <KeyframeControls clip={clip} at={at} />
          <span aria-hidden className="mx-1 h-3.5 w-px bg-white/[.08]" />
          <IconButton label={keyed ? "Reset transform and remove keyframes" : "Reset transform"} className={QUIET} onClick={reset}>
            <RotateCcw size={14} />
          </IconButton>
        </>
      }
    >
      {caption && <p className="-mt-1 text-[12px] text-muted">Moves this caption only. Position of all captions, in the Text tab, moves every caption.</p>}
      {keyed && (
        <div className="-mt-1 flex min-h-6 items-center justify-between gap-2">
          <p className="tabular text-[12px] text-muted">{atIndex >= 0 ? `Edits change keyframe ${atIndex + 1}` : "Edits add a keyframe"}</p>
          {clip.keyframes.length > 1 && <CurveMenu clip={clip} index={at.curve} last={atIndex === clip.keyframes.length - 1} />}
        </div>
      )}
      <Slider label="Scale" neutral={100} value={transform.scale * 100} min={10} max={400} step={1} unit="%" format={(v) => String(Math.round(v))} onChange={(v) => set({ scale: v / 100 }, "scale")} />
      <Slider label="Position X" neutral={0} value={transform.x * 100} min={-100} max={100} step={0.5} unit="%" format={(v) => v.toFixed(1)} onChange={(v) => set({ x: v / 100 }, "x")} />
      <Slider label="Position Y" neutral={0} value={transform.y * 100} min={-100} max={100} step={0.5} unit="%" format={(v) => v.toFixed(1)} onChange={(v) => set({ y: v / 100 }, "y")} />
      <Slider label="Rotation" neutral={0} value={transform.rotation} min={-180} max={180} step={1} unit="°" format={(v) => String(Math.round(v))} onChange={(v) => set({ rotation: v }, "rot")} />
      <Slider label="Opacity" neutral={100} value={transform.opacity * 100} min={0} max={100} step={1} unit="%" format={(v) => String(Math.round(v))} onChange={(v) => set({ opacity: v / 100 }, "opacity")} />
      {asset && asset.kind !== "audio" && (
        <Segmented
          label="Fit or fill"
          value={framing}
          onChange={(id) => (id === "fit" ? set({ scale: 1, x: 0, y: 0 }, "fit") : set({ scale: fillScale, x: 0, y: 0 }, "fill"))}
          options={[
            { id: "fit", label: <SegLabel icon={<Minimize size={13} />}>Fit</SegLabel> },
            { id: "fill", label: <SegLabel icon={<Maximize size={13} />}>Fill</SegLabel> },
          ]}
        />
      )}
      {follow && asset && <FollowButton clip={clip} asset={asset} />}
    </Section>
  );
}

/** Icon and text of a segment, centred together. */
function SegLabel({ icon, children }: { icon: ReactNode; children: ReactNode }) {
  return (
    <span className="inline-flex items-center justify-center gap-1.5">
      {icon}
      {children}
    </span>
  );
}

import { useShallow } from "zustand/react/shallow";
import { ownTransform } from "../../lib/keyframes";
import { editClip, readoutTime, setClipTransform, transformAtPlayhead, useEditor } from "../../lib/store";
import type { Asset, Clip, EditCmd, Mask, MaskKind, Transform } from "../../lib/types";
import { Checkbox, Section, Segmented, Slider } from "../ui";

type Choice = MaskKind | "none";
type Value = "x" | "y" | "width" | "height" | "rotation" | "feather";

const pct = (v: number) => String(Math.round(v));

/** A new mask: round, half the picture's shorter side across, in its middle. */
export function newMask(kind: MaskKind, asset: Asset): Mask {
  const short = Math.min(asset.width, asset.height);
  return { kind, x: 0, y: 0, width: (0.5 * short) / asset.width, height: (0.5 * short) / asset.height, rotation: 0, feather: 0 };
}

/**
 * Gives the clip's own transform and every keyframe the mask `change` makes of theirs, so the shape and its inversion
 * are the same through the whole clip while its place, size, turn and feather can move.
 */
function everywhere(clip: Clip, change: (mask: Mask | null | undefined) => Mask | null): EditCmd[] {
  const set = <T extends Transform>(t: T): T => ({ ...t, mask: change(t.mask) });
  const cmds: EditCmd[] = [{ type: "updateClip", clipId: clip.id, transform: set(ownTransform(clip)) }];
  if (clip.keyframes.length > 0) cmds.push({ type: "setKeyframes", clipId: clip.id, keyframes: clip.keyframes.map((k) => ({ ...k, transform: set(k.transform) })) });
  return cmds;
}

/**
 * Mask (videos and images): a circle, rectangle or line that shows only part of the picture. Width and height read in
 * shares of the picture's shorter side, so equal values are round or square; the project keeps them in shares of its
 * width and height.
 */
export function MaskSection({ clip, asset }: { clip: Clip; asset: Asset }) {
  // Plain values, so playback re-renders the section only when a keyframed mask moves.
  const mask = useEditor(useShallow((s) => transformAtPlayhead(clip, readoutTime(s)).mask ?? null));
  const keyed = clip.keyframes.length > 0;
  const short = Math.min(asset.width, asset.height) || 1;
  const side = { width: asset.width / short, height: asset.height / short };

  const pick = (choice: Choice) =>
    editClip(clip.id, (c): EditCmd[] => {
      const shown = transformAtPlayhead(c, useEditor.getState().timeUs).mask;
      if (choice === "none") return everywhere(c, () => null);
      return everywhere(c, (m) => ({ ...(m ?? shown ?? newMask(choice, asset)), kind: choice }));
    });
  const setInvert = (invert: boolean) => editClip(clip.id, (c) => everywhere(c, (m) => (m ? { ...m, invert } : null)));
  // Built from the transform at the playhead when the edit is sent, so a second slider never overwrites the first.
  const set = (key: Value, value: number) =>
    setClipTransform(clip.id, (t) => (t.mask ? { mask: { ...t.mask, [key]: value } } : {}), `${clip.id}:mask:${key}`);

  return (
    <Section title="Mask">
      <Segmented
        label="Mask"
        value={mask?.kind ?? "none"}
        onChange={pick}
        options={[
          { id: "none", label: "None", title: "Show the whole picture" },
          { id: "circle", label: "Circle", title: "Show the picture inside a circle" },
          { id: "rectangle", label: "Rectangle", title: "Show the picture inside a rectangle" },
          { id: "linear", label: "Linear", title: "Show the picture on one side of a line" },
        ]}
      />
      {mask && (
        <>
          {keyed && <p className="text-[12px] text-muted">The mask follows the transform keyframes</p>}
          <Slider label="Position X" name="Mask position X" value={mask.x * 100} min={-100} max={100} step={0.5} unit="%" format={(v) => v.toFixed(1)} onChange={(v) => set("x", v / 100)} />
          <Slider label="Position Y" name="Mask position Y" value={mask.y * 100} min={-100} max={100} step={0.5} unit="%" format={(v) => v.toFixed(1)} onChange={(v) => set("y", v / 100)} />
          {mask.kind !== "linear" && (
            <>
              <Slider label="Width" name="Mask width" value={mask.width * side.width * 100} min={1} max={400} step={1} unit="%" format={pct} onChange={(v) => set("width", v / 100 / side.width)} />
              <Slider label="Height" name="Mask height" value={mask.height * side.height * 100} min={1} max={400} step={1} unit="%" format={pct} onChange={(v) => set("height", v / 100 / side.height)} />
            </>
          )}
          <Slider label="Rotation" name="Mask rotation" value={mask.rotation} min={-180} max={180} step={1} unit="°" format={pct} onChange={(v) => set("rotation", v)} />
          <Slider label="Feather" name="Mask feather" value={mask.feather * 100} min={0} max={100} step={1} unit="%" format={pct} onChange={(v) => set("feather", v / 100)} />
          <Checkbox label="Invert" checked={!!mask.invert} title="Show the picture outside the shape and hide it inside" onChange={setInvert} />
        </>
      )}
    </Section>
  );
}

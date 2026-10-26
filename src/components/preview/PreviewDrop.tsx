import { useEffect, useRef } from "react";
import { useShallow } from "zustand/react/shallow";
import { aiLocked, useAiLocked, useEditor } from "../../lib/store";
import type { Canvas } from "../../lib/types";
import { addDropResolver, type DropTarget } from "../panel/assets";

/** The longer side of a picture in picture and its gap to the edges, as shares of the canvas's shorter side, as the engine places it. */
const PIP_SIZE = 0.45;
const PIP_MARGIN = 0.04;

/** Where a picture in picture of a w × h picture lands on the canvas, in canvas pixels. */
function pipBox(canvas: Canvas, w: number, h: number) {
  const short = Math.min(canvas.width, canvas.height);
  const long = PIP_SIZE * short;
  const [bw, bh] = w >= h ? [long, (long * h) / w] : [(long * w) / h, long];
  const right = canvas.safeArea?.right ?? canvas.width - PIP_MARGIN * short;
  const top = canvas.safeArea?.top ?? PIP_MARGIN * short;
  return { left: right - bw, top, width: bw, height: bh };
}

/**
 * Which part of the frame shown in `el` a point is over, by geometry, so it still answers once the drag has ended.
 * `label` is where the corner box's label was drawn last, which belongs to the box.
 */
function zoneAt(el: HTMLElement | null, label: DOMRect | null, canvas: Canvas, x: number, y: number, w: number, h: number): DropTarget["over"] | null {
  const r = el?.getBoundingClientRect();
  if (!r || x < r.left || x > r.right || y < r.top || y > r.bottom) return null;
  if (label && x >= label.left && x <= label.right && y >= label.top && y <= label.bottom) return "pictureInPicture";
  const k = r.width / canvas.width;
  const pip = pipBox(canvas, w, h);
  const [px, py] = [(x - r.left) / k, (y - r.top) / k];
  return px >= pip.left && px <= pip.left + pip.width && py >= pip.top && py <= pip.top + pip.height ? "pictureInPicture" : "overlay";
}

/** What is dragged, when it has a picture: its size, or for a file not imported yet the frame's, the likeliest guess. */
function draggedPicture(s: ReturnType<typeof useEditor.getState>) {
  if (s.assetDrag) {
    const asset = s.snap?.project.assets.find((a) => a.id === s.assetDrag!.assetId);
    return asset && asset.kind !== "audio" && asset.width > 0 ? { w: asset.width, h: asset.height } : null;
  }
  const canvas = s.snap?.project.canvas;
  return s.fileDrag && s.fileDrag.kind !== "audio" && canvas ? { w: canvas.width, h: canvas.height } : null;
}

/**
 * While a video or picture is dragged, the preview offers to put it over the video or in a picture in picture
 * at the playhead. Only this layer follows the pointer, so the preview itself does not re-render while dragging.
 */
export function PreviewDrop({ canvas }: { canvas: Canvas }) {
  const ref = useRef<HTMLDivElement>(null);
  const label = useRef<DOMRect | null>(null);
  // Under the AI lock nothing can be dropped, so nothing is offered; the lane on the timeline says why.
  const locked = useAiLocked();
  const picture = useEditor(useShallow(draggedPicture));
  const at = useEditor(useShallow((s) => (s.assetDrag ?? s.fileDrag) && { x: (s.assetDrag ?? s.fileDrag)!.x, y: (s.assetDrag ?? s.fileDrag)!.y }));

  useEffect(
    () =>
      addDropResolver((x, y, media) => {
        const { snap, timeUs } = useEditor.getState();
        if (media?.kind === "audio" || !snap || aiLocked()) return null;
        const { canvas } = snap.project;
        const shown = media?.width && media.height ? { w: media.width, h: media.height } : { w: canvas.width, h: canvas.height };
        const over = zoneAt(ref.current, label.current, canvas, x, y, shown.w, shown.h);
        return over ? { trackId: null, startUs: Math.round(timeUs), replaceId: null, over } : null;
      }),
    [],
  );

  const zone = picture && at ? zoneAt(ref.current, label.current, canvas, at.x, at.y, picture.w, picture.h) : null;
  const pip = picture && pipBox(canvas, picture.w, picture.h);
  const pct = (v: number, of: number) => `${(v / of) * 100}%`;
  const chip = "rounded-md bg-black/70 px-2 py-1 text-[12px] leading-tight font-semibold text-white";
  return (
    <div ref={ref} className="pointer-events-none absolute inset-0 z-30" data-testid="preview-drop">
      {picture && pip && !locked && (
        <>
          <div
            data-zone="overlay"
            data-hot={zone === "overlay" || undefined}
            className={`absolute inset-1.5 flex items-center justify-center rounded-md border-2 ${zone === "overlay" ? "border-accent bg-accent/15" : "border-dashed border-white/50"}`}
          >
            <span className={chip}>Overlay</span>
          </div>
          <div
            data-zone="pictureInPicture"
            data-hot={zone === "pictureInPicture" || undefined}
            className={`absolute rounded-md border-2 ${zone === "pictureInPicture" ? "border-accent bg-accent/25" : "border-dashed border-white/70 bg-black/40"}`}
            style={{ left: pct(pip.left, canvas.width), top: pct(pip.top, canvas.height), width: pct(pip.width, canvas.width), height: pct(pip.height, canvas.height) }}
          >
            {/* Under the box rather than in it: on a small preview the box is narrower than the words. */}
            <span ref={(el) => void (el && (label.current = el.getBoundingClientRect()))} className={`absolute top-full right-0 mt-1 whitespace-nowrap ${chip}`}>
              Picture in picture
            </span>
          </div>
        </>
      )}
    </div>
  );
}

import { Crosshair } from "lucide-react";
import { roomToFollow, startPicking, stopPicking, useFollow } from "../../lib/follow";
import { useReframe } from "../../lib/reframe";
import { readoutTime, transformAtPlayhead, useEditor } from "../../lib/store";
import type { Asset, Clip } from "../../lib/types";
import { Button, Section } from "../ui";

/**
 * Follow, under Motion for a video clip: box a person or a thing in the preview and the picture keeps it in the frame
 * through the clip, as keyframes that edit like any others.
 */
export function FollowSection({ clip, asset }: { clip: Clip; asset: Asset }) {
  const room = useEditor((s) => roomToFollow(asset, s.snap!.project.canvas, transformAtPlayhead(clip, readoutTime(s))));
  const picking = useFollow((s) => s.picking?.clipId === clip.id);
  const following = useFollow((s) => !!s.job);
  const reframing = useReframe((s) => !!s.job);
  const why = following
    ? "Already following. Stop it in the preview."
    : reframing
      ? "Wait for the reframe to finish"
      : !room
        ? "The picture just fills the frame, so it can't move. Change the Ratio or zoom in on the clip first."
        : undefined;
  return (
    <Section title="Follow">
      <Button
        data-follow
        aria-pressed={picking}
        className="w-full justify-center gap-1.5"
        disabled={!!why}
        disabledReason={why}
        title={
          picking
            ? "Stop choosing what to follow"
            : `Box a person or a thing in the preview, and the picture keeps it in the frame through the clip${clip.keyframes.length > 0 ? ", replacing the clip's keyframes" : ""}`
        }
        onClick={() => (picking ? stopPicking() : startPicking(clip.id))}
      >
        <Crosshair size={14} />
        {picking ? "Cancel choosing" : "Choose what to follow"}
      </Button>
    </Section>
  );
}

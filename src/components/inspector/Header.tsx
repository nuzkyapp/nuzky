import type { LucideIcon } from "lucide-react";

/** The quiet 24 px icon buttons of a section header: keyframes and reset. */
export const QUIET = "size-6 rounded-md";

const TONES = { video: "bg-clip-video", image: "bg-clip-image", audio: "bg-clip-audio", text: "bg-clip-title", caption: "bg-clip-captions" };

/** What the inspector shows: kind icon on a tile, in the colour of its clips on the timeline, and the name over a tabular detail line. */
export function InspectorHeader({ icon: Icon, title, detail, tone }: { icon: LucideIcon; title: string; detail: string; tone?: keyof typeof TONES }) {
  return (
    <div className="flex shrink-0 items-center gap-2.5 px-2.5 pb-3 pt-3">
      <span className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-fg shadow-[inset_0_0_0_1px_rgb(255_255_255/.12)] ${tone ? TONES[tone] : "bg-raised"}`}>
        <Icon size={16} />
      </span>
      <div className="min-w-0">
        <div className="truncate text-[15px] font-semibold leading-5 tracking-[-0.015em] text-fg">{title}</div>
        <div className="tabular mt-px truncate text-[12px] text-muted">{detail}</div>
      </div>
    </div>
  );
}

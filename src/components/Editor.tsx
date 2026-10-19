import { useDockLayout } from "../lib/dock";
import { PANE_MIN, paneLayout, setPane } from "../lib/layout";
import { usePreview } from "../lib/preview";
import { useReels } from "../lib/reels";
import { useEditor } from "../lib/store";
import { DockDragLayer, DockedAiPanel, FloatingAiPanel } from "./ai/AiDock";
import { Inspector } from "./inspector/Inspector";
import { LeftPanel } from "./panel/LeftPanel";
import { Preview } from "./preview/Preview";
import { ReelList } from "./reels/ReelList";
import { Timeline } from "./timeline/Timeline";
import { TopBar } from "./TopBar";
import { Splitter } from "./ui";

function DragChip() {
  const drag = useEditor((s) => s.assetDrag);
  const asset = useEditor((s) => s.snap?.project.assets.find((a) => a.id === s.assetDrag?.assetId));
  if (!drag || !asset) return null;
  return (
    <div className="pointer-events-none fixed z-[120] rounded-lg bg-raised px-2.5 py-1 text-[12px] font-medium text-fg shadow-lg shadow-black/50 ring-1 ring-white/10" style={{ left: drag.x + 12, top: drag.y + 10 }}>
      {asset.name}
    </div>
  );
}

/** The editor of the open project. It stays behind the home screen, so going back keeps everything as it was. */
export default function Editor({ panes, dock }: { panes: ReturnType<typeof paneLayout>; dock: ReturnType<typeof useDockLayout> }) {
  const recovery = useEditor((s) => !!s.snap?.recovery);
  const home = useEditor((s) => s.view === "home");
  const launcherOpen = useEditor((s) => s.launcherOpen);
  // Choosing reels: the candidates in the timeline's place, the preview wide without the inspector.
  const reels = useReels((s) => s.open);
  const full = usePreview((s) => s.full);
  return (
    <div className="flex h-full flex-col" inert={recovery || home || launcherOpen} aria-hidden={home || undefined}>
      <TopBar />
      <div className="flex min-h-0 flex-1">
        {dock.open && dock.mode === "left" && <DockedAiPanel side="left" width={dock.width} />}
        <div className="flex min-w-0 flex-1 flex-col">
          {/* The 6 px gaps between the panes are their resize handles. */}
          <div className="flex min-h-0 flex-1 px-1.5">
            <LeftPanel width={panes.library} />
            <Splitter
              what="library"
              axis="x"
              grow={1}
              size={panes.library}
              min={PANE_MIN.library}
              max={panes.max.library}
              onResize={(w) => setPane("library", w)}
              onReset={() => setPane("library", null)}
              className="relative -mx-0.5"
            />
            <Preview />
            {!reels && !(dock.open && dock.mode === "inspector") && (
              <>
                <Splitter
                  what="inspector"
                  axis="x"
                  grow={-1}
                  size={panes.inspector}
                  min={PANE_MIN.inspector}
                  max={panes.max.inspector}
                  onResize={(w) => setPane("inspector", w)}
                  onReset={() => setPane("inspector", null)}
                  className="relative -mx-0.5"
                />
                <Inspector width={panes.inspector} />
              </>
            )}
          </div>
          <Splitter
            what="timeline"
            axis="y"
            grow={-1}
            size={panes.timeline}
            min={PANE_MIN.timeline}
            max={panes.max.timeline}
            onResize={(h) => setPane("timeline", h)}
            onReset={() => setPane("timeline", null)}
            className="relative -my-0.5"
          />
          <div className="shrink-0 px-1.5 pb-1.5">
            {reels ? <ReelList height={panes.timeline} /> : <Timeline height={panes.timeline} />}
          </div>
        </div>
        {dock.open && dock.mode === "right" && <DockedAiPanel side="right" width={dock.width} />}
        {dock.open && dock.mode === "inspector" && <DockedAiPanel side="right" width={panes.inspector} inspectorMax={panes.max.inspector} />}
      </div>
      {/* Floating above the editor, it would float above the home screen and the full screen preview too; hidden there, it keeps what was typed. */}
      {dock.open && dock.mode === "float" && (
        <div hidden={home || full}>
          <FloatingAiPanel rect={dock.float} />
        </div>
      )}
      <DockDragLayer />
      <DragChip />
    </div>
  );
}

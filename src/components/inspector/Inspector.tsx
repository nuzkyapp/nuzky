import { useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { AudioLines, Blend, Captions, Film, Image as ImageIcon, Type } from "lucide-react";
import { TRANSITIONS } from "../../lib/presets";
import { useCover } from "../../lib/cover";
import { useGallery } from "../../lib/gallery";
import { usePlugins } from "../../lib/plugins";
import { findClip, isCaptionTrack, isFreeze, mainCuts, useEditor } from "../../lib/store";
import { formatDuration, formatTime } from "../../lib/time";
import type { Asset, Clip } from "../../lib/types";
import { CoverInspector } from "../cover/CoverInspector";
import { TransitionSettings } from "../panel/TransitionsTab";
import { AiLock, TabBar, TabPanel } from "../ui";
import { AnimationSection } from "./AnimationSection";
import { BackgroundSection } from "./BackgroundSection";
import { BeatsSection } from "./BeatsSection";
import { EffectsSection } from "./EffectsSection";
import { InspectorHeader } from "./Header";
import { AdjustSection, AudioSection, SpeedSection, VoiceSection } from "./MediaSections";
import { MotionSection } from "./MotionSection";
import { MultiInspector } from "./MultiInspector";
import { GalleryInspector } from "./GalleryInspector";
import { PluginInspector } from "./PluginInspector";
import { ProjectSection } from "./ProjectSection";
import { ShapeSection } from "./ShapeSection";
import { TextSection } from "./TextSection";
import { TransformSection } from "./TransformSection";

type TabId = "video" | "adjust" | "speed" | "animation" | "audio" | "text" | "transform";
type Kind = "video" | "image" | "audio" | "text" | "caption";

function tabsFor(kind: Kind, hasSound: boolean, freeze: boolean): { id: TabId; label: string }[] {
  if (kind === "audio") return [{ id: "audio", label: "Audio" }];
  if (kind === "text" || kind === "caption")
    return [
      { id: "text", label: "Text" },
      { id: "animation", label: "Animation" },
      { id: "transform", label: "Transform" },
    ];
  const tabs: { id: TabId; label: string }[] = [
    { id: "video", label: kind === "image" ? "Image" : "Video" },
    { id: "adjust", label: "Adjust" },
  ];
  if (kind === "video" && !freeze) tabs.push({ id: "speed", label: "Speed" });
  tabs.push({ id: "animation", label: "Animation" });
  if (hasSound && !freeze) tabs.push({ id: "audio", label: "Audio" });
  return tabs;
}

const KIND_ICON = { video: Film, image: ImageIcon, audio: AudioLines, text: Type, caption: Captions };

type Chosen = Partial<Record<Kind, TabId>>;

function ClipInspector({ clip, kind, asset, music, chosen, onChoose }: { clip: Clip; kind: Kind; asset?: Asset; /** A song on a music track, which the video can be cut to. */ music: boolean; chosen: Chosen; onChoose: (c: Chosen) => void }) {
  const c = clip.content;
  const hasSound = kind === "video" && !!asset?.hasAudio;
  const freeze = isFreeze(clip);
  const tabs = tabsFor(kind, hasSound, freeze);
  const tab = tabs.find((t) => t.id === chosen[kind])?.id ?? tabs[0].id;
  const title = kind === "caption" ? "Caption" : kind === "text" ? "Text" : (asset?.name ?? "Missing media");
  // A YouTube video's hidden footage stays hidden: until the clip reaches into it, times count within the part.
  const part = asset?.part;
  const within = !!part && c.type === "media" && c.sourceInUs >= part.startUs && c.sourceInUs + clip.durationUs * c.speed <= part.endUs + 1000;
  const [origin, length] = within && part ? [part.startUs, part.endUs - part.startUs] : [0, asset?.durationUs ?? 0];
  const detail =
    formatDuration(clip.durationUs) +
    (asset && asset.kind !== "image" && c.type === "media"
      ? ` · ${freeze ? "freeze frame at" : "from"} ${formatDuration(c.sourceInUs - origin)} of ${formatDuration(length)}`
      : "");

  return (
    <>
      <InspectorHeader icon={KIND_ICON[kind]} title={title} detail={detail} tone={kind} />
      <TabBar group="clip" label="Clip settings" className="mx-2.5 mb-1" tabs={tabs} value={tab} onChange={(id) => onChoose({ ...chosen, [kind]: id })} />
      <TabPanel group="clip" id={tab} className="min-h-0 flex-1 overflow-y-auto">
        <AiLock>
          {(tab === "video" || tab === "transform") && <TransformSection clip={clip} asset={asset} caption={kind === "caption"} />}
          {tab === "video" && c.type === "media" && (
            <>
              <ShapeSection clip={clip} content={c} asset={asset} />
              <BackgroundSection clips={[clip]} />
              <MotionSection clip={clip} />
            </>
          )}
          {tab === "text" && c.type === "text" && <TextSection clip={clip} text={c.text} style={c.style} caption={kind === "caption"} />}
          {tab === "adjust" && c.type === "media" && (
            <>
              <AdjustSection clip={clip} content={c} />
              <EffectsSection clip={clip} />
            </>
          )}
          {tab === "speed" && c.type === "media" && <SpeedSection clip={clip} content={c} asset={asset} />}
          {tab === "animation" && <AnimationSection clip={clip} />}
          {tab === "audio" && c.type === "media" && (
            <>
              <AudioSection clip={clip} content={c} />
              {music && asset && <BeatsSection clip={clip} asset={asset} />}
              <VoiceSection clips={[clip]} coalesce={clip.id} />
              {kind === "audio" && <SpeedSection clip={clip} content={c} asset={asset} />}
            </>
          )}
        </AiLock>
      </TabPanel>
    </>
  );
}

function CutInspector({ clipId }: { clipId: string }) {
  const cut = useEditor(useShallow((s) => (s.snap ? mainCuts(s.snap.project).find((c) => c.clipId === clipId) : undefined)));
  if (!cut) return null;
  const t = cut.transition;
  return (
    <>
      {/* The icon says what is selected, like for clips; a kind icon such as Slide left's arrow would read as "back". */}
      <InspectorHeader
        icon={Blend}
        title={t ? (TRANSITIONS.find((k) => k.kind === t.kind)?.label ?? "Transition") : "Cut"}
        detail={`${t ? "Transition · " : ""}cut at ${formatTime(cut.atUs)}`}
      />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <AiLock>
          <TransitionSettings cut={cut} />
        </AiLock>
      </div>
    </>
  );
}

export function Inspector({ width }: { width: number }) {
  const project = useEditor((s) => s.snap?.project);
  const selection = useEditor((s) => s.selection);
  const cut = useEditor((s) => s.cut);
  const cover = useCover((s) => s.open);
  const plugin = usePlugins((s) => s.list?.find((p) => p.id === s.open));
  const shared = useGallery((s) => s.list?.find((p) => p.id === s.open));
  // Remembered per clip kind, so picking another clip keeps the tab you were on, like CapCut.
  const [chosen, setChosen] = useState<Chosen>({});
  const found = project && selection.length === 1 ? findClip(project, selection[0]) : null;

  let body;
  if (!project) body = null;
  else if (cover) body = <CoverInspector format={cover} />;
  // Clips stay selectable while it shows, so a plugin can work on the selected ones.
  else if (plugin) body = <PluginInspector key={plugin.id} plugin={plugin} />;
  else if (shared) body = <GalleryInspector key={shared.id} plugin={shared} />;
  else if (cut) body = <CutInspector clipId={cut} />;
  else if (selection.length > 1)
    body = (
      <AiLock>
        <MultiInspector ids={selection} />
      </AiLock>
    );
  else if (!found) body = <ProjectSection />;
  else {
    const { clip, track } = found;
    const c = clip.content;
    const asset = c.type === "media" ? project.assets.find((a) => a.id === c.assetId) : undefined;
    const kind: Kind = c.type === "text" ? (isCaptionTrack(track) ? "caption" : "text") : track.kind === "audio" ? "audio" : (asset?.kind ?? "video");
    const music = kind === "audio" && track.keepInPlace && !!asset?.hasAudio;
    body = <ClipInspector key={clip.id} clip={clip} kind={kind} asset={asset} music={music} chosen={chosen} onChoose={setChosen} />;
  }

  return (
    <aside className="pane flex shrink-0 flex-col overflow-hidden" style={{ width }} aria-label="Inspector" data-dock-slot="inspector">
      {body}
    </aside>
  );
}

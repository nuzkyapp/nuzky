import { Captions, Languages, ScanFace, SlidersHorizontal } from "lucide-react";
import { useState } from "react";
import { checkFaces, useFaces } from "../../lib/faces";
import { useReframe } from "../../lib/reframe";
import { isLook, scaleStyle } from "../../lib/presets";
import { translateBlocked, translateCaptions, useAgent } from "../../lib/agent";
import { type CaptionLook, languageName, speaks, speechBlocker, startSpeech, useSpeech } from "../../lib/speech";
import { MAIN_TRACK, isCaptionTrack, isSpeechCaptionTrack, presetOver, useCanvasUnit, useEditor } from "../../lib/store";
import type { CaptionPreset, Clip, Track } from "../../lib/types";
import { CaptionLanguages, useSpeechLanguage } from "./CaptionLanguages";
import { MyStyles, STYLE_GRID, StyleTile, builtInStyles, forTrack, useMyStyles, type Tile } from "../inspector/CaptionStyles";
import { showCaptionText } from "../inspector/Inspector";
import { showInspector } from "../../lib/dock";
import { Button, LockReason, Segmented, useLockReason } from "../ui";
import { JobError, SpeechFields, SpeechJobCard, useSpeechJobs } from "./SpeechControls";

const WORDS = [
  { id: 1, label: "1", title: "One word at a time" },
  { id: 2, label: "2", title: "Up to two words, at most 15 characters, like reels" },
  { id: 3, label: "3", title: "Up to three words, at most 15 characters" },
  { id: 0, label: "Phrases", title: "Whole phrases" },
];

const BUSY = "Wait for speech recognition to finish";

const clips = (n: number) => `${n} ${n === 1 ? "clip" : "clips"}`;

/**
 * The selected video clips and clips of the user's own sound files, such as a voice-over, off the captions track, which
 * Generate can caption alone, as their ids joined by spaces.
 */
function useSelectedClips(): string {
  return useEditor((s) => {
    const project = s.snap?.project;
    if (!project) return "";
    return project.tracks
      .filter((t) => !isCaptionTrack(t))
      .flatMap((t) => t.clips)
      .filter((c) => {
        const media = c.content;
        return (
          s.selection.includes(c.id) &&
          media.type === "media" &&
          project.assets.some((a) => a.id === media.assetId && (a.kind === "video" || (a.kind === "audio" && speaks(a))))
        );
      })
      .map((c) => c.id)
      .join(" ");
  });
}

/** How many captions start over the clips `ids`, the ones captions made for them replace. */
const useCaptionsOver = (ids: string[]) =>
  useEditor((s) => {
    const tracks = s.snap?.project.tracks ?? [];
    const ranges = tracks.flatMap((t) => t.clips).filter((c) => ids.includes(c.id));
    const over = (c: Clip) => ranges.some((r) => r.startUs <= c.startUs && c.startUs < r.startUs + r.durationUs);
    return tracks.filter(isSpeechCaptionTrack).flatMap((t) => t.clips).filter(over).length;
  });

/**
 * The captions of `track` and the look regenerating keeps: the selected caption's, else the first one's.
 * Null without captions.
 */
function useCaptions(track: Track | undefined): { clips: Clip[]; look: CaptionLook } | null {
  const clips = track?.clips;
  const selected = useEditor((s) => clips?.find((c) => s.selection.includes(c.id)));
  const clip = selected ?? clips?.[0];
  if (!clips || clip?.content.type !== "text") return null;
  return { clips, look: { style: clip.content.style, animIn: clip.animIn ?? null, animOut: clip.animOut ?? null } };
}

export function CaptionsTab() {
  const lock = useLockReason();
  const captionWords = useSpeech((s) => s.captionWords);
  const pending = useSpeech((s) => s.captionLook);
  const tracks = useEditor((s) => s.snap?.project.tracks);
  const speech = tracks?.find(isSpeechCaptionTrack);
  const translations = tracks?.filter((t) => isCaptionTrack(t) && !!t.translation) ?? [];
  // The captions the tab is about, whose style Edit style shows: the speech's unless a translation was picked.
  const [picked, setPicked] = useState<string | null>(null);
  const translation = translations.find((t) => t.id === picked);
  const editing = translation ?? speech;
  const spoken = useSpeechLanguage(speech);
  const captions = useCaptions(editing);
  const onTimeline = captions?.look ?? null;
  const { running, last } = useSpeechJobs("captions");
  const blocker = useEditor((s) => (s.snap ? speechBlocker(s.snap.project) : null));
  const hasCaptions = onTimeline !== null;
  const unit = useCanvasUnit();
  // The look Generate gives: that of the captions on the timeline, or the one waiting, made for 1080p like a preset.
  const look = onTimeline ?? { ...pending, style: scaleStyle(pending.style, unit) };
  const mine = useMyStyles();
  const busy = !!running;
  const faceBlocker = useEditor((s) =>
    s.snap?.project.tracks.some((t) => !t.hidden && t.clips.length > 0 && (t.kind === "text" || (t.kind === "video" && t.id !== MAIN_TRACK)))
      ? null
      : "Add captions, a text or a sticker first",
  );
  const checking = useFaces((s) => !!s.job);
  const reframing = useReframe((s) => !!s.job);

  // Captions go over the selected videos unless Whole video was picked since the selection last changed.
  const selected = useSelectedClips();
  const whole = useSpeech((s) => s.captionsWhole);
  const ids = selected && !whole ? selected.split(" ") : [];
  const replaced = useCaptionsOver(ids);
  const remake = ids.length > 0 ? replaced > 0 : hasCaptions;

  // Regenerating keeps the look of the captions already there, edits included.
  const start = () => startSpeech(look, false, ids.length > 0 ? ids : null);
  // Before there are captions, a style is the look waiting for Generate; a style without its own font or case keeps the one waiting.
  const choose = (preset: CaptionPreset) =>
    useSpeech.setState({ captionLook: { style: presetOver(preset.style, pending.style, 1), animIn: preset.animIn ?? null, animOut: preset.animOut ?? null } });
  const tile: Tile = (preset, own) => (
    <StyleTile
      key={preset.name}
      preset={preset}
      font={pending.style.fontFamily}
      current={isLook(preset, pending.style, pending.animIn, pending.animOut)}
      disabled={busy || !!lock}
      title={lock ?? (busy ? BUSY : `Use ${preset.name} for new captions`)}
      onApply={() => choose(preset)}
      own={own}
    />
  );
  const heading = translations.length > 0 && editing ? `Style · ${editing.translation ? languageName(editing.translation) : (spoken ?? "Captions")}` : "Style";

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="scroll-shown flex min-h-0 flex-1 flex-col gap-3.5 overflow-y-auto p-3.5 pb-6">
        {speech && <CaptionLanguages speech={speech} language={spoken} translations={translations} editing={editing?.id ?? speech.id} onEdit={(id) => setPicked(id === speech.id ? null : id)} />}
        {!translation && (
          <>
            <SpeechFields disabled={busy} />
            <div className="flex flex-col gap-1.5">
              <span className="truncate text-[12px] text-muted" title={hasCaptions ? "Applies when you regenerate" : undefined}>
                Words per caption
              </span>
              <Segmented label="Words per caption" value={captionWords} onChange={(n) => useSpeech.setState({ captionWords: n })} options={WORDS} disabled={busy} disabledReason={BUSY} />
            </div>
          </>
        )}
        {captions && editing ? (
          // Captions on the timeline are styled in the inspector, the one place for their look.
          <div className="flex flex-col gap-1.5">
            <div className="flex min-w-0 items-baseline justify-between gap-2">
              <h3 className="shrink-0 text-[13px] font-semibold text-fg">{heading}</h3>
              <span className="truncate text-[12px] text-muted" data-caption-look>
                {lookName(captions.clips, !!translation, mine.styles, unit) ?? "Custom"}
              </span>
            </div>
            {/* It only selects a caption, so it works while the AI edits. */}
            <LockReason.Provider value={null}>
              <span className="flex">
                <Button className="flex-1" data-edit-caption-style onClick={() => editStyle(editing)}>
                  <SlidersHorizontal size={14} /> Edit style in the inspector
                </Button>
              </span>
            </LockReason.Provider>
          </div>
        ) : (
          !translation && (
            <>
              <div className="flex flex-col gap-1.5">
                <h3 className="text-[13px] font-semibold text-fg">{heading}</h3>
                <div className={STYLE_GRID}>{builtInStyles(false).map((s) => tile(s))}</div>
              </div>
              <MyStyles look={pending} mine={mine} tile={tile} />
            </>
          )
        )}
      </div>
      {/* Below the choices, in view however many styles there are. */}
      <div className="flex shrink-0 flex-col gap-2 border-t border-white/[.1] bg-panel p-3.5">
        {translation ? (
          <TranslateAgain language={translation.translation ?? ""} />
        ) : running ? (
          <SpeechJobCard job={running} />
        ) : (
          <>
            {selected && (
              <div className="flex flex-col gap-1.5">
                <span className="text-[12px] text-muted">Make captions for</span>
                <Segmented
                  label="Make captions for"
                  value={ids.length > 0 ? "selected" : "whole"}
                  onChange={(id) => useSpeech.setState({ captionsWhole: id === "whole" })}
                  options={[
                    { id: "whole", label: "Whole video" },
                    { id: "selected", label: `${clips(selected.split(" ").length)} selected` },
                  ]}
                />
              </div>
            )}
            <span className="flex">
              {/* Prominent until there are captions to replace; replacing them is then a secondary action. */}
              <Button variant={remake ? "secondary" : "primary"} className="flex-1" data-generate-captions disabled={!!blocker} disabledReason={blocker ?? undefined} onClick={start}>
                <Captions size={15} /> {remake ? "Regenerate captions" : "Generate captions"}
                {ids.length > 0 && ` for ${clips(ids.length)}`}
              </Button>
            </span>
            {replaced > 0 && <span className="text-[12px] text-muted">{`Replaces the ${replaced === 1 ? "caption" : `${replaced} captions`} over them`}</span>}
          </>
        )}
        {!running && <JobError job={last} />}
        <Button
          variant="plain"
          className="text-[12px]"
          data-check-faces
          disabled={!!faceBlocker || checking || reframing || busy}
          disabledReason={faceBlocker ?? (checking ? "Looking for faces" : reframing ? "Wait for the reframe to finish" : BUSY)}
          onClick={() => void checkFaces()}
        >
          <ScanFace size={14} /> Find text over faces
        </Button>
      </div>
    </div>
  );
}

/** The name of the style every caption of `clips` has, built in or the user's own, or null. */
function lookName(clips: Clip[], translation: boolean, own: CaptionPreset[], unit: number) {
  const has = (p: CaptionPreset) => clips.every((c) => c.content.type === "text" && isLook(p, c.content.style, c.animIn, c.animOut, unit));
  return builtInStyles(translation).find((p) => has(forTrack(p, translation, false)))?.name ?? own.find((p) => has(forTrack(p, translation, true)))?.name ?? null;
}

/**
 * Shows the captions of `track` in the inspector: selects the caption at the playhead, else the next one, moving the
 * playhead to it so the preview shows it, with its Style section in view and the keyboard on its Font.
 */
function editStyle(track: Track) {
  const { timeUs, seek, select } = useEditor.getState();
  const on = track.clips.find((c) => c.startUs <= timeUs && timeUs < c.startUs + c.durationUs);
  const caption = on ?? track.clips.find((c) => c.startUs >= timeUs) ?? track.clips[track.clips.length - 1];
  if (!caption) return;
  if (!on) seek(caption.startUs);
  showInspector();
  showCaptionText();
  select([caption.id]);
  window.setTimeout(() => {
    // The Style section from its heading, with the keyboard on Font right under it, so both stay in view.
    const section = document.querySelector("aside[aria-label=Inspector] [data-style-tile]")?.closest("section");
    const pane = section?.parentElement?.closest<HTMLElement>(".overflow-y-auto");
    if (section && pane) pane.scrollTop += section.getBoundingClientRect().top - pane.getBoundingClientRect().top;
    section?.querySelector<HTMLElement>('button[aria-label^="Font:"]')?.focus({ preventScroll: true });
  }, 0);
}

/** Translates the speech captions again into the language of the translation picked, replacing it. */
function TranslateAgain({ language }: { language: string }) {
  useAgent((s) => s.status);
  useAgent((s) => s.agent);
  const blocked = translateBlocked();
  const name = languageName(language);
  return (
    <>
      <span className="flex">
        <Button
          className="flex-1"
          data-translate-again
          title={`Replaces your edits of the ${name} captions. Undo brings them back.`}
          disabled={!!blocked}
          disabledReason={blocked ?? undefined}
          onClick={() => translateCaptions(language)}
        >
          <Languages size={15} /> Translate again with AI
        </Button>
      </span>
      {/* On a short window the styles need the room; the button's tooltip still says it. */}
      <span className="text-[12px] text-muted [@media(max-height:720px)]:hidden">{`Replaces your edits of the ${name} captions. Undo brings them back.`}</span>
    </>
  );
}

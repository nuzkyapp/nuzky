import { memo, type ReactNode } from "react";
import { AudioLines, Captions, Eye, EyeOff, Film, SlidersHorizontal, Type, Volume2, VolumeX } from "lucide-react";
import { languageName } from "../../lib/speech";
import { MAIN_TRACK, isCaptionTrack, pictureAssets, useEditor } from "../../lib/store";
import { CoverTile } from "../cover/CoverTile";
import type { Track } from "../../lib/types";

/** Track header toggle. Off states swap the icon and brighten it; accent stays for selection. */
function TrackToggle({ off, label, onIcon, offIcon, onClick, locked }: { off: boolean; label: string; onIcon: ReactNode; offIcon: ReactNode; onClick: () => void; locked: boolean }) {
  return (
    <button
      type="button"
      aria-label={locked ? `${label}: the AI is editing` : label}
      title={locked ? `${label}: the AI is editing` : label}
      aria-pressed={off}
      // Focusable while locked, so the keyboard reaches it and its reason.
      aria-disabled={locked || undefined}
      onClick={locked ? undefined : onClick}
      className={`inline-flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-md transition-colors duration-[120ms] ease-out aria-disabled:cursor-not-allowed aria-disabled:opacity-40 ${off ? "bg-raised text-fg" : locked ? "text-muted" : "text-muted hover:bg-white/[.08] hover:text-fg"}`}
    >
      {off ? offIcon : onIcon}
    </button>
  );
}

/**
 * Kind icon, name, and the hide and mute toggles of a track, pinned left while the lanes scroll.
 * It fills its whole row, gap included, and sits above the playhead (z 45), so the header column
 * stays unbroken and nothing scrolled under it shows through.
 */
export const TrackHeader = memo(function TrackHeader({ track, width, locked }: { track: Track; width: number; locked: boolean }) {
  const { edit } = useEditor.getState();
  // A video track holding only adjustment layers says so, in their colour.
  const adjusting = track.kind === "video" && track.clips.length > 0 && track.clips.every((c) => c.content.type === "adjustment");
  const KindIcon = track.kind === "audio" ? AudioLines : track.kind === "text" ? (isCaptionTrack(track) ? Captions : Type) : adjusting ? SlidersHorizontal : Film;
  // The colour of the track's clips: a video track holding only pictures takes the pictures' colour.
  const pictures = useEditor((s) => {
    const ids = s.snap && track.kind === "video" && track.clips.length > 0 ? pictureAssets(s.snap.project) : null;
    return !!ids && track.clips.every((c) => c.content.type === "media" && ids.has(c.content.assetId));
  });
  // Translated captions go by their language, so the two caption tracks read apart.
  const name = track.translation ? languageName(track.translation) : track.name || track.kind;
  const tone =
    track.kind === "audio" ? "bg-clip-audio" : track.kind === "text" ? (isCaptionTrack(track) ? "bg-clip-captions" : "bg-clip-title") : adjusting ? "bg-clip-adjustment" : pictures ? "bg-clip-image" : "bg-clip-video";
  return (
    // 22 px toggles and the 24 px tile leave the name about 50 px, enough for "Captions" and "Overlay".
    <div className="sticky left-0 z-[47] flex shrink-0 items-center gap-0.5 border-r border-white/[.07] bg-panel pl-2 pr-1" style={{ width }}>
      {/* The main track starts with the Cover tile, where CapCut has it; the toggles keep their columns. */}
      {track.id === MAIN_TRACK ? (
        // The Cover tile where CapCut has it, then the track's name, so every header reads the same way.
        <span className="flex min-w-0 flex-1 items-center gap-2">
          <CoverTile />
          <span className="min-w-0 truncate text-[12px] font-medium text-fg">{track.name || "Video"}</span>
        </span>
      ) : (
        <>
          <span aria-hidden className={`mr-1.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-[7px] text-fg shadow-[inset_0_1px_0_rgb(255_255_255/.18)] ${tone}`}>
            <KindIcon size={13} />
          </span>
          <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-fg">{name}</span>
        </>
      )}
      {/* Fixed columns: eye, then speaker; a spacer keeps the column when a toggle does not apply. */}
      {track.kind !== "audio" ? (
        <TrackToggle
          off={track.hidden}
          label={track.hidden ? `Show ${name}` : `Hide ${name}`}
          onIcon={<Eye size={14} />}
          offIcon={<EyeOff size={14} />}
          onClick={() => edit({ type: "updateTrack", trackId: track.id, hidden: !track.hidden })}
          locked={locked}
        />
      ) : (
        <span className="w-[22px] shrink-0" aria-hidden />
      )}
      {track.kind !== "text" && !adjusting ? (
        <TrackToggle
          off={track.muted}
          label={track.muted ? `Unmute ${track.name}` : `Mute ${track.name}`}
          onIcon={<Volume2 size={14} />}
          offIcon={<VolumeX size={14} />}
          onClick={() => edit({ type: "updateTrack", trackId: track.id, muted: !track.muted })}
          locked={locked}
        />
      ) : (
        <span className="w-[22px] shrink-0" aria-hidden />
      )}
    </div>
  );
});

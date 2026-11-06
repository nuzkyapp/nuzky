import { FreesoundKey, OPENVERSE, SEARCH_SENDS } from "../panel/FreesoundKey";
import { MusicStep } from "../setup/steps/MusicStep";
import { Group, Item } from "./parts";
import { StepSection } from "./StepSection";

/** Settings › Sounds & YouTube: where the sound library searches, the Freesound key, and yt-dlp for music from YouTube. */
export function SoundsPane() {
  return (
    <div className="flex flex-col gap-6">
      <Group title="Sound library" note={SEARCH_SENDS}>
        <Item title="Openverse" detail={OPENVERSE} />
        <div data-setting="freesound" className="flex flex-col gap-2 border-t border-white/[.06] px-3.5 py-3 text-[12px]">
          <span className="text-[13px] font-medium text-fg">Freesound</span>
          <FreesoundKey />
        </div>
      </Group>
      <StepSection title="Music from YouTube" id="ytdlp">
        <MusicStep />
      </StepSection>
    </div>
  );
}

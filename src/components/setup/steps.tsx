import type { ComponentType } from "react";
import type { SetupStep } from "../../lib/setup";
import { AiStep } from "./steps/AiStep";
import { CaptionsStep } from "./steps/CaptionsStep";
import { MusicStep } from "./steps/MusicStep";
import { VisionStep } from "./steps/VisionStep";

export const STEPS: Record<SetupStep, { title: string; Body: ComponentType }> = {
  captions: { title: "Captions model", Body: CaptionsStep },
  vision: { title: "Face and person models", Body: VisionStep },
  music: { title: "Music from YouTube", Body: MusicStep },
  ai: { title: "Connect your AI", Body: AiStep },
};

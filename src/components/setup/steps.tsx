import type { ComponentType } from "react";
import type { SetupStep } from "../../lib/setup";
import { AiStep } from "./steps/AiStep";
import { CaptionsStep } from "./steps/CaptionsStep";
import { VisionStep } from "./steps/VisionStep";

export const STEPS: Record<SetupStep, { title: string; Body: ComponentType }> = {
  captions: { title: "Captions model", Body: CaptionsStep },
  vision: { title: "Face and person models", Body: VisionStep },
  ai: { title: "Connect your AI", Body: AiStep },
};

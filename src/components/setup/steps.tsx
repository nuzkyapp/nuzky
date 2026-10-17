import type { ComponentType } from "react";
import type { SetupStep } from "../../lib/setup";
import { AiStep } from "./steps/AiStep";
import { CaptionsStep } from "./steps/CaptionsStep";

export const STEPS: Record<SetupStep, { title: string; Body: ComponentType }> = {
  captions: { title: "Captions model", Body: CaptionsStep },
  ai: { title: "Connect your AI", Body: AiStep },
};
